# PocketDAW — session handoff

Written so a **fresh local session** (or a person) can resume with no chat history.
Source of truth for requirements: [`docs/BRIEF.md`](docs/BRIEF.md) (verbatim). This file records
what has been done, what was decided after the brief, and what is still unproven.

- Repo: `headstone-hills/hill-apps`, branch `claude/fm1-tracker-rg35xxsp-c20kgw`
- App dir: `pocketdaw/` (new). `fm1tracker/` is the **superseded** M8-style MIDI tracker; leave it,
  do not extend it. `pocketdaw/fm1_usb.py` started as a copy of its USB layer.
- Owner preferences: wants cited sources and detailed explanations; does **not** want offers to
  write their work for them. Pronouns not stated: use they/them.

## Decisions made after the brief
| Decision | By | Notes |
|---|---|---|
| Ignore HillBeat's "built-in pattern presets" for now; add later | user | **None exist in the repo.** `hillbeat/state.py` creates 8 empty patterns; `grep -i preset` finds nothing. Presets, if they exist, live in the user's on-device `hillbeat_state.json` (gitignored) or an unpushed copy. Ask the user for it; do not invent presets. |
| Drum track renders inside the DAW's single mix callback | Claude (proposed, not yet challenged) | HillBeat fires `pygame.mixer` channels from a thread, which can't be sample-accurate against recorded audio. Reuse its data model (4 voices x 16 steps x 8 patterns, velocity, swing, chain), sample library layout and button map; not its playback path. |
| C helper for isochronous capture, cross-compiled with Zig | Claude | `pip install ziglang`; `zig cc -target aarch64-linux-musl -static` verified to produce a static aarch64 ELF here. |
| Work is SSH-driven; session will move to the user's computer later | user | Everything needed is in the repo; commit often. |

## Status
Legend: DONE+TESTED (unit tests / host) · WRITTEN (compiles, **not run on hardware**) · TODO

| Item | State |
|---|---|
| USB-MIDI out/in layer, discovery by VID/PID, reconnect (`fm1_usb.py`) | Discovery + sysfs endpoint lookup PROVEN on hardware (found `/dev/bus/usb/008/002`, ep 0x04/0x84). MIDI out PROVEN (middle C played during capture). MIDI **in** PROVEN after fixing an ioctl-return bug: FM-1 sends NOTE ON/OFF on ch1 over ep 0x84 (velocity always 90 in the test). |
| UAC1 descriptor parser (`usb_desc.py`) | PROVEN on the real device: parsed output matches the brief exactly. Real 334-byte blob is now a fixture (`tests/fixtures/fm1_descriptors.bin`, `RealDeviceTests`). |
| Iso capture helper (`helper/isocap.c`, `build_helper.sh`) | PROVEN on hardware (`docs/PHASE0_RESULTS.md`): 60 s capture, 60,000 packets, 0 bad/empty, exactly 2,646,000 frames. Not yet tested under UI/mixer load. |
| `fm1tool.py` (`list`, `descriptors`, `note`, `midi-in`, `capture`, `wavcheck`) | WRITTEN; `capture --mock`, `wavcheck`, `list` verified off-device. `latency-test` from the brief **not written yet**. |
| `wavcheck.py` numeric analysis (peak/RMS/silence/L-R identical) | DONE+TESTED |
| `deploy.sh` (tar-over-ssh deploy, `phase0` runner) | WRITTEN, never run |
| Mixer core (`mixer.py`), output backends (`audio_out.py`: aplay pipe, pygame queue, offline file), `audio_test.py/.sh` | WRITTEN, 30 host tests pass (chunk-size independence, sample-exact placement). **Not run on the SP**; backend choice pending the device test. |
| Everything else in brief sections 5-9 | TODO |

Test command: `cd pocketdaw && python3 -m unittest discover -s tests` (stdlib only; 9 tests).

## Device survey (check_env.sh, 2026-11-08)
- **numpy 2.4.6 + pygame 2.6.1 (SDL 2.28.5) are importable** from `/mnt/sdcard/ports/hillsynth/pylibs` (also `ps-client/pylibs`: pygame only) with
  `LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0`. System python has neither. `audio_test.sh` picks the first pylibs with numpy.
- Tools present: `aplay arecord ffmpeg lame flac amixer`. **No gcc/cc/sox.** (So MP3/FLAC export is feasible via ffmpeg/lame/flac; build C helpers off-device.)
- Sound cards: audiocodec (pcm 00-00, playback only), ahubdam (no pcm), ahubhdmi (pcm 02-00 playback). **No capture PCM: the SP has no audio input.**
- 996 MB RAM total, ~850 MB available. SD: `/mnt/sdcard` exfat 233 GB, 100 GB free.
- **The user's deployed Hill apps are Rust ports**: `/mnt/sdcard/ports/{hillbeat-rs,hillchord-rs,hillsequencer-rs,hillband-rs,hillmap-rs,hillsynth}`; there is no
  `ports/hillbeat`. The Python HillBeat in this repo is NOT what runs on the device. HillBeat's drum presets are probably in `hillbeat-rs` (unverified).
  There is also `/mnt/sdcard/ports/hill-buttons.cfg` (shared Hill button config; not yet read). ROMS folders include `Hill Apps` and `Ports`.
- **Audio test results (20 s each, SP):** aplay 0 underruns, est. latency ~146 ms; pygame queue 0 underruns, est. ~58 ms. Both stayed locked to the sound
  card clock (constant ~0.1 s / ~0.04 s lead of rendered audio over wall time, no growth). Render cost ~2 ms avg / 7 ms max per 23 ms chunk for click+tone.
  The aplay run printed `RESULT: PROBLEMS` only because aplay's start-up banner was logged as an error (fixed). **Provisional choice: pygame queue as the primary
  backend (lower latency, no subprocess, same path as the Hill apps), aplay kept as fallback.** **User confirmed (2026-11-08): both backends sounded good, no audible issues.** The Python output backends are superseded by the Rust direction (see `../pocketdaw-rs/HANDOFF-RUST.md`); the Rust app should use the SDL2 audio callback like the Hill Rust apps. These results still validate the SP's ALSA path: 44.1 kHz stereo s16, 1024-frame periods, ~2 ms render per chunk for a trivial mix.

## Drum presets: where they are (and aren't)
Searched every remote branch of `Headstone-Hills/hill-apps` (main, friendly-thompson, gracious-faraday, lucid-euler, relaxed-cerf): **no drum-pattern presets anywhere.**
`gracious-faraday-6pjyu5` has the Rust workspace (`rust/hill-core`, `hill-device`), but its `apps/beat.rs` is a 38-line wrapper and the only presets are HillSynth's
(`subtractive.rs::factory_presets`). The deployed `hillbeat-rs` (SP, built Oct 26) is newer than the repo and has presets compiled in
(`hillbeat-rs.bak-before-presets`, state `.bak-before-8-tracks`). **Needed from the user's SP**: `cat hillbeat-rs/README.md`,
`cat hillbeat-rs/hillbeat_state.txt.bak-before-8-tracks`, and `strings hillbeat-rs/hillbeat-rs | grep -i -B3 -A30 preset`. Do not invent presets.

## Phase 0 result (2026-11-08): capture WORKS
See [`docs/PHASE0_RESULTS.md`](docs/PHASE0_RESULTS.md). MIDI IN now proven too. Remaining Phase 0 items: (60 s soak now done); then proceed to brief section 9 step 2 onward. The numbered "IMMEDIATE NEXT STEP" below is now historical.

## Device facts learned from the user's SSH session (iPhone/Termius, root on the SP)
- Tools on the SP: `curl`, `wget`, `tar`, **no `git`**. Python **3.11.8**. **numpy not installed** (vendor into `pylibs/` like the Hill apps).
- User's workflow is iPhone-only: `deploy.sh` (needs a computer with the repo) is not usable for them. Use the tarball + `phase0.sh` route below.
- SD card `/mnt/sdcard` is exfat: exec bit / `noexec` is uncertain, so `fm1tool.py` copies `isocap` to the temp dir before running it.
- Power: PMIC regulator `regulator.20 axp2202-drivevbus` (state `disabled` when read, FM-1 attachment state unknown) is the USB VBUS supply;
  `/sys/devices/platform/soc/usbc0` exists. `dmesg | grep -i -E "vbus|otg"` printed nothing. Open question: can the SP stop powering the FM-1,
  and does the FM-1 still enumerate without VBUS? `phase0.sh` records the regulator state with the FM-1 plugged in. Do not write to the regulator
  until we know what else it feeds.

## IMMEDIATE NEXT STEP — Phase 0, needs the real device

**On the SP itself (no computer):** upload `pocketdaw-phase0.tar` to `/root/` with the Termius SFTP browser, then:
```
mkdir -p /mnt/sdcard/ports/pocketdaw && tar -xf /root/pocketdaw-phase0.tar -C /mnt/sdcard/ports/pocketdaw
cd /mnt/sdcard/ports/pocketdaw && bash phase0.sh
```
(Rebuild the tarball with `bash make_tarball.sh`.)

**Or fetch it on the SP (if the repo is public; the SP has wget):**
```
wget -O /root/pocketdaw-phase0.tar https://raw.githubusercontent.com/headstone-hills/hill-apps/claude/fm1-tracker-rg35xxsp-c20kgw/pocketdaw/release/pocketdaw-phase0.tar
```
`release/pocketdaw-phase0.tar` is a committed prebuilt bundle (includes the aarch64 `isocap` binary); regenerate with `make_tarball.sh release/pocketdaw-phase0.tar`.

**From a computer** (alternative):
Cannot be done from a cloud session (no SP access). From a machine that can reach the SP:
```
cd pocketdaw
pip install ziglang                      # once, for the aarch64 build
echo 'POCKETDAW_HOST=<sp ip>' > .deploy.env     # IP is DHCP; password is never stored
bash deploy.sh phase0                    # builds helper, deploys, runs 4 checks, prints a report
scp root@<sp ip>:/mnt/sdcard/ports/pocketdaw/{fm1_descriptors.bin,phase0_capture.wav} .
```
What each step answers (all are open questions in the brief, section 11):
1. `list` / `descriptors`: do real descriptors match the brief (iface 2 alt 1, ep 0x83, 270 B, 2ch/24-bit/44100)?
2. `midi-in` (press keys and turn knobs): does the FM-1 transmit MIDI over USB at all?
3. `capture 4 --note 60`: does isochronous capture work on OHCI/kernel 4.9.170, and does the stream
   carry synth output? You should hear middle C from the FM-1; report must say `SIGNAL PRESENT`,
   byte count near 100% of nominal (264 KB/s), `bad_status=0`.
4. idle capture: noise floor, and whether L == R (decides the mono downmix).
Interpreting failures: `claim interface: Device or resource busy` = something holds it; `set alt
setting` error = alt number wrong; `bad_status` high or bytes far under 100% = scheduling/dropped
frames (try `--urbs 16 --frames 32`); all-zero audio = stream open but not carrying the synth (check
FM-1 volume, feature-unit mute in the audio-control interface). If capture cannot work, **stop and
report with evidence** and offer the brief's fallbacks (a)-(c); do not silently change scope.

## Layout
```
pocketdaw/
  fm1_usb.py        MIDI transport + discovery (stdlib). Transports: Usb/Mock/Console.
  usb_desc.py       UAC1 descriptor parser, pick_capture()
  wavcheck.py       numeric audio analysis + WAV helpers + synth_tone()
  fm1tool.py        diagnostics CLI
  helper/isocap.c   iso IN capture; build with build_helper.sh -> dist/isocap (aarch64 static)
  deploy.sh         SSH deploy / phase0 runner
  tests/            unittest
  docs/BRIEF.md     the requirements, verbatim
```
Hill-app conventions to follow when the UI starts (all read from `hillsequencer/`, `hillchord/`):
- Launcher: PortMaster-style `.sh` in `ROMS/Ports/`, code in `ports/<name>/`; sets
  `LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0`, `PYTHONPATH=$GAMEDIR:$GAMEDIR/pylibs`, `SDL_AUDIODRIVER=alsa`;
  deps vendored into `pylibs/` by `install_deps.sh` (pygame, numpy) on first run (needs wifi once).
- Input: pygame joystick indices, A3 B4 Y5 X6 L1 7 R1 8 Select 9 Start 10 Fn 11 L2 12 R2 13,
  D-pad as hat; **no gptokeyb**; quit = Start+Select hold ~1.2 s; keyboard fallback for desktop dev.
- Do not break the muOS menu button; clean exit must release USB.

## Known gaps / risks to keep in mind
- numpy is **not** confirmed on the device; the Hill apps vendor it into `pylibs/`, so it probably can
  be, but the DSP-heavy features (8, 9, 13) depend on it or on moving DSP into C.
- Python `time.sleep` + GIL jitter was ~3 us avg on the cloud box with a mock transport (earlier tracker
  work); that is **not** representative of the A53. Measure on the SP.
- Iso capture while Python also draws pygame frames is untested; helper-as-subprocess is the isolation plan.
- FM-1 behaviour that is unverified: key presses over MIDI IN, local-sound mute, whether bank/program
  change works on the user's firmware, whether the unit is multitimbral (probably not).
- UAC2 / generic mics: the parser flags nothing for UAC2 yet; build after FM-1 capture is proven.
- Brief section 7 is large (16 features). Track each in a checklist as built and report simplifications.

## Resuming locally
```
git clone https://github.com/headstone-hills/hill-apps && cd hill-apps
git checkout claude/fm1-tracker-rg35xxsp-c20kgw
cd pocketdaw && python3 -m unittest discover -s tests
python3 fm1tool.py capture 1 /tmp/t.wav --mock      # full pipeline off-device
./build_helper.sh                                    # needs a C compiler; aarch64 needs `pip install ziglang`
```
