# HillView

Pixel-mangling video effect for abstract / avant-garde music visualisation.
Target hosts: DaVinci Resolve (free), Premiere Pro, Final Cut Pro, Avid Media Composer.

**Status (v0.1):** host-agnostic *reference engine* in Python + CLI. The host plugins
(C++/OFX, AE-SDK, FxPlug, AVX2) are not written yet — see [DESIGN.md](DESIGN.md).

## Pipeline (per frame)

1. **Frame sampling** – process only every *N*th frame; `blend` or `hold` the rest
2. **Quantize** (optional) – `posterize` (levels) or `kmeans` (palette size)
3. **Smooth** – variable-radius blur (3-pass box ≈ Gaussian)
4. **Colour remap** – rules `from → to` with OKLab distance `threshold` + `softness`
5. **Blur** – final image blur
6. **Modulation** – LFO / smooth noise / random steps / audio envelope → any parameter

## Usage

```
pip install -r requirements.txt            # needs ffmpeg + ffprobe on PATH
python -m hillview render in.mp4 out.mp4 -c presets/glitch_pop.json --audio-reactive
python -m hillview analyze song.wav -o song.mod.json --fps 30    # reusable envelope
python -m hillview render in.mp4 out.mp4 -c cfg.json --audio-mod song.mod.json
python -m pytest tests
```

## Config

See `presets/glitch_pop.json`. Modulation entry:
`{"source": {...}, "target": "smooth.sigma", "depth": 8}`. Sources: `lfo` (sine/saw/tri/square, `hz`),
`noise` (smooth, `hz`, `seed`), `steps` (sample-and-hold, `hz`, `seed`), `audio`
(`channel`: rms/low/mid/high/onset). Sources output 0–1; `lfo/noise/steps` are bipolar by default
(`-1..1`), `audio` is unipolar; override with `"bipolar"`. Targets: `quantize.levels`,
`quantize.colors`, `smooth.sigma`, `blur.sigma`, `remap.threshold` (added to every rule),
`remap.amount`, `remap.hue_shift` (radians), `sampling.step`.

## Key property: no hidden state

Output frame *i* depends only on source frames, config, seed and time. Random modulation is a
hash of time, not an RNG stream, so scrubbing and out-of-order rendering give identical results
(tested in `tests/test_hillview.py::test_random_access_equals_sequential`). Hosts require this.
