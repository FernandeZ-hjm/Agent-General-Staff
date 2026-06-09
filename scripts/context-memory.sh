#!/usr/bin/env bash
# AGS public context memory helper.
#
# Provides a small public-safe wrapper around `ags archive` so protocols can
# refer to a stable capture entry point without shipping private memory data.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/context-memory.sh capture [OPTIONS]

Options:
  --summary TEXT              One-line task summary
  --delivery-report PATH      Delivery report markdown
  --task-card PATH            Task card used for the task
  --verification-results PATH Verification results file
  --receipt PATH              Receipt JSON
  --format text|json          Output format (default: text)

Environment:
  AGS_MEMORY_DIR              Override memory root or project memory dir
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

cmd="${1:-}"
shift || true

if [[ "$cmd" != "capture" ]]; then
  usage >&2
  exit 2
fi

if command -v ags >/dev/null 2>&1; then
  AGS_BIN="ags"
elif [[ -x "./target/release/ags" ]]; then
  AGS_BIN="./target/release/ags"
elif [[ -x "./target/debug/ags" ]]; then
  AGS_BIN="./target/debug/ags"
else
  echo "context-memory: ags binary not found; run cargo build --release or add ags to PATH" >&2
  exit 1
fi

exec "$AGS_BIN" archive "$@"
