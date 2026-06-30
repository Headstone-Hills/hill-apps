#!/bin/bash
# PORTMASTER: hillband.zip, HillBand.sh
# HillBand — 8-track hybrid drum+melodic sequencer (muOS / RG35XXSP, aarch64).

XDG_DATA_HOME=${XDG_DATA_HOME:-$HOME/.local/share}

if [ -d "/opt/system/Tools/PortMaster/" ]; then
  controlfolder="/opt/system/Tools/PortMaster"
elif [ -d "/opt/tools/PortMaster/" ]; then
  controlfolder="/opt/tools/PortMaster"
elif [ -d "$XDG_DATA_HOME/PortMaster/" ]; then
  controlfolder="$XDG_DATA_HOME/PortMaster"
else
  controlfolder="/roms/ports/PortMaster"
fi

source $controlfolder/control.txt
source $controlfolder/device_info.txt

[ -f "${controlfolder}/mod_${CFW_NAME}.txt" ] && source "${controlfolder}/mod_${CFW_NAME}.txt"
get_controls

GAMEDIR=/$directory/ports/hillband

> "$GAMEDIR/log.txt" && exec > >(tee "$GAMEDIR/log.txt") 2>&1

cd $GAMEDIR

# --- Runtime -----------------------------------------------------------------
export LD_LIBRARY_PATH="/usr/lib:$GAMEDIR/libs:$LD_LIBRARY_PATH"
export PYTHONPATH="$GAMEDIR:$GAMEDIR/pylibs"
export SDL_GAMECONTROLLERCONFIG="$sdl_controllerconfig"
export SDL_AUDIODRIVER=alsa
export LD_PRELOAD="/usr/lib/libSDL2-2.0.so.0"

# Samples are SHARED with HillChord and HillSequencer (ROMs/Samples).
# HillBand's render cache lives at ROMs/.hillband_cache (separate from the others).

# First-run: build python deps if not vendored yet (needs network).
if [ ! -d "$GAMEDIR/pylibs/pygame" ] || [ ! -d "$GAMEDIR/pylibs/numpy" ]; then
  echo "[HillBand] installing python deps into pylibs/ (first run, needs wifi)..."
  $ESUDO bash "$GAMEDIR/install_deps.sh" "$(command -v python3)" "$GAMEDIR/pylibs"
fi

# Input is read directly via pygame's joystick API.  Do NOT run gptokeyb.
python3 main.py

# --- Cleanup -----------------------------------------------------------------
command -v systemctl >/dev/null && $ESUDO systemctl restart oga_events &
printf "\033c" > /dev/tty0
