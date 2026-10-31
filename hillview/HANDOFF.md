# HillView: handoff to a local session

Written at the end of a cloud session. Everything here describes the code at commit `ab745c6` (this document is the commit after it) on branch
`claude/relaxed-cerf-vswxci` (remote `https://github.com/Headstone-Hills/hill-apps`). Read section 1 first and run it before anything else: the most
important unknowns are things that could only be checked on your device.

## 0. How to work with this user

- They like **cited sources and detailed explanations**. They do **not** like offers to write their work for them: do the task, or ask a
  real question, but do not close with "want me to write X?".
- Report outcomes faithfully. If something failed or was skipped, say so with the output. "Verified" in this repo means a specific check ran; section 4
  lists exactly what ran and what did not.
- Do not open a pull request unless asked. Develop on `claude/relaxed-cerf-vswxci`. Commits in this branch carry `Co-Authored-By` / `Claude-Session`
  trailers added by the cloud environment; keep your own environment's convention.
- Treat this document as claims to check, not facts. Run the tests (section 1.4) before relying on anything.

## 1. Launch the current build for testing

Target device assumed: **Apple Silicon Mac** (the user asked for a "mac arm" build). On Linux the same steps work; skip the Homebrew line.

### 1.1 Get the code and dependencies

```bash
git clone https://github.com/Headstone-Hills/hill-apps.git     # or: cd hill-apps && git fetch origin
cd hill-apps
git checkout claude/relaxed-cerf-vswxci && git pull
cd hillview

brew install ffmpeg                  # the app shells out to ffmpeg and ffprobe
python3 -m pip install -r requirements.txt pytest      # requirements.txt is just numpy
```

Python 3.13 was used in the cloud; anything recent with numpy should do. ffmpeg 6.1 was used.

### 1.2 Launch the desktop/standalone app

```bash
cd hillview            # you must be in the folder that CONTAINS the `hillview/` package
python3 -m hillview app          # opens http://127.0.0.1:8765/ ; add --no-browser --port N to change
```

**First line it prints is the most important output of the whole session:**

| It prints | Meaning |
|---|---|
| `native core: /…/prebuilt/macos-arm64/libhillview.dylib` | The prebuilt arm64 library loaded. This is the first time anyone has seen that happen on real hardware. |
| `native core: unavailable (…reason…); using the slower numpy path` | The dylib did not load; the reason is in the parentheses. See 1.5. The app still works, ~10-30x slower. |

Then, in the browser: drop in a short video (or use the path box), move a control and watch the preview change, pick a format, Render.
Exports land in `~/HillView/output/` (override with `HILLVIEW_HOME`).

Command-line equivalents (same engine):

```bash
python3 -m hillview formats                                              # export formats your ffmpeg can encode
python3 -m hillview render in.mov out.mp4 -c presets/glitch_pop.json --audio-reactive
python3 -m hillview render in.mov out.mov -f prores_422 --audio song.wav -c presets/glitch_pop.json
python3 -m hillview render in.mov out.mp4 -c presets/glitch_pop.json --speed fast --workers 4
python3 -m hillview analyze song.wav -o song.mod.json --fps 30
```

### 1.3 Launch the Claude Artifact (browser version)

- Published (private to the user, version 2): <https://claude.ai/artifact/SAd2YTqRaYzEAS6T3ergzd>. This is the only place the `downloads`
  capability exists, so it is the only place Record / Save frame / Save look actually save files.
- Local copy for quick iteration, no publishing needed (Save buttons are disabled here because `window.claude` does not exist):

```bash
python3 - <<'EOF'
body = open('artifact/hillview.html').read()
open('/tmp/hillview_artifact.html', 'w').write('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body>' + body + '</body></html>')
EOF
open /tmp/hillview_artifact.html        # macOS; use xdg-open on Linux
```

The page must open on an animated demo with beat meters moving, without any click. Google Fonts load from the network; offline it falls back to system fonts.
To republish after editing (requires the Artifact tool, i.e. a Claude session, not plain Python): publish the same file path to keep the URL; `capabilities`
is `{downloads: true}` and carries forward if omitted.

### 1.4 Run the tests

```bash
cd hillview
python3 -m pytest -q tests                  # expect ~104 passed (needs ffmpeg). Some tests skip if llvm-nm/cmake are absent.
core/verify_arm64.sh                         # needs zig + qemu-user; only meaningful on Linux (see 4.2)
artifact/tests/run.sh                        # 86 browser checks; needs `npm i -g playwright`, ffmpeg, headless Chromium
```

Native core tests: `tests/test_native.py` loads whichever library `native.py` finds. On a Mac that is the **prebuilt dylib**, so a pass there is the
first real execution of that binary. To test a library built by your own toolchain instead (it takes priority over the prebuilt one):

```bash
core/build.sh                       # cmake + Apple clang -> hillview/_native/libhillview.dylib
python3 -m pytest -q tests/test_native.py
cd core && c++ -std=c++17 -O2 -fno-math-errno -fno-trapping-math -ffp-contract=off -Iinclude -Isrc \
    tests/selftest.cpp src/hv_api.cpp src/hv_kernels.cpp -o /tmp/hv_selftest && /tmp/hv_selftest    # Python-free self-test, expect "110 checks, 0 failed"
```

### 1.5 If the native core does not load on the Mac

`python3 -m hillview app` prints the reason. Likely causes, in order:

1. **Quarantine/Gatekeeper** if the repo came as a zip download: `xattr -dr com.apple.quarantine prebuilt/macos-arm64/`. (A `git clone` does not quarantine.)
2. **Code signature**: the dylib carries an ad-hoc signature from the linker. If `codesign -v prebuilt/macos-arm64/libhillview.dylib` complains, run
   `core/build_macos_arm64.sh` on the Mac (builds natively and re-signs with `codesign --force --sign -`).
3. Anything else: run `core/build.sh` (needs Xcode command line tools and cmake) and re-test; a local build wins over the prebuilt one.

Force the slow path with `HILLVIEW_NO_NATIVE=1`; point at a specific library with `HILLVIEW_LIB=/path/to/lib`.

### 1.6 What to report back after testing

Write these down; they decide what happens next: (a) the first printed `native core:` line; (b) pytest totals; (c) `/tmp/hv_selftest` result; (d) the render time
of `presets/glitch_pop.json` on a 1080p clip (cloud reference: 3.8 s for 60 frames on 4 x86 cores, native); (e) whether the artifact opens on the demo and
Record/Save work in the real viewer; (f) any console errors.

## 2. What exists

Repository root also contains four unrelated handheld apps (`hillbeat/`, `hillchord/`, `hillsequencer/`, `hillband/`). Do not touch them for this work.

```
hillview/
  README.md  DESIGN.md  HANDOFF.md      # README is the user-facing reference; DESIGN has the host-SDK research and architecture status
  hillview/                             # Python package: reference engine + standalone app
    engine.py effects.py modulation.py color.py audio.py video_io.py render.py cli.py app.py native.py pngio.py  ui/index.html
  core/                                 # C++17 library, plain C ABI, no host-SDK headers (what plugins will link)
    include/hillview/hillview.h  src/*.cpp|hpp  CMakeLists.txt  build.sh  sanitize.sh  build_macos_arm64.sh  verify_arm64.sh  tests/  tools/
  prebuilt/macos-arm64/                 # committed libhillview.dylib (+SHA256SUMS), reproducible with build_macos_arm64.sh
  artifact/hillview.html + tests/       # Claude Artifact (WebGL2) and its 86 browser checks
  presets/glitch_pop.json  tests/       # presets; Python tests (104)
```

**Pipeline per frame** (identical in Python, C++ and the artifact): frame sampling (process every Nth frame, blend or hold the rest) -> quantize (posterize
or k-means palette) -> smooth (3-pass box blur) -> colour remap (OKLab distance threshold + softness, rules `from -> to`) -> blur. Any parameter can be
modulated by LFO / smooth noise / stepped random / audio envelope. Presets are JSON and are shared by the CLI, the app and the artifact.

**Design properties that matter when changing code:**
- **No hidden state.** Output frame i depends only on source frames, config, seed and time. Random modulation is a hash of time, not an RNG stream
  (`modulation.py` `hash01`, mirrored bit-exactly in `hv_api.cpp` and the artifact). Tests enforce this; plugin hosts require it.
- **The Python engine is the specification.** C++ and JS are checked against it, not the other way round.
- **Fast paths replaced slow ones without changing the contract**; numpy remains as the fallback and as the test reference.

## 3. Decisions already made (do not relitigate without the user)

- **Avid is out of scope.** Search found only 2009-2015 evidence for AVX2 and nothing on current status.
- **DaVinci Resolve free is assumed to load third-party OpenFX plugins.** Unverified; the user said to assume it. Check it with the first test plugin.
- **Standalone app first, browser-based UI** (stdlib HTTP server on 127.0.0.1 with a per-launch token and a Host check, because it reads and writes files). No tkinter in the cloud image.
- **Native core uses no C++ runtime** (pthreads, malloc, no exceptions/rtti). A static libc++ had leaked `operator new/delete` exports from the macOS dylib; a test now forbids it.
- **`-ffp-contract=off` everywhere** so x86 and ARM round identically (it cost nothing on x86).
- **Artifact is one committed dark look**, not light/dark: video tools live in the dark.
- **Artifact controls are two layers.** Simple: Looks, Surprise me, five effect switches (Stutter, Crush, Melt, Recolour, Soften) each with one amount, four Movement macros
  (Reactivity, Drift, Chaos, Speed). Advanced: every original parameter. The switches are *views of real settings* (a stage is on when it is actually doing something), and a
  switch-off stashes the amount. Movement macros scale modulation routes at render and export time, so a saved look is exactly what you see.

Research behind the host-SDK choices, with confidence levels, is in `DESIGN.md` section 1. Primary sources used:
[TryUncle (Resolve free + OFX, Jul 2026)](https://tryuncle.com/learn/davinci-resolve/best-davinci-resolve-plugins-for-beginners-to-buy),
[CGChannel 2018](https://www.cgchannel.com/?p=98909), [Tella, the outlier that says Studio-only](https://www.tella.com/definition/ofx-plugins),
[Adobe forum: AE SDK effects in Premiere](https://community.adobe.com/t5/premiere-pro-discussions/can-you-build-premiere-pro-plugins-using-the-after-effects-sdk/m-p/9100157),
[Apple FxPlug](https://developer.apple.com/documentation/professional-video-applications/fxplug.md),
[Apple: FxPlug for Final Cut Pro](https://developer.apple.com/library/archive/documentation/AppleApplications/Conceptual/FXPlug_overview/FxPlugsFCPx/FxPlugsFCPx.html),
[FxFactory on delivering to FCP](https://fxfactory.com/developer/bringing-native-plugins-to-finalcutpro/). Several are old; re-check against current docs before building a plugin.

## 4. Verified vs not verified

### 4.1 Verified (a check actually ran)

- Linux x86-64 (g++ 13, 4 cores): Python suite 104 pass; native tests also pass under ASan + UBSan (`core/sanitize.sh`); C++ self-test 110 checks.
- ARM64 machine code under qemu (zig cross-compile, aarch64 Linux): the same 110-check self-test passes with output identical to x86. This covers NEON codegen and ARM FP behaviour, **not** macOS.
- macOS dylib *structure* (`core/tools/macho_info.py`, guarded by `tests/test_macos_arm64.py`): arm64, min macOS 11.0, ad-hoc signature present, exports only `hv_*`,
  imports only libSystem, byte-identical rebuild. CMake's Apple settings configured and built with zig as a Darwin cross toolchain.
- Performance, 1080p, 60 frames, 4-core cloud VM, incl. x264 encode: 120.9 s (original numpy) -> 3.7 s (native core) for a full chain every frame. Effect chain alone 30 ms/frame on 4 threads.
  The render is now encoder-bound.
- Artifact in headless Chromium (software WebGL): hash bit-identical to Python; LFO/noise/steps within 2e-16; shaders within 1 level of numpy (posterize, remap, blur);
  interpolation exact midpoint, hold exact; video/music/record/save with `downloads` **stubbed**; every switch and macro; 25 randomised looks; phone layout. One complete clean run of all
  86 checks; the 37 older checks passed on several earlier runs. Later full-suite repeats were stopped before finishing, so there is no repeated-run flakiness evidence for the new 49 checks.

### 4.2 NOT verified

- **The macOS dylib has never been loaded on a Mac.** Whether dyld/Gatekeeper accept it, and how fast it runs on M-series, is unknown. qemu timings are meaningless.
- **The artifact has never run in the real claude.ai viewer.** Unknown: whether its content policy allows playing a local video from a blob URL (the page shows a message and
  falls back to demo/image if not), and the real `downloads` confirmation flow (only stubbed). H.264/MP4 input was not tested (Playwright's Chromium has no H.264). GPU speed unmeasured.
- Windows and Intel Mac builds (the Windows branch in `core/src/hv_util.hpp` is uncompiled), any GPU backend (none exists), 4K, the desktop app's UI at phone widths.
- The desktop app UI (`hillview/ui/index.html`) was **not** given the simple layer; only the artifact was. Presets saved from the artifact already bake the macros in, so they load fine in the app.

Cloud-only tooling used for the ARM verification (not in the repo): `pip install ziglang` in a scratch venv and `apt-get install qemu-user`. `core/verify_arm64.sh` needs both.

## 5. Known gaps and differences to keep in mind

- **No host plugin exists yet** (OpenFX for Resolve, then AE SDK for Premiere, then FxPlug for Final Cut Pro). The C++ core has the per-frame chain, blend and modulation sources; **not yet ported**:
  `Engine.params_at` (base + modulation routing + clamps), frame planning for sampling/interpolation (`_plan`), k-means palette fitting, OKLab rule setup, audio-envelope JSON loading.
  The core is RGB-only; hosts hand over RGBA. See `DESIGN.md` section 2.
- Audio analysis differs by surface: desktop/CLI analyse the whole file offline (`audio.py`); the artifact analyses live with a slow adaptive gain, so audio-reactive results will not match an offline render.
  Palette k-means differs between numpy and the artifact's JS version. Artifact interpolation lags one step because a live player cannot see the next frame.
- Artifact `Record` is real time and produces WebM (MP4 on Safari); saving needs a user confirmation each time.
- 8-bit pipeline everywhere: HDR/log sources are flattened, alpha dropped. Animated WebP can be exported but ffmpeg 6.1 cannot import it.
- `prebuilt/macos-arm64/libhillview.dylib` is a committed binary (70 KB). Remove it and rely on `core/build.sh` if the user prefers no binaries in git.
- Shell gotcha from the cloud session: `pkill -f 'pattern'` matches the shell running it when the pattern appears in the command line (killed my shell three times). Kill by PID.

## 6. Suggested order of work

1. Do section 1 and report (1.6). If the dylib fails on the Mac, fix that before anything else; it blocks every Mac-side claim.
2. Open the artifact in the real viewer: demo plays, then try a local video, Record, and the three saves. Fix what the viewer's policies break.
3. Decide with the user whether the desktop app UI should get the same simple layer as the artifact (the logic in `artifact/hillview.html`'s "simple controls" block is the reference).
4. Start the OpenFX plugin for Resolve: first prove free Resolve loads a trivial OFX plugin (the standing assumption), then port the missing engine pieces listed in section 5 to C++,
   checking each against the Python reference the way `core/tests/selftest.cpp` and `gen_golden.py` already do.
5. Premiere (AE SDK) and Final Cut Pro (FxPlug, Metal) after that; GPU kernels are new work and the box-blur/remap structure ports directly.
