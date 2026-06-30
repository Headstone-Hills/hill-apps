#!/bin/bash
# Assemble a PortMaster-installable HillSequencer.zip from the source tree.
#
# Layout produced (PortMaster convention):
#   HillSequencer.zip
#   ├── HillSequencer.sh        <- launcher, lives in ports/ root
#   └── hillsequencer/          <- the gamedir
#       ├── main.py + all modules (config, state, transport, sequencer,
#       │   instruments, loop_player, chain_editor, sequence_manager,
#       │   library_overlay, ui, help_overlay)
#       ├── audio/ input/ theory/
#       ├── install_deps.sh
#       └── port.json
#
# pygame/numpy are NOT bundled; they install on-device on first launch
# (install_deps.sh) so the binaries match the device ABI. Samples are SHARED
# with HillChord (ROMs/Samples) and are NOT included in the zip.
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="$ROOT/dist"
STAGE="$DIST/stage"
GAMEDIR="$STAGE/hillsequencer"

rm -rf "$DIST"
mkdir -p "$GAMEDIR"

# Launcher at the root of the zip.
cp "$ROOT/HillSequencer.sh" "$STAGE/HillSequencer.sh"
chmod +x "$STAGE/HillSequencer.sh"

# Python source modules.
cp "$ROOT/main.py" "$ROOT/config.py" "$ROOT/state.py" "$ROOT/transport.py" \
   "$ROOT/sequencer.py" "$ROOT/instruments.py" "$ROOT/loop_player.py" \
   "$ROOT/chain_editor.py" "$ROOT/sequence_manager.py" \
   "$ROOT/library_overlay.py" "$ROOT/ui.py" "$ROOT/help_overlay.py" "$GAMEDIR/"

# Packages.
cp -r "$ROOT/audio" "$ROOT/input" "$ROOT/theory" "$GAMEDIR/"

# Packaging + metadata.
cp "$ROOT/install_deps.sh" "$ROOT/port.json" "$ROOT/requirements.txt" "$GAMEDIR/"
chmod +x "$GAMEDIR/install_deps.sh"

# Strip caches.
find "$STAGE" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE" -name '*.pyc' -delete

# Zip it.
( cd "$STAGE" && zip -r -q "$DIST/HillSequencer.zip" HillSequencer.sh hillsequencer )
echo "built: $DIST/HillSequencer.zip"
unzip -l "$DIST/HillSequencer.zip" | tail -n +2
