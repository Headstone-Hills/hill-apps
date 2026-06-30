"""HillSequencer — TrackRack.

Owns one HillChord Mixer per track, each confined to its own channel band. Loads
instruments by sid (the stable id stored in state.tracks[i]["sound"]), warms the
track's note, and pumps each engine's delay-echo queue once per frame.
"""

from __future__ import annotations

import os

import config
from audio.mixer import Mixer
from library_overlay import resolve_sid   # (kind, payload) for a saved sid


class TrackRack:
    def __init__(self, state):
        self.state = state
        self.engines = [
            Mixer(channel_base=i * config.CHANNELS_PER_TRACK,
                  channel_count=config.CHANNELS_PER_TRACK)
            for i in range(config.NUM_TRACKS)
        ]

    # ── Loading ──────────────────────────────────────────────────────────────
    def _load_into(self, engine, kind, payload, sid):
        if kind == "sound_multi":
            engine.load_files(payload, token=sid)
        else:
            engine.load_single(payload, token=sid)

    def assign(self, track_i, kind, payload, sid):
        """Assign a freshly-browsed instrument to a track and warm its note."""
        engine = self.engines[track_i]
        self._load_into(engine, kind, payload, sid)
        self.state.tracks[track_i]["sound"] = sid
        self._prewarm(track_i)

    def reload_all(self):
        """Reload every track's instrument from its saved sid (startup / load)."""
        for i, t in enumerate(self.state.tracks):
            sid = t.get("sound")
            if not sid:
                continue
            resolved = resolve_sid(config.SAMPLE_PATH, sid)
            if resolved is None:
                continue
            kind, payload = resolved
            try:
                self._load_into(self.engines[i], kind, payload, sid)
                self._prewarm(i)
            except Exception as e:
                print(f"[rack] could not restore track {i} sound '{sid}': {e}")

    def _prewarm(self, track_i):
        t = self.state.tracks[track_i]
        self.engines[track_i].prewarm_note(t["note"], t["effects"], self.state.bpm)

    # Public alias (call after a track's note/fx changes).
    def prewarm_track(self, track_i):
        if self.state.tracks[track_i].get("sound"):
            self._prewarm(track_i)

    def unload_all(self):
        """Clear every engine (after state.clear_instruments())."""
        for e in self.engines:
            e.clear()

    def prewarm_all(self):
        for i in range(config.NUM_TRACKS):
            if self.state.tracks[i].get("sound"):
                self._prewarm(i)

    # ── Runtime ──────────────────────────────────────────────────────────────
    def update(self):
        """Fire due delay echoes on every engine (call once per frame)."""
        for e in self.engines:
            e.update()

    def stop_all(self):
        for e in self.engines:
            e.stop_all()
