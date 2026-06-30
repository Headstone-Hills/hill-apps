#!/bin/bash
# Install HillBeat's Python deps on-device (run once).
# Uses device's own Python so binaries match device ABI.
# Usage: bash install_deps.sh "$(command -v python3)" ./pylibs
set -e

PYTHON="${1:-python3}"
TARGET="${2:-./pylibs}"

mkdir -p "$TARGET"

if ! "$PYTHON" -m pip --version >/dev/null 2>&1; then
  echo "[install_deps] pip not found for $PYTHON; bootstrapping with ensurepip"
  "$PYTHON" -m ensurepip --upgrade || {
    echo "[install_deps] ERROR: no pip available. Install pip on the device."
    exit 1
  }
fi

"$PYTHON" -m pip install --no-cache-dir --target "$TARGET" \
  "pygame>=2.6,<3" "numpy>=1.21"

echo "[install_deps] done -> $TARGET"
"$PYTHON" - "$TARGET" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import pygame, numpy
print("pygame", pygame.version.ver, "/ numpy", numpy.__version__)
PY
