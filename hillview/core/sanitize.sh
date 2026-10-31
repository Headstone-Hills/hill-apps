#!/usr/bin/env bash
# Run the native-core tests under AddressSanitizer + UBSan (Linux, g++). Any overflow/UB aborts the run.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$(mktemp -d)"
g++ -std=c++17 -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer \
    -fno-math-errno -fno-trapping-math -DHV_NO_MULTIVERSION -fPIC -shared -I"$here/include" \
    "$here/src/hv_api.cpp" "$here/src/hv_kernels.cpp" -o "$out/libhillview.so" -pthread
cd "$here/.."
HILLVIEW_LIB="$out/libhillview.so" LD_PRELOAD="$(g++ -print-file-name=libasan.so) $(g++ -print-file-name=libubsan.so)" \
ASAN_OPTIONS=detect_leaks=0 python3 -m pytest -q tests/test_native.py
rm -rf "$out"
