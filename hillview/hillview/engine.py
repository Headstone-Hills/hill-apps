"""HillView engine: config + time -> frame."""
import copy
import math
import multiprocessing
import os
from concurrent.futures import ProcessPoolExecutor, ThreadPoolExecutor
import numpy as np
from . import effects, native
from .color import rgb_to_oklab, oklab_to_rgb, parse_color, rotate_hue_lab
from .modulation import source_value

DEFAULTS = {
    "seed": 0,
    "quantize": {"enabled": False, "mode": "posterize", "levels": 5, "colors": 6},
    "smooth": {"sigma": 0.0},
    "remap": {"rules": [], "threshold": 0.0, "amount": 1.0, "hue_shift": 0.0},
    "blur": {"sigma": 0.0},
    "sampling": {"step": 1, "interp": "blend"},   # interp: blend | hold
    "modulation": [],
    # fast_blur: large radii blurred at reduced resolution in the numpy path (up to ~2x faster, softening error at hard edges)
    # native: use the C++ core when it is built (core/build.sh); always exact, ignores fast_blur
    "quality": {"fast_blur": False, "native": True},
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


def _to_u8(img):
    return (np.clip(img, 0, 1) * 255 + 0.5).astype(np.uint8)


_WORKER_ENGINE = None


def _init_worker(cfg, fps, audio, px_scale):
    global _WORKER_ENGINE
    _WORKER_ENGINE = Engine(cfg, fps, audio, px_scale)


def _work(u8, k):
    """Worker-process entry: uint8 source frame in, uint8 processed keyframe out."""
    return _to_u8(_WORKER_ENGINE.process_key(u8.astype(np.float32) * np.float32(1 / 255),
                                             k / _WORKER_ENGINE.fps, k))


class Engine:
    def __init__(self, config=None, fps=30.0, audio=None, px_scale=1.0):
        self.cfg = _merge(DEFAULTS, config)
        self.fps, self.audio = float(fps), audio
        self.px_scale = float(px_scale)  # <1 when previewing a downscaled frame: keeps blur radii proportional
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

    @property
    def use_native(self):
        q = self.cfg["quality"]
        return bool(q["native"]) and not q["fast_blur"] and native.available()

    def _native_params(self, p, img):
        """Resolve everything time-dependent into a native.Params (img only used to fit a palette)."""
        c = self.cfg
        mode, levels, pal = native.QUANT_NONE, 2, None
        if c["quantize"]["enabled"]:
            if c["quantize"]["mode"] == "kmeans":
                mode, pal = native.QUANT_PALETTE, effects.kmeans_palette(img, p["quantize.colors"], seed=c["seed"])
            else:
                mode, levels = native.QUANT_POSTERIZE, max(2, int(round(p["quantize.levels"])))
        rules = []
        for r in c["remap"]["rules"]:
            amt = float(np.clip(r.get("amount", 1.0) * p["remap.amount"], 0, 1))
            if amt <= 0:
                continue
            tgt = oklab_to_rgb(rotate_hue_lab(rgb_to_oklab(parse_color(r["to"])), p["remap.hue_shift"]))
            rules.append({"src_lab": rgb_to_oklab(parse_color(r["from"])), "tgt_rgb": tgt,
                          "threshold": max(0.0, r.get("threshold", 0.2) + p["remap.threshold"]),
                          "softness": max(1e-4, r.get("softness", 0.05)), "amount": amt})
        return native.Params(mode, levels, pal, p["smooth.sigma"] * self.px_scale,
                             p["blur.sigma"] * self.px_scale, rules)

    def process_key_u8(self, u8, t, threads=0):
        """uint8 in -> uint8 out through the native core (requires use_native)."""
        return native.process_u8(u8, self._native_params(self.params_at(t), u8), threads)

    def process_key(self, img, t, key_id=0):
        """Full effect chain on one source frame at time t."""
        p, c = self.params_at(t), self.cfg
        if self.use_native:
            return native.process_f32(img, self._native_params(p, img))
        out = img
        if c["quantize"]["enabled"]:
            if c["quantize"]["mode"] == "kmeans":
                out = effects.kmeans_quantize(out, p["quantize.colors"], seed=c["seed"])
            else:
                out = effects.posterize(out, p["quantize.levels"])
        out = effects.blur(out, p["smooth.sigma"] * self.px_scale, fast=c["quality"]["fast_blur"])
        out = effects.remap_colors(out, c["remap"]["rules"], p["remap.threshold"],
                                   p["remap.amount"], p["remap.hue_shift"])
        out = effects.blur(out, p["blur.sigma"] * self.px_scale, fast=c["quality"]["fast_blur"])
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

    def _plan(self, i):
        """-> (step, k0, key ids needed) for output frame i."""
        step = max(1, int(round(self.params_at(i / self.fps)["sampling.step"])))
        k0 = (i // step) * step
        if step == 1 or i == k0 or self.cfg["sampling"]["interp"] == "hold":
            return step, k0, (k0,)
        return step, k0, (k0, k0 + step)

    def frames(self, source, workers=None, lookahead=None):
        """Yield every output frame in order as uint8 (H,W,3), processing keyframes in parallel.

        Frames are pure functions of time, so keyframes are independent and run `lookahead` frames ahead
        of the consumer. With the native core: 1-2 Python threads, each calling the multithreaded C++
        kernels (ctypes releases the GIL). Without it: worker processes (numpy threads do not scale: the
        blur issues many small calls and threads convoy on the GIL). Blends are 8-bit integer arithmetic
        (within ~1 level of the float path used by `render`).
        `source` needs ensure(n) -> bool (does frame n exist) and get(n); get_u8(n) is used if present.
        """
        ncpu = os.cpu_count() or 1
        workers = max(1, workers or ncpu)
        pool, futs, nthreads = None, {}, 0
        if self.use_native:
            # The native core threads each frame itself and releases the GIL, so a couple of Python
            # threads (to overlap frames) beat worker processes: no pickling, no extra interpreters.
            frames_in_flight = 1 if workers == 1 else 2
            nthreads = max(1, workers // frames_in_flight)
            pool = ThreadPoolExecutor(frames_in_flight)
            workers = frames_in_flight
        look = lookahead or workers * 2
        if pool is None and workers > 1:
            for v in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "MKL_NUM_THREADS"):
                os.environ.setdefault(v, "1")      # inherited by workers: no BLAS-in-process oversubscription
            pool = ProcessPoolExecutor(workers, mp_context=multiprocessing.get_context("spawn"),
                                       initializer=_init_worker,
                                       initargs=(self.cfg, self.fps, self.audio, self.px_scale))
        grab = getattr(source, "get_u8", None) or (lambda n: _to_u8(source.get(n)))
        sched, ms = 0, self.max_step

        def submit(k):
            u8 = grab(k)
            if nthreads:
                futs[k] = pool.submit(self.process_key_u8, u8, k / self.fps, nthreads)
            elif pool:
                futs[k] = pool.submit(_work, u8, k)
            else:
                futs[k] = _to_u8(self.process_key(u8.astype(np.float32) * np.float32(1 / 255), k / self.fps, k))

        res = (lambda k: futs[k].result()) if pool else (lambda k: futs[k])
        try:
            i = 0
            while source.ensure(i):
                while sched < i + look and source.ensure(sched):
                    for k in self._plan(sched)[2]:
                        if k not in futs:
                            submit(k)
                    sched += 1
                step, k0, ks = self._plan(i)
                a = res(k0)
                if len(ks) == 1:
                    frame = a
                else:
                    w = int(round((i - k0) / step * 256))
                    if nthreads:
                        frame = native.blend_u8(a, res(ks[1]), w, ncpu)
                    else:
                        frame = ((a.astype(np.uint16) * (256 - w) + res(ks[1]).astype(np.uint16) * w + 128) >> 8).astype(np.uint8)
                yield frame
                for k in [k for k in futs if k < i - ms]:
                    del futs[k]
                i += 1
        finally:
            if pool:
                pool.shutdown(wait=False, cancel_futures=True)
