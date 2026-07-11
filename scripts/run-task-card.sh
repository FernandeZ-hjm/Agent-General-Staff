#!/usr/bin/env bash
# run-task-card.sh — compatibility wrapper for AGS task-card execution.
#
# The canonical gate-first authority is `ags run`. This wrapper only preserves
# the historical script entry point and delegates all validation, gate, policy,
# adapter, and receipt planning to the Rust runner.

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
  --dry-run          Output the full launch plan without executing.
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

# Run the canonical runner and PRESERVE its verdict. We no longer `exec` so that
# a best-effort, post-task update notifier can run AFTER the runner's receipt /
# verification. Because the wrapper now keeps a shell parent, it MUST forward
# cancellation signals to the child: killing the wrapper PID must stop the child
# `ags run` (a Heavy/write task card must not keep executing past cancellation).
# `set +e` stays off through `exit` so nothing below changes the runner's verdict.
set +e

run_rc=0
child_pid=""
cancelled=0
forward_signal() {
    cancelled=1
    if [ -n "$child_pid" ]; then
        kill -"$1" "$child_pid" 2>/dev/null || true
    fi
}
trap 'forward_signal TERM' TERM
trap 'forward_signal INT' INT
trap 'forward_signal HUP' HUP

# Tracked child (stdin preserved via <&0). Backgrounding + `wait` is what lets a
# trapped signal run promptly and be forwarded to the child.
ags run "${RUN_ARGS[@]}" <&0 &
child_pid=$!

# Wait for the child, re-waiting if a trapped signal interrupts `wait` while the
# child is still alive. The loop ends once the child is actually gone, with
# run_rc holding the child's real exit / signal-death status.
while :; do
    wait "$child_pid"
    run_rc=$?
    kill -0 "$child_pid" 2>/dev/null || break
done

trap - TERM INT HUP

# Only run the notifier after a NORMAL completion (not cancellation) AND a REAL
# execution (not --check-only / --dry-run gate previews, which have no task end).
notifier_eligible=1
for arg in "${RUN_ARGS[@]}"; do
    case "$arg" in
        --check-only|--dry-run) notifier_eligible=0 ;;
    esac
done

# Post-task update notifier (lazy, throttled, silent on failure). JSON is the
# authority; a reminder is printed to stderr ONLY when an update is available.
# It never changes run_rc and never blocks the runner verdict.
if [ "$cancelled" = "0" ] \
    && [ "$notifier_eligible" = "1" ] \
    && [ "${AGS_NO_UPDATE_NOTIFIER:-}" != "1" ]; then
    notify_json="$(ags update notify --format json 2>/dev/null)"
    case "$notify_json" in
        *'"notify": true'*)
            notify_msg="$(printf '%s' "$notify_json" \
                | sed -n 's/.*"message"[[:space:]]*:[[:space:]]*"\(.*\)".*/\1/p' \
                | head -n 1)"
            [ -n "$notify_msg" ] && printf '%s\n' "$notify_msg" >&2
            ;;
    esac
fi

exit "$run_rc"
