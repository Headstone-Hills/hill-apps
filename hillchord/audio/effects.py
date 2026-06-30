"""Offline DSP effects (Option D).

When a voice is triggered, mixer.py calls apply_effects() once on that voice's
PCM buffer, baking reverb / delay / chorus + wet/dry at the *current* effect
settings. The finished buffer is handed to SDL for playback. This keeps SDL's
12-voice polyphony, adds no continuous CPU load, and gives real reverb quality
at the cost of changes only applying to subsequently-triggered notes.

All processing is float32 in [-1, 1]; input/output are int16 numpy arrays
shaped (n,) mono or (n, 2) stereo.
"""

import numpy as np

import config

# --- Schroeder/Freeverb tuning (samples @ 44.1kHz), stereo-spread on R. -----
_COMB_DELAYS = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617]
_COMB_FEEDBACK = 0.90   # higher -> longer, denser tail (vs a few sharp echoes)
# More allpass stages diffuse the comb echoes into a smooth wash so the reverb
# reads as space, not as discrete repeats like the delay.
_ALLPASS_DELAYS = [556, 441, 341, 225, 180, 131]
_ALLPASS_G = 0.5
_DAMP_TAPS = 5          # short lowpass: damps highs so the tail isn't "clicky"
_REVERB_TAIL = 33075    # ~0.75s of silence appended for the tail to ring out

_CHORUS_DEPTH_MS = 4.0
_CHORUS_RATE_HZ = 1.2
_CHORUS_VOICES = 3


def _to_float_stereo(pcm: np.ndarray) -> np.ndarray:
    """int16 (n,) or (n,2) -> float32 (n,2)."""
    a = pcm.astype(np.float32) / 32768.0
    if a.ndim == 1:
        a = np.stack([a, a], axis=1)
    return a


def _to_int16(a: np.ndarray) -> np.ndarray:
    a = np.clip(a, -1.0, 1.0)
    return (a * 32767.0).astype(np.int16)


def _comb(x: np.ndarray, delay: int, fb: float) -> np.ndarray:
    """Feedback comb y[n] = x[n] + fb*y[n-delay], vectorized in delay-sized
    blocks so the Python loop runs ~n/delay times (tens), not n times."""
    y = x.copy()
    n = len(x)
    for start in range(delay, n, delay):
        end = min(start + delay, n)
        y[start:end] += fb * y[start - delay:start - delay + (end - start)]
    return y


def _allpass(x: np.ndarray, delay: int, g: float) -> np.ndarray:
    """Schroeder allpass."""
    y = np.zeros_like(x)
    buf = x.copy()
    n = len(x)
    # y[n] = -g*x[n] + x[n-d] + g*y[n-d]
    for start in range(0, n, delay):
        end = min(start + delay, n)
        seg = slice(start, end)
        prev = slice(start - delay, start - delay + (end - start))
        delayed_x = buf[prev] if start >= delay else np.zeros(end - start, np.float32)
        delayed_y = y[prev] if start >= delay else np.zeros(end - start, np.float32)
        y[seg] = -g * x[seg] + delayed_x + g * delayed_y
    return y


def _reverb_mono(x: np.ndarray, spread: int = 0) -> np.ndarray:
    out = np.zeros_like(x)
    for d in _COMB_DELAYS:
        out += _comb(x, d + spread, _COMB_FEEDBACK)
    out /= len(_COMB_DELAYS)
    for d in _ALLPASS_DELAYS:
        out = _allpass(out, d + spread, _ALLPASS_G)
    return out


def _damp(x: np.ndarray) -> np.ndarray:
    """Cheap moving-average lowpass for high-frequency damping (per channel)."""
    if _DAMP_TAPS <= 1:
        return x
    k = np.ones(_DAMP_TAPS, np.float32) / _DAMP_TAPS
    out = np.empty_like(x)
    for ch in range(x.shape[1]):
        out[:, ch] = np.convolve(x[:, ch], k, mode="same")
    return out


def _reverb(wet: np.ndarray) -> np.ndarray:
    pad = np.zeros((_REVERB_TAIL, 2), np.float32)
    wet = np.concatenate([wet, pad], axis=0)
    left = _reverb_mono(wet[:, 0], spread=0)
    right = _reverb_mono(wet[:, 1], spread=23)  # stereo de-correlation
    return _damp(np.stack([left, right], axis=1))


def _chorus(wet: np.ndarray, rate: int) -> np.ndarray:
    """LFO-modulated short delay, summed detuned voices."""
    n = len(wet)
    depth = _CHORUS_DEPTH_MS / 1000.0 * rate
    base = depth + 2
    t = np.arange(n)
    out = wet.copy()
    for v in range(_CHORUS_VOICES):
        phase = 2 * np.pi * v / _CHORUS_VOICES
        lfo = base + depth * np.sin(2 * np.pi * _CHORUS_RATE_HZ * t / rate + phase)
        idx = t - lfo
        idx = np.clip(idx, 0, n - 1)
        i0 = np.floor(idx).astype(np.int32)
        frac = (idx - i0).astype(np.float32)
        i1 = np.clip(i0 + 1, 0, n - 1)
        for ch in range(2):
            sampled = wet[i0, ch] * (1 - frac) + wet[i1, ch] * frac
            out[:, ch] += sampled * 0.5
    return out / (1 + _CHORUS_VOICES * 0.5)


def apply_effects(pcm: np.ndarray, fx, bpm: int, rate: int,
                  loop: bool = False) -> np.ndarray:
    """Bake the active effects into `pcm` at the current wet/dry. Returns int16.

    `fx` is a state.EffectsState. If nothing is active, returns pcm unchanged.
    When `loop` is True (a sustained/drone voice), the output is kept at the
    original length — the reverb/delay tail is dropped so the buffer loops
    seamlessly instead of pulsing on each repeat.
    """
    if not fx.any_active():
        return pcm if pcm.ndim == 2 else np.stack([pcm, pcm], axis=1)

    dry = _to_float_stereo(pcm)
    wet = dry.copy()

    # Chain: chorus -> reverb (modulation, then space). NOTE: delay is NOT baked
    # here — it's rendered as scheduled note repeats in the mixer (mixer.py
    # _schedule_echoes) so it produces real beat-spaced echoes that work on
    # sustained drones instead of buzzing when the loop wraps.
    if fx.chorus:
        wet = _chorus(wet, rate)
    if fx.reverb:
        wet = _reverb(wet)

    mix = fx.wetdry / 100.0

    if loop:
        # Wrap the reverb/delay tail back to the start so the wet signal is
        # periodic with the source length -> seamless loop, no pulse.
        n = len(dry)
        wet_p = _wrap_to(wet, n)
        out = dry * (1.0 - mix) + wet_p * mix
    else:
        n = max(len(dry), len(wet))
        dry_p = np.zeros((n, 2), np.float32)
        wet_p = np.zeros((n, 2), np.float32)
        dry_p[: len(dry)] = dry
        wet_p[: len(wet)] = wet
        out = dry_p * (1.0 - mix) + wet_p * mix

    if fx.crushing():
        out = _bitcrush(out, fx.crush_bits, fx.crush_down)
    return _to_int16(out)


def _bitcrush(x: np.ndarray, bits: int, down: int) -> np.ndarray:
    """Bit-depth reduction + sample-and-hold downsampling on a float [-1,1]
    stereo buffer. Baked into the render (cached) -> no runtime overhead."""
    if bits < 16:
        step = 2.0 / (2 ** bits)
        x = np.round(x / step) * step
    if down > 1 and len(x) > down:
        held = (np.arange(len(x)) // down) * down
        x = x[held]
    return x


def _time_stretch(x: np.ndarray, s: float, grain: int = 2048) -> np.ndarray:
    """Overlap-add time-stretch by factor s (output len ~= len(x)*s). Stereo."""
    if abs(s - 1.0) < 1e-3:
        return x
    n = len(x)
    hs = grain // 4                      # synthesis hop
    ha = max(1, int(round(hs / s)))      # analysis hop
    win = np.hanning(grain).astype(np.float32)[:, None]
    out_len = int(n * s) + grain
    out = np.zeros((out_len, 2), np.float32)
    norm = np.zeros((out_len, 1), np.float32)
    pa = ps = 0
    while pa + grain < n and ps + grain < out_len:
        g = x[pa:pa + grain].astype(np.float32) * win
        out[ps:ps + grain] += g
        norm[ps:ps + grain] += win
        pa += ha
        ps += hs
    norm[norm < 1e-6] = 1.0
    return (out / norm)[:int(n * s)].astype(np.int16)


def pitch_shift_preserve(pcm: np.ndarray, semitones: float) -> np.ndarray:
    """Pitch-shift by `semitones` while keeping the original duration/tempo
    (resample to change pitch, then time-stretch back to length)."""
    if semitones == 0:
        return pcm
    a = pcm if pcm.ndim == 2 else np.stack([pcm, pcm], axis=1)
    r = 2.0 ** (semitones / 12.0)
    # resample by r (pitch *r, length /r)
    n = len(a)
    new_n = max(1, int(round(n / r)))
    idx = np.linspace(0, n - 1, new_n)
    i0 = np.floor(idx).astype(np.int32)
    i1 = np.minimum(i0 + 1, n - 1)
    frac = (idx - i0).astype(np.float32)[:, None]
    resampled = (a[i0].astype(np.float32) * (1 - frac)
                 + a[i1].astype(np.float32) * frac).astype(np.int16)
    # time-stretch by r to restore the original duration
    return _time_stretch(resampled, r)


def loop_xfade(pcm: np.ndarray, xfade: int) -> np.ndarray:
    """Make an int16 buffer loop seamlessly by crossfading its tail into its
    head (equal-power). Returns a buffer shortened by `xfade` samples whose
    wrap point is continuous. No-op for buffers too short to crossfade.

    The loop body becomes buf[:M] (M = N - xfade). The head [0:xfade) is an
    equal-power blend of the original head with the tail buf[M:N]; because
    buf[M-1] and buf[M] are adjacent in the source, the M-1 -> 0 wrap is smooth.
    """
    a = pcm if pcm.ndim == 2 else np.stack([pcm, pcm], axis=1)
    n = len(a)
    L = int(xfade)
    if L <= 0 or n <= 2 * L:
        return a
    m = n - L
    out = a[:m].astype(np.float32)
    t = np.linspace(0.0, 1.0, L, dtype=np.float32)[:, None]
    w_in = np.sin(t * np.pi / 2)      # original head fades in
    w_out = np.cos(t * np.pi / 2)     # tail fades out
    out[:L] = a[:L].astype(np.float32) * w_in + a[m:n].astype(np.float32) * w_out
    return out.astype(np.int16)


def _wrap_to(sig: np.ndarray, n: int) -> np.ndarray:
    """Fold a (possibly longer) signal into length n by summing n-sized blocks,
    making the tail wrap around so it loops seamlessly."""
    out = np.zeros((n, 2), np.float32)
    for start in range(0, len(sig), n):
        seg = sig[start:start + n]
        out[: len(seg)] += seg
    return out
