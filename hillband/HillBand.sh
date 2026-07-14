#!/bin/bash
# PORTMASTER: hillband.zip, HillBand.sh
# HillBand — 8-track hybrid drum+melodic sequencer.
# Supports muOS (RG35XXSP) and Knulli (Anbernic RG-Scarab), aarch64.

XDG_DATA_HOME=${XDG_DATA_HOME:-$HOME/.local/share}

if [ -d "/opt/system/Tools/PortMaster/" ]; then
  controlfolder="/opt/system/Tools/PortMaster"
elif [ -d "/opt/tools/PortMaster/" ]; then
  controlfolder="/opt/tools/PortMaster"
elif [ -d "$XDG_DATA_HOME/PortMaster/" ]; then
  controlfolder="$XDG_DATA_HOME/PortMaster"
elif [ -d "/userdata/roms/ports/PortMaster" ]; then
  controlfolder="/userdata/roms/ports/PortMaster"   # Knulli / Batocera
elif [ -d "/roms/ports/PortMaster" ]; then
  controlfolder="/roms/ports/PortMaster"
fi

SCRIPTDIR="$(cd "$(dirname "$0")" && pwd)"

if [ -n "$controlfolder" ] && [ -f "$controlfolder/control.txt" ]; then
  source $controlfolder/control.txt
  source $controlfolder/device_info.txt

  [ -f "${controlfolder}/mod_${CFW_NAME}.txt" ] && source "${controlfolder}/mod_${CFW_NAME}.txt"
  get_controls

  GAMEDIR=/$directory/ports/hillband
else
  # No working PortMaster control folder — e.g. this port was copied straight
  # into the ports folder instead of installed through PortMaster. Fall back
  # to a self-contained launch: derive GAMEDIR from this script's own location
  # and skip PortMaster's controller-DB / ESUDO setup.
  echo "[HillBand] PortMaster control folder not found; running standalone."
  GAMEDIR="$SCRIPTDIR/hillband"
fi

> "$GAMEDIR/log.txt" && exec > >(stdbuf -oL -eL tee "$GAMEDIR/log.txt" 2>/dev/null || tee "$GAMEDIR/log.txt") 2>&1

cd $GAMEDIR

# --- Runtime -----------------------------------------------------------------
export LD_LIBRARY_PATH="/usr/lib:$GAMEDIR/libs:$LD_LIBRARY_PATH"
export PYTHONPATH="$GAMEDIR:$GAMEDIR/pylibs"
export SDL_GAMECONTROLLERCONFIG="$sdl_controllerconfig"
export SDL_AUDIODRIVER=alsa
# Path varies by CFW, so probe the known locations rather than hardcoding one.
for _sdl2 in /usr/lib/libSDL2-2.0.so.0 /usr/lib64/libSDL2-2.0.so.0 /lib/aarch64-linux-gnu/libSDL2-2.0.so.0; do
  if [ -f "$_sdl2" ]; then
    export LD_PRELOAD="$_sdl2"
    break
  fi
done

# Samples are SHARED with HillChord and HillSequencer (device Samples folder).
# HillBand's render cache lives separately (.hillband_cache).

# First-run: build python deps if not vendored yet (needs network).
if [ ! -d "$GAMEDIR/pylibs/pygame" ] || [ ! -d "$GAMEDIR/pylibs/numpy" ]; then
  echo "[HillBand] installing python deps into pylibs/ (first run, needs wifi)..."
  $ESUDO bash "$GAMEDIR/install_deps" "$(command -v python3)" "$GAMEDIR/pylibs"
fi

# Input is read directly via pygame's joystick API.  Do NOT run gptokeyb.
python3 main.py

# --- Cleanup -----------------------------------------------------------------
command -v systemctl >/dev/null && $ESUDO systemctl restart oga_events &
printf "\033c" > /dev/tty0
