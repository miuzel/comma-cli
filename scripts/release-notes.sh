#!/usr/bin/env bash
# release-notes.sh — draft release notes from git history for `gh release edit`.
#
# Given a release version (e.g. `0.27.0` or `v0.27.0`), this script collects
# the commits since the previous version tag, groups them by type and writes a
# notes draft file (feature/fix bullets + Full Changelog compare link) that can
# be passed straight to:
#
#   gh release edit v0.27.0 --repo miuzel/comma-cli --notes-file <file>
#
# The draft leads with a TODO placeholder for the 1-3 line human summary
# (feature name + user-visible effect) and keeps the compare link at the
# bottom, per AGENTS.md "Deployment process".
#
# CDN reminder (AGENTS.md): the ~10 min wait after pushing a tag applies to
# testing `, --update` / install.sh against releases/latest/download — writing
# the notes draft itself needs no download and is unaffected.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO="${COMMA_REPO:-miuzel/comma-cli}"       # upstream repo (for the compare link)

usage() {
    cat <<'EOF'
release-notes.sh — draft release notes from git history.

Usage:
  scripts/release-notes.sh <version> [--output FILE] [--from REF]

  <version>    Release version to draft notes for (e.g. 0.27.0 or v0.27.0).
               The tag v<version> must exist in this repo (git fetch --tags).
  --output     Where to write the draft. Default: release-notes-<version>.md
  --from       Base ref of the changelog range. Default: the previous version
               tag reachable from v<version>^ .
  -h, --help   Show this help.

What it does:
  1. Collects commit subjects from <base>..v<version> (no merges; skips
     "chore: release v…" / "bump version…" housekeeping commits).
  2. Groups them into Features / Fixes / Other bullets.
  3. Writes a draft file with a TODO summary placeholder and the
     **Full Changelog** compare link at the bottom, ready for
     `gh release edit <version> --repo miuzel/comma-cli --notes-file <file>`.

The ~10 min CDN wait from AGENTS.md applies to testing `, --update` /
install.sh, not to drafting notes.
EOF
}

OUTPUT=""
FROM=""
VERSION=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        -o|--output)
            [ $# -ge 2 ] || { echo "release-notes.sh: --output needs an argument" >&2; exit 2; }
            OUTPUT="$2"
            shift 2
            ;;
        --from)
            [ $# -ge 2 ] || { echo "release-notes.sh: --from needs an argument" >&2; exit 2; }
            FROM="$2"
            shift 2
            ;;
        -*)
            echo "release-notes.sh: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            [ -z "$VERSION" ] || { echo "release-notes.sh: unexpected extra argument: $1" >&2; usage >&2; exit 2; }
            VERSION="$1"
            shift
            ;;
    esac
done

if [ -z "$VERSION" ]; then
    echo "release-notes.sh: missing <version> (e.g. 0.27.0 or v0.27.0)" >&2
    echo "Run 'scripts/release-notes.sh --help' for usage." >&2
    exit 2
fi

if [[ "$VERSION" =~ ^v?([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
    VER="${BASH_REMATCH[1]}"
else
    echo "release-notes.sh: invalid version '$VERSION' (expected e.g. 0.27.0 or v0.27.0)" >&2
    exit 2
fi
TAG="v${VER}"

cd "$REPO_ROOT"

if ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    echo "release-notes.sh: tag $TAG not found in this repo." >&2
    echo "  Fetch tags first: git fetch --tags (and pull)." >&2
    exit 1
fi

# ── determine the changelog range ────────────────────────────────────────
if [ -n "$FROM" ]; then
    BASE="$FROM"
else
    BASE="$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)"
fi
if [ -n "$BASE" ]; then
    RANGE="$BASE..$TAG"
else
    RANGE="$TAG"   # first release: everything up to the tag, no compare link
fi

# ── collect and group commits ────────────────────────────────────────────
# Subjects only; no merges; skip housekeeping commits ("chore: release v…",
# "bump version…", "Update Cargo.lock"). Both plain and scoped conventional
# prefixes ("feat:" and "feat(context):") are grouped; the prefix is stripped
# from the bullet.
strip_prefix() {
    sed -E 's/^[a-z]+(\([^)]*\))?:[[:space:]]*//' <<< "$1"
}

FEAT=""
FIX=""
OTHER=""
while IFS= read -r line; do
    [ -n "$line" ] || continue
    case "$line" in
        feat:*|feat\(*\):*) FEAT="${FEAT}  - $(strip_prefix "$line")"$'\n' ;;
        fix:*|fix\(*\):*)   FIX="${FIX}  - $(strip_prefix "$line")"$'\n' ;;
        *)                  OTHER="${OTHER}  - $line"$'\n' ;;
    esac
done < <(git log --no-merges --pretty=format:%s "$RANGE" \
    | grep -Ev '^(chore: release v[0-9]|bump version|Update Cargo\.lock)' || true)

if [ -z "${FEAT}${FIX}${OTHER}" ]; then
    echo "release-notes.sh: no commits found in range $RANGE" >&2
    exit 1
fi

# ── write the draft ──────────────────────────────────────────────────────
[ -n "$OUTPUT" ] || OUTPUT="release-notes-${TAG}.md"

{
    echo "## ${TAG}"
    echo
    echo "<!-- TODO: replace with 1-3 lines summarizing the user-visible change"
    echo "     (feature name + effect); keep the Full Changelog link at the bottom. -->"
    echo
    if [ -n "$FEAT" ]; then
        echo "### Features"
        printf '%b' "$FEAT"
        echo
    fi
    if [ -n "$FIX" ]; then
        echo "### Fixes"
        printf '%b' "$FIX"
        echo
    fi
    if [ -n "$OTHER" ]; then
        echo "### Other"
        printf '%b' "$OTHER"
        echo
    fi
    if [ -n "$BASE" ]; then
        echo "**Full Changelog**: https://github.com/${REPO}/compare/${BASE}...${TAG}"
    else
        echo "<!-- first release: no compare link yet -->"
    fi
} > "$OUTPUT"

echo "Notes draft written to: $OUTPUT"
echo
echo "After filling in the TODO summary, publish with:"
echo "  gh release edit ${TAG} --repo ${REPO} --notes-file \"$OUTPUT\""
