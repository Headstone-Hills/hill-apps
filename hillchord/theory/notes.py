"""Note Mode: scale degree -> single MIDI note (spec Note Mode mapping).

Button -> scale degree:
    D-Up=Tonic(1)  D-Right=Supertonic(2)  D-Down=Mediant(3)  D-Left=Subdominant(4)
    A=Dominant(5)  B=Submediant(6)  X=Leading Tone(7)  Y=Tonic +1 octave
    A+B=Octave Down   A+X=Octave Up   (handled as octave shift in actions)
"""

from theory import keys

# Major / natural-minor scale degree -> semitone offset from tonic.
_MAJOR_SCALE = {1: 0, 2: 2, 3: 4, 4: 5, 5: 7, 6: 9, 7: 11}
_MINOR_SCALE = {1: 0, 2: 2, 3: 3, 4: 5, 5: 7, 6: 8, 7: 10}


def note_midi(key: str, minor: bool, degree: int,
              octave: int = 4, octave_shift: int = 0,
              extra_octave: bool = False) -> int:
    """MIDI note for a scale degree.

    `octave_shift` is the persistent A+B / A+X shift (in octaves).
    `extra_octave` is the per-button +1 octave (e.g. Y = Tonic octave up).
    """
    scale = _MINOR_SCALE if minor else _MAJOR_SCALE
    off = scale[degree]
    base = (octave + 1) * 12 + keys.pitch_class(key)
    return base + off + 12 * octave_shift + (12 if extra_octave else 0)


def note_name(midi: int) -> str:
    """MIDI number -> note name with octave, e.g. 60 -> 'C4'."""
    return f"{keys.CHROMATIC[midi % 12]}{midi // 12 - 1}"
