"""HillSequencer — named sequence save/load overlay (from HillBeat).

A "sequence" is the full musical state: all patterns, per-track instrument/note/
effects assignments, BPM, swing, chain, loop — one JSON file each under
SEQUENCE_DIR (the global favorites list is NOT included).

Controls:
  Up/Down  navigate saved sequences
  A        load selected (applies + closes)
  X        save current state as a NEW sequence
  Y        overwrite selected
  L1       delete selected
  B/Esc    close
"""

from __future__ import annotations

import json
import os
import re
import time

import pygame

import config
from config import (
    SEQUENCE_DIR, SCREEN_W, SCREEN_H,
    CLR_OVERLAY_BG, CLR_OVERLAY_ITEM, CLR_OVERLAY_SEL,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WHITE, CLR_WARNING, CLR_BADGE_PLAY,
)
from input import button_map as bm

_SLUG_RE = re.compile(r"seq_(\d+)\.json$", re.I)


def _count_active(seq):
    total = 0
    for pat in seq.get("patterns", []):
        for track in pat.get("steps", []):
            for cell in track:
                if cell and cell[0]:
                    total += 1
    return total


class SequenceManager:
    def __init__(self, state, font_normal, font_small):
        self.state = state
        self._font_n = font_normal
        self._font_s = font_small
        self._entries = []
        self._cursor = 0
        self._scroll = 0
        self._visible = 13
        self.closed = False
        self.loaded = False
        self._toast = ""
        self._toast_until = 0.0

    def open(self):
        self.closed = False
        self.loaded = False
        self._toast = ""
        self._scan()
        self._cursor = max(0, min(self._cursor, len(self._entries) - 1))

    def _scan(self):
        entries = []
        if os.path.isdir(SEQUENCE_DIR):
            for fn in os.listdir(SEQUENCE_DIR):
                if not fn.lower().endswith(".json"):
                    continue
                path = os.path.join(SEQUENCE_DIR, fn)
                try:
                    with open(path) as f:
                        d = json.load(f)
                    entries.append({"path": path, "name": d.get("name", os.path.splitext(fn)[0]),
                                    "bpm": d.get("bpm", "?"), "active": _count_active(d)})
                except Exception:
                    continue
        entries.sort(key=lambda e: e["name"].lower())
        self._entries = entries

    def _notify(self, msg):
        self._toast = msg
        self._toast_until = time.monotonic() + 2.0

    # ── Input ─────────────────────────────────────────────────────────────────
    def handle_hat(self, hat):
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        if bm.UP in dirs:
            self._cursor = max(0, self._cursor - 1); self._clamp_scroll()
        elif bm.DOWN in dirs:
            self._cursor = min(max(0, len(self._entries) - 1), self._cursor + 1); self._clamp_scroll()

    def handle_button(self, btn):
        if btn == bm.A:
            self._load_selected()
        elif btn == bm.X:
            self._save_new()
        elif btn == bm.Y:
            self._overwrite_selected()
        elif btn == bm.L1:
            self._delete_selected()
        elif btn == bm.B:
            self.closed = True

    def handle_key(self, key):
        if key == pygame.K_UP:
            self.handle_hat((0, 1))
        elif key == pygame.K_DOWN:
            self.handle_hat((0, -1))
        elif key in (pygame.K_RETURN, pygame.K_a):
            self._load_selected()
        elif key == pygame.K_s:
            self._save_new()
        elif key == pygame.K_y:
            self._overwrite_selected()
        elif key == pygame.K_BACKSPACE:
            self._delete_selected()
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    # ── Actions ─────────────────────────────────────────────────────────────
    def _next_slot(self):
        os.makedirs(SEQUENCE_DIR, exist_ok=True)
        used = [int(m.group(1)) for fn in os.listdir(SEQUENCE_DIR)
                for m in [_SLUG_RE.search(fn)] if m]
        n = (max(used) + 1) if used else 1
        return n, os.path.join(SEQUENCE_DIR, f"seq_{n:03d}.json")

    def _write(self, path, name):
        os.makedirs(SEQUENCE_DIR, exist_ok=True)
        with open(path, "w") as f:
            json.dump(self.state.to_sequence_dict(name), f, indent=2)

    def _save_new(self):
        n, path = self._next_slot()
        name = f"Seq {n:02d}"
        try:
            self._write(path, name)
            self._scan()
            for i, e in enumerate(self._entries):
                if e["path"] == path:
                    self._cursor = i
                    break
            self._clamp_scroll()
            self._notify(f"Saved '{name}'")
        except Exception as e:
            self._notify(f"Save failed: {e}")

    def _overwrite_selected(self):
        if not self._entries:
            self._save_new()
            return
        e = self._entries[self._cursor]
        try:
            self._write(e["path"], e["name"])
            self._scan()
            self._notify(f"Overwrote '{e['name']}'")
        except Exception as ex:
            self._notify(f"Overwrite failed: {ex}")

    def _load_selected(self):
        if not self._entries:
            return
        e = self._entries[self._cursor]
        try:
            with open(e["path"]) as f:
                d = json.load(f)
            self.state.load_sequence_dict(d)
            self.loaded = True
            self.closed = True
        except Exception as ex:
            self._notify(f"Load failed: {ex}")

    def _delete_selected(self):
        if not self._entries:
            return
        e = self._entries[self._cursor]
        try:
            os.remove(e["path"])
            self._scan()
            self._cursor = max(0, min(self._cursor, len(self._entries) - 1))
            self._clamp_scroll()
            self._notify(f"Deleted '{e['name']}'")
        except Exception as ex:
            self._notify(f"Delete failed: {ex}")

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

        padding, w, row_h, header_h = 30, SCREEN_W - 60, 26, 46
        surface.blit(self._font_n.render("Sequences", True, CLR_BADGE_PLAY), (padding, padding))
        surface.blit(self._font_s.render(f"{len(self._entries)} saved   (patterns + instruments)",
                                         True, CLR_TEXT_DIM), (padding, padding + 22))

        y = padding + header_h
        if not self._entries:
            surface.blit(self._font_s.render("No saved sequences.  Press X to save the current one.",
                                             True, CLR_TEXT_DIM), (padding, y))
        else:
            for i in range(self._visible):
                fi = self._scroll + i
                if fi >= len(self._entries):
                    break
                e = self._entries[fi]
                is_sel = fi == self._cursor
                rect = pygame.Rect(padding, y, w, row_h - 2)
                pygame.draw.rect(surface, CLR_OVERLAY_SEL if is_sel else CLR_OVERLAY_ITEM, rect, border_radius=3)
                surface.blit(self._font_s.render(e["name"][:24], True, CLR_WHITE if is_sel else CLR_TEXT), (padding + 8, y + 5))
                meta = f"{e['bpm']} BPM   {e['active']} hits"
                ms = self._font_s.render(meta, True, CLR_TEXT if is_sel else CLR_TEXT_DIM)
                surface.blit(ms, (w + padding - ms.get_width() - 8, y + 5))
                y += row_h

        if self._toast and time.monotonic() < self._toast_until:
            surface.blit(self._font_s.render(self._toast, True, CLR_WARNING), (padding, SCREEN_H - 50))
        surface.blit(self._font_s.render(
            "Up/Dn navigate  A: load  X: save new  Y: overwrite  L1: delete  B: close",
            True, CLR_TEXT_DIM), (padding, SCREEN_H - 26))
