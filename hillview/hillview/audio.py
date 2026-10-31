"""Audio -> per-video-frame modulation envelopes (host-agnostic .json)."""
import subprocess
import numpy as np
from .modulation import AudioMod

SR = 44100
BANDS = {"low": (20, 250), "mid": (250, 2000), "high": (2000, 12000)}


def decode(path, sr=SR):
    """Decode any audio/video file to mono float32 via ffmpeg."""
    r = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(sr),
                        "-f", "f32le", "-"], capture_output=True, check=True)
    return np.frombuffer(r.stdout, np.float32)


def _smooth_env(x, attack, release, fps):
    """One-pole attack/release follower (time constants in seconds)."""
    a = 1 - np.exp(-1 / max(attack * fps, 1e-3))
    r = 1 - np.exp(-1 / max(release * fps, 1e-3))
    y, out = 0.0, np.empty_like(x)
    for i, v in enumerate(x):
        y += (a if v > y else r) * (v - y)
        out[i] = y
    return out


def analyze(samples, fps, sr=SR, attack=0.01, release=0.15, n_frames=None):
    """Return AudioMod with channels rms, low, mid, high, onset (all 0..1)."""
    win = 2048
    n = n_frames or int(len(samples) / sr * fps)
    centers = ((np.arange(n) + 0.5) / fps * sr).astype(int)
    pad = np.pad(samples, (win, win))
    hann = np.hanning(win).astype(np.float32)
    freqs = np.fft.rfftfreq(win, 1 / sr)
    rms, flux = np.zeros(n), np.zeros(n)
    band = {k: np.zeros(n) for k in BANDS}
    prev = None
    for i, c in enumerate(centers):
        seg = pad[c + win // 2: c + win // 2 + win]
        rms[i] = np.sqrt(np.mean(seg ** 2))
        mag = np.abs(np.fft.rfft(seg * hann))
        for k, (lo, hi) in BANDS.items():
            m = (freqs >= lo) & (freqs < hi)
            band[k][i] = mag[m].mean() if m.any() else 0
        if prev is not None:
            flux[i] = np.maximum(mag - prev, 0).sum()
        prev = mag
    chans = {"rms": rms, **band, "onset": flux}
    out = {}
    for k, v in chans.items():
        v = np.log1p(v * 20) if k != "onset" else v      # compress dynamics (not onsets)
        hi = np.percentile(v, 99) or 1.0
        v = np.clip(v / hi, 0, 1)
        out[k] = _smooth_env(v, 0.002 if k == "onset" else attack, release, fps)
    return AudioMod(fps, out)
