"""HillBeat — sample library browser.

Folder-navigation UI matching HillChord / HillBand style:
  D-pad up/down: navigate list
  D-pad left/right / L2/R2: jump to next/prev folder
  A: enter folder or load sample
  B: go up / cancel
  Y: toggle favorite
  X: cycle filter
  L1/R1: page up/down

result  — selected WAV path (voice/loop mode) or folder path (kit mode)
closed  — True when the overlay should be dismissed
mode    — "voice" | "loop" | "kit"
"""

import os
import re

import pygame

from constants import (
    SAMPLE_ROOT,
    SCREEN_W, SCREEN_H,
    CLR_OVERLAY_BG, CLR_OVERLAY_ITEM, CLR_OVERLAY_SEL,
    CLR_TEXT, CLR_TEXT_DIM, CLR_WARNING, CLR_WHITE, CLR_BLACK,
    FONT_SIZE_NORMAL, FONT_SIZE_SMALL,
    BTN_A, BTN_B, BTN_X, BTN_Y, BTN_L1, BTN_R1, BTN_L2, BTN_R2,
    HAT_UP, HAT_DOWN, HAT_LEFT, HAT_RIGHT,
)

VISIBLE_ROWS = 11

_TRAIL_RE = re.compile(r"(?:\s+(?:\d+|rr\d+|v\d+|ppp|pp|mp|mf|fff|ff|p|f))+$", re.I)

_TAGS = [
    (("kick",),                                   "KICK",  (225, 130, 100)),
    (("snare",),                                  "SNARE", (220, 180, 100)),
    (("hat", "hihat", "hi-hat"),                  "HAT",   (100, 200, 180)),
    (("perc", "rim", "clap", "tom", "cym"),       "PERC",  (180, 140, 220)),
]


def _tag(name):
    text = name.lower()
    for keys, label, color in _TAGS:
        if any(k in text for k in keys):
            return label, color
    return None


FILTERS = [None, "FAV", "KICK", "SNARE", "HAT", "PERC"]


def _filter_label(f):
    return {None: "All", "FAV": "Favorites"}.get(f, f)


_FOLDER_TRAIL = re.compile(r"\s+(?:ds|free|lite|v\d+)$", re.I)


def _display_name(raw, kind):
    s = os.path.splitext(raw)[0] if kind == "wav" else raw
    s = re.sub(r"[_\-.]+", " ", s)
    s = re.sub(r"\s+", " ", s).strip()
    if kind == "folder":
        s = s.title()
        s = _FOLDER_TRAIL.sub("", s).strip()
    else:
        s = _TRAIL_RE.sub("", s).strip()
    return s or raw


def _list_wavs(d):
    out = []
    try:
        for f in sorted(os.listdir(d)):
            if f.lower().endswith(".wav") and not f.startswith("._"):
                out.append(os.path.join(d, f))
    except OSError:
        pass
    return out


def _wavs_recursive(d):
    out = []
    for dp, _dirs, files in os.walk(d):
        out += [os.path.join(dp, f) for f in files
                if f.lower().endswith(".wav") and not f.startswith("._")]
    return sorted(out)


def _enumerate_all(root):
    """Flat list of all wav entries (for filtered views)."""
    out = []
    for dp, dirs, files in os.walk(root):
        dirs.sort()
        for f in sorted(files):
            if f.lower().endswith(".wav") and not f.startswith("._"):
                out.append((f, "wav", os.path.join(dp, f),
                             os.path.relpath(os.path.join(dp, f), root)))
    return out


class SampleLibrary:
    """Drop-in replacement for the old SampleLibrary with HillChord-style UI."""

    def __init__(self, font_normal=None, font_small=None):
        pygame.font.init()
        self._font_n = font_normal or pygame.font.SysFont("monospace", FONT_SIZE_NORMAL)
        self._font_s = font_small or pygame.font.SysFont("monospace", FONT_SIZE_SMALL)
        self.root = SAMPLE_ROOT
        self.cur  = SAMPLE_ROOT
        self.index = 0
        self.entries = []
        self._all_cache = None
        self._fav_sids = []
        self.filter_i = 0

        self.mode    = "voice"
        self.result  = None
        self.closed  = False

    # ── Public API (matches old SampleLibrary) ────────────────────────────────

    def warmup(self, favorites=None):
        """Pre-scan at startup so first open is instant."""
        self._all_cache = _enumerate_all(self.root)

    def open(self, mode, favorites=None):
        self.mode        = mode
        self._fav_sids   = list(favorites) if favorites else []
        self.result      = None
        self.closed      = False
        self.cur         = self.root
        self.filter_i    = 0
        self._all_cache  = None
        self.index       = 0
        self.refresh()

    # ── Navigation ────────────────────────────────────────────────────────────

    @property
    def filter(self):
        return FILTERS[self.filter_i]

    def _rel(self, path):
        return os.path.relpath(path, self.root)

    def refresh(self):
        f = self.filter
        if f == "FAV":
            self.entries = [
                (os.path.basename(s), "wav", os.path.join(self.root, s), s)
                for s in self._fav_sids
                if os.path.isfile(os.path.join(self.root, s))
            ]
            self.index = max(0, min(self.index, len(self.entries) - 1))
            return
        if f is not None:
            if self._all_cache is None:
                self._all_cache = _enumerate_all(self.root)
            self.entries = [
                e for e in self._all_cache
                if _tag(e[0]) is not None and _tag(e[0])[0] == f
            ]
            self.index = max(0, min(self.index, len(self.entries) - 1))
            return

        entries = []
        at_root = os.path.abspath(self.cur) == os.path.abspath(self.root)
        if not at_root:
            entries.append(("..", "up", None, None))

        if self.mode == "kit":
            # Kit mode: only show subdirectories of root as selectable items.
            try:
                for name in sorted(os.listdir(self.root)):
                    full = os.path.join(self.root, name)
                    if os.path.isdir(full) and not name.startswith("."):
                        wavs = _list_wavs(full)
                        label = f"{_display_name(name, 'folder')}  [{len(wavs)}]"
                        entries.append((label, "kit", full, self._rel(full)))
            except OSError:
                pass
        else:
            # Voice / loop mode: folder navigation + WAV files.
            try:
                names = sorted(os.listdir(self.cur))
            except OSError:
                names = []
            for name in names:
                full = os.path.join(self.cur, name)
                if os.path.isdir(full) and not name.startswith("."):
                    entries.append((name, "folder", full, None))
            for name in names:
                full = os.path.join(self.cur, name)
                if name.lower().endswith(".wav") and not name.startswith("._"):
                    if os.path.isfile(full):
                        entries.append((name, "wav", full, self._rel(full)))

        self.entries = entries
        self.index = max(0, min(self.index, len(entries) - 1))

    def move(self, delta):
        if self.entries:
            self.index = (self.index + delta) % len(self.entries)

    def page(self):
        return VISIBLE_ROWS

    def jump_folder(self, direction):
        n = len(self.entries)
        if not n:
            return
        i = self.index
        for _ in range(n):
            i = (i + direction) % n
            if self.entries[i][1] in ("folder", "up", "kit"):
                self.index = i
                return

    def cycle_filter(self, step=1):
        self.filter_i = (self.filter_i + step) % len(FILTERS)
        if self._all_cache is None:
            self._all_cache = _enumerate_all(self.root)
        self.index = 0
        self.refresh()

    def toggle_favorite(self):
        if not self.entries:
            return
        _n, kind, path, sid = self.entries[self.index]
        if kind != "wav":
            return
        if sid in self._fav_sids:
            self._fav_sids.remove(sid)
        else:
            self._fav_sids.append(sid)
        if self.filter == "FAV":
            self.refresh()

    def select(self, state_favorites=None):
        if not self.entries:
            return
        name, kind, path, sid = self.entries[self.index]
        if kind == "up":
            self.cur = os.path.dirname(self.cur)
            self.index = 0
            self.refresh()
        elif kind == "folder":
            self.cur = path
            self.index = 0
            self.refresh()
        elif kind in ("wav", "kit"):
            self.result = path
            self.closed = True
            if state_favorites is not None and self._fav_sids is not state_favorites:
                state_favorites.clear()
                state_favorites.extend(self._fav_sids)

    # ── Input handlers ────────────────────────────────────────────────────────

    def handle_hat(self, hat):
        if hat == HAT_UP:
            self.move(-1)
        elif hat == HAT_DOWN:
            self.move(1)
        elif hat == HAT_LEFT:
            self.jump_folder(-1)
        elif hat == HAT_RIGHT:
            self.jump_folder(1)

    def handle_button(self, btn):
        if btn == BTN_A:
            self.select()
        elif btn == BTN_B:
            if os.path.abspath(self.cur) != os.path.abspath(self.root) and self.filter is None:
                self.cur = os.path.dirname(self.cur)
                self.index = 0
                self.refresh()
            else:
                self.closed = True
        elif btn == BTN_X:
            self.cycle_filter()
        elif btn == BTN_Y:
            self.toggle_favorite()
        elif btn == BTN_L1:
            self.move(-self.page())
        elif btn == BTN_R1:
            self.move(self.page())
        elif btn == BTN_L2:
            self.jump_folder(-1)
        elif btn == BTN_R2:
            self.jump_folder(1)

    # ── Draw ──────────────────────────────────────────────────────────────────

    def draw(self, surface):
        fn, fs = self._font_n, self._font_s
        surface.fill((18, 18, 24))

        f = self.filter
        mode_str = {"voice": "Voice", "loop": "Loop", "kit": "Kit"}.get(self.mode, self.mode)
        title = "Sound Library"
        if f is not None:
            title += f"  [{_filter_label(f)}]"
        else:
            title += f"  — {mode_str}"
        surface.blit(fn.render(title[:50], True, (240, 240, 255)), (20, 12))

        if f is None:
            where = f"[{self._rel(self.cur)}]" if self.cur != self.root else "[root]"
        else:
            where = f"filter: {_filter_label(f)}  ({len(self.entries)} items)"
        surface.blit(fs.render(where, True, (150, 150, 170)), (20, 36))

        if f is not None and not self.entries:
            empty = ("No favorites yet — press Y on a sample" if f == "FAV"
                     else f"No {_filter_label(f)} samples found")
            surface.blit(fn.render(empty, True, (150, 150, 170)), (24, 92))

        y0, row_h, visible = 58, 26, VISIBLE_ROWS + 3
        start = max(0, min(self.index - visible // 2,
                           max(0, len(self.entries) - visible)))
        for i, (name, kind, path, sid) in enumerate(self.entries[start:start + visible]):
            idx = start + i
            y = y0 + i * row_h
            if idx == self.index:
                pygame.draw.rect(surface, CLR_OVERLAY_SEL, (12, y - 2, 616, row_h))

            is_fav = sid in self._fav_sids if sid else False
            star = "*" if is_fav else " "

            if kind == "up":
                label = "  .. (up)"
            elif kind == "folder":
                label = f"{star}[+] {_display_name(name, 'folder')}"
            elif kind == "kit":
                label = f"{star}[kit] {name}"
            else:
                label = f"{star} {_display_name(name, 'wav')}"

            active = kind == "wav" and self.result is not None and path == self.result
            color = (255, 230, 120) if active else (220, 220, 230)
            surface.blit(fn.render(label[:46], True, color), (24, y + 2))

            if kind == "wav":
                t = _tag(name)
                if t:
                    tlabel, tcolor = t
                    ts = fs.render(tlabel, True, tcolor)
                    x = 624 - ts.get_width()
                    surface.blit(ts, (x, y + 5))

        hint = fs.render(
            "D-pad: navigate  A: pick/enter  Y: favorite  X: filter  "
            "L1/R1: page  L2/R2: folder  B: up/close",
            True, (120, 120, 140))
        surface.blit(hint, (20, SCREEN_H - 22))
