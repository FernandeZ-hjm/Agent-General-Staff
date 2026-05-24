#!/usr/bin/env bash
set -euo pipefail

# diff-local.sh - Compare suite assets with currently installed local files
# Reports: suite-only, local-only, and content-diff files.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_HOME="${TARGET_HOME:-$HOME}"

usage() {
    cat <<EOF
Usage: diff-local.sh [--summary] [--target-home PATH]

Options:
  --summary       Only print a one-line summary per asset
  --target-home   Compare against a non-default home directory (default: \$HOME)
  --help          Show this message
EOF
}

SUMMARY=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --summary) SUMMARY=true ;;
        --target-home) TARGET_HOME="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown: $1"; usage; exit 1 ;;
    esac
    shift
done

echo "=== Suite vs Local Diff ==="
echo "Suite root : $SUITE_ROOT"
echo "Target home: $TARGET_HOME"
echo ""

local_only=0
suite_only=0
content_diff=0
identical=0

diff_one_file() {
    local suite_path="$1"
    local local_path="$2"
    local label="$3"

    if [[ ! -f "$local_path" ]]; then
        echo "  [LOCAL_ONLY_MISSING] $label"
        ((suite_only++)) || true
        return
    fi

    if cmp -s "$suite_path" "$local_path" 2>/dev/null; then
        if ! $SUMMARY; then
            echo "  [IDENTICAL] $label"
        fi
        ((identical++)) || true
    else
        echo "  [DIFF] $label"
        ((content_diff++)) || true
        if ! $SUMMARY; then
            echo "    --- suite vs local ---"
            diff -u "$local_path" "$suite_path" 2>/dev/null | head -20 || true
            echo "    --- end diff ---"
        fi
    fi
}

diff_one_dir() {
    local suite_dir="$1"
    local local_dir="$2"
    local label="$3"

    if [[ ! -d "$local_dir" ]]; then
        echo "  [SUITE_ONLY] $label (no local copy)"
        ((suite_only++)) || true
        return
    fi

    # Compare files in suite dir
    while IFS= read -r -d '' suite_file; do
        rel="${suite_file#$suite_dir/}"
        local_file="$local_dir/$rel"
        if [[ ! -f "$local_file" ]]; then
            echo "  [SUITE_ONLY_FILE] $label/$rel"
            ((suite_only++)) || true
        elif ! cmp -s "$suite_file" "$local_file" 2>/dev/null; then
            echo "  [DIFF] $label/$rel"
            ((content_diff++)) || true
            if ! $SUMMARY; then
                diff -u "$local_file" "$suite_file" 2>/dev/null | head -20 || true
                echo "    --- end diff ---"
            fi
        else
            if ! $SUMMARY; then
                echo "  [IDENTICAL] $label/$rel"
            fi
            ((identical++)) || true
        fi
    done < <(find "$suite_dir" -type f -not -path '*/.git/*' -print0)

    # Check for local-only files
    while IFS= read -r -d '' local_file; do
        rel="${local_file#$local_dir/}"
        suite_file="$suite_dir/$rel"
        if [[ ! -f "$suite_file" ]]; then
            echo "  [LOCAL_ONLY] $label/$rel"
            ((local_only++)) || true
        fi
    done < <(find "$local_dir" -type f -not -path '*/.git/*' -print0 2>/dev/null || true)
}

# Check rules
echo "--- Rules ---"
for rule in SOUL.md core.md; do
    diff_one_file "$SUITE_ROOT/global-rules/$rule" "$TARGET_HOME/.agents/rules/$rule" "rules/$rule"
done
diff_one_file "$SUITE_ROOT/global-rules/RTK.md" "$TARGET_HOME/.codex/RTK.md" "RTK.md"

# Check skills
echo "--- Skills ---"
for skill_dir in "$SUITE_ROOT/global-skills/"*; do
    skill_name="$(basename "$skill_dir")"
    diff_one_dir "$skill_dir" "$TARGET_HOME/.agents/skills/$skill_name" "skills/$skill_name"
done

# Scan for local-only skills (directories in ~/.agents/skills/ not in suite)
echo "--- Local-Only Skills ---"
if [[ -d "$TARGET_HOME/.agents/skills" ]]; then
    for local_skill_dir in "$TARGET_HOME/.agents/skills/"*; do
        [[ -d "$local_skill_dir" ]] || continue
        local_skill_name="$(basename "$local_skill_dir")"
        suite_skill_dir="$SUITE_ROOT/global-skills/$local_skill_name"
        if [[ ! -d "$suite_skill_dir" ]]; then
            file_count=$(find "$local_skill_dir" -type f -not -path '*/.git/*' 2>/dev/null | wc -l | tr -d ' ')
            echo "  [LOCAL_ONLY_SKILL] $local_skill_name ($file_count files)"
            ((local_only++)) || true
        fi
    done
fi

# Summary
echo ""
echo "=== Diff Summary ==="
echo "Identical    : $identical"
echo "Content diff : $content_diff"
echo "Suite only   : $suite_only"
echo "Local only   : $local_only"

if [[ $content_diff -gt 0 ]]; then
    echo ""
    echo "To preview what bootstrap would install, run:"
    echo "  bash $SCRIPT_DIR/bootstrap.sh --dry-run"
fi
