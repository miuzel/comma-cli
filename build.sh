#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST="$SCRIPT_DIR/target/dist"

echo "Building comma from source..."
command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
(cd "$SCRIPT_DIR" && cargo build --release 2>&1)

# Stage everything into target/dist
rm -rf "$DIST"
mkdir -p "$DIST"
cp "$SCRIPT_DIR/target/release/comma" "$DIST/comma"
cp "$SCRIPT_DIR/config.json" "$DIST/,config.json"
echo "Staged to $DIST"

# Install to PREFIX
mkdir -p "$PREFIX"
cp "$DIST/comma" "$PREFIX/,"

# The config template goes to the XDG location on Linux/macOS; existing
# legacy files in ~/.local/bin are respected and left alone. The prompt
# template is compiled into the binary — no prompt.md is installed, so
# upgrades to the default prompt take effect (customize via
# additional_prompt.md or the full_prompt config key).
XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/comma"
if [ ! -f "$XDG_CONFIG/config.json" ] && [ ! -f "$PREFIX/,.config.json" ]; then
    mkdir -p "$XDG_CONFIG"
    cp "$DIST/,config.json" "$XDG_CONFIG/config.json"
    echo "  Created $XDG_CONFIG/config.json"
else
    echo "  Skipped config (already exists)"
fi

echo ""
echo "Done. Run ', -h' for usage."
