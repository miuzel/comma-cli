#!/usr/bin/env bash
set -euo pipefail

PREFIX="${HOME}/.local/bin"
REPO="miuzel/comma-cli"

# Detect platform and architecture
detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) echo "Error: unsupported OS $(uname -s)"; exit 1 ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo "Error: unsupported architecture $(uname -m)"; exit 1 ;;
    esac
    echo "${os}-${arch}"
}

PLATFORM=$(detect_platform)
echo "Installing comma for ${PLATFORM} to ${PREFIX} ..."

# Download from GitHub releases
if [ "$PLATFORM" = "windows-x86_64" ]; then
    ARCHIVE="comma-windows-x86_64.zip"
    BINARY="comma.exe"
else
    ARCHIVE="comma-${PLATFORM}.tar.gz"
    BINARY="comma"
fi

DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${ARCHIVE}"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading ${ARCHIVE}..."
if command -v curl >/dev/null 2>&1; then
    curl -sSL "$DOWNLOAD_URL" -o "$TMPDIR/$ARCHIVE"
elif command -v wget >/dev/null 2>&1; then
    wget -q "$DOWNLOAD_URL" -O "$TMPDIR/$ARCHIVE"
else
    echo "Error: curl or wget required"
    exit 1
fi

# Verify archive integrity against sha256sums.txt from the release
verify_archive() {
    local hash_cmd
    if command -v sha256sum >/dev/null 2>&1; then
        hash_cmd="sha256sum"
    elif command -v shasum >/dev/null 2>&1; then
        hash_cmd="shasum -a 256"
    else
        echo "⚠  No sha256sum/shasum available; skipping integrity check"
        return 0
    fi

    # Older releases may not ship sha256sums.txt
    local sums_url="https://github.com/${REPO}/releases/latest/download/sha256sums.txt"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$sums_url" -o "$TMPDIR/sha256sums.txt" 2>/dev/null || {
            echo "⚠  sha256sums.txt not available; skipping integrity check"; return 0; }
    else
        wget -q "$sums_url" -O "$TMPDIR/sha256sums.txt" 2>/dev/null || {
            echo "⚠  sha256sums.txt not available; skipping integrity check"; return 0; }
    fi

    local expected actual
    expected=$(awk -v name="$ARCHIVE" '$2 == name {print $1}' "$TMPDIR/sha256sums.txt")
    if [ -z "$expected" ]; then
        echo "Error: sha256sums.txt has no entry for ${ARCHIVE}"
        exit 1
    fi
    actual=$($hash_cmd "$TMPDIR/$ARCHIVE" | awk '{print $1}')
    if [ "$actual" != "$expected" ]; then
        echo "Error: checksum mismatch for ${ARCHIVE}"
        echo "  expected: ${expected}"
        echo "  actual:   ${actual}"
        exit 1
    fi
    echo "  Checksum verified"
}
verify_archive

# Extract
mkdir -p "$PREFIX"
if [ "$PLATFORM" = "windows-x86_64" ]; then
    cd "$TMPDIR" && unzip -qo "$ARCHIVE" && cd -
    cp "$TMPDIR/$BINARY" "$PREFIX/comma.exe"
else
    tar xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"
    cp "$TMPDIR/$BINARY" "$PREFIX/,"
    chmod +x "$PREFIX/,"
fi

# Download config/prompt files if not exists. The config template goes to
# the XDG location on Linux/macOS; an existing legacy config is respected.
# Windows (Git Bash/MSYS) keeps the legacy path beside the binary.
XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/comma"
LEGACY_CONFIG="$PREFIX/,.config.json"
if [ "$PLATFORM" != "windows-x86_64" ]; then
    if [ ! -f "$XDG_CONFIG/config.json" ] && [ ! -f "$LEGACY_CONFIG" ]; then
        mkdir -p "$XDG_CONFIG"
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/config.json" -o "$XDG_CONFIG/config.json" 2>/dev/null || true
        echo "  Created $XDG_CONFIG/config.json"
        CONFIG_FILE="$XDG_CONFIG/config.json"
    else
        echo "  Skipped config (already exists)"
        [ -f "$XDG_CONFIG/config.json" ] && CONFIG_FILE="$XDG_CONFIG/config.json" || CONFIG_FILE="$LEGACY_CONFIG"
    fi
    if [ ! -f "$XDG_CONFIG/prompt.md" ] && [ ! -f "$PREFIX/,.prompt.md" ]; then
        mkdir -p "$XDG_CONFIG"
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/prompt.md" -o "$XDG_CONFIG/prompt.md" 2>/dev/null || true
        echo "  Created $XDG_CONFIG/prompt.md"
    else
        echo "  Skipped prompt (already exists)"
    fi
else
    # Windows (Git Bash/MSYS): default to %APPDATA%\comma, keep legacy files respected
    if [ -n "${APPDATA:-}" ]; then
        WIN_APPDATA="$(cygpath -u "$APPDATA" 2>/dev/null || echo "$APPDATA")/comma"
    else
        WIN_APPDATA="$HOME/AppData/Roaming/comma"
    fi
    if [ ! -f "$WIN_APPDATA/config.json" ] && [ ! -f "$LEGACY_CONFIG" ]; then
        mkdir -p "$WIN_APPDATA"
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/config.json" -o "$WIN_APPDATA/config.json" 2>/dev/null || true
        echo "  Created $WIN_APPDATA/config.json"
    else
        echo "  Skipped config (already exists)"
    fi
    [ -f "$WIN_APPDATA/config.json" ] && CONFIG_FILE="$WIN_APPDATA/config.json" || CONFIG_FILE="$LEGACY_CONFIG"
    if [ ! -f "$WIN_APPDATA/prompt.md" ] && [ ! -f "$PREFIX/,.prompt.md" ]; then
        mkdir -p "$WIN_APPDATA"
        curl -sSL "https://raw.githubusercontent.com/${REPO}/main/prompt.md" -o "$WIN_APPDATA/prompt.md" 2>/dev/null || true
        echo "  Created $WIN_APPDATA/prompt.md"
    else
        echo "  Skipped prompt (already exists)"
    fi
fi

echo ""
echo "Installed files:"
ls -lh "$PREFIX/," 2>/dev/null || ls -lh "$PREFIX/comma.exe" 2>/dev/null || true
[ -f "$CONFIG_FILE" ] && ls -lh "$CONFIG_FILE"
[ -f "$PREFIX/,.prompt.md" ] && ls -lh "$PREFIX/,.prompt.md"

# Check if model is configured
if [ -f "$CONFIG_FILE" ]; then
    if ! grep -q '"auth_token"' "$CONFIG_FILE" || grep -q '"auth_token": ""' "$CONFIG_FILE"; then
        echo ""
        echo "⚠  No API key configured!"
        echo "Edit $CONFIG_FILE and set your API key:"
        echo ""
        echo '  {'
        echo '    "base_url": "https://api.cerebras.ai/v1",'
        echo '    "auth_token": "your-api-key",'
        echo '    "model": "gemma-4-31b"'
        echo '  }'
        echo ""
        echo "Free options: Cerebras (cerebras.ai), Groq (groq.com), Ollama (local)"
    fi
fi

echo ""
echo "Done. Run ', -h' for usage."
