#!/usr/bin/env bash
# run-task-card.sh — compatibility wrapper for AGS task-card preparation.
#
# The canonical gate-first authority is `ags run`. This wrapper only preserves
# the historical script entry point and delegates all validation, gate, policy,
# adapter, and receipt planning to the Rust launch-plan preparer.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat << 'EOF'
Usage: run-task-card.sh <task-card> [--check-only|--dry-run|--approve-writes|--current-task-approval] [--format text|json]

Compatibility wrapper for:
  ags run <task-card>

Options:
  --check-only       Stop after gate check; exit 0 if allowed, 1 if stopped.
  --dry-run          Mark and output the full launch plan as a dry run.
  --approve-writes   Pass the write-approval audit/hint signal to the resolver (may act as the M9 generic-adapter override).
  --current-task-approval
                     Pass live current-task approval as an audit/hint signal (task level does not downgrade the permission mode).
  --format text|json Output format passed through to ags run (default: text).
  --help             Show this message.

The task card path must come FIRST.
EOF
    exit 2
}

if [[ $# -eq 0 ]]; then
    echo "Error: task card path required" >&2
    usage
fi

TASK_CARD_PATH="$1"
shift

RUN_ARGS=("$TASK_CARD_PATH")

while [[ $# -gt 0 ]]; do
    case "$1" in
        --check-only|--dry-run|--approve-writes|--current-task-approval)
            RUN_ARGS+=("$1")
            ;;
        --format)
            if [[ $# -lt 2 ]]; then
                echo "Error: --format requires text or json" >&2
                usage
            fi
            RUN_ARGS+=("$1" "$2")
            shift
            ;;
        --help)
            usage
            ;;
        -*)
            echo "Error: unknown option '$1'" >&2
            usage
            ;;
        *)
            echo "Error: unexpected argument '$1'. Options must come after the task card path." >&2
            usage
            ;;
    esac
    shift
done

if ! command -v ags >/dev/null 2>&1; then
    echo "Error: ags CLI not found on PATH" >&2
    echo "Build/install ags first, then re-run: ags run '$TASK_CARD_PATH'" >&2
    exit 1
fi

cd "$REPO_ROOT"

# `ags run` is intentionally non-executing in 0.3.0. It returns a validated
# LaunchPlan and, when allowed, `HOST_EXECUTION_REQUIRED`. The wrapper therefore
# performs no post-task notifier, receipt, verification, or process orchestration.
exec ags run "${RUN_ARGS[@]}" <&0
