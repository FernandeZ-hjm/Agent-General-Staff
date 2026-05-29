#!/usr/bin/env bash
set -euo pipefail

# suite-doctor.sh - Read-only health report for the public suite.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_HOME="${TARGET_HOME:-$HOME}"
STABLE_SUITE="${STABLE_SUITE:-}"
RUN_PROJECT_SYNC=true

usage() {
    cat <<EOF
Usage: scripts/suite-doctor.sh [--target-home PATH] [--stable-suite PATH] [--no-project-sync]

Read-only suite health report. It does not repair, apply, pull, push, or edit.

Options:
  --target-home PATH    Inspect runtime state under this home (default: \$HOME)
  --stable-suite PATH   Compare against stable suite when present
  --no-project-sync     Skip registered project workflow drift check
  --help, -h            Show this help
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target-home)
            TARGET_HOME="${2:-}"
            [[ -n "$TARGET_HOME" ]] || { echo "[ERROR] --target-home requires a path" >&2; exit 2; }
            shift 2
            ;;
        --stable-suite)
            STABLE_SUITE="${2:-}"
            [[ -n "$STABLE_SUITE" ]] || { echo "[ERROR] --stable-suite requires a path" >&2; exit 2; }
            shift 2
            ;;
        --no-project-sync)
            RUN_PROJECT_SYNC=false
            shift
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

section() {
    echo ""
    echo "=== $1 ==="
}

ok() { echo "  [OK] $*"; }
warn() { echo "  [WARN] $*"; }
info() { echo "  [INFO] $*"; }
fail() { echo "  [FAIL] $*"; }

echo "=== Agent Governance Suite Doctor ==="
echo "Suite root : $SUITE_ROOT"
echo "Target home: $TARGET_HOME"
echo "Stable path: ${STABLE_SUITE:-<not configured>}"

section "Repository"
if git -C "$SUITE_ROOT" rev-parse --show-toplevel >/dev/null 2>&1; then
    echo "  Branch: $(git -C "$SUITE_ROOT" branch --show-current 2>/dev/null || true)"
    echo "  HEAD  : $(git -C "$SUITE_ROOT" rev-parse --short HEAD 2>/dev/null || true)"
    if [[ -n "$(git -C "$SUITE_ROOT" status --porcelain)" ]]; then
        warn "Suite worktree has local changes"
        git -C "$SUITE_ROOT" status --short | sed 's/^/    /'
    else
        ok "Suite worktree clean"
    fi
else
    fail "Suite root is not a git repository"
fi

section "Core Assets"
for path in \
    manifests/suite.yaml \
    AGENT_SUITE_PROTOCOL.md \
    protocol/task-card-template.md \
    protocol/project-profile.md \
    protocol/context-memory.md \
    scripts/kit-install.sh \
    scripts/kit-doctor.sh \
    scripts/bootstrap.sh \
    scripts/verify.sh \
    scripts/run-task-card.sh \
    scripts/context-memory.sh \
    scripts/suite-doctor.sh \
    scripts/security-doctor.sh; do
    if [[ -f "$SUITE_ROOT/$path" ]]; then
        ok "$path"
    else
        fail "$path missing"
    fi
done

section "Script Syntax"
syntax_failed=0
while IFS= read -r script; do
    [[ -n "$script" ]] || continue
    if bash -n "$script" 2>/dev/null; then
        ok "bash -n ${script#$SUITE_ROOT/}"
    else
        fail "bash -n ${script#$SUITE_ROOT/}"
        syntax_failed=$((syntax_failed + 1))
    fi
done < <(find "$SUITE_ROOT/scripts" -maxdepth 1 -type f -name '*.sh' | sort)
[[ "$syntax_failed" -eq 0 ]] || warn "Script syntax failures: $syntax_failed"

section "Runtime Config"
for config_file in "$TARGET_HOME/.claude/settings.json" "$TARGET_HOME/.codex/hooks.json"; do
    if [[ -f "$config_file" ]]; then
        ok "config exists: $config_file"
        if grep -Fq "sync-skill-aliases.py" "$config_file" 2>/dev/null; then
            ok "references sync-skill-aliases.py"
        else
            warn "missing sync-skill-aliases.py reference: $config_file"
        fi
    else
        warn "config missing: $config_file"
    fi
done
if [[ -f "$TARGET_HOME/.claude/settings.json" ]]; then
    if grep -Fq "rtk hook claude" "$TARGET_HOME/.claude/settings.json" 2>/dev/null; then
        ok "Claude settings reference rtk hook claude"
    else
        warn "Claude settings missing rtk hook claude"
    fi
fi

section "Local Install Drift"
if [[ -x "$SUITE_ROOT/scripts/diff-local.sh" ]]; then
    TARGET_HOME="$TARGET_HOME" bash "$SUITE_ROOT/scripts/diff-local.sh" --summary --target-home "$TARGET_HOME" | sed 's/^/  /'
else
    warn "diff-local.sh is not executable"
fi

section "Context Memory"
if [[ -x "$SUITE_ROOT/scripts/context-memory.sh" ]]; then
    bash "$SUITE_ROOT/scripts/context-memory.sh" status --repo "$SUITE_ROOT" | sed 's/^/  /'
else
    warn "context-memory.sh is not executable"
fi

section "Stable Alignment"
if [[ -n "$STABLE_SUITE" && -d "$STABLE_SUITE" ]]; then
    if diff -qr --exclude=.git "$STABLE_SUITE" "$SUITE_ROOT" >/tmp/ags-suite-doctor-diff.$$ 2>/dev/null; then
        ok "Stable and development suites are content-aligned"
    else
        warn "Stable and development suites differ"
        head -20 /tmp/ags-suite-doctor-diff.$$ | sed 's/^/    /'
    fi
    rm -f /tmp/ags-suite-doctor-diff.$$
else
    info "Stable suite path not configured or not found; skipped"
fi

section "Project Workflow Drift"
if $RUN_PROJECT_SYNC; then
    bash "$SUITE_ROOT/scripts/sync-project-workflow.sh" --check | sed 's/^/  /'
else
    info "Skipped by --no-project-sync"
fi

echo ""
echo "=== Doctor Complete ==="
echo "Read-only report only. No files were changed."
