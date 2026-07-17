#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"

echo "Uninstalling comma from ${PREFIX} ..."

for f in "$PREFIX/," "$PREFIX/,.config.json" "$PREFIX/,.prompt.md" "$PREFIX/,.old"; do
    if [ -f "$f" ]; then
        rm "$f"
        echo "  Removed $f"
    else
        echo "  Skipped $f (not found)"
    fi
done

# Leftover self-update temp dir
if [ -d "$PREFIX/.comma-update" ]; then
    rm -rf "$PREFIX/.comma-update"
    echo "  Removed $PREFIX/.comma-update"
else
    echo "  Skipped $PREFIX/.comma-update (not found)"
fi

echo ""
echo "Done."
