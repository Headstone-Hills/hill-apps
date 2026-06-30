#!/bin/bash
# Assemble a PortMaster-installable HillBand.zip from the source tree.
#
# Layout produced (PortMaster convention):
#   HillBand.zip
#   ├── HillBand.sh          <- launcher, lives in ports/ root
#   └── hillband/            <- the gamedir
#       ├── main.py + all modules
#       ├── audio/ input/ theory/
#       ├── install_deps.sh
#       └── port.json
#
# Samples are SHARED with HillChord/HillSequencer (ROMs/Samples) and are NOT
# included.  pygame/numpy install on-device on first launch via install_deps.sh.
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="$ROOT/dist"
STAGE="$DIST/stage"
GAMEDIR="$STAGE/hillband"

rm -rf "$DIST"
mkdir -p "$GAMEDIR"

# Launcher
cp "$ROOT/HillBand.sh" "$STAGE/HillBand.sh"
chmod +x "$STAGE/HillBand.sh"

# Python source modules
cp "$ROOT/main.py"             \
   "$ROOT/config.py"           \
   "$ROOT/state.py"            \
   "$ROOT/transport.py"        \
   "$ROOT/sequencer.py"        \
   "$ROOT/instruments.py"      \
   "$ROOT/loop_player.py"      \
   "$ROOT/library_overlay.py"  \
   "$ROOT/chain_editor.py"     \
   "$ROOT/sequence_manager.py" \
   "$ROOT/help_overlay.py"     \
   "$ROOT/track_mode_overlay.py" \
   "$ROOT/ui.py"               \
   "$GAMEDIR/"

# Packages
cp -r "$ROOT/audio" "$ROOT/input" "$ROOT/theory" "$GAMEDIR/"

# Metadata + installer
cp "$ROOT/install_deps.sh" "$ROOT/port.json" "$ROOT/requirements.txt" "$GAMEDIR/"
chmod +x "$GAMEDIR/install_deps.sh"

# Strip caches
find "$STAGE" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE" -name '*.pyc' -delete

# Zip
( cd "$STAGE" && zip -r -q "$DIST/HillBand.zip" HillBand.sh hillband )
echo "built: $DIST/HillBand.zip"
unzip -l "$DIST/HillBand.zip" | tail -n +2
