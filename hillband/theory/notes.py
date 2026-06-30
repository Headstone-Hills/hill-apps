"""Minimal note-name helper (sharps)."""

CHROMATIC = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def note_name(midi: int) -> str:
    """MIDI number -> note name with octave, e.g. 60 -> 'C4'."""
    return f"{CHROMATIC[midi % 12]}{midi // 12 - 1}"
