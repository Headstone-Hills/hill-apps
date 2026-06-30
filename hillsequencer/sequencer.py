"""HillSequencer — sequencer.

Drift-correcting playback thread (from HillBeat). On each active, non-muted step
it triggers that track's HillChord instrument at the track's note, scaling the
channel volume by (step velocity * track volume).
"""

from __future__ import annotations

import threading
import time

import config
from config import NUM_TRACKS, NUM_STEPS, VELOCITY_MAX


class Sequencer:
    def __init__(self, state, transport, rack):
        self.state     = state
        self.transport = transport
        self.rack      = rack          # TrackRack — engines[track]

        self._thread = None
        self._stop_event = threading.Event()
        self._step_callbacks = []

    # ── Playback ──────────────────────────────────────────────────────────────
    def start(self):
        if self._thread and self._thread.is_alive():
            return
        self._stop_event.clear()
        self.transport.playing = True
        self._thread = threading.Thread(target=self._run, daemon=True,
                                        name="HillSequencer-Seq")
        self._thread.start()

    def stop(self):
        self.transport.playing = False
        self._stop_event.set()
        if self._thread:
            self._thread.join(timeout=1.0)
            self._thread = None
        self.rack.stop_all()

    def toggle(self):
        if self.transport.playing:
            self.stop()
        else:
            self.start()

    def stop_and_reset(self):
        self.stop()
        self.transport.current_step = 0

    def add_step_callback(self, cb):
        self._step_callbacks.append(cb)

    # ── Internal thread ───────────────────────────────────────────────────────
    def _run(self):
        tr = self.transport
        start_time = time.perf_counter()
        step_count = 0

        while not self._stop_event.is_set():
            step_interval = tr.step_interval()
            swing_off     = tr.swing_offset(tr.current_step)

            expected_fire = start_time + step_count * step_interval + swing_off
            now = time.perf_counter()
            sleep_for = max(0.0, expected_fire - now)
            if sleep_for > 0:
                self._stop_event.wait(timeout=sleep_for)
                if self._stop_event.is_set():
                    break

            pat  = tr.current_pattern
            step = tr.current_step

            for ti in range(NUM_TRACKS):
                cell = self.state.patterns[pat][ti][step]
                active, velocity = cell[0], cell[1]
                track = self.state.tracks[ti]
                if active and not track["muted"]:
                    gain = (velocity / VELOCITY_MAX) * track.get("volume", 1.0)
                    try:
                        self.rack.engines[ti].play_step(
                            track["note"], track["effects"], tr.bpm, gain)
                    except Exception:
                        pass

            for cb in self._step_callbacks:
                try:
                    cb(step)
                except Exception:
                    pass

            next_step = (step + 1) % NUM_STEPS
            tr.current_step = next_step

            if next_step == 0:
                if tr.state.chain_enabled and tr.state.chain:
                    tr.advance_chain()
                else:
                    tr.commit_pending_pattern()

            step_count += 1

            new_interval = tr.step_interval()
            if abs(new_interval - step_interval) > 1e-6:
                start_time = time.perf_counter() - step_count * new_interval
