#!/usr/bin/env bash
set -euo pipefail

# context-memory.sh - Local cross-conversation memory helper.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

COMMAND=""
REPO_PATH="$(pwd -P)"
MEMORY_ROOT="${MEMORY_ROOT:-$HOME/.agents/memory/projects}"
DRY_RUN=false
RECEIPT_DIR=""
PROJECT_SLUG_OVERRIDE=""

usage() {
    cat <<EOF
Usage: scripts/context-memory.sh COMMAND [options]

Commands:
  status                 Show memory paths for the current project.
  init                   Create the project memory directory and capsule.
  capture RECEIPT_DIR    Archive a receipt and refresh task-memory.md.

Options:
  --repo PATH            Repository path (default: current directory)
  --memory-root PATH     Memory root (default: \$HOME/.agents/memory/projects)
  --project-slug SLUG    Override project slug for the memory directory.
                         Must use only letters, numbers, dot, underscore, dash.
  --dry-run              Preview writes without changing files
  --help, -h             Show this help

Notes:
  The context capsule is a manual project charter and stable-memory entrypoint.
  Automated capture never overwrites it. Full runner receipts are copied into
  task-archive/ for local-only history.
EOF
}

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

slugify() {
    printf "%s" "$1" | LC_ALL=C tr -cs '[:alnum:]._-' '-' | sed 's/^-//; s/-$//'
}

validate_slug() {
    local slug="$1"
    [[ "$slug" =~ ^[A-Za-z0-9._-]+$ ]]
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

abs_path() {
    local input="$1"
    local dir
    local base
    dir="$(dirname "$input")"
    base="$(basename "$input")"
    (cd "$dir" 2>/dev/null && printf "%s/%s\n" "$(pwd -P)" "$base")
}

timestamp() {
    date +%Y%m%d-%H%M%S
}

extract_task_summary() {
    local file="$1"
    awk '
        /^任务：/ { in_task=1; next }
        in_task && NF {
            gsub(/\r/, "", $0)
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

extract_task_level() {
    local file="$1"
    awk -F: '
        /^任务级别/ {
            sub(/^[^：:]*[：:]/, "", $0)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", $0)
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

extract_report_status() {
    local file="$1"
    awk '
        /^## 任务状态/ { in_status=1; next }
        in_status && NF {
            gsub(/\r/, "", $0)
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        status|init|capture)
            [[ -z "$COMMAND" ]] || die "Only one command is supported"
            COMMAND="$1"
            shift
            if [[ "$COMMAND" == "capture" ]]; then
                [[ $# -gt 0 ]] || die "capture requires RECEIPT_DIR"
                RECEIPT_DIR="$1"
                shift
            fi
            ;;
        --repo)
            [[ $# -ge 2 ]] || die "--repo requires a path"
            REPO_PATH="$2"
            shift 2
            ;;
        --memory-root)
            [[ $# -ge 2 ]] || die "--memory-root requires a path"
            MEMORY_ROOT="$2"
            shift 2
            ;;
        --project-slug)
            [[ $# -ge 2 ]] || die "--project-slug requires a value"
            PROJECT_SLUG_OVERRIDE="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            die "Unknown argument: $1"
            ;;
    esac
done

[[ -n "$COMMAND" ]] || { usage; exit 1; }
[[ -d "$REPO_PATH" ]] || die "Repository path not found: $REPO_PATH"

REPO_PATH="$(abs_path "$REPO_PATH")"
REPO_NAME="$(basename "$REPO_PATH")"
PROJECT_SLUG_SOURCE="repo name"
if [[ -n "$PROJECT_SLUG_OVERRIDE" ]]; then
    PROJECT_SLUG="$PROJECT_SLUG_OVERRIDE"
    PROJECT_SLUG_SOURCE="--project-slug"
else
    PROFILE_SLUG="$(profile_slug)"
    if [[ -n "$PROFILE_SLUG" ]]; then
        PROJECT_SLUG="$PROFILE_SLUG"
        PROJECT_SLUG_SOURCE="config/agent-project-profile.yaml"
    else
        PROJECT_SLUG="$(slugify "$REPO_NAME")"
    fi
fi
[[ -n "$PROJECT_SLUG" ]] || PROJECT_SLUG="project"
validate_slug "$PROJECT_SLUG" || die "Invalid project slug: $PROJECT_SLUG"

PROJECT_MEMORY_DIR="$MEMORY_ROOT/$PROJECT_SLUG"
TASK_ARCHIVE_DIR="$PROJECT_MEMORY_DIR/task-archive"
CAPSULE="$PROJECT_MEMORY_DIR/context-capsule.md"
TASK_MEMORY="$PROJECT_MEMORY_DIR/task-memory.md"
TASK_MEMORY_LIMIT="${TASK_MEMORY_LIMIT:-20}"

print_status() {
    echo "Suite root      : $SUITE_ROOT"
    echo "Repo path       : $REPO_PATH"
    echo "Project slug    : $PROJECT_SLUG"
    echo "Slug source     : $PROJECT_SLUG_SOURCE"
    echo "Memory dir      : $PROJECT_MEMORY_DIR"
    echo "Context capsule : $CAPSULE"
    echo "Task memory     : $TASK_MEMORY"
    echo "Task archive    : $TASK_ARCHIVE_DIR"
    if [[ -f "$CAPSULE" ]]; then
        echo "Capsule status  : present"
    else
        echo "Capsule status  : missing"
    fi
}

write_initial_capsule() {
    if $DRY_RUN; then
        echo "[DRY-RUN] mkdir -p $TASK_ARCHIVE_DIR"
        echo "[DRY-RUN] write missing $CAPSULE $TASK_MEMORY"
        return 0
    fi

    mkdir -p "$TASK_ARCHIVE_DIR"
    if [[ ! -f "$CAPSULE" ]]; then
        cat >"$CAPSULE" <<EOF
# Context Capsule: $REPO_NAME

Manual-maintained project charter.

## 项目设计目的

<只能人工修改。用于约束 AI 不偏离项目初衷、业务边界、产品方向。>

规则：
- runner / hook / capture 不得覆盖本板块。
- 自动总结不得改写本板块。
- 只能由用户明确要求时手动修改本板块。
- 每次任务开始前 AI 必须读取本文件。
- 如果任务目标和项目设计目的冲突，AI 必须停下来报告。

## Stable Facts

- Project path: \`$REPO_PATH\`
- Memory dir: \`$PROJECT_MEMORY_DIR\`

## 人工维护边界

- 项目长期边界：只能人工维护；自动流程不得改写。
- 核心业务定位：只能人工维护；自动流程不得改写。
- 原则性决策：只能人工维护；自动流程不得改写。

## 任务记忆入口

- Task memory: \`$TASK_MEMORY\`
- Task archive: \`$TASK_ARCHIVE_DIR\`
EOF
        echo "[OK] Created context capsule: $CAPSULE"
    else
        echo "[OK] Context capsule already exists: $CAPSULE"
    fi

    if [[ ! -f "$TASK_MEMORY" ]]; then
        cat >"$TASK_MEMORY" <<EOF
# Task Memory: $REPO_NAME

Updated: never

This file is automatically refreshed from local task archives. It helps agents
resume work without overwriting the manual project charter.

## Current Status

- No task archives recorded yet.

## Recent Tasks

- No task archives recorded yet.

## Task Archive Index

- Task archive: \`$TASK_ARCHIVE_DIR\`
EOF
        echo "[OK] Created task memory: $TASK_MEMORY"
    else
        echo "[OK] Task memory already exists: $TASK_MEMORY"
    fi
}

extract_report_conclusion() {
    local file="$1"
    awk '
        /^一句话结论[：:]/ {
            sub(/^一句话结论[：:][[:space:]]*/, "", $0)
            if (NF) {
                print
                exit
            }
            getline
            gsub(/\r/, "", $0)
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

extract_markdown_section() {
    local file="$1"
    local heading="$2"
    local max_lines="${3:-12}"
    awk -v heading="$heading" -v max_lines="$max_lines" '
        $0 == heading { in_section=1; next }
        in_section && /^## / { exit }
        in_section {
            gsub(/\r/, "", $0)
            print
            count++
            if (count >= max_lines) exit
        }
    ' "$file" 2>/dev/null | sed '/^[[:space:]]*$/d' || true
}

refresh_task_memory() {
    if $DRY_RUN; then
        echo "[DRY-RUN] refresh $TASK_MEMORY"
        return 0
    fi

    mkdir -p "$PROJECT_MEMORY_DIR"
    {
        echo "# Task Memory: $REPO_NAME"
        echo ""
        echo "Updated: $(timestamp)"
        echo ""
        echo "This file is automatically refreshed from local task archives. It is"
        echo "the task-continuity entrypoint for agents; the manual project charter"
        echo "remains in \`context-capsule.md\`."
        echo ""
        local latest_archive
        latest_archive="$(find "$TASK_ARCHIVE_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -r | head -n 1 || true)"
        echo "## Current Status"
        echo ""
        if [[ -n "$latest_archive" && -f "$latest_archive/delivery-report.md" ]]; then
            local latest_task
            local latest_status
            local latest_conclusion
            latest_task="$(extract_task_summary "$latest_archive/task-card.md")"
            latest_status="$(extract_report_status "$latest_archive/delivery-report.md")"
            latest_conclusion="$(extract_report_conclusion "$latest_archive/delivery-report.md")"
            [[ -n "$latest_task" ]] || latest_task="Unspecified task"
            [[ -n "$latest_status" ]] || latest_status="Unknown"
            [[ -n "$latest_conclusion" ]] || latest_conclusion="See delivery report."
            echo "- Latest task: $latest_task"
            echo "- Status: $latest_status"
            echo "- Conclusion: $latest_conclusion"
            echo "- Archive: \`$latest_archive\`"
        else
            echo "- No task archives recorded yet."
        fi
        echo ""
        echo "## Latest Delivery Report"
        echo ""
        if [[ -n "$latest_archive" && -f "$latest_archive/delivery-report.md" ]]; then
            echo "- Source: \`$latest_archive/delivery-report.md\`"
            echo ""
            sed -n '1,80p' "$latest_archive/delivery-report.md"
        else
            echo "- No delivery report recorded yet."
        fi
        echo ""
        echo "## Recent Tasks"
        echo ""
        find "$TASK_ARCHIVE_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null \
            | sort -r \
            | head -n "$TASK_MEMORY_LIMIT" \
            | while IFS= read -r dir; do
                local task_card="$dir/task-card.md"
                local report="$dir/delivery-report.md"
                local task_summary
                local status
                local conclusion
                task_summary="$(extract_task_summary "$task_card")"
                status="$(extract_report_status "$report")"
                conclusion="$(extract_report_conclusion "$report")"
                [[ -n "$task_summary" ]] || task_summary="$(basename "$dir")"
                [[ -n "$status" ]] || status="Unknown"
                if [[ -n "$conclusion" ]]; then
                    echo "- $task_summary | $status | $conclusion | \`$dir\`"
                else
                    echo "- $task_summary | $status | \`$dir\`"
                fi
            done
        echo ""
        echo "## Task Archive Index"
        echo ""
        echo "- Task archive root: \`$TASK_ARCHIVE_DIR\`"
        echo "- Each archive stores the full task card, delivery report, verification log, diff stat, and runner metadata."
    } >"$TASK_MEMORY"
}

capture_receipt() {
    [[ -n "$RECEIPT_DIR" ]] || die "capture requires RECEIPT_DIR"
    [[ -d "$RECEIPT_DIR" ]] || die "Receipt directory not found: $RECEIPT_DIR"

    local receipt_abs
    local task_card
    local delivery_report
    local archive_dir
    local now
    local task_summary

    receipt_abs="$(abs_path "$RECEIPT_DIR")"
    task_card="$receipt_abs/task-card.md"
    delivery_report="$receipt_abs/delivery-report.md"

    [[ -f "$task_card" ]] || die "Missing receipt task-card.md: $task_card"
    [[ -f "$delivery_report" ]] || die "Missing receipt delivery-report.md: $delivery_report"

    now="$(timestamp)"
    task_summary="$(extract_task_summary "$task_card")"
    [[ -n "$task_summary" ]] || task_summary="Unspecified task"

    archive_dir="$TASK_ARCHIVE_DIR/$now-$(basename "$receipt_abs")"

    if $DRY_RUN; then
        echo "[DRY-RUN] mkdir -p $archive_dir"
        echo "[DRY-RUN] copy receipt to $archive_dir"
        echo "[DRY-RUN] refresh $TASK_MEMORY"
        return 0
    fi

    mkdir -p "$TASK_ARCHIVE_DIR"
    if [[ ! -f "$CAPSULE" ]]; then
        write_initial_capsule >/dev/null
    elif [[ ! -f "$TASK_MEMORY" ]]; then
        write_initial_capsule >/dev/null
    fi
    if [[ -e "$archive_dir" ]]; then
        archive_dir="$archive_dir-$$"
    fi
    mkdir -p "$archive_dir"
    cp -R "$receipt_abs/." "$archive_dir/"

    refresh_task_memory

    echo "[OK] Archived receipt: $archive_dir"
    echo "[OK] Refreshed task memory: $TASK_MEMORY"
    echo "[OK] Context capsule unchanged: $CAPSULE"
}

case "$COMMAND" in
    status)
        print_status
        ;;
    init)
        print_status
        write_initial_capsule
        ;;
    capture)
        capture_receipt
        ;;
esac
