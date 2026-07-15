#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
REPO="miuzel/comma-cli"

echo "Installing comma to ${PREFIX} ..."

# If Cargo.toml exists in script's directory, build from source
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || true)"
if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
    echo "Building from source..."
    command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
    (cd "$SCRIPT_DIR" && cargo build --release 2>&1)
    mkdir -p "$PREFIX"
    cp "$SCRIPT_DIR/target/release/comma" "$PREFIX/,"

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
else
    # Download pre-built binary from GitHub releases
    echo "Downloading from GitHub releases..."
    LATEST_URL="https://github.com/${REPO}/releases/latest/download/comma"

    mkdir -p "$PREFIX"

    if command -v curl >/dev/null 2>&1; then
        curl -sSL "$LATEST_URL" -o "$PREFIX/comma.tmp"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$LATEST_URL" -O "$PREFIX/comma.tmp"
    else
        echo "Error: curl or wget required"
        exit 1
    fi

    mv "$PREFIX/comma.tmp" "$PREFIX/,"
    chmod +x "$PREFIX/,"

    if [ ! -f "$PREFIX/,.config.json" ]; then
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/config.json" -o "$PREFIX/,.config.json" 2>/dev/null || true
        echo "  Created $PREFIX/,.config.json (edit to set your API key)"
    else
        echo "  Skipped $PREFIX/,.config.json (already exists)"
    fi

    if [ ! -f "$PREFIX/,.prompt.md" ]; then
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/prompt.md" -o "$PREFIX/,.prompt.md" 2>/dev/null || true
        echo "  Created $PREFIX/,.prompt.md"
    else
        echo "  Skipped $PREFIX/,.prompt.md (already exists)"
    fi
fi

echo ""
echo "Installed files:"
ls -lh "$PREFIX/," 2>/dev/null || true
[ -f "$PREFIX/,.config.json" ] && ls -lh "$PREFIX/,.config.json"
[ -f "$PREFIX/,.prompt.md" ] && ls -lh "$PREFIX/,.prompt.md"
echo ""
echo "Done. Run ', -h' for usage."
