#!/bin/bash
# Deploy HillBand to the RG35XXSP (muOS) over SSH from your Mac.
#
# Usage:
#   bash deploy.sh [host] [sd_root]
#   HOST defaults to 192.168.1.201, SD_ROOT to /mnt/sdcard.
#
# Samples are SHARED at /mnt/sdcard/ROMs/Samples — nothing to copy.
# Auth tip: brew install hudochenkov/sshpass/sshpass && SSHPASS=root bash deploy.sh
set -e

HOST="${1:-192.168.1.201}"
SD_ROOT="${2:-/mnt/sdcard}"
USER_AT="root@$HOST"
PORTS_DIR="$SD_ROOT/ports"
MENU_DIR="$SD_ROOT/ROMS/Ports"
ROOT="$(cd "$(dirname "$0")" && pwd)"
STAGE="$ROOT/dist/stage"

bash "$ROOT/build_port.sh" >/dev/null
echo "[deploy] built staging tree at $STAGE"

CTL="$(mktemp -u "${TMPDIR:-/tmp}/hillband-ssh-XXXXXX")"
cleanup() { $SSH -O exit "$USER_AT" >/dev/null 2>&1 || true; rm -f "$CTL"; }
trap cleanup EXIT
SSH_OPTS="-o StrictHostKeyChecking=no -o ControlMaster=auto -o ControlPath=$CTL -o ControlPersist=120"
SSH="ssh $SSH_OPTS"
SCP="scp $SSH_OPTS"
if command -v sshpass >/dev/null && [ -n "$SSHPASS" ]; then
  SSH="sshpass -e $SSH"
  SCP="sshpass -e $SCP"
  echo "[deploy] using sshpass (non-interactive)"
else
  echo "[deploy] tip: enter the SSH password ('root') once when prompted."
fi

echo "[deploy] checking the device is reachable..."
if ! $SSH "$USER_AT" "true"; then
  echo "[deploy] ERROR: can't SSH to $USER_AT."
  exit 1
fi

echo "[deploy] creating target dirs..."
$SSH "$USER_AT" "mkdir -p '$PORTS_DIR' '$MENU_DIR'"

echo "[deploy] copying game code -> $PORTS_DIR/hillband ..."
$SCP -r "$STAGE/hillband" "$USER_AT:$PORTS_DIR/"

echo "[deploy] copying launcher -> $MENU_DIR/HillBand.sh ..."
$SCP "$STAGE/HillBand.sh" "$USER_AT:$MENU_DIR/HillBand.sh"
$SSH "$USER_AT" "chmod +x '$MENU_DIR/HillBand.sh' '$PORTS_DIR/hillband/install_deps.sh'"

echo "[deploy] building Python deps on-device (first run only; needs wifi)..."
$SSH "$USER_AT" "cd '$PORTS_DIR/hillband' && bash install_deps.sh \"\$(command -v python3)\" ./pylibs" \
  || echo "[deploy] (dep build skipped — will retry on first launch)"

echo "[deploy] done. Launch HillBand from the muOS Ports menu."
echo "[deploy] (shares $SD_ROOT/ROMs/Samples with HillChord and HillSequencer.)"
