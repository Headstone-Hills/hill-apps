"""HillBand — scrollable controls cheat-sheet overlay (Select+B)."""

from __future__ import annotations

import pygame

import config
from input import button_map as bm

_LAV  = (200, 170, 255)
_KEY  = (165, 205, 255)
_DESC = (220, 220, 230)
_HEAD = _LAV

LINES = [
    ("head", "TRANSPORT"),
    ("row", "Start", "Play / pause"),
    ("row", "Select", "Tap tempo"),
    ("row", "Start+Select tap",  "Stop & reset"),
    ("row", "Start+Select hold", "Quit to menu"),
    ("row", "L2 / R2", "BPM down / up (hold = repeat)"),
    ("head", "GRID / TRACKS"),
    ("row", "D-pad", "Move cursor (L/R step, U/D track)"),
    ("row", "A tap / hold+UD",  "Toggle step / step velocity"),
    ("row", "Y tap / hold+UD",  "Mute track / track volume"),
    ("row", "X hold + L/R",     "Transpose note ±1 semitone  (melodic)"),
    ("row", "X hold + U/D",     "Transpose note ±1 octave    (melodic)"),
    ("row", "L1 / R1",          "Prev / next pattern"),
    ("head", "INSTRUMENTS"),
    ("row", "Start+B",   "Sound library  (drum tracks: pre-filtered to DRUM)"),
    ("row", "Start+R2",  "Multi-select track (toggle)"),
    ("row", "  +Start+B","...assigns to ALL selected tracks"),
    ("row", "Start+L2",  "Chord / scale mode (melodic tracks only)"),
    ("head", "CHORD / SCALE  (melodic tracks)"),
    ("row", "In track-mode overlay:", ""),
    ("row", "  L / R", "Cycle chord type (unison, major, minor, dom7…)"),
    ("row", "  U / D", "Cycle scale mode (none, major, minor, penta…)"),
    ("row", "  A",     "Apply + close"),
    ("row", "  B",     "Cancel"),
    ("head", "SONG / GROOVE"),
    ("row", "Start+X",       "Chain editor"),
    ("row", "Start+A",       "Loop player on / off"),
    ("row", "Start+Y",       "Save state"),
    ("row", "Start+L1 / R1", "Copy / paste pattern"),
    ("row", "Select+Y",      "Sequence manager"),
    ("row", "Select+X",      "Swing on / off"),
    ("row", "Select+L2 / R2","Swing down / up"),
    ("row", "Select+A",      "Metronome toggle"),
    ("row", "Select+L1",     "Clear pattern"),
    ("row", "Select+R1",     "Clear all instruments"),
    ("row", "Fn+Select",     "This help (either order)"),
    ("head", "EFFECTS  (melodic tracks, Fn modifier)"),
    ("row", "Fn+A / B / X", "Reverb / Delay / Chorus"),
    ("row", "Fn+Y",         "Wet/dry step"),
    ("row", "Fn+R1 / L1",   "Bitcrush bits / downsample"),
]

_TITLE_SZ = 28
_HEAD_SZ  = 21
_ROW_SZ   = 20
_ROW_H    = 26
_VISIBLE  = 14
_DESC_X   = 260


class HelpOverlay:
    def __init__(self, font_normal=None, font_small=None):
        pygame.font.init()
        self._f_title = pygame.font.SysFont("monospace", _TITLE_SZ, bold=True)
        self._f_head  = pygame.font.SysFont("monospace", _HEAD_SZ,  bold=True)
        self._f_row   = pygame.font.SysFont("monospace", _ROW_SZ)
        self.scroll = 0
        self.closed = False

    def open(self):
        self.closed = False
        self.scroll = 0

    def _max_scroll(self):
        return max(0, len(LINES) - _VISIBLE)

    def handle_hat(self, hat):
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        if bm.UP in dirs:
            self.scroll = max(0, self.scroll - 1)
        elif bm.DOWN in dirs:
            self.scroll = min(self._max_scroll(), self.scroll + 1)

    def handle_button(self, btn):
        if btn in (bm.B, bm.A):
            self.closed = True

    def handle_key(self, key):
        if key == pygame.K_UP:
            self.scroll = max(0, self.scroll - 1)
        elif key == pygame.K_DOWN:
            self.scroll = min(self._max_scroll(), self.scroll + 1)
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    def draw(self, surface):
        w, h = surface.get_size()
        panel = pygame.Surface((w, h), pygame.SRCALPHA)
        panel.fill((6, 6, 12, 242))
        surface.blit(panel, (0, 0))

        surface.blit(self._f_title.render("Controls", True, (245, 245, 255)), (16, 6))
        if self.scroll > 0:
            surface.blit(self._f_row.render("▲ more", True, _LAV), (w - 86, 14))
        if self.scroll < self._max_scroll():
            surface.blit(self._f_row.render("▼ more", True, _LAV), (w - 86, h - 24))

        y = 40
        for item in LINES[self.scroll:self.scroll + _VISIBLE]:
            if item[0] == "head":
                surface.blit(self._f_head.render(item[1], True, _HEAD), (16, y))
            else:
                _, key, desc = item
                surface.blit(self._f_row.render(key,  True, _KEY),  (24, y))
                surface.blit(self._f_row.render(desc, True, _DESC), (_DESC_X, y))
            y += _ROW_H

        surface.blit(self._f_row.render("A / B: close", True, (110, 110, 140)),
                     (16, h - 24))
