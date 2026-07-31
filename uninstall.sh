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

# XDG config (Linux/macOS)
XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/comma"
for f in config.json prompt.md; do
    if [ -f "$XDG_CONFIG/$f" ]; then
        rm "$XDG_CONFIG/$f"
        echo "  Removed $XDG_CONFIG/$f"
    else
        echo "  Skipped $XDG_CONFIG/$f (not found)"
    fi
done
rmdir "$XDG_CONFIG" 2>/dev/null || true

# XDG cache (Linux/macOS)
XDG_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/comma"
if [ -f "$XDG_CACHE/cache.json" ]; then
    rm "$XDG_CACHE/cache.json"
    rmdir "$XDG_CACHE" 2>/dev/null || true
    echo "  Removed $XDG_CACHE/cache.json"
else
    echo "  Skipped $XDG_CACHE/cache.json (not found)"
fi

# %APPDATA%\comma (Windows, when run from Git Bash/MSYS)
if [ -n "${APPDATA:-}" ]; then
    WIN_APPDATA="$(cygpath -u "$APPDATA" 2>/dev/null || echo "$APPDATA")/comma"
    for f in config.json prompt.md cache.json; do
        if [ -f "$WIN_APPDATA/$f" ]; then
            rm "$WIN_APPDATA/$f"
            echo "  Removed $WIN_APPDATA/$f"
        fi
    done
    rmdir "$WIN_APPDATA" 2>/dev/null || true
fi

# Leftover self-update temp dir
if [ -d "$PREFIX/.comma-update" ]; then
    rm -rf "$PREFIX/.comma-update"
    echo "  Removed $PREFIX/.comma-update"
else
    echo "  Skipped $PREFIX/.comma-update (not found)"
fi

echo ""
echo "Done."
