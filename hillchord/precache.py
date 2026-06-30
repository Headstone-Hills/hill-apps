"""Background pre-render of all library sounds (dry, no effects) to the disk
cache.  After this runs once, every sound loads instantly — no DSP on first
play.  Runs as a low-priority daemon thread; safe to cancel at any time.

Usage (from main.py):
    stop_event = threading.Event()
    precache.start(stop_event, bpm=state.bpm)
    ...
    stop_event.set()   # on app exit
"""
from __future__ import annotations

import os
import threading

import config


# Full piano-range MIDI notes to pre-render.  Covers every note on a standard
# keyboard; rendering outside a sound's sampled range costs nothing extra (it
# just pitch-shifts the nearest sample).
_MIDI_LO = 21    # A0
_MIDI_HI = 109   # C8 exclusive  (range(21, 109))


def _enumerate_sounds(root: str):
    """Yield (kind, payload) for every selectable sound under *root*.

    kind="multi"  payload=list[str]   — multisample (list of WAV paths)
    kind="single" payload=str         — single WAV to pitch-shift
    """
    from ui.library_screen import (
        _is_single_multisample, _wavs_recursive,
        _group_files, _is_loopy,
    )
    for dirpath, dirs, _files in os.walk(root):
        # Sub-folders that are a single multisample instrument.
        for d in sorted(dirs):
            full = os.path.join(dirpath, d)
            if _is_single_multisample(full):
                yield ("multi", _wavs_recursive(full))
        # Don't walk into single-multisample folders (already yielded above).
        dirs[:] = sorted(
            d for d in dirs
            if not _is_single_multisample(os.path.join(dirpath, d))
        )
        # Loose WAV files grouped by instrument stem.
        loopy = _is_loopy(dirpath)
        for _stem, items in _group_files(dirpath).items():
            paths = [p for _m, p in items]
            pitches = {m for m, _ in items if m is not None}
            if not loopy and len(items) >= 2 and len(pitches) >= 2:
                yield ("multi", paths)
            else:
                for _m, p in items:
                    yield ("single", p)


def _needs_work(mixer, bpm: int, midis) -> bool:
    """True if at least one midi note in *midis* is missing from the disk cache."""
    from state import EffectsState
    fx = EffectsState()      # dry — effects are off by default
    loop = mixer._loop_mode
    for midi in midis:
        cid = mixer._cache_id(midi, bpm, loop)
        if cid is None:
            continue
        fxkey = mixer._fx_key(fx, loop)
        path = mixer._disk_path(cid, fxkey, loop)
        if not os.path.exists(path):
            return True
    return False


def _worker(stop: threading.Event, bpm: int):
    from audio.mixer import Mixer
    from state import EffectsState

    root = config.SAMPLE_PATH
    dry_fx = EffectsState()
    midis = list(range(_MIDI_LO, _MIDI_HI))

    total = 0
    cached = 0

    for kind, payload in _enumerate_sounds(root):
        if stop.is_set():
            return

        mx = Mixer()
        try:
            if kind == "multi":
                mx.load_files(payload)
            else:
                mx.load_single(payload)
        except Exception as e:
            print(f"[precache] load failed: {e}")
            continue

        if not _needs_work(mx, bpm, midis):
            cached += 1
            total += 1
            continue

        def _cancelled():
            return stop.is_set()

        try:
            mx.prerender(midis, dry_fx, bpm,
                         cancel=_cancelled, to_disk_only=True)
        except Exception as e:
            print(f"[precache] render error: {e}")

        total += 1
        if stop.is_set():
            return

    print(f"[precache] done — {total} sounds, {cached} already cached")


def start(stop_event: threading.Event, bpm: int = config.BPM_DEFAULT):
    """Start the background precache worker.  Call stop_event.set() to cancel."""
    t = threading.Thread(target=_worker, args=(stop_event, bpm),
                         name="hc-precache", daemon=True)
    t.start()
    return t
