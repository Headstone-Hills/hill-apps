#!/bin/bash
# PORTMASTER: hillsequencer.zip, HillSequencer.sh
# HillSequencer — 8-track sample sequencer (muOS / RG35XXSP, aarch64).

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

GAMEDIR=/$directory/ports/hillsequencer

> "$GAMEDIR/log.txt" && exec > >(tee "$GAMEDIR/log.txt") 2>&1

cd $GAMEDIR

# --- Runtime: system python3 + /usr/lib SDL2 + vendored pygame/numpy ---------
export LD_LIBRARY_PATH="/usr/lib:$GAMEDIR/libs:$LD_LIBRARY_PATH"
export PYTHONPATH="$GAMEDIR:$GAMEDIR/pylibs"
export SDL_GAMECONTROLLERCONFIG="$sdl_controllerconfig"
export SDL_AUDIODRIVER=alsa
# pygame's bundled SDL2 lacks a working display driver on this device (it falls
# back to "offscreen"). Force-load the system SDL2 from /usr/lib (the same lib
# the working ports use) and let it auto-pick its default video driver.
export LD_PRELOAD="/usr/lib/libSDL2-2.0.so.0"

# Samples are SHARED with HillChord (ROMs/Samples). HillSequencer's render cache
# lives separately in ROMs/.hillsequencer_cache so it isn't browsed.

# First-run: build python deps on-device if not vendored yet (needs network).
if [ ! -d "$GAMEDIR/pylibs/pygame" ] || [ ! -d "$GAMEDIR/pylibs/numpy" ]; then
  echo "[HillSequencer] installing python deps into pylibs/ (first run, needs wifi)..."
  $ESUDO bash "$GAMEDIR/install_deps.sh" "$(command -v python3)" "$GAMEDIR/pylibs"
fi

# Input is read directly via pygame's joystick API (indices in
# input/button_map.py). We deliberately do NOT run gptokeyb — its keyboard
# translation would double-fire every button. Exit is a Start+Select hold.
python3 main.py

# --- Cleanup (mirrors working muOS ports) -----------------------------------
command -v systemctl >/dev/null && $ESUDO systemctl restart oga_events &
printf "\033c" > /dev/tty0
