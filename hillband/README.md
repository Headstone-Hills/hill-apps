# HillBand

A **16-step × 8-track × 8-pattern** hybrid sequencer for the Anbernic RG35XXSP running muOS.  
The 8 tracks are split into two functional halves:

| Tracks | Role | Sound |
|---|---|---|
| 0–3 (top / melodic) | Pitched instruments | Any sound in the shared library; chord type + scale cycling |
| 4–7 (bottom / drums) | KICK / SNARE / HAT / PERC | Percussion library only; one-shot, no pitch-shift |

Chord, scale, velocity, swing, tap tempo, pattern chaining, sequence save/load, loop player, and baked effects (melodic tracks) are all supported.

---

## Quick orientation for AI assistants

**Entry point:** `main.py` — owns the pygame event loop, all button handling, and the overlay state machine.  
**Reference copy (Mac):** `~/sadiehouse/Projects/hillband/`  
**Live copy (device SD):** `/mnt/sdcard/ports/hillband/` (Mac: `/Volumes/Untitled/ports/hillband/`)  
**Sample library (shared):** `/mnt/sdcard/ROMs/Samples/` — same folder as HillChord and HillSequencer; never duplicated.  
**Render cache (device):** `/mnt/sdcard/ROMs/.hillband_cache/`

---

## File map

| File | Role |
|---|---|
| `main.py` | Event loop, button handling, overlay routing |
| `config.py` | All tunables: paths, display, grid, transport, channel map, colours |
| `state.py` | `AppState` — 8 tracks, 8 patterns (8×16 steps + velocity), per-melodic-track chord/scale/effects, JSON I/O |
| `sequencer.py` | Drift-correcting playback thread; chord/scale-step logic per melodic track |
| `transport.py` | BPM/swing, pattern queue + chain, tap tempo, beat clock |
| `instruments.py` | `TrackRack` — 8 `Mixer` engines (one per track), prewarm, frame pump |
| `library_overlay.py` | Sound browser (pre-filtered to DRUM when opening for drum tracks) |
| `track_mode_overlay.py` | Per-track chord type + scale mode picker (Start+L2 on melodic tracks) |
| `ui.py` | 8-row grid renderer; melodic rows (blue) vs drum rows (orange) |
| `help_overlay.py` | Scrollable controls cheat-sheet |
| `chain_editor.py` | Pattern chain editor |
| `sequence_manager.py` | Named sequence save/load |
| `loop_player.py` | Background loop audio (BPM-stretched WAV) |
| `theory/notes.py` | `note_name(midi)` helper |
| `theory/chords.py` | `CHORD_TYPES` + `SCALE_MODES` definitions |
| `audio/mixer.py` | HillChord sampler engine (pitch-shift, effects baking, render cache) |
| `audio/effects.py` | DSP: Freeverb reverb, chorus, beat-spaced delay, bitcrush, time-stretch |
| `audio/metronome.py` | Click track |
| `input/button_map.py` | Button index → logical name (joystick + keyboard) |

---

## Hardware — RG35XXSP button indices

Identical to HillBeat, HillChord, and HillSequencer.

| Logical | Index |
|---|---|
| A | 3 |
| B | 4 |
| Y | 5 |
| X | 6 |
| L1 | 7 |
| R1 | 8 |
| SELECT | 9 |
| START | 10 |
| FUNCTION | 11 |
| L2 | 12 |
| R2 | 13 |
| D-pad | hat axis |

Keyboard dev bindings: `a/b/x/y`, `q/w/e/r` (L1/R1/L2/R2), `o/p/u` (SELECT/START/FN), arrow keys (D-pad).

---

## Control scheme

### Transport
| Input | Action |
|---|---|
| START | Play / pause |
| SELECT | Tap tempo |
| START+SELECT (tap) | Stop & reset to step 1 |
| START+SELECT (hold ~1.2s) | Quit |
| L1 / R1 | Previous / next pattern |
| L2 / R2 | BPM − / + (hold = repeat) |

### Grid editing
| Input | Action |
|---|---|
| D-pad | Move cursor (L/R = step, U/D = track) |
| A (tap) | Toggle step on/off |
| A (hold) + U/D | Adjust step velocity |
| Y (tap) | Mute / unmute track |
| Y (hold) + U/D | Adjust track volume |
| X (hold) + L/R | Transpose note ±1 semitone **(melodic tracks only)** |
| X (hold) + U/D | Transpose note ±1 octave **(melodic tracks only)** |

### Chord / scale (melodic tracks)
| Input | Action |
|---|---|
| START+L2 | Open chord/scale mode overlay for the current melodic track |
| In overlay — L / R | Cycle chord type (unison, power, major, minor, sus2, sus4, dom7, maj7, min7, dim, dim7, aug, add9, minadd9) |
| In overlay — U / D | Cycle scale mode (none, major, minor, penta_maj, penta_min, blues, dorian, mixolyd, phrygian, lydian, chromatic) |
| In overlay — A | Apply + close |
| In overlay — B | Close without applying |

When a scale mode is active, each *active* step of the track advances to the next scale degree instead of repeating the root.  The counter resets at each pattern boundary.

### Instruments
| Input | Action |
|---|---|
| START+B | Sound library (drum tracks: pre-filtered to DRUM; X clears the filter) |
| START+R2 | Toggle cursor track into the multi-select set |
| multi-select + START+B | Assign the chosen instrument to all selected tracks |

### Song / groove
| Input | Action |
|---|---|
| START+X | Chain editor |
| START+A | Loop player on/off |
| START+Y | Save state |
| START+L1 / R1 | Copy / paste pattern |
| SELECT+Y | Sequence manager (load saved sequences) |
| SELECT+X | Swing on/off |
| SELECT+L2 / R2 | Swing − / + |
| SELECT+A | Metronome toggle |
| SELECT+L1 | Clear current pattern |
| SELECT+R1 | Clear all instruments |
| FUNCTION+SELECT | Help (cheat sheet) — either order |

### Effects (melodic tracks only, Fn modifier)
| Input | Action |
|---|---|
| Fn+A | Reverb on/off |
| Fn+B | Delay on/off |
| Fn+X | Chorus on/off |
| Fn+Y | Wet/dry step (0 → 25 → 50 → 75 → 100%) |
| Fn+R1 / L1 | Bitcrush bits / downsample |

---

## Melodic track parameters

Each melodic track (0–3) stores:

| Field | Description |
|---|---|
| `note` | MIDI root note (C1–C7). Transpose with X+dpad. |
| `chord_type` | Chord built on the root per active step. `"unison"` = single note. |
| `scale_mode` | When not `"none"`, active steps cycle through scale degrees from the root. Resets at pattern boundary. |
| `effects` | `EffectsState`: reverb/delay/chorus (wet/dry) + bitcrush. Baked to disk cache; zero runtime DSP cost. |

Drum tracks (4–7) have none of the above.  Their `note` field is a snap target for the percussion sampler (the mixer uses the nearest pad sample without pitch-shifting).

---

## Sample library

**Root:** `SAMPLE_PATH` → device: `/mnt/sdcard/ROMs/Samples`, dev: `../hillchord/samples`.

Identical multisample grouping logic as HillSequencer and HillChord:
- `_instr_stem(filename)` strips note/MIDI tags to derive the instrument group name.
- Folders with one group ≥2 pitches → loadable multisample.
- macOS `._filename.wav` sidecars are skipped everywhere.

**Drum filter:** When opening the library for a drum track, `Library.open()` receives `drum_only=True` and pre-sets the filter to `"DRUM"` (keyword-matched: kick, snare, hat, perc, cymbal, clap, tom, drum, kit). The user can clear the filter with X to browse any sound.

---

## State persistence

Saved to `hillband_state.json` in the app directory.  Contains BPM, swing, current pattern, all 8 patterns (8 tracks × 16 steps with velocity), per-track sound/note/mute/volume, per-melodic-track chord_type/scale_mode/effects, favorites, loop file, and chain config.

Named sequences are saved as separate JSON files under `sequences/`.

---

## Architecture

Channel allocation:

```
channels  0– 3 : Track 0  (melodic — up to 4 simultaneous chord notes)
channels  4– 7 : Track 1
channels  8–11 : Track 2
channels 12–15 : Track 3
channels 16–19 : Track 4  (drum — one channel used at a time)
channels 20–23 : Track 5
channels 24–27 : Track 6
channels 28–31 : Track 7
channel     32 : Metronome
channel     33 : Loop player
```

(`CHANNELS_PER_TRACK = 4`, uniform across all 8 tracks.)

---

## Platform / deploy

**Platform detection:** `config.ON_DEVICE = os.path.isdir("/mnt/sdcard/ROMs")`

**Build & deploy:**
```bash
bash build_port.sh          # -> dist/HillBand.zip
bash deploy.sh [host]       # scp + unzip on device (default: 192.168.1.201)
```

**Dev on Mac:**
```bash
python3 -m venv .venv && .venv/bin/pip install pygame-ce numpy scipy
.venv/bin/python main.py
# Samples default to ../hillchord/samples; override with HILLBAND_SAMPLES env var.
```

---

## Relationship to other Hill* apps

| App | Role |
|---|---|
| HillBeat | 4-voice drum machine with raw WAV playback |
| HillChord | Live chord/note instrument with looper and arpeggiator |
| HillSequencer | 8-track pitched sample sequencer (HillChord instruments) |
| **HillBand** | **Hybrid: 4 melodic tracks (HillSequencer engine) + 4 drum tracks (HillBeat philosophy, HillChord sampler)** |

All four share:
- The same RG35XXSP button index map (`JOY_BUTTONS`)
- The same `ROMs/Samples/` instrument library (HillChord + HillSequencer + HillBand)
- The same `audio/mixer.py` sampler engine codebase

---

## Known quirks / AI guidance

- `main.py` uses `nonlocal` extensively in nested functions; declare new mutable variables `nonlocal` before assigning inside `handle_button_down` / `handle_button_up`.
- `state.is_drum_track(ti)` is the canonical check — use it instead of `ti >= DRUM_TRACK_OFFSET` so the boundary is in one place.
- Drum tracks pass `state.no_fx()` (an `EffectsState()` with all defaults) to `play_step` and `prewarm_note`; never access `track["effects"]` on a drum track.
- Scale counters (`sequencer._scale_pos`) reset to zero at pattern boundary (when `next_step == 0`), not at play/stop, so looping is deterministic.
- macOS `._filename.wav` sidecars must be skipped by any file scanner (`not f.startswith("._")`).
- `ON_DEVICE` is False on Mac even with the SD card mounted at `/Volumes/Untitled` — intentional, keeps dev paths local.
