#!/usr/bin/env bash
set -euo pipefail

# validate-task-card.sh - Read-only format gate for generated task cards.

usage() {
    cat <<EOF
Usage: validate-task-card.sh TASK_CARD [...]
       validate-task-card.sh -

Checks that generated task cards use the canonical Agent Governance Suite
Markdown shape before a runner or human treats them as executable.
Use "-" to validate task-card content from stdin.
EOF
}

failures=0

fail() {
    local file="$1"
    shift
    echo "[FAIL] $file: $*" >&2
    failures=$((failures + 1))
}

ok() {
    local file="$1"
    shift
    echo "[OK] $file: $*"
}

first_nonempty_line() {
    awk '
        {
            line = $0
            gsub(/\r/, "", line)
            if (line !~ /^[[:space:]]*$/) {
                print line
                exit
            }
        }
    ' "$1"
}

has_literal() {
    local file="$1"
    local needle="$2"
    grep -Fq "$needle" "$file" 2>/dev/null
}

validate_required_fields() {
    local file="$1"
    shift
    local missing=0
    local field

    for field in "$@"; do
        if ! has_literal "$file" "$field"; then
            fail "$file" "missing required field: $field"
            missing=$((missing + 1))
        fi
    done

    [[ "$missing" -eq 0 ]]
}

validate_file() {
    local file="$1"
    local first_line
    local file_start_failures="$failures"

    if [[ ! -f "$file" ]]; then
        fail "$file" "file not found"
        return
    fi

    first_line="$(first_nonempty_line "$file")"
    if [[ "$first_line" != "## 任务卡" ]]; then
        fail "$file" "first non-empty line must be exactly: ## 任务卡"
    fi

    if has_literal "$file" '```text'; then
        fail "$file" 'outer task-card fence must not be ```text'
    fi

    if has_literal "$file" "AGENT_SUITE_COMPACT_TASK_CARD_V1"; then
        validate_required_fields "$file" \
            "## 任务卡" \
            "AGENT_SUITE_COMPACT_TASK_CARD_V1" \
            "路径：" \
            "Executor:" \
            "Runtime adapter:" \
            "Execution surface:" \
            "Permission mode:" \
            "Parallelism:" \
            "任务级别" \
            "读取：" \
            "任务：" \
            "目标：" \
            "非目标：" \
            "关键路径：" \
            "验证：" \
            "停止条件：" \
            "交付：" || true
    else
        validate_required_fields "$file" \
            "## 任务卡" \
            "读取并遵守：" \
            "Executor:" \
            "Runtime adapter:" \
            "Execution surface:" \
            "Permission mode:" \
            "Parallelism:" \
            "任务级别" \
            "Review gate:" \
            "任务：" \
            "背景：" \
            "项目画像：" \
            "记忆胶囊：" \
            "任务存档：" \
            "相关路径：" \
            "本次任务相关文件：" \
            "目标：" \
            "非目标：" \
            "验证：" \
            "Verification gate:" \
            "交付：" || true
    fi

    if [[ "$failures" -eq "$file_start_failures" ]]; then
        ok "$file" "canonical task-card format"
    fi
}

if [[ $# -eq 0 ]]; then
    usage >&2
    exit 64
fi

tmp_files=()
cleanup() {
    if [[ ${#tmp_files[@]} -gt 0 ]]; then
        rm -f "${tmp_files[@]}"
    fi
}
trap cleanup EXIT

for file in "$@"; do
    if [[ "$file" == "-" ]]; then
        tmp="$(mktemp)"
        tmp_files+=("$tmp")
        cat >"$tmp"
        validate_file "$tmp"
        continue
    fi
    validate_file "$file"
done

if [[ "$failures" -gt 0 ]]; then
    echo "[ERROR] task-card validation failed: $failures issue(s)" >&2
    exit 1
fi
