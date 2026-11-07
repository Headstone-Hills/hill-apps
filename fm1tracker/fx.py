"""FX command registry (MIDI-adapted).

Each command is a small function `handler(ctx, value)` that records its intent
on a StepCtx; the engine then applies the ctx (scheduling note events, changing
track state). To add a command, write a handler and decorate it:

    @command("XYZ", "short description", "value hint")
    def _xyz(ctx, v):
        ctx.something = v

IDs are assigned in registration order and are only an in-memory detail: projects
store commands by NAME, so the table can grow or be reordered without breaking
saved files. Id 0 is "no command".
"""

from __future__ import annotations

_names = ["---"]          # id -> name
_info = {}                # name -> (description, hint)
_handlers = {}            # name -> fn


class StepCtx(object):
    """Everything FX on one step/table-row can ask the engine to do."""
    def __init__(self):
        self.delay = 0             # DEL: ticks to delay the note
        self.kill = None           # KIL: release after N ticks
        self.arp = None            # ARP: (x, y) semitone offsets
        self.retrig = None         # RET: (interval ticks, velocity decay per repeat)
        self.bend = None           # PBN: 14-bit pitch bend
        self.ccs = []              # MCC: values (0-127) for the instrument's MCC CC
        self.pc = None             # PCH: program number
        self.tempo = None          # TPO: BPM
        self.vol = None            # VOL: velocity scale, 0x80 = 100%
        self.groove = None         # GRV
        self.tps = None            # TIC: ticks per step (0 = back to groove)
        self.hop = None            # HOP: raw value


def command(name, desc, hint="XX"):
    def deco(fn):
        if name not in _info:
            _names.append(name)
        _info[name] = (desc, hint)
        _handlers[name] = fn
        return fn
    return deco


def count():
    return len(_names)


def name_of(cmd_id):
    return _names[cmd_id] if 0 <= cmd_id < len(_names) else "---"


def id_of(name):
    try:
        return _names.index(name)
    except ValueError:
        return 0            # unknown command in a newer file -> ignored


def describe(cmd_id):
    n = name_of(cmd_id)
    return _info.get(n, ("", ""))


def run(cmd_id, ctx, value):
    h = _handlers.get(name_of(cmd_id))
    if h:
        h(ctx, value & 0xFF)


# --- the command set --------------------------------------------------------------
@command("ARP", "arpeggio: root, +X, +Y semitones", "XY")
def _arp(ctx, v):
    ctx.arp = (v >> 4, v & 15)


@command("DEL", "delay note by N ticks", "NN")
def _del(ctx, v):
    ctx.delay = v


@command("RET", "retrigger every X ticks, velocity -Y*4 per repeat", "XY")
def _ret(ctx, v):
    ctx.retrig = (max(1, v >> 4), (v & 15) * 4)


@command("KIL", "release note after N ticks", "NN")
def _kil(ctx, v):
    ctx.kill = v


@command("GRV", "select groove for this track", "NN")
def _grv(ctx, v):
    ctx.groove = v


@command("TIC", "ticks per step for this track (00 = use groove)", "NN")
def _tic(ctx, v):
    ctx.tps = v


@command("HOP", "X=repeats (0 end phrase, F forever), Y=target step", "XY")
def _hop(ctx, v):
    ctx.hop = v


@command("PBN", "pitch bend (80 = centre)", "NN")
def _pbn(ctx, v):
    ctx.bend = 8192 if v == 0x80 else (v * 16383) // 255


@command("MCC", "send CC (the instrument's MCC cc#) = NN/2", "NN")
def _mcc(ctx, v):
    ctx.ccs.append(v >> 1)


@command("PCH", "program change", "NN")
def _pch(ctx, v):
    ctx.pc = v & 127


@command("TPO", "set tempo in BPM", "NN")
def _tpo(ctx, v):
    ctx.tempo = v if v else None


@command("VOL", "velocity scale for this track (80 = 100%)", "NN")
def _vol(ctx, v):
    ctx.vol = v
