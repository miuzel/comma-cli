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
for f in config.json prompt.md; do
    cp "$SCRIPT_DIR/$f" "$DIST/,$f"
done
echo "Staged to $DIST"

# Install to PREFIX
mkdir -p "$PREFIX"
cp "$DIST/comma" "$PREFIX/,"

for f in config.json prompt.md; do
    target="$PREFIX/,.${f}"
    if [ ! -f "$target" ]; then
        cp "$DIST/,$f" "$target"
        echo "  Created $target"
    else
        echo "  Skipped $target (already exists)"
    fi
done

echo ""
echo "Done. Run ', -h' for usage."
