#!/usr/bin/env bash
# AGS Public Edition — stable -> public projection helper.
#
# This script belongs to the public worktree only. It is not part of the
# private/stable product surface. It projects the current stable suite into this
# public checkout while preserving public-only release files and stripping
# private-only payload.

set -euo pipefail

MODE="dry-run"
COMMIT=false
PUSH=false
ALLOW_DIRTY=false
STABLE_ROOT="${STABLE_ROOT:-/Volumes/AI Project/agent-governance-suite-stable}"
PUBLIC_ROOT=""
REMOTE="origin"

usage() {
  cat <<'USAGE'
AGS Public Edition — stable -> public sync

Usage:
  bash scripts/sync-public.sh --dry-run
  bash scripts/sync-public.sh --apply
  bash scripts/sync-public.sh --apply --commit
  bash scripts/sync-public.sh --apply --commit --push

Options:
  --stable DIR   Stable private checkout (default: /Volumes/AI Project/agent-governance-suite-stable)
  --public DIR   Public checkout (default: repository containing this script)
  --dry-run      Show projection plan only
  --apply        Copy public-safe files from stable into public checkout
  --commit       Commit resulting public checkout changes locally
  --push         Push committed public changes to local remote origin
  --allow-dirty  Allow an existing dirty public worktree
  --help, -h     Show help

Boundary:
  - Product code and public-safe protocol files come from stable public-full plan.
  - Public-only release files stay in the public checkout.
  - Private runtime, EvoMap/GEP/Evolver runtime payload, personal skills,
    real memory, hooks, secrets, and machine state are not copied.
  - This script never pushes to GitHub.
USAGE
}

die() {
  echo "[sync-public] ERROR: $*" >&2
  exit 1
}

info() {
  echo "[sync-public] $*"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stable)
      STABLE_ROOT="$2"; shift 2 ;;
    --public)
      PUBLIC_ROOT="$2"; shift 2 ;;
    --dry-run)
      MODE="dry-run"; shift ;;
    --apply)
      MODE="apply"; shift ;;
    --commit)
      COMMIT=true; shift ;;
    --push)
      PUSH=true; shift ;;
    --allow-dirty)
      ALLOW_DIRTY=true; shift ;;
    --help|-h)
      usage; exit 0 ;;
    *)
      die "unknown option: $1" ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
DEFAULT_PUBLIC_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
PUBLIC_ROOT="${PUBLIC_ROOT:-$DEFAULT_PUBLIC_ROOT}"

[[ -d "$STABLE_ROOT/.git" ]] || die "stable checkout not found: $STABLE_ROOT"
[[ -d "$PUBLIC_ROOT/.git" ]] || die "public checkout not found: $PUBLIC_ROOT"
[[ "$MODE" == "apply" || "$COMMIT" == "false" ]] || die "--commit requires --apply"
[[ "$COMMIT" == "true" || "$PUSH" == "false" ]] || die "--push requires --commit"

if [[ "$(git -C "$PUBLIC_ROOT" remote get-url "$REMOTE" 2>/dev/null || true)" == *github.com* ]]; then
  die "remote '$REMOTE' points to GitHub; refusing to push"
fi

if [[ "$ALLOW_DIRTY" != "true" ]]; then
  dirty="$(git -C "$PUBLIC_ROOT" status --porcelain)"
  if [[ -n "$dirty" ]]; then
    allowed="$(printf '%s\n' "$dirty" | awk '$2 != "scripts/sync-public.sh" { print }')"
    [[ -z "$allowed" ]] || die "public worktree is dirty; commit/stash first or pass --allow-dirty"
  fi
fi

info "stable: $STABLE_ROOT"
info "public: $PUBLIC_ROOT"
info "mode:   $MODE"

info "updating stable checkout"
git -C "$STABLE_ROOT" pull --ff-only

PLAN_JSON="$(mktemp)"
INCLUDED_LIST="$(mktemp)"
COPIED_LIST="$(mktemp)"
SKIPPED_LIST="$(mktemp)"
trap 'rm -f "$PLAN_JSON" "$INCLUDED_LIST" "$COPIED_LIST" "$SKIPPED_LIST"' EXIT

info "reading stable public-full package plan"
(
  cd "$STABLE_ROOT"
  cargo run -q -p ags-cli -- release package --profile public-full --dry-run --format json
) > "$PLAN_JSON"

ruby -rjson -e 'JSON.parse(File.read(ARGV[0])).fetch("included_files").each { |f| puts f }' "$PLAN_JSON" \
  > "$INCLUDED_LIST"

should_skip_public_overlay() {
  local rel="$1"
  case "$rel" in
    .gitignore|.github/*|LICENSE|COMMERCIAL.md|NOTICE.md|README.md|README.en.md|RELEASE_NOTES.md|THIRD_PARTY_NOTICES.md)
      return 0 ;;
    AGENTS.md|CLAUDE.md|WORKSPACE.md|AGENT_SUITE_PROTOCOL.md)
      return 0 ;;
    Cargo.toml|Cargo.lock)
      return 0 ;;
    docs/*|evals/*|examples/*|templates/*|config/*)
      return 0 ;;
    scripts/install.sh|scripts/sync-public.sh|scripts/update.sh|scripts/verify.sh|scripts/push-a1.sh)
      return 0 ;;
    manifests/skill-recommendations.yaml|governance/skill-adoption-log.yaml|governance/skill-ignore-list.yaml)
      return 0 ;;
    crates/ags-cli/*|crates/ags-mcp/*|crates/ags-verify/*|crates/receipt/*|crates/skill-governance/*)
      return 0 ;;
  esac
  return 1
}

contains_private_payload() {
  local source_file="$1"
  [[ -f "$source_file" ]] || return 1
  grep -Iq . "$source_file" || return 1
  grep -Eq '(/Volumes/AI Project/agent-governance-suite-private|/Users/hujiaming/git-remotes/agent-governance-suite-private\.git|node_secret|EVOLVER_PROXY_MCP|evolver-token|with-evomap|gep-mcp-server|@evomap)' "$source_file"
}

while IFS= read -r rel; do
  [[ -n "$rel" ]] || continue
  src="$STABLE_ROOT/$rel"
  dst="$PUBLIC_ROOT/$rel"

  if should_skip_public_overlay "$rel"; then
    echo "overlay $rel" >> "$SKIPPED_LIST"
    continue
  fi

  if contains_private_payload "$src"; then
    echo "private-content $rel" >> "$SKIPPED_LIST"
    continue
  fi

  echo "$rel" >> "$COPIED_LIST"
  if [[ "$MODE" == "apply" ]]; then
    mkdir -p "$(dirname "$dst")"
    cp "$src" "$dst"
  fi
done < "$INCLUDED_LIST"

echo
echo "Projection summary"
echo "=================="
echo "included from stable plan: $(wc -l < "$INCLUDED_LIST" | tr -d ' ')"
echo "copied:                    $(wc -l < "$COPIED_LIST" | tr -d ' ')"
echo "skipped/overlay:           $(wc -l < "$SKIPPED_LIST" | tr -d ' ')"
echo
echo "Skipped public overlays / private-content guards:"
sed -n '1,120p' "$SKIPPED_LIST" || true
if [[ "$(wc -l < "$SKIPPED_LIST" | tr -d ' ')" -gt 120 ]]; then
  echo "... truncated"
fi

if [[ "$MODE" == "dry-run" ]]; then
  echo
  echo "Dry-run only. Re-run with --apply to update the public checkout."
  exit 0
fi

info "checking for forbidden public payload paths"
for forbidden in \
  global-skills skill-packs .agents .codex .claude/local .evolver evomap \
  mcp/gep.mcp.json hosts/claude-code.evomap-mcp.snippet.json \
  bin/evolver-proxy-mcp manifests/runtime-profiles.yaml manifests/templates \
  memory task-archive
do
  if [[ -e "$PUBLIC_ROOT/$forbidden" ]] && ! git -C "$PUBLIC_ROOT" check-ignore -q "$forbidden"; then
    die "forbidden public payload present: $forbidden"
  fi
done

info "formatting Rust workspace"
(
  cd "$PUBLIC_ROOT"
  cargo fmt --check
)

info "running public local verify"
(
  cd "$PUBLIC_ROOT"
  cargo run -q -p ags-cli -- verify --scope local --format text
)

info "running public release boundary verify"
RELEASE_VERIFY_LOG="$(mktemp)"
if (
  cd "$PUBLIC_ROOT"
  cargo run -q -p ags-cli -- verify --scope release --format text
) > "$RELEASE_VERIFY_LOG" 2>&1; then
  cat "$RELEASE_VERIFY_LOG"
else
  cat "$RELEASE_VERIFY_LOG"
  if grep -q '^\[FAIL\]' "$RELEASE_VERIFY_LOG"; then
    rm -f "$RELEASE_VERIFY_LOG"
    die "release verification reported failing checks"
  fi
  info "release verification returned non-zero due to warnings only; continuing"
fi
rm -f "$RELEASE_VERIFY_LOG"

if [[ "$COMMIT" == "true" ]]; then
  info "committing public sync changes"
  git -C "$PUBLIC_ROOT" add -A
  if git -C "$PUBLIC_ROOT" diff --cached --quiet; then
    info "no changes to commit"
  else
    stable_head="$(git -C "$STABLE_ROOT" rev-parse --short HEAD)"
    git -C "$PUBLIC_ROOT" commit -m "chore(public): sync from stable $stable_head"
  fi
fi

if [[ "$PUSH" == "true" ]]; then
  info "pushing to local remote '$REMOTE'"
  git -C "$PUBLIC_ROOT" push "$REMOTE" HEAD:main
fi

info "public sync complete"
