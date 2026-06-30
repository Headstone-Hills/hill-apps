#!/usr/bin/env python3
"""Make loop-ready excerpts of oversized WAVs so they run on the 1GB device.

Scans INPUT_ROOT for WAVs that are too big to load safely (long duration or
large file), and writes a ~12s excerpt of each to OUTPUT_ROOT at the SAME
relative path + filename:
  * downsampled/converted to 44.1kHz 16-bit stereo (via SDL on load),
  * intro skipped (samples often fade in),
  * loop-crossfaded so it tiles seamlessly as a sustained drone.

Also writes OUTPUT_ROOT/_oversized.txt listing the original relative paths, so
the stale full-size versions can be removed from the device.

Usage:  python tools/make_excerpts.py <INPUT_ROOT> <OUTPUT_ROOT>
"""

import os
import sys
import wave

os.environ.setdefault("SDL_VIDEODRIVER", "dummy")
os.environ.setdefault("SDL_AUDIODRIVER", "dummy")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np
import pygame

import config
from audio import effects

# Thresholds: a WAV is "oversized" if it's long or a large file.
MAX_SECONDS = 20.0
MAX_MB = 25.0

# Excerpt shape.
SKIP_INTRO_S = 1.0
EXCERPT_S = 10.0          # forward length; ping-pong makes the loop ~2x this
RATE = 44100


def is_oversized(path):
    try:
        with wave.open(path) as w:
            dur = w.getnframes() / float(w.getframerate() or RATE)
    except Exception:
        return False
    size_mb = os.path.getsize(path) / 1e6
    return dur > MAX_SECONDS or size_mb > MAX_MB


def make_excerpt(path):
    """Load (SDL converts to 44.1k/16/stereo), trim, and make a ping-pong loop.

    Evolving drones don't loop when you just splice end->start (the content
    jumps). A palindrome (forward then reversed) loops with no jump: the two
    turnarounds are sample-adjacent, so it's seamless AND keeps moving."""
    snd = pygame.mixer.Sound(path)
    arr = pygame.sndarray.array(snd)
    if arr.ndim == 1:
        arr = np.stack([arr, arr], axis=1)
    n = len(arr)
    start = min(int(SKIP_INTRO_S * RATE), max(0, n - int(EXCERPT_S * RATE)))
    seg = arr[start:start + int(EXCERPT_S * RATE)].astype(np.int16)
    if len(seg) < RATE:                      # too short to bother trimming
        seg = arr.astype(np.int16)
    # forward + reverse-middle -> palindrome (no duplicated frame at the joins)
    pingpong = np.concatenate([seg, seg[-2:0:-1]], axis=0)
    return pingpong


def write_wav(path, pcm):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "w") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(np.ascontiguousarray(pcm).tobytes())


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    src_root, out_root = sys.argv[1], sys.argv[2]
    pygame.mixer.init(RATE, -16, 2, 1024)

    oversized = []
    for dirpath, _dirs, files in os.walk(src_root):
        for f in files:
            if not f.lower().endswith(".wav"):
                continue
            full = os.path.join(dirpath, f)
            if not is_oversized(full):
                continue
            rel = os.path.relpath(full, src_root)
            try:
                pcm = make_excerpt(full)
            except Exception as e:
                print(f"  FAILED {rel}: {e}")
                continue
            write_wav(os.path.join(out_root, rel), pcm)
            oversized.append(rel)
            print(f"  excerpt {rel}  ({len(pcm)/RATE:.1f}s, "
                  f"{os.path.getsize(os.path.join(out_root, rel))/1e6:.1f}MB)")

    with open(os.path.join(out_root, "_oversized.txt"), "w") as f:
        f.write("\n".join(oversized))
    print(f"\n{len(oversized)} oversized files excerpted -> {out_root}")


if __name__ == "__main__":
    main()
