#!/bin/bash
# Build/vendor HillChord's Python deps on-device (run once, on the SP itself).
#
# Using the device's own Python ensures the pygame/numpy binaries match the
# device ABI. Invoked automatically on first launch by HillChord.sh, or run
# manually:  bash install_deps.sh "$(command -v python3)" ./pylibs
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

# Install into the vendored target dir (needs network on first run).
"$PYTHON" -m pip install --no-cache-dir --target "$TARGET" \
  "pygame>=2.6,<3" "numpy>=1.26"

echo "[install_deps] done -> $TARGET"
"$PYTHON" - "$TARGET" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import pygame, numpy
print("pygame", pygame.version.ver, "/ numpy", numpy.__version__)
PY
