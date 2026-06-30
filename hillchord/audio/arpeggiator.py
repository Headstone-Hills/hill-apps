"""Arpeggiator (toggled/cycled by L2 + R2).

Modes: off -> up -> down -> bounce -> random (cycled by L2+R2). Plays the held
chord's notes locked to the transport's beat grid at config.ARP_DIVISION
steps/beat, as one-shots through the mixer (recorded into loops too).

To avoid a first-time render stutter mid-arpeggio, set_notes() pre-warms the
render cache for the chord's notes up front (only the first-ever render of a
given note+effect actually computes; after that it's a fast disk/RAM hit).
"""

import random
import time

import config

MODES = ("off", "up", "down", "bounce", "bounce4", "random")


def _sequence(n, mode):
    if n <= 1:
        return [0]
    if mode == "down":
        return list(range(n - 1, -1, -1))
    if mode == "bounce":
        return list(range(n)) + list(range(n - 2, 0, -1))
    if mode == "bounce4":
        # 4-step cycle: up to 4 notes, bouncing back for any beyond the chord.
        # 3-note C  -> 0,1,2,1   (c-e-g-e)
        # 4-note C7 -> 0,1,2,3   (c-e-g-b)
        return [max(0, min(n - 1, i if i < n else 2 * (n - 1) - i)) for i in range(4)]
    return list(range(n))                    # "up" (and fallback)


class Arpeggiator:
    def __init__(self, mixer, transport, looper=None):
        self.mixer = mixer
        self.transport = transport
        self.looper = looper
        self.notes = []
        self.mode = "up"
        self.fx = None
        self.bpm = config.BPM_DEFAULT
        self._seq = [0]
        self._step = 0
        self._next_t = 0.0
        self._last_idx = -1
        self._last_chan = None       # channel of the sounding note (to gate it)

    def set_notes(self, midis, fx, bpm, mode):
        new = list(midis)
        changed = new != self.notes or mode != self.mode
        self.notes = new
        self.mode = mode
        self.fx = fx
        self.bpm = bpm
        self._seq = _sequence(len(new), mode)
        # Always restart at the root on (re)trigger, regardless of where the
        # previous cycle ended.
        self._step = 0
        self._next_t = 0.0
        self._last_idx = -1
        if changed:
            self._prewarm()

    def clear(self):
        self.notes = []
        self.mixer.fade_index(self._last_chan, config.ARP_GATE_MS)
        self._last_chan = None

    def _prewarm(self):
        """Render this chord's notes once so the arpeggio never stutters mid-run.
        Only the first-ever render of each note+effect computes; the rest are
        fast cache hits."""
        for m in self.notes:
            self.mixer._render(m, self.fx, self.bpm, loop=False)

    def _pick(self):
        if self.mode == "random":
            if len(self.notes) <= 1:
                return 0
            idx = random.randrange(len(self.notes))
            if idx == self._last_idx:        # avoid immediate repeats
                idx = (idx + 1) % len(self.notes)
            return idx
        return self._seq[self._step % len(self._seq)]

    def update(self):
        if not self.notes:
            return
        now = time.monotonic()
        if now < self._next_t:
            return
        step = self.transport.beat_len(self.bpm) / config.ARP_DIVISION
        idx = self._pick()
        self._last_idx = idx
        note = self.notes[idx]
        # Gate the previous note so notes stay distinct instead of sustaining.
        self.mixer.fade_index(self._last_chan, config.ARP_GATE_MS)
        self._last_chan = self.mixer.play_oneshot(note, self.fx, self.bpm)
        if self.looper is not None:
            self.looper.on_trigger([note], self.fx, self.bpm, loop=False)
        self._step += 1
        self._next_t = self.transport.grid_next(now, step)
