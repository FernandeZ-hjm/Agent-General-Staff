#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUITE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

PROFILE="full"
TARGET_PROJECT=""
PROJECT_NAME=""
PROJECT_SLUG=""
MODE="dry-run"

usage() {
    cat <<EOF
Usage: scripts/install-suite-to-project.sh --target-project PATH --project-name NAME --project-slug SLUG [options]

Installs this suite's project workflow contract into a target project.

Options:
  --profile diy|full      Select public profile metadata (default: full)
  --target-project PATH   Target project root
  --project-name NAME     Human-readable project name
  --project-slug SLUG     Stable lowercase/ascii project slug
  --dry-run               Preview writes without changing files (default)
  --apply                 Write files after creating a backup
  --help                  Show this help
EOF
}

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile) PROFILE="${2:-}"; shift 2 ;;
        --target-project) TARGET_PROJECT="${2:-}"; shift 2 ;;
        --project-name) PROJECT_NAME="${2:-}"; shift 2 ;;
        --project-slug) PROJECT_SLUG="${2:-}"; shift 2 ;;
        --dry-run) MODE="dry-run"; shift ;;
        --apply) MODE="apply"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "Unknown argument: $1" ;;
    esac
done

[[ "$PROFILE" == "diy" || "$PROFILE" == "full" ]] || die "--profile must be diy or full"
[[ -n "$TARGET_PROJECT" ]] || die "--target-project is required"
[[ -n "$PROJECT_NAME" ]] || die "--project-name is required"
[[ -n "$PROJECT_SLUG" ]] || die "--project-slug is required"
[[ -d "$TARGET_PROJECT" ]] || die "Target project not found: $TARGET_PROJECT"
[[ -f "$SUITE_ROOT/project-integration/AGENTS.md.template" ]] || die "Missing AGENTS.md template"
[[ -d "$SUITE_ROOT/protocol" ]] || die "Missing protocol directory"

TARGET_PROJECT="$(cd "$TARGET_PROJECT" && pwd -P)"
BACKUP_DIR="$TARGET_PROJECT/.agent-suite-backups/$(date +%Y%m%d-%H%M%S)"

targets=(
    "AGENTS.md"
    "CLAUDE.md"
    "docs/agent-workflow"
    "config/agent-project-profile.yaml"
)

echo "=== Install Suite To Project ==="
echo "Suite root    : $SUITE_ROOT"
echo "Target project: $TARGET_PROJECT"
echo "Project name  : $PROJECT_NAME"
echo "Project slug  : $PROJECT_SLUG"
echo "Profile       : $PROFILE"
echo "Mode          : $MODE"
echo "Backup dir    : $BACKUP_DIR"
echo ""

echo "--- Planned writes ---"
for item in "${targets[@]}"; do
    echo "  $TARGET_PROJECT/$item"
done

if [[ "$MODE" == "dry-run" ]]; then
    echo ""
    echo "Dry-run complete. No files were written."
    echo "Run again with --apply after reviewing the planned writes."
    exit 0
fi

mkdir -p "$BACKUP_DIR"

backup_path() {
    local rel="$1"
    local src="$TARGET_PROJECT/$rel"
    if [[ -e "$src" ]]; then
        mkdir -p "$BACKUP_DIR/$(dirname "$rel")"
        cp -R "$src" "$BACKUP_DIR/$rel"
        echo "  [BACKUP] $rel"
    fi
}

echo "--- Backup ---"
for item in "${targets[@]}"; do
    backup_path "$item"
done

echo "--- Install ---"
mkdir -p "$TARGET_PROJECT/docs/agent-workflow" "$TARGET_PROJECT/config"

cp "$SUITE_ROOT/project-integration/AGENTS.md.template" "$TARGET_PROJECT/AGENTS.md"
cp "$SUITE_ROOT/project-integration/CLAUDE.md.template" "$TARGET_PROJECT/CLAUDE.md"
cp -R "$SUITE_ROOT/protocol/." "$TARGET_PROJECT/docs/agent-workflow/"
cp "$SUITE_ROOT/project-integration/config/agent-project-profile.yaml.template" \
    "$TARGET_PROJECT/config/agent-project-profile.yaml"

python3 - "$TARGET_PROJECT" "$PROJECT_NAME" "$PROJECT_SLUG" "$PROFILE" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
name = sys.argv[2]
slug = sys.argv[3]
profile = sys.argv[4]

replacements = {
    "<Project Name>": name,
    "<Brief description of the project - 1-2 lines.>": f"{name} project.",
    "<project-name>": name,
    "<stable-ascii-project-slug>": slug,
    "<web-app | data-pipeline | library | agent-system | other>": "other",
    "<primary verification command>": "bash scripts/verify.sh",
}

for rel in ["CLAUDE.md", "config/agent-project-profile.yaml"]:
    path = root / rel
    text = path.read_text()
    for old, new in replacements.items():
        text = text.replace(old, new)
    text += f"\n# Installed by Dongmenlaohu Multi-Agent Engineering Kit ({profile})\n"
    path.write_text(text)
PY

echo "  [WRITE] AGENTS.md"
echo "  [WRITE] CLAUDE.md"
echo "  [WRITE] docs/agent-workflow/"
echo "  [WRITE] config/agent-project-profile.yaml"

echo ""
echo "Install complete."
echo "Rollback source: $BACKUP_DIR"
