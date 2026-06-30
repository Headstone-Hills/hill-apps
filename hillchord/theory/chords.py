"""Chord construction: scale degree + tonality + D-pad modifiers -> MIDI notes.

Button -> scale degree (spec Chord Mode):
    A=I(1)  B=IV(4)  X=V(5)  Y=vi(6)
    A+B=ii(2)  A+X=iii(3)  B+Y=vii(7)

D-pad modifiers (all transient-held, all double-tap latchable):
    UP            -> flip key major/minor (handled in actions, not here)
    UP_RIGHT      -> dominant 7th
    RIGHT         -> major 7th
    RIGHT_DOWN    -> add9
    DOWN          -> sus4
    DOWN_LEFT     -> 6th
    LEFT          -> diminished
    UP_LEFT       -> augmented
"""

from theory import keys

# Button name -> scale degree (1-based).
BUTTON_DEGREE = {
    "I": 1, "IV": 4, "V": 5, "vi": 6, "ii": 2, "iii": 3, "vii": 7,
}

# Triad interval sets (semitones from chord root).
TRIAD = {
    "maj": [0, 4, 7],
    "min": [0, 3, 7],
    "dim": [0, 3, 6],
    "aug": [0, 4, 8],
}

# Diatonic (root_offset_from_tonic, quality) per scale degree, per tonality.
_MAJOR = {
    1: (0, "maj"), 2: (2, "min"), 3: (4, "min"), 4: (5, "maj"),
    5: (7, "maj"), 6: (9, "min"), 7: (11, "dim"),
}
_MINOR = {  # natural minor
    1: (0, "min"), 2: (2, "dim"), 3: (3, "maj"), 4: (5, "min"),
    5: (7, "min"), 6: (8, "maj"), 7: (10, "maj"),
}

# Modifier names (the diagonal/cardinal D-pad chord modifiers).
MOD_DOM7 = "dom7"
MOD_MAJ7 = "maj7"
MOD_ADD9 = "add9"
MOD_SUS4 = "sus4"
MOD_SIXTH = "sixth"
MOD_DIM = "dim"
MOD_AUG = "aug"


def chord_name(key: str, minor: bool, degree: int, modifiers) -> str:
    """Human-readable name of the chord, e.g. 'C', 'Am', 'Gsus4', 'Dmaj7'."""
    table = _MINOR if minor else _MAJOR
    root_off, quality = table[degree]
    root_pc = (keys.pitch_class(key) + root_off) % 12
    root = keys.CHROMATIC[root_pc]
    mods = set(modifiers)

    if MOD_DIM in mods:
        quality = "dim"
    elif MOD_AUG in mods:
        quality = "aug"

    suffix = {"maj": "", "min": "m", "dim": "dim", "aug": "aug"}[quality]
    if MOD_SUS4 in mods:
        suffix = "sus4"
    if MOD_DOM7 in mods:
        suffix += "7"
    if MOD_MAJ7 in mods:
        suffix += "maj7"
    if MOD_SIXTH in mods:
        suffix += "6"
    if MOD_ADD9 in mods:
        suffix += "add9"
    return root + suffix


def build_chord(key: str, minor: bool, degree: int, modifiers,
                octave: int = 4) -> list:
    """Return a list of MIDI note numbers for the chord.

    `modifiers` is an iterable of MOD_* constants (order-independent).
    """
    table = _MINOR if minor else _MAJOR
    root_off, quality = table[degree]
    root_pc = keys.pitch_class(key)
    root = (octave + 1) * 12 + root_pc + root_off

    mods = set(modifiers)

    # Quality overrides (diminished / augmented force the triad shape).
    if MOD_DIM in mods:
        quality = "dim"
    elif MOD_AUG in mods:
        quality = "aug"

    intervals = list(TRIAD[quality])

    # Sus4: replace the third (3 or 4) with a perfect fourth (5).
    if MOD_SUS4 in mods:
        intervals = [i for i in intervals if i not in (3, 4)]
        if 5 not in intervals:
            intervals.append(5)

    # Sevenths.
    if MOD_DOM7 in mods and 10 not in intervals:
        intervals.append(10)   # minor 7th
    if MOD_MAJ7 in mods and 11 not in intervals:
        intervals.append(11)   # major 7th

    # 6th.
    if MOD_SIXTH in mods and 9 not in intervals:
        intervals.append(9)

    # Add9 (a 9th = octave + major second).
    if MOD_ADD9 in mods and 14 not in intervals:
        intervals.append(14)

    intervals.sort()
    return [root + i for i in intervals]
