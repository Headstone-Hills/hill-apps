#!/usr/bin/env python3
"""Output test for the SP: plays a click + bar tone (+ optional WAV) and counts underruns.

  python3 audio_test.py aplay  [--seconds 20] [--bpm 120] [--buffer-ms 100] [--device hw:0,0] [--wav f.wav]
  python3 audio_test.py pygame [--seconds 20]
  python3 audio_test.py file out.wav [--seconds 8]      # offline render (no sound card needed)

LISTEN FOR: a steady click at --bpm; the first click of every 4 is higher and louder, and exactly
on it a soft low tone (220 Hz) starts and fades. The click and tone must stay locked together for the
whole run. Crackles/gaps = underruns (also counted below).
"""
import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mixer import Mixer, Click, BarTone, ClipSource, RATE          # noqa: E402
import audio_out                                                   # noqa: E402


def build_mixer(a):
    m = Mixer(RATE)
    m.add(Click(a.bpm, 4, amp=0.5))
    m.add(BarTone(a.bpm, 4, 220.0, amp=0.35))
    if a.wav:
        m.add(ClipSource.from_wav(a.wav, RATE, loop=True, gain=0.5))
    return m


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("backend", choices=["aplay", "pygame", "file"])
    ap.add_argument("out", nargs="?")
    ap.add_argument("--seconds", type=float, default=20)
    ap.add_argument("--bpm", type=float, default=120)
    ap.add_argument("--period", type=int, default=1024, help="frames per chunk")
    ap.add_argument("--buffer-ms", type=int, default=100)
    ap.add_argument("--device")
    ap.add_argument("--wav")
    a = ap.parse_args(argv)
    m = build_mixer(a)
    if a.backend == "file":
        out = audio_out.FileOutput(m, a.out or "audio_test.wav", int(a.seconds * RATE), period_frames=a.period)
        out.start(); out._thread.join()
        print("wrote", out.path, out.stats()); return 0
    if a.backend == "aplay":
        out = audio_out.AplayOutput(m, period_frames=a.period, buffer_ms=a.buffer_ms, device=a.device)
    else:
        out = audio_out.PygameQueueOutput(m, period_frames=a.period)
    print("backend=%s  period=%d frames (%.1f ms)  est. latency ~%.0f ms  bpm=%g  %gs" % (
        a.backend, a.period, 1000.0 * a.period / RATE, out.latency_ms(), a.bpm, a.seconds))
    print("LISTEN: steady click, accented every 4th with a soft low tone locked to it. Starting...")
    out.start()
    t0 = time.time()
    try:
        while time.time() - t0 < a.seconds:
            time.sleep(1.0)
            st = out.stats()
            print("t=%4.1fs  frame=%d (%.2fs audio)  underruns=%d late=%d render avg %.2f max %.2f ms  write max %.1f ms%s" % (
                time.time() - t0, st["frame"], st["frame"] / float(RATE), st["underruns"], st["late"],
                st["render_ms_avg"], st["render_ms_max"], st["write_ms_max"],
                ("  ERR " + st["errors"][-1]) if st["errors"] else ""), flush=True)
    except KeyboardInterrupt:
        pass
    finally:
        out.stop()
    st = out.stats()
    drift = (st["frame"] / float(RATE)) - (time.time() - t0)
    print("SUMMARY: underruns=%d late=%d  render max %.2f ms of %.1f ms period  audio-vs-wall drift %+.2fs" % (
        st["underruns"], st["late"], st["render_max_ms"] if "render_max_ms" in st else st["render_ms_max"],
        st["period_ms"], drift))
    print("RESULT:", "CLEAN" if st["underruns"] == 0 and not st["errors"] else "PROBLEMS (see above)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
