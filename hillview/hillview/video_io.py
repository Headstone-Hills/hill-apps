"""ffmpeg-backed video reader/writer for the CLI (hosts supply their own frames)."""
import json
import subprocess
from collections import OrderedDict
import numpy as np


def probe(path):
    r = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                        "stream=width,height,r_frame_rate,nb_frames", "-of", "json", path],
                       capture_output=True, check=True, text=True)
    s = json.loads(r.stdout)["streams"][0]
    n, d = s["r_frame_rate"].split("/")
    return int(s["width"]), int(s["height"]), float(n) / float(d)


class Source:
    """Forward-streaming frame source with a sliding window cache.
    get(n) clamps to the last frame at EOF."""

    def __init__(self, path, window):
        self.w, self.h, self.fps = probe(path)
        self.p = subprocess.Popen(["ffmpeg", "-v", "error", "-i", path, "-f", "rawvideo",
                                   "-pix_fmt", "rgb24", "-"], stdout=subprocess.PIPE)
        self.nbytes, self.next, self.window = self.w * self.h * 3, 0, window
        self.cache, self.last, self.eof = OrderedDict(), None, False

    def get(self, n):
        while self.next <= n and not self.eof:
            buf = self.p.stdout.read(self.nbytes)
            if len(buf) < self.nbytes:
                self.eof = True
                break
            self.last = np.frombuffer(buf, np.uint8).reshape(self.h, self.w, 3).astype(np.float32) / 255
            self.cache[self.next] = self.last
            self.next += 1
            while len(self.cache) > self.window:
                self.cache.popitem(last=False)
        if n in self.cache:
            return self.cache[n]
        if self.eof and n >= self.next:
            return self.last
        raise KeyError(f"frame {n} fell out of the {self.window}-frame window")

    @property
    def exhausted(self):
        return self.eof


class Writer:
    def __init__(self, path, w, h, fps, audio_from=None, crf=16):
        cmd = ["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
               "-s", f"{w}x{h}", "-r", str(fps), "-i", "-"]
        if audio_from:
            cmd += ["-i", audio_from, "-map", "0:v", "-map", "1:a?", "-c:a", "aac", "-shortest"]
        cmd += ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", str(crf), path]
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE)

    def write(self, img):
        self.p.stdin.write((np.clip(img, 0, 1) * 255 + 0.5).astype(np.uint8).tobytes())

    def close(self):
        self.p.stdin.close()
        self.p.wait()
