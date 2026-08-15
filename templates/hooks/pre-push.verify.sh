#!/usr/bin/env bash
# AGS pre-push verification gate (repo-owned, opt-in).
#
# This hook runs the AGS local verification gate before every `git push`.
# It is NOT installed automatically. Install it manually:
#
#     cp templates/hooks/pre-push.verify.sh .git/hooks/pre-push
#     chmod +x .git/hooks/pre-push
#
# To skip the gate for a single push:   git push --no-verify
# To uninstall:                         rm .git/hooks/pre-push
#
# The hook prefers the REPO-LOCAL verifier built from the current checkout
# (`cargo run -p ags-cli -- check governance`) so it always validates against
# THIS branch's rules, never a possibly stale global binary. If the repo-local
# verifier cannot be built, the hook refuses the push (fail closed).
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

echo "[ags pre-push] running local verification gate…"

# Repo-local first: build the verifier from THIS checkout so the hook can never
# validate with an older global `ags` (version skew = governance gap).
if command -v cargo >/dev/null 2>&1 && [ -f Cargo.toml ]; then
    cargo run -q -p ags-cli -- check governance --workspace . --format text
else
    echo "[ags pre-push] repo-local verifier unavailable; refusing to push (fail closed)." >&2
    exit 1
fi

echo "[ags pre-push] verification passed."
