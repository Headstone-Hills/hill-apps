#!/bin/bash
# On the SP:  cd /mnt/sdcard/ports/hilldaw && bash audio_test.sh aplay 20
#             bash audio_test.sh pygame 20
# Reuses numpy/pygame vendored by another Hill app (pylibs), exactly as the Hill launchers do.
cd "$(dirname "$0")" || exit 1
P=""
for d in ./pylibs /mnt/sdcard/ports/hillsynth/pylibs /mnt/sdcard/ports/*/pylibs; do
  [ -d "$d/numpy" ] && P="$d" && break
done
[ -z "$P" ] && { echo "no pylibs with numpy found"; exit 1; }
echo "using $P"
export PYTHONPATH="$PWD:$P"
export LD_LIBRARY_PATH="/usr/lib:$LD_LIBRARY_PATH"
export LD_PRELOAD="/usr/lib/libSDL2-2.0.so.0"
export SDL_AUDIODRIVER=alsa
B="${1:-aplay}"; S="${2:-20}"; shift 2 2>/dev/null
python3 audio_test.py "$B" --seconds "$S" "$@" 2>&1 | tee "audio_test_$B.txt"
