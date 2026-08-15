use super::*;
use ags_governance_decision::policy::{ApprovalSource, ExecutionMode, ExecutionTopology};
use std::path::PathBuf;

const VALID_CARD: &str = include_str!("../../../../tests/fixtures/valid-full.md");

fn unique_tmp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ags-runner-{label}-{}", std::process::id()))
}

#[test]
fn read_failure_stops_without_execution() {
    let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
    assert_eq!(plan.gate_decision, "stop");
    assert_eq!(plan.governance_status, GovernanceStatus::BlockedByPolicy);
    assert!(!plan.host_execution_required);
    assert!(!plan.execution_performed);
}

#[test]
fn omp_resolves_to_native_host_handoff() {
    let policy = ResolvedExecutionPolicy {
        executor: "OMP".into(),
        runtime_adapter: "omp".into(),
        effective_execution_mode: ExecutionMode::SingleWriter,
        effective_execution_topology: ExecutionTopology::Single,
        effective_execution_surface: "cli".into(),
        delegation_planning: false,
        allowed_launch_args: vec![],
        stop_before_launch: false,
        stop_reasons: vec![],
        was_downgraded: false,
        downgrade_reasons: vec![],
        execution_effort: "normal".into(),
        is_exhaustive_mode: false,
        approval_source: ApprovalSource::None,
    };

    let adapter = resolve_adapter(&policy, "task-card.md");
    assert_eq!(adapter.adapter, "omp");
    assert_eq!(adapter.executor_binary, "omp");
    assert!(adapter.is_stub);
    assert_eq!(host_for_adapter("omp"), "omp");
}

#[test]
fn unavailable_runtime_skill_tag_blocks_launch() {
    let dir = unique_tmp("blocked-skill");
    std::fs::create_dir_all(&dir).unwrap();
    let card = dir.join("card.md");
    // Use a canonical bundled route so the card passes the two static gates;
    // the isolated runtime home must then fail at the live availability gate.
    std::fs::write(&card, format!("{VALID_CARD}\n[skill: codebase-design]\n")).unwrap();

    let plan = run_task_card_inner(
        &card.to_string_lossy(),
        false,
        true,
        false,
        false,
        &dir.join("runtime-home"),
    );
    assert_eq!(
        plan.gate_error_kind.as_deref(),
        Some("skill_tags_unavailable")
    );
    assert!(!plan.host_execution_required);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn card_without_skill_tags_prepares_host_execution() {
    let dir = unique_tmp("no-tags");
    std::fs::create_dir_all(&dir).unwrap();
    let card = dir.join("card.md");
    std::fs::write(&card, VALID_CARD).unwrap();

    let plan = run_task_card_inner(
        &card.to_string_lossy(),
        false,
        true,
        false,
        false,
        &dir.join("runtime-home"),
    );
    assert!(plan.skill_tags_gate.is_none());
    assert_eq!(
        plan.governance_status,
        GovernanceStatus::HostExecutionRequired
    );
    assert!(plan.host_execution_required);
    assert_eq!(plan.schema_version, "ags://schema/contract/v2/launch-plan");
    assert!(!plan.task_card_hash.is_empty());
    assert!(!plan.launch_plan_hash.is_empty());
    let value = serde_json::to_value(&plan).unwrap();
    assert_eq!(
        canonical_launch_plan_hash(&value).unwrap(),
        plan.launch_plan_hash
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_only_skips_runtime_skill_gate() {
    let dir = unique_tmp("check-only");
    std::fs::create_dir_all(&dir).unwrap();
    let card = dir.join("card.md");
    std::fs::write(&card, format!("{VALID_CARD}\n[skill: codebase-design]\n")).unwrap();

    let plan = run_task_card_inner(
        &card.to_string_lossy(),
        true,
        false,
        false,
        false,
        &dir.join("runtime-home"),
    );
    assert_eq!(plan.mode, "check-only");
    assert!(plan.skill_tags_gate.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
