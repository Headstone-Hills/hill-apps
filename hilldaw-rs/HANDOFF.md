# HillDAW — handoff to a local session

Written 2026-11-08 at the end of a long cloud/SSH session. It is meant to be read cold: it assumes no chat history.
Read this first. The other docs are detail:

| Doc | What it is |
|---|---|
| `hilldaw/docs/BRIEF.md` | The requirements, verbatim from the owner. **Source of truth for scope.** |
| `hilldaw/docs/PHASE0_RESULTS.md` | Hardware proof results from the real SP + FM-1 (numbers). |
| `hilldaw-rs/HANDOFF-RUST.md` | Running status notes, kept while building; this file supersedes its summary. |
| `hilldaw/HANDOFF.md` | Older Python-era notes. Python is now a reference only. |
| `rust/README.md` | The owner's existing Hill apps (Rust workspace). Build recipe for the SDL device binaries. |

---

## 1. The goal in one paragraph

A pocket multitrack recorder/arranger for the **Anbernic RG35XXSP** (muOS) that records audio from an **M-VAVE FM-1 FM synth over
USB-C**, plays tracks back with a **drum track built from the owner's HillBeat presets**, doubles as a **sampler** with the FM-1 as its
keyboard, saves projects, and exports mixdown + stems. **16 extra features** (brief §7) are all required. It must run **fully standalone,
wired only** (no Wi-Fi/Bluetooth/phone/computer needed in the field); SSH/Wi-Fi are development tooling only. One USB device at a time
(FM-1 *or* a USB mic), no hub support.

## 2. Who you are working with, and how

- The owner (Sadie; use **they/them** unless told otherwise) works **from an iPhone over SSH (Termius) into the SP**. They paste terminal output
  or screenshots back. **Give one short command at a time** and say what to look for; pasting several at once once interrupted a 60 s capture.
- Stated preferences: **cited sources and detailed explanations; no offers to write their work for them.** They want honest reporting
  (what is proven on hardware vs only tested on a computer). They said to ignore HillBeat's presets "for now", then pushed the source
  that has them; they will have **a local session find the M-VAVE MIDI CC guide** (see §9).
- Never store or guess the SSH password. The SP's IP is DHCP (was 192.168.1.201).
- A local session can work differently (mount the SD card, scp/rsync directly, build and run on a computer), which removes most of the
  workarounds in §8.

## 3. Repository map

Repo: `Headstone-Hills/hill-apps` (public). **Working branch: `claude/fm1-tracker-rg35xxsp-c20kgw`.**

```
rust/                the owner's Rust workspace, MERGED INTO THIS BRANCH on 2026-11-08 from claude/gracious-faraday-6pjyu5
  hill-core/           engine + apps, no I/O, no threads. App trait, fb (640x480 software UI), grid (drum/step engine),
                       presets (267 HillBeat patterns, data/presets.json), library (kits), wav, synth (built-in sounds)
  hill-device/         SDL2 runner (audio callback + video + joystick), HillMap, FsHost (sample folders)
  hill-web/            browser build (irrelevant here)
hilldaw-rs/        OUR WORK (separate cargo workspace, path-depends on nothing in rust/ yet)
  usb/                 hilldaw-usb: usbfs MIDI, UAC1 descriptor parser, isochronous capture, WAV I/O+analysis, bin `fm1tool`
  core/                hilldaw-core: transport, mixer, effects, sampler, tempo, record session, notes, edits/undo, project model, WAV
  device/              hilldaw-device: battery, storage, project browser
  tools-bin/           hilldaw-tools: bin `hdtool` (record, latency, status)
  release/             prebuilt aarch64 binaries for the SP (fm1tool, hdtool, mixbench)
  build.sh .cargo/ tools/zigcc-aarch64-musl.sh
hilldaw/           Python reference implementation + hardware diagnostics. SUPERSEDED. Do not extend. Keep: tests/fixtures (real FM-1 descriptors).
fm1tracker/          earlier M-VAVE MIDI-tracker brief. SUPERSEDED (has SUPERSEDED.md). Ignore.
hillbeat/ hillchord/ hillsequencer/ hillband/   the owner's older Python apps (the deployed ones are the Rust `-rs` builds).
```

**The owner keeps pushing to `claude/gracious-faraday-6pjyu5`.** Re-merge it periodically (`git fetch && git merge origin/claude/gracious-faraday-6pjyu5`).
Our additions live in new directories so merges are clean. If we change `rust/` files (planned: one small additive `run_with` in `hill-device`, §7),
keep the change minimal and tell the owner so they can take it upstream.

## 4. Hardware and device facts (all measured on the real unit)

**SP:** Allwinner H700 quad A53 ~1.5 GHz, 996 MB RAM (~850 MB free), 640x480, aarch64, muOS 2601.0 JACARANDA, kernel **4.9.170**, Python 3.11.8
(no numpy; `/mnt/sdcard/ports/hillsynth/pylibs` has numpy 2.4.6 + pygame 2.6.1). Userland is **BusyBox** (`tar` has no `-z`; `sh`; `bash` exists).
Present: `curl wget aplay arecord ffmpeg lame flac amixer`. **Absent:** gcc/cc, sox, git. SD card `/mnt/sdcard` is **exfat**, 233 GB (100 GB free at last look);
exfat may not allow executing files, so run binaries from `/root` (ext4).
Sound cards: `audiocodec` (playback only), `ahubdam`, `ahubhdmi`. **No audio capture device at all** (no mic/line-in).
**No kernel USB audio/MIDI driver** (`CONFIG_SND_USB_AUDIO` off): everything USB goes through usbfs from userspace.
Battery: `/sys/class/power_supply/axp2202-battery` (`capacity`, `status`, `voltage_now`; **no `current_now`**).
USB role: `/sys/devices/platform/soc/usbc0/otg_role` reads `usb_host`. PMIC regulator `axp2202-drivevbus` reads `disabled` while the FM-1 is attached and working
(so it is not what powers it; **parked, do not write to it**). The SP does power the FM-1 over USB; whether it can be stopped, and whether the FM-1 enumerates without VBUS, is untested.
Buttons (`/mnt/sdcard/ports/hill-buttons.cfg`, written by the owner's HillMap): A=3 B=4 X=6 Y=5 L1=7 R1=8 L2=12 R2=13 SELECT=9 START=10 FN=11 L3=14, D-pad = hat.
Quit convention in all Hill apps: hold START+SELECT (~1.2-2 s). FN+SELECT = cheat sheet. Samples root candidates (first that exists):
`$HILL_SAMPLES`, `/mnt/sdcard/ROMS/Hill Apps/Samples`, `/mnt/sdcard/ROMs/Samples`, `/mnt/sdcard/ROMS/Samples`, `/mnt/mmc/ROMS/Samples`, `<app dir>/samples`.
Drum kits for presets live in `Samples/Drums` (`presets.rs` `STYLE_KITS` names them: lm vinyl, ampeg 808, boogie classic, 909 tube, ...).
Deployed Hill apps: `/mnt/sdcard/ports/hill*-rs/` (hillbeat-rs, hillchord-rs, hillsequencer-rs, hillband-rs, hillmap-rs) + `hillsynth`; launchers in `ROMS/Ports` and a `Hill Apps` folder.

**FM-1:** USB `4c4a:c755`, composite UAC1 audio + USB-MIDI, USB 1.1 full-speed (OHCI). Real descriptors are a test fixture (`hilldaw-rs/usb/tests/fixtures/fm1_descriptors.bin`, 334 bytes).
- **Capture** = interface 2, alt 1, isochronous IN endpoint `0x83`, async, 270-byte packets/1 ms, **PCM 2 ch, 24-bit, 44100 Hz** (264 KB/s). Stereo is a mirror (L-R differ by ~-68 dBFS).
  Idle = **exact digital silence**. Level follows the FM-1's volume knob (peaks seen: -13 to -28 dBFS).
- Playback = interface 1, alt 1, iso OUT `0x03`, adaptive (unused).
- **MIDI** = interface 4, bulk OUT `0x04`, bulk IN `0x84`. **MIDI out proven. MIDI in proven**: the FM-1 sends NOTE ON/OFF on **channel 1, velocity always 90** (217 events in 20 s; overlapping notes, offs out of order).
  It is **not known** whether those came from the keys or the FM-1's internal sequencer, nor whether the keys are velocity sensitive. No CC/program messages were seen.
- Single-timbre assumption (unverified): treat it as monotimbral; 12 shared voices.
- **MIDI-write -> first audible sample latency: median 12.8 ms (563 frames), min 7.3, max 14.0** (8 runs). Spread looks FM-1-side.
- A usbfs interface can be claimed **once per process fd**: opening MIDI twice fails (a bug found in the first latency tool).

## 5. What is PROVEN on the real SP vs only tested on a computer

**Proven on the SP (numbers in `hilldaw/docs/PHASE0_RESULTS.md` and `HANDOFF-RUST.md`):**

| Item | Result |
|---|---|
| Isochronous capture, no kernel driver (Python+C helper, then Rust) | 60 s = 60,000 packets, 0 empty, 0 bad, **2,646,000 frames exactly**; identical in both implementations |
| MIDI in and out | both work; see §4 |
| Record 60 s to the SD card while capturing (`hdtool record`) | 0 overruns, captured = written, worst single write 7.27 ms |
| MIDI->audio latency (`hdtool latency`) | 12.8 ms median |
| Audio output via `aplay` pipe and via pygame channel queue (Python) | 20 s each, 0 underruns, **owner confirmed they sounded clean** |
| Mixer cost (`mixbench`): 8 clip tracks with LP+HP+delay+reverb send + click/tone + 12-voice sampler | **3.76 ms avg, 6.03 ms worst per 23.2 ms chunk (~16% of one core)** |
| Battery and storage readout (`hdtool status`) | `36% Discharging 3561 mV`; `100.4 GB free = 226 h` mono 24-bit |

**Host-tested only (95 tests pass in `hilldaw-rs`; none of this has run on the SP):** transport and loop wrap; mixer (bit-identical for any chunk size); effects;
sampler; tempo; record-session logic; note recorder; edits/undo; project model and browser; WAV import/export.
**Not started:** any UI/screen, the app itself, SDL audio output from Rust on the SP, drum track, arrangement, export, FM-1 control/patch browser, FM-1->sampler live wiring.

## 6. Requirements checklist (brief §5 and §7)

| # | Feature | State |
|---|---|---|
| 1 | Click, count-in, time signature, tap tempo | `Click`, `TapTempo`, count-in in `session.rs` done (logic only) |
| 2 | Loop region + punch in/out | sample-exact loop wrap; punch in `session.rs`; each loop pass = its own take. App wiring TODO |
| 3 | Latency calibration | measured on the SP; applying the offset (`placed_start`, capture shift in `session.rs`) done; in-app flow TODO |
| 4 | Gain/pan/mute/solo, master limiter, meters | done (`mixer.rs`, -3 dB pan law, brickwall limiter) |
| 5 | Trim, fades, normalize, reverse | non-destructive `TrimmedClip`; normalize needs the file peak from loaded audio |
| 6 | Undo/redo | done (`edit.rs`: every `Edit` returns its inverse; atomic `Batch`; depth 100) |
| 7 | Snap to grid, slice | `Transport::snap`, `split`, `slice_on_grid` done |
| 8 | Filter, delay, small reverb (shared send) | done (`effects.rs`) |
| 9 | Sample pitch-shift by resampling across keyboard | done (`sampler.rs`, stateless, seekable, Hermite, loop, polyphony cap) |
| 10 | Arrangement view with sections that loop/chain, using drum presets | **TODO** (needs the drum track; plan in §7) |
| 11 | MIDI note recording from FM-1 keys | `notes.rs` + project notes + undo done; wiring TODO |
| 12 | FM-1 program/CC control + patch browser | MIDI out proven; **needs the CC guide (§9)** |
| 13 | BPM detection, fit to tempo | done; detector returns ranked candidates because **half/double time is genuinely ambiguous** (UI must offer x2 and /2) |
| 14 | WAV import with sample-rate conversion | done (linear resample) |
| 15 | Project browser (date, duration, delete/duplicate) | logic done in `device/projects.rs`; UI TODO |
| 16 | Battery and storage indicators | readouts proven on the SP; UI TODO |

Also required (brief §5): 8 mono tracks with take history, recording with click/count-in/overdub/loop-record, FM-1 monitoring option, drum track (HillBeat architecture **with all
presets**, selectable per section, chaining, swing, per-drum volume), sampler (record or load; USB mic as second source; one device at a time with clean hot-swap), project folders
(`project.json` + `audio/`, versioned, relative paths, autosave, recovery), mixdown + stems export (16/24-bit, limiter, dither option; ffmpeg/lame/flac exist on the SP for MP3/FLAC).

## 7. Architecture and decisions (and why)

1. **Language: Rust** (owner's request, and the right call: no GIL in the audio path, one static binary, no pylibs). Python remains only as a reference/diagnostic.
2. **Design rule that everything follows: output must not depend on how the host chunks the work.** One absolute frame counter positions every source; all DSP
   state advances sample by sample; sources are stateless functions of the frame where possible (sampler, clips, click). This makes seek, loop and offline export trivially correct,
   and it is tested (bit-identical across chunk sizes 1..20000).
3. **Layering:** `hilldaw-core` is pure (no I/O, no threads, no device). USB lives in `hilldaw-usb`. Anything touching sysfs/files is in `hilldaw-device`. The audio callback is the single mixing point.
4. **UI/runner: reuse the owner's `hill-core` + `hill-device`.** The DAW should implement `hill_core::App` (`event`, `audio`, `render`, `tick`, `set_rate`, `wants_quit`, `save`) and run in their SDL runner
   (`hill_device::run(name)`; the audio callback locks `Arc<Mutex<Box<dyn App>>>` and calls `app.audio(out)`; the UI thread locks for `event/tick/render` ~60 Hz).
   - `hill_device::run` currently builds the app via `apps::create(name, host)`. **Planned minimal change:** add `run_with(name, make)` and make `run` call it (additive, 6 lines).
   - **Caveat to measure:** `render()` holds the same mutex the audio callback needs. If a 640x480 software render takes several ms on the A53 the audio thread waits. Keep `render` cheap or snapshot state.
   - Capture thread, disk writer, USB, battery polling live **outside** `hill-core` (it must stay I/O-free for the web build) and talk to the App through channels/atomics.
5. **Drum track (next big task; design settled, nothing written):** reuse the owner's presets and kit logic rather than the Python HillBeat (which has no presets):
   - API to reuse: `hill_core::presets::builtin()` -> `Presets{styles, patterns}`; `Preset{id,name,style,kind,steps,div,tracks}`; `Preset::plan(&kits, voices) -> Plan{kit, voices:[PlanVoice{rows,sound}], cells[voice][step] (velocity | FLAM_BIT 0x80), pitch[voice][step] (semitones, toms)}`;
     `Sound::{File(path), Builtin(name)}`; `presets::Kit::new(name, paths)`; `library::kit_entries(&paths)`, `library::load_entry(host, entry, budget)`; `synth::builtin_by_name`; `wav::Sample::frame(i)`; `Host` trait (`list_samples`, `read_sample`).
   - Constants to honour: `NORMAL_VEL 96`, `ACCENT_VEL 127`, flam grace note `30 ms` early at `0.4` level, hit gain `vel/127 * voice_vol * 0.9`, swing delays **odd steps** by `swing * step_len` (0..0.25), voices = 8, `steps <= grid::STEPS (16)`, `div` 4 (sixteenths) or 3 (triplets / 12-8).
   - Plan: a **stateless `DrumTrack` implementing `hilldaw_core::mixer::Source`**: hit time is a pure function of section start + step; it scans back ~2 s of pattern repeats for ringing tails; per-voice volume/mute (vol up to 4.0, the newest HillBeat commit adds a soft-knee limiter; our master limiter covers it);
     sections = `{name, bars, repeats, preset id}` chained on the timeline. Put the section list in `project.json` (`#[serde(default)]`, preset **ids** as strings, not indexes). Constant tempo per project (tempo automation is out of scope).
   - Test on the host with built-in sounds and synthetic kit WAVs; then a `hdtool drums render <preset> --bars N out.wav` and `aplay out.wav` on the SP lets the owner **hear** their kits through the DAW path.
6. **Recording path:** USB iso capture thread -> `Downmixer` (24-bit stereo -> mono) -> bounded queue (512 chunks) -> writer thread -> `StreamingWav` (header rewritten ~1/s, so a crash loses <= ~1 s) -> `<project>/audio/*.wav`.
   `RecordSession` turns transport spans into capture `Segment`s (count-in, punch, one take per loop pass, capture shifted by latency).
7. **Two clocks (unmeasured risk):** FM-1 audio and the SP codec run on different crystals. Plan: store `captured_frames` vs transport frames elapsed per take (`drift_ratio`) and, only if it proves audible, play clips at that ratio (non-destructive). Measure on a multi-minute take first.
8. **Output:** SDL2 audio callback (what `hill-device` does) at 44.1 kHz stereo, 1024-frame buffer. The Python tests proved the SP's ALSA path (aplay ~146 ms, pygame ~58 ms estimated). **Rust output latency is unmeasured.**

## 8. Build, test, deploy

```
cd hilldaw-rs
cargo test --workspace            # 95 tests, ~4 s
cargo clippy --workspace --all-targets   # currently clean
./build.sh device                 # static aarch64 musl binary -> dist/fm1tool  (rustup target add aarch64-unknown-linux-musl; pip install ziglang)
cargo build --release --target aarch64-unknown-linux-musl -p hilldaw-tools   # hdtool
cargo run --release -p hilldaw-core --example bench                           # mixer cost on this machine
```
- Cross-linking uses **Zig as the linker** through `tools/zigcc-aarch64-musl.sh`; `.cargo/config.toml` must keep `-C link-self-contained=no` (otherwise a duplicate `_start`).
- The **SDL app** (not built yet) must follow `rust/README.md`: `cargo-zigbuild`, target `aarch64-unknown-linux-gnu.2.28` (glibc), arm64 libSDL2 fetched from Ubuntu ports for link time only; the SP's own libSDL2 is used at run time.
  The Hill launchers run with `LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0` (pygame's bundled SDL has no working display driver on the SP).
- A cloud session reached crates.io and installed rustup targets fine. It had no SP access: the owner ran every hardware test by hand.
- **Getting binaries onto the SP without a computer** (what we did): commit the binary under `hilldaw-rs/release/`, push, then on the SP:
  `wget -O /root/NAME https://raw.githubusercontent.com/headstone-hills/hill-apps/<FULL-COMMIT-SHA>/hilldaw-rs/release/NAME` then `chmod +x /root/NAME`.
  **Always pin the commit SHA**: raw URLs on a branch name are cached ~5 minutes and served a stale file once. Check the printed `Length:` matches. BusyBox `tar` cannot read `.tar.gz`; use plain `.tar`.
  With a local session you can `scp` instead.

Hardware test tools (expected-good output in brackets):
- `fm1tool list | descriptors [--save f] | note N | pc N | cc N V | panic | midi-in --seconds S | capture SECS out.wav [--note N] | wavcheck f.wav`
  [`capture 60`: `packets=60000 empty=0 bad_status=0 bytes=15876000`, `2646000 frames`]. Python twin: `hilldaw/fm1tool.py` + `phase0.sh`/`check_env.sh` (+ C helper `helper/isocap.c`).
- `hdtool record SECS out.wav [--note N] [--synthetic]` [`RESULT: CLEAN`, `overruns=0`], `hdtool latency [--runs N]` [median ~13 ms], `hdtool status [path]`.
- `mixbench` [single-digit ms per 23.2 ms chunk].

## 9. The MIDI CC guide (assigned to the local session)

The patch/CC browser (feature 12) needs the **official M-VAVE "FM-1 MIDI CC Control Parameter Guide"**. No session has seen it; **do not invent CC numbers.** Leads (unverified, from web searches):
- M-VAVE FM-1 **firmware V15** notes mention MIDI compatibility work (note-on velocity 0 treated as note-off) and a new CC guide: <https://www.matrixsynth.com/2026/07/m-vave-fm-1-v15-firmware-update-new.html>.
- The official manual PDF is reportedly linked from this thread: <https://www.elektronauts.com/t/m-vave-fm-1/252170?page=2> (forum users disagree on whether CC works over **USB** MIDI; one review says CC exists only for effects and is not in the manual).
- A third-party "Expander" adds MIDI out/sync: <https://www.matrixsynth.com/2026/10/expander-for-m-vave-fm-1-firmware.html> (earlier I wrongly inferred the base unit has no MIDI out from a bug of mine; the base unit does send notes over USB).
**After finding the guide, verify on the real unit** with `fm1tool cc N V` while listening, and `fm1tool pc N` for program changes (also check which firmware version is installed).
Then build the browser on `MidiDevice::write` + `midi::cc/program_change`; map named parameters in a data file, not code.

## 10. Prioritised next steps for the local session

1. **Drum track + presets** (§7.5). Make `hilldaw-rs` depend on `../rust/hill-core` by path, write the `DrumTrack` source + section/arrangement model + tests, add `hdtool drums render`, have the owner `aplay` a render on the SP.
2. **App skeleton as a `hill_core::App`** + `run_with` in `hill-device`; first screens: Mixer/transport (8 strips, meters, mute/solo/arm, BPM, bar:beat, FM-1 connected indicator, battery, storage, record state). Match the Hill apps' controls
   (A edit/confirm, B back/option, Y shift, X secondary, START play/stop, SELECT record, L1/R1 views, L2/R2 nudge/zoom, START+SELECT hold quit, FN+SELECT help; read `hill-buttons.cfg`). Confirm the final mapping with the owner.
3. **Rust SDL audio on the SP:** measure real output latency and underruns with the DAW mixer in the callback (the Python numbers do not carry over). Check the render-vs-audio mutex contention (§7.4).
4. **Wire recording:** capture thread -> segments -> take files; count-in click; punch; loop takes; latency offset (default 563 frames for MIDI-driven alignment); monitoring option. Then **measure clock drift** over 5+ minutes.
5. **Wire FM-1 keys:** MIDI-in thread timestamped against the transport -> `NoteRecorder` -> sampler track / replay as MIDI; mute-the-FM-1-local-sound question (find a CC or silent preset; unverified).
6. **Sampler UI:** load WAV (import + SRC), record a sample, trim/normalize/loop points, root note, keyboard range.
7. **Export:** offline mixdown + stems with limiter and dither; MP3/FLAC via `ffmpeg`/`lame`/`flac` on the SP if present at run time. Write to a known folder and show the path.
8. **Project handling:** autosave + recovery file, save-as, project browser UI, safe delete/duplicate (logic done, tested for path safety).
9. **FM-1 control + patch browser** after the CC guide is in hand. USB-mic input (UAC1 first; the descriptor parser is generic; UAC2 needs clock-source handling). Hot-swap: flush take, release interface, rescan.
10. Polish: help overlay, themes consistent with the Hill apps, an on-screen BPM x2 and /2 for the detector, diagnostics screen (underruns, jitter), tests for the App via the `hill-core` test style (see `rust/hill-core/tests`).

## 11. Lessons learned (don't repeat)

- **Python `fcntl.ioctl(fd, req, bytes)` returns a copy of the buffer, not the ioctl result.** It made the first MIDI-in test falsely report "nothing received", which led to a wrong conclusion about the FM-1 (retracted). Rust's `libc::ioctl` returns the int directly; keep it that way.
- Test arithmetic mistakes bit me repeatedly (expected tap tempo, drift 50 ppm, etc.). When a new test fails, **look at the actual values** before changing code or the test.
- The undo/redo round-trip test caught a real bug (undoing `AddTake` could not be redone). Keep round-trip tests for every new `Edit`.
- Tempo from audio is octave-ambiguous: the true tempo was always among the top candidates but not always the top pick. Never present a single BPM as certain.
- `include_str!` presets and `OnceLock` caches are fine on device; the browser build shares the code, so keep `hill-core` free of I/O and threads.
- Never write to `otg_role`, `usb_*` files or the VBUS regulator without a plan: it can drop the USB host and the FM-1 with it.
- Do not invent drum presets, CC numbers, or FM-1 behaviour. Say "unknown" and add a test tool.
- Stage commits per milestone and push; the owner reads the repo, not the chat.

## 12. Open questions / unknowns

- Keys or sequencer? Velocity sensitivity of the FM-1 keys? Can the FM-1's own sound be muted while sampling? Is it monotimbral? Which firmware is installed?
- Does CC control work over USB MIDI on their unit, and which parameters?
- Output latency and underruns of the Rust SDL path on the SP; whether four cores handle capture + audio + UI + disk concurrently under load; render/audio lock contention.
- FM-1 vs SP clock drift over minutes.
- Can the SP stop supplying power to the FM-1, and does the FM-1 still enumerate without VBUS?
- Preferred export sample rate/bit depth (defaulting to 44.1 kHz, 24-bit with a 16-bit option).
- Which USB mic/adapter the owner will carry (UAC1 preferred).
- Where the owner wants the project folders (default plan: under `/mnt/sdcard/ports/hilldaw/projects` or a `ROMS/...` path; ask).

## 13. Resuming locally (copy/paste)

```
git clone https://github.com/Headstone-Hills/hill-apps && cd hill-apps
git checkout claude/fm1-tracker-rg35xxsp-c20kgw
git fetch origin && git merge origin/claude/gracious-faraday-6pjyu5      # pick up new owner commits to the Rust workspace
cd hilldaw-rs && cargo test --workspace                                # expect 95 passed
(cd ../rust && cargo test -p hill-core)                                  # the owner's engine/presets: 126 passed (80+12+25+9) on 2026-11-08
python3 ../hilldaw/fm1tool.py capture 1 /tmp/t.wav --mock              # Python pipeline demo, no hardware
```
Origin session (for reference): https://claude.ai/code/session_01411JjXBtEyg1roSaSCqt5s
