#!/usr/bin/env bash
set -euo pipefail

# bootstrap.sh - Agent Governance Suite installer
# Default: dry-run. Use --apply to write files.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$SUITE_ROOT/manifests/suite.yaml"

DRY_RUN=true
# Respect env var if set, otherwise default to $HOME
TARGET_HOME="${TARGET_HOME:-$HOME}"
OBSOLETE_SKILLS=(
    graphify-project-map
    claude-execution-prompt-maker
)
OBSOLETE_HOOKS=(
    leveled-review-gate.mjs
    review-baseline-snapshot.mjs
    codex-stop-review-adapter.mjs
)

usage() {
    cat <<EOF
Usage: bootstrap.sh [--dry-run] [--apply] [--target-home PATH]

Options:
  --dry-run       Preview installation without writing files (default)
  --apply         Actually write files to target paths (requires prior --dry-run review)
  --target-home   Override target home directory (default: \$HOME)

Environment:
  TARGET_HOME     Override target home directory (CLI --target-home takes precedence)

Exit codes:
  0  Success (dry-run OK, or apply succeeded)
  1  Manifest not found
  2  Verification failed (see verify.sh)
EOF
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=true ;;
        --apply) DRY_RUN=false ;;
        --target-home) TARGET_HOME="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown option: $1"; usage; exit 1 ;;
    esac
    shift
done

BACKUP_DIR="$TARGET_HOME/.agents/backups/suite-backup-$(date +%Y%m%d-%H%M%S)"

# Check manifest
if [[ ! -f "$MANIFEST" ]]; then
    echo "[ERROR] Manifest not found: $MANIFEST"
    exit 1
fi

echo "=== Agent Governance Suite Bootstrap ==="
echo "Suite root : $SUITE_ROOT"
echo "Target home: $TARGET_HOME"
echo "Mode       : $( $DRY_RUN && echo 'DRY-RUN (no files will be written)' || echo 'APPLY (files will be written)' )"
echo "Backup dir : $BACKUP_DIR"
echo ""

# Extract source lines from manifest (section-aware parsing)
extract_sources() {
    local section="$1"
    local in_section=0
    while IFS= read -r line; do
        if [[ "$line" =~ ^[[:space:]]*${section}: ]]; then
            in_section=1
            continue
        fi
        if [[ $in_section -eq 1 ]]; then
            # Stop at next top-level or sibling key
            if [[ "$line" =~ ^[[:space:]]{0,2}[a-zA-Z] ]]; then
                break
            fi
            if [[ "$line" =~ ^[[:space:]]*-[[:space:]]*source:[[:space:]]*(.*) ]]; then
                echo "${BASH_REMATCH[1]}"
            fi
        fi
    done < "$MANIFEST"
}

# --- Check Required Rules ---
echo "--- Required Rules ---"
rules_ok=0
rules_missing=0
while IFS= read -r src; do
    [[ -z "$src" ]] && continue
    src_path="$SUITE_ROOT/$src"
    if [[ -f "$src_path" ]]; then
        echo "  [OK] $src"
        ((rules_ok++)) || true
    else
        echo "  [MISSING] $src"
        ((rules_missing++)) || true
    fi
done < <(extract_sources "required_rules")

# --- Check Required Hooks ---
echo "--- Required Hooks ---"
hooks_ok=0
hooks_missing=0
while IFS= read -r src; do
    [[ -z "$src" ]] && continue
    src_path="$SUITE_ROOT/$src"
    if [[ -f "$src_path" ]]; then
        echo "  [OK] $src"
        ((hooks_ok++)) || true
    else
        echo "  [MISSING] $src"
        ((hooks_missing++)) || true
    fi
done < <(extract_sources "required_hooks")

# --- Check Required Skills ---
echo "--- Required Skills ---"
skills_ok=0
skills_missing=0
while IFS= read -r src; do
    [[ -z "$src" ]] && continue
    src_path="$SUITE_ROOT/$src"
    if [[ -d "$src_path" ]] && [[ -f "$src_path/SKILL.md" ]]; then
        echo "  [OK] $src"
        ((skills_ok++)) || true
    else
        echo "  [MISSING] $src (directory or SKILL.md not found)"
        ((skills_missing++)) || true
    fi
done < <(extract_sources "required_skills")

echo ""
echo "Rules  : $rules_ok OK, $rules_missing missing"
echo "Hooks  : $hooks_ok OK, $hooks_missing missing"
echo "Skills : $skills_ok OK, $skills_missing missing"

if [[ $rules_missing -gt 0 || $hooks_missing -gt 0 || $skills_missing -gt 0 ]]; then
    echo "[WARN] Some assets are missing. Installation may be incomplete."
fi

# --- Preview target paths ---
echo ""
echo "--- Target Path Preview ---"
echo "Rules will be installed to:"
echo "  $TARGET_HOME/.agents/rules/SOUL.md"
echo "  $TARGET_HOME/.agents/rules/core.md"
echo "  $TARGET_HOME/.codex/RTK.md"
echo ""
echo "Skills will be installed to:"
for skill_dir in "$SUITE_ROOT/global-skills/"*; do
    skill_name="$(basename "$skill_dir")"
    echo "  $TARGET_HOME/.agents/skills/$skill_name"
done
echo ""
echo "Obsolete skill paths will be removed in --apply after backup if present:"
for skill_name in "${OBSOLETE_SKILLS[@]}"; do
    echo "  $TARGET_HOME/.agents/skills/$skill_name"
done
echo ""
echo "Hooks will be installed to:"
hook_install_count=0
while IFS= read -r src; do
    [[ -z "$src" ]] && continue
    echo "  $TARGET_HOME/.claude/hooks/$(basename "$src")"
    hook_install_count=$((hook_install_count + 1))
done < <(extract_sources "required_hooks")
if [[ $hook_install_count -eq 0 ]]; then
    echo "  (none; runtime hooks are configured directly in Claude/Codex JSON)"
fi
echo ""
echo "Obsolete review hooks will be removed in --apply after backup if present:"
for hook_name in "${OBSOLETE_HOOKS[@]}"; do
    echo "  $TARGET_HOME/.claude/hooks/$hook_name"
done
echo ""
echo "Project templates are reference-only (not auto-installed):"
echo "  $SUITE_ROOT/project-integration/"
echo ""
echo "Runtime hook config will be normalized for:"
echo "  $TARGET_HOME/.claude/settings.json"
echo "  $TARGET_HOME/.codex/hooks.json"
echo ""
echo "Runtime support scripts will be installed to:"
echo "  $TARGET_HOME/.claude/sync-skill-aliases.py"
echo "  $TARGET_HOME/.agents/scripts/memory-start-context.sh"

# --- Forbidden commands notice ---
echo "--- Forbidden Commands ---"
echo "The following commands are FORBIDDEN in any automation:"
echo "  - rm -rf \$HOME/.agents/skills/*"
echo "  - cp -rf ... \$HOME/.agents/skills/ (without dry-run)"
echo "  - lark-cli update"
echo "  - npx skills add/remove/update"
echo "  - git push --force"
echo "  - curl | bash (piping installers)"

# --- Apply logic ---
if $DRY_RUN; then
    echo ""
    echo "--- Hook Config Preview ---"
    node "$SCRIPT_DIR/configure-review-hooks.mjs" --target-home "$TARGET_HOME" --dry-run
    echo ""
    echo "=== DRY-RUN COMPLETE ==="
    echo "No files were written. Review the output above."
    echo "To apply, run: $0 --apply"
    exit 0
fi

# --apply mode
echo ""
echo "=== APPLY MODE ==="
echo "Creating backup directory: $BACKUP_DIR"
mkdir -p "$BACKUP_DIR/files/.agents/rules"
mkdir -p "$BACKUP_DIR/files/.codex"
mkdir -p "$BACKUP_DIR/files/.agents/skills"
mkdir -p "$BACKUP_DIR/files/.claude/hooks"

# Backup a single file to backup dir (preserving relative path under $TARGET_HOME)
backup_file() {
    local target="$1"
    if [[ -f "$target" ]]; then
        local rel_path="${target#$TARGET_HOME/}"
        local backup_path="$BACKUP_DIR/files/$rel_path"
        mkdir -p "$(dirname "$backup_path")"
        cp "$target" "$backup_path"
        echo "  [BACKUP] $rel_path"
    fi
}

# Backup an entire directory to backup dir
backup_dir() {
    local target="$1"
    if [[ -d "$target" ]]; then
        local rel_path="${target#$TARGET_HOME/}"
        local backup_path="$BACKUP_DIR/files/$rel_path"
        # Remove any pre-existing backup at this path to avoid nested merges
        rm -rf "$backup_path"
        mkdir -p "$(dirname "$backup_path")"
        cp -R "$target" "$backup_path"
        echo "  [BACKUP_DIR] $rel_path"
    fi
}

# Install rules
echo "Installing rules..."
mkdir -p "$TARGET_HOME/.agents/rules"
mkdir -p "$TARGET_HOME/.codex"

for pair in "global-rules/SOUL.md:.agents/rules/SOUL.md" \
            "global-rules/core.md:.agents/rules/core.md" \
            "global-rules/RTK.md:.codex/RTK.md"; do
    src="${pair%%:*}"
    dst="${pair##*:}"
    src_path="$SUITE_ROOT/$src"
    dst_path="$TARGET_HOME/$dst"
    if [[ -f "$src_path" ]]; then
        backup_file "$dst_path"
        cp "$src_path" "$dst_path"
        echo "  [INSTALL] $dst"
    fi
done

# Install hooks
echo "Installing hooks..."
mkdir -p "$TARGET_HOME/.claude/hooks"
while IFS= read -r src; do
    [[ -z "$src" ]] && continue
    src_path="$SUITE_ROOT/$src"
    dst_path="$TARGET_HOME/.claude/hooks/$(basename "$src")"
    if [[ -f "$src_path" ]]; then
        backup_file "$dst_path"
        cp "$src_path" "$dst_path"
        chmod +x "$dst_path" 2>/dev/null || true
        echo "  [INSTALL] .claude/hooks/$(basename "$src")"
    fi
done < <(extract_sources "required_hooks")

# Remove obsolete review hooks
echo "Removing obsolete review hooks..."
for hook_name in "${OBSOLETE_HOOKS[@]}"; do
    dst_path="$TARGET_HOME/.claude/hooks/$hook_name"
    if [[ -f "$dst_path" ]]; then
        backup_file "$dst_path"
        rm -f "$dst_path"
        echo "  [REMOVE_OBSOLETE] .claude/hooks/$hook_name"
    fi
done

# Configure hook entries in Claude/Codex JSON configs
echo "Normalizing hook entries..."
mkdir -p "$TARGET_HOME/.claude"
mkdir -p "$TARGET_HOME/.agents/scripts"
if [[ -f "$SCRIPT_DIR/sync-skill-aliases.py" ]]; then
    backup_file "$TARGET_HOME/.claude/sync-skill-aliases.py"
    cp "$SCRIPT_DIR/sync-skill-aliases.py" "$TARGET_HOME/.claude/sync-skill-aliases.py"
    chmod +x "$TARGET_HOME/.claude/sync-skill-aliases.py" 2>/dev/null || true
    echo "  [INSTALL] .claude/sync-skill-aliases.py"
fi
if [[ -f "$SCRIPT_DIR/memory-start-context.sh" ]]; then
    backup_file "$TARGET_HOME/.agents/scripts/memory-start-context.sh"
    cp "$SCRIPT_DIR/memory-start-context.sh" "$TARGET_HOME/.agents/scripts/memory-start-context.sh"
    chmod +x "$TARGET_HOME/.agents/scripts/memory-start-context.sh" 2>/dev/null || true
    echo "  [INSTALL] .agents/scripts/memory-start-context.sh"
fi
node "$SCRIPT_DIR/configure-review-hooks.mjs" --target-home "$TARGET_HOME" --apply

# Install skills
echo "Installing skills..."
mkdir -p "$TARGET_HOME/.agents/skills"

for skill_name in "${OBSOLETE_SKILLS[@]}"; do
    dst_path="$TARGET_HOME/.agents/skills/$skill_name"
    if [[ -d "$dst_path" ]]; then
        backup_dir "$dst_path"
        rm -rf "$dst_path"
        echo "  [REMOVE_OBSOLETE] .agents/skills/$skill_name"
    fi
done

for skill_dir in "$SUITE_ROOT/global-skills/"*; do
    skill_name="$(basename "$skill_dir")"
    dst_path="$TARGET_HOME/.agents/skills/$skill_name"
    if [[ -d "$skill_dir" ]] && [[ -f "$skill_dir/SKILL.md" ]]; then
        # Backup existing skill dir if present
        if [[ -d "$dst_path" ]]; then
            backup_dir "$dst_path"
            # Remove the old directory to prevent cp -R from nesting source inside dst
            rm -rf "$dst_path"
        fi
        # Fresh copy: suite skill -> target path
        cp -R "$skill_dir" "$dst_path"
        echo "  [INSTALL] .agents/skills/$skill_name"
    fi
done

# Save manifest snapshot to backup
cp "$MANIFEST" "$BACKUP_DIR/manifest.yaml"

# Generate changed-paths list (files + directories)
{
    echo "Changed files and directories:"
    find "$BACKUP_DIR/files" -type f | sort
} > "$BACKUP_DIR/changed-paths.txt"

# Also write legacy changed-files.txt for backwards compat
find "$BACKUP_DIR/files" -type f | sort > "$BACKUP_DIR/changed-files.txt"

echo ""
echo "=== INSTALL COMPLETE ==="
echo "Backup: $BACKUP_DIR"
echo ""
echo "Next steps:"
echo "  bash $SCRIPT_DIR/verify.sh"
echo "  bash $SCRIPT_DIR/diff-local.sh"
echo ""
echo "To rollback:"
echo "  bash $SCRIPT_DIR/rollback.sh --restore $BACKUP_DIR"
