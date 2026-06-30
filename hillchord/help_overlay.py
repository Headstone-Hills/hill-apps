"""Cheat-sheet overlay (toggled by Start+Select+L1+R1). Modal + scrollable with
the D-pad. Single readable column that fits 640x480."""

import pygame

_LAV = (200, 170, 255)

# Flat list of lines: ("head", text) or ("row", key, desc).
LINES = [
    ("head", "GLOBAL"),
    ("row", "Select + R1 / L1", "Cycle key up / down (circle of 5ths)"),
    ("row", "Start", "Open / close Library"),
    ("row", "Function (tap)", "Switch Chord / Note mode"),
    ("row", "Function + D-Up/Dn", "Octave up / down"),
    ("row", "R1", "Reverb"),
    ("row", "L1", "Delay"),
    ("row", "R1 + L1", "Chorus"),
    ("row", "R1 + R2", "Bitcrush: cycle bit depth (16/12/8/4)"),
    ("row", "L1 + R2", "Bitcrush: cycle downsample (1/2/4/6/8x)"),
    ("row", "R2", "Wet/Dry step"),
    ("row", "L2 + R2", "Arp: off/up/down/bounce/bounce4/random"),
    ("row", "L1 + L2", "Panic (silence stuck)"),
    ("row", "Select + Start (hold)", "Quit"),
    ("head", "LOOPER (L2)"),
    ("row", "L2 (in idle)", "Record (locks to beat)"),
    ("row", "L2 (recording)", "Stop & loop"),
    ("row", "L2 (looping)", "Overdub a layer"),
    ("row", "L2 (overdubbing)", "Merge layer"),
    ("row", "L2 (hold 2s)", "Clear loop"),
    ("head", "CHORD MODE"),
    ("row", "A / B / X / Y", "I / IV / V / vi"),
    ("row", "A+B / A+X / B+Y", "ii / iii / vii dim"),
    ("row", "D-Up", "Major / minor flip"),
    ("row", "D-Right", "Major 7th"),
    ("row", "D-Down", "Sus4"),
    ("row", "D-Left", "Diminished"),
    ("row", "D-Up + Right", "Dominant 7th"),
    ("row", "D-Right + Down", "Add9"),
    ("row", "D-Down + Left", "6th"),
    ("row", "D-Up + Left", "Augmented"),
    ("row", "D-pad double-tap", "Latch modifier on/off"),
    ("head", "NOTE MODE"),
    ("row", "D-Up / Right", "Tonic / Supertonic"),
    ("row", "D-Down / Left", "Mediant / Subdominant"),
    ("row", "A / B / X", "Dominant / Submed / Leading"),
    ("row", "Y", "Tonic (octave up)"),
    ("row", "A+B / A+X", "Octave down / up"),
    ("head", "LIBRARY"),
    ("row", "A", "Load sound"),
    ("row", "Y", "Favorite / unfavorite"),
    ("row", "B", "Favorites filter on/off"),
    ("row", "Up / Down", "Navigate (hold to scroll)"),
    ("row", "L1 / R1", "Page up / down"),
    ("row", "L2 / R2", "Prev / next folder"),
    ("row", "Left / Right", "BPM down / up"),
    ("row", "Select", "Metronome toggle"),
]

_VISIBLE = 15          # rows shown at once


def max_scroll():
    return max(0, len(LINES) - _VISIBLE)


def draw(surf, fonts, scroll=0):
    w, h = surf.get_size()
    panel = pygame.Surface((w, h), pygame.SRCALPHA)
    panel.fill((6, 6, 12, 238))
    surf.blit(panel, (0, 0))

    surf.blit(fonts["mid"].render("Controls", True, (245, 245, 255)), (16, 8))
    more_up = scroll > 0
    more_dn = scroll < max_scroll()
    if more_up:
        surf.blit(fonts["small"].render("▲ more", True, _LAV), (w - 90, 10))
    if more_dn:
        surf.blit(fonts["small"].render("▼ more", True, _LAV), (w - 90, h - 24))

    scroll = max(0, min(scroll, max_scroll()))
    y = 40
    for item in LINES[scroll:scroll + _VISIBLE]:
        if item[0] == "head":
            surf.blit(fonts["small"].render(item[1], True, _LAV), (16, y))
        else:
            _, key, desc = item
            surf.blit(fonts["small"].render(key, True, (160, 200, 255)), (28, y))
            surf.blit(fonts["small"].render(desc, True, (215, 215, 225)), (250, y))
        y += 27

    surf.blit(fonts["small"].render(
        "D-pad: scroll    Start+Select+L1+R1: close", True, (130, 130, 150)),
        (16, h - 24))
