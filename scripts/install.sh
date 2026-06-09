#!/usr/bin/env bash
# AGS 2.0 Public Edition — DIY Install Script
#
# Installs the public-safe Rust ags core, protocols, templates, and basic
# governance commands. No third-party skills are installed by default.
#
# Usage:
#   bash scripts/install.sh                    # Install to default ~/.local/bin
#   bash scripts/install.sh --prefix /opt/ags  # Install to custom prefix
#
# What this script does:
#   1. Builds ags from source (cargo build --release)
#   2. Copies ags binary to PREFIX/bin
#   3. Copies public protocols, templates, manifests to PREFIX/share/ags
#   4. Prints post-install summary with skill recommendations
#
# What this script does NOT do:
#   - Clone any GitHub repositories
#   - Install any third-party skills
#   - Write to $HOME/.agents/skills, $HOME/.codex/skills, or $HOME/.codex/plugins
#   - Run external installation commands
#   - Modify shell profiles

set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
AGS_SHARE="$PREFIX/share/ags"
AGS_BIN="$PREFIX/bin"
INSTALL_SKILLS=false  # Never set to true; always false unless user explicitly
                       # runs --with-skills which is a documented opt-in
SKIP_BUILD=false

# ── Parse args ────────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      PREFIX="$2"; shift 2 ;;
    --skip-build)
      SKIP_BUILD=true; shift ;;
    --help|-h)
      echo "AGS 2.0 Public Edition — DIY Install"
      echo ""
      echo "Usage: bash scripts/install.sh [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --prefix DIR     Install to DIR (default: \$HOME/.local)"
      echo "  --skip-build     Skip cargo build (use existing binary)"
      echo "  --help, -h       Show this help"
      echo ""
      echo "This script builds and installs the AGS core only."
      echo "Third-party skills are NOT installed by default."
      echo "Review docs/skill-recommendations.md for suggested skills."
      exit 0 ;;
    *)
      echo "Unknown option: $1"
      echo "Use --help for usage."
      exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo ""
echo "═══════════════════════════════════════════"
echo "  AGS 2.0 Public Edition — DIY Install"
echo "═══════════════════════════════════════════"
echo ""
echo "  Install prefix:  $PREFIX"
echo "  AGS share dir:   $AGS_SHARE"
echo ""
echo "  This installs the AGS Rust core only."
echo "  Third-party skills are NOT installed."
echo ""

# ── Build ─────────────────────────────────────────────────────────────────

if [ "$SKIP_BUILD" = false ]; then
  echo "── Building ags from source..."
  cd "$REPO_ROOT"
  cargo build --release
  echo "   Build complete."
  echo ""
fi

# ── Install binary ────────────────────────────────────────────────────────

echo "── Installing ags binary..."

mkdir -p "$AGS_BIN"
BINARY_PATH="$REPO_ROOT/target/release/ags"
if [ ! -f "$BINARY_PATH" ]; then
  echo "   ERROR: ags binary not found at $BINARY_PATH"
  echo "   Run 'cargo build --release' first or remove --skip-build."
  exit 1
fi

cp "$BINARY_PATH" "$AGS_BIN/ags"
chmod +x "$AGS_BIN/ags"
echo "   ags → $AGS_BIN/ags"

# ── Install shared data ───────────────────────────────────────────────────

echo "── Installing AGS share data..."

mkdir -p "$AGS_SHARE/protocol"
mkdir -p "$AGS_SHARE/templates"
mkdir -p "$AGS_SHARE/manifests"
mkdir -p "$AGS_SHARE/docs"

# Protocol files
for f in "$REPO_ROOT/protocol/"*.md; do
  if [ -f "$f" ]; then
    cp "$f" "$AGS_SHARE/protocol/"
    echo "   protocol/$(basename "$f")"
  fi
done

# Manifests
for f in "$REPO_ROOT/manifests/"*.yaml; do
  if [ -f "$f" ]; then
    cp "$f" "$AGS_SHARE/manifests/"
    echo "   manifests/$(basename "$f")"
  fi
done

# Docs
for f in "$REPO_ROOT/docs/"*.md; do
  if [ -f "$f" ]; then
    cp "$f" "$AGS_SHARE/docs/"
    echo "   docs/$(basename "$f")"
  fi
done

# Templates
if [ -d "$REPO_ROOT/templates" ]; then
  while IFS= read -r f; do
    rel="${f#$REPO_ROOT/}"
    mkdir -p "$AGS_SHARE/$(dirname "$rel")"
    cp "$f" "$AGS_SHARE/$rel"
    echo "   $rel"
  done < <(find "$REPO_ROOT/templates" -type f | sort)
fi

# ── Check PATH ────────────────────────────────────────────────────────────

echo ""
echo "── Checking PATH..."

if ! echo "$PATH" | tr ':' '\n' | grep -qF "$AGS_BIN"; then
  echo ""
  echo "   ⚠  $AGS_BIN is not in your PATH."
  echo ""
  echo "   Add this to your shell profile:"
  echo ""
  if [ -f "$HOME/.zshrc" ]; then
    echo "     echo 'export PATH=\"$AGS_BIN:\$PATH\"' >> ~/.zshrc"
  elif [ -f "$HOME/.bashrc" ]; then
    echo "     echo 'export PATH=\"$AGS_BIN:\$PATH\"' >> ~/.bashrc"
  else
    echo "     export PATH=\"$AGS_BIN:\$PATH\""
  fi
  echo ""
else
  echo "   ✓ $AGS_BIN is in PATH"
fi

RESOLVED_AGS="$(command -v ags 2>/dev/null || true)"
if [ -n "$RESOLVED_AGS" ]; then
  echo "   command -v ags → $RESOLVED_AGS"
  if [ "$RESOLVED_AGS" != "$AGS_BIN/ags" ]; then
    echo ""
    echo "   ⚠  The shell resolves ags to a different binary."
    echo ""
    echo "   Installed binary:"
    echo "     $AGS_BIN/ags"
    echo "   Active binary:"
    echo "     $RESOLVED_AGS"
    echo ""
    echo "   If 'ags --version' still shows an old version, move $AGS_BIN"
    echo "   earlier in PATH or update the active Rust install explicitly:"
    echo ""
    echo "     cargo install --path crates/ags-cli --force"
    echo ""
  fi
else
  echo "   ags is not currently resolvable by this shell."
fi

# ── Post-install summary ─────────────────────────────────────────────────

echo ""
echo "═══════════════════════════════════════════"
echo "  Installation Complete"
echo "═══════════════════════════════════════════"
echo ""
echo "  AGS core installed to: $PREFIX"
echo ""
echo "  Available commands:"
echo "    ags task validate     Validate task cards"
echo "    ags policy resolve    Resolve execution policy"
echo "    ags policy explain    Explain policy decisions"
echo "    ags policy check      Check policy gate"
echo "    ags sync check        Protocol drift check"
echo "    ags doctor            Suite health diagnostics"
echo "    ags bootstrap         Bootstrap operations"
echo "    ags project detect    Project identity detection"
echo "    ags project integrate Incrementally merge AGS entry blocks"
echo "    ags protocol status   Protocol file status"
echo "    ags agent instructions  Agent instructions"
echo "    ags session preflight   Session preflight"
echo "    ags verify            Scoped verification"
echo ""

# ── Skill recommendations reminder ──────────────────────────────────────

SKILL_RECOMMENDATIONS="$AGS_SHARE/docs/skill-recommendations.md"
REPO_SKILL_RECOMMENDATIONS="$REPO_ROOT/docs/skill-recommendations.md"

echo "  ═══════════════════════════════════════════"
echo "  │  RECOMMENDED: Third-Party Dev Skills  │"
echo "  ═══════════════════════════════════════════"
echo ""
echo "  You have installed the DIY edition — AGS core only."
echo "  For a full development experience, consider installing"
echo "  these recommended third-party development skills:"
echo ""
echo "  Core Development Skills:"
echo "    • brainstorm/superpowers   — Engineering workflow (brainstorm/plan/execute/review)"
echo "    • diagnose                 — Systematic debugging with evidence-chain tracing"
echo "    • tdd                      — Test-driven development (red-green-refactor)"
echo "    • code-review              — Code review with correctness/cleanup checks"
echo "    • using-git-worktrees      — Isolated git worktree management"
echo ""
echo "  Quality & Verification:"
echo "    • auto-verify              — Automatic verification on completion"
echo "    • caveman-commit           — Conventional Commit message generation"
echo "    • caveman-review           — Concise actionable code review feedback"
echo "    • verify                   — Behavioral verification of code changes"
echo ""
echo "  Architecture & Planning:"
echo "    • zoom-out                 — High-level architecture context and risk assessment"
echo "    • improve-codebase-architecture — Architecture improvement patterns"
echo "    • grill-with-docs          — Requirements clarification with project docs"
echo ""
echo "  All skill recommendations are documented in:"
echo "    $SKILL_RECOMMENDATIONS"
if [ -f "$REPO_SKILL_RECOMMENDATIONS" ]; then
  echo "    $REPO_SKILL_RECOMMENDATIONS"
fi
echo ""
echo "  ⚠  IMPORTANT: These are RECOMMENDATIONS ONLY."
echo "     No third-party skills have been installed."
echo "     No repositories have been cloned."
echo "     No files have been written to ~/.agents/, ~/.codex/, or ~/.claude/."
echo ""
echo "  To install a skill, follow the manual instructions in"
echo "  docs/skill-recommendations.md — each skill entry lists its"
echo "  source URL, purpose, risk level, and manual install steps."
echo ""

# ── Quick health check ───────────────────────────────────────────────────

echo "── Running quick health check..."
if command -v "$AGS_BIN/ags" &>/dev/null; then
  "$AGS_BIN/ags" doctor --format text 2>&1 || true
else
  echo "   Skipping — ags not yet on PATH. Run 'ags doctor' after adding to PATH."
fi

echo ""
echo "  Done. Enjoy the DIY AGS experience."
echo "  Run 'ags doctor' anytime for a health check and skill recommendations."
