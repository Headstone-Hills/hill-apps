#!/bin/sh
# Linker shim: Rust -> Zig's bundled clang/lld, producing a static aarch64 binary (no device toolchain needed).
exec python3 -m ziglang cc -target aarch64-linux-musl "$@"
