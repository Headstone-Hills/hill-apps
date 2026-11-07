#!/bin/bash
# Build the isochronous capture helper.
#   host  : for compile-checking on this machine          -> dist/isocap-host
#   aarch64: static binary for the RG35XXSP (no compiler needed on the device) -> dist/isocap
# Cross-compiling uses Zig's bundled toolchain:  pip install ziglang
set -e
cd "$(dirname "$0")"
mkdir -p dist
cc -O2 -Wall -Wextra -o dist/isocap-host helper/isocap.c
echo "built dist/isocap-host"
if python3 -m ziglang version >/dev/null 2>&1; then
  python3 -m ziglang cc -target aarch64-linux-musl -static -O2 -s -o dist/isocap helper/isocap.c
  echo "built dist/isocap (aarch64 static)"
else
  echo "skip aarch64: pip install ziglang"
fi
