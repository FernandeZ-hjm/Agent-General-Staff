#!/usr/bin/env bash
# AGS pre-push verification gate (repo-owned, opt-in).
#
# This hook runs the AGS local verification gate before every `git push`.
# It is NOT installed automatically. Install it explicitly with:
#
#     ags hooks install --confirm
#
# or manually:
#
#     cp templates/hooks/pre-push.verify.sh .git/hooks/pre-push
#     chmod +x .git/hooks/pre-push
#
# To skip the gate for a single push:   git push --no-verify
# To uninstall:                         rm .git/hooks/pre-push
#
# The hook prefers the REPO-LOCAL verifier built from the current checkout
# (`cargo run -p ags-cli -- verify`) so it always validates against THIS
# branch's rules, never a possibly-stale global `ags` on PATH (an older suite
# version would silently apply outdated rules). It falls back to
# `scripts/verify.sh`, then to a PATH `ags` (with a version-mismatch warning),
# and refuses the push if no verifier is available (fail-closed).
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

echo "[ags pre-push] running local verification gate…"

# Repo-local first: build the verifier from THIS checkout so the hook can never
# validate with an older global `ags` (version skew = governance gap).
if command -v cargo >/dev/null 2>&1 && [ -f Cargo.toml ]; then
    cargo run -q -p ags-cli -- verify --scope local --format text
elif [ -f scripts/verify.sh ]; then
    bash scripts/verify.sh
elif command -v ags >/dev/null 2>&1; then
    echo "[ags pre-push] WARNING: cannot build from source; falling back to PATH ags ($(ags --version 2>/dev/null || echo unknown)), which may not match this checkout." >&2
    ags verify --scope local --format text
else
    echo "[ags pre-push] no verifier available (cargo / scripts/verify.sh / ags); refusing to push (fail-closed)." >&2
    exit 1
fi

echo "[ags pre-push] verification passed."
