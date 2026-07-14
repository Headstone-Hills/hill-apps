# Running Hill Apps on Knulli (Anbernic RG35XXSP)

Hill Apps was originally built and tested against **muOS on the Anbernic
RG35XXSP**. This document tracks what changes were needed to also support
**Knulli** (a Batocera fork) on that same hardware. "Scarab" is Knulli's
release codename (like Ubuntu's animal codenames), not a separate device —
the board is the same RG35XXSP.

## Confirmed facts (pulled from a real device's own logs, 2026-07-14)

- Board: `rg35xx-sp` (Anbernic RG35XXSP), `CFW_NAME=knulli` (lowercase),
  Knulli release codename `scarab`, version string `scarab 2026/05/11`.
- PortMaster **is** installed on-device at
  `/userdata/system/.local/share/PortMaster` — found via the existing
  `$XDG_DATA_HOME/PortMaster` check, since `$HOME=/userdata/system` at
  launch. `mod_knulli.txt` exists there, so the `mod_${CFW_NAME}.txt` lookup
  in the launcher already works correctly.
- `control.txt` sets `directory="userdata/roms"`, so the launcher's
  `GAMEDIR=/$directory/ports/<app>` resolves to `/userdata/roms/ports/<app>`,
  matching this repo's `config.py` device-root detection.
- EmulationStation launches ports via `nice -n -4 /bin/bash
  /userdata/roms/ports/<App>.sh` — a real bash invocation, not `sh`. Its own
  launch logs live at `/userdata/system/logs/es_launch_std{out,err}.log` and
  were the single most useful diagnostic: they show the *exact* interpreter
  error for any launch failure, no SSH required — just read them off the SD
  card after a failed attempt.

## The actual root cause of every "won't launch" so far

**Not a path bug, not a PortMaster bug — CRLF line endings.** Every `.sh` file
in this repo had Windows CRLF line endings in this checkout (`core.autocrlf =
true` in git config, no `.gitattributes` to override it for shell scripts).
`es_launch_stderr.log` on the device showed the real error:

```
/userdata/roms/ports/HillSequencer.sh: line 5: $'\r': command not found
/userdata/roms/ports/HillSequencer.sh: line 10: syntax error near unexpected token `elif'
```

Bash chokes on the embedded `\r` and dies during the very first `if/elif`
block — before the script ever reaches its own `log.txt` logging setup, which
is why no per-app log was ever written and the only symptom on-device was a
black-screen flash back to EmulationStation. This affected **all four apps
identically** and was almost certainly the actual reason nothing worked from
the very first attempt, independent of the sample-path/`ON_DEVICE` fixes
below. Fixed by normalizing every `.sh` file to LF and adding a
`.gitattributes` (`*.sh text eol=lf`) so it can't regress on a future Windows
checkout.

**Lesson for next time:** if a script fails silently/instantly with no log
output on this platform, check `/userdata/system/logs/es_launch_stderr.log`
on the SD card first — it has the real interpreter error.

## What was changed

1. **Sample-library / state-file path detection** (`config.py` in HillChord,
   HillSequencer, HillBand). These three apps previously detected "am I
   running on the device" by checking for `/mnt/sdcard/ROMs`, which is a
   muOS-only mount point — on Knulli it never exists, so the apps silently
   fell back to dev-mode paths (looking for samples relative to the app
   folder instead of the shared library) and would find no samples or crash.
   They now also probe `/userdata/roms` (Knulli/Batocera's persistent storage
   root) and pick sample/state/cache paths accordingly. HillBeat was not
   affected — it never had a device-path branch; its samples always ship
   inside its own port folder.

2. **Launcher scripts** (`HillBeat.sh`, `HillChord.sh`, `HillSequencer.sh`,
   `HillBand.sh`). Two changes:
   - Added `/userdata/roms/ports/PortMaster` as a recognized PortMaster
     control-folder location (Knulli/Batocera's convention, alongside the
     existing muOS/`/opt` and generic `/roms` locations).
   - If **no** PortMaster control folder with a usable `control.txt` is
     found — e.g. the port was copied straight into the ports folder instead
     of installed through PortMaster's own install flow — the launcher no
     longer aborts. It derives `GAMEDIR` from the script's own location
     instead of from PortMaster's `$directory` variable, and skips the
     PortMaster controller-DB (`get_controls`) / `$ESUDO` setup. This is the
     failure mode that was actually reported: copying the app folders
     directly into the ports directory produced apps that wouldn't launch,
     for all four apps including HillBeat (which has no path bug), pointing
     at the launcher's hard dependency on a working PortMaster install rather
     than at per-app path issues.
   - The `LD_PRELOAD` SDL2 override now probes a short list of candidate
     paths (`/usr/lib`, `/usr/lib64`, `/lib/aarch64-linux-gnu`) instead of
     assuming `/usr/lib/libSDL2-2.0.so.0`, since that path is a muOS
     assumption not guaranteed on Batocera-based builds.

## Sample library location on Knulli

Mirroring the muOS convention (`/mnt/sdcard/ROMs/Samples`), the default on
Knulli is:

```
/userdata/roms/Samples          # shared multisample instrument library (HillChord/HillSequencer/HillBand)
/userdata/roms/ports/hillbeat/samples/Cassette Drums   # HillBand's drum kit default
```

If your actual sample library lives somewhere else on the SD card, every app
supports an env var override so you don't need to edit code:

| App | Env var(s) |
|---|---|
| HillBeat | `HILLBEAT_SAMPLES`, `HILLBEAT_STATE`, `HILLBEAT_CACHE` |
| HillChord | `HILLCHORD_SAMPLES`, `HILLCHORD_STATE`, `HILLCHORD_CACHE` |
| HillSequencer | `HILLSEQ_SAMPLES`, `HILLSEQ_CACHE`, `HILLSEQ_STATE`, `HILLSEQ_SEQUENCES`, `HILLSEQ_LOOPCACHE` |
| HillBand | `HILLBAND_SAMPLES`, `HILLBAND_DRUM_SAMPLES`, `HILLBAND_CACHE`, `HILLBAND_STATE`, `HILLBAND_SEQUENCES`, `HILLBAND_LOOPCACHE` |

Set these near the top of the launcher `.sh` (after the `GAMEDIR` block, before
`python3 main.py`) if you need a non-default location.

## Duplicate/phantom entries in the Ports list

Unlike muOS, Knulli's ports scan is **not** limited to the top level of
`roms/ports/` — it recurses into each app's subfolder and lists every `.sh`
file it finds there as its own launchable "game". This repo's own dev tooling
(`build_port.sh`, `deploy.sh`) and, previously, a second copy of the launcher
itself (this repo keeps `HillChord.sh` etc. checked into the app's own folder
alongside `main.py`, since that's the canonical source location) all showed
up as extra, confusing entries — e.g. "4 install deps", "2 HillChord", plus
inert `build_port`/`deploy` entries. None of this came from `gamelist.xml`
(stale entries there are a separate, harmless issue — see below); it's a live
directory scan.

Fixed by:
- **Not shipping `build_port.sh`/`deploy.sh` to the device at all** — they're
  dev-machine-only tooling (build a zip / scp it over from a Mac or PC) and
  never need to exist on the SD card.
- **Not duplicating the launcher inside the gamedir** — only
  `roms/ports/<App>.sh` should exist; `roms/ports/<app>/<App>.sh` should not.
  (`build_port.sh`'s own packaging logic already reflects this — it stages the
  launcher at the zip root and never copies it into the gamedir; a naive
  straight copy of this repo's app folder onto the SD card does, though.)
- **Renaming `install_deps.sh` to `install_deps`** (no extension) in all four
  apps, since it must remain in the gamedir (the launcher invokes it) but a
  `.sh` extension there gets picked up as its own spurious menu entry.
  Renaming doesn't affect anything — it's always invoked explicitly via
  `bash "$GAMEDIR/install_deps" ...`, never relying on its own shebang/`+x`
  bit or a PATH lookup by name.

## Installing manually on Knulli (no PortMaster required)

1. Copy `<App>.sh` into `/userdata/roms/ports/`.
2. Copy the app's folder (e.g. `hillchord/`) next to it, at
   `/userdata/roms/ports/hillchord/`, **excluding** `build_port.sh`,
   `deploy.sh`, and the app's own copy of `<App>.sh` (see above) — only
   `main.py` + its supporting modules, assets, `port.json`, and `install_deps`
   belong in the gamedir.
3. `chmod +x /userdata/roms/ports/<App>.sh`.
4. Rescan/restart EmulationStation (or reboot) so the new `.sh` shows up in
   the Ports list. `gamelist.xml` in `roms/ports/` is not rewritten
   immediately after every launch (EmulationStation flushes it on its own
   schedule/on a clean exit), so don't rely on `playcount`/`lastplayed` there
   to tell whether a fresh copy actually ran — check the logs below instead.
   Stale entries from a previous install (different folder casing, etc.) can
   also linger in `gamelist.xml` alongside the current one; harmless, just
   confusing to look at.
5. First launch needs network access once, to build `pygame`/`numpy` into
   `pylibs/` for Knulli's own Python ABI (`install_deps`).
6. If a launch fails, check `/userdata/system/logs/es_launch_stderr.log` and
   `es_launch_stdout.log` on the SD card — this is EmulationStation's own
   launch log and will show the real interpreter error even when the app's
   own `log.txt` was never created (e.g. the CRLF failure above never got far
   enough to write one).

## Volume on-screen display

Physical volume buttons adjust the system mixer directly (via PipeWire/
`wpctl` on Knulli) — the app process never receives a button-press event for
them at all, and our fullscreen SDL apps cover any OS-level OSD Knulli might
normally show. `volume_osd.py` (new, one copy per app) polls `wpctl
get-volume @DEFAULT_AUDIO_SINK@` from a background daemon thread every 300ms
and pops up a small "VOLUME XX%" / "MUTED" toast with a level bar for ~1.5s
whenever the value changes. Silently does nothing if `wpctl` isn't present
(e.g. dev machine, muOS). Wired into each app's main loop just before
`pygame.display.flip()`.

**Confirmed on-device:** physical volume buttons work correctly at the
system level in both directions (verified 0.60→0.70→0.95→0.00 across several
up/down presses while an app was running in the foreground), and the on-app
toast displays correctly once wired in.

## First-run install splash

`install_deps` previously installed `pygame` and `numpy` in one `pip` call
with zero visual feedback — first launch showed a black screen for however
long the download took. Since pygame itself is what's being installed, it
can't be used to show a splash for its *own* install; the fix installs
`pygame` first (fast), then launches `install_splash.py` (new, one copy per
app) in the background to show that app's `splash.png` plus an animated
"Installing dependencies..." bar while `numpy` installs, then kills it
cleanly (it traps `SIGTERM` and calls `pygame.quit()` itself, rather than
being killed abruptly — an abrupt kill of a raw SDL process risks the same
kind of stuck-display handoff issue described above) before handing off to
`main.py`.

There is still an unavoidable black window at the very start (while pygame
itself downloads, since nothing can render before pygame exists) — a few
seconds to ~20s depending on network speed. Confirmed working end-to-end on
a real fresh install on-device: pygame installs, splash appears with legible
text over a solid background bar, numpy installs, splash exits cleanly, app
launches normally.

## Confirmed on real hardware (2026-07-14, live SSH session)

Everything below was verified directly on the device, not inferred:

- All four apps (`HillBeat`, `HillChord`, `HillSequencer`, `HillBand`) launch
  successfully from the Ports menu after the CRLF fix, including a genuine
  first-run `install_deps` (pip install over real wifi, `ensurepip`
  bootstrap, both confirmed fast — under a minute total).
- Joystick button mapping in `constants.py`/`button_map.py` (A=3, B=4, X=6,
  Y=5, L1=7, R1=8, L2=12, R2=13, SELECT=9, START=10, FUNCTION=11) is **exactly
  correct** for this hardware — verified live with `tools/probe_input.py`
  against the actual pad. No mismatch; no fix needed.
- Apps exit cleanly back to EmulationStation via the documented Start+Select
  hold. The one gotcha: exiting a different way (or not exiting fully) can
  leave the `python3 main.py` process running in the background — always
  confirm a clean exit before launching another app, or you can get two
  processes fighting over the audio/display device (this is what caused a
  visible EmulationStation-menu glitch during testing, not any app bug —
  fixed by killing both stray processes and restarting EmulationStation via
  `/etc/init.d/S31emulationstation stop` then `start`).
- Audio plays correctly through PipeWire once a sample/instrument is
  assigned via each app's library overlay (`START+B` for HillBeat/
  HillSequencer/HillBand, `START` for HillChord) — a fresh install has no
  sample assigned to any voice/track by design (`sample`/`sound` default to
  `null`), so silence on first launch is expected, not a bug.
- `libSDL2-2.0.so.0` exists at `/usr/lib/libSDL2-2.0.so.0` on this device (the
  first candidate in the probed list).
- Raw `python3 main.py` runs triggered directly over SSH (bypassing Knulli's
  normal `gameStart`/`gameStop` launch hooks entirely) can leave the display
  in a glitched/stuck state after exit — this is specific to bypassing the
  normal launch chain, not a bug in the apps themselves. Recoverable without
  a hard reboot: `/etc/init.d/S31emulationstation stop` then `start` (or
  `kill` the EmulationStation binary PID and let its supervisor wrapper,
  `emulationstation-standalone`, restart it — but don't kill the wrapper
  process itself, or nothing will restart it automatically).

## Known limitations / deliberately not addressed here

- **Input remapping.** Discussed but not implemented — every app hardcodes
  which raw joystick button index maps to which action, and the live
  `tools/probe_input.py` test on real hardware confirmed the existing
  mapping is already correct, so there was no live mismatch to work around.
  Making it user-configurable would mean a settings UI, persisted custom
  mappings, and reworking every button check in all four apps' `main.py` to
  go through it instead of a fixed constant — a substantial feature on its
  own, not a quick add.
- **The initial black window before the install splash appears** (while
  `pygame` itself downloads, typically a few seconds up to ~20s) is
  inherent — nothing can render before pygame exists to render with. Not
  further addressed.
- **`gamelist.xml` cover art / descriptions** for each app in Knulli's Ports
  list were added directly on the test device during this session, not
  through this repo — `gamelist.xml` is per-install EmulationStation state
  (regenerated per SD card, not something to commit here). If you want this
  automated for future installs, it'd need a small install script that
  patches `roms/ports/gamelist.xml` with each app's existing `cover.png` and
  `port.json` description; not implemented.
- **Pre-existing minor inconsistency, not introduced by this PR:**
  `hillbeat/install_deps` pins `numpy>=1.21` while the other three pin
  `numpy>=1.26`. Harmless (both installed fine in testing) but worth
  aligning at some point.
