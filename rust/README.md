# Hill apps, Rust edition

Lightweight Rust rewrites of HillBeat, HillSequencer, HillBand and HillChord. One engine, three ways to run it:

| Target | What you get |
|---|---|
| RG35XXSP (muOS) and R36S (ArkOS) | Four small aarch64 binaries, about 600 KB each, packaged as PortMaster zips |
| Browser / Claude Artifact | One self-contained `hill-apps.html` (about 415 KB) with all four apps |
| Desktop | The same SDL2 binaries, for development |

The Python apps in the sibling folders are untouched. This folder is a separate workspace.

## Layout

```
hill-core/     the engine and all four apps (no I/O, no threads)
  grid.rs        step-grid engine shared by HillBeat / HillSequencer / HillBand
  apps/          the four apps (beat, sequencer, band are small configs; chord is its own)
  mixer.rs       voice pool + resampler (pitch-shift and loop varispeed)
  transport.rs   sample-accurate step clock with swing
  looper.rs      backing-loop player (tempo-synced by resampling)
  library.rs     WAV catalog, multisample grouping
  synth.rs       built-in sounds, so every app works with no samples
  wav.rs  fb.rs  ui.rs  ...
hill-device/   SDL2 runner (video, audio, joystick) + `hillmap` button wizard
hill-web/      wasm C-ABI wrapper + the HTML shell (template.html)
ports/         PortMaster launcher template
build.sh       test | web | device | all
```

The apps implement one trait (`hill_core::App`): feed it button events, ask it for audio and a 640x480 framebuffer. Runners provide a `Host` for sample files and saved state. Audio and the musical clock run together on the audio thread, so timing is sample-accurate.

## What the apps do

Every app has a built-in cheat sheet: hold **FN** and tap **SELECT**.

- **HillBeat**: 4 drum voices, 16 steps, 8 patterns. Swing, tap tempo, per-step velocity, voice mute/volume, a backing loop, save/load.
- **HillSequencer**: 8 pitched tracks, 16 steps, 8 patterns. Per-track transpose, a key/scale overlay (START+L2) that spreads a scale across the tracks, multi-select to give several tracks one instrument.
- **HillBand**: 4 melodic + 4 drum tracks. Chord types (14) and scale modes (11) per melodic track, multi-select, a backing loop.
- **HillChord**: play chords or single notes live. Looper with overdub (stored as an event list, so BPM changes cost nothing), arpeggiator, metronome, favorites.

The backing loop (HillBeat, HillBand) is resampled to follow the app tempo, so pitch moves with tempo. Put the loop's BPM in its file name (`funk_96bpm.wav`) or answer the "how many bars" prompt once.

### Deliberately not here

Compared with the Python apps: all effects (reverb, delay, chorus, bitcrush, wet/dry), the render/disk cache and pre-caching, time-stretching, the chain editor and pattern copy/paste, named sequence manager, metronome (except HillChord), and the sample-library tooling (variant dedup, normalization).

## Samples

Each app finds WAVs under the first of these that exists: `$HILL_SAMPLES`, `/mnt/sdcard/ROMs/Samples`, `/mnt/sdcard/ROMS/Samples`, `/mnt/mmc/ROMS/Samples`, `<app dir>/samples`. The PortMaster launcher also tries a few `/roms` paths (guesses for ArkOS, unverified); set `HILL_SAMPLES` in the launcher if yours differs.

- Folders are scanned recursively; `._*` and other hidden files are skipped.
- A folder with several files of one instrument and different note names (`Piano_C3.wav`, `Piano_C4.wav`, `HoveFlute_midi036_C2.wav`) becomes one multisample instrument. Anything else is listed per file.
- Without any samples the apps use built-in synthesized sounds.
- In the browser, add `.wav` files or a folder from the menu. They are stored in your browser (IndexedDB) and stay on the device.

State is saved next to the executable as `<app>_state.txt` (override with `HILL_DATA`). The device autosaves every 30 s and on quit.

## Buttons (R36S, RG35XXSP)

The apps need 13 inputs: D-pad, A B X Y, L1 R1 L2 R2, SELECT, START, FN. Defaults are the RG35XXSP indices the Python apps use. Other handhelds usually number buttons differently, so run **HillMap** once: it asks you to press each button, then writes `buttons.cfg` next to the executables. It can also use the left stick as a second D-pad. L3/R3 are accepted but no app uses them.

`buttons.cfg` is plain text (`A=3`, `UP=...`, `STICK=1`); `HILL_BUTTONS` points at a different file.

Desktop keys: arrows, `a b x y`, `q w e r` = L1 R1 L2 R2, `o p u` = SELECT START FN, Esc quits.

Quit on device: hold START+SELECT (2 s; HillChord 1.2 s).

## Building

```
./build.sh test      # cargo test --workspace
./build.sh web       # dist/hill-apps.html
./build.sh device    # dist/ports/*.zip
```

`device` needs `cargo-zigbuild`, `pip install ziglang`, the `aarch64-unknown-linux-gnu` rust target, and `curl ar zstd zip`. It downloads the arm64 libSDL2 once for link-time and targets glibc 2.28; the device's own libSDL2 is used at run time. Copy the zips into the PortMaster install flow as with the Python ports, or unzip into the `ports` folder.

Desktop dev run: `cargo run -p hill-device --bin hillband` (needs `libsdl2-dev`). For a headless smoke test: `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy`.

There is also a screenshot tool: `cargo run -p hill-core --example shot -- hillband out.png "a right a start"`.

## Testing

`cargo test --workspace`: unit tests for the clock, mixer, WAV decoder, catalog, looper and theory, plus integration tests that drive each app with button presses and check the audio it renders (step timing, velocity, mute, pattern queueing, looper record/overdub/clear, arpeggiator onsets, state round-trips).

## Known limits

- Not yet run on real hardware. The aarch64 binaries were checked by running them under qemu-user with dummy SDL drivers; audio output and the joystick path are untested on the RG35XXSP and R36S.
- Choosing a multisample instrument reads its files on the UI thread under the app lock, so audio can drop out for a moment on a slow SD card.
- The browser build uses an AudioWorklet. If a sandbox blocks worklet modules it falls back to a main-thread ScriptProcessor (works, but is deprecated and less steady).
- Loop varispeed changes pitch with tempo, by design.
