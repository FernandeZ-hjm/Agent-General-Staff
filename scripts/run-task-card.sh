#!/usr/bin/env bash
set -euo pipefail

# run-task-card.sh - Capture a task execution receipt package, optionally
# launch one Claude Code session, then run task-card Verification gate commands.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<EOF
Usage: run-task-card.sh TASK_CARD [--auto] [--claude] [--headless] [--worktree] [--parallel] [--memory] [--receipt-root PATH] [--label NAME]

Default behavior:
  Creates a receipt directory with task-card, git status snapshots, hook/skill
  checks, bare remote check, diff stat, verification placeholder, and delivery
  report. Without --claude it does not start Claude Code and does not execute
  task-card commands.

Options:
  --auto               Conservative orchestrator mode. Reads task-card Executor,
                       Runtime adapter, Execution surface, Permission mode, and
                       Parallelism to select runner flags. It never escalates
                       the task-card Permission mode.
  --claude             Launch one Claude Code session with the task card.
  --headless           Launch Claude Code in non-interactive print mode.
                       Implies --claude. Claude output is captured in the
                       receipt package as claude-output.log.
  --worktree           Create an isolated git worktree for the Claude run and
                       execute task-card Verification gate commands there.
                       Implies --claude.
  --parallel           Allow the Claude run to use the task-card declared
                       Parallelism mode. Implies --claude and requires
                       Parallelism to be subagent, worktree, multi-session,
                       or agent-team.
  --memory             Archive the receipt under local context memory and
                       refresh the capsule after the receipt and delivery report
                       are written. Uses
                       scripts/context-memory.sh.
  --memory-root PATH   Directory where project memory is written.
                       Default: \$HOME/.agents/memory/projects
  --receipt-root PATH  Directory where receipt packages are written.
                       Default: \$HOME/.agents/task-receipts/<repo-name>
  --label NAME         Human-readable label used in the receipt directory name.
  --help, -h           Show this help.
EOF
}

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

timestamp() {
    date +%Y%m%d-%H%M%S
}

slugify() {
    printf "%s" "$1" | LC_ALL=C tr -cs '[:alnum:]._-' '-' | sed 's/^-//; s/-$//'
}

abs_path() {
    local input="$1"
    local dir
    local base
    dir="$(dirname "$input")"
    base="$(basename "$input")"
    (cd "$dir" 2>/dev/null && printf "%s/%s\n" "$(pwd -P)" "$base")
}

run_log() {
    local logfile="$1"
    shift
    {
        printf '$'
        printf ' %q' "$@"
        printf '\n'
        set +e
        "$@"
        local status=$?
        set -e
        printf '\n[exit %s]\n' "$status"
    } >>"$logfile" 2>&1
}

section() {
    local logfile="$1"
    local title="$2"
    {
        echo ""
        echo "=== $title ==="
    } >>"$logfile"
}

extract_task_field() {
    local field="$1"
    local file="$2"
    awk -F: -v wanted="$field" '
        BEGIN { wanted = tolower(wanted) }
        {
            key = tolower($1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if (key == wanted) {
                sub(/^[^:]*:/, "", $0)
                gsub(/\r/, "", $0)
                gsub(/^[[:space:]]+|[[:space:]]+$/, "", $0)
                print $0
                exit
            }
        }
    ' "$file"
}

extract_verification_commands() {
    local file="$1"
    awk '
        /^Verification gate:/ {
            in_gate = 1
            next
        }
        in_gate && /^[[:space:]]*-[[:space:]]*commands:/ {
            in_commands = 1
            next
        }
        in_commands && /^[[:space:]]*-[[:space:]]*(expected evidence|stop condition):/ {
            exit
        }
        in_commands {
            line = $0
            gsub(/\r/, "", line)
            if (line ~ /^[[:space:]]*-[[:space:]]+/) {
                sub(/^[[:space:]]*-[[:space:]]+/, "", line)
                gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
                if (line ~ /^`.*`$/) {
                    sub(/^`/, "", line)
                    sub(/`$/, "", line)
                }
                if (line != "") {
                    print line
                }
            }
        }
    ' "$file"
}

append_verification_gate_report() {
    local gate_status="$1"
    local command_summary="$2"

    {
        echo ""
        echo "## Runner Verification Gate"
        echo ""
        echo "状态：$gate_status"
        echo ""
        echo "自动执行命令："
        echo ""
        if [[ -n "$command_summary" ]]; then
            printf "%b\n" "$command_summary"
        else
            echo "- 无"
        fi
        echo ""
        echo "详细日志：\`$RECEIPT_DIR/verification.log\`"
    } >>"$RECEIPT_DIR/delivery-report.md"
}

run_verification_gate() {
    local commands=()
    local cmd
    local gate_failed=0
    local command_summary=""

    while IFS= read -r cmd; do
        [[ -n "$cmd" ]] && commands+=("$cmd")
    done < <(extract_verification_commands "$TASK_CARD_ABS")

    section "$VERIFY_LOG" "runner verification gate"

    if [[ ${#commands[@]} -eq 0 ]]; then
        echo "[INFO] No Verification gate commands found in task card." >>"$VERIFY_LOG"
        append_verification_gate_report "未运行" "- 未找到 Verification gate commands。"
        return 0
    fi

    echo "Command count: ${#commands[@]}" >>"$VERIFY_LOG"

    for cmd in "${commands[@]}"; do
        {
            echo ""
            echo "--- command ---"
            echo "$cmd"
            echo "--- output ---"
        } >>"$VERIFY_LOG"

        set +e
        (cd "$REPO_ROOT" && bash -lc "$cmd") >>"$VERIFY_LOG" 2>&1
        local status=$?
        set -e

        echo "--- exit: $status ---" >>"$VERIFY_LOG"

        if [[ "$status" -eq 0 ]]; then
            command_summary="${command_summary}- \`$cmd\` → 通过\n"
        else
            command_summary="${command_summary}- \`$cmd\` → 失败（exit ${status}）\n"
            gate_failed=1
        fi
    done

    if [[ "$gate_failed" -eq 0 ]]; then
        append_verification_gate_report "通过" "$command_summary"
        return 0
    fi

    append_verification_gate_report "失败" "$command_summary"
    return 1
}

write_runner_delivery_report() {
    local status="$1"
    local summary="$2"
    local unverified="$3"
    local risk="$4"
    local include_marker="${5:-false}"

    {
        cat <<EOF
# 任务交付报告

EOF
        if [[ "$include_marker" == "true" ]]; then
            echo "<!-- runner-placeholder -->"
            echo ""
        fi
        cat <<EOF
## 任务状态

$status

一句话结论：

$summary

## 改动内容

修改文件：

- 无

新增文件 / 输出物：

- \`$RECEIPT_DIR\`：任务执行收据包

删除文件：

- 无

## 验证结果

已运行：

\`\`\`bash
git status --short
git diff --stat
git diff --cached --stat
hook source and installed hook checks
task-card skill tag checks
local bare remote checks
\`\`\`

结果：

- 证据已分别写入 \`git-status.before.txt\`、\`git-status.after.txt\`、\`hook-check.txt\`、\`skill-check.txt\`、\`bare-remote-check.txt\`、\`verification.log\`、\`diff-stat.txt\`。
- 编排决策写入 \`orchestrator-info.txt\`。

未验证内容：

EOF
        printf "%b\n" "$unverified"
        cat <<EOF

## 风险提示

EOF
        printf "%b\n" "$risk"
        cat <<EOF

## 下一步建议

- 按收据包、Verification gate 和任务卡交付报告复核执行结果。
EOF
    } >"$RECEIPT_DIR/delivery-report.md"
}

TASK_CARD=""
RECEIPT_ROOT=""
LABEL=""
RUN_CLAUDE=false
HEADLESS=false
USE_WORKTREE=false
ALLOW_PARALLEL=false
UPDATE_MEMORY=false
MEMORY_ROOT=""
AUTO_MODE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --auto)
            AUTO_MODE=true
            ;;
        --receipt-root)
            [[ $# -ge 2 ]] || die "--receipt-root requires a path"
            RECEIPT_ROOT="$2"
            shift
            ;;
        --label)
            [[ $# -ge 2 ]] || die "--label requires a value"
            LABEL="$2"
            shift
            ;;
        --claude)
            RUN_CLAUDE=true
            ;;
        --headless)
            RUN_CLAUDE=true
            HEADLESS=true
            ;;
        --worktree)
            RUN_CLAUDE=true
            USE_WORKTREE=true
            ;;
        --parallel)
            RUN_CLAUDE=true
            ALLOW_PARALLEL=true
            ;;
        --memory)
            UPDATE_MEMORY=true
            ;;
        --memory-root)
            [[ $# -ge 2 ]] || die "--memory-root requires a path"
            MEMORY_ROOT="$2"
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        -*)
            die "Unknown option: $1"
            ;;
        *)
            if [[ -n "$TASK_CARD" ]]; then
                die "Only one TASK_CARD is supported"
            fi
            TASK_CARD="$1"
            ;;
    esac
    shift
done

[[ -n "$TASK_CARD" ]] || { usage; exit 1; }
[[ -f "$TASK_CARD" ]] || die "Task card not found: $TASK_CARD"

TASK_CARD_ABS="$(abs_path "$TASK_CARD")"
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)"
ORIGINAL_REPO_ROOT="$REPO_ROOT"
REPO_NAME="$(basename "$REPO_ROOT")"
REPO_SLUG="$(slugify "$REPO_NAME")"

if $AUTO_MODE; then
    EXECUTOR_RAW="$(extract_task_field "executor" "$TASK_CARD_ABS")"
    RUNTIME_RAW="$(extract_task_field "runtime adapter" "$TASK_CARD_ABS")"
    SURFACE_RAW="$(extract_task_field "execution surface" "$TASK_CARD_ABS")"
    PERMISSION_RAW="$(extract_task_field "permission mode" "$TASK_CARD_ABS")"
    PARALLELISM_FOR_AUTO="$(extract_task_field "parallelism" "$TASK_CARD_ABS")"

    EXECUTOR_MODE="$(printf "%s" "$EXECUTOR_RAW" | LC_ALL=C tr '[:upper:]' '[:lower:]')"
    RUNTIME_MODE="$(printf "%s" "$RUNTIME_RAW" | LC_ALL=C tr '[:upper:]' '[:lower:]')"
    SURFACE_MODE="$(printf "%s" "$SURFACE_RAW" | LC_ALL=C tr '[:upper:]' '[:lower:]')"
    PARALLELISM_MODE_FOR_AUTO="$(printf "%s" "$PARALLELISM_FOR_AUTO" | LC_ALL=C tr '[:upper:]' '[:lower:]' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"

    if [[ "$RUNTIME_MODE" == "claude-code" || "$EXECUTOR_MODE" == "claude code" ]]; then
        RUN_CLAUDE=true
    fi

    if [[ "$SURFACE_MODE" == "background-agent" ]]; then
        RUN_CLAUDE=true
        HEADLESS=true
    fi

    case "$PARALLELISM_MODE_FOR_AUTO" in
        subagent|multi-session|agent-team)
            RUN_CLAUDE=true
            ALLOW_PARALLEL=true
            ;;
        worktree)
            RUN_CLAUDE=true
            ALLOW_PARALLEL=true
            USE_WORKTREE=true
            ;;
        ""|none)
            ;;
        *)
            die "--auto found unsupported task-card Parallelism: $PARALLELISM_FOR_AUTO"
            ;;
    esac
fi

if [[ -z "$RECEIPT_ROOT" ]]; then
    RECEIPT_ROOT="$HOME/.agents/task-receipts/$REPO_SLUG"
fi

if [[ -z "$LABEL" ]]; then
    LABEL="$(basename "$TASK_CARD")"
fi
LABEL_SLUG="$(slugify "$LABEL")"
[[ -n "$LABEL_SLUG" ]] || LABEL_SLUG="task"

RUN_ID="$(timestamp)-$LABEL_SLUG"
RECEIPT_DIR="$RECEIPT_ROOT/$RUN_ID"
if [[ -e "$RECEIPT_DIR" ]]; then
    RECEIPT_DIR="$RECEIPT_DIR-$$"
    RUN_ID="$(basename "$RECEIPT_DIR")"
fi

mkdir -p "$RECEIPT_DIR"

cp "$TASK_CARD_ABS" "$RECEIPT_DIR/task-card.md"

ORCHESTRATOR_LOG="$RECEIPT_DIR/orchestrator-info.txt"
{
    echo "Orchestrator mode"
    echo "Enabled            : $AUTO_MODE"
    if $AUTO_MODE; then
        echo "Executor           : ${EXECUTOR_RAW:-<missing>}"
        echo "Runtime adapter    : ${RUNTIME_RAW:-<missing>}"
        echo "Execution surface  : ${SURFACE_RAW:-<missing>}"
        echo "Permission mode    : ${PERMISSION_RAW:-<missing>}"
        echo "Parallelism        : ${PARALLELISM_FOR_AUTO:-<missing>}"
    fi
    echo "Resolved --claude  : $RUN_CLAUDE"
    echo "Resolved --headless: $HEADLESS"
    echo "Resolved --worktree: $USE_WORKTREE"
    echo "Resolved --parallel: $ALLOW_PARALLEL"
    echo "Resolved --memory  : $UPDATE_MEMORY"
    if $UPDATE_MEMORY; then
        echo "Memory behavior    : archive receipt, refresh task-memory.md"
    fi
    echo "Note: --auto never escalates task-card Permission mode."
} >"$ORCHESTRATOR_LOG"

WORKTREE_LOG="$RECEIPT_DIR/worktree-info.txt"
WORKTREE_DIR=""
WORKTREE_BRANCH=""
{
    echo "Worktree mode"
    echo "Enabled           : $USE_WORKTREE"
    echo "Original repo root: $ORIGINAL_REPO_ROOT"
} >"$WORKTREE_LOG"

if $USE_WORKTREE; then
    if ! git -C "$ORIGINAL_REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        die "--worktree requires a git repository"
    fi

    WORKTREE_ROOT="${WORKTREE_ROOT:-$HOME/.agents/task-worktrees/$REPO_SLUG}"
    WORKTREE_DIR="$WORKTREE_ROOT/$RUN_ID"
    WORKTREE_BRANCH_SLUG="$(printf "%s" "$RUN_ID" | LC_ALL=C tr '[:upper:]' '[:lower:]' | tr -cs '[:alnum:]-' '-' | sed 's/^-//; s/-$//')"
    [[ -n "$WORKTREE_BRANCH_SLUG" ]] || WORKTREE_BRANCH_SLUG="task"
    WORKTREE_BRANCH="codex/task-runner-$WORKTREE_BRANCH_SLUG"
    if git -C "$ORIGINAL_REPO_ROOT" show-ref --verify --quiet "refs/heads/$WORKTREE_BRANCH"; then
        WORKTREE_BRANCH="$WORKTREE_BRANCH-$$"
        WORKTREE_DIR="$WORKTREE_DIR-$$"
    fi

    mkdir -p "$WORKTREE_ROOT"
    {
        echo "Worktree root     : $WORKTREE_ROOT"
        echo "Worktree path     : $WORKTREE_DIR"
        echo "Worktree branch   : $WORKTREE_BRANCH"
        echo ""
        echo "--- git worktree add ---"
        printf '$ git -C %q worktree add -b %q %q HEAD\n' "$ORIGINAL_REPO_ROOT" "$WORKTREE_BRANCH" "$WORKTREE_DIR"
    } >>"$WORKTREE_LOG"

    if git -C "$ORIGINAL_REPO_ROOT" worktree add -b "$WORKTREE_BRANCH" "$WORKTREE_DIR" HEAD >>"$WORKTREE_LOG" 2>&1; then
        REPO_ROOT="$WORKTREE_DIR"
        {
            echo ""
            echo "[OK] Worktree created."
            echo "Execution root    : $REPO_ROOT"
        } >>"$WORKTREE_LOG"
    else
        {
            echo ""
            echo "[FAIL] Could not create worktree."
        } >>"$WORKTREE_LOG"
        die "Failed to create worktree; see $WORKTREE_LOG"
    fi
else
    {
        echo "Execution root    : $REPO_ROOT"
        echo "[INFO] Worktree mode disabled."
    } >>"$WORKTREE_LOG"
fi

PARALLEL_LOG="$RECEIPT_DIR/parallel-info.txt"
PARALLELISM_RAW="$(extract_task_field "parallelism" "$TASK_CARD_ABS")"
PARALLELISM_MODE="$(printf "%s" "$PARALLELISM_RAW" | LC_ALL=C tr '[:upper:]' '[:lower:]' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
{
    echo "Parallel mode"
    echo "Enabled              : $ALLOW_PARALLEL"
    echo "Declared parallelism : ${PARALLELISM_RAW:-<missing>}"
    echo "Execution root       : $REPO_ROOT"
} >"$PARALLEL_LOG"

if $ALLOW_PARALLEL; then
    case "$PARALLELISM_MODE" in
        subagent|multi-session|agent-team)
            echo "[OK] Parallel mode authorized by task card." >>"$PARALLEL_LOG"
            ;;
        worktree)
            if $USE_WORKTREE; then
                echo "[OK] Worktree parallelism authorized with runner worktree isolation." >>"$PARALLEL_LOG"
            else
                echo "[FAIL] Parallelism: worktree requires --worktree." >>"$PARALLEL_LOG"
                die "--parallel with Parallelism: worktree requires --worktree"
            fi
            ;;
        ""|none)
            echo "[FAIL] --parallel requires a non-none task-card Parallelism value." >>"$PARALLEL_LOG"
            die "--parallel requires task card Parallelism to allow parallel work"
            ;;
        *)
            echo "[FAIL] Unsupported task-card Parallelism: $PARALLELISM_RAW" >>"$PARALLEL_LOG"
            die "Unsupported task-card Parallelism for --parallel: $PARALLELISM_RAW"
            ;;
    esac
else
    echo "[INFO] Parallel mode disabled." >>"$PARALLEL_LOG"
fi

capture_git_status() {
    local logfile="$1"
    {
        echo "Repo root: $REPO_ROOT"
        echo "Branch   : $(git -C "$REPO_ROOT" branch --show-current 2>/dev/null || true)"
        echo "HEAD     : $(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || true)"
        echo ""
        echo "--- git status --short ---"
    } >"$logfile"
    git -C "$REPO_ROOT" status --short >>"$logfile" 2>&1 || true
}

capture_git_status "$RECEIPT_DIR/git-status.before.txt"

HOOK_LOG="$RECEIPT_DIR/hook-check.txt"
{
    echo "Hook check"
    echo "Suite root : $SUITE_ROOT"
    echo "Target home: $HOME"
} >"$HOOK_LOG"

section "$HOOK_LOG" "obsolete review hook files"
for hook in leveled-review-gate.mjs review-baseline-snapshot.mjs codex-stop-review-adapter.mjs; do
    installed_hook="$HOME/.claude/hooks/$hook"
    if [[ -f "$installed_hook" ]]; then
        echo "[WARN] obsolete hook still installed: $installed_hook" >>"$HOOK_LOG"
    else
        echo "[OK] obsolete hook absent: $installed_hook" >>"$HOOK_LOG"
    fi
done

section "$HOOK_LOG" "hook configuration files"
for config_file in "$HOME/.claude/settings.json" "$HOME/.codex/hooks.json"; do
    if [[ -f "$config_file" ]]; then
        echo "[OK] config exists: $config_file" >>"$HOOK_LOG"
        if grep -Fq "python3 ~/.claude/sync-skill-aliases.py" "$config_file" 2>/dev/null; then
            echo "  [OK] references sync-skill-aliases.py" >>"$HOOK_LOG"
        else
            echo "  [WARN] missing sync-skill-aliases.py hook" >>"$HOOK_LOG"
        fi
    else
        echo "[MISSING] config: $config_file" >>"$HOOK_LOG"
    fi
done

if [[ -f "$HOME/.claude/settings.json" ]]; then
    if grep -Fq "rtk hook claude" "$HOME/.claude/settings.json" 2>/dev/null; then
        echo "  [OK] Claude settings references rtk hook claude" >>"$HOOK_LOG"
    else
        echo "  [WARN] Claude settings missing rtk hook claude" >>"$HOOK_LOG"
    fi
fi

for config_file in "$HOME/.claude/settings.json" "$HOME/.codex/hooks.json"; do
    [[ -f "$config_file" ]] || continue
    for hook in leveled-review-gate review-baseline-snapshot codex-stop-review-adapter; do
        if grep -Fq "$hook" "$config_file" 2>/dev/null; then
            echo "  [WARN] obsolete review hook reference in $config_file: $hook" >>"$HOOK_LOG"
        else
            echo "  [OK] no obsolete review hook reference in $config_file: $hook" >>"$HOOK_LOG"
        fi
    done
done

SKILL_LOG="$RECEIPT_DIR/skill-check.txt"
{
    echo "Skill check"
    echo "Suite root : $SUITE_ROOT"
    echo "Target home: $HOME"
    echo ""
    echo "--- explicit task-card skill tags ---"
} >"$SKILL_LOG"

skill_tags=()
while IFS= read -r skill; do
    [[ -n "$skill" ]] && skill_tags+=("$skill")
done < <(
    grep -oE '\[skill:[[:space:]]*[^]]+\]' "$TASK_CARD_ABS" 2>/dev/null \
        | sed -E 's/^\[skill:[[:space:]]*//; s/[[:space:]]*\]$//' \
        | awk 'NF' \
        | sort -u
)

if [[ ${#skill_tags[@]} -eq 0 ]]; then
    echo "[INFO] No explicit [skill: ...] tags found." >>"$SKILL_LOG"
else
    for skill in "${skill_tags[@]}"; do
        suite_skill="$SUITE_ROOT/global-skills/$skill/SKILL.md"
        local_skill="$HOME/.agents/skills/$skill/SKILL.md"
        echo "skill/$skill" >>"$SKILL_LOG"
        if [[ -f "$suite_skill" ]]; then
            echo "  [OK] suite skill exists: $suite_skill" >>"$SKILL_LOG"
        else
            echo "  [MISSING] suite skill: $suite_skill" >>"$SKILL_LOG"
        fi
        if [[ -f "$local_skill" ]]; then
            echo "  [OK] installed skill exists: $local_skill" >>"$SKILL_LOG"
        else
            echo "  [MISSING] installed skill: $local_skill" >>"$SKILL_LOG"
        fi
    done
fi

section "$SKILL_LOG" "automatic trigger skills"
for skill in auto-brainstorm auto-debug auto-verify; do
    suite_skill="$SUITE_ROOT/global-skills/$skill/SKILL.md"
    local_skill="$HOME/.agents/skills/$skill/SKILL.md"
    echo "skill/$skill" >>"$SKILL_LOG"
    [[ -f "$suite_skill" ]] && echo "  [OK] suite skill exists: $suite_skill" >>"$SKILL_LOG" || echo "  [MISSING] suite skill: $suite_skill" >>"$SKILL_LOG"
    [[ -f "$local_skill" ]] && echo "  [OK] installed skill exists: $local_skill" >>"$SKILL_LOG" || echo "  [MISSING] installed skill: $local_skill" >>"$SKILL_LOG"
done

REMOTE_LOG="$RECEIPT_DIR/bare-remote-check.txt"
{
    echo "Bare remote check"
    echo "Repo root: $REPO_ROOT"
    echo ""
    echo "--- remotes ---"
} >"$REMOTE_LOG"
git -C "$REPO_ROOT" remote -v >>"$REMOTE_LOG" 2>&1 || true

section "$REMOTE_LOG" "local bare remote probes"
while IFS= read -r remote_name; do
    [[ -n "$remote_name" ]] || continue
    url="$(git -C "$REPO_ROOT" remote get-url "$remote_name" 2>/dev/null || true)"
    [[ -n "$url" ]] || continue
    echo "remote/$remote_name: $url" >>"$REMOTE_LOG"
    local_path="$url"
    if [[ "$local_path" == file://* ]]; then
        local_path="${local_path#file://}"
    fi
    if [[ "$local_path" == /* || "$local_path" == ./* || "$local_path" == ../* ]]; then
        if [[ -d "$local_path" ]]; then
            is_bare="$(git --git-dir="$local_path" rev-parse --is-bare-repository 2>/dev/null || true)"
            if [[ "$is_bare" == "true" ]]; then
                echo "  [OK] local bare repository is reachable" >>"$REMOTE_LOG"
                echo "  HEAD: $(git --git-dir="$local_path" rev-parse --short HEAD 2>/dev/null || true)" >>"$REMOTE_LOG"
            else
                echo "  [WARN] local path exists but is not a bare repository" >>"$REMOTE_LOG"
            fi
        else
            echo "  [MISSING] local remote path does not exist" >>"$REMOTE_LOG"
        fi
    else
        echo "  [INFO] non-local remote; skipped network probe" >>"$REMOTE_LOG"
    fi
done < <(git -C "$REPO_ROOT" remote 2>/dev/null || true)

VERIFY_LOG="$RECEIPT_DIR/verification.log"
{
    echo "Verification log"
    echo ""
    if $RUN_CLAUDE; then
        echo "[INFO] Runner will execute task-card Verification gate commands after Claude Code exits successfully."
        if $HEADLESS; then
            echo "[INFO] Claude Code will run in headless print mode; output is captured in claude-output.log."
        fi
        if $ALLOW_PARALLEL; then
            echo "[INFO] Parallel mode is enabled from task-card Parallelism: ${PARALLELISM_RAW:-<missing>}."
        fi
    else
        echo "[INFO] Receipt-only mode did not execute task-card Verification gate commands."
        echo "[INFO] Pass --claude to run the single-task launcher and automatic Verification gate."
    fi
    echo ""
    echo "--- task-card Verification gate excerpt ---"
} >"$VERIFY_LOG"
awk '
    /^Verification gate:/ { in_gate=1 }
    in_gate { print }
    in_gate && /^交付：/ { exit }
' "$TASK_CARD_ABS" >>"$VERIFY_LOG" 2>/dev/null || true

CLAUDE_EXIT_STATUS=""
if $RUN_CLAUDE; then
    CLAUDE_BIN="${CLAUDE_BIN:-claude}"
    CLAUDE_READY=true
    if ! command -v "$CLAUDE_BIN" >/dev/null 2>&1; then
        write_runner_delivery_report \
            "未完成" \
            "收据包已生成，但未找到 Claude Code 可执行文件：\`$CLAUDE_BIN\`。" \
            "- 未启动 Claude Code。\n- 未执行任务卡 Verification gate；这是 Step 3 的范围。" \
            "- 本机 Claude Code CLI 不可用，任务未执行。"
        section "$VERIFY_LOG" "claude launch"
        echo "[ERROR] Claude Code command not found: $CLAUDE_BIN" >>"$VERIFY_LOG"
        CLAUDE_EXIT_STATUS=127
        CLAUDE_READY=false
    fi

    PERMISSION_MODE="$(extract_task_field "permission mode" "$TASK_CARD_ABS")"
    CLAUDE_PERMISSION_LABEL="default"
    CLAUDE_ARGS=()
    if $CLAUDE_READY; then
        if $HEADLESS; then
            CLAUDE_ARGS+=(--print)
        fi
        case "$PERMISSION_MODE" in
            plan-only|read-only)
                CLAUDE_ARGS+=(--permission-mode plan)
                CLAUDE_PERMISSION_LABEL="plan"
                ;;
            execute-and-verify|edit-with-confirmation|autonomous-low-risk|"")
                CLAUDE_PERMISSION_LABEL="default"
                ;;
            *)
                write_runner_delivery_report \
                    "未完成" \
                    "收据包已生成，但任务卡 Permission mode 不受支持：\`$PERMISSION_MODE\`。" \
                    "- 未启动 Claude Code。\n- 未执行任务卡 Verification gate；这是 Step 3 的范围。" \
                    "- 需要先修正任务卡 Permission mode。"
                section "$VERIFY_LOG" "claude launch"
                echo "[ERROR] Unsupported Permission mode: $PERMISSION_MODE" >>"$VERIFY_LOG"
                CLAUDE_EXIT_STATUS=64
                CLAUDE_READY=false
                ;;
        esac
    fi

    if $CLAUDE_READY; then
        CLAUDE_ARGS+=(--add-dir "$RECEIPT_DIR")

        PARALLEL_PROMPT_BLOCK=""
        if $ALLOW_PARALLEL; then
            PARALLEL_PROMPT_BLOCK="$(cat <<EOF

Runner parallel mode:
- Enabled by runner flag: --parallel
- Task-card Parallelism: ${PARALLELISM_RAW:-<missing>}
- Use only the task-card authorized parallelism mode.
- Keep worker/session scopes disjoint and integrate results before final report.
- Record parallel workers, touched scopes, and integration evidence in delivery-report.md.
EOF
)"
        fi

        CLAUDE_PROMPT="$(cat <<EOF
请读取并执行下面的任务卡。

Runner receipt directory:
$RECEIPT_DIR

Delivery report requirement:
- 完成时必须把最终交付报告写入：$RECEIPT_DIR/delivery-report.md
- 交付报告按任务卡和协议要求填写。
- Runner 会在你退出后自动执行任务卡 Verification gate commands，并把结果追加到 delivery-report.md。
- 你仍应按任务卡自行验证并在报告中记录你的验证结果。
$PARALLEL_PROMPT_BLOCK

Task card source path:
$TASK_CARD_ABS

--- TASK CARD START ---
$(cat "$TASK_CARD_ABS")
--- TASK CARD END ---
EOF
)"

        write_runner_delivery_report \
            "部分完成" \
            "收据包已生成；Claude Code 已由 runner 启动，等待执行器写回最终交付报告。" \
            "- Runner 尚未执行任务卡 Verification gate；会在 Claude Code 退出 0 后自动执行。\n- Claude Code 的实际验证结果以其写入的 \`delivery-report.md\` 为准。" \
            "- 如果 Claude Code 未写回交付报告，本文件仍只是 runner 启动占位报告。" \
            "true"

        section "$VERIFY_LOG" "claude launch"
        {
            echo "Claude binary      : $CLAUDE_BIN"
            echo "Permission mode    : ${PERMISSION_MODE:-<missing>}"
            echo "Claude mode        : $CLAUDE_PERMISSION_LABEL"
            echo "Headless mode      : $HEADLESS"
            echo "Parallel mode      : $ALLOW_PARALLEL"
            if $ALLOW_PARALLEL; then
                echo "Parallelism        : ${PARALLELISM_RAW:-<missing>}"
            fi
            echo "Delivery report    : $RECEIPT_DIR/delivery-report.md"
            if $HEADLESS; then
                echo "Claude output      : $RECEIPT_DIR/claude-output.log"
            fi
            printf "Command            : %q" "$CLAUDE_BIN"
            for arg in "${CLAUDE_ARGS[@]}"; do
                printf " %q" "$arg"
            done
            echo " <task-card-prompt>"
        } >>"$VERIFY_LOG"

        set +e
        if $HEADLESS; then
            (cd "$REPO_ROOT" && "$CLAUDE_BIN" "${CLAUDE_ARGS[@]}" "$CLAUDE_PROMPT") >"$RECEIPT_DIR/claude-output.log" 2>&1
        else
            (cd "$REPO_ROOT" && "$CLAUDE_BIN" "${CLAUDE_ARGS[@]}" "$CLAUDE_PROMPT")
        fi
        CLAUDE_EXIT_STATUS=$?
        set -e

        {
            echo "Claude exit status : $CLAUDE_EXIT_STATUS"
        } >>"$VERIFY_LOG"

        if grep -Fq "<!-- runner-placeholder -->" "$RECEIPT_DIR/delivery-report.md" 2>/dev/null; then
            if [[ "$CLAUDE_EXIT_STATUS" -eq 0 ]]; then
                write_runner_delivery_report \
                    "部分完成" \
                    "Claude Code 进程已退出 0，但未覆盖 runner 占位交付报告。" \
                    "- Claude Code 未把最终交付报告写入收据目录。" \
                    "- 需要人工确认 Claude 终端输出，或让 Claude 补写 \`delivery-report.md\`。"
            else
                write_runner_delivery_report \
                    "未完成" \
                    "Claude Code 进程退出码为 $CLAUDE_EXIT_STATUS，且未写回最终交付报告。" \
                    "- Runner 未自动执行任务卡 Verification gate；这是 Step 3 的范围。\n- Claude Code 未把最终交付报告写入收据目录。" \
                    "- 需要查看终端输出或重新运行任务。"
            fi
        fi

        if [[ "$CLAUDE_EXIT_STATUS" -eq 0 ]]; then
            if run_verification_gate; then
                CLAUDE_EXIT_STATUS=0
            else
                CLAUDE_EXIT_STATUS=3
            fi
        else
            section "$VERIFY_LOG" "runner verification gate"
            echo "[SKIP] Claude Code exit status was $CLAUDE_EXIT_STATUS; runner did not execute Verification gate commands." >>"$VERIFY_LOG"
            append_verification_gate_report "未运行" "- Claude Code 退出码为 $CLAUDE_EXIT_STATUS，runner 跳过自动验证。"
        fi
    fi
fi

DIFF_LOG="$RECEIPT_DIR/diff-stat.txt"
{
    echo "Diff stat"
    echo "Repo root: $REPO_ROOT"
    echo ""
    echo "--- git diff --stat ---"
} >"$DIFF_LOG"
git -C "$REPO_ROOT" diff --stat >>"$DIFF_LOG" 2>&1 || true
{
    echo ""
    echo "--- git diff --cached --stat ---"
} >>"$DIFF_LOG"
git -C "$REPO_ROOT" diff --cached --stat >>"$DIFF_LOG" 2>&1 || true
{
    echo ""
    echo "--- git status --short ---"
} >>"$DIFF_LOG"
git -C "$REPO_ROOT" status --short >>"$DIFF_LOG" 2>&1 || true

capture_git_status "$RECEIPT_DIR/git-status.after.txt"

if ! $RUN_CLAUDE; then
    write_runner_delivery_report \
        "部分完成" \
        "已生成任务执行收据包；未启动 Claude Code，也未自动执行 Verification gate。" \
        "- 未执行任务卡 Verification gate；需要传入 \`--claude\`。\n- 未启动 Claude Code；需要传入 \`--claude\`。" \
        "- 收据包是本地证据，不等于任务已执行完成。\n- hook 和 skill 检查只记录存在性和基础语法，不替代完整套件验证。"
fi

if $UPDATE_MEMORY; then
    MEMORY_ARGS=(capture "$RECEIPT_DIR" --repo "$REPO_ROOT")
    if [[ -n "$MEMORY_ROOT" ]]; then
        MEMORY_ARGS+=(--memory-root "$MEMORY_ROOT")
    fi
    "$SCRIPT_DIR/context-memory.sh" "${MEMORY_ARGS[@]}"
fi

echo "Receipt created: $RECEIPT_DIR"
if $RUN_CLAUDE; then
    echo "Runner exit status: $CLAUDE_EXIT_STATUS"
    exit "$CLAUDE_EXIT_STATUS"
fi
