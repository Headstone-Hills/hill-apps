#!/bin/bash
# Build helper for the Rust Hill apps.
#
#   ./build.sh test      run all tests
#   ./build.sh web       -> dist/hill-apps.html (single self-contained file)
#   ./build.sh device    -> dist/ports/*.zip   (aarch64, PortMaster layout)
#   ./build.sh all       test + web + device
#
# `device` needs: rustup target aarch64-unknown-linux-gnu, cargo-zigbuild and
# zig (pip install ziglang), curl, ar, zstd. It fetches the arm64 libSDL2 once
# (link-time only; the device's own libSDL2 is used at run time) and targets
# glibc 2.28 so the binaries run on older firmware.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
DIST="$ROOT/dist"

APPS=(hillbeat hillsequencer hillband hillchord)
declare -A TITLE=([hillbeat]=HillBeat [hillsequencer]=HillSequencer [hillband]=HillBand [hillchord]=HillChord [hillmap]=HillMap)
declare -A DESC=(
  [hillbeat]="4-voice drum machine: 16 steps, 8 patterns, swing, backing loop"
  [hillsequencer]="8-track pitched step sequencer with a key/scale spreader"
  [hillband]="4 melodic + 4 drum tracks with chords, scale modes and a backing loop"
  [hillchord]="Live chord/note instrument with looper, arpeggiator and metronome"
  [hillmap]="Button mapping wizard: writes buttons.cfg for the Hill apps"
)

do_test() { cargo test --workspace; }

do_web() {
  rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
  cargo build --release --target wasm32-unknown-unknown -p hill-web
  python3 hill-web/build_html.py target/wasm32-unknown-unknown/release/hill_web.wasm "$DIST/hill-apps.html"
}

fetch_sdl2() {
  local dir="$ROOT/.cache/arm64sdl"
  if [ ! -e "$dir/lib/libSDL2.so" ]; then
    mkdir -p "$dir" && cd "$dir"
    curl -fsSL http://ports.ubuntu.com/ubuntu-ports/dists/noble/main/binary-arm64/Packages.gz | gunzip > Packages
    local file
    file=$(awk '/^Package: libsdl2-2.0-0$/{f=1} f&&/^Filename:/{print $2; exit}' Packages)
    curl -fsSL -O "http://ports.ubuntu.com/ubuntu-ports/$file"
    ar x libsdl2-2.0-0_*.deb data.tar.zst
    tar --use-compress-program=unzstd -xf data.tar.zst
    mkdir -p lib
    ln -sf "$dir"/usr/lib/aarch64-linux-gnu/libSDL2-2.0.so.0.* lib/libSDL2.so
    cd "$ROOT"
  fi
  echo "$ROOT/.cache/arm64sdl/lib"
}

do_device() {
  local libdir
  libdir=$(fetch_sdl2)
  RUSTFLAGS="-L $libdir -C link-arg=-Wl,--allow-shlib-undefined" \
    cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28 -p hill-device
  local bin="$ROOT/target/aarch64-unknown-linux-gnu/release"
  rm -rf "$DIST/ports"; mkdir -p "$DIST/ports"
  # NAME_SUFFIX (e.g. NAME_SUFFIX=-rs) renames launcher, folder and binary so
  # they can sit next to the Python ports (HillBeat.sh / hillbeat/) without clashing.
  local sfx="${NAME_SUFFIX:-}"
  for b in "${APPS[@]}" hillmap; do
    local a="$b$sfx" t="${TITLE[$b]}${sfx^^}"
    local stage="$DIST/ports/stage/$a"
    mkdir -p "$stage/$a"
    sed -e "s|@NAME@|$t|g" -e "s|@DIR@|$a|g" -e "s|@DESC@|${DESC[$b]}|g" ports/launcher.sh.in > "$stage/$t.sh"
    chmod +x "$stage/$t.sh"
    cp "$bin/$b" "$stage/$a/$a"
    cat > "$stage/$a/port.json" <<JSON
{
  "version": 1,
  "name": "$t.zip",
  "items": ["$t.sh", "$a/"],
  "attr": {
    "title": "$t",
    "desc": "${DESC[$b]}.",
    "inst": "Put .wav samples in the shared Samples folder (see README), or set HILL_SAMPLES. Built-in sounds work without any samples.",
    "genres": ["music", "tools"],
    "porter": ["you"],
    "image": { "screenshot": null, "covers": [], "thumbnail": null, "video": null },
    "rtr": true,
    "runtime": null,
    "reqs": ["sdl2"]
  }
}
JSON
    (cd "$DIST/ports/stage/$a" && zip -qr "$DIST/ports/$t.zip" "$t.sh" "$a")
    echo "built $DIST/ports/$t.zip"
  done
  rm -rf "$DIST/ports/stage"
}

case "${1:-all}" in
  test) do_test ;;
  web) do_web ;;
  device) do_device ;;
  all) do_test; do_web; do_device ;;
  *) echo "usage: $0 {test|web|device|all}"; exit 2 ;;
esac
