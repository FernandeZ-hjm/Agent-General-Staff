#!/usr/bin/env bash
set -euo pipefail

die() {
    echo "[ERROR] $*" >&2
    exit 1
}

info() {
    echo "[INFO] $*"
}

if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
else
    REPO_ROOT="${AGS_SOURCE:-$PWD}"
fi

[[ -f "$REPO_ROOT/Cargo.toml" ]] || die "Cargo.toml not found. Run from an AGS checkout or set AGS_SOURCE."
[[ -d "$REPO_ROOT/crates/ags-cli" ]] || die "crates/ags-cli not found in: $REPO_ROOT"
command -v cargo >/dev/null 2>&1 || die "Rust cargo is required. Install Rust first: https://rustup.rs"

info "Installing ags from: $REPO_ROOT"
cargo install --path "$REPO_ROOT/crates/ags-cli" --force

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
command -v ags >/dev/null 2>&1 || die "ags installed, but is not visible on PATH. Add ~/.cargo/bin to PATH."

info "Seeding Claude Code /ags command and Codex AGS command skills"
ags setup --yes --force >/dev/null

cat <<'EOF'
AGS install complete.

Next commands:
  1. In Claude Code, run: /ags setup
     In Codex, use: $ags-setup, $ags-init, $ags-skill, or $ags-doctor
  2. In each project you want AGS to govern, run: /ags init
EOF
