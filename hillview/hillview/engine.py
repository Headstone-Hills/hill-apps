"""HillView engine: config + time -> frame."""
import copy
import math
import numpy as np
from . import effects
from .modulation import source_value

DEFAULTS = {
    "seed": 0,
    "quantize": {"enabled": False, "mode": "posterize", "levels": 5, "colors": 6},
    "smooth": {"sigma": 0.0},
    "remap": {"rules": [], "threshold": 0.0, "amount": 1.0, "hue_shift": 0.0},
    "blur": {"sigma": 0.0},
    "sampling": {"step": 1, "interp": "blend"},   # interp: blend | hold
    "modulation": [],
}

# target path -> (min, max) clamp applied after modulation
TARGETS = {
    "quantize.levels": (2, 32), "quantize.colors": (2, 32),
    "smooth.sigma": (0, 64), "blur.sigma": (0, 64),
    "remap.threshold": (-1, 1), "remap.amount": (0, 1), "remap.hue_shift": (-2 * math.pi, 2 * math.pi),
    "sampling.step": (1, 64),
}
BIPOLAR_DEFAULT = {"lfo": True, "noise": True, "steps": True, "audio": False}


def _merge(base, over):
    out = copy.deepcopy(base)
    for k, v in (over or {}).items():
        out[k] = _merge(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out


class Engine:
    def __init__(self, config=None, fps=30.0, audio=None):
        self.cfg = _merge(DEFAULTS, config)
        self.fps, self.audio = float(fps), audio
        for m in self.cfg["modulation"]:
            if m["target"] not in TARGETS:
                raise ValueError(f"unknown modulation target {m['target']!r}; valid: {sorted(TARGETS)}")
        self._cache = {}

    @property
    def max_step(self):
        s = self.cfg["sampling"]["step"]
        hi = s
        for m in self.cfg["modulation"]:
            if m["target"] == "sampling.step":
                hi = max(hi, s + abs(m.get("depth", 1.0)))
        return int(min(math.ceil(hi), TARGETS["sampling.step"][1]))

    def params_at(self, t):
        """Resolve all parameters (base + modulation) at time t seconds."""
        vals = {tg: self._base(tg) for tg in TARGETS}
        for m in self.cfg["modulation"]:
            src = m["source"]
            v = source_value(src, t, self.audio, self.cfg["seed"])
            if src.get("bipolar", BIPOLAR_DEFAULT[src["type"]]):
                v = v * 2 - 1
            vals[m["target"]] += m.get("depth", 1.0) * v
        return {k: min(max(v, TARGETS[k][0]), TARGETS[k][1]) for k, v in vals.items()}

    def _base(self, path):
        a, b = path.split(".")
        return float(self.cfg[a][b])

    def process_key(self, img, t, key_id=0):
        """Full effect chain on one source frame at time t."""
        p, c = self.params_at(t), self.cfg
        out = img
        if c["quantize"]["enabled"]:
            if c["quantize"]["mode"] == "kmeans":
                out = effects.kmeans_quantize(out, p["quantize.colors"], seed=c["seed"])
            else:
                out = effects.posterize(out, p["quantize.levels"])
        out = effects.blur(out, p["smooth.sigma"])
        out = effects.remap_colors(out, c["remap"]["rules"], p["remap.threshold"],
                                   p["remap.amount"], p["remap.hue_shift"])
        out = effects.blur(out, p["blur.sigma"])
        return np.clip(out, 0, 1).astype(np.float32)

    def _key(self, source, k):
        if k not in self._cache:
            self._cache[k] = self.process_key(source.get(k), k / self.fps, k)
            for old in [x for x in self._cache if x < k - 2 * self.max_step - 2]:
                del self._cache[old]
        return self._cache[k]

    def render(self, i, source):
        """Render output frame i. `source.get(n)` returns source frame n (float32 RGB)."""
        step = max(1, int(round(self.params_at(i / self.fps)["sampling.step"])))
        k0 = (i // step) * step
        a = self._key(source, k0)
        if step == 1 or i == k0 or self.cfg["sampling"]["interp"] == "hold":
            return a
        f = (i - k0) / step
        return a * (1 - f) + self._key(source, k0 + step) * f
