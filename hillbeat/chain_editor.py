"""
HillBeat — chain_editor.py
Pattern chain editor overlay.

Controls:
  ↑↓     navigate entries
  ←→     change repeats count for selected entry (+/- 1)
  A / Z  add entry after cursor (default: next pattern, 1 repeat)
  X      delete selected entry
  B/Esc  close
"""

import pygame

from constants import (
    SCREEN_W, SCREEN_H,
    NUM_PATTERNS,
    CLR_OVERLAY_BG, CLR_OVERLAY_ITEM, CLR_OVERLAY_SEL,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE, CLR_WARNING,
    CLR_BADGE_CHAIN,
    FONT_SIZE_NORMAL, FONT_SIZE_SMALL,
)


class ChainEditor:

    def __init__(self, state, font_normal, font_small, reset_chain_cb=None):
        self.state   = state
        self._font_n = font_normal
        self._font_s = font_small
        self._reset_chain_cb = reset_chain_cb or (lambda: None)

        self._cursor  = 0
        self._scroll  = 0
        self._visible = 14

        self.closed   = False

    def open(self):
        self._cursor = max(0, min(self._cursor, len(self.state.chain) - 1))
        self._scroll = 0
        self.closed  = False

    # ── Input ─────────────────────────────────────────────────────────────────

    def handle_hat(self, hat):
        from constants import HAT_UP, HAT_DOWN, HAT_LEFT, HAT_RIGHT
        chain = self.state.chain
        if hat == HAT_UP:
            self._cursor = max(0, self._cursor - 1)
            self._clamp_scroll()
        elif hat == HAT_DOWN:
            self._cursor = min(max(0, len(chain) - 1), self._cursor + 1)
            self._clamp_scroll()
        elif hat == HAT_LEFT and chain and self._cursor < len(chain):
            chain[self._cursor]["repeats"] = max(1, chain[self._cursor]["repeats"] - 1)
        elif hat == HAT_RIGHT and chain and self._cursor < len(chain):
            chain[self._cursor]["repeats"] = min(16, chain[self._cursor]["repeats"] + 1)

    def handle_button(self, btn):
        from constants import BTN_A, BTN_B, BTN_X, BTN_Y, BTN_L1, BTN_R1
        if btn == BTN_A:
            self._add_entry()
        elif btn == BTN_X:
            self._delete_entry()
        elif btn == BTN_Y:
            self.state.chain_enabled = not self.state.chain_enabled
            if self.state.chain_enabled:
                self._reset_chain_cb()
        elif btn == BTN_R1:
            self._cycle_pattern(+1)
        elif btn == BTN_L1:
            self._cycle_pattern(-1)
        elif btn == BTN_B:
            self.closed = True

    def handle_key(self, key):
        if key == pygame.K_UP:
            self.handle_hat((0, 1))     # HAT_UP
        elif key == pygame.K_DOWN:
            self.handle_hat((0, -1))    # HAT_DOWN
        elif key == pygame.K_LEFT:
            self.handle_hat((-1, 0))
        elif key == pygame.K_RIGHT:
            self.handle_hat((1, 0))
        elif key == pygame.K_z:
            self._add_entry()
        elif key == pygame.K_x:
            self._delete_entry()
        elif key == pygame.K_e:
            self._cycle_pattern(+1)
        elif key == pygame.K_q:
            self._cycle_pattern(-1)
        elif key == pygame.K_RETURN:
            self.state.chain_enabled = not self.state.chain_enabled
            if self.state.chain_enabled:
                self._reset_chain_cb()
        elif key in (pygame.K_ESCAPE, pygame.K_BACKSPACE):
            self.closed = True

    def _add_entry(self):
        chain = self.state.chain
        # Guess a sensible next pattern
        if chain:
            last_pat = chain[-1]["pattern"] if self._cursor >= len(chain) - 1 else chain[self._cursor]["pattern"]
            new_pat  = (last_pat + 1) % NUM_PATTERNS
        else:
            new_pat = 0
        insert_pos = self._cursor + 1 if chain else 0
        chain.insert(insert_pos, {"pattern": new_pat, "repeats": 1})
        self._cursor = insert_pos
        self._clamp_scroll()

    def _cycle_pattern(self, direction: int):
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

    def draw(self, surface: pygame.Surface):
        overlay = pygame.Surface((SCREEN_W, SCREEN_H), pygame.SRCALPHA)
        overlay.fill(CLR_OVERLAY_BG)
        surface.blit(overlay, (0, 0))

        padding = 30
        w       = SCREEN_W - padding * 2
        row_h   = 28
        header_h = 44

        # Title
        title_surf = self._font_n.render("Pattern Chain Editor", True, CLR_BADGE_CHAIN)
        surface.blit(title_surf, (padding, padding))
        chain = self.state.chain
        enabled_label = "ENABLED" if self.state.chain_enabled else "disabled"
        en_clr = CLR_WARNING if self.state.chain_enabled else CLR_TEXT_DIM
        en_surf = self._font_s.render(f"Chain: {enabled_label}  ({len(chain)} entries)", True, en_clr)
        surface.blit(en_surf, (padding, padding + 22))

        y = padding + header_h

        if not chain:
            empty = self._font_s.render("Chain is empty.  Press A/Z to add an entry.", True, CLR_TEXT_DIM)
            surface.blit(empty, (padding, y))
        else:
            for i in range(self._visible):
                fi = self._scroll + i
                if fi >= len(chain):
                    break
                entry   = chain[fi]
                is_sel  = fi == self._cursor

                rect = pygame.Rect(padding, y, w, row_h - 2)
                bg   = CLR_OVERLAY_SEL if is_sel else CLR_OVERLAY_ITEM
                pygame.draw.rect(surface, bg, rect, border_radius=3)

                label = f"  {fi+1:2d}.  Pattern P{entry['pattern']+1}   ×{entry['repeats']} repeat{'s' if entry['repeats']!=1 else ''}"
                clr   = CLR_WHITE if is_sel else CLR_TEXT
                txt   = self._font_s.render(label, True, clr)
                surface.blit(txt, (padding + 4, y + 6))

                y += row_h

        # Footer hints
        hints = "↑↓ navigate  ←→ repeats  L1/R1 pattern  A add  X delete  Enter toggle  Esc close"
        hint_surf = self._font_s.render(hints, True, CLR_TEXT_DIM)
        surface.blit(hint_surf, (padding, SCREEN_H - 28))
