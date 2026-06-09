#!/usr/bin/env bash
# stop-archive-hook.sh — AGS public-safe Stop hook for Claude Code.
#
# Archives delivery reports, verification summaries, and receipts to the
# user's local memory directory when a task completes. Designed as a
# non-blocking Claude Code Stop hook.
#
# ## Installation (manual only — never auto-installed)
#
# Users must manually add this hook to their Claude Code settings:
#
#   1. Open ~/.claude/settings.json
#   2. Add or update the "hooks" section:
#
#   ```json
#   {
#     "hooks": {
#       "Stop": [
#         {
#           "type": "command",
#           "command": "bash /path/to/ags/scripts/stop-archive-hook.sh"
#         }
#       ]
#     }
#   }
#   ```
#
#   3. Replace /path/to/ags/ with the actual AGS installation path.
#
# ## What it does
#
# On each Claude Code Stop event, this hook:
#   1. Detects the current project slug from the working directory
#   2. Creates the memory archive directory if needed
#   3. Searches for delivery reports and verification results in common locations
#   4. Calls `ags archive` to persist them to the memory directory
#   5. Updates task-memory.md with the latest archive reference
#
# ## Public boundary
#
# - All paths use $HOME, $AGS_MEMORY_DIR, or the current working directory.
# - No hardcoded private paths, user names, or repo paths.
# - No private memory, receipt, or task history is shipped.
# - No third-party hooks are auto-installed.
#
# ## Safety
#
# - Read-only detection phase first; writes only to the memory directory.
# - Never modifies Claude Code settings or project files.
# - Non-blocking — failures are logged but don't prevent the Stop event.
# - Respects AGS_MEMORY_DIR and AGS_DISABLE_STOP_HOOK env vars.

set -euo pipefail

# Respect disable flag
if [[ "${AGS_DISABLE_STOP_HOOK:-}" == "1" ]]; then
    exit 0
fi

# Determine memory base directory
MEMORY_BASE="${AGS_MEMORY_DIR:-}"
if [[ -z "$MEMORY_BASE" ]]; then
    MEMORY_BASE="${HOME}/.agents/memory/projects"
fi

# Detect project slug from current directory
PROJECT_DIR="${PWD}"
PROJECT_SLUG="$(basename "$PROJECT_DIR")"
if [[ -z "$PROJECT_SLUG" ]]; then
    PROJECT_SLUG="unknown-project"
fi

MEMORY_DIR="${MEMORY_BASE}/${PROJECT_SLUG}"
ARCHIVE_DIR="${MEMORY_DIR}/task-archive"

# Ensure archive directory exists
mkdir -p "$ARCHIVE_DIR" 2>/dev/null || true

# Find delivery report — check common locations
DELIVERY_REPORT=""
for candidate in \
    "${PROJECT_DIR}/delivery-report.md" \
    "${PROJECT_DIR}/docs/delivery-report.md" \
    "${PROJECT_DIR}/.claude/delivery-report.md" \
    /tmp/delivery-report.md \
    /tmp/ags-delivery-report.md; do
    if [[ -f "$candidate" ]]; then
        DELIVERY_REPORT="$candidate"
        break
    fi
done

# Find verification results
VERIFICATION_RESULTS=""
for candidate in \
    "${PROJECT_DIR}/verification-results.json" \
    "${PROJECT_DIR}/.claude/verification-results.json" \
    /tmp/verification-results.json \
    /tmp/ags-verification-results.json; do
    if [[ -f "$candidate" ]]; then
        VERIFICATION_RESULTS="$candidate"
        break
    fi
done

# Find receipt
RECEIPT=""
for candidate in \
    "${PROJECT_DIR}/receipt.json" \
    "${PROJECT_DIR}/.claude/receipt.json" \
    /tmp/ags-receipt.json; do
    if [[ -f "$candidate" ]]; then
        RECEIPT="$candidate"
        break
    fi
done

# Build ags archive command
ARCHIVE_ARGS=()
if [[ -n "$DELIVERY_REPORT" ]]; then
    ARCHIVE_ARGS+=(--delivery-report "$DELIVERY_REPORT")
fi
if [[ -n "$VERIFICATION_RESULTS" ]]; then
    ARCHIVE_ARGS+=(--verification-results "$VERIFICATION_RESULTS")
fi
if [[ -n "$RECEIPT" ]]; then
    ARCHIVE_ARGS+=(--receipt "$RECEIPT")
fi

SUMMARY=""
# Try to extract a one-line summary from the delivery report
if [[ -n "$DELIVERY_REPORT" ]] && [[ -f "$DELIVERY_REPORT" ]]; then
    SUMMARY="$(head -20 "$DELIVERY_REPORT" | grep -m1 '^一句话结论：\|^## 任务状态' | head -1 | sed 's/^##\s*//' | tr '\n' ' ' || true)"
fi
if [[ -z "$SUMMARY" ]]; then
    SUMMARY="Task completed at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi
ARCHIVE_ARGS+=(--summary "$SUMMARY")

ARCHIVE_ARGS+=(--format text)

# Find the ags binary
AGS_BIN=""
if command -v ags &>/dev/null; then
    AGS_BIN="ags"
elif [[ -x "${PROJECT_DIR}/target/release/ags" ]]; then
    AGS_BIN="${PROJECT_DIR}/target/release/ags"
elif [[ -x "${PROJECT_DIR}/target/debug/ags" ]]; then
    AGS_BIN="${PROJECT_DIR}/target/debug/ags"
fi

if [[ -z "$AGS_BIN" ]]; then
    # ags not found — write a minimal archive manually
    TIMESTAMP="$(date -u +%s)"
    ARCHIVE_FILE="${ARCHIVE_DIR}/${TIMESTAMP}-archive.md"
    {
        echo "# Task Archive (manual)"
        echo ""
        echo "Timestamp: ${TIMESTAMP}"
        echo "Summary: ${SUMMARY}"
        echo ""
        echo "## Note"
        echo "ags binary not found. Install AGS for automatic archiving."
        echo "Run: cd /path/to/ags && cargo build --release"
    } > "$ARCHIVE_FILE"
    exit 0
fi

# Run ags archive
AGS_MEMORY_DIR="$MEMORY_BASE" "$AGS_BIN" archive \
    "${ARCHIVE_ARGS[@]}" 2>/tmp/ags-stop-hook-errors.log || {
    # Non-blocking: log failures but don't prevent Stop
    echo "[AGS stop-hook] archive command failed — see /tmp/ags-stop-hook-errors.log" >&2
    exit 0
}

exit 0
