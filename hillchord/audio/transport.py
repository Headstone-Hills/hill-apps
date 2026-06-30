"""Shared musical clock so the metronome, arpeggiator and looper agree on the
beat grid. A single epoch + the current BPM define where the beats fall."""

import math
import time


class Transport:
    def __init__(self):
        self.epoch = time.monotonic()

    def beat_len(self, bpm):
        return 60.0 / max(bpm, 1)

    def grid_next(self, now, step):
        """First grid point strictly after `now` (grid anchored at epoch)."""
        k = math.floor((now - self.epoch) / step + 1e-9) + 1
        return self.epoch + k * step

    def grid_prev(self, now, step):
        """Most recent grid point at or before `now` (grid anchored at epoch)."""
        k = math.floor((now - self.epoch) / step + 1e-9)
        return self.epoch + k * step

    def beat_index(self, now, bpm):
        """Which beat number `now` falls on (for metronome accenting)."""
        return int(math.floor((now - self.epoch) / self.beat_len(bpm) + 1e-9))

    def grid_nearest(self, now, step):
        """Nearest grid point to `now` — prev or next, whichever is closer."""
        prev = self.grid_prev(now, step)
        nxt = prev + step
        return prev if (now - prev) <= (nxt - now) else nxt

    def quantize_beats(self, duration, bpm):
        """Round a duration to the nearest whole number of beats (>= 1)."""
        beat = self.beat_len(bpm)
        return max(1, round(duration / beat)) * beat
