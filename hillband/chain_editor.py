"""HillSequencer — chain editor overlay (from HillBeat).

Controls:
  Up/Down   navigate entries
  Left/Right change repeats for selected entry
  A         add entry after cursor
  X         delete selected entry
  Y         toggle chain on/off
  L1/R1     cycle the selected entry's pattern number
  B/Esc     close
"""

from __future__ import annotations

import pygame

import config
from config import (
    SCREEN_W, SCREEN_H, NUM_PATTERNS,
    CLR_OVERLAY_BG, CLR_OVERLAY_ITEM, CLR_OVERLAY_SEL,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE, CLR_WARNING, CLR_BADGE_CHAIN,
)
from input import button_map as bm


class ChainEditor:
    def __init__(self, state, font_normal, font_small, reset_chain_cb=None):
        self.state = state
        self._font_n = font_normal
        self._font_s = font_small
        self._reset_chain_cb = reset_chain_cb or (lambda: None)
        self._cursor = 0
        self._scroll = 0
        self._visible = 12
        self.closed = False

    def open(self):
        self._cursor = max(0, min(self._cursor, len(self.state.chain) - 1))
        self._scroll = 0
        self.closed = False

    # ── Input ─────────────────────────────────────────────────────────────────
    def handle_hat(self, hat):
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        chain = self.state.chain
        if bm.UP in dirs:
            self._cursor = max(0, self._cursor - 1); self._clamp_scroll()
        elif bm.DOWN in dirs:
            self._cursor = min(max(0, len(chain) - 1), self._cursor + 1); self._clamp_scroll()
        elif bm.LEFT in dirs and chain and self._cursor < len(chain):
            chain[self._cursor]["repeats"] = max(1, chain[self._cursor]["repeats"] - 1)
        elif bm.RIGHT in dirs and chain and self._cursor < len(chain):
            chain[self._cursor]["repeats"] = min(16, chain[self._cursor]["repeats"] + 1)

    def handle_button(self, btn):
        if btn == bm.A:
            self._add_entry()
        elif btn == bm.X:
            self._delete_entry()
        elif btn == bm.Y:
            self.state.chain_enabled = not self.state.chain_enabled
            if self.state.chain_enabled:
                self._reset_chain_cb()
        elif btn == bm.R1:
            self._cycle_pattern(+1)
        elif btn == bm.L1:
            self._cycle_pattern(-1)
        elif btn == bm.B:
            self.closed = True

    def handle_key(self, key):
        if key == pygame.K_UP:
            self.handle_hat((0, 1))
        elif key == pygame.K_DOWN:
            self.handle_hat((0, -1))
        elif key == pygame.K_LEFT:
            self.handle_hat((-1, 0))
        elif key == pygame.K_RIGHT:
            self.handle_hat((1, 0))
        elif key in (pygame.K_RETURN, pygame.K_a):
            self._add_entry()
        elif key == pygame.K_x:
            self._delete_entry()
        elif key == pygame.K_y:
            self.state.chain_enabled = not self.state.chain_enabled
            if self.state.chain_enabled:
                self._reset_chain_cb()
        elif key == pygame.K_RIGHTBRACKET:
            self._cycle_pattern(+1)
        elif key == pygame.K_LEFTBRACKET:
            self._cycle_pattern(-1)
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    def _add_entry(self):
        chain = self.state.chain
        if chain:
            last_pat = chain[-1]["pattern"] if self._cursor >= len(chain) - 1 else chain[self._cursor]["pattern"]
            new_pat = (last_pat + 1) % NUM_PATTERNS
        else:
            new_pat = 0
        insert_pos = self._cursor + 1 if chain else 0
        chain.insert(insert_pos, {"pattern": new_pat, "repeats": 1})
        self._cursor = insert_pos
        self._clamp_scroll()

    def _cycle_pattern(self, direction):
        chain = self.state.chain
        if chain and self._cursor < len(chain):
            chain[self._cursor]["pattern"] = (chain[self._cursor]["pattern"] + direction) % NUM_PATTERNS

    def _delete_entry(self):
        chain = self.state.chain
        if chain and self._cursor < len(chain):
            chain.pop(self._cursor)
            self._cursor = max(0, min(self._cursor, len(chain) - 1))
            self._clamp_scroll()

    def _clamp_scroll(self):
        if self._cursor < self._scroll:
            self._scroll = self._cursor
        elif self._cursor >= self._scroll + self._visible:
            self._scroll = self._cursor - self._visible + 1

    # ── Draw ──────────────────────────────────────────────────────────────────
    def draw(self, surface):
        overlay = pygame.Surface((SCREEN_W, SCREEN_H), pygame.SRCALPHA)
        overlay.fill(CLR_OVERLAY_BG)
        surface.blit(overlay, (0, 0))

        padding, w, row_h, header_h = 30, SCREEN_W - 60, 26, 44
        surface.blit(self._font_n.render("Pattern Chain Editor", True, CLR_BADGE_CHAIN), (padding, padding))
        chain = self.state.chain
        en_label = "ENABLED" if self.state.chain_enabled else "disabled"
        en_clr = CLR_WARNING if self.state.chain_enabled else CLR_TEXT_DIM
        surface.blit(self._font_s.render(f"Chain: {en_label}  ({len(chain)} entries)", True, en_clr),
                     (padding, padding + 22))

        y = padding + header_h
        if not chain:
            surface.blit(self._font_s.render("Chain is empty.  Press A to add an entry.", True, CLR_TEXT_DIM), (padding, y))
        else:
            for i in range(self._visible):
                fi = self._scroll + i
                if fi >= len(chain):
                    break
                entry = chain[fi]
                is_sel = fi == self._cursor
                rect = pygame.Rect(padding, y, w, row_h - 2)
                pygame.draw.rect(surface, CLR_OVERLAY_SEL if is_sel else CLR_OVERLAY_ITEM, rect, border_radius=3)
                label = f"  {fi+1:2d}.  Pattern P{entry['pattern']+1}   x{entry['repeats']} repeat{'s' if entry['repeats'] != 1 else ''}"
                surface.blit(self._font_s.render(label, True, CLR_WHITE if is_sel else CLR_TEXT), (padding + 4, y + 5))
                y += row_h

        hints = "Up/Dn navigate  L/R repeats  L1/R1 pattern  A add  X delete  Y toggle  B close"
        surface.blit(self._font_s.render(hints, True, CLR_TEXT_DIM), (padding, SCREEN_H - 26))
