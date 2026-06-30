#!/usr/bin/env python3
"""Prep an ambient instrument set from VCSL (CC0) into HillChord's format.

Selects sustained / pad / mallet instruments well-suited to ambient music,
picks ONE (softest) sample per pitch (VCSL ships multiple dynamics +
round-robins), and copies them into samples/<Instrument>/<Note>.wav.

VCSL is already 44.1kHz/16-bit stereo, so files are copied as-is, just renamed.

Usage:  python tools/prep_vcsl.py /path/to/VCSL
"""

import glob
import os
import re
import shutil
import sys

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, HERE)
from audio.mixer import _parse_midi  # noqa: E402

OUT = os.path.join(HERE, "samples")
NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]

# (output folder name, path within VCSL). Curated for ambient music:
# pads/organs, harps & zither (shimmer), soft flute/reed, mallets & bells.
INSTRUMENTS = [
    ("Pad Pipe Organ",      "Aerophones/Edge-blown Aerophones/Pipe Organ/Quiet"),
    ("Pad Renaissance Organ", "Aerophones/Edge-blown Aerophones/Renaissance Organ/Full"),
    ("Recorder Alto",       "Aerophones/Edge-blown Aerophones/Baroque Alto Recorder/SusVib"),
    ("Tenor Sax",           "Aerophones/Reed Aerophones/Tenor Saxophone/Non-Vibrato"),
    ("Concert Harp",        "Chordophones/Composite Chordophones/Concert Harp"),
    ("Folk Harp",           "Chordophones/Composite Chordophones/Folk Harp"),
    ("Piano Steinway",      "Chordophones/Zithers/Grand Piano, Steinway B/Sus"),
    ("Dan Tranh",           "Chordophones/Zithers/Dan Tranh/Vibrato"),
    ("Glockenspiel",        "Idiophones/Struck Idiophones/Glockenspiel"),
    ("Vibraphone",          "Idiophones/Struck Idiophones/Vibraphone"),
    ("Tubular Bells",       "Idiophones/Struck Idiophones/Tubular Bells 1"),
    ("Marimba",             "Idiophones/Struck Idiophones/Marimba"),
]

# Softest-first dynamic preference (ambient favours gentle layers).
_DYN_RANK = {"mp": 0, "p": 1, "mf": 2, "pp": 3, "f": 4, "ff": 5, "ppp": 6, "fff": 7}


def _score(path):
    """Lower is preferred: gentle dynamic, low round-robin, then name."""
    stem = os.path.splitext(os.path.basename(path))[0]
    toks = re.split(r"[ _\-.]+", stem)
    dyn, rr = 9, 99
    for t in toks:
        m = re.fullmatch(r"(ppp|pp|mp|mf|fff|ff|p|f)\d*", t.lower())
        if m:
            dyn = min(dyn, _DYN_RANK.get(m.group(1), 9))
        m = re.fullmatch(r"(?:rr|v)(\d+)", t.lower())
        if m:
            rr = min(rr, int(m.group(1)))
    return (dyn, rr, stem)


def note_name(midi):
    return f"{NOTES[midi % 12]}{midi // 12 - 1}"


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    vcsl = sys.argv[1]

    for name, rel in INSTRUMENTS:
        src = os.path.join(vcsl, rel)
        wavs = glob.glob(os.path.join(src, "**", "*.wav"), recursive=True)
        if not wavs:
            print(f"SKIP {name}: no wavs at {rel}")
            continue
        # one file per pitch: keep the best-scoring sample
        best = {}
        for w in wavs:
            midi = _parse_midi(w)
            if midi is None:
                continue
            if midi not in best or _score(w) < _score(best[midi]):
                best[midi] = w
        dst = os.path.join(OUT, name)
        os.makedirs(dst, exist_ok=True)
        for midi, w in sorted(best.items()):
            shutil.copyfile(w, os.path.join(dst, note_name(midi) + ".wav"))
        lo, hi = min(best), max(best)
        print(f"{name:22} {len(best):3} notes  {note_name(lo)}..{note_name(hi)}")


if __name__ == "__main__":
    main()
