#!/usr/bin/env bash
set -euo pipefail

# sync-project-workflow.sh - Keep registered project workflow docs aligned.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
REGISTRY="$SUITE_ROOT/governance/project-sync-registry.yaml"
MODE="check"
CLEAN_ONLY=0
PROJECT_FILTER=""

ERRORS=0
WARNINGS=0
UPDATED=0
CHECKED=0

WORKFLOW_FILES=(
    "agent-task-protocol.md"
    "task-routing.md"
    "runtime-adapters.md"
    "project-profile.md"
    "context-memory.md"
    "task-card-template.md"
    "cursor-skill-index.md"
)

usage() {
    cat <<EOF
Usage: scripts/sync-project-workflow.sh [--check] [--apply] [--clean-only] [--project NAME_OR_PATH]

Checks registered projects that use docs/agent-workflow and compares their
workflow contract files with this suite's canonical protocol/ files.

Options:
  --check              Report stale project workflow files. This is the default.
  --apply              Update stale workflow files when project policy allows it.
  --clean-only         With --apply, update only git-clean project worktrees.
  --project VALUE      Limit to one registry entry by name or exact path.
  --registry PATH      Use an alternate project registry.
  --help              Show this help.

Policies:
  auto-apply-when-clean  Apply only when the project git worktree is clean.
  report-when-dirty      Report stale files while dirty; apply once clean.
  report-only            Never apply automatically; report drift only.
EOF
}

green() { echo -e "\033[32m$*\033[0m"; }
yellow() { echo -e "\033[33m$*\033[0m"; }
red() { echo -e "\033[31m$*\033[0m"; }

warn() {
    yellow "  [WARN] $*"
    WARNINGS=$((WARNINGS + 1))
}

fail() {
    red "  [FAIL] $*"
    ERRORS=$((ERRORS + 1))
}

trim_value() {
    local value="$1"
    value="${value%%#*}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    value="${value%\"}"
    value="${value#\"}"
    value="${value%\'}"
    value="${value#\'}"
    printf '%s' "$value"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --check)
            MODE="check"
            shift
            ;;
        --apply)
            MODE="apply"
            shift
            ;;
        --clean-only)
            CLEAN_ONLY=1
            shift
            ;;
        --project)
            PROJECT_FILTER="${2:-}"
            [[ -n "$PROJECT_FILTER" ]] || { echo "[ERROR] --project requires a value" >&2; exit 2; }
            shift 2
            ;;
        --registry)
            REGISTRY="${2:-}"
            [[ -n "$REGISTRY" ]] || { echo "[ERROR] --registry requires a value" >&2; exit 2; }
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "[ERROR] Unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[[ -f "$REGISTRY" ]] || { echo "[ERROR] Registry not found: $REGISTRY" >&2; exit 2; }

is_git_dirty() {
    local project_path="$1"
    if git -C "$project_path" rev-parse --show-toplevel >/dev/null 2>&1; then
        [[ -n "$(git -C "$project_path" status --porcelain)" ]]
    else
        return 1
    fi
}

project_has_git() {
    git -C "$1" rev-parse --show-toplevel >/dev/null 2>&1
}

copy_workflow_files() {
    local project_path="$1"
    local dest_dir="$project_path/docs/agent-workflow"

    mkdir -p "$dest_dir"
    for file in "${WORKFLOW_FILES[@]}"; do
        cp "$SUITE_ROOT/protocol/$file" "$dest_dir/$file"
    done
}

collect_stale_files() {
    local project_path="$1"
    local stale=()
    local dest_dir="$project_path/docs/agent-workflow"

    for file in "${WORKFLOW_FILES[@]}"; do
        if [[ ! -f "$dest_dir/$file" ]]; then
            stale+=("$file:missing")
        elif ! cmp -s "$SUITE_ROOT/protocol/$file" "$dest_dir/$file"; then
            stale+=("$file:content-diff")
        fi
    done

    if [[ ${#stale[@]} -gt 0 ]]; then
        printf '%s\n' "${stale[@]}"
    fi
}

has_legacy_markers() {
    local project_path="$1"
    local task_card="$project_path/docs/agent-workflow/task-card-template.md"

    [[ -f "$task_card" ]] || return 1
    grep -Fq "执行者：Claude Code" "$task_card" && return 0
    ! grep -Fq "Runtime adapter:" "$task_card" && return 0
    ! grep -Fq "Execution surface:" "$task_card" && return 0
    ! grep -Fq "Permission mode:" "$task_card" && return 0
    ! grep -Fq "Parallelism:" "$task_card" && return 0
    ! grep -Fq "Verification gate:" "$task_card" && return 0
    return 1
}

process_project() {
    local name="$1"
    local project_path="$2"
    local policy="$3"

    if [[ -n "$PROJECT_FILTER" && "$PROJECT_FILTER" != "$name" && "$PROJECT_FILTER" != "$project_path" ]]; then
        return 0
    fi

    CHECKED=$((CHECKED + 1))
    echo ""
    echo "== $name =="
    echo "Path  : $project_path"
    echo "Policy: $policy"

    if [[ ! -d "$project_path" ]]; then
        warn "Project path not found; skipping"
        return 0
    fi

    local git_dirty=0
    if project_has_git "$project_path" && is_git_dirty "$project_path"; then
        git_dirty=1
        warn "Project git worktree is dirty"
    fi

    stale=()
    while IFS= read -r item; do
        [[ -n "$item" ]] && stale+=("$item")
    done < <(collect_stale_files "$project_path")
    local legacy=0
    if has_legacy_markers "$project_path"; then
        legacy=1
    fi

    if [[ ${#stale[@]} -eq 0 && "$legacy" -eq 0 ]]; then
        green "  [OK] workflow docs match suite protocol"
        return 0
    fi

    echo "  Stale workflow files:"
    if [[ ${#stale[@]} -gt 0 ]]; then
        printf '    - %s\n' "${stale[@]}"
    fi
    if [[ "$legacy" -eq 1 ]]; then
        echo "    - task-card-template.md:legacy-markers"
    fi

    if [[ "$MODE" == "check" ]]; then
        case "$policy" in
            report-only)
                warn "Workflow drift is report-only for this project"
                ;;
            report-when-dirty)
                if [[ "$git_dirty" -eq 1 ]]; then
                    warn "Workflow drift will be applied after the project is clean"
                else
                    fail "Workflow drift must be applied"
                fi
                ;;
            auto-apply-when-clean)
                if [[ "$git_dirty" -eq 1 ]]; then
                    warn "Workflow drift found but project is dirty; apply when clean"
                else
                    fail "Workflow drift must be applied"
                fi
                ;;
            *)
                fail "Unknown project sync policy: $policy"
                ;;
        esac
        return 0
    fi

    case "$policy" in
        report-only)
            warn "Skipping apply because policy is report-only"
            return 0
            ;;
        report-when-dirty|auto-apply-when-clean)
            if [[ "$git_dirty" -eq 1 || "$CLEAN_ONLY" -eq 1 && "$git_dirty" -eq 1 ]]; then
                warn "Skipping apply because project worktree is dirty"
                return 0
            fi
            copy_workflow_files "$project_path"
            UPDATED=$((UPDATED + 1))
            green "  [OK] workflow docs updated"
            ;;
        *)
            fail "Unknown project sync policy: $policy"
            ;;
    esac
}

echo "=== Project Workflow Sync ==="
echo "Suite   : $SUITE_ROOT"
echo "Registry: $REGISTRY"
echo "Mode    : $MODE"

name=""
path=""
policy=""
while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$line" =~ ^[[:space:]]*-[[:space:]]name:[[:space:]]*(.*)$ ]]; then
        if [[ -n "$name" ]]; then
            process_project "$name" "$path" "${policy:-auto-apply-when-clean}"
        fi
        name="$(trim_value "${BASH_REMATCH[1]}")"
        path=""
        policy=""
    elif [[ "$line" =~ ^[[:space:]]*path:[[:space:]]*(.*)$ ]]; then
        path="$(trim_value "${BASH_REMATCH[1]}")"
    elif [[ "$line" =~ ^[[:space:]]*policy:[[:space:]]*(.*)$ ]]; then
        policy="$(trim_value "${BASH_REMATCH[1]}")"
    fi
done < "$REGISTRY"

if [[ -n "$name" ]]; then
    process_project "$name" "$path" "${policy:-auto-apply-when-clean}"
fi

echo ""
echo "=== Project Workflow Sync Summary ==="
echo "Checked : $CHECKED"
echo "Updated : $UPDATED"
echo "Warnings: $WARNINGS"
echo "Errors  : $ERRORS"

if [[ "$CHECKED" -eq 0 ]]; then
    echo "[OK] No registered projects matched; nothing to sync."
    exit 0
fi

if [[ "$ERRORS" -gt 0 ]]; then
    exit 1
fi
