#!/usr/bin/env bash
# AGS 2.0 Public Edition — Update Check / Update Script
#
# Usage:
#   bash scripts/update.sh --check
#   bash scripts/update.sh --check --max-age-days 1
#   bash scripts/update.sh --apply
#
# The check path reports update status only. It does not pull, build, install,
# edit shell profiles, or update third-party skills. A small cache file under
# ~/.cache/ags records the last successful remote check time.

set -euo pipefail

MODE="check"
MAX_AGE_DAYS=""
PREFIX="${PREFIX:-$HOME/.local}"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/ags"
CACHE_FILE="$CACHE_DIR/update-check.json"

usage() {
  cat <<'USAGE'
AGS 2.0 Public Edition — Update

Usage:
  bash scripts/update.sh --check [--max-age-days N]
  bash scripts/update.sh --apply [--prefix DIR]

Options:
  --check           Report whether a newer remote revision is available.
  --apply           Pull the configured upstream and reinstall AGS.
  --max-age-days N  Skip the remote check if the cache is newer than N days.
  --prefix DIR      Install prefix for --apply (default: $HOME/.local).
  --help, -h        Show this help.

Safety:
  --check does not pull, build, install, edit PATH, or update skills.
  --apply uses git pull --ff-only, then delegates to scripts/install.sh.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      MODE="check"; shift ;;
    --apply)
      MODE="apply"; shift ;;
    --max-age-days)
      MAX_AGE_DAYS="$2"; shift 2 ;;
    --prefix)
      PREFIX="$2"; shift 2 ;;
    --help|-h)
      usage
      exit 0 ;;
    *)
      echo "Unknown option: $1"
      echo "Use --help for usage."
      exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGS_BIN="$PREFIX/bin"
INSTALL_TARGET="$AGS_BIN/ags"

cd "$REPO_ROOT"

require_git_repo() {
  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "ERROR: update.sh must run from an AGS git checkout."
    exit 1
  fi
}

current_timestamp() {
  date +%s
}

cache_is_fresh() {
  local max_age_days="$1"
  [[ -n "$max_age_days" ]] || return 1
  [[ "$max_age_days" =~ ^[0-9]+$ ]] || {
    echo "ERROR: --max-age-days must be a non-negative integer."
    exit 2
  }
  [[ -f "$CACHE_FILE" ]] || return 1

  local checked_at now max_age_seconds
  checked_at="$(sed -n 's/.*"checked_at": \([0-9][0-9]*\).*/\1/p' "$CACHE_FILE" | head -n 1)"
  [[ -n "$checked_at" ]] || return 1

  now="$(current_timestamp)"
  max_age_seconds=$(( max_age_days * 86400 ))
  (( now - checked_at < max_age_seconds ))
}

read_cached_summary() {
  if [[ -f "$CACHE_FILE" ]]; then
    sed -n 's/.*"summary": "\(.*\)".*/\1/p' "$CACHE_FILE" | head -n 1
  fi
}

write_cache() {
  local summary="$1"
  mkdir -p "$CACHE_DIR"
  cat > "$CACHE_FILE" <<JSON
{"checked_at": $(current_timestamp), "summary": "$summary"}
JSON
}

installed_ags_path() {
  command -v ags 2>/dev/null || true
}

installed_ags_version() {
  if command -v ags >/dev/null 2>&1; then
    ags --version 2>/dev/null || true
  fi
}

workspace_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1
}

current_branch() {
  git rev-parse --abbrev-ref HEAD
}

current_head() {
  git rev-parse HEAD
}

upstream_ref() {
  git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true
}

remote_head_for_branch() {
  local upstream remote branch
  upstream="$(upstream_ref)"
  if [[ -n "$upstream" && "$upstream" == */* ]]; then
    remote="${upstream%%/*}"
    branch="${upstream#*/}"
  else
    remote="origin"
    branch="$(current_branch)"
  fi

  git ls-remote "$remote" "refs/heads/$branch" 2>/dev/null | awk '{print $1; exit}'
}

print_local_status() {
  echo ""
  echo "═══════════════════════════════════════════"
  echo "  AGS Update Check"
  echo "═══════════════════════════════════════════"
  echo ""
  echo "  Repository:        $REPO_ROOT"
  echo "  Workspace version: $(workspace_version)"
  echo "  Current branch:    $(current_branch)"
  echo "  Local HEAD:        $(current_head)"
  echo "  Resolved ags:      $(installed_ags_path)"
  echo "  Resolved version:  $(installed_ags_version)"
  echo "  Install target:    $INSTALL_TARGET"
}

check_path_resolution() {
  local resolved
  resolved="$(installed_ags_path)"
  echo ""
  echo "── PATH resolution"
  if [[ -z "$resolved" ]]; then
    echo "   ags is not currently on PATH."
    echo "   Run: bash scripts/install.sh --prefix \"$PREFIX\""
    return
  fi

  echo "   command -v ags → $resolved"
  if [[ "$resolved" != "$INSTALL_TARGET" ]]; then
    echo ""
    echo "   WARNING: scripts/install.sh installs to:"
    echo "     $INSTALL_TARGET"
    echo "   but your shell resolves ags to:"
    echo "     $resolved"
    echo ""
    echo "   If the version still looks old after install, move $AGS_BIN earlier"
    echo "   in PATH or reinstall the active binary explicitly:"
    echo "     cargo install --path crates/ags-cli --force"
  else
    echo "   ✓ PATH resolves to the install target."
  fi
}

run_check() {
  require_git_repo

  if cache_is_fresh "$MAX_AGE_DAYS"; then
    print_local_status
    echo ""
    echo "── Remote check"
    echo "   Skipped: cache is newer than $MAX_AGE_DAYS day(s)."
    local cached_summary
    cached_summary="$(read_cached_summary)"
    if [[ -n "$cached_summary" ]]; then
      echo "   Last result: $cached_summary"
    fi
    check_path_resolution
    return
  fi

  local local_head remote_head summary
  local_head="$(current_head)"
  remote_head="$(remote_head_for_branch)"

  print_local_status
  echo ""
  echo "── Remote check"

  if [[ -z "$remote_head" ]]; then
    summary="remote-unavailable"
    echo "   Remote HEAD: unavailable"
    echo "   Result: could not check remote updates."
    write_cache "$summary"
    check_path_resolution
    exit 1
  fi

  echo "   Remote HEAD:       $remote_head"
  if [[ "$local_head" == "$remote_head" ]]; then
    summary="up-to-date"
    echo "   Result: up to date."
  else
    summary="update-available"
    echo "   Result: update available."
    echo ""
    echo "   Apply with:"
    echo "     bash scripts/update.sh --apply"
  fi

  write_cache "$summary"
  check_path_resolution
}

run_apply() {
  require_git_repo

  echo ""
  echo "═══════════════════════════════════════════"
  echo "  AGS Update Apply"
  echo "═══════════════════════════════════════════"
  echo ""
  echo "  Repository:      $REPO_ROOT"
  echo "  Install prefix:  $PREFIX"
  echo ""
  echo "── Pulling latest changes..."
  git pull --ff-only
  echo ""
  echo "── Reinstalling AGS..."
  bash "$SCRIPT_DIR/install.sh" --prefix "$PREFIX"
  echo ""
  echo "── Verifying updated command..."
  check_path_resolution
  echo ""
  echo "── Version"
  "$INSTALL_TARGET" --version
  echo ""
  echo "── Local verification"
  "$INSTALL_TARGET" verify --scope local
  write_cache "updated"
}

case "$MODE" in
  check)
    run_check ;;
  apply)
    run_apply ;;
  *)
    echo "ERROR: unknown mode: $MODE"
    exit 2 ;;
esac
