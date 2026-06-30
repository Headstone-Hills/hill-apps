"""
HillBeat — state.py
Application state container, JSON serialization/deserialization.
"""

import json
import copy
import os
from constants import (
    NUM_VOICES, NUM_STEPS, NUM_PATTERNS,
    DEFAULT_BPM, DEFAULT_SWING, DEFAULT_VELOCITY,
    DEFAULT_VOICE_NAMES, STATE_FILE
)


def _empty_pattern():
    """Return a fresh 4×16 step grid: each step is [active:bool, velocity:int]."""
    return [[[False, DEFAULT_VELOCITY] for _ in range(NUM_STEPS)]
            for _ in range(NUM_VOICES)]


def _default_voice(index):
    return {
        "name":   DEFAULT_VOICE_NAMES[index],
        "sample": None,
        "muted":  False,
        "volume": 1.0,
    }


class AppState:
    """Single source of truth for all mutable application data."""

    def __init__(self):
        # Transport
        self.bpm             = DEFAULT_BPM
        self.swing           = DEFAULT_SWING
        self.current_pattern = 0
        self.loop_enabled    = False
        self.loop_file       = None
        self.loop_native_bpm = None
        self.chain_enabled   = False
        self.chain           = []          # list of {"pattern": int, "repeats": int}

        # Voices
        self.voices = [_default_voice(i) for i in range(NUM_VOICES)]

        # Patterns: NUM_PATTERNS × NUM_VOICES × NUM_STEPS × 2
        self.patterns = [_empty_pattern() for _ in range(NUM_PATTERNS)]

        # Favorites: relative WAV paths (from SAMPLE_ROOT) that are starred
        self.favorites: list = []

        # Runtime-only (not persisted)
        self._clipboard = None             # copy/paste pattern buffer

    # ── Helpers ───────────────────────────────────────────────────────────────

    def get_step(self, pattern: int, voice: int, step: int):
        return self.patterns[pattern][voice][step]

    def set_step_active(self, pattern: int, voice: int, step: int, active: bool):
        self.patterns[pattern][voice][step][0] = active

    def toggle_step(self, pattern: int, voice: int, step: int):
        s = self.patterns[pattern][voice][step]
        s[0] = not s[0]

    def set_step_velocity(self, pattern: int, voice: int, step: int, vel: int):
        from constants import VELOCITY_MIN, VELOCITY_MAX
        self.patterns[pattern][voice][step][1] = max(VELOCITY_MIN, min(VELOCITY_MAX, vel))

    def set_voice_volume(self, voice: int, vol: float):
        from constants import VOICE_VOLUME_MIN, VOICE_VOLUME_MAX
        self.voices[voice]["volume"] = max(VOICE_VOLUME_MIN, min(VOICE_VOLUME_MAX, round(vol, 3)))

    def copy_pattern(self, pattern: int):
        self._clipboard = copy.deepcopy(self.patterns[pattern])

    def paste_pattern(self, pattern: int):
        if self._clipboard is not None:
            self.patterns[pattern] = copy.deepcopy(self._clipboard)

    # ── Serialization ─────────────────────────────────────────────────────────

    def to_dict(self):
        return {
            "bpm":             self.bpm,
            "swing":           self.swing,
            "current_pattern": self.current_pattern,
            "loop_enabled":    self.loop_enabled,
            "loop_file":       self.loop_file,
            "loop_native_bpm": self.loop_native_bpm,
            "chain_enabled":   self.chain_enabled,
            "chain":           copy.deepcopy(self.chain),
            "favorites": list(self.favorites),
            "voices": [
                {"name": v["name"], "sample": v["sample"],
                 "muted": v["muted"], "volume": v.get("volume", 1.0)}
                for v in self.voices
            ],
            "patterns": [
                {"steps": copy.deepcopy(p)}
                for p in self.patterns
            ],
        }

    def from_dict(self, d: dict):
        self.bpm             = d.get("bpm",             DEFAULT_BPM)
        self.swing           = d.get("swing",           DEFAULT_SWING)
        self.current_pattern = d.get("current_pattern", 0)
        self.loop_enabled    = d.get("loop_enabled",    False)
        self.loop_file       = d.get("loop_file",       None)
        self.loop_native_bpm = d.get("loop_native_bpm", None)
        self.chain_enabled   = d.get("chain_enabled",   False)
        self.chain           = d.get("chain",           [])
        self.favorites       = list(d.get("favorites",   []))

        raw_voices = d.get("voices", [])
        for i in range(NUM_VOICES):
            if i < len(raw_voices):
                rv = raw_voices[i]
                self.voices[i]["name"]   = rv.get("name",   DEFAULT_VOICE_NAMES[i])
                self.voices[i]["sample"] = rv.get("sample", None)
                self.voices[i]["muted"]  = rv.get("muted",  False)
                self.voices[i]["volume"] = float(rv.get("volume", 1.0))

        raw_pats = d.get("patterns", [])
        for pi in range(NUM_PATTERNS):
            if pi < len(raw_pats):
                rp = raw_pats[pi]
                steps = rp.get("steps", _empty_pattern())
                for vi in range(NUM_VOICES):
                    for si in range(NUM_STEPS):
                        try:
                            cell = steps[vi][si]
                            self.patterns[pi][vi][si] = [bool(cell[0]), int(cell[1])]
                        except (IndexError, TypeError):
                            self.patterns[pi][vi][si] = [False, DEFAULT_VELOCITY]

    def save(self):
        os.makedirs(os.path.dirname(STATE_FILE), exist_ok=True)
        with open(STATE_FILE, "w") as f:
            json.dump(self.to_dict(), f, indent=2)

    def load(self):
        if os.path.exists(STATE_FILE):
            try:
                with open(STATE_FILE, "r") as f:
                    self.from_dict(json.load(f))
                return True
            except Exception as e:
                print(f"[state] Failed to load state: {e}")
        return False
