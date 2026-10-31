#!/usr/bin/env bash
# Build libhillview.dylib for Apple Silicon (arm64, macOS >= 11) into prebuilt/macos-arm64/.
#
#   On a Mac:        native Apple clang through CMake, then an ad-hoc codesign (arm64 refuses unsigned code).
#   Anywhere else:   cross-compiles with zig (pip install ziglang); needs no Apple SDK. The linker ad-hoc signs.
#
# Override the zig command with ZIG="/path/to/zig" (default: python3 -m ziglang).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../prebuilt/macos-arm64"
mkdir -p "$out"

if [ "$(uname -s)" = "Darwin" ]; then
  cmake -S "$here" -B "$here/build-macos-arm64" -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 >/dev/null
  cmake --build "$here/build-macos-arm64" -j >/dev/null
  cp -f "$here/build-macos-arm64/libhillview.dylib" "$out/libhillview.dylib"
  codesign --force --sign - "$out/libhillview.dylib"
else
  read -ra ZIGCMD <<<"${ZIG:-python3 -m ziglang}"
  "${ZIGCMD[@]}" version >/dev/null 2>&1 || { echo "zig not found: pip install ziglang (or set ZIG=...)" >&2; exit 1; }
  # The core uses no C++ runtime (see src/hv_util.hpp), so nothing from libc++ is linked or exported and the result
  # depends only on libSystem. tests/test_macos_arm64.py enforces this.
  "${ZIGCMD[@]}" c++ -target aarch64-macos.11.0 -shared -fPIC -fvisibility=hidden -std=c++17 -O3 \
      -fno-math-errno -fno-trapping-math -ffp-contract=off -fno-exceptions -fno-rtti -Wall -Wextra -I"$here/include" \
      "$here/src/hv_api.cpp" "$here/src/hv_kernels.cpp" \
      -Wl,-install_name,@rpath/libhillview.dylib -o "$out/libhillview.dylib"
fi

python3 "$here/tools/macho_info.py" "$out/libhillview.dylib"
if command -v shasum >/dev/null; then (cd "$out" && shasum -a 256 libhillview.dylib > SHA256SUMS); else (cd "$out" && sha256sum libhillview.dylib > SHA256SUMS); fi
cat "$out/SHA256SUMS"
