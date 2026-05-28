#!/usr/bin/env bash
set -euo pipefail

# rollback.sh - Rollback suite installation from backup

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_HOME="${TARGET_HOME:-$HOME}"
BACKUP_BASE="$TARGET_HOME/.agents/backups"

DRY_RUN=true
ACTION=""
TARGET=""

usage() {
    cat <<EOF
Usage: rollback.sh [--list] [--backup] [--inspect DIR] [--restore DIR] [--apply] [--target-home PATH]

Options:
  --list          List all available backups
  --backup        Create a new backup of current installed state
  --inspect DIR   Show contents of a specific backup directory
  --restore DIR   Restore from a backup (default: dry-run)
  --apply         With --restore: actually perform the restore
  --target-home   Override target home directory (default: \$HOME)
  --help          Show this message

Environment:
  TARGET_HOME     Override target home directory (CLI --target-home takes precedence)
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --list) ACTION=list ;;
        --backup) ACTION=backup ;;
        --inspect) ACTION=inspect; TARGET="$2"; shift ;;
        --restore) ACTION=restore; TARGET="$2"; shift ;;
        --apply) DRY_RUN=false ;;
        --target-home) TARGET_HOME="$2"; BACKUP_BASE="$TARGET_HOME/.agents/backups"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown: $1"; usage; exit 1 ;;
    esac
    shift
done

if [[ -z "$ACTION" ]]; then
    echo "[ERROR] No action specified. Use --list, --backup, --inspect, or --restore."
    usage
    exit 1
fi

# --- List backups ---
if [[ "$ACTION" == "list" ]]; then
    echo "=== Available Backups ==="
    if [[ ! -d "$BACKUP_BASE" ]]; then
        echo "No backups found at $BACKUP_BASE"
        exit 0
    fi
    for backup in "$BACKUP_BASE"/suite-backup-*; do
        if [[ -d "$backup" ]]; then
            echo "  $(basename "$backup")"
            if [[ -f "$backup/manifest.yaml" ]]; then
                echo "    manifest: yes"
            fi
            if [[ -f "$backup/changed-files.txt" ]]; then
                count=$(wc -l < "$backup/changed-files.txt" | tr -d ' ')
                echo "    changed files: $count"
            fi
        fi
    done
    exit 0
fi

# --- Create new backup ---
if [[ "$ACTION" == "backup" ]]; then
    TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
    BACKUP_DIR="$BACKUP_BASE/suite-backup-$TIMESTAMP"
    echo "Creating backup: $BACKUP_DIR"
    mkdir -p "$BACKUP_DIR/files/.agents/rules"
    mkdir -p "$BACKUP_DIR/files/.codex"
    mkdir -p "$BACKUP_DIR/files/.agents/skills"

    # Backup rules
    for f in "$TARGET_HOME/.agents/rules/SOUL.md" "$TARGET_HOME/.agents/rules/core.md"; do
        if [[ -f "$f" ]]; then
            rel="${f#$TARGET_HOME/}"
            mkdir -p "$(dirname "$BACKUP_DIR/files/$rel")"
            cp "$f" "$BACKUP_DIR/files/$rel"
        fi
    done
    if [[ -f "$TARGET_HOME/.codex/RTK.md" ]]; then
        mkdir -p "$BACKUP_DIR/files/.codex"
        cp "$TARGET_HOME/.codex/RTK.md" "$BACKUP_DIR/files/.codex/RTK.md"
    fi

    # Backup skills — copy each skill dir individually to avoid nesting
    if [[ -d "$TARGET_HOME/.agents/skills" ]]; then
        for skill_dir in "$TARGET_HOME/.agents/skills/"*; do
            if [[ -d "$skill_dir" ]]; then
                skill_name="$(basename "$skill_dir")"
                dst_skill="$BACKUP_DIR/files/.agents/skills/$skill_name"
                rm -rf "$dst_skill"
                mkdir -p "$(dirname "$dst_skill")"
                cp -R "$skill_dir" "$dst_skill"
                echo "  [BACKUP_DIR] .agents/skills/$skill_name"
            fi
        done
    fi

    echo "Backup created: $BACKUP_DIR"
    echo "To restore from this backup:"
    echo "  bash $0 --restore $BACKUP_DIR --apply"
    exit 0
fi

# --- Inspect backup ---
if [[ "$ACTION" == "inspect" ]]; then
    if [[ ! -d "$TARGET" ]]; then
        echo "[ERROR] Backup not found: $TARGET"
        exit 1
    fi
    echo "=== Backup Inspection: $(basename "$TARGET") ==="
    if [[ -f "$TARGET/changed-files.txt" ]]; then
        cat "$TARGET/changed-files.txt"
    else
        echo "  (no changed-files.txt)"
        echo "  Files in backup:"
        find "$TARGET/files" -type f 2>/dev/null | sort || echo "  (none)"
    fi
    exit 0
fi

# --- Restore from backup ---
if [[ "$ACTION" == "restore" ]]; then
    if [[ ! -d "$TARGET" ]]; then
        echo "[ERROR] Backup not found: $TARGET"
        echo "Use --list to see available backups."
        exit 1
    fi

    echo "=== Restore from Backup ==="
    echo "Backup : $TARGET"
    echo "Target : $TARGET_HOME"
    echo "Mode   : $( $DRY_RUN && echo 'DRY-RUN' || echo 'APPLY' )"
    echo ""

    if [[ ! -d "$TARGET/files" ]]; then
        echo "[ERROR] Backup has no files/ directory."
        exit 1
    fi

    # Track which directories we've already cleaned (to avoid double-clean)
    cleaned_dirs=""

    # Preview/apply restore
    while IFS= read -r -d '' file; do
        rel="${file#$TARGET/files/}"
        dest="$TARGET_HOME/$rel"
        dest_dir="$(dirname "$dest")"

        # For skill directories: remove target dir on first file encounter to ensure clean restore
        if [[ "$rel" =~ ^\.agents/skills/([^/]+) ]]; then
            skill_name="${BASH_REMATCH[1]}"
            skill_root="$TARGET_HOME/.agents/skills/$skill_name"
            if [[ "$cleaned_dirs" != *"|$skill_name|"* ]]; then
                if $DRY_RUN; then
                    echo "  [DRY-RUN] Would clean dir: .agents/skills/$skill_name"
                else
                    rm -rf "$skill_root"
                    echo "  [CLEAN] .agents/skills/$skill_name"
                fi
                cleaned_dirs="$cleaned_dirs|$skill_name|"
            fi
        fi

        if $DRY_RUN; then
            echo "  [DRY-RUN] Would restore: $rel"
        else
            mkdir -p "$dest_dir"
            cp "$file" "$dest"
            echo "  [RESTORED] $rel"
        fi
    done < <(find "$TARGET/files" -type f -print0)

    if $DRY_RUN; then
        echo ""
        echo "=== DRY-RUN COMPLETE ==="
        echo "To actually restore, run: $0 --restore $TARGET --apply"
    else
        echo ""
        echo "=== RESTORE COMPLETE ==="
        echo "Run verify.sh to check integrity:"
        echo "  bash $SCRIPT_DIR/verify.sh"
    fi
    exit 0
fi
