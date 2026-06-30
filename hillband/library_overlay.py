"""Sound Library: recursive browser that groups files into instruments.

Identical logic to HillSequencer's library_overlay.py, with one addition:
  * Library.open() accepts drum_only=True to pre-set the filter to "DRUM"
    so that drum tracks default to browsing only percussion sounds.
  * The user can still clear that filter with X to browse everything.
"""

import os
import re

import pygame

import config
from audio.mixer import _parse_midi, _find_note, parse_root
from theory.notes import note_name

VISIBLE_ROWS = 11
_TRAIL_RE = re.compile(r"(?:\s+(?:\d+|rr\d+|v\d+|ppp|pp|mp|mf|fff|ff|p|f))+$", re.I)

_TAGS = [
    (("kick", "snare", "hat", "perc", "cymbal", "clap", "tom", "drum", "kit"), "DRUM", (225, 120, 120)),
    (("bass",), "BASS", (225, 165, 90)),
    (("lead",), "LEAD", (235, 215, 120)),
    (("pad",), "PAD", (120, 205, 200)),
    (("drone", "texture", "atmos", "ambient", "bed", "wash"), "AMB", (160, 170, 235)),
    (("choir", "vox", "vocal", "pixie"), "VOX", (225, 150, 205)),
    (("organ", "piano", "rhodes", "keys", "harpsichord", "clav", "celest"), "KEYS", (150, 210, 150)),
    (("harp", "bell", "glock", "vibr", "marimba", "xylo", "mallet", "pluck", "kalimba"), "MALLET", (205, 185, 145)),
    (("guitar", "banjo", "mandolin", "ukulele", "strumstick", "lute", "sitar", "dulcimer"), "STRING", (210, 175, 120)),
    (("trombone", "trumpet", "tuba", "cornet", "brass", "horn"), "BRASS", (225, 195, 130)),
    (("string", "violin", "cello", "viola", "sax", "flute", "recorder", "harmonica", "accordion", "clarinet", "oboe"), "INST", (175, 195, 215)),
    (("chord", "combo"), "CHORD", (190, 175, 220)),
    (("arp",), "ARP", (200, 165, 255)),
    (("noise", "sfx", "glitch", "siren", "fx"), "FX", (150, 150, 160)),
    (("synth", "saw", "square", "pulse"), "SYNTH", (180, 200, 150)),
]


def _tag(name, sid):
    text = f"{name} {sid}".lower()
    for keys, label, color in _TAGS:
        if any(k in text for k in keys):
            return label, color
    return None


FILTERS = [None, "FAV", "LOOP", "ONESHOT"] + [label for _k, label, _c in _TAGS]


def _filter_label(f):
    return {None: "All", "FAV": "Favorites", "LOOP": "Loops",
            "ONESHOT": "One-shots"}.get(f, f)


_LOOP_WORDS = ("loop", "drone", "pad", "texture", "atmos", "ambient", "bed",
               "wash", "drift", "swell", "string", "choir", "organ", "drone")
_SHOT_WORDS = ("drum", "kit", "kick", "snare", "hat", "perc", "clap", "pluck",
               "stab", "hit", "shot", "bell", "glock", "marimba", "mallet",
               "xylo", "piano", "pizz", "arp")


def _sound_loops(name, sid):
    text = f"{name} {sid}".lower()
    if any(w in text for w in _LOOP_WORDS):
        return True
    if any(w in text for w in _SHOT_WORDS):
        return False
    return True


def _instr_stem(filename: str):
    stem = os.path.splitext(os.path.basename(filename))[0]
    f = _find_note(stem)
    if f:
        _midi, s, e = f
        stem = stem[:s] + stem[e:]
    stem = _MIDI_NUM_TAG.sub("", stem)
    stem = re.sub(r"[ _\-.]+", " ", stem).strip()
    return _TRAIL_RE.sub("", stem).strip()


_KEY_PREFIX  = re.compile(r"^(?:\d{2,3}\s+)?(?:[A-Ga-g][#b]?m?\s+)?", re.I)
_FOLDER_TRAIL = re.compile(r"\s+(?:ds|free|lite|v\d+)$", re.I)
_MIDI_NUM_TAG = re.compile(r"[_\-]midi\d{2,3}", re.I)


def _display_name(raw, kind):
    if kind == "sound_single":
        s = os.path.splitext(raw)[0]
        s = re.sub(r"[_\-.]+", " ", s)
        s = re.sub(r"\s+", " ", s).strip()
        s = _KEY_PREFIX.sub("", s)
        s = _TRAIL_RE.sub("", s).strip()
        return s or os.path.splitext(raw)[0]
    s = re.sub(r"[_\-.]+", " ", raw)
    s = re.sub(r"\s+", " ", s).strip()
    s = s.title()
    s = _FOLDER_TRAIL.sub("", s).strip()
    return s or raw


def _group_files(d):
    groups = {}
    try:
        entries = list(os.scandir(d))
    except OSError:
        return groups
    for e in sorted(entries, key=lambda e: e.name):
        if not e.name.lower().endswith(".wav"):
            continue
        if e.name.startswith("._"):
            continue
        try:
            if not e.is_file(follow_symlinks=False):
                continue
        except OSError:
            continue
        midi = _parse_midi(e.name)
        groups.setdefault(_instr_stem(e.name), []).append((midi, e.path))
    for k in groups:
        groups[k].sort(key=lambda t: (t[0] is None, t[0]))
    return groups


def _wavs_recursive(d):
    out = []
    for dp, _dirs, files in os.walk(d):
        out += [os.path.join(dp, f) for f in files
                if f.lower().endswith(".wav") and not f.startswith("._")]
    return sorted(out)


def _is_loopy(path: str) -> bool:
    return "loop" in path.lower()


def _is_single_multisample(path: str) -> bool:
    if _is_loopy(path):
        return False
    groups = _group_files(path)
    if len(groups) != 1:
        return False
    items = next(iter(groups.values()))
    pitches = {m for m, _ in items if m is not None}
    return len(pitches) >= 2


def resolve_sid(root, sid):
    """Standalone sid -> (kind, payload) for restoring a track's instrument."""
    if not sid:
        return None
    try:
        if "::" in sid:
            dirrel, stem = sid.split("::", 1)
            g = _group_files(os.path.join(root, dirrel)).get(stem)
            if g:
                return ("sound_multi", [p for _m, p in g])
        else:
            path = os.path.join(root, sid)
            if os.path.isdir(path):
                return ("sound_multi", _wavs_recursive(path))
            if os.path.isfile(path):
                return ("sound_single", path)
    except OSError:
        pass
    return None


class Library:
    """Modal overlay: assigns an instrument to one or more tracks."""

    def __init__(self, root: str, font_normal=None, font_small=None, drum_root=None):
        self.root = root
        self._melodic_root = root
        self._drum_root = drum_root or root
        self.cur = root
        self.index = 0
        self.entries = []
        self.pending = None
        self.closed = False
        self.target_track = 0
        self.target_tracks = [0]
        self.filter_i = 0
        self._fav_sids = []
        self._all_cache = None

        pygame.font.init()
        self._font_n = font_normal or pygame.font.SysFont("monospace", config.FONT_SIZE_NORMAL)
        self._font_s = font_small or pygame.font.SysFont("monospace", config.FONT_SIZE_SMALL)
        self.refresh()

    def open(self, targets, state=None, drum_only=False):
        """targets: an int track index, or a list of track indices.

        drum_only=True pre-sets the filter to DRUM so percussion browsers open
        on the percussion category; the user can clear it with X.
        """
        if isinstance(targets, int):
            targets = [targets]
        self.target_tracks = sorted(set(targets)) or [0]
        self.target_track = self.target_tracks[0]
        self.closed = False
        self.pending = None
        new_root = self._drum_root if drum_only else self._melodic_root
        if new_root != self.root:
            self.root = new_root
            self.cur = new_root
            self._all_cache = None
        if drum_only and "DRUM" in FILTERS:
            self.filter_i = FILTERS.index("DRUM")
            self._all_cache = None      # force re-enumerate with filter active
        if state is not None and self.filter == "FAV":
            self._fav_sids = list(state.favorites)
        self.refresh()

    # ── Overlay input ─────────────────────────────────────────────────────────
    def handle_hat(self, hat, state):
        from input import button_map as bm
        from input.button_map import UP, DOWN, LEFT, RIGHT
        dirs = hat if isinstance(hat, set) else bm.hat_to_dirs(hat)
        if UP in dirs:
            self.move(-1)
        elif DOWN in dirs:
            self.move(1)
        elif LEFT in dirs:
            self.jump_folder(-1)
        elif RIGHT in dirs:
            self.jump_folder(1)

    def handle_button(self, btn, state):
        from input import button_map as bm
        if btn == bm.A:
            self.select(state)
        elif btn == bm.B:
            self.closed = True
        elif btn == bm.X:
            self.cycle_filter(state)
        elif btn == bm.Y:
            self.toggle_favorite(state)
        elif btn == bm.L1:
            self.move(-self.page())
        elif btn == bm.R1:
            self.move(self.page())
        elif btn == bm.L2:
            self.jump_folder(-1)
        elif btn == bm.R2:
            self.jump_folder(1)

    def handle_key(self, key, state):
        if key == pygame.K_UP:
            self.move(-1)
        elif key == pygame.K_DOWN:
            self.move(1)
        elif key == pygame.K_LEFT:
            self.jump_folder(-1)
        elif key == pygame.K_RIGHT:
            self.jump_folder(1)
        elif key in (pygame.K_RETURN, pygame.K_a):
            self.select(state)
        elif key == pygame.K_s:
            self.cycle_filter(state)
        elif key == pygame.K_f:
            self.toggle_favorite(state)
        elif key == pygame.K_LEFTBRACKET:
            self.move(-self.page())
        elif key == pygame.K_RIGHTBRACKET:
            self.move(self.page())
        elif key in (pygame.K_ESCAPE, pygame.K_b):
            self.closed = True

    @property
    def filter(self):
        return FILTERS[self.filter_i]

    @property
    def show_favs(self):
        return self.filter == "FAV"

    def page(self):
        return VISIBLE_ROWS

    def _rel(self, path):
        return os.path.relpath(path, self.root)

    def _resolve_sid(self, sid):
        try:
            if "::" in sid:
                dirrel, stem = sid.split("::", 1)
                g = _group_files(os.path.join(self.root, dirrel)).get(stem)
                if g:
                    return (stem, "sound_multi", [p for _m, p in g], sid)
            else:
                path = os.path.join(self.root, sid)
                if os.path.isdir(path):
                    return (os.path.basename(path), "sound_multi",
                            _wavs_recursive(path), sid)
                if os.path.isfile(path):
                    return (os.path.basename(path), "sound_single", path, sid)
        except OSError:
            pass
        return None

    def toggle_favorite(self, state):
        if not self.entries:
            return
        _n, kind, _p, sid = self.entries[self.index]
        if kind not in ("sound_multi", "sound_single"):
            return
        if sid in state.favorites:
            state.favorites.remove(sid)
        else:
            state.favorites.append(sid)
        if self.filter == "FAV":
            self._fav_sids = list(state.favorites)
            self.refresh()

    def cycle_filter(self, state, step=1):
        self.filter_i = (self.filter_i + step) % len(FILTERS)
        if self.filter == "FAV":
            self._fav_sids = list(state.favorites)
        self.index = 0
        self.refresh()

    def _enumerate_all(self):
        out = []
        for dp, dirs, _files in os.walk(self.root):
            for name in sorted(dirs):
                full = os.path.join(dp, name)
                if _is_single_multisample(full):
                    out.append((name, "sound_multi", _wavs_recursive(full),
                                self._rel(full)))
            dirs[:] = [d for d in sorted(dirs)
                       if not _is_single_multisample(os.path.join(dp, d))]
            loopy = _is_loopy(dp)
            for stem, items in sorted(_group_files(dp).items()):
                paths = [p for _m, p in items]
                pitches = {m for m, _ in items if m is not None}
                if not loopy and len(items) >= 2 and len(pitches) >= 2:
                    sid = f"{self._rel(dp)}::{stem}"
                    out.append((stem or os.path.basename(dp), "sound_multi",
                                paths, sid))
                else:
                    for _m, p in items:
                        out.append((os.path.basename(p), "sound_single", p,
                                    self._rel(p)))
        return out

    def _passes(self, entry):
        name, _kind, _payload, sid = entry
        f = self.filter
        if f == "LOOP":
            return _sound_loops(name, sid)
        if f == "ONESHOT":
            return not _sound_loops(name, sid)
        t = _tag(name, sid or name)
        return t is not None and t[0] == f

    def refresh(self):
        f = self.filter
        if f == "FAV":
            self.entries = [e for e in (self._resolve_sid(s) for s in self._fav_sids) if e]
            self.index = max(0, min(self.index, len(self.entries) - 1))
            return
        if f is not None:
            if self._all_cache is None:
                self._all_cache = self._enumerate_all()
            self.entries = [e for e in self._all_cache if self._passes(e)]
            self.index = max(0, min(self.index, len(self.entries) - 1))
            return
        entries = []
        if os.path.abspath(self.cur) != os.path.abspath(self.root):
            entries.append(("..", "up", None, None))
        try:
            names = sorted(os.listdir(self.cur))
        except OSError as e:
            print(f"[hillband] library read error: {e}")
            names = []
        for name in names:
            full = os.path.join(self.cur, name)
            if os.path.isdir(full):
                if _is_single_multisample(full):
                    entries.append((name, "sound_multi", _wavs_recursive(full),
                                    self._rel(full)))
                else:
                    entries.append((name, "folder", full, None))
        loopy = _is_loopy(self.cur)
        for stem, items in sorted(_group_files(self.cur).items()):
            paths = [p for _m, p in items]
            pitches = {m for m, _ in items if m is not None}
            if not loopy and len(items) >= 2 and len(pitches) >= 2:
                disp = stem or os.path.basename(self.cur)
                sid = f"{self._rel(self.cur)}::{stem}"
                entries.append((disp, "sound_multi", paths, sid))
            else:
                for _m, p in items:
                    entries.append((os.path.basename(p), "sound_single", p,
                                    self._rel(p)))
        self.entries = entries
        self.index = max(0, min(self.index, len(entries) - 1))

    def move(self, delta: int):
        if self.entries:
            self.index = (self.index + delta) % len(self.entries)

    def jump_folder(self, direction):
        n = len(self.entries)
        if not n:
            return
        i = self.index
        for _ in range(n):
            i = (i + direction) % n
            if self.entries[i][1] in ("folder", "up"):
                self.index = i
                return

    def select(self, state):
        if not self.entries:
            return
        name, kind, payload, sid = self.entries[self.index]
        if kind == "up":
            self.cur = os.path.dirname(self.cur)
            self.index = 0
            self.refresh()
        elif kind == "folder":
            self.cur = payload
            self.index = 0
            self.refresh()
        else:
            self.pending = (kind, payload, name, sid)
            self.closed = True

    # ── Rendering ─────────────────────────────────────────────────────────────
    def draw(self, surf, state):
        fn, fs = self._font_n, self._font_s
        surf.fill(config.CLR_BG)
        f = self.filter
        title = "Sound Library" if f is None else _filter_label(f)
        if len(self.target_tracks) > 1:
            tgt = "Tracks " + ",".join(f"T{t + 1}" for t in self.target_tracks)
        else:
            tname = state.tracks[self.target_track]["name"]
            tgt = f"Track {self.target_track + 1} ({tname})"
        surf.blit(fn.render(f"Assign instrument  ->  {tgt}"[:60],
                            True, (240, 240, 255)), (20, 12))
        if f is None:
            where = f"[{self._rel(self.cur)}]"
        else:
            where = f"filter: {_filter_label(f)}  ({len(self.entries)})"
        cur_sid = state.tracks[self.target_track].get("sound")
        surf.blit(fs.render(where, True, (150, 150, 170)), (20, 36))
        if f is not None and not self.entries:
            empty = ("No favorites yet (press Y on a sound)" if f == "FAV"
                     else f"No {_filter_label(f)} sounds found")
            surf.blit(fn.render(empty, True, (150, 150, 170)), (24, 92))

        y0, row_h, visible = 58, 26, VISIBLE_ROWS + 3
        start = max(0, min(self.index - visible // 2, max(0, len(self.entries) - visible)))
        for i, (name, kind, payload, sid) in enumerate(self.entries[start:start + visible]):
            idx = start + i
            y = y0 + i * row_h
            if idx == self.index:
                pygame.draw.rect(surf, config.CLR_OVERLAY_SEL, (12, y - 2, 616, row_h))
            star = "*" if sid in state.favorites else " "
            disp = _display_name(name, kind)
            label = {"up": "  .. (up)", "folder": f"{star}[+] {disp}",
                     "sound_multi": f"{star} {disp}",
                     "sound_single": f"{star} ~ {disp}"}[kind]
            active = kind in ("sound_multi", "sound_single") and sid == cur_sid
            color = (255, 230, 120) if active else (220, 220, 230)
            surf.blit(fn.render(label[:46], True, color), (24, y + 2))
            x = 624
            if kind == "sound_single":
                r = fs.render(note_name(parse_root(payload)), True, (150, 150, 170))
                x -= r.get_width(); surf.blit(r, (x, y + 5)); x -= 8
            if kind in ("folder", "sound_multi", "sound_single"):
                t = _tag(name, sid or name)
                if t:
                    tlabel, tcolor = t
                    ts = fs.render(tlabel, True, tcolor)
                    x -= ts.get_width(); surf.blit(ts, (x, y + 5))

        hint = fs.render(
            "D-pad: navigate  A: pick/enter  Y: favorite  X: filter  "
            "L1/R1: page  L2/R2: folder  B: cancel", True, (120, 120, 140))
        surf.blit(hint, (20, config.SCREEN_H - 22))
