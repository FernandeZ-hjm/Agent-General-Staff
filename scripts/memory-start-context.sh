#!/usr/bin/env bash
set -euo pipefail

# memory-start-context.sh - Read-only task-start context hook.
# Emits a compact local memory reminder for Claude Code and Codex.

REPO_PATH="${REPO_PATH:-$(pwd -P)}"
MEMORY_ROOT="${MEMORY_ROOT:-$HOME/.agents/memory/projects}"
MAX_CAPSULE_LINES="${MAX_CAPSULE_LINES:-160}"
MAX_TASK_MEMORY_LINES="${MAX_TASK_MEMORY_LINES:-120}"

slugify() {
    printf "%s" "$1" | LC_ALL=C tr -cs '[:alnum:]._-' '-' | sed 's/^-//; s/-$//'
}

profile_slug() {
    local profile="$REPO_PATH/config/agent-project-profile.yaml"
    [[ -f "$profile" ]] || return 0

    awk '
        /^[[:space:]]*project:[[:space:]]*$/ { in_project=1; next }
        /^[^[:space:]#][^:]*:/ { in_project=0 }
        in_project && /^[[:space:]]*slug:[[:space:]]*/ {
            sub(/^[[:space:]]*slug:[[:space:]]*/, "", $0)
            sub(/[[:space:]]*#.*/, "", $0)
            gsub(/^[[:space:]"'"'"']+|[[:space:]"'"'"']+$/, "", $0)
            print
            exit
        }
    ' "$profile" 2>/dev/null || true
}

if [[ ! -d "$REPO_PATH" ]]; then
    exit 0
fi

repo_name="$(basename "$REPO_PATH")"
project_slug="$(profile_slug)"
[[ -n "$project_slug" ]] || project_slug="$(slugify "$repo_name")"
[[ -n "$project_slug" ]] || project_slug="project"

project_memory_dir="$MEMORY_ROOT/$project_slug"
capsule="$project_memory_dir/context-capsule.md"
task_memory="$project_memory_dir/task-memory.md"

if [[ ! -f "$capsule" && ! -f "$task_memory" ]]; then
    exit 0
fi

echo "=== Agent Governance Memory Context (read-only) ==="
echo "Project memory: $project_memory_dir"
echo "Read context-capsule.md before task execution. If the task conflicts with ## 项目设计目的, stop and report."
echo "Read task-memory.md for recent task continuity when present."
echo ""

if [[ -f "$capsule" ]]; then
    echo "--- context-capsule.md ---"
    sed -n "1,${MAX_CAPSULE_LINES}p" "$capsule"
    echo ""
fi

if [[ -f "$task_memory" ]]; then
    echo "--- task-memory.md ---"
    sed -n "1,${MAX_TASK_MEMORY_LINES}p" "$task_memory"
    echo ""
fi

echo "=== End Memory Context ==="
