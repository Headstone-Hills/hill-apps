"""
HillBeat — ui.py
Main rendering pass at 60 fps.  No game logic here — read-only access to
state/transport, draws to the surface passed in.
"""

import time
import pygame

from constants import (
    SCREEN_W, SCREEN_H, FPS,
    STATUS_BAR_H, LOOP_BAR_H, VOICE_LABEL_W,
    NUM_VOICES, NUM_STEPS,
    CLR_BG, CLR_STATUS_BG, CLR_VOICE_BG, CLR_VOICE_ACTIVE,
    CLR_STEP_OFF, CLR_STEP_ON, CLR_STEP_ON_LOW, CLR_STEP_MUTED, CLR_STEP_MUTED_ON,
    CLR_CURSOR, CLR_PLAYHEAD, CLR_LOOP_BG, CLR_LOOP_BAR_FG,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE, CLR_BLACK,
    CLR_BADGE_PLAY, CLR_BADGE_PAUSE, CLR_BADGE_STOP,
    CLR_BADGE_CHAIN, CLR_BADGE_LOOP, CLR_WARNING,
    FONT_SIZE_SMALL, FONT_SIZE_NORMAL, FONT_SIZE_LARGE,
)

import os


class UI:
    """Stateless renderer; call draw() each frame."""

    def __init__(self, state, transport, loop_player):
        self.state       = state
        self.transport   = transport
        self.loop_player = loop_player

        pygame.font.init()
        self.font_s = pygame.font.SysFont("monospace", FONT_SIZE_SMALL)
        self.font_n = pygame.font.SysFont("monospace", FONT_SIZE_NORMAL)
        self.font_l = pygame.font.SysFont("monospace", FONT_SIZE_LARGE, bold=True)

        # Tap feedback overlay
        self._tap_bpm:   int | None = None
        self._tap_until: float = 0.0

        # Cursor position (voice, step) — managed by input handler
        self.cursor_voice = 0
        self.cursor_step  = 0

    def notify_tap(self, bpm: int | None):
        """Call when a tap fires; shows BPM overlay for 1 second."""
        from constants import TAP_FEEDBACK_DURATION
        self._tap_bpm   = bpm
        self._tap_until = time.monotonic() + TAP_FEEDBACK_DURATION

    # ── Main draw ─────────────────────────────────────────────────────────────

    def draw(self, surface: pygame.Surface,
             show_library=False, show_chain=False,
             library=None, chain_editor=None):

        surface.fill(CLR_BG)

        has_loop_bar = self.state.loop_enabled and self.state.loop_file is not None

        # Compute voice area height
        voice_area_top = STATUS_BAR_H
        voice_area_bot = SCREEN_H - (LOOP_BAR_H if has_loop_bar else 0)
        voice_area_h   = voice_area_bot - voice_area_top
        row_h          = voice_area_h // NUM_VOICES

        self._draw_status_bar(surface)
        self._draw_voice_rows(surface, voice_area_top, row_h)
        if has_loop_bar:
            self._draw_loop_bar(surface, voice_area_bot)

        # Overlays
        if show_library and library:
            library.draw(surface)
        elif show_chain and chain_editor:
            chain_editor.draw(surface)

        # Tap feedback
        if self._tap_bpm is not None and time.monotonic() < self._tap_until:
            self._draw_tap_feedback(surface)

    # ── Status bar ────────────────────────────────────────────────────────────

    def _draw_status_bar(self, surface: pygame.Surface):
        rect = pygame.Rect(0, 0, SCREEN_W, STATUS_BAR_H)
        pygame.draw.rect(surface, CLR_STATUS_BG, rect)

        x = 8
        y = (STATUS_BAR_H - self.font_n.get_height()) // 2

        # Pattern
        pat_txt = f"P{self.transport.current_pattern + 1}"
        if self.transport.pending_pattern is not None:
            pat_txt += f"→P{self.transport.pending_pattern + 1}"
        s = self.font_n.render(pat_txt, True, CLR_TEXT)
        surface.blit(s, (x, y)); x += s.get_width() + 14

        # BPM
        bpm_txt = f"♩{self.state.bpm}"
        s = self.font_n.render(bpm_txt, True, CLR_TEXT)
        surface.blit(s, (x, y)); x += s.get_width() + 14

        # Swing (hide if 0)
        if self.state.swing:
            sw_txt = f"sw:{self.state.swing}%"
            s = self.font_n.render(sw_txt, True, CLR_TEXT_DIM)
            surface.blit(s, (x, y)); x += s.get_width() + 14

        # State badge
        if self.transport.playing:
            badge_txt, badge_clr = "▶ PLAYING", CLR_BADGE_PLAY
        else:
            badge_txt, badge_clr = "■ STOPPED", CLR_BADGE_STOP
        s = self.font_n.render(badge_txt, True, badge_clr)
        surface.blit(s, (x, y)); x += s.get_width() + 14

        # CHAIN badge
        if self.state.chain_enabled:
            s = self.font_n.render("[CHAIN]", True, CLR_BADGE_CHAIN)
            surface.blit(s, (x, y)); x += s.get_width() + 8

        # LOOP badge
        if self.state.loop_enabled:
            s = self.font_n.render("[LOOP]", True, CLR_BADGE_LOOP)
            surface.blit(s, (x, y))

    # ── Voice rows ────────────────────────────────────────────────────────────

    def _draw_voice_rows(self, surface, top, row_h):
        step_area_w = SCREEN_W - VOICE_LABEL_W
        cell_w      = step_area_w // NUM_STEPS
        cell_h      = row_h - 8

        playhead = self.transport.current_step

        for vi in range(NUM_VOICES):
            y0 = top + vi * row_h

            # Row background
            bg = CLR_VOICE_ACTIVE if vi == self.cursor_voice else CLR_VOICE_BG
            pygame.draw.rect(surface, bg, (0, y0, SCREEN_W, row_h - 2))

            # Voice label
            voice_data = self.state.voices[vi]
            name = voice_data["name"]
            vol  = voice_data.get("volume", 1.0)
            if voice_data["muted"]:
                label_clr = CLR_TEXT_DIM
                name_str  = f"({name[:4]})"
            else:
                label_clr = CLR_TEXT
                name_str  = name[:6]
            lbl = self.font_s.render(name_str, True, label_clr)
            lbl_y = y0 + (row_h - lbl.get_height()) // 2
            surface.blit(lbl, (4, lbl_y))

            # Volume pip: tiny bar below the name (only when not 100%)
            if abs(vol - 1.0) > 0.01 and not voice_data["muted"]:
                bar_x = 4
                bar_y = lbl_y + lbl.get_height() + 1
                bar_w = int((VOICE_LABEL_W - 8) * vol)
                pygame.draw.rect(surface, CLR_TEXT_DIM,
                                 (bar_x, bar_y, VOICE_LABEL_W - 8, 3))
                pygame.draw.rect(surface, CLR_STEP_ON,
                                 (bar_x, bar_y, max(1, bar_w), 3))

            # Steps
            pat = self.transport.current_pattern
            muted = voice_data["muted"]

            for si in range(NUM_STEPS):
                cx = VOICE_LABEL_W + si * cell_w + 2
                cy = y0 + 4

                cell       = self.state.patterns[pat][vi][si]
                active_bit = cell[0]
                velocity   = cell[1]

                is_cursor   = (vi == self.cursor_voice and si == self.cursor_step)
                is_playhead = (si == playhead and self.transport.playing)

                # Column highlight (cursor or playhead)
                if is_cursor:
                    col_rect = pygame.Rect(VOICE_LABEL_W + si * cell_w,
                                          top, cell_w, row_h * NUM_VOICES)
                    # Draw very faint cursor column overlay once across all rows
                    # (do it only on row 0 to avoid overdraw)
                    if vi == 0:
                        col_surf = pygame.Surface((cell_w - 2, row_h * NUM_VOICES - 4), pygame.SRCALPHA)
                        col_surf.fill((*CLR_CURSOR, 25))
                        surface.blit(col_surf, (VOICE_LABEL_W + si * cell_w, top + 2))

                # Cell colour
                if muted:
                    clr = CLR_STEP_MUTED_ON if active_bit else CLR_STEP_MUTED
                elif active_bit:
                    clr = CLR_STEP_ON_LOW if velocity < 80 else CLR_STEP_ON
                else:
                    clr = CLR_STEP_OFF

                rect = pygame.Rect(cx, cy, cell_w - 4, cell_h)
                pygame.draw.rect(surface, clr, rect, border_radius=3)

                # Cursor border
                if is_cursor:
                    pygame.draw.rect(surface, CLR_CURSOR, rect, width=2, border_radius=3)

                # Playhead border
                if is_playhead:
                    pygame.draw.rect(surface, CLR_PLAYHEAD, rect, width=2, border_radius=3)

    # ── Loop bar ──────────────────────────────────────────────────────────────

    def _draw_loop_bar(self, surface, y0):
        pygame.draw.rect(surface, CLR_LOOP_BG, (0, y0, SCREEN_W, LOOP_BAR_H))

        x = 8
        y = y0 + (LOOP_BAR_H - self.font_s.get_height()) // 2

        # Filename
        fname = os.path.basename(self.state.loop_file or "")
        if len(fname) > 28:
            fname = fname[:25] + "..."
        native = self.state.loop_native_bpm
        bpm_label = f"  {native}→{self.state.bpm}bpm" if native else ""
        s = self.font_s.render(f"▶ {fname}{bpm_label}", True, CLR_TEXT)
        surface.blit(s, (x, y)); x += s.get_width() + 12

        # Progress bar
        bar_w = 120
        bar_h = 10
        by    = y0 + (LOOP_BAR_H - bar_h) // 2
        pygame.draw.rect(surface, CLR_STEP_OFF, (x, by, bar_w, bar_h), border_radius=2)
        frac = self.loop_player.position_fraction()
        fill = int(bar_w * frac)
        if fill > 0:
            pygame.draw.rect(surface, CLR_LOOP_BAR_FG, (x, by, fill, bar_h), border_radius=2)
        x += bar_w + 10

        # Stretch warning
        if self.loop_player.stretch_warning:
            s = self.font_s.render("STRETCH N/A", True, CLR_WARNING)
            surface.blit(s, (x, y))

    # ── Tap feedback overlay ──────────────────────────────────────────────────

    def _draw_tap_feedback(self, surface):
        if self._tap_bpm is None:
            label = "Tap…"
        else:
            label = f"BPM: {self._tap_bpm}"
        text = self.font_l.render(label, True, CLR_WHITE)
        tw, th = text.get_size()
        bx = (SCREEN_W - tw) // 2 - 20
        by = (SCREEN_H - th) // 2 - 10
        bg = pygame.Surface((tw + 40, th + 20), pygame.SRCALPHA)
        bg.fill((0, 0, 0, 180))
        surface.blit(bg, (bx, by))
        surface.blit(text, (bx + 20, by + 10))
