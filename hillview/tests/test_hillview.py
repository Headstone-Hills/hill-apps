import numpy as np
import pytest
from hillview import effects
from hillview.engine import Engine
from hillview.modulation import AudioMod, hash01, source_value
from hillview.audio import analyze


class Src:
    def __init__(self, frames): self.f = frames
    def get(self, n): return self.f[min(n, len(self.f) - 1)]


def noise_frames(n=12, h=24, w=32, seed=0):
    r = np.random.default_rng(seed)
    return [r.random((h, w, 3), dtype=np.float32) for _ in range(n)]


def test_hash_deterministic_and_in_range():
    assert hash01(5, 1) == hash01(5, 1) != hash01(5, 2)
    assert all(0 <= hash01(i, 0) < 1 for i in range(1000))


def test_random_modulation_is_time_pure():
    s = {"type": "noise", "hz": 3, "seed": 4}
    assert source_value(s, 1.234) == source_value(s, 1.234)


def test_posterize_levels():
    img = np.random.default_rng(1).random((16, 16, 3), dtype=np.float32)
    assert len(np.unique(effects.posterize(img, 4))) == 4


def test_kmeans_palette_size_and_determinism():
    img = noise_frames(1)[0]
    a, b = effects.kmeans_quantize(img, 5, seed=2), effects.kmeans_quantize(img, 5, seed=2)
    assert np.array_equal(a, b) and len(np.unique(a.reshape(-1, 3), axis=0)) <= 5


def test_blur_preserves_mean_and_reduces_variance():
    img = noise_frames(1)[0]
    out = effects.blur(img, 3)
    assert abs(out.mean() - img.mean()) < 0.02 and out.var() < img.var() / 4


def test_remap_threshold_gates_replacement():
    img = np.zeros((4, 4, 3), np.float32); img[:] = [1, 0, 0]
    rule = {"from": "#ff0000", "to": "#0000ff", "threshold": 0.1, "softness": 0.01}
    assert np.allclose(effects.remap_colors(img, [rule])[0, 0], [0, 0, 1], atol=0.02)
    far = {**rule, "from": "#00ff00"}
    assert np.allclose(effects.remap_colors(img, [far]), img)
    # a big threshold offset (modulation) makes the far rule fire
    assert not np.allclose(effects.remap_colors(img, [far], threshold_offset=1.0), img)


def test_frame_sampling_hold_and_blend():
    fr = noise_frames(9)
    hold = Engine({"sampling": {"step": 4, "interp": "hold"}}, 30)
    assert np.array_equal(hold.render(2, Src(fr)), fr[0])
    bl = Engine({"sampling": {"step": 4, "interp": "blend"}}, 30)
    assert np.allclose(bl.render(2, Src(fr)), 0.5 * fr[0] + 0.5 * fr[4], atol=1e-6)
    assert np.array_equal(bl.render(4, Src(fr)), fr[4])


def test_random_access_equals_sequential():
    fr = noise_frames(10)
    cfg = {"smooth": {"sigma": 1}, "sampling": {"step": 3},
           "modulation": [{"source": {"type": "noise", "hz": 2}, "target": "blur.sigma", "depth": 2}]}
    seq = Engine(cfg, 24); a = [seq.render(i, Src(fr)) for i in range(10)]
    for i in (7, 2, 9):
        assert np.allclose(Engine(cfg, 24).render(i, Src(fr)), a[i])


def test_modulation_routing_and_clamp():
    e = Engine({"smooth": {"sigma": 1}, "modulation": [
        {"source": {"type": "audio", "channel": "low"}, "target": "smooth.sigma", "depth": 100}]},
        10, AudioMod(10, {"low": [0, 1, 0.5]}))
    assert e.params_at(0.0)["smooth.sigma"] == 1
    assert e.params_at(0.1)["smooth.sigma"] == 64  # clamped


def test_bad_target_rejected():
    with pytest.raises(ValueError):
        Engine({"modulation": [{"source": {"type": "lfo"}, "target": "nope.x"}]})


def test_audio_analysis_tracks_loudness_and_bands():
    sr, fps = 44100, 30
    t = np.arange(sr * 2) / sr
    tone = np.where(t < 1, 0.0, 1.0) * 0.5 * np.sin(2 * np.pi * 80 * t)   # bass tone from 1s
    m = analyze(tone.astype(np.float32), fps, sr)
    assert m.sample("rms", 0.5) < 0.1 < 0.8 < m.sample("rms", 1.8)
    assert m.sample("low", 1.8) > 5 * m.sample("high", 1.8)
