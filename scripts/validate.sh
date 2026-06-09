#!/usr/bin/env bash
# validate.sh — Rust task-card validator wrapper (canonical gate).
#
# Usage:
#   bash scripts/validate.sh <task-card-file> [...]
#   bash scripts/validate.sh -                    # stdin
#
# The Rust task-card-validator is the sole canonical task-card format gate.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RUST_VALIDATOR=(cargo run -q --manifest-path "$REPO_ROOT/Cargo.toml" -p ags-cli -- task-card-validator)

if [ $# -eq 0 ]; then
    echo "Usage: bash scripts/validate.sh <task-card> [...]"
    echo "       bash scripts/validate.sh -"
    exit 64
fi

overall=0

for file in "$@"; do
    echo "=== $file ==="
    echo -n "  Rust:  "
    if "${RUST_VALIDATOR[@]}" "$file" 2>&1; then
        echo ""
    else
        overall=1
    fi
done

if [ "$overall" -eq 0 ]; then
    echo "All validations passed."
else
    echo "Some validations FAILED (see above)."
fi

exit $overall
