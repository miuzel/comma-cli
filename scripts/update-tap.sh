#!/usr/bin/env bash
# update-tap.sh — semi-automate the Homebrew tap update after a comma-cli release.
#
# Given a release version (e.g. `0.27.0` or `v0.27.0`), this script:
#   1. reads the release's sha256sums.txt (pin-tag URL, not the lagging
#      releases/latest/download redirect),
#   2. rewrites the four url + sha256 pairs plus the version line in
#      miuzel/homebrew-tap's Formula/comma-cli.rb,
#   3. prints the resulting committable `git diff` for review.
#
# It deliberately does NOT touch .github/workflows/release.yml — the archive
# names referenced here (comma-<os>-<arch>.tar.gz) must stay in sync with that
# workflow's `archive:` keys.
#
# CDN reminder (see AGENTS.md "Deployment process"): after pushing the release
# tag, wait ~10 minutes before relying on releases/latest/download — it can
# briefly serve the *previous* release's sha256sums.txt together with the new
# archive, causing a spurious checksum mismatch. This script fetches the
# pin-tag URL, which is NOT affected, but test `, --update` / install.sh only
# after the wait.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO="${COMMA_REPO:-miuzel/comma-cli}"       # upstream repo
FORMULA_REL="Formula/comma-cli.rb"           # formula path inside the tap repo

usage() {
    cat <<'EOF'
update-tap.sh — update the Homebrew formula after a comma-cli release.

Usage:
  scripts/update-tap.sh <version> [--tap-dir DIR] [--sums FILE]

  <version>    Release version to update to (e.g. 0.27.0 or v0.27.0).
  --tap-dir    Path to the cloned miuzel/homebrew-tap repo.
               Default: <repo-root>/../homebrew-tap  (or $TAP_DIR).
  --sums FILE  Use a local sha256sums.txt instead of downloading it from the
               release (useful for testing or air-gapped runs).
  -h, --help   Show this help.

What it does:
  1. Downloads the release's sha256sums.txt via the pin-tag URL
     (releases/download/vX.Y.Z/... — not subject to the ~10 min CDN lag of
     releases/latest/download; see AGENTS.md).
  2. Rewrites the four url + sha256 pairs in Formula/comma-cli.rb
     (linux/macos x x86_64/aarch64) plus the version line.
  3. Prints the resulting committable `git diff` for review.

It never modifies .github/workflows/release.yml; archive names referenced here
must match that workflow's `archive:` keys.
EOF
}

TAP_DIR="${TAP_DIR:-}"
SUMS_FILE=""
VERSION=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --tap-dir)
            [ $# -ge 2 ] || { echo "update-tap.sh: --tap-dir needs an argument" >&2; exit 2; }
            TAP_DIR="$2"
            shift 2
            ;;
        --sums)
            [ $# -ge 2 ] || { echo "update-tap.sh: --sums needs an argument" >&2; exit 2; }
            SUMS_FILE="$2"
            shift 2
            ;;
        -*)
            echo "update-tap.sh: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            [ -z "$VERSION" ] || { echo "update-tap.sh: unexpected extra argument: $1" >&2; usage >&2; exit 2; }
            VERSION="$1"
            shift
            ;;
    esac
done

if [ -z "$VERSION" ]; then
    echo "update-tap.sh: missing <version> (e.g. 0.27.0 or v0.27.0)" >&2
    echo "Run 'scripts/update-tap.sh --help' for usage." >&2
    exit 2
fi

# Normalize: accept "0.27.0" or "v0.27.0".
if [[ "$VERSION" =~ ^v?([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
    NEW_VER="${BASH_REMATCH[1]}"
else
    echo "update-tap.sh: invalid version '$VERSION' (expected e.g. 0.27.0 or v0.27.0)" >&2
    exit 2
fi
NEW_TAG="v${NEW_VER}"

# The four archives published by .github/workflows/release.yml that the
# formula ships. (The Windows zip is not part of Homebrew.) Names must stay
# in sync with the workflow's `archive:` keys.
ARCHIVES=(
    "comma-macos-aarch64.tar.gz"
    "comma-macos-x86_64.tar.gz"
    "comma-linux-x86_64.tar.gz"
    "comma-linux-aarch64.tar.gz"
)

# ── resolve tap repo and formula ─────────────────────────────────────────
[ -z "$TAP_DIR" ] && TAP_DIR="$REPO_ROOT/../homebrew-tap"
FORMULA="$TAP_DIR/$FORMULA_REL"

if [ ! -d "$TAP_DIR" ]; then
    echo "update-tap.sh: tap repo not found at '$TAP_DIR'." >&2
    echo "  Clone it first, e.g.:" >&2
    echo "    git clone git@github.com:miuzel/homebrew-tap.git \"$TAP_DIR\"" >&2
    exit 1
fi
if [ ! -f "$FORMULA" ]; then
    echo "update-tap.sh: formula not found at '$FORMULA'." >&2
    exit 1
fi

# ── current version in the formula ───────────────────────────────────────
OLD_VER="$(sed -nE 's/^[[:space:]]*version "([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' "$FORMULA" | head -n1)"
if [ -z "$OLD_VER" ]; then
    echo "update-tap.sh: could not read the current version from $FORMULA" >&2
    exit 1
fi
if [ "$OLD_VER" = "$NEW_VER" ]; then
    echo "Formula is already at version $NEW_VER — nothing to do."
    exit 0
fi
OLD_TAG="v${OLD_VER}"

# ── obtain sha256sums.txt ────────────────────────────────────────────────
TMPDIR=""
if [ -z "$SUMS_FILE" ]; then
    SUMS_URL="https://github.com/${REPO}/releases/download/${NEW_TAG}/sha256sums.txt"
    echo "Fetching ${SUMS_URL} ..."
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT
    SUMS_FILE="$TMPDIR/sha256sums.txt"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$SUMS_URL" -o "$SUMS_FILE" || {
            echo "update-tap.sh: failed to download sha256sums.txt (is release ${NEW_TAG} published?)" >&2
            exit 1
        }
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$SUMS_URL" -O "$SUMS_FILE" || {
            echo "update-tap.sh: failed to download sha256sums.txt (is release ${NEW_TAG} published?)" >&2
            exit 1
        }
    else
        echo "update-tap.sh: curl or wget required to download sha256sums.txt (or pass --sums FILE)" >&2
        exit 1
    fi
    echo "  (pin-tag URL — not affected by the ~10 min CDN lag on releases/latest; see AGENTS.md)"
fi
if [ ! -f "$SUMS_FILE" ]; then
    echo "update-tap.sh: sha256sums file not found: $SUMS_FILE" >&2
    exit 1
fi

# Look up the hash for one archive in sha256sums.txt (lines: "<hash>  <name>").
hash_for() {
    awk -v a="$1" '$2 == a { print $1; exit }' "$SUMS_FILE"
}

# Ensure every archive the formula needs is present.
MISSING=0
for a in "${ARCHIVES[@]}"; do
    if [ -z "$(hash_for "$a")" ]; then
        echo "update-tap.sh: sha256sums.txt has no entry for $a" >&2
        MISSING=1
    fi
done
if [ "$MISSING" -ne 0 ]; then
    exit 1
fi

# ── rewrite the formula (version line, URLs, four hashes) ────────────────
# Extract the hash currently paired with each archive's url line.
old_hash_for() {
    awk -v a="$1" '
        index($0, a) && index($0, "url") {
            if (getline > 0 && $0 ~ /sha256/) {
                sub(/.*sha256 "/, ""); sub(/".*/, ""); print; exit
            }
        }
    ' "$FORMULA"
}

sed_args=(
    -e "s|${OLD_TAG}|${NEW_TAG}|g"                       # URLs (…/download/vX.Y.Z/…)
    -e "s|version \"${OLD_VER}\"|version \"${NEW_VER}\"|" # version line
)
for a in "${ARCHIVES[@]}"; do
    old_h="$(old_hash_for "$a")"
    new_h="$(hash_for "$a")"
    if [ -z "$old_h" ]; then
        echo "update-tap.sh: could not find the current sha256 for $a in $FORMULA" >&2
        exit 1
    fi
    sed_args+=(-e "s|${old_h}|${new_h}|")
done

tmp="$(mktemp "${FORMULA}.XXXXXX")"
trap 'rm -rf "$TMPDIR" "$tmp"' EXIT
sed "${sed_args[@]}" "$FORMULA" > "$tmp"
mv "$tmp" "$FORMULA"
tmp=""

echo "Formula updated: ${OLD_VER} -> ${NEW_VER}"

# ── show the committable diff ────────────────────────────────────────────
echo
if git -C "$TAP_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git -C "$TAP_DIR" diff -- "$FORMULA_REL"
    echo
    echo "Review the diff, then commit and push:"
    echo "  git -C \"$TAP_DIR\" commit -am \"chore: bump comma-cli to ${NEW_TAG}\""
    echo "  git -C \"$TAP_DIR\" push"
else
    echo "($TAP_DIR is not a git repo; updated file: $FORMULA)"
fi

# ── CDN reminder (AGENTS.md) ─────────────────────────────────────────────
echo
echo "Note: after pushing ${NEW_TAG}, wait ~10 min before testing ', --update' or"
echo "install.sh — releases/latest/download can briefly serve the previous release's"
echo "sha256sums.txt with the new archive. The pin-tag URL used above is unaffected."
