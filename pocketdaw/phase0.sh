#!/bin/bash
# Phase 0 hardware proof, run ON the SP (no computer needed):
#     cd /mnt/sdcard/ports/pocketdaw && bash phase0.sh
# Plug the FM-1 in first, turn it on, and turn its volume up. Output is also saved to phase0_report.txt.
cd "$(dirname "$0")" || exit 1
{
echo "=== 0. environment ==="
python3 --version; uname -r; date
echo; echo "=== 1. USB VBUS: who supplies 5 V? (FM-1 plugged in now) ==="
for r in /sys/class/regulator/regulator.*; do echo "$r $(cat $r/name 2>/dev/null) $(cat $r/state 2>/dev/null)"; done | grep -i -E "vbus|usb"
ls -la /sys/devices/platform/soc/usbc0 2>&1 | head -40
echo; echo "=== 2. device + descriptors (saved to fm1_descriptors.bin) ==="
python3 fm1tool.py list
python3 fm1tool.py descriptors --save fm1_descriptors.bin
echo; echo "=== 3. does the FM-1 SEND MIDI?  PRESS KEYS / TURN KNOBS NOW (8 s) ==="
python3 fm1tool.py midi-in --seconds 8
echo; echo "=== 4. isochronous capture 4 s while the FM-1 plays middle C (you should HEAR it) ==="
python3 fm1tool.py capture 4 phase0_capture.wav --note 60
echo; echo "=== 5. idle capture 3 s (nothing playing) ==="
python3 fm1tool.py capture 3 phase0_idle.wav
echo; echo "=== done ==="
} 2>&1 | tee phase0_report.txt
