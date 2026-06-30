# HillSequencer

An **8-track × 16-step × 8-pattern** grid sequencer that plays HillChord's pitched sampler engine. Each track holds one multisample instrument tuned to one note; the step grid fires it with per-step velocity, per-track volume/mute, and baked effects. HillBeat's sequencing workflow (patterns, chaining, swing, tap tempo, copy/paste) is fused with HillChord's DSP layer (pitch shift, time stretch, reverb/chorus/delay/bitcrush, disk render cache).

Target device: **Anbernic RG35XXSP / muOS**. Also runs on Mac for development.

---

## Quick orientation for AI assistants

**Entry point:** `main.py`  
**Reference copy (Mac):** `~/sadiehouse/Projects/hillsequencer/`  
**Live copy (device SD):** `/mnt/sdcard/ports/hillsequencer/` (Mac: `/Volumes/Untitled/ports/hillsequencer/`)  
**Sample library (shared with HillChord):** `/mnt/sdcard/ROMs/Samples/` (Mac: `/Volumes/Untitled/ROMS/Samples/`)  
**Render cache (device):** `/mnt/sdcard/ROMs/.hillsequencer_cache/`

---

## File map

| File / Dir | Role |
|---|---|
| `main.py` | Event loop, all control handling, overlay routing, wiring |
| `config.py` | All tunables: paths, display, grid, transport, channel map, colours |
| `state.py` | `AppState` — tracks, patterns (8×8×16), per-track `EffectsState`, JSON I/O |
| `transport.py` | BPM/swing, pattern queue + chain, tap tempo, beat clock |
| `sequencer.py` | Drift-correcting playback thread (`start` / `stop` / `toggle`) |
| `instruments.py` | `TrackRack` — 8 `Mixer` engines, `load_by_sid`, `prewarm`, echo pump |
| `library_overlay.py` | Per-track instrument browser (multisample grouping, favorites) |
| `sequence_manager.py` | Named sequence save/load overlay |
| `chain_editor.py` | Pattern chain editor |
| `loop_player.py` | Background audio loop player |
| `key_overlay.py` | Key/scale selector; spreads selected scale across all 8 tracks |
| `help_overlay.py` | Cheat-sheet overlay |
| `ui.py` | 8-row grid renderer, status bar, track labels, badges |
| `audio/mixer.py` | HillChord sampler engine (identical codebase, channel-banded) |
| `audio/effects.py` | DSP: reverb, chorus, delay, bitcrush, time-stretch |
| `audio/metronome.py` | Click track |
| `input/button_map.py` | Button index → logical name (joystick + keyboard) |
| `theory/` | Music theory helpers shared with HillChord |
| `sequences/` | Saved sequence JSON files |

---

## Architecture

Each track owns a dedicated `audio.mixer.Mixer` instance, confined to a private band of pygame channels:
```
channels  0– 7 : Track 0 (8 voices)
channels  8–15 : Track 1
...
channels 56–63 : Track 7
channel     64 : Metronome
channel     65 : Loop player
```
(`CHANNELS_PER_TRACK = 8`, configured in `config.py`.)

Because a track plays only one note at a time, the engine renders `(note, fx)` once (cached to disk), then replays the cached Sound per step scaled by `velocity × track_volume`. The sequencer thread fires steps; the main loop calls `instruments.update()` each frame to drain delay-echo queues.

---

## Hardware — RG35XXSP button indices

Defined in `input/button_map.py`. Identical to HillBeat and HillChord.

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

---

## Control scheme

### Transport
| Input | Action |
|---|---|
| START | Play / pause |
| SELECT | Tap tempo |
| START+SELECT (tap) | Stop & reset to step 1 |
| START+SELECT (hold 2s) | Quit |
| L1 / R1 | Previous / next pattern |
| L2 / R2 | BPM – / + |

### Grid editing
| Input | Action |
|---|---|
| D-pad | Move cursor (L/R = step, U/D = track) |
| A (tap) | Toggle step on/off |
| A (hold) + U/D | Adjust step velocity |
| Y (tap) | Mute / unmute track |
| Y (hold) + U/D | Adjust track volume |
| X (hold) + L/R | Transpose track note ±1 semitone |
| X (hold) + U/D | Transpose track note ±1 octave |

### Overlays
| Input | Action |
|---|---|
| START+B | Instrument library (current track) |
| START+R2 | Multi-select tracks (then START+B assigns to all selected) |
| START+L2 | Key/scale selector (spreads scale across all 8 tracks) |
| START+X | Chain editor |
| START+Y | Save sequence |
| SELECT+Y | Sequence manager (load saved sequences) |
| START+A | Loop player |
| START+L1/R1 | Copy / paste pattern |
| SELECT+L1 | Clear pattern |
| SELECT+L2 | Clear all instruments |
| SELECT+X | Swing on/off |
| SELECT+L2/R2 | Swing – / + |
| SELECT+A | Metronome on/off |
| FUNCTION+SELECT | Help (cheat sheet) — either order |

### Effects (per-track, under FUNCTION modifier)
| Input | Action |
|---|---|
| FN+A | Reverb on/off |
| FN+B | Delay on/off |
| FN+X | Chorus on/off |
| FN+Y | Wet/dry cycle |
| FN+R1/L1 | Bitcrush bits / downsample |

---

## Sampler engine (`audio/mixer.py`)

Identical to HillChord's sampler (shared codebase). Key behaviours:

**Pitch shifting:** Nearest sample in the `SoundBank` is repitched to the requested MIDI note using SciPy resampling. With `PITCH_PRESERVE_TEMPO=True`, pitch-shifting a loop time-stretches it to maintain its original tempo.

**Sustain fade:** Sounds longer than `MAX_SUSTAIN_BEATS = 16` beats are automatically faded out after 16 beats using a scheduled `pygame.mixer.Channel.fadeout(SUSTAIN_FADE_MS)` call. This prevents long drones from monopolizing channels in the sequencer context. Implemented in `play_step()` and drained in `update()`.

**Effects baking:** All effects (reverb, chorus, delay, bitcrush) are baked into the rendered PCM and cached to disk. Runtime playback cost is just a channel play + volume scale.

**Render cache:**
- Disk: `RENDER_CACHE_DIR/{2-hex shard}/{hash}.wav` — keyed by `(token, channel_id, fxkey, loop)`
- RAM: bounded LRU (`RAM_CACHE_BYTES`), holds decoded `pygame.Sound` objects

---

## Instrument library (`library_overlay.py`)

Mirrors HillChord's `ui/library_screen.py` with the same multisample grouping logic:

- `_instr_stem(filename)` strips note names, MIDI number tags (`_midi036`), and trailing variants to derive the instrument group name
- `_group_files(folder)` groups by stem; `_is_single_multisample` identifies loadable instruments
- Instruments are loaded as a group (all pitches); the engine pitch-shifts to the track's note

**Critical:** `_MIDI_NUM_TAG = re.compile(r"[_\-]midi\d{2,3}", re.I)` must be applied in `_instr_stem` or the MIDI-number-tagged filenames produce one group per file and no multisamples are detected.

---

## State persistence

Saved to `hillsequencer_state.json`. Contains: BPM, swing, current pattern, 8 patterns (8 tracks × 16 steps with velocity), per-track name/note/mute/volume/instrument-sid/effects, loop file path, chain config.

Named sequences (full snapshots) are saved as separate JSON files in `sequences/`.

---

## Key/scale auto-assignment (`key_overlay.py`)

When the user selects a key and scale, the notes are spread across all 8 tracks: Track 1 gets the highest root, Track 8 gets the lowest. This gives a natural bass-to-treble arrangement without manual note editing.

---

## 16-beat sustain cap

Any sound whose natural length exceeds 16 beats at the current BPM is faded out after 16 beats. This prevents a single long sample (e.g., a 30-second drone) from filling channels indefinitely in the step sequencer context.

**Implementation in `audio/mixer.py` → `play_step()`:**
```python
beat_sec = 60.0 / max(bpm, 1)
max_sec  = MAX_SUSTAIN_BEATS * beat_sec   # 16 beats
if snd.get_length() > max_sec:
    # schedule a fadeout via self._fades list, drained in update()
```

---

## Platform / deploy

**Platform detection:** `config.ON_DEVICE = os.path.isdir("/mnt/sdcard/ROMs")`

**SDL setup (launcher `HillSequencer.sh`):**
```bash
export SDL_AUDIODRIVER=alsa
export LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0
```

**Build & deploy:**
```bash
bash build_port.sh          # -> dist/HillSequencer.zip
bash deploy.sh [host]       # scp + unzip on device
```

**Dev on Mac:**
```bash
python3 -m venv .venv && .venv/bin/pip install pygame-ce numpy scipy
.venv/bin/python main.py
# Samples default to ../hillchord/samples; override with HILLSEQ_SAMPLES env var
```

---

## Relationship to HillBeat and HillChord

HillSequencer **supersedes** HillBeat's sample engine (it uses HillChord's pitched sampler instead of raw WAV playback) while keeping HillBeat's full sequencing workflow. The three apps share:

- The same RG35XXSP button index map (`JOY_BUTTONS`)
- The same `ROMS/Samples/` instrument library (HillChord + HillSequencer)
- The same `audio/mixer.py` and `audio/effects.py` codebase (HillChord + HillSequencer)
- The same `_instr_stem` / `_group_files` / `_is_single_multisample` multisample detection logic

**HillBeat** is the simpler drum machine — one WAV per voice, no pitch shifting.  
**HillChord** is the live performance instrument — looper, arpeggiator, chord theory.  
**HillSequencer** is the compositional tool — grid sequencing of HillChord instruments.

---

## Known quirks / AI guidance

- `instruments.py` (`TrackRack`) owns one `Mixer` per track. When reading audio code, check which app's `mixer.py` you're in; both HillChord and HillSequencer have one at `audio/mixer.py`.
- The 16-beat fade uses `self._fades` (a list in `Mixer`), which must be cleared in both `stop_all()` and `panic()` under `_echo_lock` to avoid ghost fadeouts after the sequencer stops.
- `_instr_stem` in `library_overlay.py` must match the version in HillChord's `ui/library_screen.py` exactly. If one is updated, the other needs the same change.
- `ON_DEVICE` detection uses `/mnt/sdcard/ROMs` presence — this will be False on Mac even with the SD card mounted (it mounts at `/Volumes/Untitled`), which is intentional (keeps dev paths local).
- macOS creates `._filename.wav` sidecar files; any file scanner must skip filenames starting with `._`.
