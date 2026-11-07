#!/bin/bash
# ./build.sh test     host unit tests
# ./build.sh device   static aarch64 binaries -> dist/   (needs: rustup target add aarch64-unknown-linux-musl; pip install ziglang)
set -e
cd "$(dirname "$0")"
case "${1:-test}" in
  test)   cargo test --workspace ;;
  device) rustup target add aarch64-unknown-linux-musl >/dev/null
          cargo build --release --target aarch64-unknown-linux-musl
          mkdir -p dist && cp target/aarch64-unknown-linux-musl/release/fm1tool dist/fm1tool
          ls -l dist ;;
  *) echo "usage: $0 test|device"; exit 1 ;;
esac
