#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
REPO="miuzel/comma-cli"

echo "Installing comma to ${PREFIX} ..."

# Always download from GitHub releases
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

# Download config files if not exists
for f in config.json prompt.md; do
    target="$PREFIX/,.${f}"
    if [ ! -f "$target" ]; then
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/${f}" -o "$target" 2>/dev/null || true
        echo "  Created $target"
    else
        echo "  Skipped $target (already exists)"
    fi
done

echo ""
echo "Installed files:"
ls -lh "$PREFIX/," 2>/dev/null || true
[ -f "$PREFIX/,.config.json" ] && ls -lh "$PREFIX/,.config.json"
[ -f "$PREFIX/,.prompt.md" ] && ls -lh "$PREFIX/,.prompt.md"
echo ""
echo "Done. Run ', -h' for usage."
