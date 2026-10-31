#!/usr/bin/env bash
# Build libhillview and copy it next to the Python package so `import hillview` finds it.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cmake -S "$here" -B "$here/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$here/build" -j >/dev/null
mkdir -p "$here/../hillview/_native"
cp -f "$here"/build/libhillview.so* "$here"/build/libhillview.dylib "$here"/build/hillview.dll "$here/../hillview/_native/" 2>/dev/null || true
ls "$here/../hillview/_native"
