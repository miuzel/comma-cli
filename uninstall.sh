#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"

echo "Uninstalling comma from ${PREFIX} ..."

for f in "$PREFIX/," "$PREFIX/,.old"; do
    if [ -f "$f" ]; then
        rm "$f"
        echo "  Removed $f"
    else
        echo "  Skipped $f (not found)"
    fi
done

# XDG cache (Linux/macOS) — generated data, safe to remove
XDG_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/comma"
if [ -f "$XDG_CACHE/cache.json" ]; then
    rm "$XDG_CACHE/cache.json"
    rmdir "$XDG_CACHE" 2>/dev/null || true
    echo "  Removed $XDG_CACHE/cache.json"
else
    echo "  Skipped $XDG_CACHE/cache.json (not found)"
fi

# %APPDATA%\comma cache (Windows, when run from Git Bash/MSYS)
if [ -n "${APPDATA:-}" ]; then
    WIN_APPDATA="$(cygpath -u "$APPDATA" 2>/dev/null || echo "$APPDATA")/comma"
    if [ -f "$WIN_APPDATA/cache.json" ]; then
        rm "$WIN_APPDATA/cache.json"
        echo "  Removed $WIN_APPDATA/cache.json"
    fi
    rmdir "$WIN_APPDATA" 2>/dev/null || true
fi

# Leftover self-update temp dir
if [ -d "$PREFIX/.comma-update" ]; then
    rm -rf "$PREFIX/.comma-update"
    echo "  Removed $PREFIX/.comma-update"
else
    echo "  Skipped $PREFIX/.comma-update (not found)"
fi

# User-defined files are kept: config.json, prompt.md, additional_prompt.md
# (in ~/.config/comma/, %APPDATA%\comma\, or legacy ,.config.json/,.prompt.md
# next to the binary). Remove them manually if you want a clean slate.
XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/comma"
echo ""
echo "Kept your configuration files (delete manually if unwanted):"
for f in "$XDG_CONFIG/config.json" "$XDG_CONFIG/prompt.md" "$XDG_CONFIG/additional_prompt.md" \
         "$PREFIX/,.config.json" "$PREFIX/,.prompt.md"; do
    [ -f "$f" ] && echo "  $f"
done

echo ""
echo "Done."
