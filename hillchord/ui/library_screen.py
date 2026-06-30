"""Sound Library: a recursive browser that groups files into instruments.

A folder can hold several instruments. Files are grouped by their instrument
name (the filename with the note/dynamic stripped):
  * a group spanning multiple pitches  -> one multisample instrument,
  * a lone file                        -> a single-sample (pitch-shifted) sound.
A sub-folder that is itself a single multisample is shown as one selectable
sound; otherwise it's a container you navigate into.

Entries are (display_name, kind, payload, sid):
  kind "up"           payload None            -> go up
  kind "folder"       payload dir path        -> descend
  kind "sound_multi"  payload list of files   -> load multisample
  kind "sound_single" payload file path       -> load pitch-shifted single
`sid` is the stable id stored in state.sound (relpath, or "dirrel::stem").
"""

import os
import re

import pygame

import config
from audio.mixer import _parse_midi, _find_note, parse_root
from theory.notes import note_name

VISIBLE_ROWS = 11
# Trailing sample-index / dynamic / round-robin tokens to drop when deriving the
# instrument name (so 'FMLead_Z_013_Db' and 'FMLead_Z_052_A' group together).
_TRAIL_RE = re.compile(r"(?:\s+(?:\d+|rr\d+|v\d+|ppp|pp|mp|mf|fff|ff|p|f))+$", re.I)

# Auto-tag a sound by keywords in its name/path. First match wins, so order
# matters (e.g. 'harpsichord'->KEYS and 'harp'->MALLET come before 'arp'->ARP).
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
    (("string", "violin", "cello", "viola", "sax", "flute", "recorder", "brass", "horn", "clarinet", "oboe"), "INST", (175, 195, 215)),
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


# Library filter cycle (B button). None = normal folder browsing; "FAV" = the
# favorites view; "LOOP"/"ONESHOT" split by playback type; the rest are the
# category tags above. Only categories that actually appear in _TAGS are listed.
FILTERS = [None, "FAV", "LOOP", "ONESHOT"] + [label for _k, label, _c in _TAGS]


def _filter_label(f):
    return {None: "All", "FAV": "Favorites", "LOOP": "Loops",
            "ONESHOT": "One-shots"}.get(f, f)


# Cheap name/path heuristic for whether a sound is a sustained loop (held =
# drone) vs a one-shot (plays once). The engine's true call needs the audio, so
# this approximates from keywords for browsing/filtering only.
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
    return True                              # default: assume it sustains


def _instr_stem(filename: str):
    """Instrument name = filename with all pitch tokens removed, so every
    per-pitch file of one instrument maps to the same stem."""
    stem = os.path.splitext(os.path.basename(filename))[0]
    f = _find_note(stem)
    if f:
        _midi, s, e = f
        stem = stem[:s] + stem[e:]
    stem = _MIDI_NUM_TAG.sub("", stem)   # strip _midi036 / _midi060 etc.
    stem = re.sub(r"[ _\-.]+", " ", stem).strip()
    return _TRAIL_RE.sub("", stem).strip()


_KEY_PREFIX = re.compile(r"^(?:\d{2,3}\s+)?(?:[A-Ga-g][#b]?m?\s+)?", re.I)


_FOLDER_TRAIL = re.compile(r"\s+(?:ds|free|lite|v\d+)$", re.I)

# Explicit MIDI-number tags embedded in filenames: _midi036, _midi060, etc.
# These encode the pitch and must be stripped before grouping, just like the
# note-name token (C2, Eb4).  Without this, every pitch gets a distinct stem
# and _group_files() can never unite them into one multisample instrument.
_MIDI_NUM_TAG = re.compile(r"[_\-]midi\d{2,3}", re.I)


def _display_name(raw, kind):
    """Tidy a raw filename/stem for display: drop extension, separators ->
    spaces, and leading BPM/key tags.  Also cleans folder/multisample names."""
    if kind == "sound_single":
        s = os.path.splitext(raw)[0]
        s = re.sub(r"[_\-.]+", " ", s)
        s = re.sub(r"\s+", " ", s).strip()
        s = _KEY_PREFIX.sub("", s)
        s = _TRAIL_RE.sub("", s).strip()
        return s or os.path.splitext(raw)[0]
    # Folders and multisamples: clean underscores, title-case, strip noise suffixes.
    s = re.sub(r"[_\-.]+", " ", raw)
    s = re.sub(r"\s+", " ", s).strip()
    s = s.title()
    s = _FOLDER_TRAIL.sub("", s).strip()
    return s or raw


def _group_files(d):
    """Immediate WAVs in dir d grouped by instrument stem -> [(midi, path), ...].
    Uses os.scandir (one syscall, cached is_file) — far faster than listdir +
    os.path.isfile per entry, which crawls on exFAT with many files."""
    groups = {}
    try:
        entries = list(os.scandir(d))
    except OSError:
        return groups
    for e in sorted(entries, key=lambda e: e.name):
        if not e.name.lower().endswith(".wav"):
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
        out += [os.path.join(dp, f) for f in files if f.lower().endswith(".wav")]
    return sorted(out)


def _is_loopy(path: str) -> bool:
    """Folders of *loops* (rhythmic phrases) shouldn't be treated as pitched
    multisample instruments even when their files share a name + key tag."""
    return "loop" in path.lower()


def _is_single_multisample(path: str) -> bool:
    """True if the sub-folder is ONE multisample (one instrument, many pitches)."""
    if _is_loopy(path):
        return False
    groups = _group_files(path)
    if len(groups) != 1:
        return False
    items = next(iter(groups.values()))
    pitches = {m for m, _ in items if m is not None}
    return len(pitches) >= 2


class Library:
    def __init__(self, root: str):
        self.root = root
        self.cur = root
        self.index = 0
        self.entries = []
        self.pending = None
        self.filter_i = 0                # index into FILTERS (0 = no filter)
        self._fav_sids = []              # snapshot of state.favorites for the view
        self._all_cache = None           # lazily-built flat list of every sound
        self.refresh()

    @property
    def filter(self):
        return FILTERS[self.filter_i]

    @property
    def show_favs(self):                 # kept for callers that check it
        return self.filter == "FAV"

    def page(self):
        return VISIBLE_ROWS

    def _rel(self, path):
        return os.path.relpath(path, self.root)

    def _resolve_sid(self, sid):
        """Turn a saved sid back into an entry tuple, or None if it's gone."""
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
        """Star/unstar the selected sound."""
        if not self.entries:
            return
        _n, kind, _p, sid = self.entries[self.index]
        if kind not in ("sound_multi", "sound_single"):
            return
        if sid in state.favorites:
            state.favorites.remove(sid)
        else:
            state.favorites.append(sid)
        if self.filter == "FAV":            # keep the filtered view in sync
            self._fav_sids = list(state.favorites)
            self.refresh()

    def cycle_filter(self, state, step=1):
        """X button: advance through the filter cycle (All -> Favorites ->
        Loops -> One-shots -> PAD/LEAD/... -> All)."""
        self.filter_i = (self.filter_i + step) % len(FILTERS)
        if self.filter == "FAV":
            self._fav_sids = list(state.favorites)
        self.index = 0
        self.refresh()

    def _enumerate_all(self):
        """Flat list of every selectable sound across the whole library, built
        once and reused while a type/loop filter is active."""
        out = []
        for dp, dirs, _files in os.walk(self.root):
            for name in sorted(dirs):
                full = os.path.join(dp, name)
                if _is_single_multisample(full):
                    out.append((name, "sound_multi", _wavs_recursive(full),
                                self._rel(full)))
            # Don't descend into single-multisample folders (they're one sound).
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
        t = _tag(name, sid or name)         # a category tag filter
        return t is not None and t[0] == f

    def refresh(self):
        f = self.filter
        if f == "FAV":                      # flat list of favorited sounds
            self.entries = [e for e in (self._resolve_sid(s) for s in self._fav_sids) if e]
            self.index = max(0, min(self.index, len(self.entries) - 1))
            return
        if f is not None:                   # flat, library-wide type/loop filter
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
            print(f"[hillchord] library read error: {e}")
            names = []

        # Sub-folders: a single multisample, else a container to descend into.
        for name in names:
            full = os.path.join(self.cur, name)
            if os.path.isdir(full):
                if _is_single_multisample(full):
                    entries.append((name, "sound_multi", _wavs_recursive(full),
                                    self._rel(full)))
                else:
                    entries.append((name, "folder", full, None))

        # Loose files in this folder, grouped into instruments (but a loops
        # folder keeps each file as its own single sample, not a multisample).
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

    def select(self, state, mixer):
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
        else:                                   # sound_multi / sound_single
            self.pending = (kind, payload, name, sid)

    def commit_pending(self, state, mixer):
        if not self.pending:
            return
        kind, payload, _name, sid = self.pending
        self.pending = None
        if kind == "sound_multi":
            mixer.load_files(payload, token=sid)
        else:
            mixer.load_single(payload, token=sid)
        state.sound = sid
        mixer.prewarm_async(state.effects, state.bpm)

    def load_sid(self, sid, state, mixer):
        """Restore a previously-saved sound id at startup."""
        if not sid:
            return
        try:
            if "::" in sid:
                dirrel, stem = sid.split("::", 1)
                group = _group_files(os.path.join(self.root, dirrel)).get(stem)
                if group:
                    mixer.load_files([p for _m, p in group], token=sid)
                    state.sound = sid
                    mixer.prewarm_async(state.effects, state.bpm)
                return
            path = os.path.join(self.root, sid)
            if os.path.isdir(path):
                mixer.load_sound(path, token=sid)
                state.sound = sid
                mixer.prewarm_async(state.effects, state.bpm)
            elif os.path.isfile(path) and path.lower().endswith(".wav"):
                mixer.load_single(path, token=sid)
                state.sound = sid
                mixer.prewarm_async(state.effects, state.bpm)
        except Exception as e:
            print(f"[hillchord] could not restore sound '{sid}': {e}")

    # ---- rendering ----
    def draw(self, surf, state, fonts):
        surf.fill((18, 18, 24))
        f = self.filter
        title = "Sound Library" if f is None else _filter_label(f)
        surf.blit(fonts["big"].render(title, True, (240, 240, 255)), (20, 16))
        if f is None:
            where = f"[{self._rel(self.cur)}]"
        else:
            where = f"filter: {_filter_label(f)}  ({len(self.entries)})"
        sub = fonts["small"].render(
            f"BPM {state.bpm}   Metronome {'ON' if state.metronome_on else 'off'}"
            f"   {where}", True, (150, 150, 170))
        surf.blit(sub, (20, 58))
        if f is not None and not self.entries:
            empty = ("No favorites yet (press Y on a sound)" if f == "FAV"
                     else f"No {_filter_label(f)} sounds found")
            surf.blit(fonts["mid"].render(empty, True, (150, 150, 170)), (24, 100))

        y0, row_h, visible = 92, 30, VISIBLE_ROWS
        start = max(0, self.index - visible // 2)
        for i, (name, kind, payload, sid) in enumerate(self.entries[start:start + visible]):
            idx = start + i
            y = y0 + i * row_h
            if idx == self.index:
                pygame.draw.rect(surf, (50, 60, 90), (12, y - 2, 616, row_h))
            star = "*" if sid in state.favorites else " "
            disp = _display_name(name, kind)
            label = {"up": "  .. (up)", "folder": f"{star}[+] {disp}",
                     "sound_multi": f"{star} {disp}",
                     "sound_single": f"{star} ~ {disp}"}[kind]
            active = kind in ("sound_multi", "sound_single") and sid == state.sound
            color = (255, 230, 120) if active else (220, 220, 230)
            surf.blit(fonts["mid"].render(label, True, color), (24, y))

            # Right side: a category tag, then (for single-samples) the root.
            x = 624
            if kind == "sound_single":
                r = fonts["small"].render(note_name(parse_root(payload)), True, (150, 150, 170))
                x -= r.get_width()
                surf.blit(r, (x, y + 4))
                x -= 8
            if kind in ("folder", "sound_multi", "sound_single"):
                t = _tag(name, sid or name)
                if t:
                    tlabel, tcolor = t
                    ts = fonts["small"].render(tlabel, True, tcolor)
                    x -= ts.get_width()
                    surf.blit(ts, (x, y + 4))

        hint = fonts["small"].render(
            "D-pad: navigate  A: load  Y: favorite  X: filter  L1/R1: page  "
            "L2/R2: folder  L/R: BPM  Sel: metronome  B: up/close",
            True, (120, 120, 140))
        surf.blit(hint, (20, 452))

        if self.pending:
            box = pygame.Rect(120, 200, 400, 70)
            pygame.draw.rect(surf, (40, 50, 75), box, border_radius=8)
            pygame.draw.rect(surf, (120, 140, 190), box, width=2, border_radius=8)
            msg = fonts["mid"].render(f"Loading {self.pending[2][:28]}...", True, (240, 240, 255))
            surf.blit(msg, (box.x + 20, box.y + 22))
