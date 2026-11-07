#!/bin/bash
# Plain .tar on purpose: the SP runs BusyBox tar, which has no -z.
# Build pocketdaw-phase0.tar: everything needed to run Phase 0 on the SP with no computer/git/network.
set -e
cd "$(dirname "$0")"
bash build_helper.sh
OUT="${1:-pocketdaw-phase0.tar}"
tar --exclude="__pycache__" -cf "$OUT" *.py phase0.sh check_env.sh audio_test.sh tests dist/isocap build_helper.sh helper
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
