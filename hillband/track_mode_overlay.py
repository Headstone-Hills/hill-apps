"""HillBand — per-track chord/scale selector overlay (Start+L2 on melodic tracks).

Controls:
  L / R       cycle chord type
  U / D       cycle scale mode
  A           apply to current track and close
  B / Esc     close without applying
"""

from __future__ import annotations

import pygame

import config
from config import (
    SCREEN_W, SCREEN_H,
    CLR_OVERLAY_BG, CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE,
    CLR_STEP_ON, CLR_BADGE_MET, CLR_MELODIC_ACCENT,
)
from input import button_map as bm
from theory.chords import CHORD_TYPE_KEYS, SCALE_MODE_KEYS, CHORD_TYPES, SCALE_MODES
from theory.notes import note_name


class TrackModeOverlay:
    def __init__(self, state, font_normal, font_small):
        self.state   = state
        self._font_n = font_normal
        self._font_s = font_small
        self.closed  = False
        self.applied = False
        self._track  = 0

        # Pending edit state (separate from state until A is pressed).
        self._chord_i = 0
        self._scale_i = 0

    def open(self, track_i):
        self._track  = track_i
        t = self.state.tracks[track_i]
        ct = t.get("chord_type", "unison")
        sm = t.get("scale_mode", "none")
        self._chord_i = CHORD_TYPE_KEYS.index(ct) if ct in CHORD_TYPE_KEYS else 0
        self._scale_i = SCALE_MODE_KEYS.index(sm) if sm in SCALE_MODE_KEYS else 0
        self.closed  = False
        self.applied = False

    # ── Input ─────────────────────────────────────────────────────────────────
    def handle_hat(self, hat):
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        if bm.LEFT in dirs:
            self._chord_i = (self._chord_i - 1) % len(CHORD_TYPE_KEYS)
        elif bm.RIGHT in dirs:
            self._chord_i = (self._chord_i + 1) % len(CHORD_TYPE_KEYS)
        elif bm.UP in dirs:
            self._scale_i = (self._scale_i - 1) % len(SCALE_MODE_KEYS)
        elif bm.DOWN in dirs:
            self._scale_i = (self._scale_i + 1) % len(SCALE_MODE_KEYS)

    def handle_button(self, btn):
        if btn == bm.A:
            self._apply()
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
        elif key in (pygame.K_RETURN, pygame.K_a):
            self._apply()
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    def _apply(self):
        t = self.state.tracks[self._track]
        t["chord_type"] = CHORD_TYPE_KEYS[self._chord_i]
        t["scale_mode"] = SCALE_MODE_KEYS[self._scale_i]
        self.applied = True
        self.closed  = True

    # ── Draw ──────────────────────────────────────────────────────────────────
    def draw(self, surface):
        overlay = pygame.Surface((SCREEN_W, SCREEN_H), pygame.SRCALPHA)
        overlay.fill(CLR_OVERLAY_BG)
        surface.blit(overlay, (0, 0))

        pad  = 30
        t    = self.state.tracks[self._track]
        root = t["note"]

        surface.blit(self._font_n.render(
            f"Track Mode  —  T{self._track + 1} ({t['name']})",
            True, CLR_MELODIC_ACCENT), (pad, pad))

        # Chord type row
        chord_key = CHORD_TYPE_KEYS[self._chord_i]
        offsets   = CHORD_TYPES.get(chord_key, [0])
        notes_txt = "  ".join(note_name(root + o) for o in offsets if 0 <= root + o <= 127)
        surface.blit(self._font_n.render(
            f"Chord:  < {chord_key} >", True, CLR_WHITE), (pad, pad + 36))
        surface.blit(self._font_s.render(
            f"  notes: {notes_txt}", True, CLR_STEP_ON), (pad, pad + 58))

        # Scale mode row
        scale_key = SCALE_MODE_KEYS[self._scale_i]
        scale_off = SCALE_MODES.get(scale_key, [])
        if scale_off:
            scale_txt = "  ".join(note_name(root + o) for o in scale_off
                                   if 0 <= root + o <= 127)
            scale_sub = f"  degrees: {scale_txt}"
        else:
            scale_sub = "  (steps repeat the root note)"

        surface.blit(self._font_n.render(
            f"Scale:  < {scale_key} >", True, CLR_WHITE), (pad, pad + 86))
        surface.blit(self._font_s.render(
            scale_sub, True, CLR_TEXT_DIM), (pad, pad + 108))

        # Explanation blurb
        if scale_off:
            blurb = ("Active steps cycle through the scale degrees in order.  "
                     "Resets each pattern loop.")
        else:
            blurb = "Active steps all play the chord built on the track root."
        surface.blit(self._font_s.render(blurb, True, CLR_TEXT_DIM), (pad, pad + 136))

        hints = "L/R chord   U/D scale   A apply   B cancel"
        surface.blit(self._font_s.render(hints, True, CLR_TEXT_DIM), (pad, SCREEN_H - 26))
