"""C++ core: parity with the numpy reference, determinism, edge cases, thread-safety."""
import ctypes as C
import subprocess
import sys
import threading

import numpy as np
import pytest

from hillview import effects, native
from hillview.engine import Engine
from hillview.modulation import hash01, source_value

pytestmark = pytest.mark.usefixtures("native_lib")


@pytest.fixture(autouse=True)
def _need_native(native_lib):
    if not native_lib:
        pytest.skip("native core not built (core/build.sh)")


RULES = [{"from": "#ff0000", "to": "#00ffff", "threshold": 0.3, "softness": 0.1},
         {"from": "#999999", "to": "#ff00aa", "threshold": 0.15, "softness": 0.2, "amount": 0.7}]
CFGS = {
    "posterize+blur": {"quantize": {"enabled": True, "levels": 4}, "smooth": {"sigma": 3}, "blur": {"sigma": 1.5}},
    "remap": {"remap": {"rules": RULES, "hue_shift": 0.7, "threshold": 0.05, "amount": 0.9}},
    "all": {"quantize": {"enabled": True, "levels": 5}, "smooth": {"sigma": 2.5},
            "remap": {"rules": RULES}, "blur": {"sigma": 4}},
    "kmeans": {"quantize": {"enabled": True, "mode": "kmeans", "colors": 6}, "remap": {"rules": RULES}},
    "modulated": {"smooth": {"sigma": 1}, "remap": {"rules": RULES},
                  "modulation": [{"source": {"type": "noise", "hz": 2, "seed": 3}, "target": "smooth.sigma", "depth": 3},
                                 {"source": {"type": "lfo", "hz": 1}, "target": "remap.threshold", "depth": 0.2},
                                 {"source": {"type": "steps", "hz": 4, "seed": 1}, "target": "remap.hue_shift", "depth": 2}]},
}


def mk(cfg, on):
    return Engine({**cfg, "quality": {"native": on}}, 30)


def photo(h, w, seed=0):
    r = np.random.default_rng(seed)
    x = r.random((h, w, 3), dtype=np.float32)
    x[:, : w // 2] = x[:, : w // 2] * 0.2 + 0.6
    x[h // 5: h // 2, w // 2:] = [1, 0, 0]
    return x


@pytest.mark.parametrize("name", list(CFGS))
def test_process_matches_numpy_reference(name):
    img = photo(70, 110)
    a, b = mk(CFGS[name], False).process_key(img, 0.4), mk(CFGS[name], True).process_key(img, 0.4)
    assert b.shape == a.shape and b.dtype == np.float32
    err = np.abs(a - b)
    assert err.max() < 3e-3 or (err.max(-1) > 3e-3).mean() < 1e-4, f"max {err.max()}"   # < 1 8-bit level (kmeans ties aside)


@pytest.mark.parametrize("shape", [(1, 1), (1, 9), (7, 1), (3, 3), (17, 33), (40, 5)])
def test_tiny_and_odd_sizes(shape):
    img = photo(*shape)
    cfg = CFGS["all"]
    a, b = mk(cfg, False).process_key(img, 0.1), mk(cfg, True).process_key(img, 0.1)
    assert np.abs(a - b).max() < 3e-3


def test_u8_path_equals_f32_path_within_one_level():
    u8 = (np.random.default_rng(2).random((50, 64, 3)) * 255).astype(np.uint8)
    e = mk(CFGS["all"], True)
    p = e._native_params(e.params_at(0.2), u8)
    got = native.process_u8(u8, p).astype(np.int16)
    want = np.clip(native.process_f32(u8.astype(np.float32) * np.float32(1 / 255), p) * 255 + 0.5, 0, 255).astype(np.int16)
    assert np.abs(got - want).max() <= 1


def test_thread_count_does_not_change_output():
    u8 = (np.random.default_rng(4).random((90, 140, 3)) * 255).astype(np.uint8)
    e = mk(CFGS["all"], True)
    p = e._native_params(e.params_at(0.2), u8)
    ref = native.process_u8(u8, p, 1)
    for t in (2, 3, 8):
        assert np.array_equal(native.process_u8(u8, p, t), ref), f"threads={t}"


def test_concurrent_callers_are_independent():
    e = mk(CFGS["all"], True)
    frames = [(np.random.default_rng(i).random((60, 80, 3)) * 255).astype(np.uint8) for i in range(8)]
    want = [e.process_key_u8(f, 0.3, 1) for f in frames]
    got = [None] * 8

    def run(i):
        for _ in range(5):
            got[i] = e.process_key_u8(frames[i], 0.3, 2)

    ts = [threading.Thread(target=run, args=(i,)) for i in range(8)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert all(np.array_equal(g, w) for g, w in zip(got, want))


@pytest.mark.parametrize("w", [0, 1, 128, 255, 256])
def test_blend_matches_integer_formula_exactly(w):
    r = np.random.default_rng(5)
    a, b = (r.integers(0, 256, (11, 13, 3)).astype(np.uint8) for _ in range(2))
    want = ((a.astype(np.uint16) * (256 - w) + b.astype(np.uint16) * w + 128) >> 8).astype(np.uint8)
    assert np.array_equal(native.blend_u8(a, b, w), want)


def test_invalid_arguments_are_rejected_not_crashed(native_lib):
    f32 = C.POINTER(C.c_float)
    p = native.Params()
    buf = (C.c_float * 12)()
    ptr = C.cast(buf, f32)
    assert native_lib.hv_process_f32(ptr, ptr, 2, 2, C.byref(p.c), 1) == -1          # aliased in/out
    assert native_lib.hv_process_f32(None, ptr, 2, 2, C.byref(p.c), 1) == -1
    assert native_lib.hv_process_f32(ptr, C.cast((C.c_float * 12)(), f32), 0, 2, C.byref(p.c), 1) == -1
    assert native_lib.hv_process_f32(ptr, C.cast((C.c_float * 12)(), f32), 2, 2, None, 1) == -1
    assert native_lib.hv_blend_u8(None, None, None, 2, 2, 5, 1) == -1
    u8 = C.POINTER(C.c_uint8)
    bb = C.cast((C.c_uint8 * 12)(), u8)
    assert native_lib.hv_blend_u8(bb, bb, bb, 2, 2, 300, 1) == -1                    # weight out of range
    assert native_lib.hv_blend_u8(bb, bb, bb, 2, 2, -1, 1) == -1


def test_no_stages_is_a_copy():
    img = photo(9, 9)
    assert np.array_equal(native.process_f32(img, native.Params()), img)


# ---- modulation parity: the host plugins rely on these matching the Python spec exactly
@pytest.mark.parametrize("seed", [0, 1, 7, -3, 2 ** 35])
def test_hash_parity(seed):
    for n in [-5, -1, 0, 1, 2, 12345, 2 ** 40]:
        assert native.hash01(n, seed) == hash01(n, seed)


@pytest.mark.parametrize("src,args", [
    ({"type": "lfo", "shape": "sine", "hz": 0.7, "phase": 0.2}, (0, 0, 0.7, 0.2, 0.5)),
    ({"type": "lfo", "shape": "saw", "hz": 1.3}, (0, 1, 1.3, 0.0, 0.5)),
    ({"type": "lfo", "shape": "tri", "hz": 2.0, "phase": 0.9}, (0, 2, 2.0, 0.9, 0.5)),
    ({"type": "lfo", "shape": "square", "hz": 3.1, "duty": 0.3}, (0, 3, 3.1, 0.0, 0.3)),
    ({"type": "noise", "hz": 2.5, "seed": 4}, (1, 0, 2.5, 0.0, 0.5)),
    ({"type": "steps", "hz": 5.0, "seed": 9}, (2, 0, 5.0, 0.0, 0.5)),
])
def test_modulation_source_parity(src, args):
    typ, shape, hz, ph, duty = args
    seed = src.get("seed", 0)
    for t in np.linspace(-3.0, 60.0, 400):
        want = source_value(src, float(t))
        got = native.mod_source(typ, shape, hz, ph, duty, seed, float(t))
        assert abs(got - want) < 1e-12, (src, t, got, want)


def test_numpy_fallback_when_native_disabled():
    code = ("import os; os.environ['HILLVIEW_NO_NATIVE']='1';"
            "from hillview import native; from hillview.engine import Engine; import numpy as np;"
            "assert not native.available();"
            "e=Engine({'smooth':{'sigma':2}},30); assert not e.use_native;"
            "print(e.process_key(np.random.rand(20,20,3).astype('float32'),0).shape)")
    r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, cwd=".")
    assert r.returncode == 0 and "(20, 20, 3)" in r.stdout, r.stderr
