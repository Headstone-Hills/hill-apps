"""Shared render routine used by the CLI and the app."""
from .engine import Engine
from . import video_io


class Cancelled(Exception):
    pass


def render_video(src_path, out_path, cfg, fmt="h264", audio_path=None, audio_mod=None,
                 fps=None, progress=None, cancel=None):
    """Render src through the effect chain into out_path. Returns frames written.

    audio_path: audio muxed into the output (defaults to the source's own audio).
    audio_mod : AudioMod driving modulation.
    progress(i, total) is called every few frames; cancel() -> True aborts.
    """
    info = video_io.probe(src_path, fps)
    eng = Engine(cfg, info["fps"], audio_mod)
    src = video_io.Source(src_path, window=2 * eng.max_step + 4, fps=info["fps"])
    mux = audio_path or (src_path if info["has_audio"] else None)
    out = video_io.Writer(out_path, src.w, src.h, src.fps, fmt, audio_from=mux)
    i = 0
    try:
        while True:
            src.get(i)
            if i >= src.next:  # past the real end of the stream
                break
            if cancel and cancel():
                raise Cancelled()
            out.write(eng.render(i, src))
            i += 1
            if progress and i % 4 == 0:
                progress(i, info["frames"])
    finally:
        src.close()
        rc = out.close()
    if rc:
        raise RuntimeError(f"ffmpeg encoder exited with code {rc}")
    if progress:
        progress(i, i)
    return i
