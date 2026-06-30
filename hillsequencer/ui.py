"""HillSequencer — main renderer (adapted from HillBeat for 8 tracks).

Read-only view over state/transport. Each of the 8 rows shows the track's name,
tuned note, assigned instrument, mute/volume, per-track effect badges, and the
16-step grid with velocity colour, cursor column, and playhead.
"""

from __future__ import annotations

import os
import time

import pygame

import config
from config import (
    SCREEN_W, SCREEN_H, STATUS_BAR_H, LOOP_BAR_H, TRACK_LABEL_W,
    NUM_TRACKS, NUM_STEPS,
    CLR_BG, CLR_STATUS_BG, CLR_VOICE_BG, CLR_VOICE_ACTIVE,
    CLR_STEP_OFF, CLR_STEP_ON, CLR_STEP_ON_LOW, CLR_STEP_MUTED, CLR_STEP_MUTED_ON,
    CLR_CURSOR, CLR_PLAYHEAD, CLR_LOOP_BG, CLR_LOOP_BAR_FG,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE,
    CLR_BADGE_PLAY, CLR_BADGE_STOP, CLR_BADGE_CHAIN, CLR_BADGE_LOOP, CLR_BADGE_MET,
    CLR_WARNING, CLR_FX_REVERB, CLR_FX_DELAY, CLR_FX_CHORUS, CLR_FX_CRUSH,
    FONT_SIZE_SMALL, FONT_SIZE_NORMAL, FONT_SIZE_LARGE,
)
from theory.notes import note_name


def _short_instrument(sid):
    """Compact display of a track's instrument sid for the label column."""
    if not sid:
        return "(none)"
    base = sid.split("::")[-1] if "::" in sid else os.path.basename(sid)
    base = os.path.splitext(base)[0]
    return base.replace("_", " ")


class UI:
    def __init__(self, state, transport, loop_player):
        self.state = state
        self.transport = transport
        self.loop_player = loop_player

        pygame.font.init()
        self.font_s = pygame.font.SysFont("monospace", FONT_SIZE_SMALL)
        self.font_n = pygame.font.SysFont("monospace", FONT_SIZE_NORMAL)
        self.font_l = pygame.font.SysFont("monospace", FONT_SIZE_LARGE, bold=True)

        self._tap_bpm = None
        self._tap_until = 0.0

        self.cursor_track = 0
        self.cursor_step = 0
        self.selected_tracks = set()   # multi-select for batch instrument assign

    def notify_tap(self, bpm):
        self._tap_bpm = bpm
        self._tap_until = time.monotonic() + config.TAP_FEEDBACK_DURATION

    # ── Main draw ─────────────────────────────────────────────────────────────
    def draw(self, surface):
        surface.fill(CLR_BG)
        has_loop_bar = self.state.loop_enabled and self.state.loop_file is not None

        top = STATUS_BAR_H
        bot = SCREEN_H - (LOOP_BAR_H if has_loop_bar else 0)
        row_h = (bot - top) // NUM_TRACKS

        self._draw_status_bar(surface)
        self._draw_track_rows(surface, top, row_h)
        if has_loop_bar:
            self._draw_loop_bar(surface, bot)

        if self._tap_bpm is not None and time.monotonic() < self._tap_until:
            self._draw_tap_feedback(surface)

    # ── Status bar ────────────────────────────────────────────────────────────
    def _draw_status_bar(self, surface):
        pygame.draw.rect(surface, CLR_STATUS_BG, pygame.Rect(0, 0, SCREEN_W, STATUS_BAR_H))
        x = 8
        y = (STATUS_BAR_H - self.font_n.get_height()) // 2

        def blit(txt, clr, gap=12):
            nonlocal x
            s = self.font_n.render(txt, True, clr)
            surface.blit(s, (x, y))
            x += s.get_width() + gap

        pat_txt = f"P{self.transport.current_pattern + 1}"
        if self.transport.pending_pattern is not None:
            pat_txt += f">P{self.transport.pending_pattern + 1}"
        blit(pat_txt, CLR_TEXT)
        blit(f"{self.state.bpm}bpm", CLR_TEXT)
        if self.state.swing:
            blit(f"sw{self.state.swing}", CLR_TEXT_DIM)
        blit(f"key:{self.state.key_label().strip()}", CLR_TEXT_DIM)
        if self.transport.playing:
            blit("> PLAY", CLR_BADGE_PLAY)
        else:
            blit("# STOP", CLR_BADGE_STOP)
        if self.state.chain_enabled:
            blit("[CHAIN]", CLR_BADGE_CHAIN, gap=6)
        if self.state.loop_enabled:
            blit("[LOOP]", CLR_BADGE_LOOP, gap=6)
        if self.state.metronome_on:
            blit("[MET]", CLR_BADGE_MET, gap=6)

    # ── Track rows ────────────────────────────────────────────────────────────
    def _draw_track_rows(self, surface, top, row_h):
        step_area_w = SCREEN_W - TRACK_LABEL_W
        cell_w = step_area_w // NUM_STEPS
        cell_h = row_h - 8
        playhead = self.transport.current_step
        pat = self.transport.current_pattern

        for ti in range(NUM_TRACKS):
            y0 = top + ti * row_h
            track = self.state.tracks[ti]
            muted = track["muted"]
            is_cursor_row = (ti == self.cursor_track)

            bg = CLR_VOICE_ACTIVE if is_cursor_row else CLR_VOICE_BG
            pygame.draw.rect(surface, bg, (0, y0, SCREEN_W, row_h - 2))

            # Multi-select accent bar on the left edge
            if ti in self.selected_tracks:
                pygame.draw.rect(surface, CLR_CURSOR, (0, y0, 3, row_h - 2))

            self._draw_track_label(surface, ti, track, muted, is_cursor_row, y0, row_h)

            # Step cells
            for si in range(NUM_STEPS):
                cx = TRACK_LABEL_W + si * cell_w + 2
                cy = y0 + 4
                cell = self.state.patterns[pat][ti][si]
                active_bit, velocity = cell[0], cell[1]
                is_cursor = (ti == self.cursor_track and si == self.cursor_step)
                is_playhead = (si == playhead and self.transport.playing)

                # Faint cursor column (draw once across all rows, on row 0)
                if is_cursor and ti == 0:
                    col = pygame.Surface((cell_w - 2, row_h * NUM_TRACKS - 4), pygame.SRCALPHA)
                    col.fill((*CLR_CURSOR, 22))
                    surface.blit(col, (TRACK_LABEL_W + si * cell_w, top + 2))

                if muted:
                    clr = CLR_STEP_MUTED_ON if active_bit else CLR_STEP_MUTED
                elif active_bit:
                    clr = CLR_STEP_ON_LOW if velocity < 80 else CLR_STEP_ON
                else:
                    clr = CLR_STEP_OFF
                rect = pygame.Rect(cx, cy, cell_w - 4, cell_h)
                pygame.draw.rect(surface, clr, rect, border_radius=3)
                if is_cursor:
                    pygame.draw.rect(surface, CLR_CURSOR, rect, width=2, border_radius=3)
                if is_playhead:
                    pygame.draw.rect(surface, CLR_PLAYHEAD, rect, width=2, border_radius=3)

    def _draw_track_label(self, surface, ti, track, muted, is_cursor_row, y0, row_h):
        name = track["name"]
        note = note_name(track["note"])
        name_clr = CLR_TEXT_DIM if muted else (CLR_WHITE if is_cursor_row else CLR_TEXT)

        # Line 1: name + note
        head = f"{name} {note}"
        surface.blit(self.font_s.render(head[:16], True, name_clr), (4, y0 + 2))

        # Line 2: instrument (dim)
        instr = _short_instrument(track["sound"])
        surface.blit(self.font_s.render(instr[:16], True, CLR_TEXT_DIM), (4, y0 + 16))

        # Line 3: fx badges
        fx = track["effects"]
        bx = 4
        by = y0 + 30
        for on, label, clr in (
            (fx.reverb and fx.wetdry > 0, "R", CLR_FX_REVERB),
            (fx.delay, "D", CLR_FX_DELAY),
            (fx.chorus and fx.wetdry > 0, "C", CLR_FX_CHORUS),
        ):
            s = self.font_s.render(label, True, clr if on else CLR_STEP_OFF)
            surface.blit(s, (bx, by)); bx += s.get_width() + 3
        if fx.any_active() and fx.wetdry > 0 and (fx.reverb or fx.chorus or fx.delay):
            s = self.font_s.render(f"{fx.wetdry}%", True, CLR_TEXT_DIM)
            surface.blit(s, (bx, by)); bx += s.get_width() + 3
        if fx.crushing():
            s = self.font_s.render("crsh", True, CLR_FX_CRUSH)
            surface.blit(s, (bx, by))

        # Volume bar along the bottom of the label column
        vol = track.get("volume", 1.0)
        bar_x, bar_w = 4, TRACK_LABEL_W - 10
        bar_y = y0 + row_h - 7
        pygame.draw.rect(surface, CLR_STEP_OFF, (bar_x, bar_y, bar_w, 3), border_radius=2)
        fillw = int(bar_w * vol)
        if fillw > 0:
            pygame.draw.rect(surface, CLR_STEP_MUTED_ON if muted else CLR_STEP_ON,
                             (bar_x, bar_y, fillw, 3), border_radius=2)

    # ── Loop bar ──────────────────────────────────────────────────────────────
    def _draw_loop_bar(self, surface, y0):
        pygame.draw.rect(surface, CLR_LOOP_BG, (0, y0, SCREEN_W, LOOP_BAR_H))
        x = 8
        y = y0 + (LOOP_BAR_H - self.font_s.get_height()) // 2
        fname = os.path.basename(self.state.loop_file or "")
        if len(fname) > 26:
            fname = fname[:23] + "..."
        native = self.state.loop_native_bpm
        bpm_label = f"  {native}>{self.state.bpm}bpm" if native else ""
        s = self.font_s.render(f"> {fname}{bpm_label}", True, CLR_TEXT)
        surface.blit(s, (x, y)); x += s.get_width() + 12
        bar_w, bar_h = 120, 8
        by = y0 + (LOOP_BAR_H - bar_h) // 2
        pygame.draw.rect(surface, CLR_STEP_OFF, (x, by, bar_w, bar_h), border_radius=2)
        fill = int(bar_w * self.loop_player.position_fraction())
        if fill > 0:
            pygame.draw.rect(surface, CLR_LOOP_BAR_FG, (x, by, fill, bar_h), border_radius=2)
        x += bar_w + 10
        if self.loop_player.stretch_warning:
            surface.blit(self.font_s.render("STRETCH N/A", True, CLR_WARNING), (x, y))

    # ── Tap feedback ───────────────────────────────────────────────────────────
    def _draw_tap_feedback(self, surface):
        label = f"BPM: {self._tap_bpm}" if self._tap_bpm else "Tap..."
        text = self.font_l.render(label, True, CLR_WHITE)
        tw, th = text.get_size()
        bx = (SCREEN_W - tw) // 2 - 20
        by = (SCREEN_H - th) // 2 - 10
        bg = pygame.Surface((tw + 40, th + 20), pygame.SRCALPHA)
        bg.fill((0, 0, 0, 180))
        surface.blit(bg, (bx, by))
        surface.blit(text, (bx + 20, by + 10))
