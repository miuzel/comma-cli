#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"

echo "Uninstalling comma from ${PREFIX} ..."

for f in "$PREFIX/," "$PREFIX/,.config.json" "$PREFIX/,.prompt.md"; do
    if [ -f "$f" ]; then
        rm "$f"
        echo "  Removed $f"
    else
        echo "  Skipped $f (not found)"
    fi
done

echo ""
echo "Done."
