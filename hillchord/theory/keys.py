"""Keys and the circle of fifths (spec: Select cycles through keys)."""

# Chromatic reference, sharps. Index = semitone within octave (C = 0).
CHROMATIC = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]

# Circle of fifths order, starting at C (spec: Select cycles this order).
CIRCLE_OF_FIFTHS = ["C", "G", "D", "A", "E", "B", "F#", "C#", "G#", "D#", "A#", "F"]

# Map note name -> pitch class (semitone 0-11).
_PITCH_CLASS = {name: i for i, name in enumerate(CHROMATIC)}


def pitch_class(key: str) -> int:
    """Semitone (0-11) of a key's root, C=0."""
    return _PITCH_CLASS[key]


def next_key(key: str, step: int = 1) -> str:
    """Advance around the circle of fifths (spec: Select cycles keys)."""
    i = CIRCLE_OF_FIFTHS.index(key)
    return CIRCLE_OF_FIFTHS[(i + step) % len(CIRCLE_OF_FIFTHS)]


def root_midi(key: str, octave: int = 4) -> int:
    """MIDI note number of the key's root at a given octave.

    MIDI: C4 = 60. midi = (octave + 1) * 12 + pitch_class.
    """
    return (octave + 1) * 12 + pitch_class(key)
