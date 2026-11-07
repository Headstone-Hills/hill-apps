"""Mixing core: sources -> one float32 stereo buffer -> int16 bytes.

Device-independent (numpy only). The output backends in audio_out.py pull from
Mixer.render(start_frame, n) in fixed-size chunks, so EVERY source is positioned by the same
absolute frame counter: tracks cannot drift against each other (the "single transport clock").
"""

from __future__ import annotations

import math
import wave

import numpy as np

RATE = 44100


class Source(object):
    gain = 1.0

    def render(self, start, n, rate):
        """-> float32 array (n, 2) for absolute frames [start, start+n), or None for silence."""
        raise NotImplementedError


class Tone(Source):
    """Continuous sine, phase locked to the absolute frame counter (no clicks across chunks)."""
    def __init__(self, freq=440.0, amp=0.2, gain=1.0):
        self.freq, self.amp, self.gain = freq, amp, gain

    def render(self, start, n, rate):
        t = (np.arange(start, start + n, dtype=np.float64)) / rate
        x = (self.amp * np.sin(2 * np.pi * self.freq * t)).astype(np.float32)
        return np.stack([x, x], axis=1)


class Click(Source):
    """Metronome: a short decaying sine on every beat, higher and louder on beat 1 of the bar."""
    def __init__(self, bpm=120.0, beats_per_bar=4, amp=0.6, gain=1.0):
        self.bpm, self.beats, self.amp, self.gain = bpm, beats_per_bar, amp, gain
        self.enabled = True

    def render(self, start, n, rate):
        if not self.enabled:
            return None
        spb = rate * 60.0 / self.bpm                      # samples per beat
        click_len = int(0.03 * rate)
        out = np.zeros(n, dtype=np.float32)
        first = max(0, int(math.floor((start - click_len) / spb)))
        last = int(math.floor((start + n) / spb))
        for b in range(first, last + 1):
            pos = int(round(b * spb)) - start                # click start relative to chunk
            lo, hi = max(0, pos), min(n, pos + click_len)
            if hi <= lo:
                continue
            accent = (b % self.beats) == 0
            f = 1500.0 if accent else 1000.0
            k = np.arange(lo - pos, hi - pos, dtype=np.float64)
            env = np.exp(-k / (0.006 * rate))
            out[lo:hi] += (self.amp * (1.0 if accent else 0.6) * env *
                           np.sin(2 * np.pi * f * k / rate)).astype(np.float32)
        return np.stack([out, out], axis=1)


class BarTone(Source):
    """A soft decaying tone struck on beat 1 of each bar (to hear sync against the click)."""
    def __init__(self, bpm=120.0, beats_per_bar=4, freq=220.0, amp=0.3, gain=1.0):
        self.bpm, self.beats, self.freq, self.amp, self.gain = bpm, beats_per_bar, freq, amp, gain

    def render(self, start, n, rate):
        bar = rate * 60.0 / self.bpm * self.beats
        idx = np.arange(start, start + n, dtype=np.float64)
        since = np.mod(idx, bar)                                   # samples since the bar start
        env = np.exp(-since / (0.25 * rate))
        x = (self.amp * env * np.sin(2 * np.pi * self.freq * idx / rate)).astype(np.float32)
        return np.stack([x, x], axis=1)


def load_wav(path):
    """-> (float32 array (frames, 2), rate). 8/16/24-bit PCM, mono or stereo (mono is duplicated)."""
    with wave.open(path, "rb") as w:
        ch, sw, rate, nf = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        raw = w.readframes(nf)
    if sw == 2:
        a = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif sw == 3:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        v = np.where(v & 0x800000, v - 0x1000000, v)
        a = v.astype(np.float32) / 8388608.0
    elif sw == 1:
        a = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    else:
        raise ValueError("unsupported sample width %d" % sw)
    a = a.reshape(-1, ch)
    if ch == 1:
        a = np.repeat(a, 2, axis=1)
    elif ch > 2:
        a = a[:, :2]
    return np.ascontiguousarray(a), rate


def resample_linear(a, src_rate, dst_rate):
    """Linear-interpolation resample of (frames, 2) audio (cheap; fine for auditioning/loops)."""
    if src_rate == dst_rate or len(a) == 0:
        return a
    n = int(round(len(a) * dst_rate / float(src_rate)))
    x = np.linspace(0, len(a) - 1, n)
    return np.stack([np.interp(x, np.arange(len(a)), a[:, c]) for c in range(2)], axis=1).astype(np.float32)


class ClipSource(Source):
    """A block of audio placed on the timeline at `offset` frames (optionally looped)."""
    def __init__(self, data, offset=0, loop=False, gain=1.0):
        self.data, self.offset, self.loop, self.gain = data, int(offset), loop, gain

    @classmethod
    def from_wav(cls, path, rate=RATE, **kw):
        a, r = load_wav(path)
        return cls(resample_linear(a, r, rate), **kw)

    def render(self, start, n, rate):
        L = len(self.data)
        if L == 0:
            return None
        pos = start - self.offset
        if self.loop:
            if pos + n <= 0:
                return None
            idx = (np.arange(pos, pos + n) % L)
            out = self.data[idx].copy()
            if pos < 0:
                out[: min(n, -pos)] = 0
            return out
        if pos >= L or pos + n <= 0:
            return None
        out = np.zeros((n, 2), dtype=np.float32)
        lo, hi = max(0, -pos), min(n, L - pos)
        out[lo:hi] = self.data[pos + lo:pos + hi]
        return out


class Mixer(object):
    def __init__(self, rate=RATE):
        self.rate = rate
        self.sources = []
        self.master = 1.0
        self.peak = 0.0                                  # last chunk's peak (for meters)

    def add(self, src):
        self.sources.append(src)
        return src

    def render_float(self, start, n):
        out = np.zeros((n, 2), dtype=np.float32)
        for s in list(self.sources):
            b = s.render(start, n, self.rate)
            if b is not None:
                out += b * np.float32(s.gain)
        out *= np.float32(self.master)
        self.peak = float(np.max(np.abs(out))) if n else 0.0
        return np.clip(out, -1.0, 1.0)

    def render(self, start, n):
        """-> little-endian int16 interleaved bytes for frames [start, start+n)."""
        f = self.render_float(start, n)
        return (f * 32767.0).astype("<i2").tobytes()
