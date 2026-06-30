"""Looper (spec L2), with overdub layering.

Records trigger *events*, bakes them into one stereo buffer, and loops that as a
single Sound (no per-frame DSP -> glitch-free). Overdub records additional
passes aligned to the playing loop's phase and merges them into the buffer.

L2 is state-aware (press acts immediately, no double-tap deferral):
    IDLE --press--> RECORDING --press--> LOOPING
    LOOPING --press--> OVERDUB --press--> LOOPING (layer merged)
    (any) --hold 2s--> IDLE (cancel/clear)

Recording start snaps to the next beat (count-in); loop length is quantized to
whole beats; the loop re-times when BPM changes (retime()).

Notes are stamped with their actual hold duration so staccato playing is
reproduced accurately (not held as drones).  Callers pass a `key` to
on_trigger() then call on_release(key) when the voice stops.
"""

import copy
import time

import numpy as np
import pygame

import config
from audio import effects
from audio.mixer import DELAY_TAPS, DELAY_FEEDBACK

IDLE = "idle"
RECORDING = "recording"
LOOPING = "looping"
OVERDUB = "overdub"


class LoopRecorder:
    def __init__(self, mixer, transport):
        self.mixer = mixer
        self.transport = transport
        self.state = IDLE
        # Events: (t_off, midis, fx, bpm, loop, duration_or_None)
        self._events = []
        # Overdub events: (t_abs, midis, fx, bpm, loop, duration_or_None)
        self._overdub = []
        # Pending (key -> (t_on, midis, fx, bpm, loop)) for in-flight notes
        self._pending: dict = {}
        self._pending_od: dict = {}
        self._t0 = 0.0
        self._play_start = 0.0   # wall time the current buffer started looping
        self._length = 0.0
        self._sound = None
        self._base_buf = None    # baked buffer at its original tempo (re-timing)
        self._base_bpm = config.BPM_DEFAULT
        # Dedicated channel past the 12 voices + metronome.
        self.channel = pygame.mixer.Channel(config.NUM_VOICES + 1)

    # ---- recording hook (voice/arp note fires) ----
    def on_trigger(self, midis, fx, bpm, loop=True, key=None):
        """Record a note-on.  Pass `key` (e.g. button id) so on_release() can
        match it and stamp the note with its actual hold duration."""
        if self.state == RECORDING:
            t = time.monotonic() - self._t0
            allow = config.LOOP_EDGE_MS / 1000.0
            if t < -allow:
                return
            if t < 0:
                t = 0.0
            if key is not None:
                self._pending[key] = (t, list(midis), copy.copy(fx), bpm, loop)
            else:
                # No key: commit immediately with unknown duration
                self._events.append((t, list(midis), copy.copy(fx), bpm, loop, None))
        elif self.state == OVERDUB:
            t_abs = time.monotonic()
            if key is not None:
                self._pending_od[key] = (t_abs, list(midis), copy.copy(fx), bpm, loop)
            else:
                self._overdub.append(
                    (t_abs, list(midis), copy.copy(fx), bpm, loop, None))

    def on_release(self, key):
        """Record a note-off; stamps the note with its real hold duration."""
        now = time.monotonic()
        if self.state == RECORDING and key in self._pending:
            t_on, midis, fx, bpm, loop = self._pending.pop(key)
            duration = max(0.02, now - self._t0 - t_on)
            self._events.append((t_on, midis, fx, bpm, loop, duration))
        elif self.state == OVERDUB and key in self._pending_od:
            t_abs_on, midis, fx, bpm, loop = self._pending_od.pop(key)
            duration = max(0.02, now - t_abs_on)
            self._overdub.append((t_abs_on, midis, fx, bpm, loop, duration))

    # ---- L2 press: cycle record -> loop -> overdub -> loop ----
    def toggle_record(self, bpm=config.BPM_DEFAULT):
        if self.state == IDLE:
            self.state = RECORDING
            self._events = []
            self._pending.clear()
            # Snap t0 to the nearest beat (prev or next) so the loop downbeat
            # locks to the grid regardless of which side of the beat L2 lands on.
            self._t0 = self.transport.grid_nearest(time.monotonic(),
                                                    self.transport.beat_len(bpm))
        elif self.state == RECORDING:
            raw = max(time.monotonic() - self._t0, 0.05)
            self._length = self.transport.quantize_beats(raw, bpm)
            self._base_bpm = bpm
            # Commit any still-held notes (button down when loop end is pressed).
            now = time.monotonic()
            for key, (t_on, midis, fx, b, loop) in list(self._pending.items()):
                duration = max(0.02, now - self._t0 - t_on)
                self._events.append((t_on, midis, fx, b, loop, duration))
            self._pending.clear()
            self._bake()
            if self._sound is None:         # nothing played -> no loop
                self.state = IDLE
                return
            self.state = LOOPING
            self._play(self._sound)
        elif self.state == LOOPING:
            self.state = OVERDUB
            self._overdub = []
            self._pending_od.clear()
        elif self.state == OVERDUB:
            self._merge_overdub(bpm)
            self.state = LOOPING

    # ---- L2 hold 2s: cancel/clear ----
    def cancel(self):
        self.channel.stop()
        self.state = IDLE
        self._events = []
        self._overdub = []
        self._pending.clear()
        self._pending_od.clear()
        self._length = 0.0
        self._sound = None
        self._base_buf = None

    def retime(self, new_bpm):
        """Re-stretch the loop to a new tempo (pitch preserved) on BPM change."""
        if self.state not in (LOOPING, OVERDUB) or self._base_buf is None:
            return
        if new_bpm == self._base_bpm:
            stretched = self._base_buf
        else:
            s = self._base_bpm / max(new_bpm, 1)
            stretched = effects._time_stretch(self._base_buf, s)
        self._play(pygame.sndarray.make_sound(np.ascontiguousarray(stretched)))

    def update(self):
        """Nothing per-frame: a baked loop plays itself on its channel."""

    # ---- internals ----
    def _play(self, snd):
        self._sound = snd
        self.channel.set_volume(1.0)
        self.channel.play(snd, loops=-1)
        self._play_start = time.monotonic()

    def _bake(self):
        rate = config.SAMPLE_RATE
        n = max(1, int(self._length * rate))
        edge = int(config.LOOP_EDGE_MS / 1000.0 * rate)
        buf = np.zeros((n, 2), np.float32)
        if not self._events:
            self._sound = self._base_buf = None
            return
        # Resolve sample positions first so gate lengths can be computed.
        positioned = []
        for t_off, midis, fx, bpm, loop, duration in self._events:
            pos = int(t_off * rate) % n
            if pos >= n - edge:
                pos = 0
            pos = _quantize_pos(pos, n, bpm, rate)
            positioned.append((pos, midis, fx, bpm, loop, duration))
        positioned.sort(key=lambda x: x[0])
        for i, (pos, midis, fx, bpm, loop, duration) in enumerate(positioned):
            if loop:
                # Gate: drone sustains until the next event starts (or loop
                # wraps back to the first event).  This prevents overlapping
                # sustained pads regardless of how long buttons were held.
                next_pos = positioned[(i + 1) % len(positioned)][0]
                gate_smp = (next_pos - pos) % n or n
                gate = gate_smp / rate
            else:
                gate = duration  # one-shot: respect actual hold duration
            self._stamp(buf, pos, midis, fx, bpm, loop, gate)
        self._store_base(buf, self._base_bpm)
        self._play_aligned()                # gapless + phase-locked to the grid

    def _merge_overdub(self, bpm):
        if not self._overdub or self._base_buf is None:
            self._overdub = []
            return
        rate = config.SAMPLE_RATE
        # Work on _base_buf (canonical, unrolled, anchored to _t0) — never on the
        # phase-rolled playback copy.  Mixing into the rolled copy would corrupt
        # _base_buf's phase reference and cause drift on every subsequent overdub
        # or retime() call.
        cur = self._base_buf.astype(np.float32)
        n = len(cur)
        length_sec = n / rate
        positioned_od = []
        for t_abs, midis, fx, bp, loop, duration in self._overdub:
            pos = int(((t_abs - self._t0) % length_sec) * rate) % n
            pos = _quantize_pos(pos, n, bp, rate)
            positioned_od.append((pos, midis, fx, bp, loop, duration))
        positioned_od.sort(key=lambda x: x[0])
        for i, (pos, midis, fx, bp, loop, duration) in enumerate(positioned_od):
            if loop:
                next_pos = positioned_od[(i + 1) % len(positioned_od)][0]
                gate_smp = (next_pos - pos) % n or n
                gate = gate_smp / rate
            else:
                gate = duration
            self._stamp(cur, pos, midis, fx, bp, loop, gate)
        self._overdub = []
        self._store_base(cur, bpm)
        self._play_aligned()    # re-roll to current beat phase, gapless

    def _stamp(self, buf, pos, midis, fx, bpm, loop, duration=None):
        """Render & place a note (+ its delay echoes) into buf at pos.
        `duration` is the actual hold time in seconds; the PCM is faded out
        at that boundary so staccato notes don't sustain as drones."""
        rate = config.SAMPLE_RATE
        n = len(buf)
        echo_step = int(rate * 60.0 / max(bpm, 1)) if fx.delay else 0
        for midi in midis:
            # Always render one-shot: the buffer loops as a unit; per-note
            # loop=True PCM adds seamless-loop crossfade processing that makes
            # drone PCM longer and wrong here. Duration truncation handles length.
            snd = self.mixer._render(midi, fx, bpm, loop=False)
            if snd is None:
                continue
            arr = pygame.sndarray.array(snd).astype(np.float32)
            if arr.ndim == 1:
                arr = np.stack([arr, arr], axis=1)
            # Truncate to the note's actual hold duration with a short fade-out.
            if duration is not None:
                dur_smp = int(duration * rate)
                if dur_smp < len(arr):
                    arr = arr[:dur_smp].copy()
                    fade_smp = min(int(0.05 * rate), max(1, len(arr) // 4))
                    fade = np.linspace(1.0, 0.0, fade_smp, dtype=np.float32)
                    arr[-fade_smp:] *= fade[:, None]
            _add_wrap(buf, arr, pos)
            if echo_step:
                for k in range(1, DELAY_TAPS + 1):
                    gain = DELAY_FEEDBACK ** k
                    if gain < 0.02:
                        break
                    _add_wrap(buf, arr * gain, (pos + k * echo_step) % n)

    def _store_base(self, buf, bpm):
        peak = np.abs(buf).max()
        if peak > 32767.0:
            buf = buf * (32767.0 / peak)
        self._base_buf = np.ascontiguousarray(buf.astype(np.int16))
        self._base_bpm = bpm

    def _play_aligned(self):
        """Play the baked loop rolled to the current beat phase, so it's gapless
        AND its on-beat content coincides with the metronome grid."""
        rate = config.SAMPLE_RATE
        n = len(self._base_buf)
        length_sec = n / rate
        p = int(((time.monotonic() - self._t0) % length_sec) * rate) % n
        rolled = np.roll(self._base_buf, -p, axis=0)
        self._play(pygame.sndarray.make_sound(np.ascontiguousarray(rolled)))


def _quantize_pos(pos, n, bpm, rate):
    """Snap a sample offset to the nearest 1/LOOP_QUANTIZE_DIV of a beat, so
    notes captured into the loop land tight on the grid. No-op if disabled."""
    div = config.LOOP_QUANTIZE_DIV
    if not div:
        return pos
    step = rate * 60.0 / max(bpm, 1) / div      # quarter-beat in samples
    if step < 1:
        return pos
    return int(round(pos / step) * step) % n


def _add_wrap(buf, arr, pos):
    """Add `arr` into `buf` at `pos`, wrapping the tail to the start. Capped at
    one loop length so a long note can't compound onto itself."""
    n = len(buf)
    i = 0
    remaining = min(len(arr), n)
    while remaining > 0:
        end = min(pos + remaining, n)
        chunk = end - pos
        buf[pos:end] += arr[i:i + chunk]
        i += chunk
        remaining -= chunk
        pos = 0
