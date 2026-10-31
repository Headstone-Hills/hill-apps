import argparse
import json
import sys
from .engine import Engine
from .modulation import AudioMod
from . import audio as audio_mod, video_io


def cmd_analyze(a):
    _, _, fps = video_io.probe(a.input) if a.fps is None else (0, 0, a.fps)
    m = audio_mod.analyze(audio_mod.decode(a.input), fps)
    m.save(a.output)
    print(f"wrote {a.output}: {len(m.channels['rms'])} frames @ {fps:g} fps, channels {sorted(m.channels)}")


def cmd_render(a):
    cfg = json.load(open(a.config)) if a.config else {}
    w, h, fps = video_io.probe(a.input)
    am = None
    if a.audio_mod:
        am = AudioMod.load(a.audio_mod)
    elif a.audio_reactive:
        am = audio_mod.analyze(audio_mod.decode(a.input), fps)
    eng = Engine(cfg, fps, am)
    src = video_io.Source(a.input, window=2 * eng.max_step + 4)
    out = video_io.Writer(a.output, w, h, fps, audio_from=a.input)
    i = 0
    while True:
        src.get(i)
        if i >= src.next:  # past the real end of the stream
            break
        out.write(eng.render(i, src))
        i += 1
        if i % 30 == 0:
            print(f"\r{i} frames", end="", file=sys.stderr)
    out.close()
    print(f"\nrendered {i} frames -> {a.output}")


def main(argv=None):
    p = argparse.ArgumentParser(prog="hillview")
    s = p.add_subparsers(required=True)
    r = s.add_parser("render", help="render a video through the effect chain")
    r.add_argument("input"); r.add_argument("output")
    r.add_argument("-c", "--config", help="JSON config (see presets/)")
    r.add_argument("--audio-mod", help="precomputed .json from `analyze`")
    r.add_argument("--audio-reactive", action="store_true", help="analyze the input's own audio")
    r.set_defaults(fn=cmd_render)
    n = s.add_parser("analyze", help="audio/video file -> modulation envelope JSON")
    n.add_argument("input"); n.add_argument("-o", "--output", required=True)
    n.add_argument("--fps", type=float, help="video frame rate (default: probe input)")
    n.set_defaults(fn=cmd_analyze)
    a = p.parse_args(argv)
    a.fn(a)
