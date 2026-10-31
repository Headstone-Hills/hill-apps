import argparse
import json
import os
import sys
from .modulation import AudioMod
from . import audio as audio_mod, native, video_io
from .render import render_video


def cmd_analyze(a):
    fps = a.fps or video_io.probe(a.input)["fps"]
    m = audio_mod.analyze(audio_mod.decode(a.input), fps)
    m.save(a.output)
    print(f"wrote {a.output}: {len(m.channels['rms'])} frames @ {fps:g} fps, channels {sorted(m.channels)}")


def cmd_formats(_a):
    for k, v in video_io.available_formats().items():
        print(f"{k:12s} .{v['ext']:5s} {v['label']}")


def cmd_render(a):
    cfg = json.load(open(a.config)) if a.config else {}
    fmt = a.format
    if not fmt:  # infer from extension
        ext = os.path.splitext(a.output)[1].lstrip(".").lower()
        fmt = next((k for k, v in video_io.available_formats().items() if v["ext"] == ext and v["kind"] != "seq"), "h264")
    print(native.describe(), file=sys.stderr)
    info = video_io.probe(a.input, a.fps)
    am = None
    if a.audio_mod:
        am = AudioMod.load(a.audio_mod)
    elif a.audio_reactive or a.audio:
        am = audio_mod.analyze(audio_mod.decode(a.audio or a.input), info["fps"], n_frames=info["frames"])
    n = render_video(a.input, a.output, cfg, fmt, audio_path=a.audio, audio_mod=am, fps=a.fps, workers=a.workers, speed=a.speed,
                     progress=lambda i, t: print(f"\r{i}/{t} frames", end="", file=sys.stderr))
    print(f"\nrendered {n} frames ({fmt}) -> {a.output}")


def cmd_app(a):
    from .app import serve
    serve(a.port, open_browser=not a.no_browser)


def main(argv=None):
    p = argparse.ArgumentParser(prog="hillview")
    s = p.add_subparsers(required=True)
    r = s.add_parser("render", help="render a video through the effect chain")
    r.add_argument("input", help="any ffmpeg-readable video/image, or a sequence pattern like shot_%%04d.png")
    r.add_argument("output")
    r.add_argument("-c", "--config", help="JSON config (see presets/)")
    r.add_argument("-f", "--format", help="export format id (see `formats`); default: from output extension")
    r.add_argument("--fps", type=float, help="override frame rate (needed for image sequences; default 25)")
    r.add_argument("--audio", help="separate audio file: drives modulation and is muxed into the output")
    r.add_argument("--workers", type=int, help="parallel frame threads (default: CPU count)")
    r.add_argument("--speed", choices=["fast", "balanced", "quality"], help="x264/x265 encoder preset: veryfast / medium / slow")
    r.add_argument("--audio-mod", help="precomputed .json from `analyze`")
    r.add_argument("--audio-reactive", action="store_true", help="analyze the input's own audio")
    r.set_defaults(fn=cmd_render)
    n = s.add_parser("analyze", help="audio/video file -> modulation envelope JSON")
    n.add_argument("input"); n.add_argument("-o", "--output", required=True)
    n.add_argument("--fps", type=float, help="video frame rate (default: probe input)")
    n.set_defaults(fn=cmd_analyze)
    s.add_parser("formats", help="list export formats this machine can encode").set_defaults(fn=cmd_formats)
    g = s.add_parser("app", help="launch the standalone app (local web UI)")
    g.add_argument("--port", type=int, default=8765)
    g.add_argument("--no-browser", action="store_true")
    g.set_defaults(fn=cmd_app)
    a = p.parse_args(argv)
    a.fn(a)
