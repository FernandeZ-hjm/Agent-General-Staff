#!/usr/bin/env bash
# verify.sh — AGS public edition verification gate (compatibility wrapper).
#
# Canonical verification authority is `ags verify`. This script:
#   1. Runs `ags verify --scope full --format text` as the primary gate.
#   2. Runs remaining shell-only smoke tests that have not yet been
#      migrated to Rust check items.
#   3. Exits with the combined result.
#
# Full-blood public edition: tests all public commands (task validate/compile/new,
# policy resolve/explain/check, sync check, gate check, doctor, bootstrap,
# project detect, protocol status, agent instructions, session preflight,
# verify, run, receipt, compliance, skill, capability, archive).
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

echo "=== AGS Public Edition Verification Gate ==="
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
    failures=$((failures + 1))
fi
echo "--- End Primary Gate ---"
echo ""

# ── Supply-chain gate: cargo-deny (repo-local deny.toml) ────────────────────
# Fail-closed: a missing cargo-deny is a gate FAILURE (not skipped), so a
# minimal CI image or fresh box cannot pass verification without the
# advisory/license/source checks running. Install: cargo install cargo-deny --locked.
echo "--- Supply-chain Gate: cargo deny check ---"
if command -v cargo-deny >/dev/null 2>&1; then
    if cargo deny check >/tmp/ags-deny.log 2>&1; then
        echo "[OK] cargo deny check"
    else
        echo "[FAIL] cargo deny check (supply-chain policy violation)"
        tail -10 /tmp/ags-deny.log || true
        failures=$((failures + 1))
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

echo "--- Remaining Shell-Only Smoke Tests ---"
echo "(These tests are candidates for future Rust migration.)"
echo ""

# ── Public Command Surface Smoke Tests ─────────────────────────────────────
echo "--- Public Command Surface Smoke Tests ---"

run_check "ags task validate valid fixture" \
    cargo run -q -p ags-cli -- task validate "$REPO_ROOT/tests/fixtures/valid-compact.md"

run_check "ags task validate valid full fixture" \
    cargo run -q -p ags-cli -- task validate "$REPO_ROOT/tests/fixtures/valid-full.md"

run_check "ags policy resolve valid compact (text)" \
    cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-compact.md" --format text

run_check "ags policy resolve valid full (json)" \
    cargo run -q -p ags-cli -- policy resolve "$REPO_ROOT/tests/fixtures/valid-full.md" --format json

run_check "ags policy explain valid compact (text)" \
    cargo run -q -p ags-cli -- policy explain "$REPO_ROOT/tests/fixtures/valid-compact.md" --format text

run_check "ags policy check valid compact (json)" \
    cargo run -q -p ags-cli -- policy check "$REPO_ROOT/tests/fixtures/valid-compact.md" --format json

# ags sync check is tested by Rust unit tests (workflow-sync-check crate).
# The shell-level smoke test requires a properly configured multi-target
# setup (source + target directories with manifest files), which is
# covered by the Rust test suite.

run_check "ags doctor" \
    cargo run -q -p ags-cli -- doctor --format text

run_check "ags bootstrap --dry-run" \
    cargo run -q -p ags-cli -- bootstrap --dry-run --format text

run_check "ags project detect" \
    cargo run -q -p ags-cli -- project detect

run_check "ags protocol status" \
    cargo run -q -p ags-cli -- protocol status

run_check "ags agent instructions --for claude-code" \
    cargo run -q -p ags-cli -- agent instructions --for claude-code

run_check "ags session preflight --for claude-code" \
    cargo run -q -p ags-cli -- session preflight --for claude-code --format json --target "$REPO_ROOT"

run_check "ags verify --scope local --format text" \
    cargo run -q -p ags-cli -- verify --scope local --format text

run_check "ags verify --scope release --format text" \
    cargo run -q -p ags-cli -- verify --scope release --format text

# ── Full-Blood Command Surface (new commands) ────────────────────────────────
echo "--- Full-Blood Command Surface Smoke Tests ---"

run_check "ags task new compact" \
    cargo run -q -p ags-cli -- task new --card-type compact

run_check "ags task compile (stdin, request)" \
    bash -c 'echo "任务：test compile
目标：verify smoke test" | cargo run -q -p ags-cli -- task compile - --task-card-requested --output card --format text'

run_check "ags gate check" \
    cargo run -q -p ags-cli -- gate check "$REPO_ROOT/tests/fixtures/valid-compact.md" --format text

run_check "ags run --dry-run" \
    cargo run -q -p ags-cli -- run "$REPO_ROOT/tests/fixtures/valid-compact.md" --dry-run --format text

run_check "ags skill scan" \
    cargo run -q -p ags-cli -- skill scan --format text

# skill check is expected to report manifest-to-adoption-log gap
# when optional skills are recommended but not yet adopted
echo -n "[....] ags skill check "
if cargo run -q -p ags-cli -- skill check --format text > /tmp/ags-smoke-skill-check.log 2>&1; then
    echo "OK (all governance files consistent)"
else
    if grep -q "manifest-to-adoption-log" /tmp/ags-smoke-skill-check.log; then
        echo "OK (expected: optional skills not yet adopted)"
    else
        echo "FAIL"
        cat /tmp/ags-smoke-skill-check.log
        failures=$((failures + 1))
    fi
fi

run_check "ags skill propose remove" \
    cargo run -q -p ags-cli -- skill propose --action remove --skill auto-brainstorm --format text

run_check "ags skill overview --fix" \
    cargo run -q -p ags-cli -- skill --fix --format text

run_check "ags skill inventory" \
    cargo run -q -p ags-cli -- skill inventory --format text

run_check "ags skill verify codex" \
    cargo run -q -p ags-cli -- skill verify --host codex --format text

run_check "ags skill propose verify" \
    cargo run -q -p ags-cli -- skill propose --action verify --skill auto-brainstorm --format text

run_check "ags capability list" \
    cargo run -q -p ags-cli -- capability list --format text

run_check "ags capability show" \
    cargo run -q -p ags-cli -- capability show "policy:agent-task-protocol" --format text

INTEGRATE_SMOKE_DIR="/tmp/ags-smoke-project-integrate"
INTEGRATE_MEMORY_DIR="/tmp/ags-smoke-project-memory"
rm -rf "$INTEGRATE_SMOKE_DIR" "$INTEGRATE_MEMORY_DIR"
mkdir -p "$INTEGRATE_SMOKE_DIR"

run_check "ags project integrate (dry-run)" \
    cargo run -q -p ags-cli -- project integrate --target "$INTEGRATE_SMOKE_DIR" --dry-run --format text

run_check "ags project integrate --confirm" \
    env AGS_MEMORY_DIR="$INTEGRATE_MEMORY_DIR" cargo run -q -p ags-cli -- project integrate --target "$INTEGRATE_SMOKE_DIR" --confirm --format text

echo -n "[....] ags project integrate initializes memory "
if [ -f "$INTEGRATE_MEMORY_DIR/ags-smoke-project-integrate/context-capsule.md" ] \
    && [ -f "$INTEGRATE_MEMORY_DIR/ags-smoke-project-integrate/task-memory.md" ] \
    && [ -d "$INTEGRATE_MEMORY_DIR/ags-smoke-project-integrate/task-archive" ]; then
    echo "OK"
else
    echo "FAIL"
    find "$INTEGRATE_MEMORY_DIR" -maxdepth 3 -type f -o -type d 2>/dev/null || true
    failures=$((failures + 1))
fi

# receipt generate + verify smoke
echo -n "[....] ags receipt generate + verify "
if cargo run -q -p ags-cli -- receipt generate --task-card "$REPO_ROOT/tests/fixtures/valid-compact.md" --gate-result allow --format json > /tmp/ags-smoke-receipt.json 2>&1; then
    if cargo run -q -p ags-cli -- receipt verify /tmp/ags-smoke-receipt.json --format text > /tmp/ags-smoke-verify.log 2>&1; then
        if grep -q "VALID" /tmp/ags-smoke-verify.log; then
            echo "OK"
        else
            echo "FAIL (receipt verify did not report VALID)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (receipt verify failed)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (receipt generate failed)"
    failures=$((failures + 1))
fi

# compliance check smoke — pass --review-gate-status directly, no jq/sed
echo -n "[....] ags compliance check "
# Copy test fixture to /tmp to avoid /Volumes/AI Project/ path in receipt
cp "$REPO_ROOT/tests/fixtures/valid-compact.md" /tmp/ags-smoke-task-card.md
if cargo run -q -p ags-cli -- receipt generate \
    --task-card /tmp/ags-smoke-task-card.md \
    --gate-result allow \
    --verification "cargo test:0" \
    --verification "cargo build:0" \
    --review-gate-status "完成 — Light verification gate passed" \
    --format json > /tmp/ags-smoke-receipt-compliance.json 2>&1; then
    if cargo run -q -p ags-cli -- compliance check /tmp/ags-smoke-receipt-compliance.json --format text > /tmp/ags-smoke-compliance.log 2>&1; then
        if grep -q "COMPLIANT" /tmp/ags-smoke-compliance.log; then
            echo "OK"
        else
            echo "FAIL (compliance check did not report COMPLIANT)"
            failures=$((failures + 1))
        fi
    else
        echo "FAIL (compliance check failed)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (receipt generate failed)"
    failures=$((failures + 1))
fi

# skill install: MUST block without --confirm
echo -n "[....] ags skill install blocks without confirm "
if cargo run -q -p ags-cli -- skill install --skill auto-brainstorm --format text > /tmp/ags-smoke-install.log 2>&1; then
    echo "FAIL (skill install without --confirm should exit non-zero)"
    failures=$((failures + 1))
elif grep -q "blocked" /tmp/ags-smoke-install.log; then
    echo "OK"
else
    echo "FAIL (missing block message)"
    failures=$((failures + 1))
fi

# skill install: --confirm installs (directory structure with SKILL.md)
echo -n "[....] ags skill install --confirm "
if cargo run -q -p ags-cli -- skill install --skill auto-brainstorm --confirm --target /tmp/ags-smoke-skills --format text > /tmp/ags-smoke-install2.log 2>&1; then
    if [ -f /tmp/ags-smoke-skills/auto-brainstorm/SKILL.md ]; then
        # Verify SKILL.md has frontmatter
        if grep -q 'name:' /tmp/ags-smoke-skills/auto-brainstorm/SKILL.md && \
           grep -q 'description:' /tmp/ags-smoke-skills/auto-brainstorm/SKILL.md; then
            echo "OK"
        else
            echo "FAIL (SKILL.md created but missing frontmatter)"
            failures=$((failures + 1))
        fi
    elif [ -f /tmp/ags-smoke-skills/auto-brainstorm.md ]; then
        echo "FAIL (legacy flat file format — must be directory/SKILL.md)"
        failures=$((failures + 1))
    else
        echo "FAIL (SKILL.md not created)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (confirm install failed)"
    failures=$((failures + 1))
fi

# skill install: install receipt exists after confirm
echo -n "[....] ags skill install receipt exists "
if [ -f /tmp/ags-smoke-skills/install-receipt.yaml ]; then
    echo "OK"
else
    echo "FAIL (install receipt not created)"
    failures=$((failures + 1))
fi

# skill install: recommended creates multiple directories
echo -n "[....] ags skill install recommended "
rm -rf /tmp/ags-smoke-skills2 2>/dev/null || true
if cargo run -q -p ags-cli -- skill install --skill recommended --confirm --target /tmp/ags-smoke-skills2 --format text > /tmp/ags-smoke-install3.log 2>&1; then
    ok_count=0
    for name in auto-brainstorm auto-debug auto-verify tdd diagnose caveman-review caveman-commit; do
        if [ -f "/tmp/ags-smoke-skills2/$name/SKILL.md" ]; then
            ok_count=$((ok_count + 1))
        fi
    done
    if [ "$ok_count" -ge 7 ]; then
        echo "OK ($ok_count/7+ skills installed)"
    else
        echo "FAIL (only $ok_count/7 skills installed)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (recommended install failed)"
    failures=$((failures + 1))
fi

# doctor: public edition checks report expanded health status
echo -n "[....] ags doctor reports full-blood checks "
if cargo run -q -p ags-cli -- doctor --format text > /tmp/ags-smoke-doctor.log 2>&1; then
    has_checks=0
    grep -q 'mcp_registry_gep_adopted' /tmp/ags-smoke-doctor.log && has_checks=$((has_checks + 1)) || true
    grep -q 'evolver_proxy_health' /tmp/ags-smoke-doctor.log && has_checks=$((has_checks + 1)) || true
    grep -q 'runtime_profile_template_exists' /tmp/ags-smoke-doctor.log && has_checks=$((has_checks + 1)) || true
    grep -q 'codex_planner_hook_template_exists' /tmp/ags-smoke-doctor.log && has_checks=$((has_checks + 1)) || true
    grep -q 'claude_code_stop_hook_template_exists' /tmp/ags-smoke-doctor.log && has_checks=$((has_checks + 1)) || true
    if [ "$has_checks" -ge 3 ]; then
        echo "OK ($has_checks/5 public health check categories reported)"
    else
        echo "FAIL (only $has_checks/5 check categories found)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (doctor failed)"
    failures=$((failures + 1))
fi

# compliance: catches missing verification
echo -n "[....] ags compliance catches missing verification "
cat > /tmp/test-compliance-no-verify.json << 'EOF'
{"schema_version":"2.0-m6","receipt_id":"receipt-test","timestamp":"unix-0","task_card_hash":"abc123","task_card_path":null,"gate_result":{"decision":"allow","reason":null},"verification_results":[],"delivery_report_hash":null,"exit_code":0,"review_gate_status":null,"metadata":null}
EOF
set +e
cargo run -q -p ags-cli -- compliance check /tmp/test-compliance-no-verify.json --format text > /tmp/test-comp-log.log 2>&1
comp_rc=$?
set -e
if [ "$comp_rc" -ne 0 ] && grep -q 'verification_presence' /tmp/test-comp-log.log; then
    echo "OK"
else
    echo "FAIL (should reject empty verification_results with verification_presence)"
    failures=$((failures + 1))
fi

# compliance: catches missing review gate
echo -n "[....] ags compliance catches missing review gate "
cat > /tmp/test-compliance-no-review.json << 'EOF'
{"schema_version":"2.0-m6","receipt_id":"receipt-test2","timestamp":"unix-0","task_card_hash":"abc123","task_card_path":null,"gate_result":{"decision":"allow","reason":null},"verification_results":[{"command":"echo ok","exit_code":0,"output_hash":"abc"}],"delivery_report_hash":null,"exit_code":0,"review_gate_status":null,"metadata":null}
EOF
set +e
cargo run -q -p ags-cli -- compliance check /tmp/test-compliance-no-review.json --format text > /tmp/test-comp-log2.log 2>&1
comp_rc2=$?
set -e
if [ "$comp_rc2" -ne 0 ] && grep -q 'review_gate' /tmp/test-comp-log2.log; then
    echo "OK"
else
    echo "FAIL (should reject missing review_gate_status)"
    failures=$((failures + 1))
fi

# compliance: catches private path in receipt
echo -n "[....] ags compliance catches private paths "
cat > /tmp/test-compliance-priv.json << 'EOF'
{"schema_version":"2.0-m6","receipt_id":"receipt-test3","timestamp":"unix-0","task_card_hash":"abc123","task_card_path":"/Users/example/task.md","gate_result":{"decision":"allow","reason":null},"verification_results":[{"command":"echo ok","exit_code":0,"output_hash":"abc"}],"delivery_report_hash":null,"exit_code":0,"review_gate_status":"完成","metadata":null}
EOF
set +e
cargo run -q -p ags-cli -- compliance check /tmp/test-compliance-priv.json --format text > /tmp/test-comp-log3.log 2>&1
comp_rc3=$?
set -e
if [ "$comp_rc3" -ne 0 ] && grep -q 'private_path' /tmp/test-comp-log3.log; then
    echo "OK"
else
    echo "FAIL (should catch private path /Users/ in receipt)"
    failures=$((failures + 1))
fi

# archive smoke
echo -n "[....] ags archive "
if cargo run -q -p ags-cli -- archive --summary "verify.sh smoke test" --task-card "$REPO_ROOT/tests/fixtures/valid-compact.md" --format text > /tmp/ags-smoke-archive.log 2>&1; then
    if grep -q "Archive complete" /tmp/ags-smoke-archive.log; then
        echo "OK"
    else
        echo "FAIL (archive did not complete)"
        failures=$((failures + 1))
    fi
else
    echo "FAIL (archive failed)"
    failures=$((failures + 1))
fi

# ── Resolve-Policy Smoke Tests ──────────────────────────────────────────────
echo "--- Resolve-Policy Smoke Tests ---"

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

# ── Resolver Hardening Smoke Tests (F8-F10) ─────────────────────────────────
echo ""
echo "--- Resolver Hardening Smoke Tests (F8-F10) ---"

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
交付：返回审计方案供 review，等待明确批准。
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

# ── Background-agent audit (F10) ───────────────────────────────────────────
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
            echo "OK"
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

# ── Protocol Lifecycle Semantics ────────────────────────────────────────────
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

# ── Final result ────────────────────────────────────────────────────────────
echo ""
if [ "$failures" -eq 0 ]; then
    echo "=== All checks passed ==="
else
    echo "=== $failures check(s) FAILED ==="
    exit 1
fi
