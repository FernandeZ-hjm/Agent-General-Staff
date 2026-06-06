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
run_check "resolve-policy valid compact (text)" \
    cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-compact.md" --format text
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
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-compact.md" --format yaml > /tmp/verify-resolve-format.log 2>&1; then
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
if cat "$REPO_ROOT/tests/fixtures/valid-compact.md" | cargo run -q -p ags-cli -- policy resolve - --format json > /tmp/verify-resolve-stdin.log 2>&1; then
    echo "OK"
else
    echo "FAIL"
    cat /tmp/verify-resolve-stdin.log
    failures=$((failures + 1))
fi

# ── Resolver Hardening Smoke Tests (F1-F7) ──────────────────────────────────
echo "--- Resolver Hardening Smoke Tests (F1-F7) ---"

# F5: JSON uses canonical protocol values, not Rust variant names
echo -n "[....] JSON permission_mode uses canonical values "
if cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-compact.md" --format json > /tmp/verify-json-canonical.log 2>&1; then
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
cat > /tmp/test-readonly-worktree.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: read-only
Parallelism: worktree
Execution effort: normal
Workflow authority: plan-only
任务级别：Light
读取：- 本任务卡
任务：Test read-only + worktree gate.
目标：Verify --parallel is not output.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - test passes - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-plan-only-worktree.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: plan-only
Parallelism: worktree
Execution effort: normal
Workflow authority: plan-only
任务级别：Medium
读取：- 本任务卡
任务：Test plan-only + worktree gate.
目标：Verify --parallel is not output in plan-only.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - no --parallel - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-heavy-approve.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: edit-with-confirmation
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Heavy
读取：- 本任务卡
任务：Test Heavy with approval.
目标：Verify Heavy + --approve-writes keeps edit-with-confirmation.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - write mode preserved - stop condition: - any
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-bg-agent.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: background-agent
Permission mode: execute-and-verify
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Light
读取：- 本任务卡
任务：Test background-agent surface.
目标：Verify background-agent passes validation.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - surface accepted - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-heavy-stop.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: edit-with-confirmation
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Heavy
读取：- 本任务卡
任务：Test Heavy stop gate.
目标：Verify Heavy without --approve-writes sets stop_before_launch=true.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - stop_before_launch=true - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-heavy-plan.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: plan-only
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Heavy
读取：- 本任务卡
任务：Test Heavy plan-only gate.
目标：Verify Heavy plan-only does NOT trigger stop_before_launch.
非目标：不执行写操作。
关键路径：- .
停止条件：- 方案完成后返回用户审阅，等待明确批准
验证：Verification gate: - commands: - echo done - expected evidence: - no stop - stop condition: - any failure
交付：返回审计方案供 Codex review，等待明确批准。
TASKEOF
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
cat > /tmp/test-po-wt-none.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: plan-only
Parallelism: worktree
Execution effort: normal
Workflow authority: plan-only
任务级别：Medium
读取：- 本任务卡
任务：Test effective_parallelism consistency.
目标：Verify effective_parallelism is none when plan-only strips worktree.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - effective_parallelism is none - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-ro-wt-none.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: read-only
Parallelism: worktree
Execution effort: normal
Workflow authority: plan-only
任务级别：Light
读取：- 本任务卡
任务：Test read-only effective_parallelism.
目标：Verify effective_parallelism=none when read-only strips worktree.
非目标：None.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - effective_parallelism=none - stop condition: - any
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-bg-ro-audit.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: background-agent
Permission mode: read-only
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Light
读取：- 本任务卡
任务：Test background-agent audit trail.
目标：Verify background-agent+read-only sets stop_before_launch.
非目标：不执行写操作。
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - stop_before_launch=true - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-bg-po-audit.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: background-agent
Permission mode: plan-only
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Medium
读取：- 本任务卡
任务：Test background-agent + plan-only audit.
目标：Verify plan-only + background-agent sets stop_before_launch.
非目标：不执行写操作。
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - stop_before_launch=true - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-bg-ev-ok.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: background-agent
Permission mode: execute-and-verify
Parallelism: none
Execution effort: normal
Workflow authority: none
任务级别：Light
读取：- 本任务卡
任务：Test background-agent + execute-and-verify still works.
目标：Verify execute-and-verify allows --headless.
非目标：无.
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - --headless present - stop condition: - any
交付：Delivery report.
TASKEOF
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
cat > /tmp/test-combined-stop-reasons.md << 'TASKEOF'
## 任务卡
路径：- .
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: background-agent
Permission mode: read-only
Parallelism: worktree
Execution effort: normal
Workflow authority: plan-only
任务级别：Light
读取：- 本任务卡
任务：Test combined stop reasons.
目标：Verify combined blocked worktree and background-agent preserve both stop reasons.
非目标：不执行写操作。
关键路径：- .
停止条件：- 字段验证失败时停止
验证：Verification gate: - commands: - echo done - expected evidence: - two stop reasons - stop condition: - any failure
交付：Delivery report.
TASKEOF
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
if cargo run -q -p ags-cli -- gate check "$REPO_ROOT/tests/fixtures/valid-compact.md" --format json > /tmp/verify-m3-gate-light.log 2>&1; then
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
if "$REPO_ROOT/scripts/run-task-card.sh" "$REPO_ROOT/tests/fixtures/valid-compact.md" --check-only --format json > /tmp/verify-m8-run.log 2>&1; then
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

# ── Final result ────────────────────────────────────────────────────────────
echo ""
if [ "$failures" -eq 0 ]; then
    echo "=== All checks passed ==="
else
    echo "=== $failures check(s) FAILED ==="
    exit 1
fi
