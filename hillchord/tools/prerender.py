#!/usr/bin/env python3
"""Pre-render every instrument's notes to the on-disk render cache.

After this runs, playing is just a fast disk load (no DSP), so there's zero
note-on lag — even the first time. Trades storage for compute, which suits a
device with lots of storage and little RAM.

Run it where the samples live and it writes config.RENDER_CACHE_DIR. The cache
keys are path-independent, so you can run it on the Mac (against a copy of the
device's Samples) and copy the cache over, OR run it on the device via SSH.

Usage:
  python tools/prerender.py            # common fx set (dry + reverb/chorus)
  python tools/prerender.py --full     # every reverb/chorus/wet combination
  python tools/prerender.py --dry      # dry only (smallest, fastest)
  python tools/prerender.py --only PAT # only instruments whose path contains PAT
                                        # (repeatable; case-insensitive)
  python tools/prerender.py --fav      # only sounds you've favorited (Y in app)
  python tools/prerender.py --all      # include one-shot single samples too

By default only multisamples and loops are pre-baked; one-shot single samples
(drum hits, SFX, pads) bake lazily on first play.

  e.g.  python tools/prerender.py --dry          # all multisamples + loops, dry
        python tools/prerender.py --dry --fav    # just your favorites, dry
"""

import os
import sys

os.environ.setdefault("SDL_VIDEODRIVER", "dummy")
os.environ.setdefault("SDL_AUDIODRIVER", "dummy")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import pygame

import config
from state import EffectsState
from audio.mixer import Mixer
from ui.library_screen import (_group_files, _is_single_multisample,
                               _wavs_recursive)

LO, HI = 36, 84            # MIDI range to warm (C2..C6)


def walk_sounds(root, d=None):
    """Enumerate every selectable sound as (kind, payload, sid), mirroring the
    library browser's grouping."""
    d = d or root
    res = []
    rel = os.path.relpath(d, root)
    if d != root and _is_single_multisample(d):
        return [("multi", _wavs_recursive(d), rel)]
    for stem, items in _group_files(d).items():
        paths = [p for _m, p in items]
        pitches = {m for m, _ in items if m is not None}
        if len(items) >= 2 and len(pitches) >= 2:
            res.append(("multi", paths, f"{rel}::{stem}"))
        else:
            for _m, p in items:
                res.append(("single", p, os.path.relpath(p, root)))
    for name in sorted(os.listdir(d)):
        full = os.path.join(d, name)
        if os.path.isdir(full):
            if _is_single_multisample(full):
                res.append(("multi", _wavs_recursive(full),
                            os.path.relpath(full, root)))
            else:
                res += walk_sounds(root, full)
    return res


def fx_variants(mode):
    out = [EffectsState()]                      # dry
    if mode == "dry":
        return out
    combos = ([(1, 0, 50), (1, 0, 100), (0, 1, 100), (1, 1, 100)] if mode == "common"
              else [(r, c, w) for r in (0, 1) for c in (0, 1)
                    for w in config.WETDRY_STEPS[1:] if (r or c)])
    for r, c, w in combos:
        out.append(EffectsState(reverb=bool(r), chorus=bool(c), wetdry=w))
    return out


def main():
    mode = "common"
    if "--full" in sys.argv:
        mode = "full"
    elif "--dry" in sys.argv:
        mode = "dry"

    # Collect --only PATTERN filters (repeatable).
    only = [sys.argv[i + 1].lower() for i, a in enumerate(sys.argv)
            if a == "--only" and i + 1 < len(sys.argv)]

    pygame.mixer.init(config.SAMPLE_RATE, -16, 2, 1024)

    # Scan per top-level pack with progress (the scan itself is slow on exFAT
    # for big libraries, so show that it's working).
    print(f"scanning {config.SAMPLE_PATH} ...", flush=True)
    sounds = []
    root = config.SAMPLE_PATH
    for name in sorted(os.listdir(root)):
        p = os.path.join(root, name)
        if os.path.isdir(p):
            print(f"  scanning {name} ...", flush=True)
            sounds += walk_sounds(root, p)
        elif p.lower().endswith(".wav"):
            sounds.append(("single", p, os.path.relpath(p, root)))
    print(f"found {len(sounds)} instruments.", flush=True)

    # Default: only multisamples + loops (the playable/heavy sounds). One-shot
    # single samples (drum hits, SFX, pads) bake lazily on first play. --all
    # overrides to pre-bake everything.
    if "--all" not in sys.argv:
        sounds = [s for s in sounds
                  if s[0] == "multi" or "loop" in s[2].lower()]
        print(f"multisamples + loops: {len(sounds)} (use --all for everything)",
              flush=True)
    print(flush=True)

    if "--fav" in sys.argv:                 # only sounds you've favorited
        favs = set()
        try:
            import json
            favs = set(json.load(open(config.STATE_PATH)).get("favorites", []))
        except Exception as e:
            print(f"could not read favorites: {e}")
        sounds = [s for s in sounds if s[2] in favs]
        print(f"favorites filter: {len(sounds)} of {len(favs)} favorites found.")
    if only:
        sounds = [s for s in sounds if any(p in s[2].lower() for p in only)]
    if not sounds:
        print("no instruments to prerender (check --only / --fav).")
        return
    variants = fx_variants(mode)
    print(f"source : {config.SAMPLE_PATH}", flush=True)
    print(f"cache  : {config.RENDER_CACHE_DIR}", flush=True)
    print(f"{len(sounds)} instruments x {len(variants)} fx x {HI - LO} notes "
          f"x 2 loop-modes\n", flush=True)

    mx = Mixer()
    for n, (kind, payload, sid) in enumerate(sounds, 1):
        print(f"  [{n}/{len(sounds)}] {sid} ...", flush=True)   # before -> live
        if kind == "multi":
            mx.load_files(payload, token=sid)
        else:
            mx.load_single(payload, token=sid)
        for fx in variants:
            # Both loop variants: True covers held chords/drones, False covers
            # arpeggiated/one-shot notes -> chords AND arps are pre-baked.
            for lp in (True, False):
                mx.prerender(range(LO, HI), fx, config.BPM_DEFAULT,
                             loop=lp, to_disk_only=True)
    total = sum(len(f) for _d, _s, f in os.walk(config.RENDER_CACHE_DIR)) \
        if os.path.isdir(config.RENDER_CACHE_DIR) else 0
    print(f"done. {total} cached renders.")


if __name__ == "__main__":
    main()
