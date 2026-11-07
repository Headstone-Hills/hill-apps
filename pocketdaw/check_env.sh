#!/bin/bash
# Read-only survey of the SP for the audio-output design (nothing is written or changed).
#   cd /mnt/sdcard/ports/pocketdaw && bash check_env.sh
# Output is also saved to env_report.txt.
cd "$(dirname "$0")" || exit 1
{
echo "=== python + modules (system) ==="
python3 --version
for m in pygame numpy scipy; do python3 -c "import $m; print('$m', $m.__version__ if hasattr($m,'__version__') else '?', '->', $m.__file__)" 2>&1 | tail -1; done
echo; echo "=== vendored pylibs in the Hill apps (can be reused via PYTHONPATH) ==="
ls -d /mnt/sdcard/ports/*/pylibs 2>/dev/null || echo "none found under /mnt/sdcard/ports"
for d in /mnt/sdcard/ports/*/pylibs; do
  [ -d "$d" ] && echo "$d: $(ls "$d" | tr '\n' ' ' | cut -c1-200)"
done
echo; echo "=== try importing from the first pylibs found ==="
P=$(ls -d /mnt/sdcard/ports/*/pylibs 2>/dev/null | head -1)
if [ -n "$P" ]; then
  PYTHONPATH="$P" LD_PRELOAD=/usr/lib/libSDL2-2.0.so.0 python3 -c "
import numpy, pygame
print('numpy', numpy.__version__, '| pygame', pygame.version.ver, '| SDL', pygame.version.SDL)
" 2>&1 | tail -3
fi
echo; echo "=== sound hardware ==="
cat /proc/asound/cards 2>&1
echo "--- pcm devices ---"; cat /proc/asound/pcm 2>&1
echo "--- /dev/snd ---"; ls /dev/snd 2>&1
ls /usr/lib/libasound* /usr/lib/libSDL2* 2>&1 | head
echo; echo "=== tools ==="
for t in aplay arecord ffmpeg lame flac gcc cc sox amixer; do printf "%-8s %s\n" "$t" "$(command -v $t || echo '-')"; done
echo; echo "=== where Hill apps / HillBeat keep data ==="
ls /mnt/sdcard/ports 2>&1
ls -la /mnt/sdcard/ports/hillbeat 2>&1 | head -30
ls /mnt/sdcard/ROMS 2>&1 | head -30
ls /mnt/sdcard/ROMs 2>&1 | head -30
echo; echo "=== CPU / memory / storage ==="
grep -m1 -i "model name\|Hardware" /proc/cpuinfo; grep -E "MemTotal|MemAvailable" /proc/meminfo
df -h /mnt/sdcard | tail -1
echo; echo "=== done ==="
} 2>&1 | tee env_report.txt
