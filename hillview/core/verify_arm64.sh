#!/usr/bin/env bash
# Verify the Apple Silicon build as far as a non-Mac machine can.
#   1. Run the C++ self-test as real ARM64 machine code under qemu (aarch64 Linux static binary):
#      checks the NEON build and ARM floating-point behaviour against golden output from the numpy reference.
#   2. Cross-build libhillview.dylib and inspect its Mach-O structure (arch, min macOS, deps, ad-hoc signature).
#   3. Report whether the fresh build is byte-identical to the existing prebuilt/macos-arm64/ one.
# It cannot LOAD the dylib: that needs an Apple Silicon Mac (see README, "Apple Silicon").
# Needs: zig (pip install ziglang; or ZIG=/path/to/zig) and qemu-user (apt install qemu-user) for step 1.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
read -ra ZIGCMD <<<"${ZIG:-python3 -m ziglang}"
FL=(-std=c++17 -O3 -fno-math-errno -fno-trapping-math -ffp-contract=off -fno-exceptions -fno-rtti -Wall -Wextra -I"$here/include" -I"$here/src")
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

echo "== 1. ARM64 self-test under qemu"
if command -v qemu-aarch64 >/dev/null; then
  "${ZIGCMD[@]}" c++ -target aarch64-linux-musl -static "${FL[@]}" "$here/tests/selftest.cpp" "$here/src/hv_api.cpp" "$here/src/hv_kernels.cpp" -o "$tmp/selftest_arm64" -pthread
  qemu-aarch64 "$tmp/selftest_arm64"
else
  echo "   qemu-aarch64 not found: SKIPPED (apt install qemu-user)"
fi

echo "== 2. macOS arm64 dylib"
previous="$(cat "$here/../prebuilt/macos-arm64/SHA256SUMS" 2>/dev/null || true)"
"$here/build_macos_arm64.sh" >/dev/null
python3 "$here/tools/macho_info.py" "$here/../prebuilt/macos-arm64/libhillview.dylib"
if command -v llvm-nm >/dev/null; then
  echo "   exports: $(llvm-nm -gU "$here/../prebuilt/macos-arm64/libhillview.dylib" | awk '{print $3}' | tr '\n' ' ')"
  echo "   imports: $(llvm-nm -u "$here/../prebuilt/macos-arm64/libhillview.dylib" | tr '\n' ' ')"
fi

echo "== 3. reproducibility"
fresh="$(cat "$here/../prebuilt/macos-arm64/SHA256SUMS")"
if [ "$previous" = "$fresh" ]; then echo "   fresh build is byte-identical to the existing prebuilt/macos-arm64/ one"; else echo "   fresh build DIFFERS from the previous one (different zig version?): $fresh"; fi
