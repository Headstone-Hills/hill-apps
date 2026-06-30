"""Central application state.

A single AppState instance is created in main.py and threaded through the
input -> actions -> audio -> ui pipeline. No module keeps its own globals;
they read and mutate this object.
"""

from dataclasses import dataclass, field

import config


@dataclass
class EffectsState:
    """Toggle + wet/dry state for the three effects (spec: R1/L1/R1+L1, R2)."""
    reverb: bool = False
    delay: bool = False
    chorus: bool = False
    wetdry: int = 0  # one of config.WETDRY_STEPS
    crush_bits: int = 16   # bit depth (16 = clean); from config.CRUSH_BITS_STEPS
    crush_down: int = 1    # downsample factor (1 = off); config.CRUSH_DOWN_STEPS

    def crushing(self) -> bool:
        return self.crush_bits < 16 or self.crush_down > 1

    def any_active(self) -> bool:
        return self.crushing() or (
            (self.reverb or self.delay or self.chorus) and self.wetdry > 0)


@dataclass
class AppState:
    # Performance state
    key: str = config.DEFAULT_KEY          # e.g. "C", "G", "D" (circle of fifths)
    minor: bool = False                    # major/minor tonality of the key
    mode: str = config.DEFAULT_MODE        # "chord" | "note"
    bpm: int = config.BPM_DEFAULT
    sound: str = ""                        # selected sound name (folder under SAMPLE_PATH)
    favorites: list = field(default_factory=list)   # sids of favorited sounds

    effects: EffectsState = field(default_factory=EffectsState)

    # Runtime-only flags (not persisted)
    metronome_on: bool = False
    screen: str = "play"                   # "play" | "library"
    now_playing: str = ""                  # current chord/note name for the HUD
    octave: int = 0                        # octave offset (Function + D-Up/Down)
    arp: str = "off"                       # arp mode: off/up/down/bounce/random
    help: bool = False                     # cheat-sheet overlay (Start+Select+L1+R1)
    help_scroll: int = 0                   # cheat-sheet scroll offset
    sustain: bool = False                  # sustain pedal (SELECT hold)

    # ---- Persistence helpers (only the saved subset) ----
    def to_dict(self) -> dict:
        return {
            "key": self.key,
            "minor": self.minor,
            "mode": self.mode,
            "bpm": self.bpm,
            "sound": self.sound,
            "favorites": list(self.favorites),
            "octave": self.octave,
            "arp": self.arp,
            "effects": {
                "reverb": self.effects.reverb,
                "delay": self.effects.delay,
                "chorus": self.effects.chorus,
                "wetdry": self.effects.wetdry,
                "crush_bits": self.effects.crush_bits,
                "crush_down": self.effects.crush_down,
            },
        }

    def load_dict(self, d: dict) -> None:
        self.key = d.get("key", self.key)
        self.minor = d.get("minor", self.minor)
        self.mode = d.get("mode", self.mode)
        self.bpm = d.get("bpm", self.bpm)
        self.sound = d.get("sound", self.sound)
        self.favorites = list(d.get("favorites", []))
        self.octave = d.get("octave", self.octave)
        arp = d.get("arp", self.arp)
        if isinstance(arp, bool):          # migrate old boolean arp flag
            arp = "up" if arp else "off"
        self.arp = arp
        fx = d.get("effects", {})
        self.effects.reverb = fx.get("reverb", False)
        self.effects.delay = fx.get("delay", False)
        self.effects.chorus = fx.get("chorus", False)
        self.effects.wetdry = fx.get("wetdry", 0)
        self.effects.crush_bits = fx.get("crush_bits", 8 if fx.get("bitcrush") else 16)
        self.effects.crush_down = fx.get("crush_down", 4 if fx.get("bitcrush") else 1)
