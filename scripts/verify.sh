#!/usr/bin/env bash
# verify.sh — AGS verification gate (compatibility wrapper).
#
# Canonical verification authority is now `ags verify`. This script:
#   1. Runs `ags verify --scope full --format text` as the primary gate.
#   2. Runs remaining shell-only smoke tests that have not yet been
#      migrated to Rust check items.
#   3. Exits with the combined result.
#
# To add new checks, prefer adding Rust CheckItems in crates/ags-verify/
# over extending this script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
failures=0

run_check() {
    local label="$1"
    shift
    echo -n "[....] $label "
    if "$@" > /tmp/verify-check.log 2>&1; then
        echo "OK"
    else
        echo "FAIL"
        cat /tmp/verify-check.log
        failures=$((failures + 1))
    fi
}

write_smoke_card() {
    local path="$1"
    local execution_surface="$2"
    local permission_mode="$3"
    local parallelism="$4"
    local workflow_authority="$5"
    local task_level="$6"
    local task="$7"
    local goal="$8"
    local non_goal="$9"
    local stop_condition="${10}"
    local expected_evidence="${11}"
    local delivery="${12}"

    cat > "$path" << TASKEOF
## 任务卡

读取并遵守：
- AGENTS.md

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: $execution_surface

Permission mode: $permission_mode

Parallelism: $parallelism

Execution effort: normal

Workflow authority: $workflow_authority

任务级别：$task_level

Review gate:
- Smoke test review

任务：$task

背景：verify.sh resolver smoke test uses the canonical classic task-card skeleton.

项目画像：Agent Governance Suite private Rust workspace.

记忆胶囊：暂无相关记忆。

任务存档：verify.sh shell-only smoke regression.

目标文件夹路径：
- $REPO_ROOT

相关路径：
- .

本次任务相关文件：
- scripts/verify.sh

目标：$goal

非目标：$non_goal

验证：
echo done

Verification gate:
- commands: echo done
- expected evidence: $expected_evidence
- stop condition: $stop_condition

交付：
$delivery
TASKEOF
}

cd "$REPO_ROOT"
export PATH="$REPO_ROOT/target/debug:$PATH"

echo "=== AGS Verification Gate ==="
echo "Repo: $REPO_ROOT"
echo ""

# ── Primary gate: ags verify (Rust structured verification) ─────────────────
echo "--- Primary Gate: ags verify --scope full ---"
echo ""
set +e
cargo run -q -p ags-cli -- verify --scope full --format text
ags_verify_rc=$?
set -e
echo ""
if [ "$ags_verify_rc" -ne 0 ]; then
    # ags verify already printed the structured report.
    # We count this as one aggregated failure for the exit code.
    failures=$((failures + 1))
fi
echo "--- End Primary Gate ---"
echo ""

# ── Supply-chain gate: cargo-deny ───────────────────────────────────────────
# Dependency advisory / license / source policy. Defined in deny.toml.
# Fail-closed: a missing cargo-deny is treated as a gate FAILURE (not skipped),
# so a minimal CI image or fresh box cannot pass verification without the
# supply-chain checks actually running. Install: cargo install cargo-deny --locked.
echo "--- Supply-chain Gate: cargo deny check ---"
if command -v cargo-deny >/dev/null 2>&1; then
    set +e
    cargo deny check 2>&1 | tail -3
    deny_rc=${PIPESTATUS[0]}
    set -e
    if [ "$deny_rc" -ne 0 ]; then
        echo "[FAIL] cargo deny check (supply-chain policy violation)"
        failures=$((failures + 1))
    else
        echo "[OK] cargo deny check"
    fi
else
    echo "[FAIL] cargo-deny not installed — supply-chain gate cannot run (fail-closed)."
    echo "       Install with: cargo install cargo-deny --locked"
    failures=$((failures + 1))
fi
echo ""

# ── Shell-only smoke tests (pending migration to Rust check items) ──────────
# These tests create temporary task cards, run policy resolution, parse JSON
# output, and check for specific patterns. They are integration-level smoke
# tests that require the full CLI, stdin/stdout, and JSON parsing.
# Migration status: candidates for future ags-verify check items, blocked on:
#   - Complex inline task card generation (F1-F10, M3-M8, R1-R6, SP1-SP11)
#   - Cross-command JSON field comparison (CLI compat)
#   - Document text grep checks (L1-L11)
#   - Temporary directory/bootstrap state management (M7)

echo "--- Remaining Shell-Only Smoke Tests ---"
echo "(These tests are candidates for future Rust migration.)"
echo ""

echo "--- Verify CLI Surface Smoke Tests ---"
run_check "ags verify documented form accepts --scope" \
    cargo run -q -p ags-cli -- verify --scope local --format json
run_check "ags verify run compatibility alias accepts --scope" \
    cargo run -q -p ags-cli -- verify run --scope local --format json

# ── Resolve-Policy Smoke Tests ──────────────────────────────────────────────
echo "--- Resolve-Policy Smoke Tests ---"
run_check "resolve-policy valid card (text)" \
    cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format text
run_check "resolve-policy valid full (json)" \
    cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format json
echo -n "[....] resolve-policy invalid fixture (expected FAIL) "
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/invalid-ultracode-authority-abuse.md" --format text > /tmp/verify-resolve-invalid.log 2>&1; then
    echo "FAIL (expected non-zero exit)"
    failures=$((failures + 1))
elif ! grep -q "ULTRACODE_AUTHORITY_ABUSE" /tmp/verify-resolve-invalid.log; then
    echo "FAIL (missing ULTRACODE_AUTHORITY_ABUSE)"
    cat /tmp/verify-resolve-invalid.log
    failures=$((failures + 1))
else
    echo "OK (correctly rejected: ULTRACODE_AUTHORITY_ABUSE)"
fi
echo -n "[....] resolve-policy illegal format (expected FAIL) "
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format yaml > /tmp/verify-resolve-format.log 2>&1; then
    echo "FAIL (expected non-zero exit)"
    failures=$((failures + 1))
elif ! grep -qE 'invalid value|possible values' /tmp/verify-resolve-format.log; then
    echo "FAIL (missing clap format error)"
    cat /tmp/verify-resolve-format.log
    failures=$((failures + 1))
else
    echo "OK (correctly rejected: clap format validation)"
fi
echo -n "[....] resolve-policy stdin (json) "
if cat "$REPO_ROOT/tests/fixtures/valid-full.md" | cargo run -q -p ags-cli -- policy resolve - --format json > /tmp/verify-resolve-stdin.log 2>&1; then
    echo "OK"
else
    echo "FAIL"
    cat /tmp/verify-resolve-stdin.log
    failures=$((failures + 1))
fi

# ── Task-Card Format Gate Smoke (single canonical format) ───────────────────
echo "--- Task-Card Format Gate Smoke ---"
echo -n "[....] task validate rejects removed compact format (expected FAIL) "
if cargo run -q -p ags-cli -- task validate "$REPO_ROOT/tests/fixtures/invalid-compact.md" > /tmp/verify-compact-reject.log 2>&1; then
    echo "FAIL (compact format was accepted)"
    cat /tmp/verify-compact-reject.log
    failures=$((failures + 1))
elif ! grep -q "compact 任务卡格式已删除" /tmp/verify-compact-reject.log; then
    echo "FAIL (rejected, but not for compact-format removal — fail closed)"
    cat /tmp/verify-compact-reject.log
    failures=$((failures + 1))
else
    echo "OK (removed compact format correctly rejected)"
fi
echo -n "[....] task compile output is classic + passes real validator "
compile_intent=$'任务：verify.sh e2e compile smoke\n目标：确认编译产物为经典骨架并通过真实校验器'
if printf '%s' "$compile_intent" | cargo run -q -p ags-cli -- task compile - --task-card-requested --output card > /tmp/verify-compiled-card.md 2>/dev/null \
    && ! grep -q "AGENT_SUITE_COMPACT_TASK_CARD_V1" /tmp/verify-compiled-card.md \
    && head -n 1 /tmp/verify-compiled-card.md | grep -q "^## 任务卡" \
    && cargo run -q -p ags-cli -- task validate /tmp/verify-compiled-card.md > /tmp/verify-compiled-validate.log 2>&1; then
    echo "OK (compiled card is classic and validator-clean)"
else
    echo "FAIL (compiled card not classic or rejected by validator)"
    cat /tmp/verify-compiled-card.md /tmp/verify-compiled-validate.log 2>/dev/null
    failures=$((failures + 1))
fi
echo -n "[....] no generatable fallback/compact task-card template files remain "
# Single canonical template: the only task-card skeleton is
# protocol/task-card-template.md. Per-level fallback templates and any
# templates/fallback-task-cards/ set must not exist. The compact marker
# AGENT_SUITE_COMPACT_TASK_CARD_V1 may appear ONLY in the invalid fixture and
# validator/compiler/verify rejection code — never in a usable template.
# Skip generated/gitignored dirs (target/, .git/, .codegraph/): the CodeGraph
# index is a binary cache that mirrors source text and is not a usable source.
stray_templates=$(find "$REPO_ROOT" -type f \
    \( -name '*-task-template.md' -o -path '*templates/fallback-task-cards/*' \) \
    -not -path '*/target/*' -not -path '*/.git/*' -not -path '*/.codegraph/*' 2>/dev/null)
marker_files=$(grep -rl --exclude-dir=target --exclude-dir=.git --exclude-dir=.codegraph \
    "AGENT_SUITE_COMPACT_TASK_CARD_V1" "$REPO_ROOT" 2>/dev/null \
    | grep -v -E 'tests/fixtures/(invalid|removed)-compact\.md$|task-card-validator/src/(validate|tests)\.rs$|task-compiler/src/lib\.rs$|ags-verify/src/lib\.rs$|scripts/verify\.sh$' \
    || true)
if [ -n "$stray_templates" ]; then
    echo "FAIL (stray fallback/per-level task-card template files found)"
    echo "$stray_templates"
    failures=$((failures + 1))
elif [ -n "$marker_files" ]; then
    echo "FAIL (compact marker outside fixture/rejection code)"
    echo "$marker_files"
    failures=$((failures + 1))
else
    echo "OK (single canonical task-card template; no fallback/compact sources)"
fi

# ── Prompt Request / Output Gate Smoke (entry intent-recognition) ───────────
echo "--- Prompt Request / Output Gate Smoke ---"

# Positive: a task-card/prompt request must be detected and require a card.
echo -n "[....] gate prompt-request detects '给我提示词' (require_task_card) "
if cargo run -q -p ags-cli -- gate prompt-request "给我提示词" --no-preflight --format json > /tmp/verify-gate-pr-pos.log 2>&1 \
    && grep -q '"decision": "require_task_card"' /tmp/verify-gate-pr-pos.log \
    && grep -q '"is_task_card_request": true' /tmp/verify-gate-pr-pos.log; then
    echo "OK"
else
    echo "FAIL (task-card request not detected)"
    cat /tmp/verify-gate-pr-pos.log
    failures=$((failures + 1))
fi

# Negative: ordinary prose must NOT be classified as a task-card request.
echo -n "[....] gate prompt-request lets prose through (allow) "
if cargo run -q -p ags-cli -- gate prompt-request "解释这段代码是做什么的" --no-preflight --format json > /tmp/verify-gate-pr-neg.log 2>&1 \
    && grep -q '"decision": "allow"' /tmp/verify-gate-pr-neg.log \
    && grep -q '"is_task_card_request": false' /tmp/verify-gate-pr-neg.log; then
    echo "OK"
else
    echo "FAIL (prose misclassified as task-card request)"
    cat /tmp/verify-gate-pr-neg.log
    failures=$((failures + 1))
fi

# Value Route (效价比路由) is exposed on the entry gate (advisory — it never
# changes the task level, permission mode, Review gate, or Verification gate).
echo -n "[....] gate prompt-request exposes value_route (recommended_path) "
if cargo run -q -p ags-cli -- gate prompt-request "给我提示词" --no-preflight --format json > /tmp/verify-gate-vr.log 2>&1 \
    && grep -q '"value_route"' /tmp/verify-gate-vr.log \
    && grep -q '"recommended_path"' /tmp/verify-gate-vr.log \
    && grep -q '"authority_note"' /tmp/verify-gate-vr.log; then
    echo "OK"
else
    echo "FAIL (value_route block missing from gate prompt-request)"
    cat /tmp/verify-gate-vr.log
    failures=$((failures + 1))
fi

# Positive end-to-end: request → compiled canonical card → gate output ALLOW.
echo -n "[....] '给我提示词' → compile → gate output ALLOW + validator-clean "
pr_intent=$'任务：verify.sh prompt-request e2e smoke\n目标：确认触发词请求经编译产出经典骨架并通过 output gate'
if printf '%s' "$pr_intent" | cargo run -q -p ags-cli -- task compile - --task-card-requested --output card > /tmp/verify-gate-card.md 2>/dev/null \
    && cargo run -q -p ags-cli -- gate output /tmp/verify-gate-card.md --format json > /tmp/verify-gate-out-pos.log 2>&1 \
    && grep -q '"decision": "allow"' /tmp/verify-gate-out-pos.log \
    && cargo run -q -p ags-cli -- task validate /tmp/verify-gate-card.md > /dev/null 2>&1; then
    echo "OK"
else
    echo "FAIL (canonical card rejected by output gate or validator)"
    cat /tmp/verify-gate-out-pos.log
    failures=$((failures + 1))
fi

# Negative end-to-end (fail-closed): a non-canonical foreground answer for a
# detected task-card request must be BLOCKED with a governance_miss event.
echo -n "[....] non-'## 任务卡' output for a request is BLOCKED + governance_miss "
if printf 'This is a normal prose answer, not a task card.\n' \
    | cargo run -q -p ags-cli -- gate output - --for-request "给我提示词" --format json > /tmp/verify-gate-out-neg.log 2>&1; then
    echo "FAIL (non-canonical output was allowed — gate not fail-closed)"
    cat /tmp/verify-gate-out-neg.log
    failures=$((failures + 1))
elif grep -q '"block_reason": "bad_output_shape"' /tmp/verify-gate-out-neg.log \
    && grep -q '"event": "governance_miss"' /tmp/verify-gate-out-neg.log; then
    echo "OK (blocked, governance_miss emitted)"
else
    echo "FAIL (blocked, but missing bad_output_shape/governance_miss — fail closed)"
    cat /tmp/verify-gate-out-neg.log
    failures=$((failures + 1))
fi

# ── Change Lane Smoke Tests (diff-aware verification routing) ───────────────
# Build a throwaway git repo and verify `ags verify lane` classifies real diffs
# into the right lane + minimal-sufficient profile. Proves ignore-only takes the
# minimal path while source/protocol take heavier profiles — the basis the push
# gate relies on to avoid full/stable verification for hygiene changes.
echo "--- Change Lane Smoke Tests ---"
lane_tmp="$(mktemp -d)"
git -C "$lane_tmp" init -q
git -C "$lane_tmp" config user.email "verify@ags.test"
git -C "$lane_tmp" config user.name "ags-verify"
echo "init" > "$lane_tmp/README.md"
git -C "$lane_tmp" add -A && git -C "$lane_tmp" commit -q -m "base"

# ignore-only → ignore_only / minimal
ignore_base="$(git -C "$lane_tmp" rev-parse HEAD)"
printf 'target/\n' >> "$lane_tmp/.gitignore"
git -C "$lane_tmp" add -A && git -C "$lane_tmp" commit -q -m "ignore"
echo -n "[....] verify lane: .gitignore-only → ignore_only/minimal "
if cargo run -q -p ags-cli -- verify lane --range "${ignore_base}..HEAD" --target "$lane_tmp" --format json > /tmp/verify-lane-ignore.json 2>&1 \
    && grep -q '"lane": "ignore_only"' /tmp/verify-lane-ignore.json \
    && grep -q '"profile": "minimal"' /tmp/verify-lane-ignore.json; then
    echo "OK"
else
    echo "FAIL (expected ignore_only/minimal)"
    cat /tmp/verify-lane-ignore.json
    failures=$((failures + 1))
fi

# source-code → source_code / standard
src_base="$(git -C "$lane_tmp" rev-parse HEAD)"
mkdir -p "$lane_tmp/src"
echo "fn main() {}" > "$lane_tmp/src/main.rs"
git -C "$lane_tmp" add -A && git -C "$lane_tmp" commit -q -m "src"
echo -n "[....] verify lane: src/main.rs → source_code/standard "
if cargo run -q -p ags-cli -- verify lane --range "${src_base}..HEAD" --target "$lane_tmp" --format json > /tmp/verify-lane-src.json 2>&1 \
    && grep -q '"lane": "source_code"' /tmp/verify-lane-src.json \
    && grep -q '"profile": "standard"' /tmp/verify-lane-src.json; then
    echo "OK"
else
    echo "FAIL (expected source_code/standard)"
    cat /tmp/verify-lane-src.json
    failures=$((failures + 1))
fi

# protocol-core → protocol_core / full (mixed ignore+protocol escalates to full)
proto_base="$(git -C "$lane_tmp" rev-parse HEAD)"
mkdir -p "$lane_tmp/protocol"
echo "# proto" > "$lane_tmp/protocol/x.md"
printf 'extra/\n' >> "$lane_tmp/.gitignore"
git -C "$lane_tmp" add -A && git -C "$lane_tmp" commit -q -m "proto"
echo -n "[....] verify lane: protocol/ + .gitignore → protocol_core/full (escalates) "
if cargo run -q -p ags-cli -- verify lane --range "${proto_base}..HEAD" --target "$lane_tmp" --format json > /tmp/verify-lane-proto.json 2>&1 \
    && grep -q '"lane": "protocol_core"' /tmp/verify-lane-proto.json \
    && grep -q '"profile": "full"' /tmp/verify-lane-proto.json; then
    echo "OK"
else
    echo "FAIL (expected protocol_core/full)"
    cat /tmp/verify-lane-proto.json
    failures=$((failures + 1))
fi

# empty range (no diff) → ignore_only / minimal (safe no-op)
echo -n "[....] verify lane: empty range → ignore_only/minimal (no-op safe) "
empty_head="$(git -C "$lane_tmp" rev-parse HEAD)"
if cargo run -q -p ags-cli -- verify lane --range "${empty_head}..HEAD" --target "$lane_tmp" --format json > /tmp/verify-lane-empty.json 2>&1 \
    && grep -q '"lane": "ignore_only"' /tmp/verify-lane-empty.json \
    && grep -q '"profile": "minimal"' /tmp/verify-lane-empty.json; then
    echo "OK"
else
    echo "FAIL (empty diff must be ignore_only/minimal)"
    cat /tmp/verify-lane-empty.json
    failures=$((failures + 1))
fi

rm -rf "$lane_tmp"

# ── Push Lane Decision Smoke Tests (trusted shell allowlist) ────────────────
# scripts/lane-decision.sh is the push gate's MINIMAL/FULL decision, kept in
# pure shell so a change to the in-tree classifier cannot route a source or
# protocol diff onto the minimal path. These tests prove exactly that.
echo "--- Push Lane Decision Smoke Tests ---"
check_lane_decision() {
    local label="$1" files="$2" expected="$3"
    echo -n "[....] lane-decision: $label → $expected "
    local got
    got="$(printf '%s\n' "$files" | bash "$REPO_ROOT/scripts/lane-decision.sh" 2>/dev/null || echo ERR)"
    if [ "$got" = "$expected" ]; then
        echo "OK"
    else
        echo "FAIL (got '$got')"
        failures=$((failures + 1))
    fi
}
check_lane_decision ".gitignore-only" ".gitignore" "MINIMAL"
check_lane_decision "docs-only" $'docs/README.md\nCHANGELOG.md' "MINIMAL"
check_lane_decision "source-code" "crates/ags-cli/src/main.rs" "FULL"
check_lane_decision "protocol-core" "protocol/agent-task-protocol.md" "FULL"
check_lane_decision "scripts(gate-selection)" "scripts/lane-decision.sh" "FULL"
check_lane_decision "manifests" "manifests/suite.yaml" "FULL"
check_lane_decision "governance" "governance/skill-adoption-log.yaml" "FULL"
check_lane_decision "Cargo.toml" "Cargo.toml" "FULL"
check_lane_decision "root-entry CLAUDE.md" "CLAUDE.md" "FULL"
check_lane_decision "mixed ignore+source" $'.gitignore\ncrates/x/src/lib.rs' "FULL"
check_lane_decision "mixed ignore+protocol" $'.gitignore\nprotocol/x.md' "FULL"
check_lane_decision "empty(fail-safe)" "" "FULL"

# ── Resolver Hardening Smoke Tests (F1-F7) ──────────────────────────────────
echo "--- Resolver Hardening Smoke Tests (F1-F7) ---"

# F5: JSON uses canonical protocol values, not Rust variant names
echo -n "[....] JSON permission_mode uses canonical values "
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format json > /tmp/verify-json-canonical.log 2>&1; then
    json=$(cat /tmp/verify-json-canonical.log)
    if echo "$json" | grep -q '"ReadOnly"\|"PlanOnly"\|"ExecuteAndVerify"\|"EditWithConfirmation"'; then
        echo "FAIL (Rust variant names found in JSON)"
        echo "$json" | head -20
        failures=$((failures + 1))
    else
        echo "OK"
    fi
else
    echo "FAIL (resolve-policy failed)"
    failures=$((failures + 1))
fi

# F1: read-only + worktree must NOT output --parallel / --worktree
echo -n "[....] read-only+worktree must not output --parallel "
write_smoke_card /tmp/test-readonly-worktree.md cli read-only worktree plan-only Light \
    "Test read-only + worktree gate." \
    "Verify --parallel is not output." \
    "None." \
    "any failure" \
    "test passes" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-readonly-worktree.md --format json > /tmp/verify-ro-wt.log 2>&1; then
    if grep -q -- '--parallel\|--worktree' /tmp/verify-ro-wt.log; then
        echo "FAIL (--parallel or --worktree found in read-only output)"
        grep -- '--parallel\|--worktree' /tmp/verify-ro-wt.log
        failures=$((failures + 1))
    else
        echo "OK"
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-ro-wt.log
    failures=$((failures + 1))
fi

# F1: plan-only + worktree must NOT output --parallel
echo -n "[....] plan-only+worktree must not output --parallel "
write_smoke_card /tmp/test-plan-only-worktree.md cli plan-only worktree plan-only Medium \
    "Test plan-only + worktree gate." \
    "Verify --parallel is not output in plan-only." \
    "None." \
    "any failure" \
    "no --parallel" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-plan-only-worktree.md --format json > /tmp/verify-po-wt.log 2>&1; then
    if grep -q -- '--parallel\|--worktree' /tmp/verify-po-wt.log; then
        echo "FAIL (--parallel or --worktree found in plan-only output)"
        grep -- '--parallel\|--worktree' /tmp/verify-po-wt.log
        failures=$((failures + 1))
    else
        echo "OK"
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-po-wt.log
    failures=$((failures + 1))
fi

# F4: --approve-writes flag preserves Heavy write mode
echo -n "[....] Heavy + --approve-writes preserves write mode "
write_smoke_card /tmp/test-heavy-approve.md cli edit-with-confirmation none none Heavy \
    "Test Heavy with approval." \
    "Verify Heavy + --approve-writes keeps edit-with-confirmation." \
    "None." \
    "any failure" \
    "write mode preserved" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-heavy-approve.md --format json --approve-writes > /tmp/verify-heavy-approve.log 2>&1; then
    json=$(cat /tmp/verify-heavy-approve.log)
    if echo "$json" | grep -q '"edit-with-confirmation"'; then
        if echo "$json" | grep -q '"cli-flag"'; then
            echo "OK"
        else
            echo "FAIL (approval_source must be cli-flag)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (expected edit-with-confirmation, got downgraded)"
        echo "$json" | head -5
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-heavy-approve.log
    failures=$((failures + 1))
fi

# F6: background-agent execution surface passes validator
echo -n "[....] background-agent execution surface passes "
write_smoke_card /tmp/test-bg-agent.md background-agent execute-and-verify none none Light \
    "Test background-agent surface." \
    "Verify background-agent passes validation." \
    "None." \
    "any failure" \
    "surface accepted" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-bg-agent.md --format json > /tmp/verify-bg-agent.log 2>&1; then
    json=$(cat /tmp/verify-bg-agent.log)
    if echo "$json" | grep -q '"--headless"'; then
        echo "OK"
    else
        echo "FAIL (background-agent must produce --headless)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (background-agent rejected by validator)"
    cat /tmp/verify-bg-agent.log
    failures=$((failures + 1))
fi

# ── Resolver Hardening Smoke Tests (F8-F9: stop and parallelism audit) ──────
echo ""
echo "--- Resolver Hardening Smoke Tests (F8-F9: stop and parallelism audit) ---"

# F8: Heavy edit-with-confirmation without --approve-writes → stop_before_launch
echo -n "[....] Heavy edit-with-confirmation without approve → stop_before_launch "
write_smoke_card /tmp/test-heavy-stop.md cli edit-with-confirmation none none Heavy \
    "Test Heavy stop gate." \
    "Verify Heavy without --approve-writes sets stop_before_launch=true." \
    "None." \
    "any failure" \
    "stop_before_launch=true" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-heavy-stop.md --format json > /tmp/verify-heavy-stop.log 2>&1; then
    json=$(cat /tmp/verify-heavy-stop.log)
    if echo "$json" | grep -q '"stop_before_launch": true'; then
        if echo "$json" | grep -q '"heavy-requires-write-approval"'; then
            echo "OK"
        else
            echo "FAIL (stop_reasons kind must be heavy-requires-write-approval)"
            echo "$json" | grep -o '"stop_reasons":[^]]*]'
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (stop_before_launch must be true for Heavy write without approval)"
        echo "$json" | grep -o '"stop_before_launch":[^,]*'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-heavy-stop.log
    failures=$((failures + 1))
fi

# F8: Heavy plan-only without --approve-writes → no stop (it's a plan card)
echo -n "[....] Heavy plan-only without approve → no stop "
write_smoke_card /tmp/test-heavy-plan.md cli plan-only none none Heavy \
    "Test Heavy plan-only gate." \
    "Verify Heavy plan-only does NOT trigger stop_before_launch." \
    "不执行写操作。" \
    "方案完成后返回用户审阅，等待明确批准" \
    "no stop" \
    "返回审计方案供 Codex review，等待明确批准。"
if cargo run -q -p ags-cli -- policy resolve /tmp/test-heavy-plan.md --format json > /tmp/verify-heavy-plan.log 2>&1; then
    json=$(cat /tmp/verify-heavy-plan.log)
    if echo "$json" | grep -q '"stop_before_launch": false'; then
        if echo "$json" | grep -q '"requires_confirmation_gate": true'; then
            echo "OK"
        else
            echo "FAIL (Heavy plan-only must still have requires_confirmation_gate)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (Heavy plan-only must NOT stop — it is a plan/review card)"
        echo "$json" | grep -o '"stop_before_launch":[^,]*'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-heavy-plan.log
    failures=$((failures + 1))
fi

# F9: plan-only + worktree → effective_parallelism is "none" in JSON
echo -n "[....] plan-only+worktree → effective_parallelism=none "
write_smoke_card /tmp/test-po-wt-none.md cli plan-only worktree plan-only Medium \
    "Test effective_parallelism consistency." \
    "Verify effective_parallelism is none when plan-only strips worktree." \
    "None." \
    "any failure" \
    "effective_parallelism is none" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-po-wt-none.md --format json > /tmp/verify-po-wt-none.log 2>&1; then
    json=$(cat /tmp/verify-po-wt-none.log)
    if echo "$json" | grep -q '"effective_parallelism": "none"'; then
        if echo "$json" | grep -q -- '--parallel\|--worktree'; then
            echo "FAIL (launch args must not contain --parallel or --worktree)"
            failures=$((failures + 1))
        else
            echo "OK"
        fi
    else
        echo "FAIL (effective_parallelism must be 'none' when plan-only strips worktree)"
        echo "$json" | grep -o '"effective_parallelism":"[^"]*"'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-po-wt-none.log
    failures=$((failures + 1))
fi

# F9: read-only + worktree → effective_parallelism is "none" in JSON
echo -n "[....] read-only+worktree → effective_parallelism=none "
write_smoke_card /tmp/test-ro-wt-none.md cli read-only worktree plan-only Light \
    "Test read-only effective_parallelism." \
    "Verify effective_parallelism=none when read-only strips worktree." \
    "None." \
    "any failure" \
    "effective_parallelism=none" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-ro-wt-none.md --format json > /tmp/verify-ro-wt-none.log 2>&1; then
    json=$(cat /tmp/verify-ro-wt-none.log)
    if echo "$json" | grep -q '"effective_parallelism": "none"'; then
        echo "OK"
    else
        echo "FAIL (effective_parallelism must be 'none' when read-only strips worktree)"
        echo "$json" | grep -o '"effective_parallelism":"[^"]*"'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-ro-wt-none.log
    failures=$((failures + 1))
fi

# ── Resolver Hardening Smoke Tests (F10: background-agent audit) ────────────
echo "--- Resolver Hardening Smoke Tests (F10: background-agent audit) ---"

# F10: background-agent + read-only → stop_before_launch + stop_reasons + downgrade
echo -n "[....] background-agent+read-only → stop and audit "
write_smoke_card /tmp/test-bg-ro-audit.md background-agent read-only none none Light \
    "Test background-agent audit trail." \
    "Verify background-agent+read-only sets stop_before_launch." \
    "不执行写操作。" \
    "any failure" \
    "stop_before_launch=true" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-bg-ro-audit.md --format json > /tmp/verify-bg-ro-audit.log 2>&1; then
    json=$(cat /tmp/verify-bg-ro-audit.log)
    if echo "$json" | grep -q '"stop_before_launch": true'; then
        if echo "$json" | grep -q '"background-surface-blocked-by-permission"'; then
            if echo "$json" | grep -q '"field": "execution_surface"'; then
                if echo "$json" | grep -q '"before": "background-agent"'; then
                    echo "OK"
                else
                    echo "FAIL (downgrade before must be background-agent)"
                    failures=$((failures + 1))
                fi
            else
                echo "FAIL (must have execution_surface downgrade)"
                failures=$((failures + 1))
            fi
        else
            echo "FAIL (stop_reasons kind must be background-surface-blocked-by-permission)"
            echo "$json" | grep -o '"stop_reasons":[^]]*]'
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (stop_before_launch must be true for read-only + background-agent)"
        echo "$json" | grep -o '"stop_before_launch":[^,]*'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-bg-ro-audit.log
    failures=$((failures + 1))
fi

# F10: background-agent + plan-only → stop_before_launch
echo -n "[....] background-agent+plan-only → stop and audit "
write_smoke_card /tmp/test-bg-po-audit.md background-agent plan-only none none Medium \
    "Test background-agent + plan-only audit." \
    "Verify plan-only + background-agent sets stop_before_launch." \
    "不执行写操作。" \
    "any failure" \
    "stop_before_launch=true" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-bg-po-audit.md --format json > /tmp/verify-bg-po-audit.log 2>&1; then
    json=$(cat /tmp/verify-bg-po-audit.log)
    if echo "$json" | grep -q '"stop_before_launch": true'; then
        if echo "$json" | grep -q '"background-surface-blocked-by-permission"'; then
            if echo "$json" | grep -q -- '--headless'; then
                echo "FAIL (plan-only must not produce --headless for background-agent)"
                failures=$((failures + 1))
            else
                echo "OK"
            fi
        else
            echo "FAIL (stop_reasons kind must be background-surface-blocked-by-permission)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (stop_before_launch must be true for plan-only + background-agent)"
        echo "$json" | grep -o '"stop_before_launch":[^,]*'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-bg-po-audit.log
    failures=$((failures + 1))
fi

# F10: background-agent + execute-and-verify → still allows --headless
echo -n "[....] background-agent+execute-and-verify → still --headless "
write_smoke_card /tmp/test-bg-ev-ok.md background-agent execute-and-verify none none Light \
    "Test background-agent + execute-and-verify still works." \
    "Verify execute-and-verify allows --headless." \
    "无。" \
    "any failure" \
    "--headless present" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-bg-ev-ok.md --format json > /tmp/verify-bg-ev-ok.log 2>&1; then
    json=$(cat /tmp/verify-bg-ev-ok.log)
    if echo "$json" | grep -q '"stop_before_launch": false'; then
        if echo "$json" | grep -q '"--headless"'; then
            echo "OK"
        else
            echo "FAIL (execute-and-verify must still produce --headless for background-agent)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (execute-and-verify + background-agent must not stop)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-bg-ev-ok.log
    failures=$((failures + 1))
fi

# F10/P3: combined worktree + background-agent must preserve BOTH stop reasons
echo -n "[....] worktree+background-agent → two stop reasons "
write_smoke_card /tmp/test-combined-stop-reasons.md background-agent read-only worktree plan-only Light \
    "Test combined stop reasons." \
    "Verify combined blocked worktree and background-agent preserve both stop reasons." \
    "不执行写操作。" \
    "any failure" \
    "two stop reasons" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-combined-stop-reasons.md --format json > /tmp/verify-combined-stop-reasons.log 2>&1; then
    json=$(cat /tmp/verify-combined-stop-reasons.log)
    if ! echo "$json" | grep -q '"stop_reasons": \['; then
        echo "FAIL (stop_reasons must be an array)"
        failures=$((failures + 1))
    elif ! echo "$json" | grep -q '"writable-parallelism-blocked-by-permission"'; then
        echo "FAIL (missing parallelism stop reason)"
        failures=$((failures + 1))
    elif ! echo "$json" | grep -q '"background-surface-blocked-by-permission"'; then
        echo "FAIL (missing background surface stop reason)"
        failures=$((failures + 1))
    elif echo "$json" | grep -q '"stop_reason":'; then
        echo "FAIL (legacy singular stop_reason field must not be emitted)"
        failures=$((failures + 1))
    elif ! echo "$json" | grep -q '"allowed_launch_args": \[\]'; then
        echo "FAIL (stop_before_launch policies must expose allowed_launch_args: [])"
        echo "$json" | grep -A4 '"allowed_launch_args"'
        failures=$((failures + 1))
    else
        echo "OK"
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-combined-stop-reasons.log
    failures=$((failures + 1))
fi

# F2-doc: runtime-adapters.md must NOT teach runner to read raw fields directly
echo -n "[....] runtime-adapters.md no raw-field→direct-flag pattern "
PROTOCOL="$REPO_ROOT/protocol/runtime-adapters.md"
if grep -q 'Parallelism: subagent.*enable.*--parallel' "$PROTOCOL"; then
    echo "FAIL (raw Parallelism→--parallel mapping found — must route through resolver)"
    failures=$((failures + 1))
elif grep -q 'Parallelism: worktree.*enable.*--parallel --worktree' "$PROTOCOL"; then
    echo "FAIL (raw Parallelism→--worktree mapping found — must route through resolver)"
    failures=$((failures + 1))
elif grep -q 'Execution surface: background-agent.*enable.*--headless' "$PROTOCOL"; then
    echo "FAIL (raw Execution surface→--headless mapping found — must route through resolver)"
    failures=$((failures + 1))
elif ! grep -q 'allowed_launch_args' "$PROTOCOL"; then
    echo "FAIL (runner auto-mode must reference allowed_launch_args from resolver)"
    failures=$((failures + 1))
elif ! grep -q 'effective_permission_mode' "$PROTOCOL"; then
    echo "FAIL (runner auto-mode must reference effective_permission_mode from resolver)"
    failures=$((failures + 1))
else
    echo "OK"
fi

# ── M3-M8 Smoke Tests (collapsed — primary verification is now ags verify) ──
echo ""
echo "--- M3-M8 Integrated Smoke Tests ---"

# M3 Gate/Policy Engine
echo -n "[....] gate check valid light → decision=allow "
if cargo run -q -p ags-cli -- gate check "$REPO_ROOT/tests/fixtures/valid-full.md" --format json > /tmp/verify-m3-gate-light.log 2>&1; then
    if grep -q '"decision": "allow"' /tmp/verify-m3-gate-light.log && \
       grep -q '"resolved_policy"' /tmp/verify-m3-gate-light.log; then
        echo "OK"
    else
        echo "FAIL"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (gate check failed)"
    failures=$((failures + 1))
fi

echo -n "[....] gate check Heavy write → decision=stop (exit 1) "
set +e
cargo run -q -p ags-cli -- gate check /tmp/test-heavy-stop.md --format json > /tmp/verify-m3-gate-stop.log 2>&1
rc=$?
set -e
if [ "$rc" -eq 1 ] && grep -q '"decision": "stop"' /tmp/verify-m3-gate-stop.log; then
    echo "OK"
else
    echo "FAIL (expected exit 1 + decision=stop, got $rc)"
    failures=$((failures + 1))
fi

# M5 Capability
echo -n "[....] capability list --format json "
if cargo run -q -p ags-cli -- capability list --format json > /tmp/verify-m5-list.json 2>&1; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

# M6 Receipt
echo -n "[....] receipt verify valid fixture -> exit 0 "
if cargo run -q -p ags-cli -- receipt verify "$REPO_ROOT/tests/fixtures/receipt-valid.json" --format json > /tmp/verify-m6-verify.json 2>&1; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

# M7 Bootstrap
echo -n "[....] bootstrap --dry-run "
if cargo run -q -p ags-cli -- bootstrap --dry-run --format text > /tmp/verify-m7-bs.log 2>&1; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

# M8 Dogfood — run-task-card.sh --check-only
echo -n "[....] run-task-card.sh --check-only valid light → ALLOW "
if "$REPO_ROOT/scripts/run-task-card.sh" "$REPO_ROOT/tests/fixtures/valid-full.md" --check-only --format json > /tmp/verify-m8-run.log 2>&1; then
    if python3 -c "import json,sys; data=json.load(sys.stdin); assert data['gate_decision'] == 'allow'" < /tmp/verify-m8-run.log; then
        echo "OK"
    else
        echo "FAIL"
        cat /tmp/verify-m8-run.log
        failures=$((failures + 1))
    fi
else
    echo "FAIL"
    cat /tmp/verify-m8-run.log
    failures=$((failures + 1))
fi

# ── Session Preflight Smoke ─────────────────────────────────────────────────
echo -n "[....] session preflight --for claude-code --format json --target repo "
if cargo run -q -p ags-cli -- session preflight --for claude-code --format json --target "$REPO_ROOT" > /tmp/verify-sp-claude.json 2>&1; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

# Tencent Agent host clients resolve and brand correctly (recognized-host table;
# generic fallback for unknown hosts is preserved separately).
echo -n "[....] session preflight --for workbuddy → Tencent Agent (WorkBuddy) "
if cargo run -q -p ags-cli -- session preflight --for workbuddy --format json --target "$REPO_ROOT" > /tmp/verify-sp-wb.json 2>&1 \
    && grep -q 'Tencent Agent (WorkBuddy)' /tmp/verify-sp-wb.json; then
    echo "OK"
else
    echo "FAIL (workbuddy must brand as Tencent Agent (WorkBuddy))"
    cat /tmp/verify-sp-wb.json
    failures=$((failures + 1))
fi
echo -n "[....] session preflight --for CodeBuddy-Code → Tencent Agent (CodeBuddy-Code) "
if cargo run -q -p ags-cli -- session preflight --for CodeBuddy-Code --format json --target "$REPO_ROOT" > /tmp/verify-sp-cb.json 2>&1 \
    && grep -q 'Tencent Agent (CodeBuddy-Code)' /tmp/verify-sp-cb.json; then
    echo "OK"
else
    echo "FAIL (CodeBuddy-Code must brand as Tencent Agent (CodeBuddy-Code))"
    cat /tmp/verify-sp-cb.json
    failures=$((failures + 1))
fi
echo -n "[....] unknown host keeps generic fallback (not broken) "
if cargo run -q -p ags-cli -- session preflight --for some-unknown-host --format json --target "$REPO_ROOT" > /tmp/verify-sp-unknown.json 2>&1 \
    && grep -q 'Generic Agent (some-unknown-host)' /tmp/verify-sp-unknown.json; then
    echo "OK"
else
    echo "FAIL (unknown host must keep Generic Agent fallback)"
    cat /tmp/verify-sp-unknown.json
    failures=$((failures + 1))
fi

# ── Skill & MCP Console Smoke Tests ─────────────────────────────────────────
echo ""
echo "--- Skill & MCP Console Smoke Tests ---"
run_check "skill overview (json)" \
    cargo run -q -p ags-cli -- skill --format json
run_check "skill scan (json)" \
    cargo run -q -p ags-cli -- skill scan --format json
run_check "skill check (json)" \
    cargo run -q -p ags-cli -- skill check --format json
run_check "skill verify --host claude-code (json)" \
    cargo run -q -p ags-cli -- skill verify --host claude-code --format json

# Unified inventory: four kinds, AGS suite-interface, BOTH hosts, canonical field.
# Route-target rows are metadata-only internal entrypoints and intentionally have
# no host visibility evidence.
echo -n "[....] skill inventory: 4 kinds + both hosts + canonical "
if cargo run -q -p ags-cli -- skill --format json > /tmp/verify-skill-inv.json 2>&1; then
    if python3 - <<'PY'
import json

d = json.load(open('/tmp/verify-skill-inv.json'))['inventory']
ks = {c['kind'] for c in d['capabilities']}
assert {'skill', 'mcp', 'suite-interface', 'cli-backed'} <= ks, ks
assert next(c for c in d['capabilities'] if c['name'] == 'ags')['kind'] == 'suite-interface'
assert set(d['hosts']) >= {'claude-code', 'codex'}, d['hosts']
assert isinstance(d['summary']['canonical_present'], int)
assert all(
    c.get('managed_status') == 'route-target'
    or any(v['host'] == 'codex' for v in c['host_visibility'])
    for c in d['capabilities']
)
PY
    then
        echo "OK"
    else
        echo "FAIL (missing a kind / AGS not suite-interface / missing host / no canonical)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill overview failed)"
    failures=$((failures + 1))
fi

# Dry-run proposal in the public edition: optional skills are recommendations
# with URL sources, not bundled canonical bodies. AGS must not write anything or
# create dangling host thin indexes from a remote URL; the proposal is safely
# blocked until a concrete local source exists.
echo -n "[....] skill propose adopt diagnose (public recommendation) is blocked/no-write "
set +e
cargo run -q -p ags-cli -- skill propose --action adopt --skill diagnose --format json > /tmp/verify-skill-propose.json 2>&1
skill_propose_rc=$?
set -e
if python3 -c "import json; d=json.load(open('/tmp/verify-skill-propose.json')); assert d['found'] is True; assert d['apply_requested'] is False; assert d['applied'] is False; assert d['apply_status']=='dry-run'; assert d['planned_writes'] == []; assert d['applied_writes'] == []; assert d['apply_errors'] == []; assert d['blocked_reasons'], d" \
    && [ "$skill_propose_rc" -eq 1 ]; then
    echo "OK"
else
    echo "FAIL (public recommendation dry-run must exit 1, be blocked, and write nothing; got rc=$skill_propose_rc)"
    cat /tmp/verify-skill-propose.json
    failures=$((failures + 1))
fi

# MCP --apply is advised-only: AGS performs nothing (no writes), exits nonzero.
echo -n "[....] skill propose adopt MCP --apply → advised-only, exit 1, no writes "
set +e
cargo run -q -p ags-cli -- skill propose --action adopt --skill context7 --apply --format json > /tmp/verify-skill-mcp-apply.json 2>&1
mcp_rc=$?
set -e
if python3 -c "import json; d=json.load(open('/tmp/verify-skill-mcp-apply.json')); assert d['applied'] is False; assert d['apply_status']=='advised-only'; assert d['applied_writes']==[]; assert d['planned_writes']==[]" && [ "$mcp_rc" -eq 1 ]; then
    echo "OK"
else
    echo "FAIL (MCP --apply must be advised-only, exit 1, write nothing; got rc=$mcp_rc)"
    failures=$((failures + 1))
fi

# Verify exposes the failure-aware fields (all_visible / expected / failed).
echo -n "[....] skill verify exposes all_visible/expected/failed "
if cargo run -q -p ags-cli -- skill verify --host claude-code --format json > /tmp/verify-skill-verify.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-skill-verify.json')); s=d['summary']; assert isinstance(s['all_visible'], bool); assert 'expected' in s and 'failed' in s; assert d['status'] in ('ok','degraded','incomplete')"; then
        echo "OK"
    else
        echo "FAIL (verify must expose all_visible/expected/failed + valid status)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill verify failed)"
    failures=$((failures + 1))
fi

# Codex is a real (supported) host: skill path + `codex mcp list`.
echo -n "[....] skill verify --host codex → supported (real checks) "
if cargo run -q -p ags-cli -- skill verify --host codex --format json > /tmp/verify-skill-codex.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-skill-codex.json')); assert d['supported'] is True; assert d['status'] in ('ok','degraded','incomplete')"; then
        echo "OK"
    else
        echo "FAIL (codex must be a supported host with a valid status)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill verify codex failed)"
    failures=$((failures + 1))
fi

# Cursor is the reserved host: stable unsupported fields (no panic, no probing).
echo -n "[....] skill verify --host cursor → unsupported (stable fields) "
if cargo run -q -p ags-cli -- skill verify --host cursor --format json > /tmp/verify-skill-cursor.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-skill-cursor.json')); assert d['supported'] is False; assert d['status'] == 'unsupported'; assert d['checks'] == []"; then
        echo "OK"
    else
        echo "FAIL (cursor must be unsupported with stable fields)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill verify cursor failed)"
    failures=$((failures + 1))
fi

# ── Lifecycle Semantics Regression ──────────────────────────────────────────
echo ""
echo "--- Lifecycle Semantics Regression ---"
echo -n "[....] task-routing.md has no classify-first language "
if grep -q 'Classify the task first' "$REPO_ROOT/protocol/task-routing.md"; then
    echo "FAIL"
    failures=$((failures + 1))
else
    echo "OK"
fi
echo -n "[....] agent-task-protocol.md has ambient preflight phase "
if grep -q 'Ambient Preflight\|ambient preflight' "$REPO_ROOT/protocol/agent-task-protocol.md"; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi
echo -n "[....] agent-task-protocol.md has Value Route (效价比) section "
if grep -q 'Value Route' "$REPO_ROOT/protocol/agent-task-protocol.md" \
    && grep -q '效价比' "$REPO_ROOT/protocol/agent-task-protocol.md"; then
    echo "OK"
else
    echo "FAIL (Value Route / 效价比 missing from agent-task-protocol.md)"
    failures=$((failures + 1))
fi

# ── Final result ────────────────────────────────────────────────────────────
echo ""
if [ "$failures" -eq 0 ]; then
    echo "=== All checks passed ==="
else
    echo "=== $failures check(s) FAILED ==="
    exit 1
fi
