#!/bin/bash
# Deploy HillSequencer to the RG35XXSP (muOS) over SSH from your Mac.
#
# Usage:
#   bash deploy.sh [host] [sd_root]
#   HOST defaults to 192.168.1.201, SD_ROOT to /mnt/sdcard.
#
# muOS builds the Ports menu from <sd>/ROMS/Ports/*.sh, while game code lives in
# lowercase <sd>/ports/<name>/. So we place them SEPARATELY (a launcher left in
# lowercase ports/ would NOT appear in the menu):
#   launcher -> /mnt/sdcard/ROMS/Ports/HillSequencer.sh
#   code     -> /mnt/sdcard/ports/hillsequencer/
# Samples are shared with HillChord at /mnt/sdcard/ROMs/Samples (already there).
#
# Auth: the device's SSH password is "root". By default scp/ssh will PROMPT for
# it (a few times). To avoid prompts, install sshpass and export SSHPASS=root:
#   brew install hudochenkov/sshpass/sshpass   &&   SSHPASS=root bash deploy.sh
set -e

HOST="${1:-192.168.1.201}"
SD_ROOT="${2:-/mnt/sdcard}"
USER_AT="root@$HOST"
PORTS_DIR="$SD_ROOT/ports"
MENU_DIR="$SD_ROOT/ROMS/Ports"
ROOT="$(cd "$(dirname "$0")" && pwd)"
STAGE="$ROOT/dist/stage"

# 1) Assemble the staged tree (dist/stage/HillSequencer.sh + dist/stage/hillsequencer/).
bash "$ROOT/build_port.sh" >/dev/null
echo "[deploy] built staging tree at $STAGE"

# Connection multiplexing: the first connection opens a master socket that every
# later ssh/scp reuses, so the password is typed only ONCE for the whole deploy.
CTL="$(mktemp -u "${TMPDIR:-/tmp}/hillseq-ssh-XXXXXX")"
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
  echo "[deploy] ERROR: can't SSH to $USER_AT. Power on the device, connect it to"
  echo "         wifi, confirm its IP (muOS: Settings > Wi-Fi), then re-run with"
  echo "         that IP:  bash deploy.sh <ip>"
  exit 1
fi

echo "[deploy] creating target dirs..."
$SSH "$USER_AT" "mkdir -p '$PORTS_DIR' '$MENU_DIR'"

echo "[deploy] copying game code -> $PORTS_DIR/hillsequencer ..."
$SCP -r "$STAGE/hillsequencer" "$USER_AT:$PORTS_DIR/"

echo "[deploy] copying launcher -> $MENU_DIR/HillSequencer.sh ..."
$SCP "$STAGE/HillSequencer.sh" "$USER_AT:$MENU_DIR/HillSequencer.sh"
$SSH "$USER_AT" "chmod +x '$MENU_DIR/HillSequencer.sh' '$PORTS_DIR/hillsequencer/install_deps.sh'"

echo "[deploy] building Python deps on-device (first run only; needs wifi)..."
$SSH "$USER_AT" "cd '$PORTS_DIR/hillsequencer' && bash install_deps.sh \"\$(command -v python3)\" ./pylibs" \
  || echo "[deploy] (dep build skipped/failed — it will retry automatically on first launch)"

echo "[deploy] done. Launch HillSequencer from the muOS Ports menu."
echo "[deploy] (it shares HillChord's $SD_ROOT/ROMs/Samples — no samples copied.)"
