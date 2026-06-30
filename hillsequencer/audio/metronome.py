"""Programmatic metronome (spec: Select toggles in library; BPM from state).

Generates a short click tone with numpy (no click.wav needed) and plays it on
a reserved channel at BPM intervals, driven from the main loop's clock.
"""

import time

import numpy as np
import pygame

import config


def _make_click(freq: int, ms: int = 30) -> "pygame.mixer.Sound":
    n = int(config.SAMPLE_RATE * ms / 1000)
    t = np.arange(n)
    env = np.exp(-t / (n / 4.0))                      # fast decay
    tone = np.sin(2 * np.pi * freq * t / config.SAMPLE_RATE) * env
    pcm = (np.clip(tone, -1, 1) * 0.6 * 32767).astype(np.int16)
    stereo = np.ascontiguousarray(np.stack([pcm, pcm], axis=1))
    return pygame.sndarray.make_sound(stereo)


class Metronome:
    def __init__(self, transport):
        self.transport = transport
        self._hi = _make_click(1760)   # accented beat (beat 0 of the bar)
        self._lo = _make_click(880)    # other beats
        self._next_t = 0.0
        # Reserved channel past all the per-track bands (see config channel map).
        self.channel = pygame.mixer.Channel(config.METRONOME_CHANNEL)

    def reset(self):
        self._next_t = 0.0

    def update(self, state):
        if not state.metronome_on:
            return
        now = time.monotonic()
        if now < self._next_t:
            return
        beat = self.transport.beat_len(state.bpm)
        # Click on the shared grid so the arpeggiator lines up with it.
        click = self._hi if self.transport.beat_index(now, state.bpm) % 4 == 0 else self._lo
        self.channel.play(click)
        self._next_t = self.transport.grid_next(now, beat)
