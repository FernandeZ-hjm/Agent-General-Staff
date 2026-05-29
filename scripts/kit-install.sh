#!/usr/bin/env bash
set -euo pipefail

# kit-install.sh - Public one-click installer wrapper.
# It keeps project workflow installation separate from global runtime setup.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

PROFILE="full"
TARGET_HOME="${TARGET_HOME:-$HOME}"
TARGET_PROJECT=""
PROJECT_NAME=""
PROJECT_SLUG=""
MODE="dry-run"
SCOPE=""

usage() {
    cat <<EOF
Usage: scripts/kit-install.sh --target-project PATH --project-name NAME --project-slug SLUG [options]

One-click public installer for the Dongmenlaohu kit.

Options:
  --profile diy|full      Install profile (default: full)
  --scope project|runtime|all
                          Install project files, runtime files, or both.
                          Default: project for diy, all for full.
  --target-project PATH   Target project root for project workflow files
  --project-name NAME     Human-readable project name
  --project-slug SLUG     Stable lowercase/ascii project slug
  --target-home PATH      Runtime home for global rules/skills (default: \$HOME)
  --dry-run               Preview writes without changing files (default)
  --apply                 Write after reviewing dry-run output
  --help, -h              Show this help
EOF
}

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

run_step() {
    echo ""
    echo "=== $1 ==="
    shift
    "$@"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile)
            PROFILE="${2:-}"
            shift 2
            ;;
        --scope)
            SCOPE="${2:-}"
            shift 2
            ;;
        --target-project)
            TARGET_PROJECT="${2:-}"
            shift 2
            ;;
        --project-name)
            PROJECT_NAME="${2:-}"
            shift 2
            ;;
        --project-slug)
            PROJECT_SLUG="${2:-}"
            shift 2
            ;;
        --target-home)
            TARGET_HOME="${2:-}"
            shift 2
            ;;
        --dry-run)
            MODE="dry-run"
            shift
            ;;
        --apply)
            MODE="apply"
            shift
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
if [[ -z "$SCOPE" ]]; then
    if [[ "$PROFILE" == "full" ]]; then
        SCOPE="all"
    else
        SCOPE="project"
    fi
fi
[[ "$SCOPE" == "project" || "$SCOPE" == "runtime" || "$SCOPE" == "all" ]] || die "--scope must be project, runtime, or all"
[[ "$MODE" == "dry-run" || "$MODE" == "apply" ]] || die "Mode must be dry-run or apply"

if [[ "$SCOPE" == "project" || "$SCOPE" == "all" ]]; then
    [[ -n "$TARGET_PROJECT" ]] || die "--target-project is required for project install"
    [[ -n "$PROJECT_NAME" ]] || die "--project-name is required for project install"
    [[ -n "$PROJECT_SLUG" ]] || die "--project-slug is required for project install"
fi

echo "=== Dongmenlaohu Kit Install ==="
echo "Profile       : $PROFILE"
echo "Scope         : $SCOPE"
echo "Mode          : $MODE"
echo "Target home   : $TARGET_HOME"
echo "Target project: ${TARGET_PROJECT:-<not requested>}"

mode_arg="--dry-run"
if [[ "$MODE" == "apply" ]]; then
    mode_arg="--apply"
fi

if [[ "$SCOPE" == "runtime" || "$SCOPE" == "all" ]]; then
    if [[ "$PROFILE" == "diy" ]]; then
        echo ""
        echo "=== Runtime Install ==="
        echo "DIY/Core does not install the bundled full skill runtime."
        echo "Use --profile full for global rules, skills, and hook normalization."
    else
        run_step "Runtime Install" \
            bash "$SCRIPT_DIR/bootstrap.sh" "$mode_arg" --target-home "$TARGET_HOME"
    fi
fi

if [[ "$SCOPE" == "project" || "$SCOPE" == "all" ]]; then
    run_step "Project Workflow Install" \
        bash "$SCRIPT_DIR/install-suite-to-project.sh" \
            --profile "$PROFILE" \
            --target-project "$TARGET_PROJECT" \
            --project-name "$PROJECT_NAME" \
            --project-slug "$PROJECT_SLUG" \
            "$mode_arg"
fi

if [[ "$MODE" == "apply" ]]; then
    echo ""
    echo "=== Post-Install Check ==="
    doctor_args=(
        doctor
        --profile "$PROFILE" \
        --target-home "$TARGET_HOME"
    )
    if [[ -n "$TARGET_PROJECT" ]]; then
        doctor_args+=(--target-project "$TARGET_PROJECT")
    fi
    bash "$SCRIPT_DIR/kit-doctor.sh" "${doctor_args[@]}"
else
    echo ""
    echo "Dry-run complete. No files were written."
    echo "Run again with --apply after reviewing the planned writes."
fi
