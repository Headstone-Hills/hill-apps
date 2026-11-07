"""USB descriptor parser for USB Audio Class 1.0 streaming interfaces.

Reads the raw descriptor blob the kernel exposes at /sys/bus/usb/devices/<dev>/descriptors
(or any saved dump) and returns every audio-streaming alternate setting that has an
isochronous endpoint, with the format the endpoint carries. The app uses this at runtime
instead of hardcoding FM-1 values, so other firmware revisions and generic USB mics work.

UAC2 devices (class-specific descriptors with bcdADC 2.00) are detected and reported but
not decoded: they need clock-source handling and are a later step.
"""

from __future__ import annotations

import os

DT_CONFIG, DT_INTERFACE, DT_ENDPOINT, DT_CS_INTERFACE = 0x02, 0x04, 0x05, 0x24
SUB_AS_GENERAL, SUB_FORMAT_TYPE = 0x01, 0x02
CLASS_AUDIO, SUBCLASS_CONTROL, SUBCLASS_STREAMING, SUBCLASS_MIDI = 1, 1, 2, 3
SYNC_NAMES = {0: "none", 1: "async", 2: "adaptive", 3: "sync"}


class StreamAlt(object):
    """One audio-streaming alternate setting with an isochronous endpoint."""
    def __init__(self):
        self.iface = self.alt = self.ep = self.max_packet = self.interval = 0
        self.sync = "none"
        self.channels = self.subframe = self.bits = 0
        self.rates = []
        self.uac2 = False

    @property
    def direction(self):
        return "in" if self.ep & 0x80 else "out"      # "in" = device -> host (capture)

    @property
    def frame_bytes(self):
        return self.channels * self.subframe

    def __repr__(self):
        return ("<iface %d alt %d ep 0x%02x %s %s maxpkt %d  %dch %d-bit(%dB) rates %s>" %
                (self.iface, self.alt, self.ep, self.direction, self.sync, self.max_packet,
                 self.channels, self.bits, self.subframe, self.rates))


def parse_audio_streams(blob):
    """-> list[StreamAlt] for every isochronous audio-streaming alternate setting."""
    out, cur_iface, cur, pending = [], None, None, None
    i, n = 0, len(blob)
    while i + 2 <= n:
        ln, dt = blob[i], blob[i + 1]
        if ln < 2 or i + ln > n:
            break
        d = blob[i:i + ln]
        if dt == DT_INTERFACE and ln >= 9:
            num, alt, neps, cls, sub = d[2], d[3], d[4], d[5], d[6]
            if cls == CLASS_AUDIO and sub == SUBCLASS_STREAMING:
                cur = StreamAlt()
                cur.iface, cur.alt = num, alt
                pending = cur if neps > 0 else None
            else:
                cur = pending = None
        elif dt == DT_CS_INTERFACE and cur is not None and ln >= 3:
            sub = d[2]
            if sub == SUB_FORMAT_TYPE and ln >= 8 and d[3] == 1:       # FORMAT_TYPE_I (UAC1)
                cur.channels, cur.subframe, cur.bits = d[4], d[5], d[6]
                nfreq = d[7]
                if nfreq == 0 and ln >= 14:                             # continuous range
                    cur.rates = [d[8] | d[9] << 8 | d[10] << 16, d[11] | d[12] << 8 | d[13] << 16]
                else:
                    cur.rates = [d[8 + 3 * k] | d[9 + 3 * k] << 8 | d[10 + 3 * k] << 16
                                 for k in range(nfreq) if 10 + 3 * k < ln]
        elif dt == DT_ENDPOINT and pending is not None and ln >= 7:
            attrs = d[3]
            if attrs & 3 == 1:                                          # isochronous
                pending.ep = d[2]
                pending.max_packet = d[4] | d[5] << 8
                pending.sync = SYNC_NAMES[(attrs >> 2) & 3]
                pending.interval = d[6]
                out.append(pending)
                pending = None
        i += ln
    return out


def pick_capture(streams, prefer_rate=44100):
    """Choose the capture (IN) alt setting to use: PCM, preferring `prefer_rate`, then the
    smallest frame size that still has that rate. Returns None if there is no capture."""
    cands = [s for s in streams if s.direction == "in" and s.channels and s.rates]
    if not cands:
        return None
    ok = [s for s in cands if prefer_rate in s.rates] or cands
    return sorted(ok, key=lambda s: (s.frame_bytes, s.alt))[0]


def read_descriptors(sysdir):
    with open(os.path.join(sysdir, "descriptors"), "rb") as f:
        return f.read()
