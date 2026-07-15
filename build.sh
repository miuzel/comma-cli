#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Building comma from source..."
command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
(cd "$SCRIPT_DIR" && cargo build --release 2>&1)

mkdir -p "$PREFIX"
cp "$SCRIPT_DIR/target/release/comma" "$PREFIX/,"

for f in config.json prompt.md; do
    target="$PREFIX/,.${f}"
    if [ ! -f "$target" ]; then
        cp "$SCRIPT_DIR/$f" "$target"
        echo "  Created $target"
    else
        echo "  Skipped $target (already exists)"
    fi
done

echo ""
echo "Done. Run ', -h' for usage."
