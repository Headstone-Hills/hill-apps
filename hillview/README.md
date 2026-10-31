# HillView

Pixel-mangling video effect for abstract / avant-garde music visualisation.

Two deliverables share one engine:

1. **Standalone app** (this folder, working now): import almost any video, preview live, export to many formats.
2. **Host plugins**: DaVinci Resolve (OpenFX, assumed to load in the free version), then Premiere Pro (After Effects SDK),
   then Final Cut Pro (FxPlug). **Not written yet**; see [DESIGN.md](DESIGN.md). Avid is out of scope.

## Claude Artifact (browser version)

`artifact/hillview.html` is HillView as a single-page Claude Artifact: the same pipeline (frame sampling, quantize, smooth, colour remap with
OKLab distance, blur, modulation) running as WebGL2 shaders on the viewer's GPU, with no ffmpeg, Python or upload. It opens on a built-in demo
with a synthetic beat; you can open a video or image, add a music file, pick remap colours with an eyedropper on the picture, and record the
processed result. Presets use the same JSON as the desktop app and CLI. Export (video, PNG, preset) goes through the artifact `downloads`
capability, so the viewer confirms each save.

Differences from the desktop app, by design: audio analysis is live (a slow adaptive gain instead of the desktop's whole-file normalisation),
so audio-reactive results differ from an offline render; recording is real time and produces WebM (MP4 on Safari); palette k-means is a small
JS implementation, not numpy, so palettes differ; frame interpolation delays output by one step because a live player cannot see the next frame.

```
artifact/tests/run.sh    # needs ffmpeg and `npm i -g playwright`; headless Chromium on software WebGL
```
37 checks: modulation hash bit-identical to Python (incl. negative and 2^40 inputs), LFO/noise/steps within 1e-15, OKLab within 3e-7,
shader output within 1 level of numpy for posterize, remap (with hue shift and offsets) and blur, interpolation is an exact midpoint cross-fade,
hold is exact, video/music/record/save flows with `downloads` stubbed, phone layout. **Not verified:** the real claude.ai viewer (its media
and download policies), MP4/H.264 input in the test browser (Playwright's Chromium has no H.264), and speed on real GPUs.

## Native core (optional, 10-30x faster)

`core/` is a C++17 library with a plain C interface (`core/include/hillview/hillview.h`) and no host-SDK dependencies. The Python app uses it
automatically when built, and the host plugins will link the same library. Without it everything still works through numpy.

```
core/build.sh            # needs cmake + a C++17 compiler; copies the library to hillview/_native/
core/sanitize.sh         # native tests under AddressSanitizer + UBSan (Linux, g++)
core/verify_arm64.sh     # ARM64 self-test under qemu + macOS dylib build and structure check
python -m pytest tests   # native tests build the library themselves if cmake + g++ exist, else skip
HILLVIEW_NO_NATIVE=1 ...  # force the numpy path;  HILLVIEW_LIB=/path/to/lib to point at a specific build
```

What it contains: posterize / palette quantize, separable blur (cache-blocked running sums, three box passes), the OKLab colour remap
(vectorised, all rules fused into one pass), integer frame blending, and the modulation hash/LFO/noise/steps functions, parity-tested
against the Python definitions bit-for-bit (they are what plugins must reproduce). Each frame is multithreaded inside the library;
ctypes releases the GIL, so Python also overlaps two frames. AVX2/FMA versions of the hot loops are chosen at load time (x86-64 Linux);
other platforms get the baseline build.

**Verified:** Linux x86-64 (g++ 13, AVX-512 CPU, so the AVX2 variants ran; the baseline variants ran via the sanitizer build) and
Linux aarch64 under qemu (see Apple Silicon below). The Python suite passes and the native tests also pass under ASan + UBSan.
`core/tests/selftest.cpp` is a Python-free C++ test (naive double-precision references, golden output from numpy, modulation values from Python).
**Not verified:** Windows, Intel Macs, any GPU backend (none exists yet).

## Apple Silicon (macOS arm64)

`prebuilt/macos-arm64/libhillview.dylib` is a ready-made arm64 build (macOS >= 11, 71 KB, depends only on libSystem, ad-hoc signed).
On an Apple Silicon Mac the app finds it automatically; reproduce it with `core/build_macos_arm64.sh` (on a Mac: Apple clang via CMake;
elsewhere: cross-compiled with zig, `pip install ziglang`, no Apple SDK needed; the output is byte-identical between runs).
SHA-256 is in `prebuilt/macos-arm64/SHA256SUMS`.

**What I could verify without a Mac** (`core/verify_arm64.sh` reruns all of it):
- The ARM64 build of the core, run as real aarch64 machine code under qemu, passes the 110-check self-test and produces output identical to x86
  (same golden matches, same rounding differences). This covers NEON code generation and ARM floating-point behaviour.
- The dylib's Mach-O structure (`core/tools/macho_info.py`, guarded by `tests/test_macos_arm64.py`): arm64, dylib, min macOS 11.0, embedded ad-hoc code
  signature (Apple Silicon refuses unsigned libraries), exports only `hv_*`, imports only libSystem (no C++ runtime).
- Library lookup and platform detection logic, and that a load failure is reported with its reason rather than silently falling back.
- CMake's Apple settings, by configuring and building with zig as a Darwin cross toolchain.

**What I could NOT verify:** that macOS actually loads and runs the dylib, and how fast it is on an M-series chip. qemu proves the logic, not the
speed (emulation timing means nothing) and not dyld/Gatekeeper behaviour. Please confirm on your Mac:

```
brew install ffmpeg                      # the app shells out to ffmpeg/ffprobe
pip3 install -r requirements.txt
python3 -m hillview app                  # first line printed: "native core: .../libhillview.dylib"  (or why it is unavailable)
python3 -m pytest tests                  # native tests exercise the dylib directly
```

If that first line says *unavailable*, the reason is printed. Likely causes: macOS quarantined a file you downloaded as a zip
(`xattr -d com.apple.quarantine prebuilt/macos-arm64/libhillview.dylib`), or you want a build from your own toolchain
(`core/build.sh`, needs Xcode command line tools + cmake; it takes priority over the prebuilt one). Without the library everything still works through numpy,
just slower. Not done: a double-clickable `.app`, bundling Python/ffmpeg, notarization, an Intel Mac build.
## Standalone app

```
pip install -r requirements.txt        # needs ffmpeg + ffprobe on PATH
python -m hillview app                 # opens http://127.0.0.1:8765/ in your browser
```

Drop a file on the window (or use the path box), adjust controls with a live preview
(scrub the timeline, hold *Hold to compare* to flip to the original), optionally add a music track, pick a format, Render.
Exports are saved to `~/HillView/output/` (override with `HILLVIEW_HOME`) and are downloadable from the page.
Presets save/load as JSON and are the same format the CLI uses.

### Import

Anything your ffmpeg can decode: MP4/MOV/MKV/AVI/WebM/MXF/MTS/FLV/WMV/OGV/3GP and the ~370 demuxers in a stock ffmpeg 6.1 build,
H.264/H.265/AV1/VP8/VP9/ProRes/DNxHD/MPEG-2/MJPEG/FFV1 etc., animated GIF, single images, and image sequences
(`shot_%04d.png`, set the frame rate). Variable-frame-rate input is conformed to constant frame rate, rotation metadata is
applied, odd sizes are cropped by 1 px. Audio comes from the video or from a separate music file.
(Tested in `tests/`: H.264 MP4, rotated MP4, VFR MP4, PNG sequences, a single PNG, and every export format re-imported where ffmpeg can read it.
Other containers rely on ffmpeg's own demuxers and are not individually tested.)

### Export (`python -m hillview formats` lists what your ffmpeg can encode)

| id | format | audio |
|---|---|---|
| h264, h265, av1 | MP4 | AAC |
| vp9, vp8 | WebM | Opus |
| prores_422, prores_4444 | MOV (ProRes via ffmpeg's `prores_ks`) | PCM |
| dnxhr_hq | MOV (min. 256x120) | PCM |
| mjpeg | MOV | PCM |
| ffv1 | MKV, lossless | FLAC |
| mpeg4 | AVI | MP3 |
| gif, webp | animated GIF / animated WebP | none |
| png_seq, tiff_seq, jpg_seq | folder of frames (zipped on download) | none |

### Known limits

- **8-bit pipeline.** Sources are decoded to 8-bit RGB, so 10-bit/HDR/log footage is flattened and not colour-managed.
  ProRes/DNxHR outputs hold 8-bit-derived data. Alpha is dropped.
- **Animated WebP can be exported but not imported:** ffmpeg 6.1's WebP decoder rejects animated files (static WebP is fine).
- Preview is rendered at <= 640 px wide with blur radii scaled to match; exact pixels differ slightly from a full-res render.
- **Performance (measured)**, 1920x1080, 60 frames, 4-core cloud VM, synthetic test-pattern clip, x264 CRF 16 encode included
  (median of 3 runs):

  | | original numpy | numpy + processes | **native core** |
  |---|---|---|---|
  | `presets/glitch_pop.json` (k-means, every 4th frame) | 43.1 s | 7.4 s | **3.8 s** (2.4 s with `--speed fast`) |
  | every frame: posterize + smooth 3 + colour remap + blur 2 | 120.9 s | 8.9 s | **3.7 s** (3.1 s with `--speed fast`) |
  | pass-through, no effects (decode + encode floor) | | 3.5 s | 2.3 s |
  | effect chain alone, per 1080p frame | ~1900 ms | 340 ms | **108 ms on 1 thread, 30 ms on 4** |
  | interactive preview (640 px wide) | | ~37 ms | a few ms |

  With the native core the render is **encoder-bound**: a profile shows Python doing ~0.6 s of work in a 3.8 s run while waiting on x264.
  The effect chain is ~30 fps-capable on 4 cores at 1080p; the file encode is what limits wall time. Not tested at 4K.
  Machines without a built native core fall back to numpy automatically (8.1 s for the same full-chain render).
  Output differences vs. the original numpy version: blur edges use plain edge-extension (differs only in a border strip a few radii wide),
  remap is within 0.003 (< 1 8-bit level), and frames passed between stages in the parallel pipeline are 8-bit (within ~1 level).
  `quality.fast_blur` applies only to the numpy path; the native blur is always exact.
- The app is a Python program with a browser UI, **not yet a packaged installer**. Bundling ffmpeg and Python into per-OS
  executables is unfinished.
- Security: the server listens on 127.0.0.1 only and requires a per-launch token plus a Host check on every API call.

## CLI

```
python -m hillview render in.mov out.mp4 -c presets/glitch_pop.json --audio-reactive   # --workers N parallelism, --speed fast|balanced|quality encoder preset
python -m hillview render in.mov out.mov -f prores_422 --audio song.wav -c cfg.json   # music drives modulation + is muxed in
python -m hillview render "shot_%04d.png" out.mp4 --fps 24                           # image sequence in
python -m hillview render in.mp4 frames_dir -f png_seq                               # image sequence out (path is a directory)
python -m hillview analyze song.wav -o song.mod.json --fps 30                        # reusable audio envelope
python -m pytest tests                                                               # needs ffmpeg
```

## Pipeline (per frame)

1. **Frame sampling** - process only every *N*th frame; `blend` or `hold` the rest
2. **Quantize** (optional) - `posterize` (levels) or `kmeans` (palette size)
3. **Smooth** - variable-radius blur (3-pass box, approximately Gaussian)
4. **Colour remap** - rules `from -> to` with OKLab distance `threshold` + `softness`
5. **Blur** - final image blur
6. **Modulation** - LFO / smooth noise / random steps / audio envelope -> any parameter

## Config

See `presets/glitch_pop.json`. Modulation entry: `{"source": {...}, "target": "smooth.sigma", "depth": 8}`.
Sources: `lfo` (sine/saw/tri/square, `hz`), `noise` (smooth, `hz`, `seed`), `steps` (sample-and-hold, `hz`, `seed`),
`audio` (`channel`: rms/low/mid/high/onset). Sources output 0-1; `lfo/noise/steps` are bipolar by default (`-1..1`),
`audio` is unipolar; override with `"bipolar"`. Targets: `quantize.levels`, `quantize.colors`, `smooth.sigma`,
`blur.sigma`, `remap.threshold` (added to every rule), `remap.amount`, `remap.hue_shift` (radians), `sampling.step`.

## Key property: no hidden state

Output frame *i* depends only on source frames, config, seed and time. Random modulation is a hash of time, not an RNG
stream, so scrubbing and out-of-order rendering give identical results
(`tests/test_hillview.py::test_random_access_equals_sequential`). Plugin hosts require this.
