"""Deterministic modulation sources.

Everything is a pure function of time (and seed) so random modulation is
repeatable and renders identically no matter which frame a host asks for.
All sources return a value in [0, 1]; routing maps it to a parameter.
"""
import json
import math
import numpy as np

_MASK = (1 << 64) - 1


def hash01(n: int, seed: int) -> float:
    """splitmix64-style integer hash -> [0, 1)."""
    z = (n * 0x9E3779B97F4A7C15 + seed * 0xD1B54A32D192ED03 + 0x632BE59BD9B4E019) & _MASK
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & _MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & _MASK
    z ^= z >> 31
    return (z >> 11) / float(1 << 53)


def _smooth(x):
    return x * x * (3 - 2 * x)


class AudioMod:
    """Per-video-frame audio envelopes (see hillview.audio.analyze)."""

    def __init__(self, fps, channels):
        self.fps = float(fps)
        self.channels = {k: np.asarray(v, np.float32) for k, v in channels.items()}

    @classmethod
    def load(cls, path):
        with open(path) as f:
            d = json.load(f)
        return cls(d["fps"], d["channels"])

    def save(self, path):
        with open(path, "w") as f:
            json.dump({"version": 1, "fps": self.fps,
                       "channels": {k: [round(float(x), 4) for x in v]
                                    for k, v in self.channels.items()}}, f)

    def sample(self, channel, t):
        a = self.channels.get(channel)
        if a is None or len(a) == 0:
            return 0.0
        x = min(max(t * self.fps, 0.0), len(a) - 1)
        i = int(x)
        j = min(i + 1, len(a) - 1)
        return float(a[i] + (a[j] - a[i]) * (x - i))


def source_value(src, t, audio=None, global_seed=0):
    """Evaluate one modulation source at time t (seconds) -> [0,1]."""
    typ = src["type"]
    seed = int(src.get("seed", 0)) + int(global_seed)
    if typ == "lfo":
        hz, ph = src.get("hz", 1.0), src.get("phase", 0.0)
        p = (t * hz + ph) % 1.0
        shape = src.get("shape", "sine")
        if shape == "sine":
            return 0.5 - 0.5 * math.cos(2 * math.pi * p)
        if shape == "saw":
            return p
        if shape == "tri":
            return 1 - abs(2 * p - 1)
        if shape == "square":
            return 1.0 if p < src.get("duty", 0.5) else 0.0
        raise ValueError(f"unknown lfo shape {shape!r}")
    if typ == "noise":  # smooth value noise
        x = t * src.get("hz", 1.0)
        i = math.floor(x)
        return hash01(i, seed) * (1 - _smooth(x - i)) + hash01(i + 1, seed) * _smooth(x - i)
    if typ == "steps":  # sample & hold random
        return hash01(math.floor(t * src.get("hz", 4.0)), seed)
    if typ == "audio":
        return audio.sample(src.get("channel", "rms"), t) if audio else 0.0
    raise ValueError(f"unknown modulation source type {typ!r}")
