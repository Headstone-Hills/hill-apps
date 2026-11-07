#!/usr/bin/env python3
"""fm1tool - verify the FM-1 hardware over SSH, no UI needed.

  fm1tool.py list
  fm1tool.py note 60 [--vel 100] [--len 0.5] [--ch 1]
  fm1tool.py pc 12 [--ch 1]
  fm1tool.py cc 74 64 [--ch 1]
  fm1tool.py panic
  fm1tool.py latency-test [--bpm 120] [--beats 16]
  fm1tool.py timing [--bpm 160] [--seconds 10]     # clock-thread jitter, 16th notes

Add --mock to run without hardware (prints the MIDI instead).
"""

from __future__ import annotations

import argparse
import sys
import time

import fm1_usb as usb


def _open(args):
    t = usb.ConsoleTransport() if args.mock else usb.UsbTransport()
    t.start()
    if not t.connected:
        print("FM-1 not found (looking for %04x:%04x). Is it plugged into the USB-C port?" %
              (usb.FM1_VID, usb.FM1_PID), file=sys.stderr)
        t.close()
        sys.exit(2)
    return t


def cmd_list(args):
    devs = usb.find_devices()
    if not devs:
        print("no FM-1 (%04x:%04x) found" % (usb.FM1_VID, usb.FM1_PID))
        return 1
    for d in devs:
        print("%s  %s  product=%r  MIDI interface=%d  bulk-OUT=0x%02x" %
              (d.sysname, d.path, d.product, d.iface, d.ep_out))
    return 0


def cmd_note(args):
    t = _open(args)
    ch = args.ch - 1
    try:
        t.send(usb.pkt_note_on(ch, args.n, args.vel))
        time.sleep(args.len)
    finally:
        t.send(usb.pkt_note_off(ch, args.n))
        t.close()
    print("sent note %d on ch%d (vel %d, %.2fs)" % (args.n, args.ch, args.vel, args.len))


def cmd_pc(args):
    t = _open(args)
    ok = t.send(usb.pkt_pc(args.ch - 1, args.n))
    t.close()
    print("program change %d on ch%d: %s" % (args.n, args.ch, "sent" if ok else "FAILED"))


def cmd_cc(args):
    t = _open(args)
    ok = t.send(usb.pkt_cc(args.ch - 1, args.n, args.v))
    t.close()
    print("CC %d = %d on ch%d: %s" % (args.n, args.v, args.ch, "sent" if ok else "FAILED"))


def cmd_panic(args):
    t = _open(args)
    ok = t.send(usb.panic_packets())
    for ch in range(16):
        t.send(usb.pkt_pitchbend(ch, 8192))
    t.close()
    print("panic (CC123+CC120 on all channels): %s" % ("sent" if ok else "FAILED"))


def cmd_latency(args):
    """Measures what software CAN measure (USB write time + wake-up error) and plays a
    steady click so YOU can judge audible latency against a reference."""
    t = _open(args)
    period = 60.0 / args.bpm
    writes, errs = [], []
    print("Playing %d quarter-note clicks (note 72) at %d BPM." % (args.beats, args.bpm))
    print("LISTEN: start a metronome at the same BPM on a phone and line up the first")
    print("click; if the FM-1 trails it by ~X ms, set Project > LATENCY to X.")
    t0 = time.monotonic_ns()
    try:
        for i in range(args.beats):
            due = t0 + int(i * period * 1e9)
            while True:
                rem = due - time.monotonic_ns()
                if rem <= 1_500_000:
                    break
                time.sleep((rem - 1_500_000) / 1e9)
            while time.monotonic_ns() < due:
                pass
            errs.append((time.monotonic_ns() - due) // 1000)
            w0 = time.monotonic_ns()
            t.send(usb.pkt_note_on(0, 72 if i % 4 else 84, 100))
            writes.append((time.monotonic_ns() - w0) // 1000)
            time.sleep(0.05)
            t.send(usb.pkt_note_off(0, 72 if i % 4 else 84))
    finally:
        t.send(usb.panic_packets([0]))
        t.close()
    print("USB bulk write: min %d / avg %d / max %d us" % (min(writes), sum(writes) // len(writes), max(writes)))
    print("wake-up error : min %d / avg %d / max %d us" % (min(errs), sum(errs) // len(errs), max(errs)))


def cmd_timing(args):
    """Hardcoded 16-step loop through the real engine + clock thread; prints jitter."""
    from model import Project
    from engine import Engine
    p = Project()
    p.bpm = args.bpm
    ph = p.phrase(0)
    for i in range(16):
        ph.steps[i].note = 60 if i % 4 == 0 else 72
        ph.steps[i].vel = 110 if i % 4 == 0 else 70
    p.chain(0).phrase[0] = 0
    p.song[0][0] = 0
    t = usb.MockTransport() if args.mock else _open(args)
    eng = Engine(t, p)
    eng.start_thread()
    print("16th notes at %d BPM for %ds on %s ..." % (args.bpm, args.seconds, t.name))
    eng.play_song(0)
    time.sleep(args.seconds)
    eng.shutdown()
    s = eng.stats
    print("ticks=%d  send error us: min %s avg %.0f max %d  late(>1ms)=%d  usb write max %d us" %
          (s.n, s.min, s.avg, s.max, s.late, s.write_us_max))
    t.close()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mock", action="store_true", help="no hardware; print MIDI to stdout")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    p = sub.add_parser("note"); p.add_argument("n", type=int)
    p.add_argument("--vel", type=int, default=100); p.add_argument("--len", type=float, default=0.8)
    p.add_argument("--ch", type=int, default=1); p.set_defaults(fn=cmd_note)
    p = sub.add_parser("pc"); p.add_argument("n", type=int); p.add_argument("--ch", type=int, default=1)
    p.set_defaults(fn=cmd_pc)
    p = sub.add_parser("cc"); p.add_argument("n", type=int); p.add_argument("v", type=int)
    p.add_argument("--ch", type=int, default=1); p.set_defaults(fn=cmd_cc)
    sub.add_parser("panic").set_defaults(fn=cmd_panic)
    p = sub.add_parser("latency-test"); p.add_argument("--bpm", type=int, default=120)
    p.add_argument("--beats", type=int, default=16); p.set_defaults(fn=cmd_latency)
    p = sub.add_parser("timing"); p.add_argument("--bpm", type=int, default=160)
    p.add_argument("--seconds", type=int, default=10); p.set_defaults(fn=cmd_timing)
    args = ap.parse_args(argv)
    return args.fn(args) or 0


if __name__ == "__main__":
    sys.exit(main())
