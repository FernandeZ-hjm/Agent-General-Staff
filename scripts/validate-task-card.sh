#!/usr/bin/env bash
# validate-task-card.sh — compatibility alias for scripts/validate.sh.
#
# This is a THIN WRAPPER that delegates to the canonical task-card validator
# entry point (scripts/validate.sh → Rust task-card-validator via
# `ags task-card-validator`). It adds NO second validation logic; it exists
# only because some documentation and the public release manifest reference
# the `validate-task-card.sh` name. There is exactly one canonical gate.
#
# Usage:
#   bash scripts/validate-task-card.sh <task-card-file> [...]
#   bash scripts/validate-task-card.sh -                    # stdin
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec bash "$SCRIPT_DIR/validate.sh" "$@"
