"""HillSequencer — Key / Scale selector overlay.

Pick a key signature (root + major/minor) and a base octave, then apply it to
spread the scale across all tracks: T1 = high root (one octave up), the last
track = low root, the middle tracks the scale degrees descending in between.

Controls:
  Left/Right  root note (chromatic)
  Up/Down     base octave (of the low root)
  X           major / minor
  A           apply to all tracks (and close)
  B/Esc       close without applying
"""

from __future__ import annotations

import pygame

import config
from config import (
    SCREEN_W, SCREEN_H, NUM_TRACKS,
    CLR_OVERLAY_BG, CLR_OVERLAY_ITEM, CLR_OVERLAY_SEL,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE, CLR_WARNING, CLR_BADGE_MET, CLR_STEP_ON,
)
from input import button_map as bm
from theory.notes import note_name

KEY_OCT_MIN = 1
KEY_OCT_MAX = 5


class KeyOverlay:
    def __init__(self, state, font_normal, font_small):
        self.state = state
        self._font_n = font_normal
        self._font_s = font_small
        self.closed = False
        self.applied = False        # main prewarms tracks when this is set

    def open(self):
        self.closed = False
        self.applied = False

    # ── Input ─────────────────────────────────────────────────────────────────
    def handle_hat(self, hat):
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        if bm.LEFT in dirs:
            self.state.key_root = (self.state.key_root - 1) % 12
        elif bm.RIGHT in dirs:
            self.state.key_root = (self.state.key_root + 1) % 12
        elif bm.UP in dirs:
            self.state.key_octave = min(KEY_OCT_MAX, self.state.key_octave + 1)
        elif bm.DOWN in dirs:
            self.state.key_octave = max(KEY_OCT_MIN, self.state.key_octave - 1)

    def handle_button(self, btn):
        if btn == bm.X:
            self.state.key_minor = not self.state.key_minor
        elif btn == bm.A:
            self.state.apply_key_spread()
            self.applied = True
            self.closed = True
        elif btn == bm.B:
            self.closed = True

    def handle_key(self, key):
        if key == pygame.K_LEFT:
            self.handle_hat((-1, 0))
        elif key == pygame.K_RIGHT:
            self.handle_hat((1, 0))
        elif key == pygame.K_UP:
            self.handle_hat((0, 1))
        elif key == pygame.K_DOWN:
            self.handle_hat((0, -1))
        elif key == pygame.K_x:
            self.state.key_minor = not self.state.key_minor
        elif key in (pygame.K_RETURN, pygame.K_a):
            self.state.apply_key_spread()
            self.applied = True
            self.closed = True
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    # ── Draw ──────────────────────────────────────────────────────────────────
    def draw(self, surface):
        overlay = pygame.Surface((SCREEN_W, SCREEN_H), pygame.SRCALPHA)
        overlay.fill(CLR_OVERLAY_BG)
        surface.blit(overlay, (0, 0))

        padding = 30
        surface.blit(self._font_n.render("Key / Scale  ->  spread across tracks",
                                         True, CLR_BADGE_MET), (padding, padding))
        sub = f"Key: {self.state.key_label()}      low-root octave: {self.state.key_octave}"
        surface.blit(self._font_s.render(sub, True, CLR_TEXT), (padding, padding + 24))

        # Preview the resulting per-track notes.
        notes = self.state.key_spread_notes()
        y = padding + 52
        row_h = 24
        for i in range(NUM_TRACKS):
            tag = "  (high root)" if i == 0 else ("  (low root)" if i == NUM_TRACKS - 1 else "")
            tname = self.state.tracks[i]["name"]
            label = f"  T{i+1}  {tname:<3}  ->  {note_name(notes[i])}{tag}"
            clr = CLR_STEP_ON if tag else CLR_TEXT
            surface.blit(self._font_s.render(label, True, clr), (padding, y))
            y += row_h

        hints = "L/R root   Up/Dn octave   X major/minor   A apply to all   B cancel"
        surface.blit(self._font_s.render(hints, True, CLR_TEXT_DIM), (padding, SCREEN_H - 26))
