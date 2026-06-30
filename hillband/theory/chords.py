"""Chord type and scale mode definitions for HillBand melodic tracks."""

# Chord: semitone offsets from the root (root always included).
CHORD_TYPES = {
    "unison":   [0],
    "power":    [0, 7],
    "major":    [0, 4, 7],
    "minor":    [0, 3, 7],
    "sus2":     [0, 2, 7],
    "sus4":     [0, 5, 7],
    "dom7":     [0, 4, 7, 10],
    "maj7":     [0, 4, 7, 11],
    "min7":     [0, 3, 7, 10],
    "dim":      [0, 3, 6],
    "dim7":     [0, 3, 6, 9],
    "aug":      [0, 4, 8],
    "add9":     [0, 4, 7, 14],
    "minadd9":  [0, 3, 7, 14],
}

CHORD_TYPE_KEYS = list(CHORD_TYPES.keys())

# Scale: chromatic offsets for each degree (root = 0).
# When active, each *active* step of a track advances to the next scale degree
# instead of repeating the fixed root.  Resets at each pattern boundary.
SCALE_MODES = {
    "none":      [],
    "major":     [0, 2, 4, 5, 7, 9, 11],
    "minor":     [0, 2, 3, 5, 7, 8, 10],
    "penta_maj": [0, 2, 4, 7, 9],
    "penta_min": [0, 3, 5, 7, 10],
    "blues":     [0, 3, 5, 6, 7, 10],
    "dorian":    [0, 2, 3, 5, 7, 9, 10],
    "mixolyd":   [0, 2, 4, 5, 7, 9, 10],
    "phrygian":  [0, 1, 3, 5, 7, 8, 10],
    "lydian":    [0, 2, 4, 6, 7, 9, 11],
    "chromatic": list(range(12)),
}

SCALE_MODE_KEYS = list(SCALE_MODES.keys())
