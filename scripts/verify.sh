#!/usr/bin/env bash
# verify.sh — compatibility wrapper for the canonical AGS verification gate.
#
# Rust unit and CLI contract tests run exactly once through `ags verify`.
# This wrapper retains only checks that intentionally live outside the Rust
# workspace: supply-chain policy and the trusted shell lane classifier.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
failures=0
VERIFY_SCOPE="${AGS_VERIFY_SCOPE:-full}"

case "$VERIFY_SCOPE" in
    local | full | release) ;;
    *)
        echo "invalid AGS_VERIFY_SCOPE: $VERIFY_SCOPE (expected local, full, or release)" >&2
        exit 2
        ;;
esac

run_gate() {
    local label="$1"
    shift
    echo "--- $label ---"
    if "$@"; then
        echo "[OK] $label"
    else
        echo "[FAIL] $label"
        failures=$((failures + 1))
    fi
    echo
}

check_lane() {
    local label="$1"
    local files="$2"
    local expected="$3"
    local actual

    actual="$(printf '%s' "$files" | bash "$REPO_ROOT/scripts/lane-decision.sh")"
    if [[ "$actual" == "$expected" ]]; then
        echo "[OK] lane-decision: $label -> $expected"
    else
        echo "[FAIL] lane-decision: $label -> expected $expected, got $actual"
        failures=$((failures + 1))
    fi
}

cd "$REPO_ROOT"

echo "=== AGS Verification Gate ==="
echo "Repo: $REPO_ROOT"
echo

# Canonical structured verification. This already runs workspace fmt, tests,
# release build, fixtures, governance YAML and preflight. Full adds drift
# checks; release adds the fail-closed public release boundary.
verify_args=(verify --scope "$VERIFY_SCOPE" --format text)
if [[ "$VERIFY_SCOPE" == "release" && -n "${AGS_RELEASE_PUBLIC_ROOT:-}" ]]; then
    verify_args+=(--public-root "$AGS_RELEASE_PUBLIC_ROOT")
fi
run_gate "ags verify --scope $VERIFY_SCOPE" \
    cargo run -q -p ags-cli -- "${verify_args[@]}"

# External supply-chain authority. Missing cargo-deny is fail-closed.
echo "--- cargo deny check ---"
if command -v cargo-deny >/dev/null 2>&1; then
    if cargo deny check; then
        echo "[OK] cargo deny check"
    else
        echo "[FAIL] cargo deny check"
        failures=$((failures + 1))
    fi
else
    echo "[FAIL] cargo-deny not installed; supply-chain gate cannot run"
    failures=$((failures + 1))
fi
echo

# Keep this contract outside the in-tree classifier. Release/sync automation
# must not let a changed Rust binary decide whether its own verification may be
# skipped.
echo "--- trusted lane-decision contract ---"
check_lane "ignore-only" $'.gitignore\n' "MINIMAL"
check_lane "documentation-only" $'docs/notes.md\n' "MINIMAL"
check_lane "protocol" $'protocol/task-routing.md\n' "FULL"
check_lane "gate-selection script" $'scripts/lane-decision.sh\n' "FULL"
check_lane "empty input" "" "FULL"
echo

if [[ "$failures" -eq 0 ]]; then
    echo "=== All checks passed ==="
    exit 0
fi

echo "=== $failures check group(s) failed ==="
exit 1
