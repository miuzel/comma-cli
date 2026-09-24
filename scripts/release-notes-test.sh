#!/usr/bin/env bash
# release-notes-test.sh — regression tests for scripts/release-notes.sh.
#
# Every case runs the script under test inside a throwaway git repository built
# under tmp/ (never against the real comma-cli history) and asserts how the
# changelog base is detected.
#
# Bug pinned down here: the integration-branch verification tag
# (`0.28.0-alpha-verified`, docs/version-integration-and-release-sop.md step 7)
# is deliberately not `v`-prefixed and is tagged right next to the release
# commit. A bare `git describe --tags --abbrev=0` therefore picked it as "the
# previous version", the range held only the version-bump commit, the
# housekeeping filter dropped that one commit and the script aborted with a
# bogus "no commits found".
#
# Usage: scripts/release-notes-test.sh          (exit 0 = all assertions green)
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SCRIPT_UNDER_TEST="$SCRIPT_DIR/release-notes.sh"

TMP_BASE="${RELEASE_NOTES_TEST_TMP:-$REPO_ROOT/tmp}"
mkdir -p "$TMP_BASE"
SANDBOX="$(mktemp -d "$TMP_BASE/release-notes-test.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); printf 'ok   - %s\n' "$1"; }
bad() {
    FAIL=$((FAIL + 1))
    printf 'FAIL - %s\n' "$1"
    [ -n "${2:-}" ] && printf '       %s\n' "$2"
    return 0
}

assert_eq() { # <actual> <expected> <label>
    if [ "$1" = "$2" ]; then ok "$3"; else bad "$3" "expected [$2], got [$1]"; fi
}
assert_contains() { # <haystack> <needle> <label>
    case "$1" in *"$2"*) ok "$3" ;; *) bad "$3" "expected to contain [$2]" ;; esac
}
assert_absent() { # <haystack> <needle> <label>
    case "$1" in *"$2"*) bad "$3" "expected NOT to contain [$2]" ;; *) ok "$3" ;; esac
}

# ── fixture helpers ──────────────────────────────────────────────────────
# new_fixture <name>: repo holding a copy of the script under test, so the
# script's own REPO_ROOT is the fixture and not comma-cli.
new_fixture() {
    local dir="$SANDBOX/$1"
    mkdir -p "$dir/scripts"
    cp "$SCRIPT_UNDER_TEST" "$dir/scripts/release-notes.sh"
    chmod +x "$dir/scripts/release-notes.sh"
    git -C "$dir" init -q -b main
    git -C "$dir" config user.email "release-notes-test@example.com"
    git -C "$dir" config user.name "release-notes test"
    git -C "$dir" config commit.gpgsign false
    printf '%s' "$dir"
}
commit() { git -C "$1" commit -q --allow-empty -m "$2"; }
tag() { git -C "$1" tag "$2"; }

RUN_OUT=""
RUN_RC=0
run() { # run <repo> [args...] -> RUN_OUT (stdout+stderr), RUN_RC
    local repo="$1"
    shift
    RUN_OUT="$( (cd "$repo" && ./scripts/release-notes.sh "$@") 2>&1 )"
    RUN_RC=$?
}

# ── 1. an integration verification tag must not hijack the base ──────────
A="$(new_fixture test-tag-hijack)"
commit "$A" "feat: initial feature (g-000)"
tag "$A" v0.27.1
commit "$A" "fix(repl): bound the auto-refine chain per intent (g-014)"
commit "$A" "chore: set integration version to 0.28.0-alpha (test build)"
tag "$A" 0.28.0-alpha-verified          # verification tag, not v-prefixed
commit "$A" "chore: release v0.28.0 (bump version)"
tag "$A" v0.28.0

# Negative control: the pre-fix detection really does pick the test tag here,
# which is what produced the empty (all-housekeeping) range.
assert_eq "$(git -C "$A" describe --tags --abbrev=0 'v0.28.0^')" "0.28.0-alpha-verified" \
    "control: bare git describe picks the verification tag in this fixture"

run "$A" v0.28.0 --output notes.md
assert_eq "$RUN_RC" 0 "verification tag present: auto-detected base succeeds (was: no commits found)"
NOTES="$(cat "$A/notes.md" 2>/dev/null || true)"
assert_contains "$NOTES" "compare/v0.27.1...v0.28.0" "compare link uses the real previous release v0.27.1"
assert_absent "$NOTES" "alpha-verified" "the verification tag never appears in the draft"
assert_contains "$NOTES" "bound the auto-refine chain per intent" "commits since v0.27.1 are collected"
assert_absent "$NOTES" "bump version" "the version-bump commit stays filtered as housekeeping"

run "$A" v0.28.0 --from v0.27.1 --output notes-from.md
assert_eq "$RUN_RC" 0 "explicit --from v0.27.1 still works"
assert_eq "$(cat "$A/notes.md")" "$(cat "$A/notes-from.md")" \
    "auto-detected base produces the same draft as --from v0.27.1"

# ── 2. other non-release shapes are ignored too ─────────────────────────
B="$(new_fixture non-release-shapes)"
commit "$B" "feat: base feature"
tag "$B" v1.0.0
commit "$B" "fix: the real change"
tag "$B" v1.1.0-rc1                      # nearest, pre-release suffix
tag "$B" 1.1.0                           # nearest, no v prefix
tag "$B" v1.1                            # nearest, not MAJOR.MINOR.PATCH
commit "$B" "chore: release v1.1.0 (bump version)"
tag "$B" v1.1.0

run "$B" 1.1.0 --output n.md
assert_eq "$RUN_RC" 0 "suffixed / un-prefixed / short tags do not break detection"
assert_absent "$(git -C "$B" describe --tags --abbrev=0 'v1.1.0^')" "v1.0.0" \
    "control: raw git describe would not have chosen the release tag v1.0.0"
assert_contains "$(cat "$B/n.md")" "compare/v1.0.0...v1.1.0" "base is the newest release-shaped tag v1.0.0"

# ── 3. an empty range must fail with an actionable message ──────────────
C="$(new_fixture empty-range)"
commit "$C" "feat: base feature"
tag "$C" v2.0.0
commit "$C" "chore: release v2.0.1 (bump version)"
tag "$C" v2.0.1

run "$C" v2.0.1 --output n.md
assert_eq "$RUN_RC" 1 "housekeeping-only range still fails"
assert_contains "$RUN_OUT" "no commits found in range v2.0.0..v2.0.1" "error names the range"
assert_contains "$RUN_OUT" "--from <tag>" "error shows the --from re-run command"
assert_contains "$RUN_OUT" "Release tags usable as --from" "error lists the candidate bases"
assert_contains "$RUN_OUT" "v2.0.0" "candidate list contains v2.0.0"
assert_eq "$(printf '%s\n' "$RUN_OUT" | sed -n 's/^    \(v[0-9].*\)$/\1/p')" "v2.0.0" \
    "candidate list is exactly the release tags before v2.0.1"

# A long history must not dump every tag: the list is bounded (10 + a count).
E="$(new_fixture candidate-limit)"
commit "$E" "feat: base feature"
for i in 1 2 3 4 5 6 7 8 9 10 11; do tag "$E" "v0.0.$i"; done
commit "$E" "chore: release v1.0.0 (bump version)"
tag "$E" v1.0.0

run "$E" v1.0.0 --output n.md
assert_eq "$RUN_RC" 1 "empty range with many old tags fails"
assert_eq "$(printf '%s\n' "$RUN_OUT" | sed -n 's/^    \(v[0-9].*\)$/\1/p' | grep -c .)" "10" \
    "candidate list is capped at 10 entries"
assert_contains "$RUN_OUT" "and 1 older release tag(s)" "candidate list reports the omitted tags"

run "$C" v2.0.1 --from v9.9.9 --output n.md
assert_eq "$RUN_RC" 2 "an unknown --from ref is rejected"
assert_contains "$RUN_OUT" "not a known commit/ref" "unknown --from message is explicit"

# ── 4. first release: full history, no compare link ─────────────────────
D="$(new_fixture first-release)"
commit "$D" "feat: the very first feature"
tag "$D" v3.0.0

run "$D" v3.0.0 --output n.md
assert_eq "$RUN_RC" 0 "first release drafts from the full history"
assert_contains "$RUN_OUT" "no previous release tag" "first release says so on stderr"
assert_contains "$(cat "$D/n.md")" "first release: no compare link yet" "no compare link for the first release"
assert_contains "$(cat "$D/n.md")" "the very first feature" "first release collects the history"

# ── 5. --help documents the base rule and --from ────────────────────────
run "$D" --help
assert_eq "$RUN_RC" 0 "--help exits 0"
assert_contains "$RUN_OUT" "--from REF" "--help documents --from"
assert_contains "$RUN_OUT" "verification tag" "--help explains which tags are ignored"

printf '\nrelease-notes-test.sh: %d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
exit 0
