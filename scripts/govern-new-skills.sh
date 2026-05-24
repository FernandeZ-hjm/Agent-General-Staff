#!/usr/bin/env bash
set -euo pipefail

# govern-new-skills.sh — One-command skill governance entry point.
#
# Subcommands:
#   scan                      Discover ungoverned skills, generate proposals (dry-run default).
#   adopt                     Interactively adopt one or more skills into the suite.
#   adopt --source <PATH|URL> Import from directory, git URL, or tarball.
#   ignore <name> --reason R  Add a skill to the ignore list.
#   list                      List governed, ungoverned, and ignored skills.
#
# Backwards-compatible shortcuts:
#   (no args)     → scan
#   --apply       → scan + adopt --apply
#   --source ...  → adopt --source ...

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_HOME="${TARGET_HOME:-$HOME}"
MANIFEST="$SUITE_ROOT/manifests/suite.yaml"
SKILLS_DIR="$SUITE_ROOT/global-skills"
OPTIONAL_DIR="$SUITE_ROOT/skill-packs/optional"
PERSONAL_DIR="$SUITE_ROOT/skill-packs/personal"
LOCAL_SKILLS="$TARGET_HOME/.agents/skills"
PROPOSALS_DIR="$SUITE_ROOT/proposals/skill-adoption"
ADOPTION_LOG="$SUITE_ROOT/governance/skill-adoption-log.yaml"
IGNORE_LIST="$SUITE_ROOT/governance/skill-ignore-list.yaml"

DRY_RUN=true
AUTO_YES=false
FORCE=false
SOURCE=""
SKILL_NAME=""
PROFILE=""
IGNORE_REASON=""
SUBCMD="scan"

# --- Safety scan patterns ---
DANGER_USER="a92550"
DANGER_USER2="hujiaming"
DANGER_VOL="AI Project"
DANGER_PATTERNS=(
    "/Volumes/${DANGER_VOL}"
    "/Users/${DANGER_USER2}"
    "/Users/${DANGER_USER}"
)
SECRET_PATTERNS=(
    "BEGIN OPENSSH PRIVATE KEY"
    "OPENAI_API_KEY"
    "ANTHROPIC_API_KEY"
    "FEISHU"
    "LARK"
    "\.env"
    "token"
    "secret"
    "password"
)

usage() {
    cat <<'EOF'
Usage:
  govern-new-skills.sh scan                     Discover ungoverned skills (dry-run)
  govern-new-skills.sh scan --target-home PATH   Scan a non-default home
  govern-new-skills.sh adopt                     Interactively adopt skills
  govern-new-skills.sh adopt --apply            Adopt with prompts
  govern-new-skills.sh adopt --apply --yes      Adopt all without prompts
  govern-new-skills.sh adopt --source PATH      Import from directory/git/tarball
  govern-new-skills.sh adopt --source PATH --profile PROFILE --apply
  govern-new-skills.sh ignore <name> --reason R Add to ignore list
  govern-new-skills.sh list                     List all skills (governed + ungoverned + ignored)
  govern-new-skills.sh --help                   Show this message

Backwards-compatible shortcuts:
  (no args)     → same as 'scan'
  --apply       → same as 'adopt --apply'
  --source ...  → same as 'adopt --source ...'

Profiles:
  required   → global-skills/<skill>      (suite-managed, installed by bootstrap)
  optional   → skill-packs/optional/<skill>  (curated, not auto-installed)
  personal   → skill-packs/personal/<skill>  (user customization, not auto-installed)
  ignored    → recorded in ignore list, never copied into suite
EOF
}

# ======================================================================
# Parse arguments
# ======================================================================

# Check if first arg is a known subcommand
if [[ $# -gt 0 && ! "$1" =~ ^-- ]]; then
    case "$1" in
        scan|adopt|ignore|list)
            SUBCMD="$1"
            shift
            ;;
        help|--help|-h)
            usage; exit 0
            ;;
        *)
            # Treat as legacy: positional skill name for adopt
            SKILL_NAME="$1"
            SUBCMD="adopt"
            shift
            ;;
    esac
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --source) SOURCE="$2"; shift ;;
        --name) SKILL_NAME="$2"; shift ;;
        --profile) PROFILE="$2"; shift ;;
        --reason) IGNORE_REASON="$2"; shift ;;
        --target-home) TARGET_HOME="$2"; shift ;;
        --dry-run) DRY_RUN=true ;;
        --apply) DRY_RUN=false ;;
        --force) FORCE=true ;;
        --yes|-y) AUTO_YES=true ;;
        --help|-h) usage; exit 0 ;;
        *)
            # Try as positional name for ignore subcommand
            if [[ "$SUBCMD" == "ignore" && -z "$SKILL_NAME" ]]; then
                SKILL_NAME="$1"
            else
                echo "Unknown option: $1"
                usage; exit 1
            fi
            ;;
    esac
    shift
done

# Re-resolve TARGET_HOME paths after arg parsing
LOCAL_SKILLS="$TARGET_HOME/.agents/skills"

# ======================================================================
# Helper functions
# ======================================================================

timestamp() {
    date +%Y%m%d-%H%M%S
}

is_governed() {
    local name="$1"
    [[ -d "$SKILLS_DIR/$name" ]] || [[ -d "$OPTIONAL_DIR/$name" ]] || [[ -d "$PERSONAL_DIR/$name" ]]
}

is_ignored() {
    local name="$1"
    [[ -f "$IGNORE_LIST" ]] && grep -q "skill: ${name}$" "$IGNORE_LIST" 2>/dev/null
}

skill_dest_for_profile() {
    local profile="$1"
    case "$profile" in
        required) echo "$SKILLS_DIR" ;;
        optional) echo "$OPTIONAL_DIR" ;;
        personal) echo "$PERSONAL_DIR" ;;
        *) echo "" ;;
    esac
}

suggest_profile() {
    local name="$1"
    case "$name" in
        lark-*|feishu-*)
            echo "optional" ;;
        产经破壁机-*|六爻|辐射塔罗牌|深度科技评论|设计助手|开发助手|优化助手|a)
            echo "personal" ;;
        auto-brainstorm|auto-debug|auto-verify|tdd|diagnose|zoom-out|caveman-commit|caveman-review|finishing-a-development-branch|using-git-worktrees|webapp-testing|grill-with-docs|improve-codebase-architecture|prototype|database-migration|supply-chain-risk-auditor|skill-creator|graphify-project-map|claude-execution-prompt-maker|claude-delivery-report|superpowers)
            echo "required" ;;
        *)
            echo "optional" ;;
    esac
}

safety_scan() {
    local src_dir="$1"
    local label="$2"
    local risk=0
    local findings=""

    # Check hardcoded paths
    for pattern in "${DANGER_PATTERNS[@]}"; do
        if grep -rq "$pattern" "$src_dir" 2>/dev/null; then
            [[ $risk -eq 0 ]] && echo "  [RISK] $label:"
            risk=1
            findings="${findings}  - hardcoded_path: $pattern"$'\n'
            echo "         hardcoded path: $pattern"
            grep -rn "$pattern" "$src_dir" 2>/dev/null | head -5 | while IFS= read -r line; do
                echo "           $line"
            done
        fi
    done

    # Check secrets-like patterns
    for pattern in "${SECRET_PATTERNS[@]}"; do
        if grep -rq "$pattern" "$src_dir" 2>/dev/null; then
            [[ $risk -eq 0 ]] && echo "  [RISK] $label:"
            risk=1
            findings="${findings}  - secret_like: $pattern"$'\n'
            echo "         secret-like pattern: $pattern"
        fi
    done

    if [[ $risk -eq 0 ]]; then
        echo "  [OK] $label"
    fi

    # Write findings to a temp file for caller to read
    echo "$risk" > /tmp/govern-safety-risk-$$.txt
    echo "$findings" > /tmp/govern-safety-findings-$$.txt
}

compute_hash() {
    local dir="$1"
    find "$dir" -type f -not -path '*/.git/*' -exec shasum -a 256 {} \; 2>/dev/null \
        | sort | shasum -a 256 | cut -d' ' -f1
}

write_proposal() {
    local name="$1" src_dir="$2" profile="$3"
    local dest
    dest=$(skill_dest_for_profile "$profile")/$name
    local ts
    ts=$(timestamp)
    local proposal_file="$PROPOSALS_DIR/${name}-${ts}.md"
    local file_count
    file_count=$(find "$src_dir" -type f -not -path '*/.git/*' 2>/dev/null | wc -l | tr -d ' ')
    local hash_val
    hash_val=$(compute_hash "$src_dir")
    local safety_output
    safety_output=$(cat /tmp/govern-safety-findings-$$.txt 2>/dev/null || echo "  (no findings)")

    mkdir -p "$PROPOSALS_DIR"

    cat > "$proposal_file" <<PROPOSAL
# Skill Adoption Proposal: $name

- **Timestamp**: $ts
- **Skill**: $name
- **Source**: $src_dir
- **Source type**: directory
- **Suggested profile**: $profile
- **Destination**: $dest
- **File count**: $file_count
- **Content hash (sha256)**: $hash_val

## Top-level files
PROPOSAL
    find "$src_dir" -maxdepth 1 -type f -not -name '.git' 2>/dev/null | while IFS= read -r f; do
        echo "- $(basename "$f")"
    done >> "$proposal_file"

    cat >> "$proposal_file" <<PROPOSAL

## Safety Scan Findings
$safety_output

## Manifest Change Preview
\`\`\`yaml
    - source: $(echo "$dest" | sed "s|$SUITE_ROOT/||")/$name
      dest: .agents/skills/$name
      description: Adopted skill (profile: $profile)
\`\`\`

## Recommended Next Command
\`\`\`bash
bash scripts/govern-new-skills.sh adopt --name $name --profile $profile --apply
\`\`\`
PROPOSAL

    echo "$proposal_file"
}

write_adoption_log() {
    local name="$1" profile="$2" src_dir="$3"
    local ts
    ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    local dest
    dest=$(skill_dest_for_profile "$profile")/$name
    local file_count
    file_count=$(find "$dest" -type f 2>/dev/null | wc -l | tr -d ' ')
    local hash_val
    hash_val=$(compute_hash "$dest")
    local safety_out
    safety_out=$(cat /tmp/govern-safety-findings-$$.txt 2>/dev/null | tr '\n' ' ' || echo "none")

    mkdir -p "$(dirname "$ADOPTION_LOG")"

    if [[ ! -f "$ADOPTION_LOG" ]]; then
        cat > "$ADOPTION_LOG" <<'LOGHEADER'
# Skill Adoption Log
# Auto-generated by govern-new-skills.sh — do not edit manually without care.
#
LOGHEADER
    fi

    cat >> "$ADOPTION_LOG" <<ENTRY
- timestamp: "$ts"
  skill: "$name"
  source: "$src_dir"
  source_type: directory
  profile: "$profile"
  destination: "$dest"
  file_count: $file_count
  content_hash: "$hash_val"
  safety_findings: "$safety_out"
  decision: adopted
  operator: "${USER:-unknown}"
ENTRY
}

append_to_manifest() {
    local name="$1"
    local profile="$2"

    # Only required skills go into suite.yaml required_skills
    if [[ "$profile" != "required" ]]; then
        return 0
    fi

    # Skip if already present
    if grep -q "global-skills/${name}" "$MANIFEST" 2>/dev/null; then
        return 0
    fi

    local anchor_line
    anchor_line=$(grep -n '^  project_integration:' "$MANIFEST" | head -1 | cut -d: -f1)
    if [[ -z "$anchor_line" ]]; then
        echo "  [WARN] Cannot find 'project_integration:' anchor in suite.yaml"
        return 1
    fi
    sed -i '' "${anchor_line}i\\
    - source: global-skills/${name}\\
      dest: .agents/skills/${name}\\
      description: Adopted skill (profile: required)
" "$MANIFEST"
}

copy_skill_files() {
    local src_dir="$1"
    local dest_dir="$2"

    if [[ -d "$dest_dir" ]]; then
        if $FORCE; then
            echo "  [RM] $dest_dir (force overwrite)"
            rm -rf "$dest_dir"
        else
            echo "  [SKIP] Destination already exists: $dest_dir"
            echo "         Use --force to overwrite."
            return 1
        fi
    fi

    mkdir -p "$dest_dir"
    local count=0
    while IFS= read -r -d '' file; do
        rel="${file#$src_dir/}"
        mkdir -p "$(dirname "$dest_dir/$rel")"
        cp "$file" "$dest_dir/$rel"
        count=$((count + 1))
    done < <(find "$src_dir" -type f -not -path '*/.git/*' -print0 2>/dev/null || true)

    [[ -d "$dest_dir/.git" ]] && rm -rf "$dest_dir/.git"

    # Verify SKILL.md
    if [[ ! -f "$dest_dir/SKILL.md" ]]; then
        echo "  [ERROR] SKILL.md missing after copy for $(basename "$dest_dir")"
        return 1
    fi

    echo "  [COPIED] $count files → $dest_dir"
    return 0
}

# ======================================================================
# scan
# ======================================================================
cmd_scan() {
    echo "=== Skill Governance: Scan ==="
    echo "Suite root : $SUITE_ROOT"
    echo "Local home : $TARGET_HOME"
    echo ""

    if [[ ! -d "$LOCAL_SKILLS" ]]; then
        echo "No local skills directory at $LOCAL_SKILLS"
        exit 0
    fi

    # Discover ungoverned skills
    UNGOVERNED=()
    for local_dir in "$LOCAL_SKILLS/"*; do
        [[ -d "$local_dir" ]] || continue
        local name
        name="$(basename "$local_dir")"
        [[ -f "$local_dir/SKILL.md" ]] || continue

        is_governed "$name" && continue
        is_ignored "$name" && continue
        UNGOVERNED+=("$name")
    done

    if [[ ${#UNGOVERNED[@]} -eq 0 ]]; then
        echo "All local skills are governed or ignored. Nothing to adopt."
        return 0
    fi

    echo "Found ${#UNGOVERNED[@]} ungoverned skill(s):"
    echo ""

    # Safety scan + suggest profile for each
    echo "--- Safety Scan ---"
    for name in "${UNGOVERNED[@]}"; do
        safety_scan "$LOCAL_SKILLS/$name" "$name"
    done
    echo ""

    # Show summary with suggested profiles
    echo "--- Scan Summary ---"
    for name in "${UNGOVERNED[@]}"; do
        local prof
        prof=$(suggest_profile "$name")
        local fc
        fc=$(find "$LOCAL_SKILLS/$name" -type f -not -path '*/.git/*' 2>/dev/null | wc -l | tr -d ' ')
        printf "  %-40s %3s files  suggested: %s\n" "$name" "$fc" "$prof"
    done
    echo ""

    # Generate proposals for each
    echo "--- Proposals ---"
    for name in "${UNGOVERNED[@]}"; do
        local prof
        prof=$(suggest_profile "$name")
        local prop_file
        prop_file=$(write_proposal "$name" "$LOCAL_SKILLS/$name" "$prof")
        echo "  [PROPOSAL] $prop_file"
    done
    echo ""

    echo "=== SCAN COMPLETE ==="
    echo ""
    echo "Review proposals in: $PROPOSALS_DIR"
    echo "To adopt interactively:"
    echo "  bash $0 adopt --apply"
    echo ""
    echo "To ignore specific skills:"
    echo "  bash $0 ignore <name> --reason \"why\""
}

# ======================================================================
# adopt
# ======================================================================
cmd_adopt() {
    echo "=== Skill Governance: Adopt ==="
    echo "Suite root : $SUITE_ROOT"
    echo "Mode       : $( $DRY_RUN && echo 'DRY-RUN (proposal only)' || echo 'APPLY' )"
    echo ""

    # --- Mode: --source import ---
    if [[ -n "$SOURCE" ]]; then
        adopt_from_source
        return
    fi

    # --- Mode: adopt from local scan ---
    if [[ ! -d "$LOCAL_SKILLS" ]]; then
        echo "No local skills directory at $LOCAL_SKILLS"
        exit 0
    fi

    # Collect ungoverned skills
    UNGOVERNED=()
    for local_dir in "$LOCAL_SKILLS/"*; do
        [[ -d "$local_dir" ]] || continue
        local name
        name="$(basename "$local_dir")"
        [[ -f "$local_dir/SKILL.md" ]] || continue
        is_governed "$name" && continue
        is_ignored "$name" && continue
        UNGOVERNED+=("$name")
    done

    if [[ ${#UNGOVERNED[@]} -eq 0 ]]; then
        echo "All local skills are governed or ignored. Nothing to adopt."
        exit 0
    fi

    echo "Found ${#UNGOVERNED[@]} ungoverned skill(s):"
    for name in "${UNGOVERNED[@]}"; do
        local prof
        prof=$(suggest_profile "$name")
        local fc
        fc=$(find "$LOCAL_SKILLS/$name" -type f -not -path '*/.git/*' 2>/dev/null | wc -l | tr -d ' ')
        echo "  $name ($fc files, suggested: $prof)"
    done
    echo ""

    # Safety scan
    echo "--- Safety Scan ---"
    for name in "${UNGOVERNED[@]}"; do
        safety_scan "$LOCAL_SKILLS/$name" "$name" || true
    done
    echo ""

    if $DRY_RUN; then
        # Generate proposals and exit
        echo "--- Proposals ---"
        for name in "${UNGOVERNED[@]}"; do
            local prof
            prof=$(suggest_profile "$name")
            local prop_file
            prop_file=$(write_proposal "$name" "$LOCAL_SKILLS/$name" "$prof")
            echo "  [PROPOSAL] $prop_file"
        done
        echo ""
        echo "=== DRY-RUN COMPLETE ==="
        echo "Review proposals, then run with --apply to adopt."
        exit 0
    fi

    # APPLY mode: interactive adoption
    local adopted=0 skipped=0
    for name in "${UNGOVERNED[@]}"; do
        if $AUTO_YES; then
            answer="y"
            PROFILE=$(suggest_profile "$name")
        else
            local suggested
            suggested=$(suggest_profile "$name")
            PROFILE=""
            read -r -p "Adopt '$name' into suite? Profile [required/optional/personal/skip] (suggested: $suggested): " answer
            case "$answer" in
                required|optional|personal)
                    PROFILE="$answer" ;;
                skip|n|no|N)
                    echo "  [SKIP] $name"
                    skipped=$((skipped + 1))
                    continue ;;
                *)
                    PROFILE="$suggested"
                    ;;
            esac
        fi

        # Safety re-check for apply
        safety_scan "$LOCAL_SKILLS/$name" "$name"
        local safety_risk
        safety_risk=$(cat /tmp/govern-safety-risk-$$.txt 2>/dev/null || echo "0")

        if [[ $safety_risk -ne 0 ]] && ! $AUTO_YES; then
            read -r -p "  Safety risks found. Proceed anyway? [y/N] " confirm
            [[ ! "$confirm" =~ ^[Yy]$ ]] && {
                echo "  [SKIP] $name (safety risk, user declined)"
                skipped=$((skipped + 1))
                continue
            }
        fi

        local dest
        dest=$(skill_dest_for_profile "$PROFILE")/"$name"

        # Write proposal
        write_proposal "$name" "$LOCAL_SKILLS/$name" "$PROFILE" > /dev/null

        # Copy
        copy_skill_files "$LOCAL_SKILLS/$name" "$dest" || {
            echo "  [FAIL] $name — could not copy"
            skipped=$((skipped + 1))
            continue
        }

        # Update manifest (only for required)
        append_to_manifest "$name" "$PROFILE"

        # Write adoption log
        write_adoption_log "$name" "$PROFILE" "$LOCAL_SKILLS/$name"

        echo "  [ADOPTED] $name → $dest (profile: $PROFILE)"
        adopted=$((adopted + 1))
    done

    echo ""
    echo "=== DONE ==="
    echo "Adopted: $adopted"
    echo "Skipped: $skipped"
    if [[ $adopted -gt 0 ]]; then
        echo ""
        echo "Next steps:"
        echo "  1. Review adopted files and proposals."
        echo "  2. bash $SCRIPT_DIR/verify.sh"
        echo "  3. git add agent-governance/"
        echo "  4. git commit -m 'feat(governance): adopt skill(s)'"
        echo "  5. git push"
    fi
}

# --- adopt --source handler ---
adopt_from_source() {
    # Determine source type
    local src_type
    if [[ -d "$SOURCE" ]]; then
        src_type="directory"
    elif [[ -f "$SOURCE" ]]; then
        src_type="file"
    elif [[ "$SOURCE" =~ ^git@|^https://|\.git$ ]]; then
        src_type="git"
    else
        echo "[ERROR] Unknown source type: $SOURCE"
        exit 1
    fi

    # Determine name
    if [[ -z "${SKILL_NAME:-}" ]]; then
        SKILL_NAME="$(basename "$SOURCE")"
        SKILL_NAME="${SKILL_NAME%.git}"
        SKILL_NAME="${SKILL_NAME%.tar.gz}"
        SKILL_NAME="${SKILL_NAME%.tgz}"
    fi

    # Determine profile
    if [[ -z "$PROFILE" ]]; then
        PROFILE=$(suggest_profile "$SKILL_NAME")
    fi

    echo "Source      : $SOURCE"
    echo "Source type : $src_type"
    echo "Skill name  : $SKILL_NAME"
    echo "Profile     : $PROFILE"
    echo ""

    # Resolve to temp dir if needed
    local tmp_dir=""
    local cleanup_tmp=false
    local src_dir

    case "$src_type" in
        directory)
            if [[ ! -f "$SOURCE/SKILL.md" ]]; then
                echo "[ERROR] Source directory has no SKILL.md: $SOURCE"
                exit 1
            fi
            src_dir="$SOURCE"
            ;;
        git)
            tmp_dir="$(mktemp -d)"
            cleanup_tmp=true
            echo "Cloning $SOURCE ..."
            git clone --depth 1 "$SOURCE" "$tmp_dir" 2>&1
            if [[ ! -f "$tmp_dir/SKILL.md" ]]; then
                echo "[ERROR] Cloned repo has no SKILL.md at root."
                rm -rf "$tmp_dir"
                exit 1
            fi
            src_dir="$tmp_dir"
            ;;
        file)
            tmp_dir="$(mktemp -d)"
            cleanup_tmp=true
            echo "Extracting $SOURCE ..."
            tar -xzf "$SOURCE" -C "$tmp_dir" 2>&1
            local dir_count
            dir_count=$(find "$tmp_dir" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
            if [[ $dir_count -eq 1 ]]; then
                tmp_dir="$tmp_dir/$(ls "$tmp_dir")"
            fi
            if [[ ! -f "$tmp_dir/SKILL.md" ]]; then
                echo "[ERROR] Extracted archive has no SKILL.md."
                rm -rf "$tmp_dir"
                exit 1
            fi
            src_dir="$tmp_dir"
            ;;
    esac

    # Safety scan
    echo "--- Safety Scan ---"
    safety_scan "$src_dir" "$SKILL_NAME"
    echo ""

    # Dry-run → proposal only
    if $DRY_RUN; then
        local prop_file
        prop_file=$(write_proposal "$SKILL_NAME" "$src_dir" "$PROFILE")
        echo "--- Files to be installed ---"
        local count=0
        while IFS= read -r -d '' file; do
            local dest
            dest=$(skill_dest_for_profile "$PROFILE")/"$SKILL_NAME"
            rel="${file#$src_dir/}"
            echo "  $dest/$rel"
            count=$((count + 1))
        done < <(find "$src_dir" -type f -not -path '*/.git/*' -print0)
        echo "Total: $count files"
        echo ""
        echo "=== DRY-RUN COMPLETE ==="
        echo "Proposal: $prop_file"
        echo "Run with --apply to import."
        $cleanup_tmp && rm -rf "$tmp_dir"
        exit 0
    fi

    # Confirm
    if ! $AUTO_YES; then
        read -r -p "Adopt '$SKILL_NAME' (profile: $PROFILE) into suite? [y/N] " answer
        [[ ! "$answer" =~ ^[Yy]$ ]] && {
            echo "  [SKIP] $SKILL_NAME"
            $cleanup_tmp && rm -rf "$tmp_dir"
            exit 0
        }
    fi

    # Apply
    local dest
    dest=$(skill_dest_for_profile "$PROFILE")/"$SKILL_NAME"
    copy_skill_files "$src_dir" "$dest" || {
        $cleanup_tmp && rm -rf "$tmp_dir"
        exit 1
    }
    append_to_manifest "$SKILL_NAME" "$PROFILE"
    write_adoption_log "$SKILL_NAME" "$PROFILE" "$src_dir"

    $cleanup_tmp && rm -rf "$tmp_dir"

    echo "  [ADOPTED] $SKILL_NAME → $dest"
    echo ""
    echo "=== DONE ==="
}

# ======================================================================
# ignore
# ======================================================================
cmd_ignore() {
    if [[ -z "${SKILL_NAME:-}" ]]; then
        echo "[ERROR] Usage: $0 ignore <skill-name> --reason \"why\""
        exit 1
    fi

    if is_governed "$SKILL_NAME"; then
        echo "[WARN] '$SKILL_NAME' is already governed. Un-adopt first before ignoring."
        exit 1
    fi

    mkdir -p "$(dirname "$IGNORE_LIST")"

    if [[ ! -f "$IGNORE_LIST" ]]; then
        cat > "$IGNORE_LIST" <<'IGHEAD'
# Skill Ignore List
# Skills listed here will be skipped by scan and adopt.
# Auto-generated by govern-new-skills.sh
#
IGHEAD
    fi

    local ts
    ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)

    cat >> "$IGNORE_LIST" <<ENTRY
- skill: $SKILL_NAME
  reason: "${IGNORE_REASON:-manual ignore}"
  timestamp: "$ts"
ENTRY

    echo "  [IGNORED] $SKILL_NAME — reason: ${IGNORE_REASON:-manual ignore}"
}

# ======================================================================
# list
# ======================================================================
cmd_list() {
    echo "=== Skill Governance: List ==="
    echo ""

    echo "--- Governed (global-skills / required) ---"
    for d in "$SKILLS_DIR/"*; do
        [[ -d "$d" ]] || continue
        echo "  [required] $(basename "$d")"
    done

    echo ""
    echo "--- Governed (skill-packs/optional) ---"
    if [[ -d "$OPTIONAL_DIR" ]]; then
        for d in "$OPTIONAL_DIR/"*; do
            [[ -d "$d" ]] || continue
            echo "  [optional] $(basename "$d")"
        done
    fi

    echo ""
    echo "--- Governed (skill-packs/personal) ---"
    if [[ -d "$PERSONAL_DIR" ]]; then
        for d in "$PERSONAL_DIR/"*; do
            [[ -d "$d" ]] || continue
            echo "  [personal] $(basename "$d")"
        done
    fi

    echo ""
    echo "--- Ignored ---"
    if [[ -f "$IGNORE_LIST" ]]; then
        while IFS= read -r line; do
            [[ -n "$line" ]] || continue
            echo "  $line"
        done < <(grep '^[[:space:]]*- skill:' "$IGNORE_LIST" 2>/dev/null || true)
    fi

    echo ""
    echo "--- Ungoverned (local only, not ignored) ---"
    if [[ -d "$LOCAL_SKILLS" ]]; then
        for d in "$LOCAL_SKILLS/"*; do
            [[ -d "$d" ]] || continue
            local name
            name="$(basename "$d")"
            [[ -f "$d/SKILL.md" ]] || continue
            is_governed "$name" && continue
            is_ignored "$name" && continue
            local fc
            fc=$(find "$d" -type f -not -path '*/.git/*' 2>/dev/null | wc -l | tr -d ' ')
            local prof
            prof=$(suggest_profile "$name")
            echo "  $name ($fc files, suggested: $prof)"
        done
    fi
}

# ======================================================================
# Main dispatch
# ======================================================================

case "$SUBCMD" in
    scan)   cmd_scan ;;
    adopt)  cmd_adopt ;;
    ignore) cmd_ignore ;;
    list)   cmd_list ;;
    *)      usage; exit 1 ;;
esac
