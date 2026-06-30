#!/bin/bash
# Deploy HillChord to the RG35XXSP over SSH and trigger the on-device dep build.
# Usage: bash deploy.sh [host] [ports_dir]
# Requires the device on the network. Will prompt for the SSH password unless
# sshpass is installed (then set SSHPASS=root in the environment).
set -e

HOST="${1:-192.168.1.201}"
PORTS_DIR="${2:-/mnt/sdcard/ports}"     # adjust if your muOS ports live elsewhere
ROOT="$(cd "$(dirname "$0")" && pwd)"

bash "$ROOT/build_port.sh"

SSH="ssh -o StrictHostKeyChecking=no root@$HOST"
SCP="scp -o StrictHostKeyChecking=no"
if command -v sshpass >/dev/null && [ -n "$SSHPASS" ]; then
  SSH="sshpass -e $SSH"
  SCP="sshpass -e $SCP"
fi

echo "[deploy] copying zip..."
$SCP "$ROOT/dist/HillChord.zip" "root@$HOST:/tmp/HillChord.zip"

echo "[deploy] unpacking into $PORTS_DIR ..."
$SSH "mkdir -p '$PORTS_DIR' && cd '$PORTS_DIR' && unzip -o /tmp/HillChord.zip && rm /tmp/HillChord.zip"

echo "[deploy] building python deps on-device (first run only)..."
$SSH "cd '$PORTS_DIR/hillchord' && bash install_deps.sh \"\$(command -v python3)\" ./pylibs"

echo "[deploy] done. Launch HillChord from the muOS ports menu."
