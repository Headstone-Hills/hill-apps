"""HillSequencer — application state.

Merges HillBeat's pattern/transport state with HillChord's per-instrument
effects. A single AppState is created in main.py and threaded through the
input -> sequencer -> audio -> ui pipeline.

Layout:
  * 8 tracks, each = {name, sound(sid), note(midi), muted, volume, effects}
  * patterns = NUM_PATTERNS x NUM_TRACKS x NUM_STEPS x [active:bool, velocity:int]
"""

from __future__ import annotations

import copy
import json
import os
from dataclasses import dataclass

import config
from config import (
    NUM_TRACKS, NUM_STEPS, NUM_PATTERNS,
    DEFAULT_BPM, DEFAULT_SWING, DEFAULT_VELOCITY,
    DEFAULT_VOICE_NAMES, DEFAULT_TRACK_NOTES, DEFAULT_TRACK_VOLUME,
    VELOCITY_MIN, VELOCITY_MAX, NOTE_MIN, NOTE_MAX, STATE_PATH,
)


@dataclass
class EffectsState:
    """Per-track toggle + wet/dry state (HillChord's effects, baked per note)."""
    reverb: bool = False
    delay: bool = False
    chorus: bool = False
    wetdry: int = 0          # one of config.WETDRY_STEPS
    crush_bits: int = 16     # bit depth (16 = clean); config.CRUSH_BITS_STEPS
    crush_down: int = 1      # downsample factor (1 = off); config.CRUSH_DOWN_STEPS

    def crushing(self) -> bool:
        return self.crush_bits < 16 or self.crush_down > 1

    def any_active(self) -> bool:
        return self.crushing() or (
            (self.reverb or self.delay or self.chorus) and self.wetdry > 0)

    def to_dict(self):
        return {
            "reverb": self.reverb, "delay": self.delay, "chorus": self.chorus,
            "wetdry": self.wetdry,
            "crush_bits": self.crush_bits, "crush_down": self.crush_down,
        }

    @classmethod
    def from_dict(cls, d):
        d = d or {}
        return cls(
            reverb=bool(d.get("reverb", False)),
            delay=bool(d.get("delay", False)),
            chorus=bool(d.get("chorus", False)),
            wetdry=int(d.get("wetdry", 0)),
            crush_bits=int(d.get("crush_bits", 16)),
            crush_down=int(d.get("crush_down", 1)),
        )


# Diatonic scale semitone offsets, root..octave inclusive (8 notes = 8 tracks).
_MAJOR_OFFSETS = [0, 2, 4, 5, 7, 9, 11, 12]
_MINOR_OFFSETS = [0, 2, 3, 5, 7, 8, 10, 12]


def _empty_pattern():
    """Fresh NUM_TRACKS x NUM_STEPS grid: each step [active:bool, velocity:int]."""
    return [[[False, DEFAULT_VELOCITY] for _ in range(NUM_STEPS)]
            for _ in range(NUM_TRACKS)]


def _default_track(index):
    return {
        "name":    DEFAULT_VOICE_NAMES[index],
        "sound":   None,                       # HillChord sid (relpath / dir::stem)
        "note":    DEFAULT_TRACK_NOTES[index],  # MIDI note this track plays
        "muted":   False,
        "volume":  DEFAULT_TRACK_VOLUME,
        "effects": EffectsState(),
    }


class AppState:
    """Single source of truth for all mutable application data."""

    def __init__(self):
        # Transport
        self.bpm             = DEFAULT_BPM
        self.swing           = DEFAULT_SWING
        self.current_pattern = 0
        self.metronome_on    = False
        self.chain_enabled   = False
        self.chain           = []          # list of {"pattern": int, "repeats": int}

        # Key / scale — used by the "spread key across tracks" command
        self.key_root        = 0           # pitch class 0-11 (C=0)
        self.key_minor       = False
        self.key_octave      = 3           # base octave of the LOW root (T8)

        # Background loop player (HillBeat amenity)
        self.loop_enabled    = False
        self.loop_file       = None
        self.loop_native_bpm = None

        # Sound-library favorites: HillChord sids
        self.favorites       = []

        # Tracks
        self.tracks = [_default_track(i) for i in range(NUM_TRACKS)]

        # Patterns
        self.patterns = [_empty_pattern() for _ in range(NUM_PATTERNS)]

        # Runtime-only (not persisted)
        self._clipboard = None

    # ── Step helpers ──────────────────────────────────────────────────────────
    def get_step(self, pattern, track, step):
        return self.patterns[pattern][track][step]

    def toggle_step(self, pattern, track, step):
        s = self.patterns[pattern][track][step]
        s[0] = not s[0]

    def set_step_velocity(self, pattern, track, step, vel):
        self.patterns[pattern][track][step][1] = max(VELOCITY_MIN, min(VELOCITY_MAX, vel))

    def clear_pattern(self, pattern):
        self.patterns[pattern] = _empty_pattern()

    def copy_pattern(self, pattern):
        self._clipboard = copy.deepcopy(self.patterns[pattern])

    def paste_pattern(self, pattern):
        if self._clipboard is not None:
            self.patterns[pattern] = copy.deepcopy(self._clipboard)

    # ── Track helpers ─────────────────────────────────────────────────────────
    def set_track_volume(self, track, vol):
        self.tracks[track]["volume"] = max(0.0, min(1.0, round(vol, 2)))

    def transpose_track(self, track, delta):
        n = self.tracks[track]["note"] + delta
        self.tracks[track]["note"] = max(NOTE_MIN, min(NOTE_MAX, n))

    def clear_instruments(self):
        """Unassign the instrument from every track (keeps names + notes)."""
        for t in self.tracks:
            t["sound"] = None

    # ── Key / scale spread ────────────────────────────────────────────────────
    def key_label(self):
        from theory.notes import CHROMATIC
        return f"{CHROMATIC[self.key_root % 12]}{'m' if self.key_minor else ' maj'}"

    def key_spread_notes(self):
        """Diatonic notes laid out high->low across the tracks: T1 = high root
        (one octave up), the last track = low root, the rest the scale degrees
        descending in between."""
        from config import NUM_TRACKS, NOTE_MIN, NOTE_MAX
        offs = _MINOR_OFFSETS if self.key_minor else _MAJOR_OFFSETS
        base = (self.key_octave + 1) * 12 + (self.key_root % 12)   # low root
        n = len(offs)
        out = []
        for i in range(NUM_TRACKS):
            idx = (n - 1) - i if i < n else 0   # T1 highest ... last track lowest
            note = base + offs[max(0, idx)]
            out.append(max(NOTE_MIN, min(NOTE_MAX, note)))
        return out

    def apply_key_spread(self):
        """Write the key spread into every track's note."""
        for i, note in enumerate(self.key_spread_notes()):
            self.tracks[i]["note"] = note

    # ── Serialization ─────────────────────────────────────────────────────────
    def to_dict(self):
        return {
            "bpm":             self.bpm,
            "swing":           self.swing,
            "current_pattern": self.current_pattern,
            "metronome_on":    self.metronome_on,
            "chain_enabled":   self.chain_enabled,
            "chain":           copy.deepcopy(self.chain),
            "key_root":        self.key_root,
            "key_minor":       self.key_minor,
            "key_octave":      self.key_octave,
            "loop_enabled":    self.loop_enabled,
            "loop_file":       self.loop_file,
            "loop_native_bpm": self.loop_native_bpm,
            "favorites":       list(self.favorites),
            "tracks": [
                {"name": t["name"], "sound": t["sound"], "note": t["note"],
                 "muted": t["muted"], "volume": t["volume"],
                 "effects": t["effects"].to_dict()}
                for t in self.tracks
            ],
            "patterns": [{"steps": copy.deepcopy(p)} for p in self.patterns],
        }

    def from_dict(self, d):
        self.bpm             = d.get("bpm",             DEFAULT_BPM)
        self.swing           = d.get("swing",           DEFAULT_SWING)
        self.current_pattern = d.get("current_pattern", 0)
        self.metronome_on    = d.get("metronome_on",    False)
        self.chain_enabled   = d.get("chain_enabled",   False)
        self.chain           = d.get("chain",           [])
        self.key_root        = int(d.get("key_root",    0))
        self.key_minor       = bool(d.get("key_minor",  False))
        self.key_octave      = int(d.get("key_octave",  3))
        self.loop_enabled    = d.get("loop_enabled",    False)
        self.loop_file       = d.get("loop_file",       None)
        self.loop_native_bpm = d.get("loop_native_bpm", None)
        self.favorites       = d.get("favorites",       [])

        raw_tracks = d.get("tracks", [])
        for i in range(NUM_TRACKS):
            if i < len(raw_tracks):
                rt = raw_tracks[i]
                self.tracks[i]["name"]    = rt.get("name", DEFAULT_VOICE_NAMES[i])
                self.tracks[i]["sound"]   = rt.get("sound", None)
                self.tracks[i]["note"]    = int(rt.get("note", DEFAULT_TRACK_NOTES[i]))
                self.tracks[i]["muted"]   = bool(rt.get("muted", False))
                self.tracks[i]["volume"]  = float(rt.get("volume", DEFAULT_TRACK_VOLUME))
                self.tracks[i]["effects"] = EffectsState.from_dict(rt.get("effects"))

        raw_pats = d.get("patterns", [])
        for pi in range(NUM_PATTERNS):
            if pi < len(raw_pats):
                steps = raw_pats[pi].get("steps", _empty_pattern())
                for ti in range(NUM_TRACKS):
                    for si in range(NUM_STEPS):
                        try:
                            cell = steps[ti][si]
                            self.patterns[pi][ti][si] = [bool(cell[0]), int(cell[1])]
                        except (IndexError, TypeError):
                            self.patterns[pi][ti][si] = [False, DEFAULT_VELOCITY]

    # ── Named sequences (full musical state minus global favorites) ───────────
    def to_sequence_dict(self, name):
        d = self.to_dict()
        d.pop("favorites", None)
        d["name"] = name
        return d

    def load_sequence_dict(self, d):
        favs = list(self.favorites)
        self.from_dict(d)
        self.favorites = favs

    def save(self):
        os.makedirs(os.path.dirname(STATE_PATH), exist_ok=True)
        with open(STATE_PATH, "w") as f:
            json.dump(self.to_dict(), f, indent=2)

    def load(self):
        if os.path.exists(STATE_PATH):
            try:
                with open(STATE_PATH) as f:
                    self.from_dict(json.load(f))
                return True
            except Exception as e:
                print(f"[state] Failed to load state: {e}")
        return False
