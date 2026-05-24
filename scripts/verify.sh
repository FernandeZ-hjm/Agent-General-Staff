#!/usr/bin/env bash
set -euo pipefail

# verify.sh - Check integrity of the Agent Governance Suite

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$SUITE_ROOT/manifests/suite.yaml"

ERRORS=0
WARNINGS=0

red() { echo -e "\033[31m$*\033[0m"; }
green() { echo -e "\033[32m$*\033[0m"; }
yellow() { echo -e "\033[33m$*\033[0m"; }

check() {
    local label="$1"; shift
    if "$@"; then
        green "  [OK] $label"
    else
        red "  [FAIL] $label"
        ((ERRORS++)) || true
    fi
}

warn() {
    yellow "  [WARN] $*"
    ((WARNINGS++)) || true
}

echo "=== Agent Governance Suite Verification ==="
echo "Suite root: $SUITE_ROOT"
echo ""

# 1. Manifest exists
echo "--- Manifest ---"
check "manifest/suite.yaml present" test -f "$MANIFEST"

# 2. Required rules exist
echo "--- Required Rules ---"
for rule in global-rules/SOUL.md global-rules/core.md global-rules/RTK.md; do
    check "$rule present" test -f "$SUITE_ROOT/$rule"
done

# 3. Required skills exist (must have SKILL.md)
echo "--- Required Skills ---"
required_skills=(
    auto-brainstorm auto-debug auto-verify
    claude-execution-prompt-maker claude-delivery-report
    tdd diagnose zoom-out
    caveman-commit caveman-review
    finishing-a-development-branch using-git-worktrees
    webapp-testing grill-with-docs
    improve-codebase-architecture prototype
    database-migration supply-chain-risk-auditor
    skill-creator graphify-project-map
    superpowers
)
for skill in "${required_skills[@]}"; do
    check "skill/$skill" test -f "$SUITE_ROOT/global-skills/$skill/SKILL.md"
done

# 4. Fallback templates exist and have required skeleton
echo "--- Fallback Task Cards ---"
FALLBACK_DIR="$SUITE_ROOT/templates/fallback-task-cards"
ALLOWED_TEMPLATES=("light.md" "medium.md" "heavy.md")

# Check no extra files in fallback dir
extra_count=0
for f in "$FALLBACK_DIR"/*; do
    [[ -f "$f" ]] || continue
    fname="$(basename "$f")"
    allowed=0
    for allowed_name in "${ALLOWED_TEMPLATES[@]}"; do
        [[ "$fname" == "$allowed_name" ]] && allowed=1
    done
    if [[ $allowed -eq 0 ]]; then
        warn "Unexpected file in fallback-task-cards/: $fname"
        extra_count=$((extra_count + 1))
    fi
done
[[ $extra_count -eq 0 ]] && green "  [OK] Only allowed files in fallback-task-cards/"

# Check each template has required skeleton
required_fields=("## 任务卡" "任务级别" "任务" "目标" "非目标" "验证" "交付")
for level in light medium heavy; do
    template="$FALLBACK_DIR/$level.md"
    if [[ -f "$template" ]]; then
        missing_fields=0
        for field in "${required_fields[@]}"; do
            if ! grep -Fq "$field" "$template" 2>/dev/null; then
                red "  [FAIL] template/$level: missing field '$field'"
                missing_fields=$((missing_fields + 1))
            fi
        done
        if [[ $missing_fields -eq 0 ]]; then
            green "  [OK] template/$level: all required fields present"
        else
            ERRORS=$((ERRORS + 1))
        fi
    else
        red "  [FAIL] template/$level: file not found"
        ERRORS=$((ERRORS + 1))
    fi
done

# 5. Project integration templates
echo "--- Project Integration ---"
check "AGENTS.md.template" test -f "$SUITE_ROOT/project-integration/AGENTS.md.template"
check "CLAUDE.md.template" test -f "$SUITE_ROOT/project-integration/CLAUDE.md.template"

# 6. Protocol files
echo "--- Protocol Files ---"
check "AGENT_SUITE_PROTOCOL.md" test -f "$SUITE_ROOT/AGENT_SUITE_PROTOCOL.md"
for proto in agent-task-protocol.md task-routing.md task-card-template.md cursor-skill-index.md; do
    check "protocol/$proto" test -f "$SUITE_ROOT/protocol/$proto"
done

# 7. Governance docs
echo "--- Governance Docs ---"
check "governance/sync-protocol.md" test -f "$SUITE_ROOT/governance/sync-protocol.md"
check "governance/rollback.md" test -f "$SUITE_ROOT/governance/rollback.md"
check "governance/agent-toolchain-sync-governance.md" test -f "$SUITE_ROOT/governance/agent-toolchain-sync-governance.md"

# 7.5 Manifest-governance script consistency
echo "--- Manifest Script Consistency ---"
# Check each governance_scripts source exists
while IFS= read -r line; do
    script_rel=$(echo "$line" | sed 's/.*source: *//' | tr -d ' ')
    [[ -z "$script_rel" ]] && continue
    check "governance script: $script_rel" test -f "$SUITE_ROOT/$script_rel"
done < <(sed -n '/governance_scripts:/,/verification:/p' "$MANIFEST" | grep 'source:' 2>/dev/null || true)

# Check each verification command references a real file (for bash -n entries)
echo "--- Manifest Verification Consistency ---"
while IFS= read -r line; do
    vscript=$(echo "$line" | sed 's/.*bash -n *//' | tr -d ' ')
    [[ -z "$vscript" ]] && continue
    check "verification target: $vscript" test -f "$SUITE_ROOT/$vscript"
done < <(grep 'bash -n scripts/' "$MANIFEST" 2>/dev/null || true)

# 7.6 Governance log integrity
echo "--- Governance Logs ---"
check "governance/skill-adoption-log.yaml" test -f "$SUITE_ROOT/governance/skill-adoption-log.yaml"
check "governance/skill-ignore-list.yaml" test -f "$SUITE_ROOT/governance/skill-ignore-list.yaml"

# Basic YAML structure check for adoption log (must have header comment)
if [[ -f "$SUITE_ROOT/governance/skill-adoption-log.yaml" ]]; then
    if grep -q "^# Skill Adoption Log" "$SUITE_ROOT/governance/skill-adoption-log.yaml" 2>/dev/null; then
        green "  [OK] skill-adoption-log.yaml header valid"
    else
        warn "skill-adoption-log.yaml header missing or malformed"
    fi
fi

# Basic YAML structure check for ignore list
if [[ -f "$SUITE_ROOT/governance/skill-ignore-list.yaml" ]]; then
    if grep -q "^# Skill Ignore List" "$SUITE_ROOT/governance/skill-ignore-list.yaml" 2>/dev/null; then
        green "  [OK] skill-ignore-list.yaml header valid"
    else
        warn "skill-ignore-list.yaml header missing or malformed"
    fi
fi

# 8. Script syntax check
echo "--- Script Syntax ---"
for script in bootstrap.sh verify.sh diff-local.sh rollback.sh govern-new-skills.sh; do
    if bash -n "$SUITE_ROOT/scripts/$script" 2>/dev/null; then
        green "  [OK] bash -n scripts/$script"
    else
        red "  [FAIL] bash -n scripts/$script"
        ((ERRORS++)) || true
    fi
done

# 9. Check for machine-bound paths in installable assets (risk detection)
echo "--- Risk Detection ---"
ASSET_DIRS=(
    "$SUITE_ROOT/global-rules"
    "$SUITE_ROOT/global-skills"
    "$SUITE_ROOT/templates"
    "$SUITE_ROOT/project-integration"
)
ASSET_SCOPE=""
for d in "${ASSET_DIRS[@]}"; do
    if [[ -d "$d" ]]; then
        ASSET_SCOPE="${ASSET_SCOPE}${ASSET_SCOPE:+ }$d"
    fi
done

# Patterns: username-bound paths
for pattern in "/Users/a92550" "/Users/hujiaming"; do
    if grep -rq "$pattern" $ASSET_SCOPE 2>/dev/null; then
        warn "Hardcoded path $pattern found in installable assets"
    fi
done

# Patterns: machine-bound volume paths
for pattern in "/Volumes/AI Project" "/Volumes/AI\\ Project"; do
    if grep -rq "$pattern" $ASSET_SCOPE 2>/dev/null; then
        warn "Hardcoded path $pattern found in installable assets"
    fi
done

# Check protocol/governance docs for hardcoded paths (allowed as examples but flag for review)
for pattern in "/Users/a92550" "/Users/hujiaming"; do
    if grep -rq "$pattern" "$SUITE_ROOT/protocol/" "$SUITE_ROOT/governance/" 2>/dev/null; then
        echo "  [INFO] $pattern referenced in protocol/governance docs (allowed as example)"
    fi
done

# 10. Validate @../ cross-references in skill files resolve to existing targets
echo "--- Cross-Reference Validation ---"
ref_ok=0
ref_missing=0
while IFS= read -r -d '' md_file; do
    skill_label="$(basename "$(dirname "$md_file")")/$(basename "$md_file")"
    file_dir="$(dirname "$md_file")"
    while IFS= read -r ref_line; do
        # Extract all @../... paths from the line (stop at space, paren, bracket, brace, or backtick)
        refs=$(echo "$ref_line" | grep -oE '@\.\./[^[:space:]]+' 2>/dev/null || true)
        for ref in $refs; do
            [[ -z "$ref" ]] && continue
            # Strip trailing punctuation from inline references: ) ] } `
            ref=$(echo "$ref" | sed 's/[])}`]$//')
            # Skip prose placeholder patterns like @../superpowers/playbooks/...
            [[ "$ref" == *"..."* ]] && continue
            rel="${ref#@}"
            resolved="$file_dir/$rel"
            if [[ -f "$resolved" ]]; then
                green "  [OK] $skill_label → $ref"
                ref_ok=$((ref_ok + 1))
            else
                red "  [FAIL] $skill_label → $ref (not found: $resolved)"
                ref_missing=$((ref_missing + 1))
            fi
        done
    done < <(grep -n '@\.\./' "$md_file" 2>/dev/null || true)
done < <(find "$SUITE_ROOT/global-skills" -name "*.md" -type f -not -path '*/.git/*' -print0 2>/dev/null || true)

if [[ $ref_missing -gt 0 ]]; then
    echo ""
    red "Cross-reference errors: $ref_missing missing, $ref_ok OK"
    ERRORS=$((ERRORS + ref_missing))
else
    echo "  All @../ cross-references resolved ($ref_ok OK)"
fi

# Summary
echo ""
echo "=== Verification Summary ==="
echo "Errors  : $ERRORS"
echo "Warnings: $WARNINGS"

if [[ $ERRORS -gt 0 ]]; then
    echo "Status  : FAILED"
    exit 2
else
    echo "Status  : PASSED"
    exit 0
fi
