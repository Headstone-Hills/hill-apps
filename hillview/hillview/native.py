"""ctypes binding for the C++ core (core/). Optional: everything falls back to numpy.

Lookup order: $HILLVIEW_LIB, hillview/_native/ (your own build: core/build.sh), prebuilt/<os>-<arch>/ (checked in),
../core/build/.
ctypes releases the GIL during calls, so Python threads can drive it in parallel.
"""
import ctypes as C
import glob
import os
import platform
import sys
import numpy as np

_here = os.path.dirname(os.path.abspath(__file__))


class HVRule(C.Structure):
    _fields_ = [("src_lab", C.c_float * 3), ("tgt_rgb", C.c_float * 3),
                ("threshold", C.c_float), ("softness", C.c_float), ("amount", C.c_float)]


class HVFrameParams(C.Structure):
    _fields_ = [("quant_mode", C.c_int32), ("levels", C.c_int32),
                ("palette", C.POINTER(C.c_float)), ("palette_n", C.c_int32),
                ("smooth_sigma", C.c_float), ("blur_sigma", C.c_float),
                ("rules", C.POINTER(HVRule)), ("n_rules", C.c_int32)]


QUANT_NONE, QUANT_POSTERIZE, QUANT_PALETTE = 0, 1, 2
EXPECTED_VERSION = 1
_lib, _tried = None, False


def platform_tag():
    """e.g. 'macos-arm64', 'linux-x86_64': names the prebuilt/ subdirectory for this machine."""
    osname = {"darwin": "macos", "win32": "windows"}.get(sys.platform, "linux")
    m = platform.machine().lower()
    return f"{osname}-{ {'x86_64': 'x86_64', 'amd64': 'x86_64', 'arm64': 'arm64', 'aarch64': 'arm64'}.get(m, m) }"


def _libname():
    return {"win32": "hillview.dll", "darwin": "libhillview.dylib"}.get(sys.platform, "libhillview.so")


def _candidates():
    env = os.environ.get("HILLVIEW_LIB")
    if env:
        yield env
    name = _libname()
    for d in (os.path.join(_here, "_native"),
              os.path.join(_here, "..", "prebuilt", platform_tag()),
              os.path.join(_here, "..", "core", "build")):
        yield os.path.join(d, name)
        yield from sorted(glob.glob(os.path.join(d, name + "*")))


_errors = []


def load(force=False):
    """-> CDLL or None. Cached."""
    global _lib, _tried
    if _tried and not force:
        return _lib
    _tried, _lib = True, None
    del _errors[:]
    if os.environ.get("HILLVIEW_NO_NATIVE"):
        _errors.append("disabled by HILLVIEW_NO_NATIVE")
        return None
    for path in _candidates():
        if not os.path.exists(path):
            continue
        try:
            lib = C.CDLL(path)
            if lib.hv_version() != EXPECTED_VERSION:
                _errors.append(f"{path}: core version {lib.hv_version()}, expected {EXPECTED_VERSION}")
                continue
            fp, u8 = C.POINTER(C.c_float), C.POINTER(C.c_uint8)
            pp = C.POINTER(HVFrameParams)
            lib.hv_process_f32.argtypes = [fp, fp, C.c_int, C.c_int, pp, C.c_int]
            lib.hv_process_u8.argtypes = [u8, u8, C.c_int, C.c_int, pp, C.c_int]
            lib.hv_process_f32.restype = lib.hv_process_u8.restype = C.c_int
            lib.hv_blend_u8.argtypes = [u8, u8, u8, C.c_int, C.c_int, C.c_int, C.c_int]
            lib.hv_blend_u8.restype = C.c_int
            lib.hv_hash01.argtypes, lib.hv_hash01.restype = [C.c_int64, C.c_int64], C.c_double
            lib.hv_mod_source.argtypes = [C.c_int, C.c_int, C.c_double, C.c_double, C.c_double, C.c_int64, C.c_double]
            lib.hv_mod_source.restype = C.c_double
            _lib = lib
            _errors.append(f"loaded {os.path.abspath(path)}")
            return lib
        except (OSError, AttributeError) as e:   # e.g. wrong architecture, missing/invalid code signature
            _errors.append(f"{path}: {e}")
    return None


def describe():
    """One line for logs: which core is in use, or why numpy is being used instead."""
    if load() is not None:
        return f"native core: {_errors[-1][len('loaded '):]}"
    why = "; ".join(e for e in _errors if not e.startswith("loaded")) or \
        f"no library found for {platform_tag()} (build it: core/build.sh)"
    return f"native core: unavailable ({why}); using the slower numpy path"


def available():
    return load() is not None


class Params:
    """Owns the arrays a HVFrameParams points into so they outlive the call."""

    def __init__(self, quant_mode=QUANT_NONE, levels=2, palette=None, smooth_sigma=0.0, blur_sigma=0.0, rules=()):
        self.palette = None if palette is None else np.ascontiguousarray(palette, np.float32).reshape(-1, 3)
        self.rules = (HVRule * max(1, len(rules)))()
        for i, r in enumerate(rules):
            d = self.rules[i]
            d.src_lab[:], d.tgt_rgb[:] = [float(x) for x in r["src_lab"]], [float(x) for x in r["tgt_rgb"]]
            d.threshold, d.softness, d.amount = r["threshold"], r["softness"], r["amount"]
        self.c = HVFrameParams(
            quant_mode, int(levels),
            self.palette.ctypes.data_as(C.POINTER(C.c_float)) if self.palette is not None else None,
            0 if self.palette is None else len(self.palette),
            float(smooth_sigma), float(blur_sigma), self.rules, len(rules))


def _check(rc):
    if rc != 0:
        raise RuntimeError(f"native core rejected its arguments (code {rc})")


def process_f32(img, params, threads=0):
    """(H,W,3) float32 in [0,1] -> processed float32."""
    lib = load()
    img = np.ascontiguousarray(img, np.float32)
    out = np.empty_like(img)
    h, w, _ = img.shape
    _check(lib.hv_process_f32(img.ctypes.data_as(C.POINTER(C.c_float)), out.ctypes.data_as(C.POINTER(C.c_float)),
                              w, h, C.byref(params.c), threads))
    return out


def process_u8(img, params, threads=0):
    """(H,W,3) uint8 -> processed uint8."""
    lib = load()
    img = np.ascontiguousarray(img, np.uint8)
    out = np.empty_like(img)
    h, w, _ = img.shape
    _check(lib.hv_process_u8(img.ctypes.data_as(C.POINTER(C.c_uint8)), out.ctypes.data_as(C.POINTER(C.c_uint8)),
                             w, h, C.byref(params.c), threads))
    return out


def blend_u8(a, b, weight256, threads=0):
    """(a*(256-w) + b*w + 128) >> 8 per byte for two uint8 (H,W,3) frames."""
    lib = load()
    a, b = np.ascontiguousarray(a, np.uint8), np.ascontiguousarray(b, np.uint8)
    out = np.empty_like(a)
    h, w, _ = a.shape
    u8 = C.POINTER(C.c_uint8)
    _check(lib.hv_blend_u8(a.ctypes.data_as(u8), b.ctypes.data_as(u8), out.ctypes.data_as(u8), w, h, int(weight256), threads))
    return out


def hash01(n, seed):
    return load().hv_hash01(int(n), int(seed))


def mod_source(type_, shape, hz, phase, duty, seed, t):
    return load().hv_mod_source(type_, shape, hz, phase, duty, int(seed), t)
