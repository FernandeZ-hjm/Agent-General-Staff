#!/usr/bin/env bash
# lane-decision.sh — trusted, pure-shell push-lane decision.
#
# Reads changed file paths (one per line) on stdin and prints exactly one of:
#   MINIMAL  — every changed file is ignore-rule or doc hygiene; the push gate
#              may skip cargo test/build + validator and must NOT touch stable.
#   FULL     — anything else: source, scripts, Cargo.*, config, protocol,
#              governance, manifests, root-entry docs, unknown files, or an
#              empty input. Takes the full guarded path.
#
# This is deliberately independent of the Rust workspace. The push gate must
# never ask the in-tree (possibly changed) `ags` binary whether it may skip
# verification — a broken or malicious classifier change could then route a
# source/protocol diff through MINIMAL before any full verification ran. Keeping
# the decision here, in plain shell over an allowlist, makes it a fail-safe gate
# rather than a self-verification bypass.
#
# Because any change to gate-selection code (scripts/, crates/, classifier) is a
# non-hygiene file, such a change always forces FULL — including a change to
# this very script.
set -euo pipefail

minimal=1
seen=0
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    seen=1
    case "$f" in
        # Protected paths ALWAYS force FULL (checked first).
        protocol/* | governance/* | manifests/* | AGENTS.md | CLAUDE.md | AGENT_SUITE_PROTOCOL.md | WORKSPACE.md)
            minimal=0 ;;
        # ignore-rule hygiene (root or nested).
        .gitignore | .dockerignore | .*ignore | */.gitignore | */.dockerignore) ;;
        # doc hygiene (protocol/governance docs already forced FULL above).
        *.md | *.txt) ;;
        # Everything else (source, scripts, Cargo.*, config, unknown) forces FULL.
        *) minimal=0 ;;
    esac
    [[ "$minimal" -eq 0 ]] && break
done

# Empty input is not "hygiene-only" — fail safe to FULL.
if [[ "$seen" -eq 1 && "$minimal" -eq 1 ]]; then
    echo "MINIMAL"
else
    echo "FULL"
fi
