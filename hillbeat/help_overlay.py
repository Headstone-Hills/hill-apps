"""HillBeat cheat-sheet overlay.  Open with FUNCTION+SELECT, close with A or B."""

import pygame

_LAV = (200, 170, 255)
_KEY = (160, 200, 255)
_DSC = (215, 215, 225)

LINES = [
    ("head", "SEQUENCER"),
    ("row",  "START",               "Play / Pause"),
    ("row",  "START+SELECT tap",    "Stop & reset to bar 1"),
    ("row",  "SELECT",              "Tap tempo"),
    ("row",  "A  (tap)",            "Toggle step on/off"),
    ("row",  "A  (hold) + ↑↓",      "Step velocity up / down"),
    ("row",  "Y  (tap)",            "Mute / unmute voice"),
    ("row",  "Y  (hold) + ↑↓",      "Voice volume up / down"),
    ("row",  "↑↓",                  "Move cursor voice"),
    ("row",  "◀▶",                  "Move cursor step"),
    ("head", "PATTERNS"),
    ("row",  "R1 / L1",             "Next / prev pattern"),
    ("row",  "START+R1",            "Copy current pattern"),
    ("row",  "START+L1",            "Paste to current pattern"),
    ("row",  "START+X",             "Open chain editor"),
    ("head", "BPM & SWING"),
    ("row",  "L2 / R2",             "BPM -1 / +1  (hold to repeat)"),
    ("row",  "SELECT+R2",           "Swing +5%"),
    ("row",  "SELECT+L2",           "Swing -5%"),
    ("head", "LIBRARY"),
    ("row",  "START+B",             "Open library (assign voice)"),
    ("row",  "FUNCTION+SELECT",     "Open this cheat sheet (either order)"),
    ("row",  "START+A",             "Toggle loop player"),
    ("head", "INSIDE LIBRARY"),
    ("row",  "↑↓",                  "Navigate items"),
    ("row",  "L1 / R1",             "Jump ±10 items"),
    ("row",  "◀ / ▶",               "Jump to prev / next section"),
    ("row",  "A",                   "Load selected sample / kit"),
    ("row",  "Y",                   "Star / unstar (favorite)"),
    ("row",  "X",                   "Toggle favorites filter"),
    ("row",  "B",                   "Close library"),
    ("head", "OTHER"),
    ("row",  "START+Y",             "Save state"),
    ("row",  "A / B",               "Close this cheat sheet"),
    ("row",  "START+SELECT tap",   "Stop & reset to bar 1"),
    ("row",  "START+SELECT hold",  "Quit HillBeat (1.2 sec)"),
]

_VISIBLE = 16


def max_scroll():
    return max(0, len(LINES) - _VISIBLE)


def draw(surf, fonts, scroll=0):
    w, h = surf.get_size()
    panel = pygame.Surface((w, h), pygame.SRCALPHA)
    panel.fill((6, 6, 14, 245))
    surf.blit(panel, (0, 0))

    surf.blit(fonts["mid"].render("Controls", True, (245, 245, 255)), (16, 10))

    if scroll > 0:
        surf.blit(fonts["small"].render("▲ more", True, _LAV), (w - 90, 12))
    if scroll < max_scroll():
        surf.blit(fonts["small"].render("▼ more", True, _LAV), (w - 90, h - 24))

    scroll = max(0, min(scroll, max_scroll()))
    y = 44
    row_h = fonts["small"].get_height() + 8
    for item in LINES[scroll: scroll + _VISIBLE]:
        if item[0] == "head":
            surf.blit(fonts["small"].render(item[1], True, _LAV), (16, y))
        else:
            _, key, desc = item
            surf.blit(fonts["small"].render(key,  True, _KEY), (28,  y))
            surf.blit(fonts["small"].render(desc, True, _DSC), (260, y))
        y += row_h

    surf.blit(
        fonts["small"].render("↑↓ scroll    A / B: close", True, (110, 110, 140)),
        (16, h - 24),
    )
