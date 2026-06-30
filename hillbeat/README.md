# HillBeat

A **4-voice × 16-step × 8-pattern** drum machine for the Anbernic RG35XXSP running muOS. Plays WAV samples through pygame's mixer, with swing, per-step velocity, pattern chaining, and a loop player for backing audio.

---

## Quick orientation for AI assistants

**Entry point:** `main.py` — owns the pygame event loop, all button handling, and the overlay state machine.  
**Reference copy (Mac):** `~/sadiehouse/Projects/hillbeat/`  
**Live copy (device SD):** `/mnt/sdcard/ports/hillbeat/` (mounted at `/Volumes/Untitled/ports/hillbeat/` on the Mac).

---

## File map

| File | Role |
|---|---|
| `main.py` | Event loop, button handling, screen routing, quit logic |
| `constants.py` | All magic numbers: display, sequencer, mixer, paths, button indices |
| `state.py` | `AppState` — patterns (8×4×16), voices, transport, favorites, JSON I/O |
| `sequencer.py` | Drift-correcting playback thread; calls `load_voice` / plays steps |
| `library.py` | Sample browser overlay — scans `SAMPLE_ROOT`, deduplicates variants, 2-level dirs |
| `ui.py` | Main grid renderer, status bar, voice labels |
| `transport.py` | BPM, swing, tap-tempo, pattern queue logic |
| `help_overlay.py` | Cheat-sheet overlay drawn over the grid |
| `chain_editor.py` | Pattern chain editor overlay |
| `loop_player.py` | Background loop audio (long WAV played under the beat) |
| `samples/` | WAV sample library (organized in subdirectory trees) |

---

## Hardware — RG35XXSP button indices

These are the raw joystick button numbers used in `constants.py`. They're the canonical source for all hardware I/O.

| Logical | Index | Notes |
|---|---|---|
| A | 3 | |
| B | 4 | |
| Y | 5 | |
| X | 6 | |
| L1 | 7 | |
| R1 | 8 | |
| SELECT | 9 | |
| START | 10 | |
| FUNCTION | 11 | |
| L2 | 12 | |
| R2 | 13 | |
| D-pad | hat axis | `HAT_LEFT/RIGHT/UP/DOWN` constants |

---

## Control scheme

### Playback
| Input | Action |
|---|---|
| START | Play / pause |
| SELECT | Tap tempo |
| START+SELECT (tap) | Stop & reset to bar 1 |
| START+SELECT (hold 2s) | Quit HillBeat |
| L1 / R1 | Previous / next pattern |
| L2 / R2 | BPM – / + |

### Step editing
| Input | Action |
|---|---|
| D-pad | Move cursor (L/R = step, U/D = voice) |
| A (tap) | Toggle step on/off |
| A (hold) + U/D | Adjust step velocity |
| Y | Mute / unmute voice |
| Y (hold) + U/D | Adjust voice volume |
| B | Select voice (for library) |

### Overlays
| Input | Action |
|---|---|
| START+B | Open sample library |
| A or B | Close cheat sheet |
| FUNCTION+SELECT | Cheat sheet (help) — either order |
| START+A | Loop player |
| START+X | Chain editor |
| START+Y | Save state |
| START+L1/R1 | Copy / paste pattern |
| SELECT+L1 | Clear pattern |

---

## Sample library

**Root:** `constants.SAMPLE_ROOT` — defaults to `samples/` inside the app dir, relative so the app works anywhere.

**Structure:**
```
samples/
  Cassette Drums/
    Hat/      hat_*.wav
    Kick/     kick_*.wav
    Snare/    snare_*.wav
    Perc/     perc_*.wav
  OtherKit/
    *.wav
```

**2-level directory support:** If a folder contains subdirectories, the library shows `"Parent  ›  Child"` section headers. Flat folders get a plain section header.

**Variant deduplication:** Multiple round-robin / velocity-layer variants of the same hit (e.g. `snare_x_l1_rr1.wav`, `snare_x_l2_rr3.wav`) are collapsed into one entry by `_base_stem()` using `_VARIANT_TRAIL` regex, which strips `_rr\d+`, `_l\d+`, `_loud\d+`, `_[a-g]\d+`, dynamic markings (`_pp`, `_mf`, etc.) from filenames before deduplication. The first alphabetical representative is stored and loaded.

**Normalization:** All 492 WAV files under `Cassette Drums/` are peak-normalized to –3 dBFS (peak ≈ 23 200 / 32767) for consistent volume across kits. One corrupted file (`perc_noisebox_03_rattles_36.wav`) is skipped.

**Relevant functions in `library.py`:**
- `_base_stem(path)` — strips variant suffixes, returns the canonical instrument name
- `_build_wav_items()` — scans `SAMPLE_ROOT`, builds `(kind, label, path)` list with section headers
- `warmup(favorites)` — pre-scans without opening the overlay (called at startup)

---

## State persistence

Saved to `hillbeat_state.json` in the app directory. Contains: BPM, swing, current pattern, all 8 patterns (4×16 steps with velocity), voice names, voice sample paths (stored as paths relative to `SAMPLE_ROOT` when possible), voice mute/volume, favorites list, loop file.

`_find_sample(name)` performs a recursive search under `SAMPLE_ROOT` so samples in subdirectories are found even if the saved path is just a basename.

---

## Sequencer internals

`sequencer.py` runs a drift-correcting thread:
- Expected fire time = `start_time + step_count * step_interval + swing_offset`
- Sleeps until the exact fire time, then triggers all active steps
- BPM changes mid-play are handled by reanchoring `start_time`

Each voice maps to a pygame mixer channel (channels 0–3). The loop player uses channel 4.

---

## Known quirks / AI guidance

- `main.py` uses `nonlocal` extensively inside nested functions; always declare new mutable variables in `nonlocal` before assigning inside `handle_button_down` / `handle_button_up`.
- `show_help` guard must check close conditions (B, SELECT+X) **before** the blanket `return` so the overlay can be dismissed.
- The START+SELECT quit timer (`start_select_since`) is checked in the main loop, not in button handlers.
- macOS creates `._filename.wav` sidecar files alongside every WAV; any file scanner must skip filenames starting with `._`.
- `._` sidecars on the SD card cause no playback issues (pygame ignores them) but will confuse wave-header parsers.

---

## Development

```bash
# Mac dev — no device needed
python3 -m venv .venv && .venv/bin/pip install pygame numpy
.venv/bin/python main.py
```

Keyboard bindings are defined in the `KEYBOARD` dict at the top of `main.py` (arrow keys = D-pad, etc.).

## Deploy to RG35XXSP

Copy `ports/hillbeat/` to the device SD card at `/mnt/sdcard/ports/hillbeat/` and the launcher `ROMS/Ports/HillBeat.sh` to `/mnt/sdcard/ROMS/Ports/HillBeat.sh`. On first run the launcher script calls `install_deps.sh` to vendor pygame and numpy into `pylibs/`.
