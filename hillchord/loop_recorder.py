"""Looper (spec L2).

Records trigger *events* while recording, then — when recording stops — bakes
them into a single stereo loop buffer and plays that as ONE looping Sound. This
means loop playback costs nothing per frame (no re-rendering), so it stays
glitch-free on the handheld. Note tails that run past the loop end wrap around,
so the loop is seamless.

State machine:
    IDLE --L2--> RECORDING --L2--> LOOPING
    LOOPING --double-tap--> PAUSED --double-tap--> LOOPING
    (RECORDING|LOOPING|PAUSED) --hold 2s--> IDLE (cancel)
"""

import copy
import time

import numpy as np
import pygame

import config

IDLE = "idle"
RECORDING = "recording"
LOOPING = "looping"
PAUSED = "paused"


class LoopRecorder:
    def __init__(self, mixer):
        self.mixer = mixer
        self.state = IDLE
        self._events = []        # (t_offset, midis, fx_snapshot, bpm)
        self._t0 = 0.0
        self._length = 0.0
        self._sound = None
        # Dedicated channel (past the 12 voices + metronome) so the loop never
        # competes with live playback for a voice channel.
        self.channel = pygame.mixer.Channel(config.NUM_VOICES + 1)

    # ---- recording hook (called by actions when a voice fires) ----
    def on_trigger(self, midis, fx, bpm):
        if self.state == RECORDING:
            self._events.append(
                (time.monotonic() - self._t0, list(midis), copy.copy(fx), bpm)
            )

    # ---- L2 single press: advance record -> loop ----
    def toggle_record(self):
        if self.state == IDLE:
            self.state = RECORDING
            self._events = []
            self._t0 = time.monotonic()
        elif self.state == RECORDING:
            self._length = max(time.monotonic() - self._t0, 0.05)
            self._bake()
            self.state = LOOPING
            if self._sound is not None:
                self.channel.play(self._sound, loops=-1)

    # ---- L2 double-tap: pause / resume ----
    def toggle_pause(self):
        if self.state == LOOPING:
            self.channel.pause()
            self.state = PAUSED
        elif self.state == PAUSED:
            self.channel.unpause()
            self.state = LOOPING

    # ---- L2 hold 2s: cancel ----
    def cancel(self):
        self.channel.stop()
        self.state = IDLE
        self._events = []
        self._length = 0.0
        self._sound = None

    def update(self):
        """Nothing to do per-frame: a baked loop plays itself on its channel."""

    # ---- bake recorded events into one looping buffer ----
    def _bake(self):
        rate = config.SAMPLE_RATE
        n = max(1, int(self._length * rate))
        buf = np.zeros((n, 2), np.float32)
        for t_off, midis, fx, bpm in self._events:
            pos = int(t_off * rate) % n
            for midi in midis:
                # loop=True reuses the buffers already rendered during live
                # play (cached) -> baking is just array copies, no DSP stall.
                snd = self.mixer._render(midi, fx, bpm, loop=True)
                if snd is None:
                    continue
                arr = pygame.sndarray.array(snd).astype(np.float32)
                if arr.ndim == 1:
                    arr = np.stack([arr, arr], axis=1)
                _add_wrap(buf, arr, pos)
        if not self._events:
            self._sound = None
            return
        peak = np.abs(buf).max()
        if peak > 32767.0:                 # tame summed-note clipping
            buf *= 32767.0 / peak
        out = np.ascontiguousarray(buf.astype(np.int16))
        self._sound = pygame.sndarray.make_sound(out)


def _add_wrap(buf, arr, pos):
    """Add `arr` into `buf` starting at `pos`, wrapping the tail to the start."""
    n = len(buf)
    i = 0
    remaining = len(arr)
    while remaining > 0:
        end = min(pos + remaining, n)
        chunk = end - pos
        buf[pos:end] += arr[i:i + chunk]
        i += chunk
        remaining -= chunk
        pos = 0
