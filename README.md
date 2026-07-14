# Hill Apps

A suite of four custom music apps for the Anbernic RG35XXSP handheld (muOS /
PortMaster). Also supports Knulli on that same hardware — see [KNULLI.md](KNULLI.md).

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

## Installing on Knulli (Anbernic RG35XXSP)

1. Copy `<App>.sh` (e.g. `HillBeat.sh`) into `roms/ports/` on the SD card.
2. Copy the app's own folder (e.g. `hillbeat/`) next to it, at
   `roms/ports/hillbeat/`. Don't copy `build_port.sh`, `deploy.sh`, or a
   second copy of the launcher `.sh` into that folder — see
   [KNULLI.md](KNULLI.md) for why.
3. Make sure `<App>.sh` is executable (`chmod +x`).
4. Rescan/restart EmulationStation (or reboot) so the new entry shows up
   under Ports.
5. Drop your sample library into place (see Samples above).
6. Launch it. First run needs wifi once to install `pygame`/`numpy` for
   Knulli's own Python (usually under a minute) — a loading screen with the
   app's cover art shows while that happens, then it starts normally.

**No sound on first launch?** That's expected — a fresh install has no
sample/instrument assigned to any voice or track yet. Press `START+B`
(HillBeat/HillSequencer/HillBand) or `START` (HillChord) to open the
library and assign one. Hold `FUNCTION+SELECT` any time for the full
controls cheat-sheet.

See [KNULLI.md](KNULLI.md) for full details, troubleshooting, and what
changed to make this work.

## Setup

Each app has an `install_deps` that installs its Python dependencies
(`pygame`, `numpy`) into a local `pylibs/` directory using the target device's
own Python, so the wheels match the device's ABI:

```
bash install_deps "$(command -v python3)" ./pylibs
```

On-device this happens automatically on first launch (needs wifi once,
usually under a minute) — a splash screen with a loading indicator is shown
once pygame itself finishes installing (see [KNULLI.md](KNULLI.md)).
