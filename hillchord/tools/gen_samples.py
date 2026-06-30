#!/usr/bin/env python3
"""Generate placeholder per-pitch WAV timbres for local testing.

Creates folders under samples/ each containing one WAV per pitch (named by note,
e.g. C4.wav), 44100Hz 16-bit — matching the spec's sample format and the
SoundBank loader's filename convention.
"""

import os
import struct
import wave

import numpy as np

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(HERE, "samples")
SR = 44100
NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def midi_to_name(m):
    return f"{NAMES[m % 12]}{m // 12 - 1}"


def freq(m):
    return 440.0 * 2 ** ((m - 69) / 12.0)


def render(m, kind, dur=1.2):
    f = freq(m)
    if kind == "synth":
        # Trim length to a whole number of carrier cycles so the waveform (and
        # its integer harmonics) loops seamlessly for sustained drones.
        cycles = max(1, round(dur * f))
        n = int(round(cycles * SR / f))
        t = np.arange(n) / SR
        wave_ = (np.sin(2 * np.pi * f * t)
                 + 0.5 * np.sin(2 * np.pi * 2 * f * t)
                 + 0.25 * np.sin(2 * np.pi * 3 * f * t))
        # Tremolo with an integer number of cycles over the buffer -> still loops.
        env = 1.0 + 0.03 * np.sin(2 * np.pi * 4 * np.arange(n) / n)
    else:  # bell (decaying one-shot, not meant to drone)
        n = int(SR * dur)
        t = np.arange(n) / SR
        wave_ = (np.sin(2 * np.pi * f * t)
                 + 0.6 * np.sin(2 * np.pi * 2.76 * f * t)
                 + 0.3 * np.sin(2 * np.pi * 5.4 * f * t))
        env = np.exp(-t * 3.5)
    sig = wave_ * env
    sig /= np.max(np.abs(sig)) + 1e-9
    return (sig * 0.8 * 32767).astype(np.int16)


def write_wav(path, pcm):
    with wave.open(path, "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def main():
    for kind in ("synth", "bell"):
        folder = os.path.join(OUT, kind.capitalize())
        os.makedirs(folder, exist_ok=True)
        for m in range(36, 85):  # C2..C6
            write_wav(os.path.join(folder, f"{midi_to_name(m)}.wav"),
                      render(m, kind))
        print(f"wrote {folder}")


if __name__ == "__main__":
    main()
