#!/bin/bash
# Deploy PocketDAW to the RG35XXSP over SSH (dev tooling only - the app itself never needs a network).
#
#   bash deploy.sh                 build helper, copy code to the SP
#   bash deploy.sh phase0          ...then run the Phase 0 hardware proof on the SP and print the report
#   bash deploy.sh run <args...>   run `python3 fm1tool.py <args>` on the SP, e.g.  bash deploy.sh run list
#
# Host/user come from env or a git-ignored .deploy.env next to this script:
#     POCKETDAW_HOST=192.168.1.201      (DHCP - it changes; update it)
#     POCKETDAW_USER=root
#     POCKETDAW_DIR=/mnt/sdcard/ports/pocketdaw
# The SSH password is never stored: ssh prompts once (connection is multiplexed for the whole run).
set -e
ROOT="$(cd "$(dirname "$0")" && pwd)"
[ -f "$ROOT/.deploy.env" ] && . "$ROOT/.deploy.env"
HOST="${POCKETDAW_HOST:-192.168.1.201}"
USER_="${POCKETDAW_USER:-root}"
DIR="${POCKETDAW_DIR:-/mnt/sdcard/ports/pocketdaw}"
TARGET="$USER_@$HOST"

CTL="$(mktemp -u "${TMPDIR:-/tmp}/pocketdaw-ssh-XXXXXX")"
SSH_OPTS="-o StrictHostKeyChecking=no -o ControlMaster=auto -o ControlPath=$CTL -o ControlPersist=300"
trap 'ssh $SSH_OPTS -O exit "$TARGET" >/dev/null 2>&1 || true; rm -f "$CTL"' EXIT
SSH="ssh $SSH_OPTS"
if command -v sshpass >/dev/null && [ -n "$SSHPASS" ]; then SSH="sshpass -e $SSH"; fi

sync_code() {
  bash "$ROOT/build_helper.sh" >/dev/null
  echo "[deploy] -> $TARGET:$DIR"
  $SSH "$TARGET" "mkdir -p '$DIR'" || { echo "[deploy] cannot reach $TARGET (IP changed? set POCKETDAW_HOST)"; exit 1; }
  # tar over ssh: the SP has no rsync guarantee
  ( cd "$ROOT" && tar --exclude='__pycache__' --exclude='.deploy.env' --exclude='dist/isocap-host' \
      -cf - *.py tests dist/isocap build_helper.sh 2>/dev/null ) | $SSH "$TARGET" "cd '$DIR' && tar -xf - && cp dist/isocap ./isocap && chmod +x ./isocap"
  echo "[deploy] done"
}

remote() { $SSH "$TARGET" "cd '$DIR' && $*"; }

case "${1:-}" in
  run)    shift; sync_code; remote "python3 fm1tool.py $*" ;;
  phase0)
    sync_code
    echo; echo "=== 1. device + descriptors (also saved to fm1_descriptors.bin) ==="
    remote "python3 fm1tool.py list; python3 fm1tool.py descriptors --save fm1_descriptors.bin"
    echo; echo "=== 2. does the FM-1 SEND MIDI? PRESS KEYS / TURN KNOBS ON THE FM-1 NOW (8 s) ==="
    remote "python3 fm1tool.py midi-in --seconds 8"
    echo; echo "=== 3. isochronous capture, 4 s, FM-1 plays middle C from MIDI. Expect: a note you can hear on the FM-1 ==="
    remote "python3 fm1tool.py capture 4 phase0_capture.wav --note 60"
    echo; echo "=== 4. idle capture, 3 s, nothing playing (noise floor) ==="
    remote "python3 fm1tool.py capture 3 phase0_idle.wav"
    echo; echo "Copy results off the SP for inspection:"
    echo "  scp $TARGET:$DIR/phase0_capture.wav $TARGET:$DIR/fm1_descriptors.bin ."
    ;;
  *) sync_code ;;
esac
