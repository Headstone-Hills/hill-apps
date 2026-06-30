"""
HillBeat — transport.py
Playback state and BPM/swing accessors shared between sequencer and UI.
"""

import time
from constants import (
    DEFAULT_BPM, BPM_MIN, BPM_MAX,
    DEFAULT_SWING, SWING_MIN, SWING_MAX,
    TAP_BUFFER_SIZE, TAP_MIN_TAPS, TAP_RESET_GAP,
)


class Transport:
    """
    Holds the live playback state that both the sequencer thread and the
    render thread need to read.  Write operations that affect step timing
    should be done with the sequencer's lock held where indicated.
    """

    def __init__(self, state):
        self.state = state          # shared AppState reference

        # Derived from state — mirrored here for fast lock-free reads
        self.playing         = False
        self.current_step    = 0
        self.current_pattern = state.current_pattern
        self.pending_pattern = None  # pattern queued for next loop boundary

        # Chain runtime
        self.chain_position  = 0    # index into state.chain
        self.chain_repeats   = 0    # how many times current entry has played

        # Tap-tempo
        self._tap_times = []

        # BPM hold state (managed by input handler)
        self.bpm_hold_dir   = 0     # +1 or -1 while R2/L2 held
        self.bpm_hold_start = None
        self.bpm_hold_last  = None

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

    def swing_offset(self, step_index: int) -> float:
        """
        Extra delay for even-indexed steps when swing > 0.
        Step 0 is considered odd (no delay), step 1 is even (delayed), etc.
        """
        from constants import SWING_FACTOR
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

    def queue_pattern(self, index: int):
        """Request pattern change; takes effect at next loop boundary."""
        from constants import NUM_PATTERNS
        index = index % NUM_PATTERNS
        if index != self.current_pattern:
            self.pending_pattern = index

    def queue_next_pattern(self):
        from constants import NUM_PATTERNS
        self.queue_pattern((self.current_pattern + 1) % NUM_PATTERNS)

    def queue_prev_pattern(self):
        from constants import NUM_PATTERNS
        self.queue_pattern((self.current_pattern - 1) % NUM_PATTERNS)

    def commit_pending_pattern(self):
        """Called by sequencer at the boundary (step 0)."""
        if self.pending_pattern is not None:
            self.current_pattern = self.pending_pattern
            self.state.current_pattern = self.pending_pattern
            self.pending_pattern = None
            return True
        return False

    # ── Chain ─────────────────────────────────────────────────────────────────

    def advance_chain(self):
        """
        Called at each loop boundary when chain_enabled.
        Returns the new pattern index to play.
        """
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

    def tap(self) -> int | None:
        """
        Record a tap.  Returns the new BPM if there are enough taps,
        else None.
        """
        now = time.time()
        if self._tap_times and (now - self._tap_times[-1]) > TAP_RESET_GAP:
            self._tap_times = []
        self._tap_times.append(now)
        if len(self._tap_times) > TAP_BUFFER_SIZE:
            self._tap_times = self._tap_times[-TAP_BUFFER_SIZE:]
        if len(self._tap_times) >= TAP_MIN_TAPS:
            intervals = [
                self._tap_times[i] - self._tap_times[i - 1]
                for i in range(1, len(self._tap_times))
            ]
            avg = sum(intervals) / len(intervals)
            new_bpm = int(round(60.0 / avg))
            self.bpm = new_bpm
            return self.bpm
        return None

    # ── Play / Stop ───────────────────────────────────────────────────────────

    def stop_and_reset(self):
        self.playing         = False
        self.current_step    = 0
        self.pending_pattern = None
        self.reset_chain()
