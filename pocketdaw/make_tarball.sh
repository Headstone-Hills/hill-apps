#!/bin/bash
# Build pocketdaw-phase0.tar.gz: everything needed to run Phase 0 on the SP with no computer/git/network.
set -e
cd "$(dirname "$0")"
bash build_helper.sh
OUT="${1:-pocketdaw-phase0.tar.gz}"
tar --exclude='__pycache__' -czf "$OUT" *.py phase0.sh tests dist/isocap build_helper.sh helper
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
