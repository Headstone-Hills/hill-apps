"""Numeric verification of captured PCM (Claude can't listen; this can).

analyze_pcm() takes raw little-endian signed PCM and reports peak/RMS in dBFS, silence,
whether left and right are identical (=> safe to downmix by keeping one channel), DC
offset and whether the byte count is frame-aligned.
"""

from __future__ import annotations

import math
import struct
import wave


def _to_int32(raw, subframe):
    """Raw LE PCM -> array('i') with every sample left-aligned in 32 bits (value << shift).
    Pure slice-assignment + array.frombytes: runs at C speed (the SP has no numpy)."""
    import sys
    from array import array
    if subframe not in (2, 3, 4):
        raise ValueError("unsupported subframe size %d" % subframe)
    n = len(raw) // subframe
    out = bytearray(4 * n)
    for b in range(subframe):
        out[(4 - subframe) + b::4] = raw[b:n * subframe:subframe]
    a = array("i")
    a.frombytes(bytes(out))
    if sys.byteorder == "big":
        a.byteswap()
    return a, 8 * (4 - subframe)


def _db(x, full):
    return -999.0 if x <= 0 else 20 * math.log10(x / full)


def analyze_pcm(raw, channels=2, subframe=3, rate=44100, silence_db=-70.0, max_stat_samples=200000):
    """Exact peak and L/R difference over every sample; RMS/DC/silence from an evenly strided
    subset of at most `max_stat_samples` per channel (plenty for a level check)."""
    import operator
    frame = channels * subframe
    full = float(1 << (subframe * 8 - 1))
    r = {"bytes": len(raw), "misaligned_bytes": len(raw) % frame, "channels": channels,
         "subframe": subframe, "rate": rate}
    raw = raw[:len(raw) - len(raw) % frame]
    frames = len(raw) // frame
    r["frames"] = frames
    r["seconds"] = frames / float(rate) if rate else 0.0
    if frames == 0:
        r.update(peak_db=[-999.0] * channels, rms_db=[-999.0] * channels, dc=[0.0] * channels,
                 silent=True, lr_identical=None, lr_max_diff=None, silent_fraction=1.0)
        return r
    arr, shift = _to_int32(raw, subframe)
    f32 = float(1 << 31)
    chans = [arr[c::channels] for c in range(channels)]
    r["peak_db"] = [_db(max(max(ch), -min(ch)), f32) for ch in chans]
    step = max(1, frames // max_stat_samples)
    rms, dc = [], []
    for ch in chans:
        sub = ch[::step]
        rms.append(_db(math.sqrt(math.fsum(float(v) * v for v in sub) / len(sub)), f32))
        dc.append(math.fsum(sub) / len(sub) / f32)
    r["rms_db"], r["dc"] = rms, dc
    thr = f32 * 10 ** (silence_db / 20.0)
    sub0 = chans[0][::step]
    r["silent_fraction"] = sum(1 for v in sub0 if abs(v) < thr) / float(len(sub0))
    r["silent"] = max(r["peak_db"]) < silence_db
    if channels >= 2:
        diff = max(map(abs, map(operator.sub, chans[0], chans[1]))) >> shift      # native LSBs
        r["lr_max_diff"] = diff
        r["lr_identical"] = diff == 0
        r["lr_near_identical"] = diff <= full * 10 ** (-60 / 20.0)
    else:
        r["lr_identical"] = r["lr_max_diff"] = None
    return r


def format_report(r):
    lines = ["captured %.2f s  (%d frames, %d bytes%s)" % (
        r["seconds"], r["frames"], r["bytes"],
        ", %d stray bytes" % r["misaligned_bytes"] if r["misaligned_bytes"] else "")]
    for c in range(r["channels"]):
        lines.append("  ch%d  peak %7.1f dBFS   rms %7.1f dBFS   dc %+.5f" %
                     (c, r["peak_db"][c], r["rms_db"][c], r["dc"][c]))
    lines.append("  silent frames: %.1f%%" % (100 * r["silent_fraction"]))
    if r["lr_identical"] is not None:
        lines.append("  left/right identical: %s (max diff %s)%s" % (
            r["lr_identical"], r["lr_max_diff"],
            "  -> mono downmix is lossless" if r["lr_identical"] else
            ("  -> near-identical, mono downmix OK" if r.get("lr_near_identical") else
             "  -> channels DIFFER, keep stereo or check")))
    lines.append("  verdict: " + ("SILENT (no signal above %s)" % "-70 dBFS" if r["silent"]
                                  else "SIGNAL PRESENT"))
    return "\n".join(lines)


def write_wav(path, raw, channels=2, subframe=3, rate=44100):
    with wave.open(path, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(subframe)
        w.setframerate(rate)
        w.writeframes(raw[:len(raw) - len(raw) % (channels * subframe)])


def read_wav(path):
    with wave.open(path, "rb") as w:
        return w.readframes(w.getnframes()), w.getnchannels(), w.getsampwidth(), w.getframerate()


def synth_tone(seconds=1.0, freq=440.0, channels=2, subframe=3, rate=44100, amp=0.5):
    """Test signal as raw PCM (used by --mock capture and the tests)."""
    full = (1 << (subframe * 8 - 1)) - 1
    out = bytearray()
    for i in range(int(seconds * rate)):
        v = int(amp * full * math.sin(2 * math.pi * freq * i / rate))
        b = v.to_bytes(subframe, "little", signed=True)
        out += b * channels
    return bytes(out)
