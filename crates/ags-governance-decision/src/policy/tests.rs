use super::*;

fn input() -> TaskPolicyInput {
    TaskPolicyInput {
        executor: "Claude Code".into(),
        runtime_adapter: "claude-code".into(),
        execution_surface: "cli".into(),
        permission_mode: "execute-and-verify".into(),
        parallelism: "none".into(),
        task_level: "Medium".into(),
        execution_effort: Some("normal".into()),
        workflow_authority: Some("none".into()),
        approval_source: ApprovalSource::None,
    }
}

#[test]
fn task_level_does_not_grant_or_remove_execution_authority() {
    for level in ["Light", "Medium", "Heavy"] {
        let policy = resolve_policy(TaskPolicyInput {
            task_level: level.into(),
            ..input()
        });
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify,
            "{level}"
        );
        assert!(!policy.was_downgraded, "{level}");
        assert!(!policy.stop_before_launch, "{level}");
    }
}

#[test]
fn declared_plan_only_never_emits_write_or_parallel_flags() {
    for adapter in ["claude-code", "codex-local", "cursor", "omp", "generic"] {
        let policy = resolve_policy(TaskPolicyInput {
            runtime_adapter: adapter.into(),
            permission_mode: "plan-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("allowed".into()),
            ..input()
        });
        let args = policy.allowed_launch_args.join(" ");
        for forbidden in [
            "acceptEdits",
            "bypassPermissions",
            "--parallel",
            "--worktree",
            "--headless",
        ] {
            assert!(!args.contains(forbidden), "{adapter}: {args}");
        }
    }
}

#[test]
fn parallelism_requires_declared_workflow_authority() {
    for (authority, expected) in [
        ("none", Parallelism::None),
        ("allowed", Parallelism::Worktree),
    ] {
        let policy = resolve_policy(TaskPolicyInput {
            parallelism: "worktree".into(),
            workflow_authority: Some(authority.into()),
            ..input()
        });
        assert_eq!(policy.effective_parallelism, expected, "{authority}");
    }
}

#[test]
fn generic_adapter_requires_structured_write_approval() {
    for (approval, expected) in [
        (ApprovalSource::None, PermissionMode::PlanOnly),
        (
            ApprovalSource::CurrentTaskInstruction,
            PermissionMode::ExecuteAndVerify,
        ),
        (ApprovalSource::CliFlag, PermissionMode::ExecuteAndVerify),
    ] {
        let policy = resolve_policy(TaskPolicyInput {
            executor: "Other".into(),
            runtime_adapter: "generic".into(),
            approval_source: approval.clone(),
            ..input()
        });
        assert_eq!(policy.effective_permission_mode, expected, "{approval:?}");
    }
}

#[test]
fn exhaustive_effort_changes_only_thinking_intensity() {
    let normal = resolve_policy(input());
    let exhaustive = resolve_policy(TaskPolicyInput {
        execution_effort: Some("exhaustive".into()),
        ..input()
    });
    assert!(exhaustive.is_exhaustive_mode);
    assert_eq!(
        exhaustive.effective_permission_mode,
        normal.effective_permission_mode
    );
    assert_eq!(
        exhaustive.effective_parallelism,
        normal.effective_parallelism
    );
    assert_eq!(exhaustive.allowed_launch_args, normal.allowed_launch_args);
}

#[test]
fn gate_decision_is_derived_from_stop_before_launch() {
    for (policy_input, expected) in [
        (input(), GateDecision::Allow),
        (
            TaskPolicyInput {
                permission_mode: "plan-only".into(),
                parallelism: "worktree".into(),
                workflow_authority: Some("allowed".into()),
                ..input()
            },
            GateDecision::Stop,
        ),
    ] {
        let output = gate_check(&policy_input);
        assert_eq!(output.decision, expected);
        assert_eq!(
            output.resolved_policy.stop_before_launch,
            expected == GateDecision::Stop
        );
    }
}

#[test]
fn stopped_policy_exposes_no_launch_arguments() {
    let policy = resolve_policy(TaskPolicyInput {
        permission_mode: "plan-only".into(),
        parallelism: "worktree".into(),
        workflow_authority: Some("allowed".into()),
        ..input()
    });
    assert!(policy.stop_before_launch);
    assert!(policy.allowed_launch_args.is_empty());
}

#[test]
fn approval_mapping_uses_only_structured_signals() {
    let fields = std::collections::HashMap::from([
        ("Executor:".to_string(), "Other".to_string()),
        ("Runtime adapter:".to_string(), "generic".to_string()),
        (
            "Permission mode:".to_string(),
            "execute-and-verify".to_string(),
        ),
    ]);
    assert_eq!(
        TaskPolicyInput::from_fields(&fields).approval_source,
        ApprovalSource::None
    );
    assert_eq!(
        TaskPolicyInput::from_fields_with_approval(&fields, true, false).approval_source,
        ApprovalSource::CliFlag
    );
    assert_eq!(
        TaskPolicyInput::from_fields_with_approval(&fields, false, true).approval_source,
        ApprovalSource::CurrentTaskInstruction
    );
}

#[test]
fn structured_failure_and_explain_outputs_keep_their_contract() {
    let failure = gate_check_failed("validation_failed", vec!["bad card".into()]);
    assert_eq!(failure.decision, GateDecision::Stop);
    assert_eq!(failure.error_kind, "validation_failed");

    let explanation = explain_policy(&input());
    let json = serde_json::to_value(explanation).unwrap();
    assert_eq!(json["schema_version"], "0.3.4-execution-policy");
    assert!(json.get("task_summary").is_some());
    assert!(json.get("resolved_policy").is_some());
}

#[test]
fn resolved_policy_json_has_one_stable_schema() {
    let value = serde_json::to_value(resolve_policy(input())).unwrap();
    assert_eq!(value["schema_version"], "0.3.4-execution-policy");
    assert!(value.get("effective_permission_mode").is_some());
    assert!(value.get("allowed_launch_args").is_some());
}
