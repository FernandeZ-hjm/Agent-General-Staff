use super::*;
#[allow(unused_imports)]
use super::{apply::*, decision::*, preflight::*, wire::*};
use request_governance::{
    ExecutionAuthority, ProposalPhase, SolutionState, HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
};

#[cfg(unix)]
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn binding() -> PreflightBinding {
    PreflightBinding {
        host: "codex".to_string(),
        target: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap(),
        host_home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[test]
fn stale_capability_snapshot_downgrades_false_green_preflight() {
    let mut report = serde_json::json!({
        "overall_status": "ok",
        "governance_status": "OK",
        "should_stop": false,
        "warnings": [],
        "next_steps": [
            "✓ All clear — project is fully integrated.",
            "  Codex may execute tasks per AGS governance lifecycle."
        ]
    });
    let capability = serde_json::json!({
        "uri": CURRENT_HOST_CAPABILITIES_URI,
        "status": "snapshot_stale",
        "snapshot_hash": null,
        "refresh_required": true,
        "refresh": {
            "argv": [
                "ags",
                "capability",
                "snapshot",
                "--host",
                "codex",
                "--target",
                "/tmp/project",
                "--write"
            ],
            "requires_repreflight": true
        }
    });

    attach_capability_catalog(&mut report, capability);

    assert_eq!(report["overall_status"], "warning");
    assert_eq!(report["governance_status"], "NEEDS_USER_DECISION");
    assert_eq!(report["should_stop"], false);
    assert!(report["warnings"]
        .as_array()
        .is_some_and(|warnings| warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|text| text.contains("capability snapshot is stale"))
        })));
    assert!(report["next_steps"]
        .as_array()
        .is_some_and(|steps| steps.iter().all(|step| {
            step.as_str().is_none_or(|text| {
                !text.contains("All clear") && !text.contains("may execute tasks")
            })
        })));
}

#[test]
fn ready_capability_snapshot_preserves_green_preflight() {
    let mut report = serde_json::json!({
        "overall_status": "ok",
        "governance_status": "OK",
        "should_stop": false,
        "warnings": [],
        "next_steps": ["✓ All clear — project is fully integrated."]
    });
    let capability = serde_json::json!({
        "uri": CURRENT_HOST_CAPABILITIES_URI,
        "status": "ready",
        "snapshot_hash": "sha256:snapshot",
        "refresh_required": false
    });

    attach_capability_catalog(&mut report, capability);

    assert_eq!(report["overall_status"], "ok");
    assert_eq!(report["governance_status"], "OK");
    assert!(report["warnings"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        report["capability_catalog"]["snapshot_hash"],
        "sha256:snapshot"
    );
}

fn direct_proposal() -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:req",
        "phase": ProposalPhase::DirectResponse,
        "solution_state": SolutionState::NotRequired,
        "execution_authority": ExecutionAuthority::None,
        "scope_hash": "sha256:scope",
        "targets": [{"kind": "direct_response"}]
    })
}

fn machine_proposal() -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:req",
        "phase": "execution",
        "solution_state": "confirmed",
        "execution_authority": "task_card_handoff",
        "scope_hash": "sha256:scope",
        "targets": [{
            "kind": "machine_cli",
            "capability": "task_compile",
            "input": {
                "kind": "confirmed_handoff_contract",
                "content": "任务：test contract"
            }
        }]
    })
}

fn valid_execution_card() -> String {
    "## 任务卡\n\
读取并遵守：\n- 本任务卡\n\
Contract ID: tc-0123456789abcdef\n\
Handoff source: existing-card\n\
Executor: Codex\n\
Runtime adapter: codex-local\n\
Execution surface: local-workspace\n\
Permission mode: execute-and-verify\n\
Parallelism: none\n\
Execution effort: high\n\
Workflow authority: none\n\
任务级别：Medium\n\
Review gate:\n- 按协议执行当前任务级别\n\
任务：验证执行准备策略\n\
背景：验证只读路由会先完成任务卡校验和策略解析\n\
项目画像：无\n\
记忆胶囊：无\n\
任务存档：无\n\
目标文件夹路径：\n- .\n\
相关路径：\n- .\n\
本次任务相关文件：\n- .\n\
目标：\n- G-01: 生成宿主执行所需的 LaunchPlan\n\
验收标准：\n- AC-01 -> G-01: 路由返回允许执行且包含真实策略哈希\n\
非目标：不在 AGS Runner 内执行宿主任务\n\
验证：\ncargo test -p ags-mcp\n\
Verification gate:\n- commands: cargo test -p ags-mcp\n\
- commands:\n  - V-01 -> AC-01: cargo test -p ags-mcp\n\
- expected evidence:\n  - EV-01 -> AC-01: task_prepare 路由返回 OK\n\
- stop condition:\n  - 策略解析失败时停止\n\
交付：\n返回 host_execution_required\n"
        .to_string()
}

fn machine_fixture(tag: &str) -> (PathBuf, PreflightBinding, PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("ags-mcp-machine-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let target = base.join("target");
    let runtime = base.join("runtime");
    let home = base.join("home");
    let executable = base.join("fake-ags");
    let spy = base.join("process-spy.txt");
    std::fs::create_dir_all(target.join("manifests")).unwrap();
    std::fs::write(
        target.join("manifests/skills-registry.yaml"),
        "skills: []\ndemand_routes: []\n",
    )
    .unwrap();
    std::fs::write(target.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
    let snapshot =
        skill_resolver::build_capability_snapshot_with_roots(&target, "codex", &runtime, &home)
            .unwrap();
    skill_resolver::write_private_atomic(
        &skill_resolver::snapshot_path(&runtime, "codex"),
        serde_json::to_string_pretty(&snapshot).unwrap().as_bytes(),
    )
    .unwrap();
    std::fs::write(
            &executable,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$@\" > \"$AGS_PROCESS_SPY\"\npwd > \"${AGS_PROCESS_SPY}.cwd\"\n",
        )
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let binding = PreflightBinding {
        host: "codex".to_string(),
        target,
        host_home: home,
    };
    (base, binding, runtime, executable, spy)
}

fn route_action(output: &str) -> (String, String) {
    let value: serde_json::Value = serde_json::from_str(output).unwrap();
    let lease_id = value["lease"]["lease_id"].as_str().unwrap().to_string();
    let action_id = value["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target.get("action_id").and_then(|value| value.as_str()))
        .unwrap()
        .to_string();
    (lease_id, action_id)
}

#[cfg(unix)]
fn tree_digest(root: &Path) -> String {
    fn visit(root: &Path, path: &Path, rows: &mut Vec<Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            let mut row = relative.as_bytes().to_vec();
            if path.is_file() {
                row.extend(std::fs::read(&path).unwrap_or_default());
            }
            rows.push(row);
            if path.is_dir() {
                visit(root, &path, rows);
            }
        }
    }
    let mut rows = Vec::new();
    visit(root, root, &mut rows);
    request_governance::sha256(&rows.concat())
}

#[test]
fn tools_expose_read_only_route_and_separate_apply() {
    let tools = list_tools();
    assert_eq!(tools.tools.len(), 9);
    assert!(tools
        .tools
        .iter()
        .any(|tool| tool.name == TOOL_ONBOARDING_PLAN));
    let route = tools
        .tools
        .iter()
        .find(|tool| tool.name == TOOL_ROUTE_REQUEST)
        .expect("route tool");
    let capabilities = route.inputSchema["$defs"]["MachineCliTarget"]["properties"]["capability"]
        ["enum"]
        .as_array()
        .expect("capability enum");
    assert!(capabilities
        .iter()
        .any(|value| value == "task_prepare_execution"));
    assert!(capabilities.iter().any(|value| value == "skill_adopt"));
    assert!(capabilities.iter().all(|value| value != "task_execute"));
    let typed_variants = route.inputSchema["$defs"]["TypedCliInput"]["oneOf"]
        .as_array()
        .expect("typed input variants");
    assert_eq!(typed_variants.len(), 5);
    let skill_adopt = typed_variants
        .iter()
        .find(|variant| {
            variant["properties"]["kind"]["const"]
                .as_str()
                .is_some_and(|kind| kind == "skill_adopt")
        })
        .expect("skill_adopt input schema");
    assert!(skill_adopt["properties"]["host"]["enum"]
        .as_array()
        .is_some_and(|hosts| hosts.iter().any(|host| host == "omp")));
    assert!(tools
        .tools
        .iter()
        .any(|tool| tool.name == TOOL_APPLY_ACTION));
}

#[test]
fn onboarding_plan_is_public_and_holds_only_closed_actions() {
    let target =
        std::env::temp_dir().join(format!("ags-mcp-onboarding-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).unwrap();
    let binding = PreflightBinding {
        host: "codex".to_string(),
        target: target.clone(),
        host_home: target.join("home"),
    };
    let mut session = RoutingSession::default();
    let result = tool_onboarding_plan(&serde_json::json!({}), &binding, &mut session).unwrap();
    let value: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["plan"]["profile"], "public");
    assert_eq!(value["binding"], "bootstrap_required");
    assert_eq!(
        value["plan"]["excluded_capabilities"],
        serde_json::json!([])
    );
    assert!(value["actions"]
        .as_array()
        .is_some_and(|actions| !actions.is_empty()));
    assert!(value["lease"]["lease_id"].as_str().is_some());
    let _ = std::fs::remove_dir_all(target);
}

#[test]
fn legacy_raw_request_is_rejected() {
    let mut session = RoutingSession::default();
    let error = tool_route_request(
        &serde_json::json!({"request": "please route this"}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap_err();
    assert_eq!(error, "legacy_raw_request_unsupported");
}

#[test]
fn missing_proposal_fields_are_structured_and_stable() {
    let mut session = RoutingSession::default();
    let error = tool_route_request(
        &serde_json::json!({"proposal": {"schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION}}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap_err();
    let value: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(value["code"], "typed_proposal_missing_fields");
    assert!(!value["fields"].as_array().unwrap().is_empty());

    let error = tool_route_request(
        &serde_json::json!({}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap_err();
    let value: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(value["code"], "typed_proposal_missing_fields");
    assert_eq!(value["fields"], serde_json::json!(["proposal"]));
}

#[test]
fn route_rejects_fields_outside_the_typed_proposal() {
    let mut session = RoutingSession::default();
    let error = tool_route_request(
        &serde_json::json!({"proposal": direct_proposal(), "foo": "bar"}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap_err();
    let value: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(value["code"], "typed_proposal_unexpected_fields");
    assert_eq!(value["fields"], serde_json::json!(["foo"]));

    let mut nested = direct_proposal();
    nested["targets"][0]["request"] = serde_json::json!("raw text");
    let error = tool_route_request(
        &serde_json::json!({"proposal": nested}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap_err();
    assert!(error.contains("invalid_typed_proposal"));
    assert!(error.contains("unknown field"));
}

#[test]
fn malformed_route_attempt_invalidates_the_previous_lease() {
    let (base, binding, runtime, _, _) = machine_fixture("malformed-invalidation");
    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let error = tool_route_request(
        &serde_json::json!({"proposal": {"schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION}}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap_err();
    assert!(error.contains("typed_proposal_missing_fields"));
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "decision_lease_invalid_or_expired"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn direct_route_creates_no_effectful_action() {
    let mut session = RoutingSession::default();
    let output = tool_route_request(
        &serde_json::json!({"proposal": direct_proposal()}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["governance_status"], "OK");
    assert!(session.actions.is_empty());
}

#[test]
fn legacy_verify_local_is_read_only_guidance_for_project_verify_apply() {
    let output = tool_verify_local(&serde_json::json!({}), &binding()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["governance_status"], "ADVISORY_NO_MUTATION");
    assert_eq!(value["mutation_performed"], false);
    assert_eq!(value["process_launched"], false);
    assert_eq!(value["next_action"]["capability"], "project_verify");
    assert_eq!(value["next_action"]["input"]["kind"], "empty");
    assert_eq!(
        tool_verify_local(&serde_json::json!({"target": "/tmp/other"}), &binding()).unwrap_err(),
        "ags_verify_local_is_preflight_bound"
    );
}

#[test]
fn machine_mapping_is_fixed_and_shell_free() {
    let (args, stdin) = machine_invocation(
        CliCapabilityId::TaskCompile,
        &TypedCliInput::ConfirmedHandoffContract {
            content: "任务：contract".to_string(),
            handoff_source: TaskCardHandoffSource::ExplicitHandoff,
        },
        "codex",
        Path::new("."),
    )
    .unwrap();
    assert_eq!(args[0..3], ["task", "compile", "-"]);
    assert!(args.iter().any(|arg| arg == "--task-card-requested"));
    assert_eq!(stdin, "任务：contract");
}

#[test]
fn host_plan_handoff_maps_to_the_plan_final_compiler_flag() {
    let (args, stdin) = machine_invocation(
        CliCapabilityId::TaskCompile,
        &TypedCliInput::ConfirmedHandoffContract {
            content: "任务：closed plan contract".to_string(),
            handoff_source: TaskCardHandoffSource::HostPlanMode,
        },
        "codex",
        Path::new("."),
    )
    .unwrap();
    assert!(args.iter().any(|arg| arg == "--host-plan-mode-final"));
    assert!(!args.iter().any(|arg| arg == "--task-card-requested"));
    assert_eq!(stdin, "任务：closed plan contract");
}

#[test]
fn route_rejects_machine_input_kind_before_holding_an_action() {
    let mut session = RoutingSession::default();
    let mut proposal = machine_proposal();
    proposal["targets"][0]["input"] = serde_json::json!({
        "kind": "task_card",
        "content": "## 任务卡\n"
    });
    let output = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["governance_status"], "BLOCKED_BY_POLICY");
    assert_eq!(value["errors"][0]["code"], "machine_input_kind_mismatch");
    assert!(session.actions.is_empty());
}

#[test]
fn task_prepare_resolves_real_policy_before_holding_a_lease() {
    let (base, binding, runtime, _, _) = machine_fixture("prepare-policy");
    let mut proposal = machine_proposal();
    proposal["targets"][0]["capability"] = serde_json::json!("task_prepare_execution");
    proposal["targets"][0]["input"] = serde_json::json!({
        "kind": "task_card",
        "content": "## 任务卡\n"
    });
    let mut session = RoutingSession::default();
    let blocked = tool_route_request(
        &serde_json::json!({"proposal": proposal.clone()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let blocked: serde_json::Value = serde_json::from_str(&blocked).unwrap();
    assert_eq!(blocked["governance_status"], "BLOCKED_BY_POLICY");
    assert_eq!(blocked["errors"][0]["code"], "machine_policy_rejected");
    assert!(session.actions.is_empty());

    proposal["targets"][0]["input"]["content"] = serde_json::json!(valid_execution_card());
    let routed = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let routed: serde_json::Value = serde_json::from_str(&routed).unwrap();
    assert_eq!(routed["governance_status"], "OK");
    let policy_hash = routed["lease"]["policy_hash"].as_str().unwrap();
    assert!(policy_hash.starts_with("sha256:"));
    assert_ne!(policy_hash, "sha256:not-applicable");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn coexisting_skill_and_machine_records_outcome_in_the_same_apply() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_LOCK.lock().unwrap();
    let base = std::env::temp_dir().join(format!(
        "ags-mcp-skill-machine-outcome-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let runtime = base.join("runtime");
    let root = binding().target;
    let skill_id = "mcp-skill-machine-demo";
    let body = home.join(".agents/skills").join(skill_id);
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
            body.join("SKILL.md"),
            "---\nname: mcp-skill-machine-demo\ndescription: skill machine outcome\nintent_tags: [outcome]\n---\n",
        )
        .unwrap();
    skill_resolver::mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        skill_resolver::OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    let snapshot =
        skill_resolver::load_validated_snapshot_with_roots(&root, &runtime, "codex", &home)
            .unwrap()
            .0;
    let executable = base.join("fake-ags");
    let spy = base.join("spy");
    std::fs::write(
        &executable,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$@\" > \"$AGS_PROCESS_SPY\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let old_bin = std::env::var_os("AGS_CLI_BIN");
    let old_spy = std::env::var_os("AGS_PROCESS_SPY");
    std::env::set_var("AGS_CLI_BIN", &executable);
    std::env::set_var("AGS_PROCESS_SPY", &spy);
    let route_binding = PreflightBinding {
        host: "codex".to_string(),
        target: root,
        host_home: home,
    };
    let proposal = serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:skill-machine-request",
        "phase": "execution",
        "solution_state": "confirmed",
        "execution_authority": "task_card_handoff",
        "scope_hash": "sha256:scope",
        "targets": [
            {
                "kind": "skill",
                "skill_id": skill_id,
                "snapshot_hash": snapshot.snapshot_hash
            },
            {
                "kind": "machine_cli",
                "capability": "task_compile",
                "input": {
                    "kind": "confirmed_handoff_contract",
                    "content": "任务：coexisting skill and machine"
                }
            }
        ]
    });
    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &route_binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let applied = tool_apply_action(
        &serde_json::json!({
            "lease_id": lease_id,
            "action_id": action_id,
            "outcome": {"status": "succeeded", "quality": 91}
        }),
        &route_binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let applied: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert!(applied["outcome_event_id"].as_str().is_some());
    assert!(spy.exists());
    let events = skill_resolver::load_usage_events(&runtime, "codex");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].skill_id, skill_id);
    assert_eq!(events[0].outcome, skill_resolver::SkillOutcome::Succeeded);

    match old_bin {
        Some(value) => std::env::set_var("AGS_CLI_BIN", value),
        None => std::env::remove_var("AGS_CLI_BIN"),
    }
    match old_spy {
        Some(value) => std::env::set_var("AGS_PROCESS_SPY", value),
        None => std::env::remove_var("AGS_PROCESS_SPY"),
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn later_stale_skill_target_cannot_leave_an_earlier_machine_action() {
    let (base, binding, runtime, _, _) = machine_fixture("ordered-failure");
    let mut proposal = machine_proposal();
    proposal["targets"] = serde_json::json!([
        proposal["targets"][0].clone(),
        {
            "kind": "skill",
            "skill_id": "missing-skill",
            "snapshot_hash": "sha256:stale"
        }
    ]);
    let mut session = RoutingSession::default();
    let output = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["governance_status"], "BLOCKED_BY_POLICY");
    assert_eq!(value["errors"][0]["code"], "skill_snapshot_stale");
    assert!(session.actions.is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skill_tags_mapping_uses_preflight_host_and_target() {
    let target = Path::new("/tmp/ags-target");
    let (args, stdin) = machine_invocation(
        CliCapabilityId::SkillTagsVerify,
        &TypedCliInput::TaskCard {
            content: "## 任务卡\n".to_string(),
        },
        "codex",
        target,
    )
    .unwrap();
    assert_eq!(
        args,
        vec![
            "gate",
            "skill-tags",
            "-",
            "--target",
            "/tmp/ags-target",
            "--for",
            "codex",
            "--format",
            "json"
        ]
    );
    assert_eq!(stdin, "## 任务卡\n");
}

#[test]
fn skill_adopt_mapping_uses_only_typed_fields_and_fixed_argv() {
    let source = "https://github.com/acme/skills/tree/main/apple-design;touch /tmp/pwn";
    let (args, stdin) = machine_invocation(
        CliCapabilityId::SkillAdopt,
        &TypedCliInput::SkillAdopt {
            source: source.to_string(),
            host: "all".to_string(),
            apply: true,
        },
        "codex",
        Path::new("/tmp/ags-target"),
    )
    .unwrap();
    assert_eq!(
        args,
        vec!["skill", "adopt", "--host", "all", "--format", "json", "--apply", "--", source]
    );
    assert!(stdin.is_empty());
}

#[cfg(unix)]
#[test]
fn route_is_side_effect_free_and_apply_uses_only_fixed_argv_once() {
    let _guard = ENV_LOCK.lock().unwrap();
    let (base, binding, runtime, executable, spy) = machine_fixture("fixed-argv");
    let old_bin = std::env::var_os("AGS_CLI_BIN");
    let old_spy = std::env::var_os("AGS_PROCESS_SPY");
    std::env::set_var("AGS_CLI_BIN", &executable);
    std::env::set_var("AGS_PROCESS_SPY", &spy);

    let before = tree_digest(&base);
    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    assert_eq!(tree_digest(&base), before);
    assert!(!spy.exists(), "route must not launch the fake executable");

    let (lease_id, action_id) = route_action(&route);
    let applied = tool_apply_action(
        &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let result: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert_eq!(result["governance_status"], "HOST_EXECUTION_REQUIRED");
    let argv = std::fs::read_to_string(&spy).unwrap();
    assert_eq!(
        argv.lines().collect::<Vec<_>>(),
        vec![
            "task",
            "compile",
            "-",
            "--format",
            "json",
            "--output",
            "report",
            "--task-card-requested",
            "--confirmed-handoff-contract"
        ]
    );
    assert_eq!(
        std::fs::canonicalize(
            std::fs::read_to_string(format!("{}.cwd", spy.display()))
                .unwrap()
                .trim()
        )
        .unwrap(),
        std::fs::canonicalize(&binding.target).unwrap()
    );
    let replay = tool_apply_action(
        &serde_json::json!({
            "lease_id": result["lease_id"],
            "action_id": result["action_id"]
        }),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap_err();
    assert_eq!(replay, "decision_lease_invalid_or_consumed");

    match old_bin {
        Some(value) => std::env::set_var("AGS_CLI_BIN", value),
        None => std::env::remove_var("AGS_CLI_BIN"),
    }
    match old_spy {
        Some(value) => std::env::set_var("AGS_PROCESS_SPY", value),
        None => std::env::remove_var("AGS_PROCESS_SPY"),
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn new_route_and_new_connection_invalidate_old_lease() {
    let (base, binding, runtime, _, _) = machine_fixture("invalidation");
    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    tool_route_request(
        &serde_json::json!({"proposal": direct_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "decision_lease_invalid_or_expired"
    );

    let mut first_connection = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut first_connection,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let mut second_connection = RoutingSession::default();
    let second_route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut second_connection,
        &runtime,
    )
    .unwrap();
    let (second_lease_id, second_action_id) = route_action(&second_route);
    assert_ne!(lease_id, second_lease_id);
    assert_ne!(action_id, second_action_id);
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &binding,
            &mut second_connection,
            &runtime,
        )
        .unwrap_err(),
        "decision_lease_invalid_or_expired"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn shape_failure_is_non_consuming_but_binding_and_registry_failures_consume() {
    let (base, binding, runtime, _, _) = machine_fixture("tamper");

    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({
                "lease_id": lease_id,
                "action_id": action_id,
                "argv": ["arbitrary"]
            }),
            &binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "held_action_tampering_rejected"
    );
    assert!(
        !session.actions.get(&action_id).unwrap().consumed,
        "shape-invalid input must be repairable before the lease consumption point"
    );

    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let wrong_binding = PreflightBinding {
        host: "claude-code".to_string(),
        target: binding.target.clone(),
        host_home: binding.host_home.clone(),
    };
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &wrong_binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "preflight_binding_conflict"
    );

    let route = tool_route_request(
        &serde_json::json!({"proposal": machine_proposal()}),
        &binding,
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let registry_path = binding.target.join("manifests/skills-registry.yaml");
    let original = std::fs::read(&registry_path).unwrap();
    std::fs::write(&registry_path, "skills: []\ndemand_routes: []\n# changed\n").unwrap();
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "decision_lease_registry_hash_mismatch"
    );
    std::fs::write(&registry_path, original).unwrap();
    assert_eq!(
        tool_apply_action(
            &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
            &binding,
            &mut session,
            &runtime,
        )
        .unwrap_err(),
        "decision_lease_invalid_or_consumed"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn confirmed_direct_edit_is_host_native_without_task_replanning() {
    let mut session = RoutingSession::default();
    let proposal = serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:req",
        "phase": "execution",
        "solution_state": "confirmed",
        "execution_authority": "direct_edit",
        "scope_hash": "sha256:scope",
        "targets": []
    });
    let output = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &binding(),
        &mut session,
        &std::env::temp_dir(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["governance_status"], "HOST_EXECUTION_REQUIRED");
    assert_eq!(value["resolved_targets"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["resolved_targets"][0]["kind"],
        "host_native_direct_edit"
    );
    assert!(session.actions.is_empty());
}

#[cfg(unix)]
#[test]
fn skill_outcome_is_written_only_through_apply_without_sensitive_fields() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_LOCK.lock().unwrap();
    let base = std::env::temp_dir().join(format!("ags-mcp-outcome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let runtime = base.join("runtime");
    let root = binding().target;
    let skill_id = "mcp-outcome-demo";
    let body = home.join(".agents/skills").join(skill_id);
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
            body.join("SKILL.md"),
            "---\nname: mcp-outcome-demo\ndescription: Records a controlled outcome.\nintent_tags: [outcome-demo]\n---\nbody\n",
        )
        .unwrap();
    skill_resolver::mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        skill_resolver::OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    let snapshot =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
            .unwrap();
    assert!(
        snapshot
            .active_skills
            .iter()
            .any(|skill| skill.skill_id == skill_id),
        "adopted ready candidate must enter the active table: {:?}",
        snapshot
            .catalog
            .iter()
            .find(|card| card.skill_id == skill_id)
    );
    skill_resolver::write_private_atomic(
        &skill_resolver::snapshot_path(&runtime, "codex"),
        serde_json::to_string_pretty(&snapshot).unwrap().as_bytes(),
    )
    .unwrap();

    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    let proposal = serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:non-sensitive-request-fingerprint",
        "phase": "execution",
        "solution_state": "confirmed",
        "execution_authority": "direct_edit",
        "scope_hash": "sha256:scope",
        "targets": [{
            "kind": "skill",
            "skill_id": skill_id,
            "snapshot_hash": snapshot.snapshot_hash
        }]
    });
    let mut session = RoutingSession::default();
    let route = tool_route_request(
        &serde_json::json!({"proposal": proposal}),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let route_value: serde_json::Value = serde_json::from_str(&route).unwrap();
    let lease_id = route_value["lease"]["lease_id"].as_str().unwrap();
    let outcome_action = route_value["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["action_kind"] == "skill_outcome")
        .and_then(|target| target["action_id"].as_str())
        .unwrap();
    let applied = tool_apply_action(
        &serde_json::json!({
            "lease_id": lease_id,
            "action_id": outcome_action,
            "outcome": {"status": "succeeded", "quality": 87}
        }),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let applied: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert_eq!(applied["governance_status"], "DONE_WITH_RECEIPT");
    let usage = skill_resolver::usage_path(&runtime, "codex");
    let line = std::fs::read_to_string(&usage).unwrap();
    let event: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(event["skill_id"], skill_id);
    assert_eq!(event["outcome"], "succeeded");
    assert!(event.get("raw_prompt").is_none());
    assert!(event.get("credential").is_none());
    assert!(!line.contains(&home.to_string_lossy().to_string()));
    assert_eq!(
        std::fs::metadata(&usage).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let solution_proposal = serde_json::json!({
        "schema_version": HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
        "request_fingerprint": "sha256:non-sensitive-request-fingerprint",
        "phase": "solution_formation",
        "solution_state": "open",
        "execution_authority": "none",
        "scope_hash": "sha256:scope",
        "targets": [{
            "kind": "skill",
            "skill_id": skill_id,
            "snapshot_hash": snapshot.snapshot_hash
        }]
    });
    let route = tool_route_request(
        &serde_json::json!({"proposal": solution_proposal.clone()}),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    tool_apply_action(
        &serde_json::json!({
            "lease_id": lease_id,
            "action_id": action_id,
            "outcome": {"status": "abandoned"}
        }),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let events = skill_resolver::load_usage_events(&runtime, "codex");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].outcome, skill_resolver::SkillOutcome::Abandoned);
    assert_eq!(events[0].request_fingerprint, events[1].request_fingerprint);

    let route = tool_route_request(
        &serde_json::json!({"proposal": solution_proposal}),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let (lease_id, action_id) = route_action(&route);
    let error = tool_apply_action(
        &serde_json::json!({
            "lease_id": lease_id,
            "action_id": action_id,
            "outcome": {"status": "failed", "raw_prompt": "must never be stored"}
        }),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root.clone(),
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap_err();
    assert!(error.contains("invalid_outcome"));
    let corrected = tool_apply_action(
        &serde_json::json!({
            "lease_id": lease_id,
            "action_id": action_id,
            "outcome": {"status": "failed"}
        }),
        &PreflightBinding {
            host: "codex".to_string(),
            target: root,
            host_home: home.clone(),
        },
        &mut session,
        &runtime,
    )
    .unwrap();
    let corrected: serde_json::Value = serde_json::from_str(&corrected).unwrap();
    assert_eq!(corrected["governance_status"], "DONE_WITH_RECEIPT");
    assert_eq!(
        skill_resolver::load_usage_events(&runtime, "codex").len(),
        3
    );

    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    let _ = std::fs::remove_dir_all(base);
}
