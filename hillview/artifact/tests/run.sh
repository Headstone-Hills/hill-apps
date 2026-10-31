#!/usr/bin/env bash
# Browser tests for the artifact: parity with the Python reference + end-to-end flows in headless Chromium.
# Needs: python3 + numpy, ffmpeg, node with the `playwright` package installed globally (npm i -g playwright).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
python3 "$here/gen_golden.py" "$tmp"
# the same skeleton the Artifact tool wraps around the page at publish time
python3 - "$here/../hillview.html" "$tmp/index.html" <<'PY'
import sys
body = open(sys.argv[1]).read()
open(sys.argv[2], "w").write('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover"><style>:root{color-scheme:light;padding-top:env(safe-area-inset-top,0px);padding-bottom:env(safe-area-inset-bottom,0px)}body{margin:0;font:14px system-ui}img{max-width:100%}[hidden]{display:none!important}</style></head><body>' + body + '</body></html>')
PY
ffmpeg -y -v error -f lavfi -i "testsrc2=size=640x360:rate=30:duration=3" -f lavfi -i "aevalsrc='0.6*sin(2*PI*70*t)*gt(sin(2*PI*2*t),0)':s=44100:d=3" -c:v libvpx-vp9 -b:v 1M -c:a libopus -shortest "$tmp/clip.webm"
ffmpeg -y -v error -f lavfi -i "sine=f=330:d=3" "$tmp/music.wav"
export NODE_PATH="$(npm root -g)"
node "$here/test_parity.js" "$tmp"
node "$here/test_flows.js" "$tmp"
