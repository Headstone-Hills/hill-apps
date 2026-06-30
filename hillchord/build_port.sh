#!/bin/bash
# Assemble a PortMaster-installable HillChord.zip from the source tree.
#
# Layout produced (PortMaster convention):
#   HillChord.zip
#   ├── HillChord.sh            <- launcher, lives in ports/ root
#   └── hillchord/              <- the gamedir
#       ├── main.py + all modules (config, state, audio/, input/, theory/, ui/)
#       ├── install_deps.sh
#       ├── hillchord.gameinfo.xml
#       └── port.json
#
# pygame/numpy are NOT bundled; they are installed on-device on first launch
# (install_deps.sh) so the binaries match the device ABI.
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="$ROOT/dist"
STAGE="$DIST/stage"
GAMEDIR="$STAGE/hillchord"

rm -rf "$DIST"
mkdir -p "$GAMEDIR"

# Launcher at the root of the zip.
cp "$ROOT/HillChord.sh" "$STAGE/HillChord.sh"
chmod +x "$STAGE/HillChord.sh"

# Python source + packages.
cp "$ROOT/main.py" "$ROOT/config.py" "$ROOT/state.py" \
   "$ROOT/persistence.py" "$GAMEDIR/"
cp -r "$ROOT/audio" "$ROOT/input" "$ROOT/theory" "$ROOT/ui" "$GAMEDIR/"

# On-device helper scripts (input probe, sample generator, smoke test).
cp -r "$ROOT/tools" "$GAMEDIR/"

# Packaging + metadata.
cp "$ROOT/install_deps.sh" "$GAMEDIR/"
cp "$ROOT/hillchord.gptk" "$GAMEDIR/"
cp "$ROOT/hillchord.gameinfo.xml" "$GAMEDIR/"
cp "$ROOT/port.json" "$GAMEDIR/"
chmod +x "$GAMEDIR/install_deps.sh"

# Strip caches.
find "$STAGE" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE" -name '*.pyc' -delete

# Zip it.
( cd "$STAGE" && zip -r -q "$DIST/HillChord.zip" HillChord.sh hillchord )
echo "built: $DIST/HillChord.zip"
unzip -l "$DIST/HillChord.zip" | tail -n +2
