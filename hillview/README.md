# HillView

Pixel-mangling video effect for abstract / avant-garde music visualisation.

Two deliverables share one engine:

1. **Standalone app** (this folder, working now): import almost any video, preview live, export to many formats.
2. **Host plugins**: DaVinci Resolve (OpenFX, assumed to load in the free version), then Premiere Pro (After Effects SDK),
   then Final Cut Pro (FxPlug). **Not written yet**; see [DESIGN.md](DESIGN.md). Avid is out of scope.

## Native core (optional, 10-30x faster)

`core/` is a C++17 library with a plain C interface (`core/include/hillview/hillview.h`) and no host-SDK dependencies. The Python app uses it
automatically when built, and the host plugins will link the same library. Without it everything still works through numpy.

```
core/build.sh            # needs cmake + a C++17 compiler; copies the library to hillview/_native/
core/sanitize.sh         # native tests under AddressSanitizer + UBSan (Linux, g++)
python -m pytest tests   # native tests build the library themselves if cmake + g++ exist, else skip
HILLVIEW_NO_NATIVE=1 ...  # force the numpy path;  HILLVIEW_LIB=/path/to/lib to point at a specific build
```

What it contains: posterize / palette quantize, separable blur (cache-blocked running sums, three box passes), the OKLab colour remap
(vectorised, all rules fused into one pass), integer frame blending, and the modulation hash/LFO/noise/steps functions, parity-tested
against the Python definitions bit-for-bit (they are what plugins must reproduce). Each frame is multithreaded inside the library;
ctypes releases the GIL, so Python also overlaps two frames. AVX2/FMA versions of the hot loops are chosen at load time (x86-64 Linux);
other platforms get the baseline build.

**Verified:** Linux x86-64, g++ 13, on an AVX-512 CPU (so the AVX2 variants ran; the baseline variants were run via the sanitizer build).
93 tests pass; the 33 native tests also pass under ASan + UBSan. **Not verified:** macOS, Windows, ARM builds (the CMake is portable and
the SIMD dispatch is compiled out there, but I have not built it), and any GPU backend (none exists yet).

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
