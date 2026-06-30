"""HillBand — application state.

8 tracks split into two groups:
  * Tracks 0–3 (melodic): pitched sampler + chord_type + scale_mode + effects
  * Tracks 4–7 (drums):   one-shot percussion, fixed names, no chord/scale/effects

A single AppState is created in main.py and threaded through the pipeline.
"""

from __future__ import annotations

import copy
import json
import os
from dataclasses import dataclass

import config
from config import (
    NUM_TRACKS, NUM_STEPS, NUM_PATTERNS, DRUM_TRACK_OFFSET,
    DEFAULT_BPM, DEFAULT_SWING, DEFAULT_VELOCITY,
    DEFAULT_VOICE_NAMES, DEFAULT_TRACK_NOTES, DEFAULT_TRACK_VOLUME,
    VELOCITY_MIN, VELOCITY_MAX, NOTE_MIN, NOTE_MAX, STATE_PATH,
    DRUM_VOICE_NAMES,
)


@dataclass
class EffectsState:
    """Per-melodic-track toggle + wet/dry (baked at render time, zero runtime cost)."""
    reverb:     bool = False
    delay:      bool = False
    chorus:     bool = False
    wetdry:     int  = 0
    crush_bits: int  = 16
    crush_down: int  = 1

    def crushing(self):
        return self.crush_bits < 16 or self.crush_down > 1

    def any_active(self):
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


# Null-effects sentinel used when firing drum steps through the same Mixer API.
_NO_FX = EffectsState()


def _empty_pattern():
    return [[[False, DEFAULT_VELOCITY] for _ in range(NUM_STEPS)]
            for _ in range(NUM_TRACKS)]


def _default_track(index):
    is_drum = index >= DRUM_TRACK_OFFSET
    name = (DRUM_VOICE_NAMES[index - DRUM_TRACK_OFFSET] if is_drum
            else DEFAULT_VOICE_NAMES[index])
    t = {
        "name":    name,
        "sound":   None,
        "note":    DEFAULT_TRACK_NOTES[index],
        "muted":   False,
        "volume":  DEFAULT_TRACK_VOLUME,
        "is_drum": is_drum,
    }
    if not is_drum:
        t["effects"]    = EffectsState()
        t["chord_type"] = "unison"
        t["scale_mode"] = "none"
    return t


class AppState:
    """Single source of truth for all mutable application data."""

    def __init__(self):
        self.bpm             = DEFAULT_BPM
        self.swing           = DEFAULT_SWING
        self.current_pattern = 0
        self.metronome_on    = False
        self.chain_enabled   = False
        self.chain           = []
        self.loop_enabled    = False
        self.loop_file       = None
        self.loop_native_bpm = None
        self.favorites       = []
        self.tracks          = [_default_track(i) for i in range(NUM_TRACKS)]
        self.patterns        = [_empty_pattern() for _ in range(NUM_PATTERNS)]
        self._clipboard      = None

    def is_drum_track(self, ti):
        return bool(self.tracks[ti]["is_drum"])

    def no_fx(self):
        return _NO_FX

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
        if self.is_drum_track(track):
            return
        n = self.tracks[track]["note"] + delta
        self.tracks[track]["note"] = max(NOTE_MIN, min(NOTE_MAX, n))

    def clear_instruments(self):
        for t in self.tracks:
            t["sound"] = None

    # ── Serialization ─────────────────────────────────────────────────────────
    def to_dict(self):
        tracks_out = []
        for t in self.tracks:
            td = {
                "name": t["name"], "sound": t["sound"], "note": t["note"],
                "muted": t["muted"], "volume": t["volume"], "is_drum": t["is_drum"],
            }
            if not t["is_drum"]:
                td["effects"]    = t["effects"].to_dict()
                td["chord_type"] = t.get("chord_type", "unison")
                td["scale_mode"] = t.get("scale_mode", "none")
            tracks_out.append(td)
        return {
            "bpm":             self.bpm,
            "swing":           self.swing,
            "current_pattern": self.current_pattern,
            "metronome_on":    self.metronome_on,
            "chain_enabled":   self.chain_enabled,
            "chain":           copy.deepcopy(self.chain),
            "loop_enabled":    self.loop_enabled,
            "loop_file":       self.loop_file,
            "loop_native_bpm": self.loop_native_bpm,
            "favorites":       list(self.favorites),
            "tracks":          tracks_out,
            "patterns":        [{"steps": copy.deepcopy(p)} for p in self.patterns],
        }

    def from_dict(self, d):
        self.bpm             = d.get("bpm",             DEFAULT_BPM)
        self.swing           = d.get("swing",           DEFAULT_SWING)
        self.current_pattern = d.get("current_pattern", 0)
        self.metronome_on    = d.get("metronome_on",    False)
        self.chain_enabled   = d.get("chain_enabled",   False)
        self.chain           = d.get("chain",           [])
        self.loop_enabled    = d.get("loop_enabled",    False)
        self.loop_file       = d.get("loop_file",       None)
        self.loop_native_bpm = d.get("loop_native_bpm", None)
        self.favorites       = d.get("favorites",       [])

        raw_tracks = d.get("tracks", [])
        for i in range(NUM_TRACKS):
            if i < len(raw_tracks):
                rt = raw_tracks[i]
                self.tracks[i]["name"]   = rt.get("name",   DEFAULT_VOICE_NAMES[i])
                self.tracks[i]["sound"]  = rt.get("sound",  None)
                self.tracks[i]["note"]   = int(rt.get("note", DEFAULT_TRACK_NOTES[i]))
                self.tracks[i]["muted"]  = bool(rt.get("muted",  False))
                self.tracks[i]["volume"] = float(rt.get("volume", DEFAULT_TRACK_VOLUME))
                if not self.tracks[i]["is_drum"]:
                    self.tracks[i]["effects"]    = EffectsState.from_dict(rt.get("effects"))
                    self.tracks[i]["chord_type"] = rt.get("chord_type", "unison")
                    self.tracks[i]["scale_mode"] = rt.get("scale_mode", "none")

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
