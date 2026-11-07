"""Data model: Song -> Chain -> Phrase -> Step, plus Instrument/Table/Groove/Scale.

Plain Python (no pygame) so it is trivially unit-testable. Everything is stored
sparsely: only non-empty chains/phrases/etc. are written to disk.
"""

from __future__ import annotations

import copy

FORMAT_VERSION = 1

NUM_TRACKS = 8
SONG_ROWS = 128
CHAIN_LEN = 16
PHRASE_LEN = 16
NUM_CHAINS = 256
NUM_PHRASES = 256
NUM_INSTRUMENTS = 128
NUM_TABLES = 64
NUM_GROOVES = 32
NUM_SCALES = 16
TABLE_LEN = 16
CC_SLOTS = 8
FX_SLOTS = 3

EMPTY = 255          # "no value" in any byte-sized cell
NOTE_OFF = 128       # phrase-step note value meaning "release"

NOTE_NAMES = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"]


def note_name(n):
    if n == EMPTY:
        return "---"
    if n == NOTE_OFF:
        return "OFF"
    return "%s%d" % (NOTE_NAMES[n % 12], n // 12 - 1)    # MIDI 60 = C-4


def clamp(v, lo, hi):
    return lo if v < lo else hi if v > hi else v


class Step(object):
    """One phrase step."""
    __slots__ = ("note", "vel", "instr", "fx")

    def __init__(self):
        self.note = EMPTY
        self.vel = EMPTY            # EMPTY = instrument default
        self.instr = EMPTY          # EMPTY = keep the track's current instrument
        self.fx = [[0, 0] for _ in range(FX_SLOTS)]   # [cmd id, value]

    def is_empty(self):
        return (self.note == EMPTY and self.vel == EMPTY and self.instr == EMPTY
                and all(c == 0 for c, _ in self.fx))

    def clear(self):
        self.__init__()

    def copy(self):
        s = Step()
        s.note, s.vel, s.instr = self.note, self.vel, self.instr
        s.fx = [list(f) for f in self.fx]
        return s


class Phrase(object):
    def __init__(self):
        self.steps = [Step() for _ in range(PHRASE_LEN)]

    def is_empty(self):
        return all(s.is_empty() for s in self.steps)

    def copy(self):
        p = Phrase()
        p.steps = [s.copy() for s in self.steps]
        return p


class Chain(object):
    def __init__(self):
        self.phrase = [EMPTY] * CHAIN_LEN
        self.transpose = [0] * CHAIN_LEN

    def length(self):
        """Steps up to and including the last non-empty one (min 1)."""
        for i in range(CHAIN_LEN - 1, -1, -1):
            if self.phrase[i] != EMPTY:
                return i + 1
        return 1

    def is_empty(self):
        return all(p == EMPTY for p in self.phrase)

    def copy(self):
        c = Chain()
        c.phrase, c.transpose = list(self.phrase), list(self.transpose)
        return c


class Instrument(object):
    """MIDI OUT instrument."""
    def __init__(self, idx=0, channel=0):
        self.name = "INST %02X" % idx
        self.channel = channel          # 0-15
        self.program = idx & 127
        self.bank_msb = EMPTY           # EMPTY = don't send
        self.bank_lsb = EMPTY
        self.transpose = 0
        self.gate = 0                   # ticks; 0 = full step length
        self.velocity = 100
        self.send_mode = 0              # 0: CCs on instrument select, 1: on every note
        self.mcc_cc = 74                # CC number driven by the MCC fx
        self.table = EMPTY
        self.table_rate = 6             # ticks per table row (tick-mode tables)
        self.ccs = [[EMPTY, 0] for _ in range(CC_SLOTS)]   # [cc number, value]

    def copy(self):
        return copy.deepcopy(self)


class TableRow(object):
    __slots__ = ("trn", "vel", "fx")

    def __init__(self):
        self.trn = 0                    # semitones
        self.vel = 0                    # velocity offset
        self.fx = [[0, 0], [0, 0]]

    def is_empty(self):
        return self.trn == 0 and self.vel == 0 and all(c == 0 for c, _ in self.fx)


class Table(object):
    def __init__(self):
        self.mode = 0                   # 0: one row per note (per-note), 1: rows run on ticks
        self.rows = [TableRow() for _ in range(TABLE_LEN)]

    def length(self):
        for i in range(TABLE_LEN - 1, -1, -1):
            if not self.rows[i].is_empty():
                return i + 1
        return 1


class Groove(object):
    """Per-step tick lengths (cycled over the 16 phrase steps)."""
    def __init__(self, ticks=None):
        self.ticks = list(ticks) if ticks else [6] * PHRASE_LEN


class Scale(object):
    def __init__(self, name="CHROMATIC", root=0, mask=None):
        self.name, self.root = name, root
        self.mask = list(mask) if mask else [True] * 12     # relative to root

    def allows(self, note):
        return self.mask[(note - self.root) % 12]

    def snap(self, note, direction=0):
        """Nearest allowed note; direction -1/+1 searches only that way."""
        if self.allows(note):
            return note
        for d in range(1, 12):
            if direction >= 0 and 0 <= note + d <= 127 and self.allows(note + d):
                return note + d
            if direction <= 0 and 0 <= note - d <= 127 and self.allows(note - d):
                return note - d
        return note

    def step(self, note, direction):
        """Next allowed note above/below `note`."""
        n = note
        for _ in range(13):
            n += direction
            if n < 0 or n > 127:
                return note
            if self.allows(n):
                return n
        return note


_SCALE_PRESETS = [
    ("CHROMATIC", [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("MAJOR", [0, 2, 4, 5, 7, 9, 11]),
    ("MINOR", [0, 2, 3, 5, 7, 8, 10]),
    ("DORIAN", [0, 2, 3, 5, 7, 9, 10]),
    ("MIXOLYD", [0, 2, 4, 5, 7, 9, 10]),
    ("PENT MAJ", [0, 2, 4, 7, 9]),
    ("PENT MIN", [0, 3, 5, 7, 10]),
    ("BLUES", [0, 3, 5, 6, 7, 10]),
]


class Project(object):
    def __init__(self):
        self.name = "UNTITLED"
        self.bpm = 120
        self.transpose = 0
        self.midi_channel = 0           # default channel for new instruments
        self.latency_ms = 0
        self.loop_song = True
        self.arp_rate = 3               # ticks per ARP note
        self.scale = EMPTY              # active scale for note entry (EMPTY = off)
        self.groove = 0                 # default groove
        self.song = [[EMPTY] * NUM_TRACKS for _ in range(SONG_ROWS)]
        self.chains = {}
        self.phrases = {}
        self.instruments = {}
        self.tables = {}
        self.grooves = {0: Groove(), 1: Groove([7, 5] * 8), 2: Groove([8, 4] * 8),
                        3: Groove([4, 8] * 8)}
        self.scales = {i: Scale(n, 0, [k in ivs for k in range(12)])
                       for i, (n, ivs) in enumerate(_SCALE_PRESETS)}

    # -- lazy accessors (create on demand) -------------------------------------
    def chain(self, i):
        if i not in self.chains:
            self.chains[i] = Chain()
        return self.chains[i]

    def phrase(self, i):
        if i not in self.phrases:
            self.phrases[i] = Phrase()
        return self.phrases[i]

    def instrument(self, i):
        if i not in self.instruments:
            self.instruments[i] = Instrument(i, self.midi_channel)
        return self.instruments[i]

    def table(self, i):
        if i not in self.tables:
            self.tables[i] = Table()
        return self.tables[i]

    def groove_(self, i):
        if i not in self.grooves:
            self.grooves[i] = Groove()
        return self.grooves[i]

    def scale_(self, i):
        if i not in self.scales:
            self.scales[i] = Scale("SCALE %X" % i)
        return self.scales[i]

    # -- helpers ----------------------------------------------------------------
    def song_last_row(self):
        for r in range(SONG_ROWS - 1, -1, -1):
            if any(c != EMPTY for c in self.song[r]):
                return r
        return -1

    def next_free_chain(self):
        for i in range(NUM_CHAINS):
            if i not in self.chains or self.chains[i].is_empty():
                return i
        return 0

    def next_free_phrase(self):
        for i in range(NUM_PHRASES):
            if i not in self.phrases or self.phrases[i].is_empty():
                return i
        return 0

    def clone_chain(self, i):
        n = self.next_free_chain()
        self.chains[n] = self.chain(i).copy()
        return n

    def clone_phrase(self, i):
        n = self.next_free_phrase()
        self.phrases[n] = self.phrase(i).copy()
        return n

    def active_scale(self):
        return None if self.scale == EMPTY else self.scale_(self.scale)

    def sanitize(self):
        """Clamp everything into range (guards against hand-edited/corrupt files)."""
        self.bpm = clamp(int(self.bpm), 20, 300)
        self.midi_channel = clamp(int(self.midi_channel), 0, 15)
        self.latency_ms = clamp(int(self.latency_ms), 0, 500)
        self.arp_rate = clamp(int(self.arp_rate), 1, 24)
        for g in self.grooves.values():
            g.ticks = [clamp(int(t), 1, 24) for t in (g.ticks + [6] * PHRASE_LEN)[:PHRASE_LEN]]


# -- serialization -------------------------------------------------------------
def _fx_name(i):
    from fx import name_of
    return name_of(i)


def _fx_id(name):
    from fx import id_of
    return id_of(name)


def project_to_dict(p):
    d = {"version": FORMAT_VERSION, "app": "fm1tracker",
         "name": p.name, "bpm": p.bpm, "transpose": p.transpose,
         "midi_channel": p.midi_channel, "latency_ms": p.latency_ms,
         "loop_song": p.loop_song, "arp_rate": p.arp_rate, "scale": p.scale,
         "groove": p.groove}
    last = p.song_last_row()
    d["song"] = [list(p.song[r]) for r in range(last + 1)]
    d["chains"] = {str(i): {"p": c.phrase, "t": c.transpose}
                   for i, c in sorted(p.chains.items()) if not c.is_empty()}
    ph = {}
    for i, phr in sorted(p.phrases.items()):
        if phr.is_empty():
            continue
        steps = {}
        for k, s in enumerate(phr.steps):
            if not s.is_empty():
                steps[str(k)] = {"n": s.note, "v": s.vel, "i": s.instr,
                                 "f": [[_fx_name(c), v] for c, v in s.fx if c]}
        ph[str(i)] = steps
    d["phrases"] = ph
    d["instruments"] = {str(i): {
        "name": x.name, "ch": x.channel, "prog": x.program, "bank_msb": x.bank_msb,
        "bank_lsb": x.bank_lsb, "trn": x.transpose, "gate": x.gate, "vel": x.velocity,
        "send": x.send_mode, "mcc": x.mcc_cc, "table": x.table, "trate": x.table_rate,
        "ccs": x.ccs} for i, x in sorted(p.instruments.items())}
    d["tables"] = {str(i): {"mode": t.mode, "rows": [
        {"trn": r.trn, "vel": r.vel, "f": [[_fx_name(c), v] for c, v in r.fx if c]}
        for r in t.rows]} for i, t in sorted(p.tables.items())}
    d["grooves"] = {str(i): g.ticks for i, g in sorted(p.grooves.items())}
    d["scales"] = {str(i): {"name": s.name, "root": s.root, "mask": [int(m) for m in s.mask]}
                   for i, s in sorted(p.scales.items())}
    return d


def _g(d, k, default):
    v = d.get(k, default)
    return default if v is None else v


def project_from_dict(d):
    ver = d.get("version", 0)
    if ver > FORMAT_VERSION:
        raise ValueError("project is format v%s; this build reads up to v%s" % (ver, FORMAT_VERSION))
    p = Project()
    p.name = str(_g(d, "name", p.name))[:16]
    p.bpm, p.transpose = _g(d, "bpm", 120), _g(d, "transpose", 0)
    p.midi_channel, p.latency_ms = _g(d, "midi_channel", 0), _g(d, "latency_ms", 0)
    p.loop_song, p.arp_rate = bool(_g(d, "loop_song", True)), _g(d, "arp_rate", 3)
    p.scale, p.groove = _g(d, "scale", EMPTY), _g(d, "groove", 0)
    for r, row in enumerate(d.get("song", [])[:SONG_ROWS]):
        for t, c in enumerate(row[:NUM_TRACKS]):
            p.song[r][t] = c
    for k, c in d.get("chains", {}).items():
        ch = Chain()
        ch.phrase = (list(c.get("p", [])) + [EMPTY] * CHAIN_LEN)[:CHAIN_LEN]
        ch.transpose = (list(c.get("t", [])) + [0] * CHAIN_LEN)[:CHAIN_LEN]
        p.chains[int(k)] = ch
    for k, steps in d.get("phrases", {}).items():
        phr = Phrase()
        for sk, sd in steps.items():
            si = int(sk)
            if 0 <= si < PHRASE_LEN:
                s = phr.steps[si]
                s.note, s.vel, s.instr = sd.get("n", EMPTY), sd.get("v", EMPTY), sd.get("i", EMPTY)
                for slot, (name, val) in enumerate(sd.get("f", [])[:FX_SLOTS]):
                    s.fx[slot] = [_fx_id(name), val]
        p.phrases[int(k)] = phr
    for k, x in d.get("instruments", {}).items():
        i = Instrument(int(k), p.midi_channel)
        i.name = x.get("name", i.name)
        i.channel, i.program = x.get("ch", 0), x.get("prog", 0)
        i.bank_msb, i.bank_lsb = x.get("bank_msb", EMPTY), x.get("bank_lsb", EMPTY)
        i.transpose, i.gate, i.velocity = x.get("trn", 0), x.get("gate", 0), x.get("vel", 100)
        i.send_mode, i.mcc_cc = x.get("send", 0), x.get("mcc", 74)
        i.table, i.table_rate = x.get("table", EMPTY), x.get("trate", 6)
        ccs = x.get("ccs", [])
        i.ccs = [list(c) for c in (ccs + [[EMPTY, 0]] * CC_SLOTS)[:CC_SLOTS]]
        p.instruments[int(k)] = i
    for k, t in d.get("tables", {}).items():
        tb = Table()
        tb.mode = t.get("mode", 0)
        for ri, rd in enumerate(t.get("rows", [])[:TABLE_LEN]):
            r = tb.rows[ri]
            r.trn, r.vel = rd.get("trn", 0), rd.get("vel", 0)
            for slot, (name, val) in enumerate(rd.get("f", [])[:2]):
                r.fx[slot] = [_fx_id(name), val]
        p.tables[int(k)] = tb
    for k, ticks in d.get("grooves", {}).items():
        p.grooves[int(k)] = Groove(ticks)
    for k, s in d.get("scales", {}).items():
        p.scales[int(k)] = Scale(s.get("name", "SCALE"), s.get("root", 0),
                                 [bool(m) for m in s.get("mask", [1] * 12)])
    p.sanitize()
    return p
