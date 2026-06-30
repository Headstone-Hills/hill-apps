# Hill Apps

A suite of four custom music apps for the RG35XXSP handheld (muOS / PortMaster):

- **HillBeat** — step-sequencer drum machine
- **HillChord** — chord/note instrument with looper, arpeggiator, and effects
- **HillSequencer** — 8-track step sequencer built on HillChord's instrument engine
- **HillBand** — 8-track × 16-step hybrid (4 melodic tracks + 4 drum tracks)

Each app has its own `README.md` with architecture notes, button bindings, and known quirks.

## Layout

```
hillbeat/        drum machine
hillchord/       chord/note instrument
hillsequencer/   8-track sequencer
hillband/        melodic + drum hybrid sequencer
```

## Samples

Sample audio is not committed to this repo (see `.gitignore`) — it's distributed
separately and lives on the device's SD card / shared sample library. Each app's
README documents where it expects samples to be found.

## Setup

Each app has an `install_deps.sh` that installs its Python dependencies
(`pygame`, `numpy`) into a local `pylibs/` directory using the target device's
own Python, so the wheels match the device's ABI:

```
bash install_deps.sh "$(command -v python3)" ./pylibs
```
