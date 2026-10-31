"""The optimised kernels must agree with straightforward reference implementations."""
import numpy as np
import pytest
from hillview import effects
from hillview.color import rgb_to_oklab, oklab_to_rgb, parse_color, rotate_hue_lab
from hillview.engine import Engine


# ---- slow, obviously-correct references (float64, per-axis np.convolve / pixelwise maths)
def ref_blur(img, sigma):
    """3 sequential box blurs per axis on the edge-extended image, float64."""
    r = max(1, int(round((np.sqrt(4 * sigma * sigma + 1) - 1) / 2)))
    x = img.astype(np.float64)
    for axis in (0, 1):
        pad = [(0, 0)] * 3
        pad[axis] = (3 * r, 3 * r)
        x = np.pad(x, pad, mode="edge")
        k = np.ones(2 * r + 1) / (2 * r + 1)
        for _ in range(3):
            x = np.apply_along_axis(lambda v: np.convolve(v, k, mode="valid"), axis, x)
    return x


def ref_remap(img, rules, thr_off=0.0, amt=1.0, hue=0.0):
    lab = rgb_to_oklab(img)
    out = img.astype(np.float64).copy()
    for r in rules:
        src = rgb_to_oklab(parse_color(r["from"]))
        tgt = oklab_to_rgb(rotate_hue_lab(rgb_to_oklab(parse_color(r["to"])), hue))
        d = np.sqrt(((lab - src) ** 2).sum(-1))
        t = np.clip((d - max(0, r.get("threshold", .2) + thr_off)) / max(1e-4, r.get("softness", .05)), 0, 1)
        m = (1 - t * t * (3 - 2 * t)) * np.clip(r.get("amount", 1) * amt, 0, 1)
        out = out * (1 - m[..., None]) + tgt * m[..., None]
    return out


@pytest.fixture(scope="module")
def img():
    r = np.random.default_rng(3)
    x = r.random((60, 90, 3), dtype=np.float32)
    x[:, :40] = x[:, :40] * 0.2 + 0.6
    x[10:30, 50:80] = [1, 0, 0]
    return x


@pytest.mark.parametrize("sigma", [0.5, 1.5, 3, 8])
def test_blur_matches_float64_reference_everywhere(img, sigma):
    # identical edge handling, so the whole image (borders included) must agree to float32 rounding
    assert np.abs(effects.blur(img, sigma) - ref_blur(img, sigma)).max() < 1e-4 if sigma >= 0.4 else True


def test_blur_below_threshold_is_identity(img):
    assert effects.blur(img, 0.2) is img


def test_blur_constant_image_unchanged_and_odd_sizes_ok():
    for shape in [(7, 11, 3), (1, 20, 3), (33, 1, 3), (64, 65, 3)]:
        x = np.full(shape, 0.37, np.float32)
        assert np.allclose(effects.blur(x, 3), 0.37, atol=1e-5)
        assert effects.blur(x, 3).shape == shape


def test_fast_blur_close_to_exact(img):
    big = np.kron(img, np.ones((8, 8, 1), np.float32))   # 480x720 so the reduced-resolution path engages
    ex, fa = effects.blur(big, 20), effects.blur(big, 20, fast=True)
    assert fa.shape == ex.shape and np.abs(fa - ex).mean() < 0.005


def test_remap_matches_reference(img):
    rules = [{"from": "#ff0000", "to": "#00ffff", "threshold": 0.3, "softness": 0.1},
             {"from": "#999999", "to": "#ff00aa", "threshold": 0.15, "softness": 0.2, "amount": 0.7}]
    got = effects.remap_colors(img, rules, 0.05, 0.9, 0.7)
    assert np.abs(got - ref_remap(img, rules, 0.05, 0.9, 0.7)).max() < 3e-3   # < 1 8-bit level


def test_remap_skip_paths_leave_image_unchanged(img):
    far = {"from": "#00ff00", "to": "#000000", "threshold": 0.0, "softness": 0.001}
    assert np.allclose(effects.remap_colors(img, [far]), img)               # nothing in range
    assert np.allclose(effects.remap_colors(img, [{**far, "threshold": 5, "amount": 0}]), img)  # amount 0


def test_kmeans_assignment_is_nearest_centroid(img):
    out = effects.kmeans_quantize(img, 4, seed=1)
    pal = np.unique(out.reshape(-1, 3), axis=0)
    brute = ((img.reshape(-1, 1, 3) - pal[None]) ** 2).sum(-1).argmin(1)
    assert np.array_equal(pal[brute], out.reshape(-1, 3))


# ---- parallel pipeline
class Src:
    """Minimal source: ensure/get like video_io.Source."""
    def __init__(self, frames): self.f = frames
    def ensure(self, n): return n < len(self.f)
    def get(self, n): return self.f[min(n, len(self.f) - 1)]


def frames(n=11, h=32, w=48):
    # 8-bit-exact like real decoded frames (the parallel path stores sources as uint8)
    r = np.random.default_rng(5)
    return [(r.integers(0, 256, (h, w, 3)).astype(np.float32) * np.float32(1 / 255)) for _ in range(n)]


CFGS = [
    {"smooth": {"sigma": 1.5}, "remap": {"rules": [{"from": "#808080", "to": "#ff00ff", "threshold": 0.3}]}},
    {"sampling": {"step": 3, "interp": "blend"}, "quantize": {"enabled": True, "levels": 3}},
    {"sampling": {"step": 4, "interp": "hold"}, "blur": {"sigma": 2}},
    {"sampling": {"step": 2}, "smooth": {"sigma": 1},
     "modulation": [{"source": {"type": "noise", "hz": 3, "seed": 2}, "target": "blur.sigma", "depth": 2},
                    {"source": {"type": "lfo", "hz": 1}, "target": "sampling.step", "depth": 2}]},
]


def with_native(cfg, on):
    return {**cfg, "quality": {"native": on}}


@pytest.mark.parametrize("native_on", [False, True], ids=["numpy-procs", "native-threads"])
@pytest.mark.parametrize("workers", [1, 2])
@pytest.mark.parametrize("cfg", CFGS, ids=["step1", "blend3", "hold4", "modulated"])
def test_parallel_frames_match_sequential_render(cfg, workers, native_on, native_lib):
    if native_on and not native_lib:
        pytest.skip("native core not built")
    fr = frames()
    ref = Engine(with_native(cfg, False), 24)        # the numpy reference renderer
    got = list(Engine(with_native(cfg, native_on), 24).frames(Src(fr), workers=workers))
    assert len(got) == len(fr) and all(g.dtype == np.uint8 for g in got)
    for i, g in enumerate(got):
        want = ref.render(i, Src(fr))
        assert np.abs(g.astype(np.float32) / 255 - want).max() <= 2.5 / 255, f"frame {i}"


@pytest.mark.parametrize("native_on", [False, True])
def test_frames_stops_cleanly_when_consumer_abandons(native_on, native_lib):
    if native_on and not native_lib:
        pytest.skip("native core not built")
    gen = Engine(with_native({"smooth": {"sigma": 1}}, native_on), 24).frames(Src(frames(20)), workers=2)
    next(gen)
    gen.close()  # must shut the worker pool down without hanging
