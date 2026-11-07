> **Start with `HANDOFF.md` (this folder).** This file is running notes kept while building.

# PocketDAW in Rust — status and decisions

**Direction (2026-11-08): Rust is the target language.** The user asked for it ("ideal, unless another language is more efficient").
It is: no GIL in the audio/clock path, one static binary (no pylibs/numpy/pygame on the SP), tiny footprint, and the user's Hill apps on
the SP are already Rust (`/mnt/sdcard/ports/hill*-rs`). The Python tree in `../pocketdaw/` is now the **reference implementation + hardware
diagnostics** (it proved every protocol detail); do not extend it. Its docs (`../pocketdaw/HANDOFF.md`, `docs/BRIEF.md`,
`docs/PHASE0_RESULTS.md`) remain the source of truth for requirements, device facts and results.

## What exists here (`pocketdaw-rs/`)
| Crate | State |
|---|---|
| `usb/` (`pocketdaw-usb`) | **Done and PROVEN ON THE SP (2026-11-08)**: `list` matches; `midi-in` 20 s = 318 reads / 0 errors / 217 events (NOTE ON/OFF ch1, vel 90); `capture 60` = 60000 packets, 0 empty, 0 bad, 15,876,000 bytes = exactly 2,646,000 frames, identical to the Python+C result. 16 host tests pass. `sys` (usbfs structs + ioctl numbers, pinned by tests to values printed from the real `linux/usbdevice_fs.h`), `desc` (UAC1 parser; real FM-1 blob as a test fixture), `midi` (packets/decoder), `dev` (sysfs discovery by VID/PID, MIDI endpoint handle, released on Drop), `iso` (isochronous capture, replaces the C helper), `wav` (WAV I/O + numeric analysis; matches the Python `wavcheck` output to the printed precision). |
| `usb/src/bin/fm1tool.rs` | `list descriptors note pc cc panic midi-in capture wavcheck`. Same output shape as the Python tool. |
| `core/` (`pocketdaw-core`) | **Started, 21 host tests pass** (not on the SP yet). `transport` (frame clock, bars/beats, snap, loop wrap that is sample-exact for any chunk size), `mixer` (Tone/Click/Clip sources, per-track gain/pan(-3 dB law)/mute/solo/peak, master gain + brickwall limiter, output **bit-identical for any chunk size**), `wav` (decode 8/16/24/32/float -> stereo f32, linear resample, 16/24-bit encode), `project` (versioned JSON, takes kept per track, non-destructive clip metadata, safe relative paths only, sanitising loader). Covers brief features 1/2 groundwork (click, loop region), 4 (gain/pan/mute/solo/limiter/meters), 14 (import+SRC basics). `examples/bench.rs` measures render cost; `release/mixbench-aarch64` is the device build. Dev box: 9 tracks = 0.055 ms per 23.2 ms chunk. **SP number not yet measured.** |
| (planned) device app | UI + SDL audio on the SP. Should reuse `hill-core` / `hill-device` from the Rust workspace (see below). |

## Building
```
cd pocketdaw-rs
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
| 3 | Latency calibration | Measurement done and run on the SP (`pdtool latency`, 12.8 ms median); `placed_start` applies the offset. In-app flow TODO. |
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
| 16 | Battery and storage indicators | `device/` crate (`battery`, `storage`, `record_seconds_left`, formatting; 4 tests on a fake sysfs). `pdtool status` **PROVEN ON THE SP**: `battery: 36% Discharging 3561 mV`, `/mnt/sdcard: 100.4 GB free of 233.1 GB = 226h of mono 24-bit recording`. No `current_now` exists on the SP. |

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
`tools-bin/` (`pdtool`, release `pdtool-aarch64`): `record SECS OUT.wav [--note N]` = capture thread -> bounded queue (512 chunks) -> writer thread -> SD card,
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

Done, host-tested, and **verified on the SP (2026-11-08)**: `pdtool drums render afro-cuban-r1 ... --cycles 4` picked the owner's `04. LM Vinyl` kit from `/mnt/sdcard/ROMS/Hill Apps/Samples`, played through `aplay`, owner confirmed audio quality is fine.

- `core/src/drums.rs`: stateless `DrumTrack` (a mixer `Source`). Sections chain back to back; each is a pattern repeated `cycles` times. Hit time is a pure
  function of section start, cycle, step, bpm, swing, so output is bit-identical for chunk sizes 1..9999 (tested). Mirrors HillBeat: step = 1/`div` beat, odd steps
  delayed by `swing*step` (0..0.25), gain `vel/127 * vol * 0.9`, flam grace note 30 ms early at 0.4, per-voice volume up to 4.0, tom pitch by resampling (linear interp).
- `drums/` (`pocketdaw-drums`): the only crate depending on `hill-core` (path `../../rust/hill-core`). `DrumLibrary` finds kits like HillBeat (`library::kit_entries`),
  calls `Preset::plan(kits, 8)`, loads sounds from a `Host`, caches. `DirHost` is a read-only folder host (no SDL). All 267 shipped presets load with built-in sounds (tested).
- `project.json`: new `drums` block (`#[serde(default)]`, so old files load). Sections store the preset **id**, not an index. Values clamped in `sanitize`.
- `pdtool drums list [TEXT]` and `pdtool drums render ID OUT.wav [--cycles N] [--bpm B] [--swing S] [--samples DIR]`: renders through the real `Mixer` + limiter to a 16-bit WAV.

Decisions / limits to know about:
1. A "cycle" is one pass of the pattern (`steps/div` beats), so for 4/4 sixteenths it is one bar; 3/4 and 12/8 presets have their own length. The UI should say "cycles" or compute bars.
2. No choke: HillBeat limits 4 voices per drum and fades the oldest; `DrumTrack` lets every hit ring to the end of its sample (capped at 4 s). Retriggers of a long open hat will overlap. Add a choke only if it is audible.
3. Tempo is the project's constant bpm; the grid does not change mid-song.
4. Once a `DrumTrack` is boxed into a mixer `Track` its vol/mute can't be changed live. The app needs a shared handle (atomics or `Arc<Mutex>`) when it is wired in.
5. `libc::Ioctl` is Linux-only; `usb/src/sys.rs` now uses `req as _` so the crate also compiles on macOS. Type-checked for `aarch64-unknown-linux-gnu`; the musl release binaries were not rebuilt.

Known nit: `pdtool drums list | head` panics with Broken pipe (println! on a closed pipe, panic=abort). Harmless; fix with the next change.
Build without the pip zig: `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=<shim running 'zig cc -target aarch64-linux-musl'> cargo build --release --target aarch64-unknown-linux-musl -p pocketdaw-tools` (Homebrew zig works).
