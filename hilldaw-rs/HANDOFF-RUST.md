> **Start with `HANDOFF.md` (this folder).** This file is running notes kept while building.

# HillDAW in Rust — status and decisions

**Direction (2026-11-08): Rust is the target language.** The user asked for it ("ideal, unless another language is more efficient").
It is: no GIL in the audio/clock path, one static binary (no pylibs/numpy/pygame on the SP), tiny footprint, and the user's Hill apps on
the SP are already Rust (`/mnt/sdcard/ports/hill*-rs`). The Python tree in `../hilldaw/` is now the **reference implementation + hardware
diagnostics** (it proved every protocol detail); do not extend it. Its docs (`../hilldaw/HANDOFF.md`, `docs/BRIEF.md`,
`docs/PHASE0_RESULTS.md`) remain the source of truth for requirements, device facts and results.

## What exists here (`hilldaw-rs/`)
| Crate | State |
|---|---|
| `usb/` (`hilldaw-usb`) | **Done and PROVEN ON THE SP (2026-11-08)**: `list` matches; `midi-in` 20 s = 318 reads / 0 errors / 217 events (NOTE ON/OFF ch1, vel 90); `capture 60` = 60000 packets, 0 empty, 0 bad, 15,876,000 bytes = exactly 2,646,000 frames, identical to the Python+C result. 16 host tests pass. `sys` (usbfs structs + ioctl numbers, pinned by tests to values printed from the real `linux/usbdevice_fs.h`), `desc` (UAC1 parser; real FM-1 blob as a test fixture), `midi` (packets/decoder), `dev` (sysfs discovery by VID/PID, MIDI endpoint handle, released on Drop), `iso` (isochronous capture, replaces the C helper), `wav` (WAV I/O + numeric analysis; matches the Python `wavcheck` output to the printed precision). |
| `usb/src/bin/fm1tool.rs` | `list descriptors note pc cc panic midi-in capture wavcheck`. Same output shape as the Python tool. |
| `core/` (`hilldaw-core`) | **Started, 21 host tests pass** (not on the SP yet). `transport` (frame clock, bars/beats, snap, loop wrap that is sample-exact for any chunk size), `mixer` (Tone/Click/Clip sources, per-track gain/pan(-3 dB law)/mute/solo/peak, master gain + brickwall limiter, output **bit-identical for any chunk size**), `wav` (decode 8/16/24/32/float -> stereo f32, linear resample, 16/24-bit encode), `project` (versioned JSON, takes kept per track, non-destructive clip metadata, safe relative paths only, sanitising loader). Covers brief features 1/2 groundwork (click, loop region), 4 (gain/pan/mute/solo/limiter/meters), 14 (import+SRC basics). `examples/bench.rs` measures render cost; `release/mixbench-aarch64` is the device build. Dev box: 9 tracks = 0.055 ms per 23.2 ms chunk. **SP number not yet measured.** |
| (planned) device app | UI + SDL audio on the SP. Should reuse `hill-core` / `hill-device` from the Rust workspace (see below). |

## Building
```
cd hilldaw-rs
./build.sh test          # host tests
./build.sh device        # static aarch64 musl binary -> dist/fm1tool   (rustup target + `pip install ziglang`)
```
aarch64 uses Zig as the linker through `tools/zigcc-aarch64-musl.sh` (config in `.cargo/config.toml`, incl. `link-self-contained=no`, which
avoids a duplicate `_start` with Zig's own crt). Verified here: cargo 1.97, crates.io reachable, static ELF produced. `release/fm1tool-aarch64`
is a committed prebuilt for the SP. Run it from `/root` (ext4); the SD card is exfat and may not allow executing.

## The existing Rust workspace (not in this branch)
`origin/claude/gracious-faraday-6pjyu5` has `rust/` (`hill-core`: engine + `App` trait, sample-accurate step clock on the audio thread, mixer,
framebuffer UI `fb.rs`/`ui.rs`, WAV decoder, kv state; `hill-device`: SDL2 runner, `buttons.cfg`, HillMap; `hill-web`). Build recipe in its
`rust/README.md` (cargo-zigbuild, glibc 2.28 target, system libSDL2 at run time).
**Open problem:** the `hillbeat-rs` deployed on the SP (built Oct 26, has drum presets and 8 tracks) is newer than anything pushed to GitHub.
HillBeat's presets can only be reused from that newer source (or from the user's README/state files). Ask the user where the current Rust
source lives (local checkout? another repo?) before integrating. Do not rebase onto the stale branch blindly.

## Feature checklist (brief section 7) as of 2026-11-08
All logic below is in `core/` with host tests only unless noted. Nothing here has a UI yet.
| # | Feature | State |
|---|---|---|
| 1 | Click, count-in, time signature, tap tempo | `Click`, `Transport.beats_per_bar`, `TapTempo`, and the count-in in `session.rs` done. |
| 2 | Loop region + punch in/out | Loop wrap (sample-exact) and punch in/out in `session.rs` (`RecordSession`: each loop pass = its own take, latency-shifted capture index, result independent of callback size). **App wiring TODO** (route `Segment`s from the capture stream to take files). |
| 3 | Latency calibration | Measurement done and run on the SP (`hdtool latency`, 12.8 ms median); `placed_start` applies the offset. In-app flow TODO. |
| 4 | Gain/pan/mute/solo, limiter, meters | Done. |
| 5 | Trim, fades, normalize, reverse | Non-destructive, rendered by `TrimmedClip`; `normalize_gain` needs the file peak (needs loaded audio). |
| 6 | Undo/redo | Done (`Edit` returns its inverse). |
| 7 | Snap-to-grid, slice | `Transport::snap`, `split`, `slice_on_grid` done. |
| 8 | Filter, delay, reverb | `effects.rs`: LP/HP biquad, delay, shared send reverb. Done. |
| 9 | Sample pitch-shift across keyboard, root note | `sampler.rs`: stateless, seekable, Hermite interpolation, loop, polyphony cap. Done. |
| 10 | Arrangement view, sections | **TODO** (needs the drum track / HillBeat presets). |
| 11 | MIDI note recording from FM-1 keys | `notes.rs`: `NoteRecorder` (pairs ons/offs incl. overlaps, retrigger, stuck notes; tested on the shape of the real FM-1 stream), `quantize`, `midi_schedule` + `events_in_range` (send notes back to the FM-1 / retrigger), `to_sampler_notes`; `NoteModel` stored per track in the project; `Edit::SetNotes` for undo. USB MIDI-in is proven. **App wiring TODO** (timestamp each event with the transport position minus MIDI-in latency). FM-1 notes always arrive at velocity 90 in the one test; unknown if its keys are velocity-sensitive. |
| 12 | FM-1 program/CC control + patch browser | MIDI out proven. **Browser TODO; needs the M-VAVE CC guide (never supplied).** |
| 13 | BPM detection, fit to tempo | `tempo.rs`: ranked candidates incl. octave alternatives (audio cannot always settle half/double time; the UI must offer x2 and /2), `varispeed`, `fit_to_bars_ratio`. Done. |
| 14 | WAV import with sample-rate conversion | `wav.rs` decode + linear resample. Done. |
| 15 | Project browser | `device/src/projects.rs`: `scan` (newest first, duration from clips/notes via WAV headers, size, track count, bad projects listed with the error), `duplicate` (never overwrites), `delete` (only real project folders directly under the root; refuses `..`, separators, symlinks). 5 tests. Not run on the SP (no projects exist yet). |
| 16 | Battery and storage indicators | `device/` crate (`battery`, `storage`, `record_seconds_left`, formatting; 4 tests on a fake sysfs). `hdtool status` **PROVEN ON THE SP**: `battery: 36% Discharging 3561 mV`, `/mnt/sdcard: 100.4 GB free of 233.1 GB = 226h of mono 24-bit recording`. No `current_now` exists on the SP. |

Bench (`examples/bench.rs`): 8 clip tracks each with LP+HP+delay+reverb send + click/tone + 12-voice sampler = 0.58 ms avg / 2.8 ms worst per 23.2 ms chunk
on the dev box (~2.5% of budget). **SP measured (2026-11-08): 3.76 ms avg / 6.03 ms worst per 23.2 ms chunk = ~16% of one core, 6x headroom** for the heaviest load tested (~6.5x slower than the dev box). The Python mixer needed ~2 ms for just a click + tone.

## Editing (done in core, host tests only)
`core/src/edit.rs` (8 tests) + `mixer::TrimmedClip`: non-destructive trim / linear fades / gain / reverse rendered at playback; `Edit` enum where every
edit returns its inverse (so undo/redo is two stacks; atomic `Batch`, redo cleared on a new edit, depth capped at 100), `split` (a split plays
bit-identically to the original, forward and reversed), `slice_on_grid` (one undo step), `normalize_gain`. Covers brief features 5, 6, 7 at the model level.
A bug found by the undo/redo round-trip test: removing a take could not be redone (the inverse of `PopTake` lost the take); fixed.
Not yet: UI for any of it; computing a file's peak for normalise (needs the loaded audio); per-track effects (feature 8), pitch-shift/BPM (9, 13).

## Recording (in progress)
`core/src/record.rs` (4 tests): `Downmixer` (24-bit stereo -> 24-bit mono, (L+R)/2, independent of how the byte stream is split),
`StreamingWav` (header rewritten by `flush_header()`; a crash keeps everything up to the last flush), `detect_onset`, `placed_start`
(take start = record start minus latency offset), `drift_ratio`.
`tools-bin/` (`hdtool`, release `hdtool-aarch64`): `record SECS OUT.wav [--note N]` = capture thread -> bounded queue (512 chunks) -> writer thread -> SD card,
reports overruns / captured vs written frames / worst single write; `latency [--runs N]` = MIDI note-on -> first audible sample in the capture.
**PROVEN ON THE SP (2026-11-08):**
- `record 60` to the SD card (exfat) while capturing: 2,646,000 captured = 2,646,000 written mono frames, **0 overruns**, 59 header flushes,
  worst single write 7.27 ms (queue slack is ~500 ms). Streaming 132 KB/s to the SD card is not a problem.
- `latency --runs 8` (MIDI note-on -> first audible sample in the capture): **median 12.8 ms (563 frames), min 7.3, max 14.0, spread 6.7 ms**.
  The spread looks like FM-1-side quantisation (block/MIDI polling), not our timing, but that is unconfirmed. Use ~563 frames as the default
  `latency_offset_frames` for MIDI-driven recording and expect about +/-3 ms of jitter on sequenced notes. Live-played audio has no MIDI leg:
  its offset is the USB capture path plus whatever the player hears from the SP output, which needs the Rust output path to measure.
- Not yet measured: clock drift over minutes (FM-1 vs SP sound card).

**Design risk to measure: two clocks.** The FM-1 samples audio on its own clock and the SP's sound card plays on another. Over a long take they drift
(e.g. 50 ppm = 132 frames = 3 ms per minute). Plan: store `captured_frames` and the transport frames elapsed per take (`drift_ratio`), and play
the clip back at that ratio (non-destructive, linear interpolation) if the measured drift matters. Measure first: record a long take against a running
transport on the SP before building the correction. Also expect `latency` to be accurate only to ~1-2 ms (origin = submit of the first iso transfer).

## Audio output (decision)
User confirmed the Python aplay and pygame-queue tests both sounded clean (0 underruns, no audible issues), so the SP's ALSA path and a
44.1 kHz / s16 / 1024-frame period are known-good. In Rust, output through the **SDL2 audio callback** (what `hill-device` already does; the callback
is the single mixing point driven by one transport clock). Re-measure underruns and latency on the SP once the Rust mixer exists; do not assume the
Python numbers (~58 ms pygame, ~146 ms aplay) carry over.

## Next steps
1. DONE: Rust `fm1tool` verified on the SP (see table above). Capture level that run was only -27.9 dBFS peak (FM-1 volume); 24-bit has headroom.
2. Get the current Rust source + HillBeat presets; decide workspace integration.
3. DONE in core: mixer + transport + project model (above). NEXT: (user) run `mixbench` on the SP; then recording (capture thread -> ring buffer -> disk, mono
   downmix, take management), count-in, latency calibration (MIDI note -> capture onset), then the device app (SDL2 audio callback + framebuffer UI),
   drums from HillBeat once the current Rust source/presets are available, then export (mixdown/stems; ffmpeg/lame/flac exist on the SP for MP3/FLAC).

## Drum track (2026-11-07, local session)

Done, host-tested, and **verified on the SP (2026-11-08)**: `hdtool drums render afro-cuban-r1 ... --cycles 4` picked the owner's `04. LM Vinyl` kit from `/mnt/sdcard/ROMS/Hill Apps/Samples`, played through `aplay`, owner confirmed audio quality is fine.

- `core/src/drums.rs`: stateless `DrumTrack` (a mixer `Source`). Sections chain back to back; each is a pattern repeated `cycles` times. Hit time is a pure
  function of section start, cycle, step, bpm, swing, so output is bit-identical for chunk sizes 1..9999 (tested). Mirrors HillBeat: step = 1/`div` beat, odd steps
  delayed by `swing*step` (0..0.25), gain `vel/127 * vol * 0.9`, flam grace note 30 ms early at 0.4, per-voice volume up to 4.0, tom pitch by resampling (linear interp).
- `drums/` (`hilldaw-drums`): the only crate depending on `hill-core` (path `../../rust/hill-core`). `DrumLibrary` finds kits like HillBeat (`library::kit_entries`),
  calls `Preset::plan(kits, 8)`, loads sounds from a `Host`, caches. `DirHost` is a read-only folder host (no SDL). All 267 shipped presets load with built-in sounds (tested).
- `project.json`: new `drums` block (`#[serde(default)]`, so old files load). Sections store the preset **id**, not an index. Values clamped in `sanitize`.
- `hdtool drums list [TEXT]` and `hdtool drums render ID OUT.wav [--cycles N] [--bpm B] [--swing S] [--samples DIR]`: renders through the real `Mixer` + limiter to a 16-bit WAV.

Decisions / limits to know about:
1. A "cycle" is one pass of the pattern (`steps/div` beats), so for 4/4 sixteenths it is one bar; 3/4 and 12/8 presets have their own length. The UI should say "cycles" or compute bars.
2. No choke: HillBeat limits 4 voices per drum and fades the oldest; `DrumTrack` lets every hit ring to the end of its sample (capped at 4 s). Retriggers of a long open hat will overlap. Add a choke only if it is audible.
3. Tempo is the project's constant bpm; the grid does not change mid-song.
4. Once a `DrumTrack` is boxed into a mixer `Track` its vol/mute can't be changed live. The app needs a shared handle (atomics or `Arc<Mutex>`) when it is wired in.
5. `libc::Ioctl` is Linux-only; `usb/src/sys.rs` now uses `req as _` so the crate also compiles on macOS. Type-checked for `aarch64-unknown-linux-gnu`; the musl release binaries were not rebuilt.

Fixed 2026-11-08: `hdtool drums list | head` used to die with Broken pipe (println! panics on a closed pipe; panic=abort made it an abort on the SP). Now writes through a locked stdout and stops quietly; regression test `tools-bin/tests/closed_pipe.rs`. It only showed on the SP because `head` quit while the slower CPU was still writing.
Build without the pip zig: `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=<shim running 'zig cc -target aarch64-linux-musl'> cargo build --release --target aarch64-unknown-linux-musl -p hilldaw-tools` (Homebrew zig works).

## HillDAW app skeleton (2026-11-08, local session)

The project is now called **HillDAW** (was PocketDAW): dirs `hilldaw-rs/`, `hilldaw/` (Python reference), crates `hilldaw-*`, tool `hdtool`. The git branch name was left as is.

- `app/` (`hilldaw-app`): `HillDaw` implements `hill_core::App`. No SDL, so it is tested on any computer (`app/tests/app.rs`, 18 tests: audio, controls, quit chord, save/restore, drawing).
  Screens: **Mixer** (transport, 8 tracks + drums, level/pan/mute/solo/arm, meters, master) and **Arrange** (drum sections: pattern, style, cycles, mini step grid). Status bar: FM-1 on the USB bus, battery, free space + recording time.
  Status comes from a background thread (`status::spawn_poller`) so the UI/audio mutex never waits on sysfs.
- `sp/` (`hilldaw`): the SDL program = `hill_device::run_with("hilldaw", ...)`. **`run_with` is the one change made to the owner's `rust/hill-device`** (additive; `run` now calls it). Tell the owner so they can take it upstream.
- Plain `cargo test` / `clippy` skip `sp` (`default-members`) so no SDL is needed; build it with `-p hilldaw`. Desktop link flags for Homebrew SDL2 are in `.cargo/config.toml`.
- Controls follow the Hill apps: START play/stop, SELECT record (not wired: says so), L1/R1 screens, FN+SELECT help, START+SELECT hold quit, FN+Left/Right tempo, FN+A tap, FN+X click, FN+B rewind, FN+L2/R2 swing.
  Mixer: Left/Right strip, Up/Down level (1 dB), L2/R2 pan, X mute, Y+X solo, A arm, B reset. Arrange: Up/Down section, Left/Right pattern, L2/R2 style, Y+Left/Right cycles, A add, X delete.
- The project is saved through the host's state store under key `hilldaw` (autosave every 30 s by the runner and on quit). Project *folders* (`project.json` + `audio/`), the browser UI and recovery are still to do.
- Honest gaps: no recording, no FM-1 keys/MIDI, no sampler, no export, no loop/punch UI, no count-in. The REC lamp reads "n/a". Track strips 1-8 are empty (no clips can be loaded yet).
- Decisions: the click track is hidden and stays audible while something is soloed; changing tempo while playing keeps the musical position; drum strip pan/solo/mute live in `project.drums` (`pan`, `solo`, `enabled`);
  a project that names a preset that no longer exists loses that section (with a toast) instead of failing to load; output rate other than 44.1 kHz is accepted (positions are frames at the output rate).

Build for the SP (needs `cargo-zigbuild`, Homebrew `zig`, the arm64 libSDL2 cached by the owner's `rust/build.sh`):
`RUSTFLAGS="-L <rust/.cache/arm64sdl/lib> -C link-arg=-Wl,--allow-shlib-undefined" cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28 -p hilldaw`
Install on the SP: binary to `/mnt/sdcard/ports/hilldaw/hilldaw`, `ports/HillDAW.sh` to `/mnt/sdcard/ROMS/Hill Apps/HillDAW.sh` (a launcher generated from `rust/ports/launcher.sh.in`).
**Run on the SP (2026-11-08): the owner ran the checklist (START plays the drums, Arrange edits, mixer level/mute/tempo, quit and restart keeps the project) and reports it all works.** Their own wording was \"tests work\"; audio latency, underruns and mutex contention under load were not measured. Also verified on the Mac: tests, screenshots of every screen, SDL window launches.

## FM-1 MIDI guide (2026-11-08)

Found: the vendor's "MIDI CONTROL EN" guide and the release note are on <https://www.m-vave.com/download.html>. Transcribed with sources in `docs/FM1_MIDI.md`.
Key points: CC for FX goes on the **FX channel (default 2), not the note channel**; Program Change 0-127 selects voices; **the guide lists FX CCs 0-23 but the V16 release note says they became CC 70-93**, with no per-number table, so which applies is unverified.
Verified on the owner's V16 unit (2026-11-08): FX CCs are 70-93 on channel 2; the old 0-23 do nothing. The order within 70-93 is still only inferred (old number + 70) apart from 70 and 72. Nothing is coded from it yet.

## FM-1 CC probe (2026-11-08)

`hdtool probe-cc [--ch 2] [--from 70] [--to 93] [--out FILE]` measures what each CC does, from the FM-1's own USB audio (so room noise is irrelevant).
Logic is pure and tested in `core/src/probe.rs` (FFT centroid, level, tail; thresholds learned from repeated baselines; a fake FM-1 with a hidden mapping is used to test the two passes).
Pass 1 raises each CC alone from all-off. Pass 2 (revised after run 1, see docs/FM1_MIDI.md) tests one effect group at a time (default 4 CCs per group, `--group N`): the group's first CC (its switch) on, its parameters at 64, everything else off, then each CC to 0 and to 127. A group that is silent at rest falls back to parameters at 0, then is skipped. ~80 trials, ~4 min. Leaves the probed CCs at 0. The first version lowered each CC from an all-127 state, which is nearly silent (filter high-passes everything), so its pass 2 was unusable.
A trial that captures no sound when the baseline had sound counts as a change ("silenced"). Run 1 on the real unit worked mechanically (pass 1 usable); run 2 (grouped) not yet done.

Probe run 2 (grouped) on the real unit confirmed the FX map: **CC 70-93 = the old CC 0-23 table + 70**, six groups of four, switch first; values are native units (clamp, not scale).
Data: `core/data/fm1_fx_cc.json`, loader `core/src/fm1.rs` (`fx_map()`, `FxParam::clamp/from_fraction`). Four entries (76, 79, 91, 92) are `order`-only: no change was measurable.
Next for the FX/patch browser: Program Change 0-127 on the note channel (documented, not yet tested on the unit), then a UI over `fx_map()`.

## FM-1 screen (2026-11-08)

Third screen in HillDAW (L1/R1 cycle Mixer / Arrange / FM-1): **Voice** (Program Change 0-127, shown as voice 001-128) and the six FX groups from `core/data/fm1_fx_cc.json`.
Keys: L2/R2 section, Up/Down row (voice: +-10), Left/Right value (Y: +-10), A effect on/off, X test note (C4, 0.7 s), B all notes off.
- Only what the user chose is stored (`project.fm1`) and sent; untouched controls show `--`. A never-set parameter starts from the middle of its range. On connect (status poll sees the FM-1) the stored values are resent once; again after an unplug/replug.
- `app/src/fm1link.rs`: a thread owns the USB MIDI port, opened lazily on the first message and reopened after a failed write. **While HillDAW has the port open, `fm1tool`/`hdtool` cannot claim it** (usbfs: one claim per interface): quit HillDAW first, or use the tools before touching the FM-1 screen.
- Controls marked `?` (reverb decay, delay decay, phaser freq/depth) could not be measured; the screen says so.
- 25 app tests + 3 link tests; screens checked by snapshot. **Run on the SP 2026-11-08: the owner ran the checklist (voice steps change the sound, effect switch and value audible, test note, settings restored after restart) and reports everything works.** Not measured: how long the first message takes after opening the port.

## Clock drift test (2026-11-08)

Question: the FM-1 records on its clock, the SP plays on its own; do they agree well enough to record against the drums?
Method (`core/src/drift.rs`): each clock is compared with the system's monotonic clock by a least-squares fit of frames against time (precise to a few ppm even with 20 ms timestamp jitter; tested).
- `hdtool drift-cap [--secs 300]`: FM-1 capture clock. Prints a running figure every 30 s and saves `/root/drift-capture.txt`.
- HillDAW itself counts the frames its audio callback delivers; after 30 s a line "AUDIO OUT 44100.xx Hz +n ppm" appears above the footer, and on quit the figures are saved to `hilldaw-drift_state.txt` (state store) next to the program.
- Relative drift = capture rate / output rate - 1. Rule of thumb: under ~20 ppm is negligible for a few-minute song (1.2 ms per minute); 50 ppm is 3 ms per minute (30 ms per 10 minutes).
Run both together for 5+ minutes (HillDAW on the Mixer screen with the drums playing, `hdtool drift-cap --secs 300` in Termius). Not yet run on the SP.

## Drums stopped after 4 bars (2026-11-08, found by the owner on the SP)

Cause: the starter groove is one section of 4 cycles (= 4 bars) and the transport had no loop, so the arrangement simply ended. Fix: `project.drums.loop_arrangement` (default true, old files get it too) makes the transport wrap at the end of the drum arrangement (`apply_loop`, sample-exact via `Transport::loop_region`); FN+Y toggles it; the Mixer shows "loop on/off". An edit that shortens the song wraps the playhead instead of leaving it past the end.
Regression tests: `the_drums_keep_going_past_the_end_of_the_arrangement_while_looping`, `fn_y_turns_the_loop_off...`, `looping_is_identical_for_any_buffer_size...`.

### Clock drift result (2026-11-08, the SP, FM-1 plugged in, HillDAW running)

| Clock | Rate | vs system clock | Basis |
|---|---|---|---|
| FM-1 capture (`hdtool drift-cap`, 300 s) | 44101.885 Hz | +42.8 ppm | 2679 samples, timestamp jitter 0.4 ms; running figures stayed within 41.5-42.4 ppm; 300016 packets, 0 empty/bad |
| SP sound output (HillDAW readout, 329 s) | 44101.83 Hz | +42 ppm | read from the screen (photo), 2 decimals |

**Relative drift = capture / output - 1 = about +1.2 ppm** (0.07 ms per minute, about 4 ms per hour). Negligible: **no drift correction is needed in the recording code.** Still store `captured_frames` and `elapsed_transport_frames` per take (`drift_ratio`) as a cheap sanity check.
Both clocks read +42 ppm against the monotonic clock, so most likely the SP's system clock is the one that is ~42 ppm slow; only the difference matters here. The two readings were taken in separate runs (the FM-1 figure at the same time as an earlier, shorter output reading of +42 ppm at 44 s that agreed); a fully simultaneous long run was not repeated.
Not yet checked: drift while the SP is hot after a long session, and over 30+ minutes.
Note: the saved drift file is `hilldaw-drift_state.txt` (the host appends `_state.txt` to the key).

Loop confirmed on the SP by the owner (2026-11-08): "looping worked".

## Recording (2026-11-08)

SELECT records onto the one armed track (A on the Mixer arms; arming more than one is refused). Count-in (one bar of click, FN+L1 toggles, stored in the project) plays the click only; the transport and drums wait for it.
- **One continuous capture per recording session** (`app/src/recorder.rs`, `UsbRecorder`): FM-1 -> mono 24-bit -> bounded queue -> writer thread -> `<project>/audio/rec-NNN.wav`, header refreshed about every second so a crash or power loss costs at most ~1 s (the same path proven by `hdtool record`: 60 s, 0 overruns). Files are never modified.
- **Takes are non-destructive clips of that file** (`ClipModel{file, start, offset, len}` + a `TakeModel` in the history). `RecordSession` (core) decides the takes: each loop pass is its own take, punch is supported by the model (no UI yet). The track plays only the latest take; the others stay in `takes`.
- **Alignment:** the capture index of the first recorded frame is worked out from the common clock: `cap_base = (time recording began - time the capture started) * 44100`, where the time is the audio callback's. The clip offset is `cap_base + latency + position`. `project.latency_offset_frames` (default 563 = the measured MIDI-to-capture latency of 12.8 ms) is the only calibration. **It does not yet include the SP's output latency (unmeasured)**, so a first take may sit a little early or late against the drums; the plan is a nudge control once the real offset is known from a test on the SP.
  Residual error is the audio callback's wake-up jitter (a few ms at most expected; not measured).
- Clock drift was measured at ~1.2 ppm (see above), so no correction is applied.
- Stopping (SELECT, START, or the quit chord) closes the file, keeps capturing until the delayed audio has arrived, clamps every take to what reached the disk, reports dropped chunks ("N chunks dropped"), saves the project at once, and loads the take for playback.
- Playback uses `MonoClip` (mono floats in memory, ~160 kB per second of audio; recordings over 600 MB on disk are refused). 10 minutes is about 106 MB per take in memory. Streaming playback from disk is future work.
- The recorder is a trait; 11 recording tests use a fake that serves a ramp so exact sample alignment, count-in handoff mid-chunk, loop takes, clamping, persistence and the REC lamp are all checked.
- Not done: input level meter, latency nudge/calibration screen, choosing between takes, punch UI, import of orphaned recordings after a crash during a take (the audio file is valid but the project does not know it), multiple projects (the folder is `projects/song` next to the program, `HILL_PROJECTS` overrides).
**Not run on the SP yet.** Run it with the FM-1 plugged in: arm a track, press SELECT, play, press SELECT again.

## Loop semantics changed (2026-11-08, after the owner's first recording test on the SP)

Owner report: "the measure counter loops after four measures" (and a long recording is cut into takes). Cause: the previous fix wrapped the **transport** at the end of the 4-bar drum arrangement, so the bar counter wrapped and every pass of a long performance became its own take (only the last plays back). **This supersedes the "Drums stopped after 4 bars" section above.**
Now: the **drums repeat** (`DrumTrack::looping`, stateless: every cycle that can still sound in a chunk is rendered from its own origin, so the output is chunk-size independent and a long hit rings over the seam) while the **transport runs on**: the bar counter keeps counting and a long performance is one take.
- `project.drums.loop_arrangement` (default on) now means "the drum arrangement repeats"; FN+Y toggles it ("Drums loop on/off"); the mixer shows "drums loop" / "drums once".
- The transport only wraps inside `project.loop_region` (none by default; `HillDaw::set_loop_region`, no UI yet). Recording over a loop region makes each pass its own take (tested).
- Regression tests: `the_drums_keep_going_past_the_end_of_the_arrangement_and_the_bar_counter_keeps_counting`, `a_long_performance_over_the_repeating_drums_is_one_take_and_the_bar_counter_does_not_wrap`, and three DrumTrack looping tests.
- The owner's three test recordings (rec-001 about 49 s, rec-002, rec-003) were made with the old behaviour; rec-001 was probably five takes. Their audio files are valid.

First recording test on the SP (2026-11-08): the owner reports the take "seemed to be on-time" against the drums with the default offset (`latency_offset_frames` = 563, the measured MIDI-to-capture latency, nothing for the SP's output latency). By ear, one listener, one session: not measured. No nudge control is needed yet; revisit if a later take sounds early or late.

## Gain up to +24 dB (2026-11-08, owner request)

Strip gain (tracks and drums) now goes to **+24 dB** (`MAX_STRIP_GAIN` = 16.0 in the project, `MAX_DB` in the app; it was +6 dB in the app, +12 dB in the file). Up/Down is 1 dB, **Y+Up/Down is 6 dB**. The meters and faders share a -60..+24 dB scale with a 0 dB tick; meter colours: green below -9 dBFS, amber to -1, red above. The master limiter (ceiling 0.98) is the safety net; boosting a recording also boosts its noise.

## Mixdown and stem export (2026-11-08)

Fourth screen (L1/R1): **Export**. Options (stored in `project.export`): WAV 24-bit or 16-bit, dither (16-bit only, TPDF, deterministic), normalize the mixdown's peak to -1 dBFS (stems get the same gain), stems on/off. A starts, B cancels, quitting cancels.
- Core (`core/src/export.rs`, pure): `render_offline` (identical for any chunk size), `encode_pcm` (16/24-bit, dither, clip counting), `normalize_gain`, `Dither`.
- App (`app/src/export.rs`): the UI thread builds a `Snapshot` (clip audio shared as `Arc`, drum track cloned) and creates the files; a worker thread renders it through a fresh `Mixer` and writes with `StreamingWav`, so playback is untouched and the UI stays responsive (progress bar, cancel).
- **Mixdown** = every strip with level/pan/mute/solo through the master limiter. **Stems** = one file per track that has audio and one for the drums, through their own level and pan but **not** muted/soloed and **without** the limiter: they add up to the mixdown only while the mix itself is not being limited (the starter groove alone peaks above the limiter's 0.98, so turn things down for a clean sum). Clipped samples (past full scale, in any file) are counted and reported on screen.
- Length = the longest of the last clip's end and one pass of the drum arrangement, plus a 2 s tail. Looping drums repeat to cover a longer take, then stop at the end (`DrumTrack::end_frame`); hits already sounding ring into the tail.
- Files go to `<project>/export/`: `song-mixdown.wav`, `song-stem-1-track-1.wav`, `song-stem-drums.wav`; a number is added instead of overwriting (`song-mixdown-2.wav`). Names are made file-safe.
- Tested: the exported drums equal the live render to 1e-5 (24-bit); a recorded take is at the right place (stem = file frame 563+p through gain and pan); stems add up to the mixdown; mute/solo shape the mixdown only; normalize hits -1.00 dBFS; 16-bit is the rounded 24-bit value (within 1 LSB) and dither changes it (within 2 LSB); cancel; nothing-to-export; a refused file.
- **Not done:** MP3/FLAC (ffmpeg, lame and flac exist on the SP; a later step can run them on the WAV), a cancelled export leaves its partly written file, memory: recordings stay in memory as mono f32 (see Recording), no export of a loop region or a selection, no per-export file name.
- **Not run on the SP yet.** Render time is not measured on the device: the mixer benchmark was 3.8 ms per 23 ms of audio (about 6x real time), so a 5-minute song should take about a minute, twice with normalize.

## Track screen, input meter, loop/punch, per-track effects, FLAC/MP3 (2026-11-08, one batch)

Screens (L1/R1 cycle): **Mixer, Track, Arrange, FM-1, Export**. **Track** works on the strip picked on the Mixer (B goes back); L2/R2 switch its four tabs.
- **TAKES**: the take history of the track (from bar, length, file, PLAYING). Up/Down pick, **A** makes it the playing one (`set_active_take`, rebuilds the clip from `TakeModel{start, offset, placed}`; takes from before this was stored are `placed=false` and refused), **X twice** removes it from the list (audio stays on the card; if it was playing the newest remaining take plays).
- **FX** (per strip, drums too): low-pass, high-pass (0 = off; cutoff steps are multiplicative, Y bigger; a filter comes in from its end and switches off past it), delay time/feedback/mix, reverb send, and the **shared reverb room**. `TrackFxModel::apply` builds the chain; `sync_reverb` creates the reverb only while something sends to it. The **export uses the same model**, so exported audio has the effects (tested equal to the live render).
- **INPUT**: live FM-1 level (-60..0 dBFS bar, 1.5 s hold tick, CLIP warning within 0.2 dB of full scale), **latency offset** nudge (about 1 ms per step, Y = 5 ms, 0..100 ms; default 563 frames = 12.8 ms) and the count-in switch. The meter also shows in the transport panel while armed/recording.
  `Recorder::monitor/level`: the USB recorder runs the capture without a file while a track is armed or this tab is open (one claim of the interface; **recording takes over from monitoring**, and `hdtool`/`fm1tool` cannot claim the capture interface while HillDAW is monitoring). A missing FM-1 is shown on the tab and retried every 2 s, not every tick.
- **LOOP**: loop on/off, start bar, length in bars; punch on/off, from bar, length in bars. A sets a start/length row from the playhead's bar; Y moves 4 bars. Loop on brings back the last region. Loop recording makes a take per pass; punch records only the window (`RecordConfig.punch`). Both stored in the project (`loop_region`, `punch`).
- **Export formats**: the Format row cycles WAV 24 / WAV 16 / FLAC 24 / FLAC 16 / MP3. FLAC/MP3 are made by converting the rendered WAV with `flac` / `lame` (ffmpeg as the fallback); the WAV is deleted only after the encoder succeeded; if none works the WAV is kept and a warning is shown. Export file names are unique across wav/flac/mp3 so a converted file is never overwritten.
- New project fields (all `#[serde(default)]`, old files load): `TakeModel.{start,offset,placed}`, `TrackModel.fx`, `DrumsModel.fx`, `Project.{punch, reverb_room}`, `ExportModel.format`.
- Tests: 16 in `app/tests/track.rs` plus updated navigation tests. Not exercised without hardware: the real `UsbRecorder` monitor/level, and the real `flac`/`lame` (the converter is tested with `cp` and `false` standing in).
- Not done: undo/redo, trimming/fades/split on clips, sampler, FM-1 keys as notes, project browser, recovery of recordings left by a crash during a take, MIDI/automation.
- **Not run on the SP yet.**

Owner test on the SP (2026-11-08) of the 38c16a8 batch (+24 dB gain, mixdown/stem export, Track screen, input meter, loop/punch, effects, FLAC/MP3): "everything seems to be working fine". By ear and by use, one session; the owner did not report per-step details (for example whether the FLAC/MP3 encoders were present), so treat the encoder path as seen working but unmeasured. Owner asked how to go back to bar 1 (answer: FN+B, no single-button shortcut yet).

## Generic USB audio input and undo/redo (2026-11-08)

### Any USB audio input, not just the FM-1 (owner request; audio only, no MIDI)
- `hilldaw-usb::audio` finds every attached **USB Audio Class 1.0** device with a capture stream (from `/sys/bus/usb/devices/*/descriptors`) and chooses a format: prefers 44.1 kHz, then 48 kHz, then the lowest rate above CD quality, then the highest; among equals the smallest frame. 8-bit and non-PCM streams are skipped. **USB Audio 2.0 (most high-speed USB-C interfaces) is not decoded and does not appear.**
- `MonoMixer` (core) turns any capture format (1..N channels, 16/24/32-bit) into mono 24-bit, identical to the old FM-1 downmixer for 2ch/24-bit (tested). The recorder writes mono 24-bit files **at the device's own rate**; a take at another rate (48 kHz usually) is **resampled to 44.1 kHz when it is loaded** (4-point Hermite, `resample_mono`), so the app keeps counting in project frames. Linear/Hermite resampling is "good", not studio-grade.
- For a device that offers several rates the recorder sends the UAC1 `SET_CUR` sampling-frequency request (`dev::set_sampling_rate`, `IsoCapture::open_with_rate`). The new usbfs control-transfer struct and `USBDEVFS_CONTROL = 0xC0185500` were proven with `_Static_assert` against the real `linux/usbdevice_fs.h` (zig cc, aarch64-linux-musl). **The request itself has not run on any device.**
- Choice: `Settings.input_device` (`vid:pid` or Auto), stored under its own key `hilldaw-settings` (a device setting, not part of a song). Auto = the FM-1 if attached, else the first input. **A chosen device that is unplugged is shown as "not connected" and is never silently replaced by another.** Input tab row 0 steps Auto/devices with Left/Right; changing it while recording is refused; the meter reopens on the new input. The top bar now names the input in use (was an FM-1 dot); the FM-1 screen shows "FM-1 MIDI: connected / not found".
- `hdtool inputs` lists inputs; `hdtool drift-cap --input VID:PID` measures any device's clock (the 1.2 ppm result is for the FM-1 only; **other devices have not been measured**).
- Latency: the default 563 frames is the FM-1's measured figure; other devices will differ: use the latency nudge on the Input tab.
- Not verified on hardware: any non-FM-1 device. Two identical devices share an id; the first is used.

### Undo / redo
- **Whole-project snapshots** (not the core `edit.rs` command history, which the app does not use): every edit calls `checkpoint(label, key)` before changing anything; the snapshot is kept only if the project really changed by the end of the button press (`settle`). Same key within 700 ms = one step (holding a button changes a level in one step); a new edit clears redo; 100 steps.
- Keys: **Y+B undo, Y+A redo** (a toast names the edit, e.g. "Undo: level"). Refused while recording. Covers levels, pan, mute, solo, tempo, swing, drum patterns/sections/cycles/loop, FM-1 voice and effects, per-track effects and reverb room, loop and punch, latency and count-in, takes (choose/remove) and a recording (one step: undo removes the take from the track, the audio file stays on the card; redo brings it back).
- `restore` rebuilds everything built from the project (click, drums, mixer, effects, clips, loop) and **resends changed FM-1 settings** (the unit keeps what it was last told). Export options are settings, not edits: undo leaves them alone.
- Not covered: arming, view/selection, export option changes, anything not in `Project`.
- Tests: `app/tests/undo.rs` (11), generic input tests in `app/tests/track.rs`, usb/core/recorder tests. **Not run on the SP yet.**

## Waves screen and export drums switch (2026-11-08, owner request)

- **WAVES** (L1/R1: Mixer, Track, **Waves**, Arrange, FM-1, Export): one row per track and the drums, time left to right, a bar ruler on top. Tracks with audio get a bright label and their waveform (drawn at the level as heard, i.e. times the strip gain); empty ones are dim and say "empty"; muted/un-soloed ones are grey with an M / S flag; the drum arrangement is shown as blocks (lighter copies while the drums loop); a white playhead (red while recording); the loop region (green) and punch region (red) as bands on the ruler; the picked strip row is outlined.
- Keys: Left/Right scroll a bar (Y: a page), L2/R2 zoom out/in (2, 4, 8, 16, 32 or 64 bars across), Up/Down pick the row (the same strip as the Mixer and Track screens), A moves the playhead to the left edge, X follow on/off, B back to the Mixer. Following turns the page when the playhead leaves the window (the window starts on a multiple of its own width); scrolling by hand turns following off. The window always starts on a whole bar and cannot scroll past the last bar that has anything in it.
- How: `core/src/waveform.rs` makes a (min, max) overview per 512 frames once when a recording is loaded (`peak_cache`) and squeezes it to one pair per pixel at any zoom (`envelope`), so drawing costs a few buckets per pixel however long the take is. Memory: 8 bytes per 512 frames, about 30 kB per minute.
- Not done: editing from this screen (moving/trimming clips), waveform colours per take, selecting a range.
- **Export option Drums on/off** (Export screen row 5, `project.export.drums`, default on): off leaves the drums out of the mixdown **and** the stems, the song length is then set by the clips alone (a drums-only song reports "Nothing to export"), and a soloed drum strip does not silence the rest.
- Tests: `app/tests/waves.rs` (8, pixel-level: used vs empty rows, waveform shape, boost, muted grey, drum blocks and repeats, playhead/loop/punch positions, zoom/scroll/follow/seek), 3 export tests, 4 core waveform tests. **Not run on the SP yet.** The footer tab names were shortened to fit six screens (MIX, TRACK, WAVES, ARRANGE, FM-1, EXPORT).

## Playback through a USB audio device (2026-11-08, owner request) — **never run on any hardware**

- Why it is userspace: the SP kernel has no USB audio driver, so a USB DAC/FM-1 is not a sound card. `usb/src/iso_out.rs` (`IsoPlayback`) claims the playback interface, selects the alt setting, sets the rate if the device offers several, and keeps 4 isochronous OUT URBs (4 x 1 ms packets each, about 16 ms queued) in flight. `PacketSizer` gives 44.1 kHz as nine 44-frame packets and one 45 (and 48 kHz as 48); `fill_urb` sets every packet's length. Tests cover sizing and the URB layout only.
- Format: `usb/src/audio.rs` (`list_outputs`, `choose_playback_format`, `pick_output`): UAC1 only. Prefers 44.1 kHz, then 48 kHz; prefers a **rate-following (adaptive/synchronous) endpoint over an asynchronous one**; stereo > mono > more channels; more bits. The FM-1 fixture resolves to 2ch / 24-bit / 44.1 kHz / adaptive, interface 1 alt 1, endpoint 0x03 (from its descriptors, not from a run).
- Clocks: the SDL audio callback (the SP's codec) stays the master. `HillDaw::audio_at` hands each finished block to `AudioOut::push`; if the sink takes it, the block given back to SDL is zeroed, so the built-in output is silent. `app/src/usbout.rs`: a ring (target 1536 frames, about 35 ms; 8192 maximum) feeds a thread that resamples with `core/src/outstream.rs` (`StreamResampler`, 2-point linear) and nudges the pace by up to +-0.5 % according to ring fill (`adaptive_step`) so the two clocks never starve or overfill it (a 120 s simulated mismatch holds the fill level in tests). It plays silence until the ring has reached its target (and again after an underrun, counted).
- Fail-safe: if the device cannot be opened, `select` returns an error (shown on the Input tab and in a toast) and the built-in output keeps playing. If the device is unplugged mid-play the stream ends, `push` is refused and the very next block plays on the built-in output; the saved choice is kept and the app retries every 2 s while the device is listed.
- UI: Track screen, Input tab, **row 3 "Output"**: Left/Right steps Built-in and every attached USB output. Choice saved as `Settings.output_device` (device setting, not in the song). The line under it says "playing through ...", "waiting for the device" or the error; the top bar shows `OUT <name>` while streaming. Export is unaffected (offline render).
- Known limits: **asynchronous endpoints need the device's feedback endpoint, which is not implemented** (a device with only an async playback endpoint is still used, with the adaptive nudge as the only rate control; it may drift or glitch). UAC2 devices do not appear. Resampling quality is basic (linear). Extra delay about 35 ms + 16 ms: the performer hears the drums later while recording, so re-check the latency offset after switching. No volume or mute control of the device is sent.
- Tests: `usb` (format choice, packet sizing, URB layout), `core/outstream` (6), `app/src/usbout.rs` (5: priming, underrun, overrun cap, 48 kHz mono 16-bit packet, no-device error), `app/tests/output.rs` (5: choose/persist/mute, open failure, unplug/replug, saved-but-absent, no output connected). **None of the USB side has run on the SP or on any device.**
