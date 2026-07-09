#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Installing comma to ${PREFIX} ..."

# Build release binary
echo "Building release binary..."
command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
(cd "$SCRIPT_DIR" && cargo build --release 2>&1)

# Install files
mkdir -p "$PREFIX"
cp "$SCRIPT_DIR/target/release/comma" "$PREFIX/,"

# Only copy config/prompt if they don't exist (don't overwrite user edits)
if [ ! -f "$PREFIX/,.config.json" ]; then
    cp "$SCRIPT_DIR/config.json" "$PREFIX/,.config.json"
    echo "  Created $PREFIX/,.config.json (edit to set your API key)"
else
    echo "  Skipped $PREFIX/,.config.json (already exists)"
fi

if [ ! -f "$PREFIX/,.prompt.md" ]; then
    cp "$SCRIPT_DIR/prompt.md" "$PREFIX/,.prompt.md"
    echo "  Created $PREFIX/,.prompt.md"
else
    echo "  Skipped $PREFIX/,.prompt.md (already exists)"
fi

echo ""
echo "Installed files:"
ls -lh "$PREFIX/," "$PREFIX/,.config.json" "$PREFIX/,.prompt.md"
echo ""
echo "Done. Run ', -h' for usage."
