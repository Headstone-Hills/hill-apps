#!/usr/bin/env python3
"""Build a playable drum kit as a multisample mapped to C-major scale degrees.

HillChord plays pitches, so a kit is laid out one drum per diatonic note of C
major. In Note mode that means:
    D-Up=C3  D-Right=D3  D-Down=E3  D-Left=F3  A=G3  B=A3  X=B3
so the seven drums fall under the D-pad + face buttons.

Usage:
  make_drumkit.py "<Kit Name>" kick snare hat openhat tom perc cymbal
(each argument is a path to a one-shot WAV; fewer than 7 is fine)
"""

import os
import sys
import wave

os.environ.setdefault("SDL_VIDEODRIVER", "dummy")
os.environ.setdefault("SDL_AUDIODRIVER", "dummy")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np
import pygame

import config

# Octave 4 = Note mode's default octave, so the diatonic degrees land on these.
NOTES = ["C4", "D4", "E4", "F4", "G4", "A4", "B4"]


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    name, srcs = sys.argv[1], sys.argv[2:]
    pygame.mixer.init(44100, -16, 2, 1024)
    out = os.path.join(config.SAMPLE_PATH, name)
    os.makedirs(out, exist_ok=True)
    for note, src in zip(NOTES, srcs):
        snd = pygame.mixer.Sound(src)
        arr = pygame.sndarray.array(snd)
        if arr.ndim == 1:
            arr = np.stack([arr, arr], axis=1)
        with wave.open(os.path.join(out, note + ".wav"), "w") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(44100)
            w.writeframes(np.ascontiguousarray(arr.astype(np.int16)).tobytes())
        print(f"  {note} <- {os.path.basename(src)}")
    print(f"built kit: {out}")


if __name__ == "__main__":
    main()
