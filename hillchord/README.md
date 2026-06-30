# HillChord

A **live chord/note instrument** for the Anbernic RG35XXSP running muOS. Loads multisampled instruments, plays chords and single notes with effects (reverb, chorus, delay, bitcrush), a loop recorder, and an arpeggiator. Notes are pitch-shifted in software from the nearest available sample using a disk+RAM render cache so first-play latency is near-zero after pre-caching.

---

## Quick orientation for AI assistants

**Entry point:** `main.py`  
**Reference copy (Mac):** `~/sadiehouse/Projects/hillchord/`  
**Live copy (device SD):** `/mnt/sdcard/ports/hillchord/` (Mac: `/Volumes/Untitled/ports/hillchord/`)  
**Sample library (shared):** `/mnt/sdcard/ROMs/Samples/` (Mac: `/Volumes/Untitled/ROMS/Samples/`)  
**Render cache (device):** `/mnt/sdcard/ROMs/.hillchord_cache/`

---

## File map

| File / Dir | Role |
|---|---|
| `main.py` | pygame init, state restore, main loop, precache wiring |
| `config.py` | All tunables: paths, mixer, BPM, effects, cache sizes, loop parameters |
| `state.py` | `AppState` — key, mode, BPM, effects, octave, sound, favorites |
| `persistence.py` | JSON save/restore of `AppState` |
| `actions.py` | Maps `InputFrame` → audio/state mutations (the core logic layer) |
| `effects.py` | Top-level effects entry point |
| `mixer.py` | Thin shim that re-exports `audio/mixer.py` at module root |
| `precache.py` | Background thread that pre-renders all instruments to disk |
| `play_screen.py` | Main HUD renderer |
| `help_overlay.py` | Cheat-sheet overlay |
| `audio/mixer.py` | **Core sampler engine** — pitch shift, time stretch, effects baking, cache |
| `audio/effects.py` | DSP: Freeverb reverb, chorus, beat-spaced delay, bitcrush, time-stretch |
| `audio/loop_recorder.py` | Loop recorder — event capture, baking, overdub, phase alignment |
| `audio/arpeggiator.py` | Step arpeggiator — fires per-note triggers at `ARP_DIVISION` rate |
| `audio/metronome.py` | Click track (dedicated channel) |
| `audio/transport.py` | Beat clock, grid quantization helpers |
| `input/button_map.py` | Button index → logical name mapping (joystick + keyboard) |
| `input/combo_handler.py` | Raw events → `InputFrame` (pressed, released, held, dpad) |
| `ui/library_screen.py` | Instrument browser — multisample grouping, favorites, filters |
| `ui/play_screen.py` | Play-screen renderer |
| `theory/` | Music theory helpers: chords, notes, scales |
| `tools/probe_input.py` | Dev utility to identify button indices on new hardware |

---

## Hardware — RG35XXSP button indices

Defined in `input/button_map.py` as `JOY_BUTTONS`. Shared with HillBeat and HillSequencer.

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

### Play mode (chord)
| Input | Action |
|---|---|
| A / B / X / Y | Play chord (degree depends on face button combo) |
| D-pad (while chord held) | Add mod (sus2, sus4, add9, etc.) |
| D-Up alone | Toggle major/minor |
| L2 | Loop recorder: IDLE→REC→LOOP→OVERDUB→LOOP |
| L2 hold 2s | Cancel / clear loop |

### Play mode (note)
| Input | Action |
|---|---|
| A / B / X / Y | Play single note (scale degree) |
| Hold button | Sustain; release = stop |
| A+B held → any | Octave down |
| A+X held → any | Octave up |

### Global controls
| Input | Action |
|---|---|
| R1 | Next sound / instrument |
| L1 | Previous sound / instrument |
| D-Left / D-Right | BPM – / + |
| R2 | Cycle wet/dry (0/25/50/75/100%) |
| SELECT+L2/R2 | Chorus on/off / Delay on/off |
| FUNCTION+A/B/X | Reverb / delay / chorus toggle |
| FUNCTION+Y | Wet/dry |
| FUNCTION+R1/L1 | Bitcrush bits / downsample |
| START+SELECT (tap) | Stop & reset |
| START+SELECT (hold 1.2s) | Quit |

### Overlays
| Input | Action |
|---|---|
| START | Library overlay |
| FUNCTION+SELECT | Help (cheat sheet) — either order |
| B or A | Close help overlay |

---

## Sampler engine (`audio/mixer.py`)

**Loading:** `load_sound(folder)` or `load_files(files)` or `load_single(wav_path)`.  
For a multisample folder, `SoundBank` maps MIDI numbers to WAV files; any requested pitch is filled by pitch-shifting the nearest available sample.

**Rendering pipeline** (`_render(midi, fx, bpm, loop)`):
1. Check RAM LRU cache (keyed by `(midi, bpm_quantized, loop, fxkey)`)
2. Check disk cache (`RENDER_CACHE_DIR/{shard}/{hash}.wav`)
3. Compute: source PCM → optional tempo-match → optional loop crossfade → effects baking → write disk → store RAM

**Effects baking** (all offline, zero runtime cost):
- Reverb: Freeverb (`ROOM_SIZE`, `DAMPING`, `WET`)
- Chorus: comb filter with LFO
- Beat-spaced delay: echos at beat multiples
- Bitcrush: bit depth reduction + sample-rate downsampling

**Sustain detection** (`_is_sustaining`): majority-vote over 8 windows — if a sound's energy stays high past the middle it's classified as a looping drone (`_loop_mode=True`), otherwise one-shot. Controls whether `play_voice` loops the sound or plays it once.

**`_loop_mode`:** Set at load time. When True, `play_voice` plays the sound with `loops=-1`; when False, it plays once. This affects the RAM/disk key so looped and one-shot renderings are cached separately.

**Normalization:** Each loaded sound is RMS-normalized to `NORM_TARGET_RMS` (~–18 dBFS) before caching.

---

## Loop recorder (`audio/loop_recorder.py`)

**States:** `IDLE → RECORDING → LOOPING → OVERDUB → LOOPING`; hold L2 2s → `IDLE` from any state.

**Loop start:** Snapped to the nearest beat (grid_nearest), not the previous beat, so the downbeat is tight regardless of which side of the beat L2 is pressed.

**Event recording:** Notes are captured via `on_trigger(midis, fx, bpm, loop, key=)` and matched by `on_release(key)`. The looper stores the actual hold duration so staccato notes are baked at their real length instead of as infinite drones. Notes with no key (arpeggiator) are committed immediately as one-shots.

**Baking (`_bake`):** All events are rendered to a single float32 stereo buffer. Each note's PCM is truncated to its hold duration and faded out at the boundary. The buffer is stored as `_base_buf` (int16, canonical), then phase-rolled to the current beat position for gapless playback.

**Overdub (`_merge_overdub`):** Always mixes into `_base_buf` (not the rolled playback copy) and anchors timing to `_t0` to prevent phase drift across multiple overdub passes.

**Retime:** On BPM change the base buffer is time-stretched and replayed; pitch is preserved.

**Callers must:**
1. Call `on_trigger(..., key=some_id)` when a voice starts
2. Call `on_release(key)` when that voice stops
3. Pass `key=None` for notes with no explicit release (arp, one-shots)

---

## Instrument library (`ui/library_screen.py`)

**Multisample grouping:**
- `_instr_stem(filename)` strips note names (`C2`, `Eb4`), MIDI number tags (`_midi036`), and trailing variant tokens to derive the instrument group name
- `_group_files(folder)` groups all WAVs by their `_instr_stem`
- `_is_single_multisample(folder)` returns True if the folder has exactly 1 group with ≥2 distinct pitches → shown as a loadable `sound_multi` entry
- Instruments are shown as single entries (not per-file); the sampler loads the whole group

**Sample filename conventions** (what `_instr_stem` handles):
- `HoveFlute_midi036_C2.wav` — explicit MIDI number + note name
- `Piano_Eb4.wav` — note name only  
- `Pad_120_Ab.wav` — bare pitch class

**Favorites:** Stored by sound-id (`sid`) in `AppState.favorites`. The library marks them with a star and can filter to favorites only.

---

## Pre-caching (`precache.py`)

Background daemon thread started in `main.py` after the initial sound is loaded. Walks the entire sample library (same logic as `library_screen`), calls `Mixer.prerender(to_disk_only=True)` for every instrument × full MIDI range (21–109). Uses a generation counter so a new library load cancels the in-flight pre-cache.

**Purpose:** Eliminates first-play lag for any note in any instrument after the device has been running for a few minutes. On first boot notes may still stutter; subsequent plays are instant.

---

## Platform / deploy

**Platform detection:** `config.ON_DEVICE = os.path.isdir("/mnt/sdcard/ROMs")` — when True, paths point to `/mnt/sdcard`; when False, paths are relative to the app directory (dev mode).

**SDL setup (launcher `HillChord.sh`):**
```bash
export SDL_AUDIODRIVER=alsa
export LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0
```

**Build & deploy:**
```bash
bash build_port.sh        # -> dist/HillChord.zip
bash deploy.sh [host]     # scp + unzip on device
```

**Dev on Mac:**
```bash
python3 -m venv venv && venv/bin/pip install pygame-ce numpy scipy
venv/bin/python main.py
```

---

## Known quirks / AI guidance

- `_instr_stem` must strip BOTH note-name tokens (`C2`) AND MIDI-number tags (`_midi036`) or `_group_files` will create one group per file and `_is_single_multisample` will return False for every instrument.
- The render cache key is a hash of `(token, channel_id, fxkey, loop)`. Changing the effects or loop flag produces a different cached file; old entries are never invalidated automatically.
- `_base_buf` vs `self._sound`: `_base_buf` is the canonical unrolled recording; `self._sound` is a phase-rolled playback copy. **Always mix overdubs into `_base_buf`**, never into `self._sound`, or phase drifts on each overdub pass.
- `_loop_mode` is set at load time (majority vote). There is no per-note override; the flag applies to all notes played with that instrument.
- The arpeggiator always calls `on_trigger(loop=False, key=None)` — no duration tracking needed because arp notes are one-shots by design.
- `combo_handler.py` delivers an `InputFrame` with `.pressed`, `.released`, `.held`, `.dpad` sets per frame. Actions consumes these; it never sees raw pygame events.
