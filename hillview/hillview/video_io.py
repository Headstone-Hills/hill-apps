"""ffmpeg-backed import/export.

Import: anything ffmpeg can demux/decode (hundreds of containers/codecs), plus image
sequences (`frame_%04d.png`) and single images. Frames are conformed to constant frame
rate and display rotation, and handed to the engine as 8-bit RGB.
Export: see FORMATS; only formats whose encoders exist in the local ffmpeg are offered.
"""
import functools
import json
import os
import re
import subprocess
from collections import OrderedDict
import numpy as np

DEFAULT_SEQ_FPS = 25.0


def is_sequence(path):
    return bool(re.search(r"%0?\d*d", path))


def _frac(s):
    try:
        n, d = s.split("/")
        return float(n) / float(d) if float(d) else 0.0
    except (ValueError, AttributeError):
        return 0.0


def probe(path, fps=None):
    """-> dict(width, height, fps, frames, duration, has_audio, codec, container)."""
    pre = ["-framerate", str(fps or DEFAULT_SEQ_FPS)] if is_sequence(path) else []
    r = subprocess.run(["ffprobe", "-v", "error", *pre, "-show_streams", "-show_format",
                        "-of", "json", path], capture_output=True, text=True)
    if r.returncode:
        raise ValueError(f"cannot read {os.path.basename(path)}: {r.stderr.strip() or 'unsupported file'}")
    d = json.loads(r.stdout)
    vs = next((s for s in d["streams"] if s["codec_type"] == "video"
               and s.get("disposition", {}).get("attached_pic", 0) == 0), None) \
        or next((s for s in d["streams"] if s["codec_type"] == "video"), None)
    if vs is None:
        raise ValueError(f"{os.path.basename(path)} has no video stream")
    w, h = int(vs["width"]), int(vs["height"])
    rot = vs.get("tags", {}).get("rotate")
    for sd in vs.get("side_data_list", []):
        rot = sd.get("rotation", rot)
    if rot is not None and abs(int(float(rot))) % 180 == 90:
        w, h = h, w
    f = fps or _frac(vs.get("avg_frame_rate", "")) or _frac(vs.get("r_frame_rate", "")) or DEFAULT_SEQ_FPS
    dur = float(vs.get("duration") or d["format"].get("duration") or 0)
    nb = vs.get("nb_frames")
    frames = int(nb) if nb and nb.isdigit() and int(nb) > 0 else max(1, round(dur * f))
    return {"width": w - w % 2, "height": h - h % 2, "fps": f, "frames": frames,
            "duration": frames / f, "has_audio": any(s["codec_type"] == "audio" for s in d["streams"]),
            "codec": vs.get("codec_name"), "container": d["format"].get("format_name")}


def _fit(w, h):
    """Even dims; the source frame is cropped by 1px if odd so encoders accept it."""
    return w - w % 2, h - h % 2


class Source:
    """Forward-streaming frame source with a sliding window cache.
    get(n) clamps to the last frame at EOF."""

    def __init__(self, path, window, fps=None, scale_to=None):
        self.info = probe(path, fps)
        self.w, self.h, self.fps = self.info["width"], self.info["height"], self.info["fps"]
        self.ow, self.oh = (scale_to or (self.w, self.h))
        pre = ["-framerate", str(self.fps)] if is_sequence(path) else []
        vf = f"fps={self.fps},crop={self.w}:{self.h}:0:0,scale={self.ow}:{self.oh}"
        self.p = subprocess.Popen(["ffmpeg", "-v", "error", *pre, "-i", path, "-map", "0:v:0", "-vf", vf,
                                   "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.nbytes, self.next, self.window = self.ow * self.oh * 3, 0, window
        self.cache, self.last, self.eof = OrderedDict(), None, False   # cache holds uint8 frames

    def _fill(self, n):
        while self.next <= n and not self.eof:
            buf = self.p.stdout.read(self.nbytes)
            if len(buf) < self.nbytes:
                self.eof = True
                break
            self.last = np.frombuffer(buf, np.uint8).reshape(self.oh, self.ow, 3)
            self.cache[self.next] = self.last
            self.next += 1
            while len(self.cache) > self.window:
                self.cache.popitem(last=False)

    def ensure(self, n):
        """True if frame n exists (decoding forward as needed)."""
        self._fill(n)
        return n < self.next

    def get(self, n):
        """Frame n as float32 RGB in [0,1]; clamps to the last frame at EOF."""
        self._fill(n)
        if n in self.cache:
            f = self.cache[n]
        elif self.eof and n >= self.next:
            f = self.last
        else:
            raise KeyError(f"frame {n} fell out of the {self.window}-frame window")
        return f.astype(np.float32) * np.float32(1 / 255)

    def get_u8(self, n):
        """Frame n as uint8 (no conversion); clamps to the last frame at EOF."""
        self._fill(n)
        if n in self.cache:
            return self.cache[n]
        if self.eof and n >= self.next:
            return self.last
        raise KeyError(f"frame {n} fell out of the {self.window}-frame window")

    @property
    def exhausted(self):
        return self.eof

    def close(self):
        try:
            self.p.kill()
        except OSError:
            pass
        self.p.stdout.close()
        self.p.wait()


class RandomSource:
    """Random-access (seeking) source for interactive preview, with optional downscale."""

    def __init__(self, path, info, scale_to=None, max_cache=64):
        self.path, self.info = path, info
        self.ow, self.oh = scale_to or (info["width"], info["height"])
        self.cache, self.max_cache = OrderedDict(), max_cache

    def get(self, n):
        n = max(0, min(n, self.info["frames"] - 1))
        if n in self.cache:
            return self.cache[n]
        i = self.info
        pre = ["-framerate", str(i["fps"])] if is_sequence(self.path) else []
        vf = f"crop={i['width']}:{i['height']}:0:0,scale={self.ow}:{self.oh}"
        r = subprocess.run(["ffmpeg", "-v", "error", "-ss", f"{n / i['fps']:.6f}", *pre, "-i", self.path,
                            "-map", "0:v:0", "-frames:v", "1", "-vf", vf, "-f", "rawvideo",
                            "-pix_fmt", "rgb24", "-"], capture_output=True)
        if len(r.stdout) < self.ow * self.oh * 3:  # past the end / seek miss: use last good frame
            return self.cache[max(self.cache)] if self.cache else np.zeros((self.oh, self.ow, 3), np.float32)
        img = np.frombuffer(r.stdout, np.uint8)[:self.ow * self.oh * 3].reshape(self.oh, self.ow, 3)
        self.cache[n] = img.astype(np.float32) / 255
        while len(self.cache) > self.max_cache:
            self.cache.popitem(last=False)
        return self.cache[n]


# ---------------------------------------------------------------- export formats
# id: (label, extension, video args, pix_fmt, audio args or None, kind)
FORMATS = {
    "h264":       ("H.264 / MP4",              "mp4",  ["-c:v", "libx264", "-crf", "16", "-preset", "medium"], "yuv420p", ["-c:a", "aac", "-b:a", "256k"], "video"),
    "h265":       ("H.265 (HEVC) / MP4",       "mp4",  ["-c:v", "libx265", "-crf", "20", "-tag:v", "hvc1"], "yuv420p", ["-c:a", "aac", "-b:a", "256k"], "video"),
    "av1":        ("AV1 / MP4",                "mp4",  ["-c:v", "libsvtav1", "-crf", "28"], "yuv420p", ["-c:a", "aac", "-b:a", "256k"], "video"),
    "vp9":        ("VP9 / WebM",               "webm", ["-c:v", "libvpx-vp9", "-crf", "30", "-b:v", "0"], "yuv420p", ["-c:a", "libopus", "-b:a", "160k"], "video"),
    "vp8":        ("VP8 / WebM",               "webm", ["-c:v", "libvpx", "-crf", "10", "-b:v", "4M"], "yuv420p", ["-c:a", "libopus", "-b:a", "160k"], "video"),
    "prores_422": ("ProRes 422 HQ / MOV",      "mov",  ["-c:v", "prores_ks", "-profile:v", "3"], "yuv422p10le", ["-c:a", "pcm_s16le"], "video"),
    "prores_4444":("ProRes 4444 / MOV",        "mov",  ["-c:v", "prores_ks", "-profile:v", "4"], "yuv444p10le", ["-c:a", "pcm_s16le"], "video"),
    "dnxhr_hq":   ("DNxHR HQ / MOV",           "mov",  ["-c:v", "dnxhd", "-profile:v", "dnxhr_hq"], "yuv422p", ["-c:a", "pcm_s16le"], "video"),
    "mjpeg":      ("Motion JPEG / MOV",        "mov",  ["-c:v", "mjpeg", "-q:v", "2"], "yuvj422p", ["-c:a", "pcm_s16le"], "video"),
    "ffv1":       ("FFV1 lossless / MKV",      "mkv",  ["-c:v", "ffv1", "-level", "3"], "yuv444p", ["-c:a", "flac"], "video"),
    "mpeg4":      ("MPEG-4 Part 2 / AVI",      "avi",  ["-c:v", "mpeg4", "-q:v", "2"], "yuv420p", ["-c:a", "libmp3lame", "-b:a", "192k"], "video"),
    "gif":        ("Animated GIF",             "gif",  ["-loop", "0"], None, None, "gif"),
    "webp":       ("Animated WebP",            "webp", ["-c:v", "libwebp_anim", "-loop", "0", "-q:v", "75"], "yuv420p", None, "video"),
    "png_seq":    ("PNG image sequence",       "png",  ["-c:v", "png"], "rgb24", None, "seq"),
    "tiff_seq":   ("TIFF image sequence",      "tiff", ["-c:v", "tiff"], "rgb24", None, "seq"),
    "jpg_seq":    ("JPEG image sequence",      "jpg",  ["-c:v", "mjpeg", "-q:v", "2"], "yuvj420p", None, "seq"),
}
MIN_SIZE = {"dnxhr_hq": (256, 120)}  # encoder limits; ffmpeg's own error for these is cryptic


def check_size(fmt, w, h):
    mw, mh = MIN_SIZE.get(fmt, (2, 2))
    if w < mw or h < mh:
        raise ValueError(f"{FORMATS[fmt][0]} needs at least {mw}x{mh} pixels; this video is {w}x{h}. "
                         "Pick another format.")


@functools.lru_cache(maxsize=1)
def _encoders():
    r = subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True)
    return {m.group(1) for m in re.finditer(r"^\s[VAS][\w.]{5}\s+(\S+)", r.stdout, re.M)}


def available_formats():
    """{id: {label, ext, kind}} for formats the installed ffmpeg can encode."""
    enc, out = _encoders(), {}
    for k, (label, ext, vargs, _pf, aargs, kind) in FORMATS.items():
        v = vargs[vargs.index("-c:v") + 1] if "-c:v" in vargs else "gif"
        if v in enc or (kind == "gif" and "gif" in enc):
            if aargs is None or aargs[1] in enc:
                out[k] = {"label": label, "ext": ext, "kind": kind}
            elif aargs is not None:  # video OK but audio codec missing: export silent
                out[k] = {"label": label + " (no audio: encoder missing)", "ext": ext, "kind": kind}
    return out


def output_path(path, fmt):
    """Resolve the real output target: a directory pattern for image sequences."""
    kind = FORMATS[fmt][5]
    if kind == "seq":
        os.makedirs(path, exist_ok=True)  # for image sequences `path` is the output directory
        return os.path.join(path, f"frame_%05d.{FORMATS[fmt][1]}")
    return path


class Writer:
    def __init__(self, path, w, h, fps, fmt="h264", audio_from=None):
        if fmt not in FORMATS:
            raise ValueError(f"unknown format {fmt!r}; choose from {sorted(FORMATS)}")
        _label, _ext, vargs, pix, aargs, kind = FORMATS[fmt]
        check_size(fmt, w, h)
        has_audio = bool(audio_from) and aargs is not None and aargs[1] in _encoders()
        cmd = ["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
               "-s", f"{w}x{h}", "-r", str(fps), "-i", "-"]
        if has_audio:
            cmd += ["-i", audio_from, "-map", "0:v", "-map", "1:a:0?"] + aargs + ["-shortest"]
        if kind == "gif":
            cmd += ["-vf", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer"] + vargs
        else:
            cmd += vargs + ["-pix_fmt", pix]
        self.p = subprocess.Popen(cmd + [output_path(path, fmt)], stdin=subprocess.PIPE)

    def write(self, img):
        """img: uint8 (H,W,3) written as-is, or float RGB in [0,1]."""
        if img.dtype != np.uint8:
            img = (np.clip(img, 0, 1) * 255 + 0.5).astype(np.uint8)
        self.p.stdin.write(np.ascontiguousarray(img).data)

    def close(self):
        try:
            self.p.stdin.close()
        except BrokenPipeError:
            pass
        return self.p.wait()
