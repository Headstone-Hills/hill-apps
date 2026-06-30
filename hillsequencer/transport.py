"""HillSequencer — transport.

Holds live playback state shared between the sequencer thread and the renderer:
BPM/swing accessors, pattern queueing + chain advancement, tap tempo. Also
provides a monotonic beat clock (epoch + grid helpers) for the metronome and the
beat-pulse dot, mirroring HillChord's transport.
"""

from __future__ import annotations

import math
import time

import config
from config import (
    BPM_MIN, BPM_MAX, SWING_MIN, SWING_MAX, SWING_FACTOR,
    NUM_PATTERNS, NUM_STEPS,
    TAP_BUFFER_SIZE, TAP_MIN_TAPS, TAP_RESET_GAP,
)


class Transport:
    def __init__(self, state):
        self.state = state

        self.playing         = False
        self.current_step    = 0
        self.current_pattern = state.current_pattern
        self.pending_pattern = None

        self.chain_position  = 0
        self.chain_repeats   = 0

        self._tap_times = []

        # Beat clock (for metronome + beat dot)
        self.epoch = time.monotonic()

    # ── BPM ───────────────────────────────────────────────────────────────────
    @property
    def bpm(self):
        return self.state.bpm

    @bpm.setter
    def bpm(self, value):
        self.state.bpm = max(BPM_MIN, min(BPM_MAX, int(value)))

    def step_interval(self):
        """Seconds per 16th-note step at current BPM."""
        return 60.0 / self.bpm / 4.0

    def swing_offset(self, step_index):
        """Extra delay for odd-indexed (off-beat) steps when swing > 0."""
        if self.state.swing == 0 or step_index % 2 == 0:
            return 0.0
        return (self.state.swing / 100.0) * self.step_interval() * SWING_FACTOR

    # ── Swing ─────────────────────────────────────────────────────────────────
    @property
    def swing(self):
        return self.state.swing

    @swing.setter
    def swing(self, value):
        self.state.swing = max(SWING_MIN, min(SWING_MAX, int(value)))

    # ── Pattern ───────────────────────────────────────────────────────────────
    def queue_pattern(self, index):
        index = index % NUM_PATTERNS
        if index != self.current_pattern:
            self.pending_pattern = index

    def queue_next_pattern(self):
        self.queue_pattern((self.current_pattern + 1) % NUM_PATTERNS)

    def queue_prev_pattern(self):
        self.queue_pattern((self.current_pattern - 1) % NUM_PATTERNS)

    def commit_pending_pattern(self):
        if self.pending_pattern is not None:
            self.current_pattern = self.pending_pattern
            self.state.current_pattern = self.pending_pattern
            self.pending_pattern = None
            return True
        return False

    # ── Chain ─────────────────────────────────────────────────────────────────
    def advance_chain(self):
        chain = self.state.chain
        if not chain:
            return self.current_pattern
        entry = chain[self.chain_position]
        self.chain_repeats += 1
        if self.chain_repeats >= entry["repeats"]:
            self.chain_repeats = 0
            self.chain_position = (self.chain_position + 1) % len(chain)
        new_entry = chain[self.chain_position]
        self.current_pattern = new_entry["pattern"]
        self.state.current_pattern = self.current_pattern
        return self.current_pattern

    def reset_chain(self):
        self.chain_position = 0
        self.chain_repeats  = 0
        if self.state.chain:
            self.current_pattern       = self.state.chain[0]["pattern"]
            self.state.current_pattern = self.current_pattern

    # ── Tap Tempo ─────────────────────────────────────────────────────────────
    def tap(self):
        now = time.time()
        if self._tap_times and (now - self._tap_times[-1]) > TAP_RESET_GAP:
            self._tap_times = []
        self._tap_times.append(now)
        if len(self._tap_times) > TAP_BUFFER_SIZE:
            self._tap_times = self._tap_times[-TAP_BUFFER_SIZE:]
        if len(self._tap_times) >= TAP_MIN_TAPS:
            intervals = [self._tap_times[i] - self._tap_times[i - 1]
                         for i in range(1, len(self._tap_times))]
            avg = sum(intervals) / len(intervals)
            self.bpm = int(round(60.0 / avg))
            return self.bpm
        return None

    # ── Beat clock (metronome + beat dot) ─────────────────────────────────────
    def beat_len(self, bpm=None):
        return 60.0 / max(bpm or self.bpm, 1)

    def grid_next(self, now, step):
        k = math.floor((now - self.epoch) / step + 1e-9) + 1
        return self.epoch + k * step

    def beat_index(self, now, bpm=None):
        return int(math.floor((now - self.epoch) / self.beat_len(bpm) + 1e-9))

    # ── Play / Stop ───────────────────────────────────────────────────────────
    def stop_and_reset(self):
        self.playing         = False
        self.current_step    = 0
        self.pending_pattern = None
        self.reset_chain()
