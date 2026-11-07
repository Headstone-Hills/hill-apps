#!/usr/bin/env python3
"""fm1tool - hardware diagnostics for the FM-1 (run over SSH on the SP; no UI).

  fm1tool.py list                       find the FM-1, show bus/device/interfaces
  fm1tool.py descriptors [--file F] [--save F]   decode USB audio streaming alt settings
  fm1tool.py note 60 [--len 0.8]        MIDI out: play a note (proven path)
  fm1tool.py midi-in [--seconds 10]     print what the FM-1 SENDS (do its keys emit MIDI?)
  fm1tool.py capture 3 out.wav [--note 60]   PHASE 0: isochronous audio capture + numeric report
  fm1tool.py wavcheck file.wav          peak/RMS/silence/L-R analysis of any WAV

--mock on `capture` synthesises a tone so the whole pipeline can be tested off-device.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import fm1_usb as usb                      # noqa: E402
import usb_desc                            # noqa: E402
import wavcheck                            # noqa: E402


def find_helper():
    """Locate isocap and return a path that is actually executable. The SD card is exfat and
    may be mounted noexec / without exec bits, so the helper is copied to a temp dir first."""
    import shutil
    for p in (os.environ.get("POCKETDAW_ISOCAP"), os.path.join(HERE, "isocap"),
              os.path.join(HERE, "dist", "isocap"), os.path.join(HERE, "dist", "isocap-host")):
        if p and os.path.isfile(p):
            dst = os.path.join(tempfile.gettempdir(), "pocketdaw-isocap")
            shutil.copyfile(p, dst)
            os.chmod(dst, 0o755)
            return dst
    return None


def need_device():
    devs = usb.find_devices()
    if not devs:
        print("FM-1 (%04x:%04x) not found. Plugged into the USB-C port? Try `lsusb`." %
              (usb.FM1_VID, usb.FM1_PID), file=sys.stderr)
        sys.exit(2)
    return devs[0]


def cmd_list(a):
    for d in usb.find_devices():
        print("%s  %s  product=%r" % (d.sysname, d.path, d.product))
        print("  MIDI interface %d: bulk-OUT 0x%02x, bulk-IN 0x%02x" % (d.iface, d.ep_out, d.ep_in))
        for s in usb_desc.parse_audio_streams(usb_desc.read_descriptors(d.sysdir)):
            print("  audio", s)
        return 0
    print("no FM-1 found")
    return 1


def cmd_descriptors(a):
    if a.file:
        blob = open(a.file, "rb").read()
    else:
        d = need_device()
        blob = usb_desc.read_descriptors(d.sysdir)
        if a.save:
            open(a.save, "wb").write(blob)
            print("saved %d bytes to %s" % (len(blob), a.save))
    print("descriptor blob: %d bytes" % len(blob))
    streams = usb_desc.parse_audio_streams(blob)
    for s in streams:
        print(" ", s)
    cap = usb_desc.pick_capture(streams)
    print("capture choice:", cap if cap else "none")
    return 0


def cmd_note(a):
    t = usb.UsbTransport()
    t.start()
    if not t.connected:
        print("FM-1 not connected:", t.status(), file=sys.stderr)
        return 2
    try:
        t.send(usb.pkt_note_on(a.ch - 1, a.n, a.vel))
        time.sleep(a.len)
    finally:
        t.send(usb.pkt_note_off(a.ch - 1, a.n))
        t.close()
    print("note %d sent" % a.n)
    return 0


def cmd_midi_in(a):
    d = need_device()
    dev = usb.UsbMidiDevice(d)
    dev.open()
    print("listening %ds on endpoint 0x%02x - press FM-1 keys / turn knobs ..." % (a.seconds, d.ep_in))
    end, got, reads, empty, errs = time.monotonic() + a.seconds, 0, 0, 0, 0
    try:
        while time.monotonic() < end:
            try:
                data = dev.read(100)
            except usb.UsbError as e:
                errs += 1
                print("read error:", e)
                time.sleep(0.2)
                continue
            reads += 1
            if not data:
                empty += 1
            for i in range(0, len(data) - len(data) % 4, 4):
                if data[i:i + 4] == b"\0\0\0\0":
                    continue
                got += 1
                print(" ".join(usb.decode_packets(data[i:i + 4])))
    finally:
        dev.close()
    print("reads=%d (timeouts=%d, errors=%d), MIDI packets=%d" % (reads, empty, errs, got))
    if got:
        print("RESULT: FM-1 TRANSMITS MIDI")
    elif errs:
        print("RESULT: read errors - the endpoint is not readable; see messages above")
    else:
        print("RESULT: endpoint readable (%d clean timeouts) but the FM-1 sent nothing" % empty)
    return 0


def cmd_capture(a):
    if a.mock:
        raw = wavcheck.synth_tone(a.seconds)
        ch, sub, rate = 2, 3, 44100
        stats = {}
    else:
        d = need_device()
        cap = usb_desc.pick_capture(usb_desc.parse_audio_streams(usb_desc.read_descriptors(d.sysdir)))
        if not cap:
            print("no capture endpoint in descriptors", file=sys.stderr)
            return 2
        helper = find_helper()
        if not helper:
            print("isocap helper not found; run ./build_helper.sh and deploy dist/isocap", file=sys.stderr)
            return 2
        print("capturing from %s via %s: iface %d alt %d ep 0x%02x maxpkt %d (%dch %d-bit %s Hz)" %
              (d.path, os.path.basename(helper), cap.iface, cap.alt, cap.ep, cap.max_packet,
               cap.channels, cap.bits, cap.rates))
        ch, sub, rate = cap.channels, cap.subframe, (44100 if 44100 in cap.rates else cap.rates[0])
        tmp = tempfile.NamedTemporaryFile(suffix=".raw", delete=False)
        tmp.close()
        cmd = [helper, d.path, str(cap.iface), str(cap.alt), "0x%02x" % cap.ep, str(cap.max_packet),
               str(a.frames), str(a.urbs), str(a.seconds), tmp.name]
        midi = None
        if a.note is not None:
            midi = usb.UsbTransport()
            midi.start()

            def play():
                time.sleep(min(0.5, a.seconds / 4.0))
                midi.send(usb.pkt_note_on(0, a.note, 110))
                time.sleep(max(0.3, a.seconds / 2.0))
                midi.send(usb.pkt_note_off(0, a.note))
            threading.Thread(target=play, daemon=True).start()
        t0 = time.monotonic()
        p = subprocess.run(cmd, stderr=subprocess.PIPE, universal_newlines=True)
        wall = time.monotonic() - t0
        if midi:
            midi.send(usb.panic_packets([0]))
            midi.close()
        stats = dict(l.split("=", 1) for l in p.stderr.splitlines() if "=" in l)
        raw = open(tmp.name, "rb").read()
        os.unlink(tmp.name)
        if p.returncode != 0:
            print("isocap exit %d:\n%s" % (p.returncode, p.stderr), file=sys.stderr)
        print("helper stats:", ", ".join("%s=%s" % kv for kv in stats.items()), " wall=%.2fs" % wall)
        expect = int(wall * rate * ch * sub)
        print("bytes %d vs %d expected at nominal rate (%.0f%%)" %
              (len(raw), expect, 100.0 * len(raw) / max(1, expect)))
    wavcheck.write_wav(a.out, raw, ch, sub, rate)
    print("wrote", a.out, "- analysing...", flush=True)
    print(wavcheck.format_report(wavcheck.analyze_pcm(raw, ch, sub, rate)))
    return 0


def cmd_wavcheck(a):
    raw, ch, sub, rate = wavcheck.read_wav(a.file)
    print(wavcheck.format_report(wavcheck.analyze_pcm(raw, ch, sub, rate)))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    p = sub.add_parser("descriptors"); p.add_argument("--file"); p.add_argument("--save")
    p.set_defaults(fn=cmd_descriptors)
    p = sub.add_parser("note"); p.add_argument("n", type=int); p.add_argument("--vel", type=int, default=100)
    p.add_argument("--len", type=float, default=0.8); p.add_argument("--ch", type=int, default=1)
    p.set_defaults(fn=cmd_note)
    p = sub.add_parser("midi-in"); p.add_argument("--seconds", type=int, default=10)
    p.set_defaults(fn=cmd_midi_in)
    p = sub.add_parser("capture"); p.add_argument("seconds", type=float); p.add_argument("out")
    p.add_argument("--note", type=int, help="play this MIDI note on the FM-1 during capture")
    p.add_argument("--frames", type=int, default=16, help="packets per URB")
    p.add_argument("--urbs", type=int, default=8, help="URBs kept in flight")
    p.add_argument("--mock", action="store_true"); p.set_defaults(fn=cmd_capture)
    p = sub.add_parser("wavcheck"); p.add_argument("file"); p.set_defaults(fn=cmd_wavcheck)
    a = ap.parse_args(argv)
    return a.fn(a) or 0


if __name__ == "__main__":
    sys.exit(main())
