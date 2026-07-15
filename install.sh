#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
REPO="miuzel/comma-cli"

echo "Installing comma to ${PREFIX} ..."

# Detect if stdin is a pipe (curl | bash) or a file (./install.sh)
if [ ! -t 0 ]; then
    # Piped: download from GitHub
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

    # Download config files
    for f in ",.config.json" ",.prompt.md"; do
        if [ ! -f "$PREFIX/$f" ]; then
            curl -sSL "https://raw.githubusercontent.com/${REPO}/main/${f#,}" -o "$PREFIX/$f" 2>/dev/null || true
            echo "  Created $PREFIX/$f"
        else
            echo "  Skipped $PREFIX/$f (already exists)"
        fi
    done
else
    # Local file: build from source
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
        echo "Building from source..."
        command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
        (cd "$SCRIPT_DIR" && cargo build --release 2>&1)
        mkdir -p "$PREFIX"
        cp "$SCRIPT_DIR/target/release/comma" "$PREFIX/,"

        for f in "config.json" "prompt.md"; do
            target="$PREFIX/,.${f}"
            if [ ! -f "$target" ]; then
                cp "$SCRIPT_DIR/$f" "$target"
                echo "  Created $target"
            else
                echo "  Skipped $target (already exists)"
            fi
        done
    else
        echo "Error: Cargo.toml not found in $SCRIPT_DIR"
        exit 1
    fi
fi

echo ""
echo "Installed files:"
ls -lh "$PREFIX/," 2>/dev/null || true
[ -f "$PREFIX/,.config.json" ] && ls -lh "$PREFIX/,.config.json"
[ -f "$PREFIX/,.prompt.md" ] && ls -lh "$PREFIX/,.prompt.md"
echo ""
echo "Done. Run ', -h' for usage."
