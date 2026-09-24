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
# "Previous version" means a *release-shaped* tag: exactly `vMAJOR.MINOR.PATCH`
# (nothing after the patch number), reachable from `v<version>^` and nearest to
# it — the `git describe --tags --abbrev=0` notion, restricted to release tags.
# Every other tag is ignored on purpose. The integration-branch verification tag
# (`0.28.0-alpha-verified`, docs/version-integration-and-release-sop.md step 7)
# is deliberately not `v`-prefixed and is tagged right next to the release
# commit: taking it as "the previous version" leaves a range whose only commit is
# the version bump, which the housekeeping filter below drops, so the script used
# to abort with a bogus "no commits found". Pass `--from <ref>` to choose the
# base yourself; when the range turns out empty the script lists the tags you can
# pass there.
#
# Regression tests for the base detection (and for the empty-range message):
#   scripts/release-notes-test.sh
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
  --from REF   Base ref of the changelog range (tag, branch or commit).
               Default: the previous *release* tag, i.e. a tag matching
               exactly v<major>.<minor>.<patch> that is reachable from
               v<version>^, nearest to it first. Every other tag is ignored
               on purpose — an integration-branch verification tag such as
               0.28.0-alpha-verified, a v<x>.<y>.<z>-rc1 pre-release, or a tag
               without the `v` prefix is not a release and must never be used
               as the previous version. Pass --from when you need a different
               base (e.g. v0.27.1).
  -h, --help   Show this help.

What it does:
  1. Collects commit subjects from <base>..v<version> (no merges; skips
     "chore: release v…" / "chore: set integration version …" / "bump version…"
     / "Update Cargo.lock" housekeeping commits).
  2. Groups them into Features / Fixes / Other bullets.
  3. Writes a draft file with a TODO summary placeholder and the
     **Full Changelog** compare link at the bottom, ready for
     `gh release edit <version> --repo miuzel/comma-cli --notes-file <file>`.

When the range is empty the script fails with the candidate release tags and
the exact `--from <tag>` command to re-run — it never guesses a different base
silently.

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
# Only a release-shaped tag may become the base: exactly `vMAJOR.MINOR.PATCH`,
# no `-rc1`/`-alpha-verified`/`+build` suffix, no leading digit-only name. See
# the header comment for why (integration test tags hijack the range).
RELEASE_TAG_RE='^v[0-9]+\.[0-9]+\.[0-9]+$'
CANDIDATE_LIMIT=10   # how many candidate bases the error messages list

# Release tags reachable from v<version>^ — the candidates for "previous
# version" — newest version first, so equal commit distances prefer the newer
# version in detect_previous_release() below.
release_tag_candidates() {
    git tag --merged "$TAG^" --sort=-v:refname --list 2>/dev/null \
        | grep -E "$RELEASE_TAG_RE" || true
}

# Print the bases the user may pass to --from (bounded, newest first). Used by
# the error paths so a failed run always shows what to re-run with.
print_candidates() {
    local cands total
    cands="$(release_tag_candidates)"
    if [ -z "$cands" ]; then
        echo "  (no release tag vX.Y.Z reachable from $TAG^ — this looks like the first release)" >&2
        return 0
    fi
    echo "  Release tags usable as --from (reachable from $TAG^, newest first):" >&2
    printf '%s\n' "$cands" | head -n "$CANDIDATE_LIMIT" | sed 's/^/    /' >&2
    total="$(printf '%s\n' "$cands" | grep -c .)"
    if [ "$total" -gt "$CANDIDATE_LIMIT" ]; then
        echo "    … and $((total - CANDIDATE_LIMIT)) older release tag(s)" >&2
    fi
}

# Nearest candidate by commit distance: the `git describe --tags --abbrev=0`
# semantics, restricted to release tags. Prints nothing when there is none.
detect_previous_release() {
    local cand dist best="" best_dist=""
    while IFS= read -r cand; do
        [ -n "$cand" ] || continue
        dist="$(git rev-list --count "$cand..$TAG^")"
        if [ -z "$best_dist" ] || [ "$dist" -lt "$best_dist" ]; then
            best="$cand"
            best_dist="$dist"
        fi
    done < <(release_tag_candidates)
    printf '%s' "$best"
}

AUTO_BASE=1
if [ -n "$FROM" ]; then
    AUTO_BASE=0
    if ! git rev-parse -q --verify "${FROM}^{commit}" >/dev/null; then
        echo "release-notes.sh: --from '$FROM' is not a known commit/ref." >&2
        print_candidates
        exit 2
    fi
    BASE="$FROM"
else
    BASE="$(detect_previous_release)"
fi

if [ -n "$BASE" ]; then
    RANGE="$BASE..$TAG"
else
    RANGE="$TAG"   # first release: everything up to the tag, no compare link
    if [ "$AUTO_BASE" = 1 ]; then
        echo "note: no previous release tag (vX.Y.Z) reachable from $TAG^; using the" >&2
        echo "      full history. Pass --from <ref> to narrow the range." >&2
    fi
fi

# ── collect and group commits ────────────────────────────────────────────
# Subjects only; no merges; skip housekeeping commits ("chore: release v…",
# "chore: set integration version …" — the integration-branch version bump of
# docs/version-integration-and-release-sop.md step 5 is build plumbing, not a
# user-visible change —, "bump version…", "Update Cargo.lock"). Both plain and
# scoped conventional prefixes ("feat:" and "feat(context):") are grouped; the
# prefix is stripped from the bullet.
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
    | grep -Ev '^(chore: release v[0-9]|chore: set integration version|bump version|Update Cargo\.lock)' || true)

if [ -z "${FEAT}${FIX}${OTHER}" ]; then
    echo "release-notes.sh: no commits found in range $RANGE" >&2
    if [ "$AUTO_BASE" = 1 ] && [ -n "$BASE" ]; then
        echo "  Base $BASE was auto-detected (nearest release tag before $TAG), so the range" >&2
        echo "  is empty or every commit in it is housekeeping (\"chore: release v…\"," >&2
        echo "  \"chore: set integration version …\", \"bump version…\", \"Update Cargo.lock\")." >&2
    elif [ "$AUTO_BASE" = 1 ]; then
        echo "  No previous release tag was found, so the range is the whole history and every" >&2
        echo "  commit in it is housekeeping (\"chore: release v…\", \"chore: set integration" >&2
        echo "  version …\", \"bump version…\")." >&2
    else
        echo "  --from $(printf '%q' "$BASE") is not an ancestor of $TAG, or every commit between" >&2
        echo "  them is housekeeping (\"chore: release v…\", \"chore: set integration version …\"," >&2
        echo "  \"bump version…\")." >&2
    fi
    echo "  Re-run with the base you want in the changelog:" >&2
    echo "    scripts/release-notes.sh $VER --from <tag>" >&2
    print_candidates
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
