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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

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
if printf '%s' "$compile_intent" | cargo run -q -p ags-cli -- task compile - --task-card-requested --confirmed-handoff-contract --output card > /tmp/verify-compiled-card.md 2>/dev/null \
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

# ── Typed Request Governance / Output Gate Smoke ────────────────────────────
echo "--- Typed Request Governance / Output Gate Smoke ---"

echo -n "[....] typed request governance contract tests "
if cargo test -q -p request-governance; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

echo -n "[....] deterministic skill resolver contract tests "
if cargo test -q -p skill-resolver --test skill_resolver; then
    echo "OK"
else
    echo "FAIL"
    failures=$((failures + 1))
fi

# Positive end-to-end: confirmed structured contract → canonical card → output ALLOW.
echo -n "[....] confirmed contract → compile → gate output ALLOW + validator-clean "
handoff_contract=$'任务：verify.sh typed handoff e2e smoke\n目标：确认结构化交接契约经编译产出经典骨架并通过 output gate'
if printf '%s' "$handoff_contract" | cargo run -q -p ags-cli -- task compile - --task-card-requested --confirmed-handoff-contract --output card > /tmp/verify-gate-card.md 2>/dev/null \
    && cargo run -q -p ags-cli -- gate output /tmp/verify-gate-card.md --format json > /tmp/verify-gate-out-pos.log 2>&1 \
    && grep -q '"decision": "allow"' /tmp/verify-gate-out-pos.log \
    && cargo run -q -p ags-cli -- task validate /tmp/verify-gate-card.md > /dev/null 2>&1; then
    echo "OK"
else
    echo "FAIL (canonical card rejected by output gate or validator)"
    cat /tmp/verify-gate-out-pos.log
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

# ── Release/Sync Lane Decision Smoke Tests (trusted shell allowlist) ────────
# scripts/lane-decision.sh is the release/sync MINIMAL/FULL decision, kept in
# pure shell so a change to the in-tree classifier cannot route a source or
# protocol diff onto the minimal path. These tests prove exactly that.
echo "--- Release/Sync Lane Decision Smoke Tests ---"
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

echo -n "[....] retired task-card permission/scheduler semantics are absent "
retired_permission_pattern='edit-with-''confirmation|autonomous-low-''risk|requires_''confirmation_gate|GateDecision::''Confirm|PermissionMode::''ReadOnly|Permission mode: read-''only|resolver adds a ''confirmation gate'
if command -v rg >/dev/null 2>&1; then
    deprecated_permission_hits="$(rg -n \
        --glob '!target/**' \
        --glob '!graphify-out/**' \
        --glob '!scripts/verify.sh' \
        "$retired_permission_pattern" \
        "$REPO_ROOT" || true)"
else
    deprecated_permission_hits="$(git -C "$REPO_ROOT" grep -n -E \
        "$retired_permission_pattern" \
        -- . ':(exclude)scripts/verify.sh' || true)"
fi
if [ -z "$deprecated_permission_hits" ]; then
    echo "OK"
else
    echo "FAIL (retired permission or scheduler semantics found)"
    echo "$deprecated_permission_hits"
    failures=$((failures + 1))
fi

# F5: JSON uses canonical protocol values, not Rust variant names
echo -n "[....] JSON permission_mode uses canonical values "
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format json > /tmp/verify-json-canonical.log 2>&1; then
    json=$(cat /tmp/verify-json-canonical.log)
    if echo "$json" | grep -q '"PlanOnly"\|"ExecuteAndVerify"'; then
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

# F4: --approve-writes is audit-only for a Heavy direct-execution card
echo -n "[....] Heavy direct execution + --approve-writes preserves mode "
write_smoke_card /tmp/test-heavy-approve.md cli execute-and-verify none none Heavy \
    "Test Heavy with approval." \
    "Verify Heavy + --approve-writes keeps execute-and-verify." \
    "None." \
    "any failure" \
    "direct mode preserved" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-heavy-approve.md --format json --approve-writes > /tmp/verify-heavy-approve.log 2>&1; then
    json=$(cat /tmp/verify-heavy-approve.log)
    if echo "$json" | grep -q '"execute-and-verify"'; then
        if echo "$json" | grep -q '"cli-flag"'; then
            echo "OK"
        else
            echo "FAIL (approval_source must be cli-flag)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (expected execute-and-verify, got downgraded)"
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

# F8: Medium defaults to direct execution; no intermediate permission mode exists.
echo -n "[....] Medium execute-and-verify → direct execution (no stop) "
write_smoke_card /tmp/test-medium-execute.md cli execute-and-verify none none Medium \
    "Test Medium direct execution." \
    "Verify execute-and-verify is preserved without an intermediate gate." \
    "None." \
    "any failure" \
    "execute-and-verify preserved" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-medium-execute.md --format json > /tmp/verify-medium-execute.log 2>&1; then
    json=$(cat /tmp/verify-medium-execute.log)
    if echo "$json" | grep -q '"effective_permission_mode": "execute-and-verify"' \
        && echo "$json" | grep -q '"stop_before_launch": false' \
        && echo "$json" | grep -q '"was_downgraded": false'; then
        echo "OK"
    else
        echo "FAIL (Medium execute-and-verify must run directly without downgrade/stop)"
        echo "$json" | head -20
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-medium-execute.log
    failures=$((failures + 1))
fi

# F8b: --current-task-approval is recorded as an audit signal; direct mode unchanged
echo -n "[....] Medium direct + current-task approval: mode preserved, signal recorded "
if cargo run -q -p ags-cli -- policy resolve /tmp/test-medium-execute.md --format json --current-task-approval > /tmp/verify-medium-current.log 2>&1; then
    json=$(cat /tmp/verify-medium-current.log)
    if echo "$json" | grep -q '"effective_permission_mode": "execute-and-verify"' \
        && echo "$json" | grep -q '"stop_before_launch": false' \
        && echo "$json" | grep -q '"approval_source": "current-task-instruction"'; then
        echo "OK"
    else
        echo "FAIL (current-task approval must preserve Medium direct execution)"
        echo "$json" | head -20
        failures=$((failures + 1))
    fi
else
    echo "FAIL (policy resolve --current-task-approval failed)"
    cat /tmp/verify-medium-current.log
    failures=$((failures + 1))
fi

# F8c: ags run forwards --current-task-approval into the resolver
echo -n "[....] ags run forwards current-task approval "
if cargo run -q -p ags-cli -- run /tmp/test-medium-execute.md --dry-run --current-task-approval --format json > /tmp/verify-run-current.log 2>&1; then
    json=$(cat /tmp/verify-run-current.log)
    if echo "$json" | grep -q '"effective_permission_mode": "execute-and-verify"' \
        && echo "$json" | grep -q '"approval_source": "current-task-instruction"'; then
        echo "OK"
    else
        echo "FAIL (ags run must forward current-task approval)"
        echo "$json" | head -30
        failures=$((failures + 1))
    fi
else
    echo "FAIL (ags run --current-task-approval failed)"
    cat /tmp/verify-run-current.log
    failures=$((failures + 1))
fi

# F8: Heavy plan-only without --approve-writes → planning is allowed, writes remain forbidden.
echo -n "[....] Heavy plan-only without approve → planning allowed "
write_smoke_card /tmp/test-heavy-plan.md cli plan-only none none Heavy \
    "Test Heavy plan-only gate." \
    "Verify Heavy plan-only can produce a plan without launching writes." \
    "不执行写操作。" \
    "方案完成后返回用户审阅，等待明确批准" \
    "no stop" \
    "返回审计方案供 Codex review，等待明确批准。"
if cargo run -q -p ags-cli -- policy resolve /tmp/test-heavy-plan.md --format json > /tmp/verify-heavy-plan.log 2>&1; then
    json=$(cat /tmp/verify-heavy-plan.log)
    if echo "$json" | grep -q '"stop_before_launch": false'; then
        echo "OK"
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

# ── Resolver Hardening Smoke Tests (F10: background-agent audit) ────────────
echo "--- Resolver Hardening Smoke Tests (F10: background-agent audit) ---"

# F10: background-agent + plan-only → stop_before_launch + stop_reasons + downgrade
echo -n "[....] background-agent+plan-only → stop and audit "
write_smoke_card /tmp/test-bg-plan-audit.md background-agent plan-only none none Light \
    "Test background-agent audit trail." \
    "Verify background-agent+plan-only sets stop_before_launch." \
    "不执行写操作。" \
    "any failure" \
    "stop_before_launch=true" \
    "Delivery report."
if cargo run -q -p ags-cli -- policy resolve /tmp/test-bg-plan-audit.md --format json > /tmp/verify-bg-plan-audit.log 2>&1; then
    json=$(cat /tmp/verify-bg-plan-audit.log)
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
        echo "FAIL (stop_before_launch must be true for plan-only + background-agent)"
        echo "$json" | grep -o '"stop_before_launch":[^,]*'
        failures=$((failures + 1))
    fi
else
    echo "FAIL (resolve-policy failed)"
    cat /tmp/verify-bg-plan-audit.log
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
write_smoke_card /tmp/test-combined-stop-reasons.md background-agent plan-only worktree plan-only Light \
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
    echo "FAIL (runner LaunchPlan must reference allowed_launch_args from resolver)"
    failures=$((failures + 1))
elif ! grep -q 'effective_permission_mode' "$PROTOCOL"; then
    echo "FAIL (runner LaunchPlan must reference effective_permission_mode from resolver)"
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

# Genuine resolver STOP: plan-only + worktree (writability gate). Task level
# never stops, so Heavy alone is no longer a stop case.
echo -n "[....] gate check writability stop (plan-only + worktree) → decision=stop (exit 1) "
write_smoke_card /tmp/test-gate-stop.md cli plan-only worktree plan-only Medium \
    "Test writability stop gate." \
    "Verify plan-only + worktree triggers stop_before_launch." \
    "不执行写操作。" \
    "方案完成后返回用户审阅，等待明确批准" \
    "stop" \
    "返回审计方案供 Codex review，等待明确批准。"
set +e
cargo run -q -p ags-cli -- gate check /tmp/test-gate-stop.md --format json > /tmp/verify-m3-gate-stop.log 2>&1
rc=$?
set -e
if [ "$rc" -eq 1 ] && grep -q '"decision": "stop"' /tmp/verify-m3-gate-stop.log; then
    echo "OK"
else
    echo "FAIL (expected exit 1 + decision=stop, got $rc)"
    cat /tmp/verify-m3-gate-stop.log
    failures=$((failures + 1))
fi

# Heavy + execute-and-verify is a direct execution card: decision=allow (exit 0),
# no stop and no downgrade. This is the headline two-mode regression.
echo -n "[....] gate check Heavy execute-and-verify → decision=allow (exit 0) "
write_smoke_card /tmp/test-gate-heavy-exec.md cli execute-and-verify none none Heavy \
    "Test Heavy execute-and-verify direct execution." \
    "Verify Heavy execute-and-verify resolves to allow and runs directly." \
    "None." \
    "any failure" \
    "execute-and-verify runs directly" \
    "Delivery report."
set +e
cargo run -q -p ags-cli -- gate check /tmp/test-gate-heavy-exec.md --format json > /tmp/verify-m3-gate-heavy-exec.log 2>&1
rc=$?
set -e
if [ "$rc" -eq 0 ] \
    && grep -q '"decision": "allow"' /tmp/verify-m3-gate-heavy-exec.log \
    && grep -q '"effective_permission_mode": "execute-and-verify"' /tmp/verify-m3-gate-heavy-exec.log \
    && grep -q '"stop_before_launch": false' /tmp/verify-m3-gate-heavy-exec.log \
    && grep -q '"was_downgraded": false' /tmp/verify-m3-gate-heavy-exec.log; then
    echo "OK"
else
    echo "FAIL (Heavy execute-and-verify must be allow/exit0, no stop, no downgrade; got rc=$rc)"
    cat /tmp/verify-m3-gate-heavy-exec.log
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

# Cancellation safety: since the wrapper no longer `exec`s, killing the wrapper
# PID MUST forward the signal to the child `ags run` — no orphaned task execution
# past cancellation. Uses a stub `ags` on PATH that sleeps (no real runner, no
# network) and asserts the child is gone after the wrapper is killed.
echo -n "[....] run-task-card.sh forwards cancellation to child ags run "
set +e
cancel_dir="$(mktemp -d)"
cat > "$cancel_dir/ags" << 'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "run" ]; then
    echo $$ > "$AGS_STUB_PIDFILE"
    sleep 30 &
    sp=$!
    trap 'kill "$sp" 2>/dev/null; exit 143' TERM INT HUP
    wait "$sp"
fi
exit 0
STUB
chmod +x "$cancel_dir/ags"
echo "## 任务卡" > "$cancel_dir/card.md"
AGS_STUB_PIDFILE="$cancel_dir/child.pid" PATH="$cancel_dir:$PATH" \
    bash "$REPO_ROOT/scripts/run-task-card.sh" "$cancel_dir/card.md" >/dev/null 2>&1 &
cancel_wrapper_pid=$!
cancel_stub_pid=""
for _ in $(seq 1 50); do
    if [ -s "$cancel_dir/child.pid" ]; then cancel_stub_pid="$(cat "$cancel_dir/child.pid")"; break; fi
    sleep 0.1
done
kill -TERM "$cancel_wrapper_pid" 2>/dev/null
for _ in $(seq 1 40); do
    { [ -n "$cancel_stub_pid" ] && kill -0 "$cancel_stub_pid" 2>/dev/null; } || break
    sleep 0.1
done
wait "$cancel_wrapper_pid" 2>/dev/null
if [ -z "$cancel_stub_pid" ]; then
    echo "FAIL (stub child never started)"
    failures=$((failures + 1))
elif kill -0 "$cancel_stub_pid" 2>/dev/null; then
    echo "FAIL (child ags run survived wrapper cancellation)"
    kill -KILL "$cancel_stub_pid" 2>/dev/null
    failures=$((failures + 1))
else
    echo "OK"
fi
rm -rf "$cancel_dir"
set -e

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
echo -n "[....] skill inventory: 4 kinds + both hosts + canonical "
if cargo run -q -p ags-cli -- skill --format json > /tmp/verify-skill-inv.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-skill-inv.json'))['inventory']; ks={c['kind'] for c in d['capabilities']}; assert {'skill','mcp','suite-interface','cli-backed'} <= ks, ks; ags=next(c for c in d['capabilities'] if c['name']=='ags'); assert ags['kind']=='suite-interface'; assert set(d['hosts']) >= {'claude-code','codex'}, d['hosts']; assert isinstance(d['summary']['canonical_present'], int); assert any(v['host']=='codex' for v in ags['host_visibility']), ags['host_visibility']"; then
        echo "OK"
    else
        echo "FAIL (missing a kind / AGS not suite-interface / missing host / no canonical / AGS no codex visibility)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill overview failed)"
    failures=$((failures + 1))
fi

# Machine-private lifecycle: dry-run is read-only; apply/ignore/rollback use only
# a temporary HOME/runtime and the hidden propose wrapper delegates to the same
# overlay service.
echo -n "[....] skill lifecycle overlay dry-run/adopt/ignore/rollback + compat wrapper "
external_skill_home="$(mktemp -d /tmp/ags-verify-skill-lifecycle.XXXXXX)"
external_skill_runtime="$external_skill_home/runtime"
external_skill_id=verify-overlay-candidate
mkdir -p "$external_skill_home/.agents/skills/$external_skill_id"
printf '%s\n' '---' "name: $external_skill_id" 'description: Isolated machine-private lifecycle fixture.' 'intent_tags: [verify-overlay]' '---' 'fixture body' \
    > "$external_skill_home/.agents/skills/$external_skill_id/SKILL.md"
if HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_runtime" target/release/ags skill adopt "$external_skill_id" --format json > "$external_skill_home/adopt-dry-run.json" 2>&1 \
    && test ! -e "$external_skill_runtime/skill-registry/user-overlay.yaml" \
    && HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_runtime" target/release/ags skill propose --action adopt --skill "$external_skill_id" --format json > "$external_skill_home/propose-compat.json" 2> "$external_skill_home/propose-compat.err" \
    && test ! -e "$external_skill_runtime/skill-registry/user-overlay.yaml" \
    && HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_runtime" target/release/ags skill adopt "$external_skill_id" --apply --format json > "$external_skill_home/adopt-apply.json" 2>&1 \
    && HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_runtime" target/release/ags skill ignore "$external_skill_id" --apply --format json > "$external_skill_home/ignore-apply.json" 2>&1 \
    && HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_runtime" target/release/ags skill rollback "$external_skill_id" --to 1 --apply --format json > "$external_skill_home/rollback-apply.json" 2>&1; then
    if python3 - "$external_skill_home" "$external_skill_runtime" "$external_skill_id" <<'PY'
import json
import os
import pathlib
import stat
import sys

home = pathlib.Path(sys.argv[1])
runtime = pathlib.Path(sys.argv[2])
skill_id = sys.argv[3]

dry = json.loads((home / "adopt-dry-run.json").read_text())
compat = json.loads((home / "propose-compat.json").read_text())
adopted = json.loads((home / "adopt-apply.json").read_text())
ignored = json.loads((home / "ignore-apply.json").read_text())
rolled_back = json.loads((home / "rollback-apply.json").read_text())

assert dry["dry_run"] is True and dry["applied"] is False
assert dry["status"] == "planned" and dry["skill_id"] == skill_id
assert compat["schema_version"] == dry["schema_version"]
assert compat["proposed_entry"] == dry["proposed_entry"]
assert "deprecated" in (home / "propose-compat.err").read_text()
assert adopted["applied"] is True and adopted["overlay_revision"] == 1
assert ignored["applied"] is True and ignored["overlay_revision"] == 2
assert ignored["proposed_entry"]["state"] == "ignored"
assert rolled_back["applied"] is True and rolled_back["overlay_revision"] == 3
assert rolled_back["proposed_entry"]["state"] == "active"

overlay = runtime / "skill-registry/user-overlay.yaml"
events = runtime / "skill-registry/user-overlay-events.ndjson"
snapshot = runtime / "capability-snapshot/codex.json"
for path in (overlay, events, snapshot):
    assert path.is_file(), path
    assert stat.S_IMODE(path.stat().st_mode) == 0o600, oct(stat.S_IMODE(path.stat().st_mode))
assert len(events.read_text().splitlines()) == 3
assert str(home) not in events.read_text()
PY
    then
        echo "OK"
    else
        echo "FAIL (overlay result, revision, 0600 mode, receipt, or wrapper invariant)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (machine-private lifecycle command failed)"
    failures=$((failures + 1))
fi

# Official registry IDs always win; overlay apply must fail without writing.
echo -n "[....] skill overlay cannot shadow official registry "
set +e
HOME="$external_skill_home" AGS_RUNTIME_HOME="$external_skill_home/official-runtime" target/release/ags skill adopt diagnosing-bugs --apply --format json > "$external_skill_home/official-precedence.log" 2>&1
official_rc=$?
set -e
if [ "$official_rc" -ne 0 ] \
    && grep -q "official_registry_precedence" "$external_skill_home/official-precedence.log" \
    && test ! -e "$external_skill_home/official-runtime/skill-registry/user-overlay.yaml"; then
    echo "OK"
else
    echo "FAIL (official registry precedence must fail closed and write no overlay; got rc=$official_rc)"
    failures=$((failures + 1))
fi
rm -rf -- "$external_skill_home"

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

# Cursor is a supported host with real .cursor/skills + shared-user probes.
echo -n "[....] skill verify --host cursor → supported (real checks) "
if cargo run -q -p ags-cli -- skill verify --host cursor --format json > /tmp/verify-skill-cursor.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-skill-cursor.json')); assert d['supported'] is True; assert d['status'] in ('ok','degraded','incomplete'); assert isinstance(d['checks'], list)"; then
        echo "OK"
    else
        echo "FAIL (cursor must be supported with a valid status and checks)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (skill verify cursor failed)"
    failures=$((failures + 1))
fi

# ── Cross-Agent Capability Layer Smoke Tests ────────────────────────────────
echo ""
echo "--- Cross-Agent Capability Layer Smoke Tests ---"

# capability inventory shares the skill-governance console model (4 kinds).
echo -n "[....] capability inventory (json) shares console model "
if cargo run -q -p ags-cli -- capability inventory --format json > /tmp/verify-cap-inv.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-cap-inv.json')); ks={c['kind'] for c in d['capabilities']}; assert {'skill','mcp','suite-interface','cli-backed'} <= ks, ks; assert set(d['hosts']) >= {'claude-code','codex'}, d['hosts']"; then
        echo "OK"
    else
        echo "FAIL (capability inventory must share the unified console model)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (capability inventory failed)"
    failures=$((failures + 1))
fi

# capability verify is the canonical host-visibility home (claude-code supported).
run_check "capability verify --host claude-code (json)" \
    cargo run -q -p ags-cli -- capability verify --host claude-code --format json

# capability install of an MCP advises BOTH hosts and never writes (advise-only).
echo -n "[....] capability install MCP advises claude+codex, no writes "
if cargo run -q -p ags-cli -- capability install --capability context7 --format json > /tmp/verify-cap-install.json 2>&1; then
    if python3 -c "import json; d=json.load(open('/tmp/verify-cap-install.json')); assert d['planned_writes']==[]; cmds=[c['command'] for c in d['advised_commands']]; assert any(c.startswith('claude mcp add context7') for c in cmds), cmds; assert any(c.startswith('codex mcp add context7') for c in cmds), cmds"; then
        echo "OK"
    else
        echo "FAIL (MCP install must advise both hosts and write nothing)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (capability install failed)"
    failures=$((failures + 1))
fi

# capability sync dry-run: informational batch (exit 0), syncs only adopted/governed.
echo -n "[....] capability sync (dry-run) batch plan, exit 0 "
set +e
cargo run -q -p ags-cli -- capability sync --format json > /tmp/verify-cap-sync.json 2>&1
sync_rc=$?
set -e
if [ "$sync_rc" -eq 0 ] && python3 -c "import json; d=json.load(open('/tmp/verify-cap-sync.json'))['summary']; assert d['applied']==0; assert d['considered']>0; assert d['needs_action']>=0"; then
    echo "OK"
else
    echo "FAIL (dry-run sync must be informational, exit 0, applied=0)"
    failures=$((failures + 1))
fi

# ags setup (no --yes) renders the cross-platform init wizard, plan-only.
echo -n "[....] ags setup renders cross-platform wizard (plan-only) "
if cargo run -q -p ags-cli -- setup > /tmp/verify-setup-wizard.txt 2>&1; then
    if grep -q "Cross-Platform Initialization Wizard" /tmp/verify-setup-wizard.txt \
        && grep -q "plan-only" /tmp/verify-setup-wizard.txt \
        && grep -q "never runs an external host registrar" /tmp/verify-setup-wizard.txt; then
        echo "OK"
    else
        echo "FAIL (setup must render the plan-only cross-platform wizard)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (ags setup plan failed)"
    failures=$((failures + 1))
fi

# ── Five-Segment Command Surface (0.3.0) ────────────────────────────────────
echo ""
echo "--- Five-Segment Command Surface (agents / update / skill) ---"

# Current architecture documents must not present the retired 0.2.8 request
# router as a live surface. Release notes and explicitly-marked compatibility
# material are intentionally outside this focused current-document check.
echo -n "[....] current documents use the 0.3 typed-proposal architecture "
current_docs=(AGENTS.md CLAUDE.md WORKSPACE.md README.en.md docs/architecture.md)
if rg -n -i -S 'Request Router|RequestDecision|request-router/|AGS 0\.2\.8 has|0\.2\.8 uses' "${current_docs[@]}" > /tmp/verify-current-doc-routing.log 2>&1; then
    echo "FAIL (retired 0.2.8 routing terminology found; see /tmp/verify-current-doc-routing.log)"
    failures=$((failures + 1))
else
    echo "OK"
fi

echo -n "[....] ags agents scan runs read-only (json) "
if cargo run -q -p ags-cli -- agents scan --format json > /tmp/verify-agents-scan.json 2>&1 \
    && grep -q '"command": "agents scan"' /tmp/verify-agents-scan.json; then
    echo "OK"
else
    echo "FAIL (ags agents scan json)"
    failures=$((failures + 1))
fi

echo -n "[....] ags agents govern is advise-only (applied=false, no host write) "
if cargo run -q -p ags-cli -- agents govern --format json > /tmp/verify-agents-govern.json 2>&1 \
    && grep -q '"apply_status": "advised-only"' /tmp/verify-agents-govern.json \
    && grep -q '"applied": false' /tmp/verify-agents-govern.json \
    && grep -q '"mcp_tools":' /tmp/verify-agents-govern.json; then
    echo "OK"
else
    echo "FAIL (ags agents govern must be advise-only, applied=false, tools visible)"
    failures=$((failures + 1))
fi

echo -n "[....] ags agents govern --apply remains dialog-only (no receipt) "
receipt_count_before=$(find "$HOME/.ags/runtime/receipts" -maxdepth 1 -type f -name 'ar-agents-govern-*.json' 2>/dev/null | wc -l | tr -d ' ' || true)
if cargo run -q -p ags-cli -- agents govern --apply --format json > /tmp/verify-agents-govern-apply.json 2>&1 \
    && grep -q '"apply_status": "advice-only-no-write"' /tmp/verify-agents-govern-apply.json \
    && grep -q '"selection_required": true' /tmp/verify-agents-govern-apply.json \
    && ! grep -q '"receipt_ref"' /tmp/verify-agents-govern-apply.json; then
    receipt_count_after=$(find "$HOME/.ags/runtime/receipts" -maxdepth 1 -type f -name 'ar-agents-govern-*.json' 2>/dev/null | wc -l | tr -d ' ' || true)
    if [ "$receipt_count_before" = "$receipt_count_after" ]; then
        echo "OK"
    else
        echo "FAIL (ags agents govern --apply must not create receipt files)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (ags agents govern --apply must stay dialog-only and omit receipt_ref)"
    failures=$((failures + 1))
fi

echo -n "[....] ags update check reports six lanes "
if cargo run -q -p ags-cli -- update check --format json > /tmp/verify-update-check.json 2>&1 \
    && [ "$(grep -c '"lane":' /tmp/verify-update-check.json)" -eq 6 ]; then
    echo "OK"
else
    echo "FAIL (ags update check must report six lanes)"
    failures=$((failures + 1))
fi

echo -n "[....] ags update projects lane is executable and drift-aware "
if python3 -c 'import json;d=json.load(open("/tmp/verify-update-check.json"));p=next(x for x in d["lanes"] if x["lane"]=="projects");assert p["auto_executes_locally"] is True and p["advice_only"] is False and isinstance(p["drift"], bool) and p["commands"]==["ags update apply --lane projects --apply"]'; then
    echo "OK"
else
    echo "FAIL (projects lane must be executable, drift-aware, and expose the real apply command)"
    failures=$((failures + 1))
fi

# ── Update Notifier Smoke (hermetic: fake envs, NO real GitHub) ─────────────
# The notifier never hits the network here — disabled / fake-fetch-fail /
# fake-latest cover every front-stage path with an injected runtime home.
echo -n "[....] ags update notify --format json valid + exit 0 (no network) "
NOTIFY_HOME1="$(mktemp -d)"
set +e
AGS_RUNTIME_HOME="$NOTIFY_HOME1" AGS_UPDATE_FAKE_FETCH_FAIL=1 AGS_UPDATE_FAKE_DATE=2026-06-19 \
    cargo run -q -p ags-cli -- update notify --format json > /tmp/verify-notify-valid.json 2>&1
notify_rc=$?
set -e
if [ "$notify_rc" -eq 0 ] && python3 -c 'import json;d=json.load(open("/tmp/verify-notify-valid.json"));assert d["notify"] is False and d["current_version"]'; then
    echo "OK"
else
    echo "FAIL (update notify must emit valid JSON and exit 0 even on check failure)"
    cat /tmp/verify-notify-valid.json
    failures=$((failures + 1))
fi
rm -rf "$NOTIFY_HOME1"

echo -n "[....] ags update notify disabled → notify=false reason=disabled "
if AGS_NO_UPDATE_NOTIFIER=1 cargo run -q -p ags-cli -- update notify --format json > /tmp/verify-notify-off.json 2>&1 \
    && python3 -c 'import json;d=json.load(open("/tmp/verify-notify-off.json"));assert d["notify"] is False and d["reason"]=="disabled"'; then
    echo "OK"
else
    echo "FAIL (disabled must be notify=false reason=disabled)"
    cat /tmp/verify-notify-off.json
    failures=$((failures + 1))
fi

echo -n "[....] ags update notify fake-latest newer → notify=true (no network) "
NOTIFY_HOME2="$(mktemp -d)"
if AGS_RUNTIME_HOME="$NOTIFY_HOME2" AGS_UPDATE_FAKE_LATEST=99.0.0 AGS_UPDATE_FAKE_DATE=2026-06-19 \
    cargo run -q -p ags-cli -- update notify --format json > /tmp/verify-notify-new.json 2>&1 \
    && python3 -c 'import json;d=json.load(open("/tmp/verify-notify-new.json"));assert d["notify"] is True and d["latest_version"]=="99.0.0" and d["update_command"]=="/ags update"'; then
    echo "OK"
else
    echo "FAIL (fake-latest newer must notify=true with update_command)"
    cat /tmp/verify-notify-new.json
    failures=$((failures + 1))
fi
rm -rf "$NOTIFY_HOME2"

echo -n "[....] ags update repair-local defaults to dry-run (no write) "
if cargo run -q -p ags-cli -- update repair-local --format json > /tmp/verify-update-repair.json 2>&1 \
    && grep -q '"apply_status": "dry-run"' /tmp/verify-update-repair.json; then
    echo "OK"
else
    echo "FAIL (repair-local must default to dry-run)"
    failures=$((failures + 1))
fi

echo -n "[....] ags skill dedupe defaults to dry-run "
if cargo run -q -p ags-cli -- skill dedupe --format json > /tmp/verify-skill-dedupe.json 2>&1 \
    && grep -q '"apply_status": "dry-run"' /tmp/verify-skill-dedupe.json; then
    echo "OK"
else
    echo "FAIL (skill dedupe must default to dry-run)"
    failures=$((failures + 1))
fi

echo -n "[....] ags skill --help exposes front-stage dedupe + sync "
if cargo run -q -p ags-cli -- skill --help > /tmp/verify-skill-help.txt 2>&1 \
    && grep -q 'dedupe' /tmp/verify-skill-help.txt \
    && grep -q 'sync' /tmp/verify-skill-help.txt; then
    echo "OK"
else
    echo "FAIL (skill help must list dedupe + sync)"
    failures=$((failures + 1))
fi

echo -n "[....] ags setup gates on Global Entry Protocol Templates (plan-only, no write) "
GE_TARGET="/tmp/verify-ags-global-entry-$$"
rm -rf "$GE_TARGET"
if cargo run -q -p ags-cli -- setup --target "$GE_TARGET" --format text > /tmp/verify-global-entry.txt 2>&1 \
    && grep -q "Global Entry Protocol Templates" /tmp/verify-global-entry.txt \
    && grep -q "Claude Code" /tmp/verify-global-entry.txt \
    && grep -q "WorkBuddy" /tmp/verify-global-entry.txt \
    && grep -q "CodeBuddy-Code" /tmp/verify-global-entry.txt \
    && [ ! -e "$GE_TARGET" ]; then
    echo "OK"
else
    echo "FAIL (setup must show Global Entry Protocol Templates and write nothing without --yes)"
    failures=$((failures + 1))
fi
rm -rf "$GE_TARGET"

echo -n "[....] host entry policy uses typed routing + OMP Plan single-card semantics "
if rg -q 'HostRouteProposal' crates/ags-cli/src/setup/templates.rs \
    && rg -q 'RouteResolution' crates/ags-cli/src/setup/templates.rs \
    && rg -q 'OMP Plan mode' crates/ags-cli/src/setup/templates.rs \
    && rg -q 'task_card_hash' crates/ags-cli/src/setup/templates.rs \
    && ! rg -n 'RequestDecision|把完整当前请求交给 `ags_route_request`|AGS 0\.2\.8 入口' \
        crates/ags-cli/src/setup/templates.rs crates/ags-cli/src/setup/global_entry.rs > /tmp/verify-host-entry-drift.txt 2>&1; then
    echo "OK"
else
    echo "FAIL (host entry template must use AGS 0.3 typed proposal and immutable OMP task-card dispatch)"
    cat /tmp/verify-host-entry-drift.txt
    failures=$((failures + 1))
fi
rm -f /tmp/verify-host-entry-drift.txt

echo -n "[....] ags update apply --format json emits pure JSON (no leading progress text) "
if cargo run -q -p ags-cli -- update apply --lane agents --apply --format json > /tmp/verify-update-apply.json 2>/dev/null \
    && head -1 /tmp/verify-update-apply.json | grep -q '^{' \
    && python3 -c 'import json;json.load(open("/tmp/verify-update-apply.json"))' 2>/dev/null; then
    echo "OK"
else
    echo "FAIL (update apply --format json must be valid JSON with no leading text)"
    failures=$((failures + 1))
fi
rm -f /tmp/verify-update-apply.json

echo -n "[....] ags setup --yes emits a setup-apply action receipt "
SETUP_HOME="/tmp/verify-ags-setup-home-$$"
rm -rf "$SETUP_HOME"
AGS_HOME="$SETUP_HOME" cargo run -q -p ags-cli -- setup --target "$SETUP_HOME" --yes --format text > /tmp/verify-setup-apply.txt 2>&1 || true
if ls "$SETUP_HOME"/receipts/ar-setup-apply-*.json >/dev/null 2>&1; then
    echo "OK"
else
    echo "FAIL (setup --yes must write a setup-apply receipt under <AGS_HOME>/receipts)"
    failures=$((failures + 1))
fi
rm -rf "$SETUP_HOME" /tmp/verify-setup-apply.txt

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
echo -n "[....] agent-task-protocol.md has host semantic proposal contract "
if grep -q 'Host Semantic Proposal（宿主语义提案）' "$REPO_ROOT/protocol/agent-task-protocol.md" \
    && grep -q 'HostRouteProposal' "$REPO_ROOT/protocol/agent-task-protocol.md" \
    && grep -q 'ags_apply_action' "$REPO_ROOT/protocol/agent-task-protocol.md"; then
    echo "OK"
else
    echo "FAIL (typed host proposal / explicit apply contract missing from agent-task-protocol.md)"
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
