#!/usr/bin/env bash
set -euo pipefail

# security-doctor.sh - Read-only security scan for suite surfaces.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_HOME="${TARGET_HOME:-$HOME}"

usage() {
    cat <<EOF
Usage: scripts/security-doctor.sh [--target-home PATH]

Read-only security report for hooks, scripts, task cards, rules, MCP-like
configuration, dangerous command patterns, and secret-looking values.

Options:
  --target-home PATH    Inspect runtime state under this home (default: \$HOME)
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

echo "=== Agent Governance Security Doctor ==="
echo "Suite root : $SUITE_ROOT"
echo "Target home: $TARGET_HOME"

section "Obsolete Review Hooks"
obsolete_found=0
for hook in leveled-review-gate.mjs review-baseline-snapshot.mjs codex-stop-review-adapter.mjs; do
    if [[ -e "$SUITE_ROOT/global-hooks/claude/$hook" ]]; then
        warn "obsolete suite hook still present: global-hooks/claude/$hook"
        obsolete_found=$((obsolete_found + 1))
    fi
    if [[ -e "$TARGET_HOME/.claude/hooks/$hook" ]]; then
        warn "obsolete local hook installed: $TARGET_HOME/.claude/hooks/$hook"
        obsolete_found=$((obsolete_found + 1))
    fi
done
[[ "$obsolete_found" -eq 0 ]] && ok "obsolete review hook files absent"

section "Runtime Hook Config"
for config_file in "$TARGET_HOME/.claude/settings.json" "$TARGET_HOME/.codex/hooks.json"; do
    if [[ ! -f "$config_file" ]]; then
        info "config missing: $config_file"
        continue
    fi
    ok "scanning $config_file"
    for pattern in \
        "leveled-review-gate" \
        "review-baseline-snapshot" \
        "codex-stop-review-adapter" \
        "curl | bash" \
        "rm -rf" \
        "git push --force"; do
        if grep -Fq "$pattern" "$config_file" 2>/dev/null; then
            warn "pattern in $config_file: $pattern"
        fi
    done
done

section "Dangerous Command Patterns"
danger_patterns=(
    "rm -rf \\$HOME/.agents/skills"
    "cp -rf .*\\.agents/skills"
    "curl .*\\| *bash"
    "git push --force"
    "lark-cli update"
    "npx skills (add|remove|update)"
)
danger_hits=0
for pattern in "${danger_patterns[@]}"; do
    if rg -n --pcre2 "$pattern" "$SUITE_ROOT/scripts" "$SUITE_ROOT/global-skills" "$SUITE_ROOT/protocol" "$SUITE_ROOT/templates" "$SUITE_ROOT/project-integration" \
        | grep -v "/scripts/security-doctor.sh:" \
        | grep -v '/scripts/bootstrap.sh:.*echo "  -' \
        | grep -v "不得" \
        >/tmp/ags-security-hits.$$ 2>/dev/null; then
        warn "potential dangerous command pattern: $pattern"
        head -10 /tmp/ags-security-hits.$$ | sed 's/^/    /'
        danger_hits=$((danger_hits + 1))
    fi
done
rm -f /tmp/ags-security-hits.$$
[[ "$danger_hits" -eq 0 ]] && ok "no dangerous command patterns found outside manifest docs"

section "Secret-Looking Values"
secret_patterns=(
    "sk-[A-Za-z0-9_-]{20,}"
    "xox[baprs]-[A-Za-z0-9-]{20,}"
    "AIza[0-9A-Za-z_-]{20,}"
    "BEGIN (RSA|OPENSSH|EC|DSA) PRIVATE KEY"
)
secret_hits=0
for pattern in "${secret_patterns[@]}"; do
    if rg -n --pcre2 "$pattern" "$SUITE_ROOT" --glob '!.git' --glob '!proposals/skill-adoption/*.md' \
        | grep -v "/scripts/security-doctor.sh:" \
        | grep -v "/scripts/govern-new-skills.sh:" \
        >/tmp/ags-secret-hits.$$ 2>/dev/null; then
        warn "secret-looking pattern found: $pattern"
        head -10 /tmp/ags-secret-hits.$$ | sed 's/^/    /'
        secret_hits=$((secret_hits + 1))
    fi
done
rm -f /tmp/ags-secret-hits.$$
[[ "$secret_hits" -eq 0 ]] && ok "no obvious secret-looking values found"

section "Public Boundary Hints"
if rg -n "/Users/hujiaming|/Users/a92550|/Volumes/AI Project|agent-governance-suite-private|name:.*private|description:.*private" "$SUITE_ROOT" --glob '!.git' \
    | grep -v "/scripts/security-doctor.sh:" \
    | grep -v "/scripts/verify.sh:" \
    >/tmp/ags-boundary-hits.$$ 2>/dev/null; then
    info "private/local boundary references exist; review before public release"
    head -20 /tmp/ags-boundary-hits.$$ | sed 's/^/    /'
else
    ok "no obvious local/private boundary references found"
fi
rm -f /tmp/ags-boundary-hits.$$

section "Memory Store"
memory_root="$TARGET_HOME/.agents/memory"
if [[ -d "$memory_root" ]]; then
    ok "memory root exists: $memory_root"
    if find "$memory_root" -maxdepth 4 -type f 2>/dev/null | head -1 | grep -q .; then
        info "memory files present; ensure they are never published"
    fi
else
    info "memory root not present yet: $memory_root"
fi

echo ""
echo "=== Security Doctor Complete ==="
echo "Read-only report only. No files were changed."
