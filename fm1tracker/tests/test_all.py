import os
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import fm1_usb as usb
import fx as fxmod
from engine import Engine, RELEASE
from model import (EMPTY, NOTE_OFF, Project, Scale, note_name, project_from_dict,
                   project_to_dict)
import persist


def make_engine(p=None, transport=None):
    p = p or Project()
    return Engine(transport or usb.MockTransport(), p), p


def run(e, ticks):
    """-> list of (tick, [decoded msgs])"""
    out = []
    for t in range(ticks):
        d = e.process_tick()
        if d:
            out.append((t, usb.decode_packets(d)))
    return out


def basic_song(p, steps):
    ph = p.phrase(0)
    for i, (n, kw) in steps.items():
        ph.steps[i].note = n
        for k, v in kw.items():
            setattr(ph.steps[i], k, v)
    p.chain(0).phrase[0] = 0
    p.song[0][0] = 0
    return ph


class UsbTests(unittest.TestCase):
    def test_ioctl_numbers(self):
        self.assertEqual(usb.USBDEVFS_CLAIMINTERFACE, 0x8004550F)
        self.assertEqual(usb.USBDEVFS_RELEASEINTERFACE, 0x80045510)
        if usb._PTR64:
            self.assertEqual(usb.USBDEVFS_BULK, 0xC0185502)

    def test_packets(self):
        self.assertEqual(usb.pkt_note_on(0, 60, 100), bytes([0x09, 0x90, 60, 100]))
        self.assertEqual(usb.pkt_note_off(1, 60), bytes([0x08, 0x81, 60, 0]))
        self.assertEqual(usb.pkt_cc(2, 74, 64), bytes([0x0B, 0xB2, 74, 64]))
        self.assertEqual(usb.pkt_pc(0, 12), bytes([0x0C, 0xC0, 12, 0]))
        self.assertEqual(usb.pkt_pitchbend(0, 8192), bytes([0x0E, 0xE0, 0, 64]))
        self.assertEqual(usb.pkt_realtime(0xF8), bytes([0x0F, 0xF8, 0, 0]))
        self.assertEqual(usb.pkt_note_on(0, 60, 0)[3], 1)        # vel 0 would be a note-off

    def test_discovery_from_fake_sysfs(self):
        with tempfile.TemporaryDirectory() as d:
            def w(rel, val):
                path = os.path.join(d, rel)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                open(path, "w").write(val + "\n")
            w("8-1/idVendor", "4c4a"); w("8-1/idProduct", "c755")
            w("8-1/busnum", "8"); w("8-1/devnum", "7"); w("8-1/product", "FM-1")
            w("8-1/8-1:1.4/bInterfaceClass", "01"); w("8-1/8-1:1.4/bInterfaceSubClass", "03")
            w("8-1/8-1:1.4/bInterfaceNumber", "04")
            w("8-1/8-1:1.4/ep_04/direction", "out"); w("8-1/8-1:1.4/ep_04/bEndpointAddress", "04")
            w("8-1/8-1:1.4/ep_84/direction", "in"); w("8-1/8-1:1.4/ep_84/bEndpointAddress", "84")
            w("1-1/idVendor", "dead"); w("1-1/idProduct", "beef")
            found = usb.find_devices(sysfs=d)
            self.assertEqual(len(found), 1)
            self.assertEqual(found[0].path, "/dev/bus/usb/008/007")   # devnum from sysfs, not "last glob"
            self.assertEqual((found[0].iface, found[0].ep_out), (4, 4))

    def test_no_device(self):
        self.assertEqual(usb.find_devices(sysfs="/nonexistent"), [])

    def test_usb_transport_never_raises_when_absent(self):
        t = usb.UsbTransport(sysfs="/nonexistent")
        t.start()
        self.assertFalse(t.connected)
        self.assertFalse(t.send(usb.pkt_note_on(0, 60, 100)))
        t.close()


class ModelTests(unittest.TestCase):
    def test_roundtrip(self):
        p = Project()
        p.name, p.bpm, p.latency_ms = "SONG", 143, 12
        basic_song(p, {0: (60, {"vel": 90, "instr": 3}), 5: (NOTE_OFF, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("ARP"), 0x37]
        p.chain(0).transpose[0] = -5
        p.instrument(3).program = 42
        p.instrument(3).ccs[0] = [74, 100]
        p.table(1).rows[2].trn = 7
        p.table(1).rows[2].fx[0] = [fxmod.id_of("KIL"), 3]
        p.groove_(5).ticks[0] = 9
        d = project_to_dict(p)
        self.assertEqual(d["version"], 1)
        q = project_from_dict(d)
        self.assertEqual(project_to_dict(q), d)
        self.assertEqual(q.phrase(0).steps[0].fx[0], [fxmod.id_of("ARP"), 0x37])
        self.assertEqual(q.instrument(3).program, 42)

    def test_sparse(self):
        d = project_to_dict(Project())
        self.assertEqual(d["chains"], {})
        self.assertEqual(d["phrases"], {})
        self.assertEqual(d["song"], [])

    def test_future_version_refused(self):
        with self.assertRaises(ValueError):
            project_from_dict({"version": 99})

    def test_unknown_fx_name_ignored(self):
        d = project_to_dict(Project())
        d["phrases"] = {"0": {"0": {"n": 60, "v": 255, "i": 255, "f": [["ZZZ", 1]]}}}
        q = project_from_dict(d)
        self.assertEqual(q.phrase(0).steps[0].fx[0][0], 0)

    def test_sanitize(self):
        q = project_from_dict({"version": 1, "bpm": 9999, "midi_channel": 99, "latency_ms": -4})
        self.assertEqual((q.bpm, q.midi_channel, q.latency_ms), (300, 15, 0))

    def test_scale(self):
        s = Scale("MAJ", 0, [i in (0, 2, 4, 5, 7, 9, 11) for i in range(12)])
        self.assertEqual(s.snap(61, +1), 62)
        self.assertEqual(s.snap(61, -1), 60)
        self.assertEqual(s.step(64, +1), 65)
        self.assertEqual(s.step(65, -1), 64)

    def test_note_names(self):
        self.assertEqual(note_name(60), "C-4")
        self.assertEqual(note_name(61), "C#4")
        self.assertEqual(note_name(EMPTY), "---")

    def test_save_load_recovery(self):
        with tempfile.TemporaryDirectory() as d:
            p = Project(); p.name = "ABC"
            persist.save_project(p, persist.slot_path(d, 1))
            self.assertEqual(persist.load_project(persist.slot_path(d, 1)).name, "ABC")
            a = persist.AutoSaver(d, interval=0)
            self.assertFalse(a.has_recovery())
            a.tick(p, dirty=True)
            self.assertTrue(a.has_recovery())
            self.assertEqual(a.load_recovery().name, "ABC")
            a.clear()
            self.assertFalse(a.has_recovery())
            open(persist.recovery_path(d), "w").write("{garbage")
            self.assertIsNone(a.load_recovery())


class EngineTests(unittest.TestCase):
    def test_sixteenth_grid(self):
        e, p = make_engine()
        basic_song(p, {i: (60 + i, {}) for i in range(4)})
        e.play_song(0)
        ons = [t for t, m in run(e, 24) if any("NOTE ON" in x for x in m)]
        self.assertEqual(ons, [0, 6, 12, 18])

    def test_program_change_before_first_note_only(self):
        e, p = make_engine()
        p.instrument(0).program = 17
        basic_song(p, {0: (60, {}), 1: (62, {})})
        e.play_song(0)
        msgs = [m for _, ms in run(e, 20) for m in ms]
        self.assertEqual(sum("PROGRAM" in m for m in msgs), 1)
        self.assertLess([i for i, m in enumerate(msgs) if "PROGRAM" in m][0],
                        [i for i, m in enumerate(msgs) if "NOTE ON" in m][0])

    def test_gate_and_mono(self):
        e, p = make_engine()
        p.instrument(0).gate = 2
        basic_song(p, {0: (60, {}), 1: (62, {})})
        e.play_song(0)
        evs = run(e, 14)
        self.assertIn((2, ["ch1 NOTE OFF 60"]), evs)

    def test_note_off_step(self):
        e, p = make_engine()
        p.instrument(0).gate = 20
        basic_song(p, {0: (60, {}), 2: (NOTE_OFF, {})})
        e.play_song(0)
        evs = dict(run(e, 14))
        self.assertIn("ch1 NOTE OFF 60", evs[12])

    def test_transpose_chain_project_instrument(self):
        e, p = make_engine()
        p.transpose = 1
        p.instrument(0).transpose = 2
        basic_song(p, {0: (60, {})})
        p.chain(0).transpose[0] = 4
        e.play_song(0)
        ons = [m for _, ms in run(e, 2) for m in ms if "NOTE ON" in m]
        self.assertIn("ch1 NOTE ON 67 vel 100", ons)

    def test_velocity_and_instrument_default(self):
        e, p = make_engine()
        p.instrument(2).velocity = 77
        basic_song(p, {0: (60, {"instr": 2}), 1: (61, {"vel": 33})})
        e.play_song(0)
        ons = [m for _, ms in run(e, 8) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, ["ch1 NOTE ON 60 vel 77", "ch1 NOTE ON 61 vel 33"])

    def test_instrument_change_releases_and_reselects(self):
        e, p = make_engine()
        p.instrument(0).gate = 50
        p.instrument(1).program = 9
        basic_song(p, {0: (60, {}), 1: (62, {"instr": 1})})
        e.play_song(0)
        msgs = [m for _, ms in run(e, 8) for m in ms]
        i_off, i_pc = msgs.index("ch1 NOTE OFF 60"), msgs.index("ch1 PROGRAM 9")
        self.assertLess(i_off, i_pc)

    def test_fx_del(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("DEL"), 3]
        e.play_song(0)
        self.assertEqual([t for t, m in run(e, 6) if any("NOTE ON" in x for x in m)], [3])

    def test_fx_kil(self):
        e, p = make_engine()
        p.instrument(0).gate = 6
        basic_song(p, {0: (60, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("KIL"), 2]
        e.play_song(0)
        self.assertIn(2, [t for t, m in run(e, 6) if any("NOTE OFF" in x for x in m)])

    def test_fx_ret(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("RET"), 0x22]     # every 2 ticks, -8 vel
        e.play_song(0)
        ons = [m for _, ms in run(e, 6) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, ["ch1 NOTE ON 60 vel 100", "ch1 NOTE ON 60 vel 92",
                               "ch1 NOTE ON 60 vel 84"])

    def test_fx_arp(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {})})
        p.arp_rate = 2
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("ARP"), 0x47]
        e.play_song(0)
        ons = [m.split()[3] for _, ms in run(e, 6) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, ["60", "64", "67"])

    def test_fx_pbn_mcc_pch(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {})})
        p.instrument(0).mcc_cc = 71
        s = p.phrase(0).steps[0]
        s.fx[0] = [fxmod.id_of("PBN"), 0x80]
        s.fx[1] = [fxmod.id_of("MCC"), 0xFE]
        s.fx[2] = [fxmod.id_of("PCH"), 5]
        e.play_song(0)
        msgs = [m for _, ms in run(e, 2) for m in ms]
        self.assertIn("ch1 BEND 8192", msgs)
        self.assertIn("ch1 CC 71 = 127", msgs)
        self.assertEqual([m for m in msgs if "PROGRAM" in m][-1], "ch1 PROGRAM 5")   # PCH wins

    def test_fx_tpo_and_vol(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {}), 1: (60, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("TPO"), 200]
        p.phrase(0).steps[0].fx[1] = [fxmod.id_of("VOL"), 0x40]
        e.play_song(0)
        ons = [m for _, ms in run(e, 8) for m in ms if "NOTE ON" in m]
        self.assertEqual(e.bpm, 200)
        self.assertEqual(ons[0], "ch1 NOTE ON 60 vel 50")

    def test_fx_tic_and_grv(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {}), 1: (61, {}), 2: (62, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("TIC"), 3]
        e.play_song(0)
        self.assertEqual([t for t, m in run(e, 10) if any("NOTE ON" in x for x in m)], [0, 3, 6])
        e, p = make_engine()
        basic_song(p, {0: (60, {}), 1: (61, {})})
        p.phrase(0).steps[0].fx[0] = [fxmod.id_of("GRV"), 1]      # groove 1 = 7,5,7,5...
        e.play_song(0)
        self.assertEqual([t for t, m in run(e, 14) if any("NOTE ON" in x for x in m)], [0, 7])

    def test_fx_hop(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {}), 1: (61, {}), 2: (62, {})})
        p.phrase(0).steps[2].fx[0] = [fxmod.id_of("HOP"), 0x10]    # once back to step 0
        e.play_song(0)
        ons = [m.split()[3] for _, ms in run(e, 6 * 8) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons[:6], ["60", "61", "62", "60", "61", "62"])

    def test_groove_swing(self):
        e, p = make_engine()
        p.groove = 1
        basic_song(p, {0: (60, {}), 1: (61, {}), 2: (62, {})})
        e.play_song(0)
        self.assertEqual([t for t, m in run(e, 20) if any("NOTE ON" in x for x in m)], [0, 7, 12])

    def test_table_per_note(self):
        e, p = make_engine()
        t = p.table(0)
        t.rows[0].trn, t.rows[1].trn = 0, 12
        p.instrument(0).table = 0
        basic_song(p, {0: (60, {}), 1: (60, {}), 2: (60, {})})
        e.play_song(0)
        ons = [m.split()[3] for _, ms in run(e, 14) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, ["60", "72", "60"])

    def test_table_tick_mode(self):
        e, p = make_engine()
        t = p.table(0)
        t.mode = 1
        t.rows[1].trn = 7
        p.instrument(0).table = 0
        p.instrument(0).table_rate = 2
        basic_song(p, {0: (60, {})})
        e.play_song(0)
        ons = [(t, m.split()[3]) for t, ms in run(e, 6) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, [(0, "60"), (2, "67")])

    def test_cc_slots_on_select(self):
        e, p = make_engine()
        p.instrument(0).ccs[0] = [74, 99]
        p.instrument(0).bank_msb = 1
        basic_song(p, {0: (60, {})})
        e.play_song(0)
        msgs = [m for _, ms in run(e, 2) for m in ms]
        self.assertIn("ch1 CC 74 = 99", msgs)
        self.assertIn("ch1 CC 0 = 1", msgs)

    def test_mute_solo(self):
        e, p = make_engine()
        basic_song(p, {0: (60, {})})
        e.play_song(0)
        e.set_mute(0, True)
        e.transport.log.clear()
        self.assertFalse([m for _, ms in run(e, 2) for m in ms if "NOTE ON" in m])
        e.set_mute(0, False)
        e.toggle_solo(3)
        self.assertFalse(e.audible(e.tracks[0]))
        e.toggle_solo(3)
        self.assertTrue(e.audible(e.tracks[0]))

    def test_stop_sends_noteoffs_and_cc123(self):
        t = usb.MockTransport()
        e, p = make_engine(transport=t)
        p.instrument(0).gate = 99
        basic_song(p, {0: (60, {})})
        e.play_song(0)
        for _ in range(2):
            t.send(e.process_tick())
        e.stop()
        msgs = t.messages()
        self.assertIn("ch1 NOTE OFF 60", msgs)
        self.assertIn("ch1 CC 123 = 0", msgs)
        self.assertFalse(e.playing)

    def test_shared_note_refcount(self):
        t = usb.MockTransport()
        e, p = make_engine(transport=t)
        p.instrument(0).gate = 99
        for tr in (0, 1):
            p.song[0][tr] = 0
        basic_song(p, {0: (60, {})})
        e.play_song(0)
        t.send(e.process_tick())
        e.set_mute(0, True)           # track 1 still holds C4: no note-off may be sent yet
        self.assertEqual(t.messages().count("ch1 NOTE OFF 60"), 0)
        e.set_mute(1, True)
        self.assertEqual(t.messages().count("ch1 NOTE OFF 60"), 1)

    def test_song_rows_and_loop(self):
        e, p = make_engine()
        p.loop_song = False
        ph = p.phrase(0); ph.steps[0].note = 60
        p.chain(0).phrase[0] = 0
        p.song[0][0] = 0
        p.song[1][0] = 0
        e.play_song(0)
        ons = [t for t, m in run(e, 16 * 6 * 3) if any("NOTE ON" in x for x in m)]
        self.assertEqual(ons, [0, 96])
        self.assertTrue(e._finish)

        e, p = make_engine()
        p.loop_song = True
        p.phrase(0).steps[0].note = 60
        p.chain(0).phrase[0] = 0
        p.song[0][0] = 0
        e.play_song(0)
        ons = [t for t, m in run(e, 96 * 3) if any("NOTE ON" in x for x in m)]
        self.assertEqual(ons, [0, 96, 192])

    def test_empty_song_does_not_start(self):
        e, p = make_engine()
        self.assertFalse(e.play_song(0))
        self.assertFalse(e.playing)

    def test_play_phrase_and_chain_isolation(self):
        e, p = make_engine()
        p.phrase(4).steps[0].note = 64
        e.play_phrase(2, 4)
        ons = [t for t, m in run(e, 200) if any("NOTE ON" in x for x in m)]
        self.assertEqual(ons[:3], [0, 96, 192])
        e.stop()
        p.chain(7).phrase[0] = 4
        p.chain(7).phrase[1] = 4
        e.play_chain(0, 7)
        ons = [t for t, m in run(e, 200) if any("NOTE ON" in x for x in m)]
        self.assertEqual(ons[:3], [0, 96, 192])

    def test_live_queue_chain(self):
        e, p = make_engine()
        p.phrase(0).steps[0].note = 60
        p.phrase(1).steps[0].note = 72
        p.chain(0).phrase[0] = 0
        p.chain(1).phrase[0] = 1
        p.song[0][0] = 0
        p.song[1][0] = 0
        e.play_song(0)
        e.queue_chain(0, 1)
        ons = [m.split()[3] for _, ms in run(e, 96 * 3) for m in ms if "NOTE ON" in m]
        self.assertEqual(ons, ["60", "72", "72"])        # then loops chain 1
        e.queue_chain(0, None)

    def test_usb_failure_does_not_crash_engine(self):
        t = usb.MockTransport()
        e, p = make_engine(transport=t)
        basic_song(p, {0: (60, {})})
        e.start_thread()
        t.fail_next = 1
        e.play_song(0)
        time.sleep(0.15)
        self.assertFalse(t.connected)
        self.assertTrue(e.playing)               # engine keeps running
        t.set_connected(True)
        time.sleep(0.3)
        e.shutdown()
        self.assertFalse(e._thread.is_alive())

    def test_clock_thread_timing_and_latency(self):
        t = usb.MockTransport()
        e, p = make_engine(transport=t)
        p.bpm = 180
        p.latency_ms = 20
        basic_song(p, {i: (60, {}) for i in range(16)})
        e.start_thread()
        t0 = time.monotonic_ns()
        e.play_song(0)
        time.sleep(1.0)
        e.shutdown()
        self.assertGreater(e.stats.n, 40)
        first_on = [ts for ts, d in t.log if b"\x09\x90" in d][0]
        # first tick is scheduled `latency` + 3 ms after start and sent latency early => ~3 ms
        self.assertLess((first_on - t0) / 1e6, 30)
        self.assertLess(e.stats.avg, 2000)       # generous: dev boxes vary

    def test_preview(self):
        t = usb.MockTransport()
        e, p = make_engine(transport=t)
        e.preview(0, 64, ms=1)
        time.sleep(0.01)
        e._service_timed(time.monotonic_ns())
        m = t.messages()
        self.assertIn("ch1 NOTE ON 64 vel 100", m)
        self.assertIn("ch1 NOTE OFF 64", m)


class FxTests(unittest.TestCase):
    def test_registry_extensible(self):
        n = fxmod.count()

        @fxmod.command("TST", "test", "XX")
        def _t(ctx, v):
            ctx.delay = v
        self.assertEqual(fxmod.count(), n + 1)
        c = fxmod.StepCtx()
        fxmod.run(fxmod.id_of("TST"), c, 9)
        self.assertEqual(c.delay, 9)

    def test_required_commands(self):
        for name in "ARP DEL RET KIL GRV TIC HOP PBN MCC PCH TPO VOL".split():
            self.assertNotEqual(fxmod.id_of(name), 0, name)


if __name__ == "__main__":
    unittest.main()
