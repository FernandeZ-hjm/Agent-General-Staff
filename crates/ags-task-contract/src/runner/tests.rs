use super::*;
use ags_governance_decision::policy::{ApprovalSource, Parallelism, PermissionMode};
use std::path::PathBuf;

#[test]
fn test_read_error_produces_stop_plan() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    assert!(!plan.validation_passed);
    assert_eq!(plan.gate_decision, "stop");
    assert!(plan.gate_error_kind.is_some());
    assert!(plan.resolved_policy.is_none());
    assert!(plan.adapter.is_stub);
    assert_eq!(plan.governance_status, GovernanceStatus::BlockedByPolicy);
    assert!(!plan.host_execution_required);
    assert!(!plan.execution_performed);
    assert!(!plan.verification_performed);
}

#[test]
fn test_check_only_mode_flag() {
    let plan = run_task_card("/nonexistent/path/task-card.md", true, false, false, false);
    assert_eq!(plan.mode, "check-only");
    assert!(!plan.validation_passed);
}

#[test]
fn test_dry_run_mode_flag() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    assert_eq!(plan.mode, "dry-run");
}

#[test]
fn test_default_mode_prepares_execution() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, false, false, false);
    assert_eq!(plan.mode, "prepare-execution");
}

#[test]
fn test_schema_version_constant() {
    assert_eq!(SCHEMA_VERSION, "0.3.0-launch-plan");
}

#[test]
fn test_claude_code_adapter_is_not_stub() {
    // Build a minimal policy that maps to claude-code
    let policy = ResolvedExecutionPolicy {
        executor: "Claude Code".into(),
        runtime_adapter: "claude-code".into(),
        effective_permission_mode: PermissionMode::PlanOnly,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "cli".into(),
        allowed_launch_args: vec!["--permission-mode".into(), "plan".into()],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    assert_eq!(adapter.adapter, "claude-code");
    assert!(!adapter.is_stub);
    assert_eq!(adapter.executor_binary, "claude");
    assert_eq!(adapter.launch_args, vec!["--permission-mode", "plan"]);
}

#[test]
fn test_codex_adapter_is_stub() {
    let policy = ResolvedExecutionPolicy {
        executor: "Codex".into(),
        runtime_adapter: "codex-local".into(),
        effective_permission_mode: PermissionMode::ExecuteAndVerify,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "local-workspace".into(),
        allowed_launch_args: vec![],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    assert_eq!(adapter.adapter, "codex-local");
    assert!(adapter.is_stub);
    assert!(adapter.stub_reason.is_some());
}

#[test]
fn test_cursor_adapter_is_stub() {
    let policy = ResolvedExecutionPolicy {
        executor: "Cursor".into(),
        runtime_adapter: "cursor".into(),
        effective_permission_mode: PermissionMode::ExecuteAndVerify,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "ide".into(),
        allowed_launch_args: vec![],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    assert_eq!(adapter.adapter, "cursor");
    assert!(adapter.is_stub);
}

#[test]
fn test_omp_adapter_is_native_host_handoff() {
    let policy = ResolvedExecutionPolicy {
        executor: "OMP".into(),
        runtime_adapter: "omp".into(),
        effective_permission_mode: PermissionMode::ExecuteAndVerify,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "cli".into(),
        allowed_launch_args: vec![],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    assert_eq!(adapter.adapter, "omp");
    assert_eq!(adapter.executor_binary, "omp");
    assert!(adapter.is_stub);
}

#[test]
fn test_generic_adapter_is_stub() {
    let policy = ResolvedExecutionPolicy {
        executor: "Other".into(),
        runtime_adapter: "generic".into(),
        effective_permission_mode: PermissionMode::PlanOnly,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "cli".into(),
        allowed_launch_args: vec![],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    assert_eq!(adapter.adapter, "generic");
    assert!(adapter.is_stub);
}

#[test]
fn test_receipt_plan_skipped_on_stop() {
    // Simulate: validation failure, plan has no resolved_policy
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    assert!(!plan.receipt_plan.host_should_generate);
    assert_eq!(plan.receipt_plan.gate_result_for_receipt, "stop");
}

#[test]
fn test_receipt_hash_matches_receipt_crate_sha256() {
    let content = b"## Task Card\ncanonical receipt hash\n";
    assert_eq!(receipt_hash(content), ags_platform::sha256_hex(content));
    assert_eq!(receipt_hash(content).len(), 64);
}

#[test]
fn test_render_text_produces_output() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    let text = render_text(&plan);
    assert!(text.contains("AGS Runner"));
    assert!(text.contains("STOP"));
}

#[test]
fn test_render_json_produces_valid_json() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    let json = render_json(&plan);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
    assert!(parsed.is_ok());
    let v = parsed.unwrap();
    assert_eq!(v["schema_version"], SCHEMA_VERSION);
    assert_eq!(v["gate_decision"], "stop");
}

#[test]
fn test_runner_never_reads_raw_parallelism() {
    // The runner module does NOT import or use raw task-card fields
    // for launch decisions. All execution params come from resolved_policy.
    // This test verifies the structural invariant: resolve_adapter()
    // only reads from ResolvedExecutionPolicy, not from task card fields.
    let policy = ResolvedExecutionPolicy {
        executor: "Claude Code".into(),
        runtime_adapter: "claude-code".into(),
        effective_permission_mode: PermissionMode::PlanOnly,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "cli".into(),
        allowed_launch_args: vec![], // empty — M5/M6 enforced
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: true, // parallelism was downgraded
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    // Even though the raw card might have said Parallelism: worktree,
    // the resolved policy says none — and the adapter uses the resolved value.
    assert!(adapter.launch_args.is_empty());
    // No --parallel, --worktree, or other write-enabling flags appear.
    let cmd = &adapter.launch_command;
    assert!(!cmd.contains("--parallel"));
    assert!(!cmd.contains("--worktree"));
}

#[test]
fn test_launch_args_flow_verbatim_from_policy() {
    let policy = ResolvedExecutionPolicy {
        executor: "Claude Code".into(),
        runtime_adapter: "claude-code".into(),
        effective_permission_mode: PermissionMode::PlanOnly,
        effective_parallelism: Parallelism::None,
        effective_execution_surface: "cli".into(),
        allowed_launch_args: vec![
            "--permission-mode".into(),
            "plan".into(),
            "--output-format".into(),
            "json".into(),
        ],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "test-task-card.md");
    // allowed_launch_args flow verbatim into the adapter
    assert_eq!(
        adapter.launch_args,
        vec!["--permission-mode", "plan", "--output-format", "json"]
    );
    assert!(adapter
        .launch_command
        .contains("--permission-mode plan --output-format json"));
}

#[test]
fn test_approve_writes_flag_sets_approval_source() {
    // Read a real task card fixture to test the approve_writes flow
    let fixture = std::path::Path::new("../tests/fixtures");
    let card_path = fixture.join("heavy-plan-only.md");
    if card_path.exists() {
        let path_str = card_path.to_string_lossy().to_string();
        let plan_without = run_task_card(&path_str, false, true, false, false);
        let plan_with = run_task_card(&path_str, false, true, true, false);

        // Both should pass validation
        assert!(plan_without.validation_passed);
        assert!(plan_with.validation_passed);

        // With approve_writes, the policy should reflect CliFlag approval
        if let Some(ref policy) = plan_with.resolved_policy {
            assert_eq!(policy.approval_source.to_string(), "cli-flag");
        }
    }
}

#[test]
fn test_current_task_approval_flag_sets_approval_source() {
    let fixture = std::path::Path::new("../tests/fixtures");
    let card_path = fixture.join("heavy-plan-only.md");
    if card_path.exists() {
        let path_str = card_path.to_string_lossy().to_string();
        let plan = run_task_card(&path_str, false, true, false, true);

        assert!(plan.validation_passed);
        if let Some(ref policy) = plan.resolved_policy {
            assert_eq!(
                policy.approval_source.to_string(),
                "current-task-instruction"
            );
        }
    }
}

// ── Runtime skill-tag availability gate (the third gate) integration ─────

const VALID_CARD: &str = include_str!("../../../../tests/fixtures/valid-full.md");

fn unique_tmp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ags-runner-skilltag-{}-{}",
        label,
        std::process::id()
    ))
}

#[test]
fn host_for_adapter_maps_known_adapters() {
    assert_eq!(host_for_adapter("claude-code"), "claude-code");
    assert_eq!(host_for_adapter("codex-local"), "codex");
    assert_eq!(host_for_adapter("cursor"), "cursor");
    assert_eq!(host_for_adapter("omp"), "omp");
    // generic / unknown → host-agnostic (fail-closed).
    assert_eq!(host_for_adapter("generic"), "");
    assert_eq!(host_for_adapter("anything-else"), "");
}

#[test]
fn runtime_skill_tag_gate_stops_unavailable_tag() {
    // The card PASSES the offline static validator (skill-creator is a
    // routable registry tag), but its runtime availability fails: an empty
    // runtime home has no ActiveSkillTable snapshot, so skill resolution stops
    // (`not-enrolled`) and the third gate stops the launch. This proves the
    // runtime gate runs automatically on the `ags run` launch-plan path —
    // not only as the manual `ags gate skill-tags` subcommand.
    let dir = unique_tmp("stop");
    std::fs::create_dir_all(&dir).unwrap();
    let card_path = dir.join("card.md");
    std::fs::write(
        &card_path,
        format!("{VALID_CARD}\n[skill: skill-creator]\n"),
    )
    .unwrap();
    let runtime_home = dir.join("runtime-home"); // absent snapshot → governance precondition

    let plan = run_task_card_inner(
        &card_path.to_string_lossy(),
        false, // not check-only
        true,  // dry-run (launch-plan path)
        false,
        false,
        &runtime_home,
    );
    assert!(plan.validation_passed, "card must pass static validation");
    assert_eq!(plan.gate_decision, "stop");
    assert_eq!(
        plan.gate_error_kind.as_deref(),
        Some("skill_tags_unavailable")
    );
    let gate = plan.skill_tags_gate.expect("skill_tags_gate present");
    assert!(!gate.all_accepted);
    assert!(gate.rejected.iter().any(|t| t == "skill-creator"));
    assert!(
        !plan.receipt_plan.host_should_generate,
        "a blocked launch must not plan a receipt"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_skill_tag_gate_absent_when_card_has_no_tags() {
    // The base valid-full fixture has no trailing [skill: …] tags, so the
    // runtime gate has nothing to check and never appears / never stops.
    let dir = unique_tmp("notags");
    std::fs::create_dir_all(&dir).unwrap();
    let card_path = dir.join("card.md");
    std::fs::write(&card_path, VALID_CARD).unwrap();
    let runtime_home = dir.join("runtime-home");

    let plan = run_task_card_inner(
        &card_path.to_string_lossy(),
        false,
        true,
        false,
        false,
        &runtime_home,
    );
    assert!(plan.validation_passed);
    assert!(plan.skill_tags_gate.is_none());
    assert_eq!(
        plan.governance_status,
        GovernanceStatus::HostExecutionRequired
    );
    assert!(plan.host_execution_required);
    assert!(!plan.execution_performed);
    assert!(!plan.verification_performed);
    assert!(!plan.receipt_plan.generated);
    assert!(plan.receipt_plan.host_should_generate);
    assert_ne!(
        plan.gate_error_kind.as_deref(),
        Some("skill_tags_unavailable")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_only_skips_runtime_skill_tag_gate() {
    // check-only stops at the offline policy gate; the runtime skill-tag gate
    // belongs to the launch-plan path and must NOT run in check-only mode.
    let dir = unique_tmp("checkonly");
    std::fs::create_dir_all(&dir).unwrap();
    let card_path = dir.join("card.md");
    std::fs::write(
        &card_path,
        format!("{VALID_CARD}\n[skill: skill-creator]\n"),
    )
    .unwrap();
    let runtime_home = dir.join("runtime-home");

    let plan = run_task_card_inner(
        &card_path.to_string_lossy(),
        true, // check-only
        false,
        false,
        false,
        &runtime_home,
    );
    assert_eq!(plan.mode, "check-only");
    assert!(plan.skill_tags_gate.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
