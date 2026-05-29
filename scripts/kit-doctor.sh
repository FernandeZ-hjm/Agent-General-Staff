#!/usr/bin/env bash
set -euo pipefail

# kit-doctor.sh - Public environment checker and update gate.
# Default mode is read-only and never pulls or edits files.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

ACTION="doctor"
PROFILE="full"
TARGET_HOME="${TARGET_HOME:-$HOME}"
TARGET_PROJECT=""
REMOTE="origin"
BRANCH=""
UPDATE_MODE="check"

usage() {
    cat <<EOF
Usage: scripts/kit-doctor.sh [doctor|update] [options]

Environment checker and public update gate for the Dongmenlaohu kit.

Commands:
  doctor                 Read-only suite, runtime, and optional project check
  update                 Check, diff, or apply public kit updates

Doctor options:
  --profile diy|full     Expected install profile (default: full)
  --target-home PATH     Runtime home to inspect (default: \$HOME)
  --target-project PATH  Optional project root to inspect

Update options:
  --check                Check whether remote updates exist (default)
  --diff                 Fetch remote branch into FETCH_HEAD and show summary diff
  --apply                Fast-forward pull, then run verify and security doctor
  --remote NAME          Git remote to inspect (default: origin)
  --branch NAME          Branch to inspect (default: current branch, then main)

General:
  --help, -h             Show this help
EOF
}

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

section() {
    echo ""
    echo "=== $1 ==="
}

ok() { echo "  [OK] $*"; }
warn() { echo "  [WARN] $*"; }
info() { echo "  [INFO] $*"; }

if [[ $# -gt 0 ]]; then
    case "$1" in
        doctor|update)
            ACTION="$1"
            shift
            ;;
    esac
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile)
            PROFILE="${2:-}"
            shift 2
            ;;
        --target-home)
            TARGET_HOME="${2:-}"
            shift 2
            ;;
        --target-project)
            TARGET_PROJECT="${2:-}"
            shift 2
            ;;
        --check)
            UPDATE_MODE="check"
            shift
            ;;
        --diff)
            UPDATE_MODE="diff"
            shift
            ;;
        --apply)
            UPDATE_MODE="apply"
            shift
            ;;
        --remote)
            REMOTE="${2:-}"
            shift 2
            ;;
        --branch)
            BRANCH="${2:-}"
            shift 2
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

[[ "$PROFILE" == "diy" || "$PROFILE" == "full" ]] || die "--profile must be diy or full"
[[ "$ACTION" == "doctor" || "$ACTION" == "update" ]] || die "Command must be doctor or update"

inspect_target_project() {
    local project_path="$1"

    section "Target Project"
    if [[ -z "$project_path" ]]; then
        info "No target project supplied; skipped"
        return 0
    fi
    if [[ ! -d "$project_path" ]]; then
        warn "Target project not found: $project_path"
        return 0
    fi

    project_path="$(cd "$project_path" && pwd -P)"
    echo "  Path: $project_path"

    if git -C "$project_path" rev-parse --show-toplevel >/dev/null 2>&1; then
        if [[ -n "$(git -C "$project_path" status --porcelain)" ]]; then
            warn "Project git worktree has local changes; review before applying installer updates"
        else
            ok "Project git worktree clean"
        fi
    else
        info "Project is not a git worktree"
    fi

    for rel in AGENTS.md CLAUDE.md docs/agent-workflow config/agent-project-profile.yaml; do
        if [[ -e "$project_path/$rel" ]]; then
            echo "  [EXISTS] $rel"
        else
            echo "  [MISSING] $rel"
        fi
    done

    if [[ -d "$project_path/docs/agent-workflow" ]]; then
        local stale=0
        for proto in agent-task-protocol.md task-routing.md runtime-adapters.md project-profile.md context-memory.md task-card-template.md cursor-skill-index.md; do
            if [[ ! -f "$project_path/docs/agent-workflow/$proto" ]]; then
                warn "workflow file missing: $proto"
                stale=$((stale + 1))
            elif ! cmp -s "$SUITE_ROOT/protocol/$proto" "$project_path/docs/agent-workflow/$proto"; then
                warn "workflow file differs from suite: $proto"
                stale=$((stale + 1))
            fi
        done
        if [[ "$stale" -eq 0 ]]; then
            ok "Project workflow docs match suite protocol"
        fi
    fi
}

resolve_branch() {
    if [[ -n "$BRANCH" ]]; then
        printf '%s' "$BRANCH"
        return 0
    fi
    local current
    current="$(git -C "$SUITE_ROOT" branch --show-current 2>/dev/null || true)"
    if [[ -n "$current" ]]; then
        printf '%s' "$current"
    else
        printf '%s' "main"
    fi
}

run_doctor() {
    echo "=== Dongmenlaohu Kit Doctor ==="
    echo "Profile       : $PROFILE"
    echo "Suite root    : $SUITE_ROOT"
    echo "Target home   : $TARGET_HOME"
    echo "Target project: ${TARGET_PROJECT:-<not supplied>}"

    section "Suite Health"
    bash "$SCRIPT_DIR/suite-doctor.sh" --target-home "$TARGET_HOME" --no-project-sync

    section "Security Boundary"
    bash "$SCRIPT_DIR/security-doctor.sh" --target-home "$TARGET_HOME"

    inspect_target_project "$TARGET_PROJECT"

    section "Suggested Next Commands"
    echo "  Preview install:"
    echo "    bash $SCRIPT_DIR/kit-install.sh --profile $PROFILE --target-project /path/to/project --project-name \"My Project\" --project-slug my-project --dry-run"
    echo "  Check public updates:"
    echo "    bash $SCRIPT_DIR/kit-doctor.sh update --check"
}

run_update() {
    local branch
    branch="$(resolve_branch)"

    [[ -n "$REMOTE" ]] || die "--remote is required"
    [[ "$UPDATE_MODE" == "check" || "$UPDATE_MODE" == "diff" || "$UPDATE_MODE" == "apply" ]] || die "Update mode must be check, diff, or apply"

    section "Update Gate"
    echo "Suite root : $SUITE_ROOT"
    echo "Remote     : $REMOTE"
    echo "Branch     : $branch"
    echo "Mode       : $UPDATE_MODE"

    if ! git -C "$SUITE_ROOT" rev-parse --show-toplevel >/dev/null 2>&1; then
        die "Suite root is not a git repository"
    fi
    if ! git -C "$SUITE_ROOT" remote get-url "$REMOTE" >/dev/null 2>&1; then
        die "Remote not found: $REMOTE"
    fi

    if [[ "$UPDATE_MODE" == "check" ]]; then
        git -C "$SUITE_ROOT" fetch --dry-run "$REMOTE" "$branch"
        local local_head
        local remote_head
        local_head="$(git -C "$SUITE_ROOT" rev-parse HEAD)"
        remote_head="$(git -C "$SUITE_ROOT" ls-remote "$REMOTE" "refs/heads/$branch" | awk '{print $1}')"
        [[ -n "$remote_head" ]] || die "Remote branch not found: $REMOTE/$branch"
        echo "  Local HEAD : ${local_head:0:12}"
        echo "  Remote HEAD: ${remote_head:0:12}"
        if [[ "$local_head" == "$remote_head" ]]; then
            ok "Local suite matches remote branch"
        else
            warn "Remote update is available"
            echo "  Review diff:"
            echo "    bash $SCRIPT_DIR/kit-doctor.sh update --diff --remote $REMOTE --branch $branch"
        fi
        return 0
    fi

    git -C "$SUITE_ROOT" fetch "$REMOTE" "$branch"

    if [[ "$UPDATE_MODE" == "diff" ]]; then
        section "Remote Diff Summary"
        git -C "$SUITE_ROOT" log --oneline --left-right --cherry-pick HEAD...FETCH_HEAD || true
        echo ""
        git -C "$SUITE_ROOT" diff --stat HEAD..FETCH_HEAD || true
        echo ""
        echo "No files were changed. To apply a fast-forward update:"
        echo "  bash $SCRIPT_DIR/kit-doctor.sh update --apply --remote $REMOTE --branch $branch"
        return 0
    fi

    if [[ -n "$(git -C "$SUITE_ROOT" status --porcelain)" ]]; then
        die "Suite worktree has local changes; commit or stash before update --apply"
    fi

    git -C "$SUITE_ROOT" merge --ff-only FETCH_HEAD
    section "Post-Update Verification"
    bash "$SCRIPT_DIR/verify.sh"
    bash "$SCRIPT_DIR/security-doctor.sh" --target-home "$TARGET_HOME"
}

case "$ACTION" in
    doctor)
        run_doctor
        ;;
    update)
        run_update
        ;;
esac
