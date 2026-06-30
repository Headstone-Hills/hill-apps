"""
HillBeat — sequencer.py
Drift-correcting playback thread; fires samples via pygame mixer.
"""

import os
import threading
import time
import pygame

from constants import NUM_VOICES, NUM_STEPS, VELOCITY_MAX, SAMPLE_ROOT


class Sequencer:
    """
    Runs a dedicated thread that advances through steps at the correct
    BPM tempo, firing pygame Sound objects per active step.

    The thread uses a drift-correcting loop:
        expected_fire = start_time + step_count * step_interval + swing_offset
        sleep_for = max(0, expected_fire - time.time())
    so accumulated drift is self-correcting.
    """

    def __init__(self, state, transport):
        self.state     = state
        self.transport = transport

        # Loaded pygame.Sound objects, indexed [voice]
        self._sounds: list[pygame.mixer.Sound | None] = [None] * NUM_VOICES

        # One pygame channel per voice
        self._channels: list[pygame.mixer.Channel | None] = [None] * NUM_VOICES

        self._lock   = threading.Lock()
        self._thread = None
        self._stop_event = threading.Event()

        # Listeners called on each step fire (for UI playhead sync)
        # signature: callback(step_index: int)
        self._step_callbacks: list = []

    # ── Sound loading ─────────────────────────────────────────────────────────

    def load_voice(self, voice_index: int, filepath: str | None):
        """Load (or unload) a WAV sample for a voice.  Thread-safe."""
        with self._lock:
            if filepath is None:
                self._sounds[voice_index] = None
            else:
                try:
                    snd = pygame.mixer.Sound(filepath)
                    self._sounds[voice_index] = snd
                except Exception as e:
                    print(f"[sequencer] Failed to load {filepath}: {e}")
                    self._sounds[voice_index] = None

    def reload_all_voices(self):
        """Reload samples from current state.voices, resolving relative names via SAMPLE_ROOT.
        Searches recursively so samples in subdirectories (e.g. Cassette Drums/) are found."""
        for i, v in enumerate(self.state.voices):
            raw = v.get("sample")
            if raw is None:
                self.load_voice(i, None)
            elif os.path.isabs(raw):
                self.load_voice(i, raw)
            else:
                direct = os.path.join(SAMPLE_ROOT, raw)
                if os.path.isfile(direct):
                    self.load_voice(i, direct)
                else:
                    self.load_voice(i, self._find_sample(os.path.basename(raw)))

    def _find_sample(self, basename: str):
        """Search SAMPLE_ROOT recursively for a file matching basename."""
        for dirpath, _, filenames in os.walk(SAMPLE_ROOT):
            if basename in filenames:
                return os.path.join(dirpath, basename)
        return None

    # ── Playback ──────────────────────────────────────────────────────────────

    def start(self):
        if self._thread and self._thread.is_alive():
            return
        self._stop_event.clear()
        self.transport.playing = True
        self._thread = threading.Thread(
            target=self._run,
            daemon=True,
            name="HillBeat-Sequencer"
        )
        self._thread.start()

    def stop(self):
        self.transport.playing = False
        self._stop_event.set()
        if self._thread:
            self._thread.join(timeout=1.0)
            self._thread = None

    def toggle(self):
        if self.transport.playing:
            self.stop()
        else:
            self.start()

    def stop_and_reset(self):
        self.stop()
        self.transport.current_step = 0

    # ── Callback registration ─────────────────────────────────────────────────

    def add_step_callback(self, cb):
        self._step_callbacks.append(cb)

    # ── Internal thread ───────────────────────────────────────────────────────

    def _run(self):
        tr = self.transport
        start_time = time.perf_counter()
        step_count = 0                  # total steps fired since thread start

        while not self._stop_event.is_set():
            step_interval = tr.step_interval()
            swing_off     = tr.swing_offset(tr.current_step)

            # When to fire this step
            expected_fire = start_time + step_count * step_interval + swing_off
            now = time.perf_counter()
            sleep_for = max(0.0, expected_fire - now)
            if sleep_for > 0:
                self._stop_event.wait(timeout=sleep_for)
                if self._stop_event.is_set():
                    break

            pat  = tr.current_pattern
            step = tr.current_step

            # Fire active, non-muted voices
            with self._lock:
                for vi in range(NUM_VOICES):
                    cell = self.state.patterns[pat][vi][step]
                    active, velocity = cell[0], cell[1]
                    if active and not self.state.voices[vi]["muted"]:
                        snd = self._sounds[vi]
                        if snd is not None:
                            ch = pygame.mixer.Channel(vi)
                            voice_vol = self.state.voices[vi].get("volume", 1.0)
                            vol = (velocity / VELOCITY_MAX) * voice_vol
                            ch.set_volume(vol)
                            ch.play(snd)

            # Notify UI callbacks (non-blocking)
            for cb in self._step_callbacks:
                try:
                    cb(step)
                except Exception:
                    pass

            # Advance step
            next_step = (step + 1) % NUM_STEPS
            tr.current_step = next_step

            # At loop boundary: handle pending pattern / chain
            if next_step == 0:
                if tr.state.chain_enabled and tr.state.chain:
                    tr.advance_chain()
                else:
                    tr.commit_pending_pattern()

            step_count += 1

            # If BPM changed, re-anchor the timeline so we don't drift weirdly.
            # We recalculate start_time based on how many steps we have consumed
            # at the new interval.
            new_interval = tr.step_interval()
            if abs(new_interval - step_interval) > 1e-6:
                start_time = time.perf_counter() - step_count * new_interval
