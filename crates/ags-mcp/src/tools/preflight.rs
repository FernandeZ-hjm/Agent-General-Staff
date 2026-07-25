use super::*;
#[allow(unused_imports)]
use super::{apply::*, decision::*, wire::*};
pub(super) fn tool_preflight(
    args: &serde_json::Value,
    capability_source: Option<&dyn CapabilityCatalogSource>,
) -> Result<String, String> {
    let agent = get_string(args, "agent")?;
    let agent_type = project_discovery::AgentType::from_str(&agent)
        .map_err(|error| format!("Invalid agent: {error}"))?;
    let target = get_target(args);
    let report = project_discovery::run_session_preflight(&target, &agent_type);
    let resolved_target = report.target.clone();
    let mut value = serde_json::to_value(report).map_err(json_error)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("agent".to_string(), serde_json::json!(agent_type.as_str()));
    }
    let capability = capability_source.map_or_else(
        || capability_reference(&resolved_target, agent_type.as_str()),
        |source| source.capability_reference(&resolved_target, agent_type.as_str()),
    );
    attach_capability_catalog(&mut value, capability);
    pretty(&value)
}

pub(super) fn attach_capability_catalog(
    report: &mut serde_json::Value,
    capability: serde_json::Value,
) {
    let Some(object) = report.as_object_mut() else {
        return;
    };
    let snapshot_stale = capability
        .get("status")
        .and_then(|status| status.as_str())
        .is_some_and(|status| status == "snapshot_stale");
    object.insert("capability_catalog".to_string(), capability);
    if !snapshot_stale {
        return;
    }

    let already_stopped = object
        .get("overall_status")
        .and_then(|status| status.as_str())
        .is_some_and(|status| status == "stop");
    if !already_stopped {
        object.insert("overall_status".to_string(), serde_json::json!("warning"));
        object.insert(
            "governance_status".to_string(),
            serde_json::json!(GovernanceStatus::NeedsUserDecision),
        );
    }

    let warning = "Host capability snapshot is stale; DirectResponse remains available, but SkillTarget and MachineCli routing are blocked until an explicit refresh and re-preflight.";
    let warnings = object
        .entry("warnings".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if let Some(warnings) = warnings.as_array_mut() {
        if !warnings
            .iter()
            .any(|entry| entry.as_str().is_some_and(|entry| entry == warning))
        {
            warnings.push(serde_json::json!(warning));
        }
    }

    let mut existing_steps = object
        .get("next_steps")
        .and_then(|steps| steps.as_array())
        .cloned()
        .unwrap_or_default();
    existing_steps.retain(|step| {
        step.as_str()
            .is_none_or(|text| !text.contains("All clear") && !text.contains("may execute tasks"))
    });
    let mut next_steps = vec![
        serde_json::json!(
            "⚠ Capability snapshot refresh requires user confirmation before governed routing."
        ),
        serde_json::json!(
            "  Review capability_catalog.refresh.argv, run it explicitly, then rerun ags_preflight."
        ),
    ];
    next_steps.append(&mut existing_steps);
    object.insert(
        "next_steps".to_string(),
        serde_json::Value::Array(next_steps),
    );
}

pub(super) fn capability_reference(target: &Path, host: &str) -> serde_json::Value {
    let runtime_home = skill_resolver::locate_runtime_home();
    let authority = skill_resolver::resolve_capability_authority_root(
        target,
        &runtime_home,
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    );
    let loaded = authority
        .as_ref()
        .ok()
        .and_then(|root| skill_resolver::load_validated_snapshot(root, &runtime_home, host).ok());
    if let Some((snapshot, _)) = loaded {
        serde_json::json!({
            "uri": CURRENT_HOST_CAPABILITIES_URI,
            "status": "ready",
            "snapshot_hash": snapshot.snapshot_hash,
            "refresh_required": false
        })
    } else {
        serde_json::json!({
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
                    host,
                    "--target",
                    target.to_string_lossy(),
                    "--write"
                ],
                "requires_repreflight": true
            }
        })
    }
}

pub(super) fn tool_protocol_status(args: &serde_json::Value) -> Result<String, String> {
    pretty(&project_discovery::check_protocol_status(&get_target(args)))
}

pub(super) fn tool_agent_instructions(args: &serde_json::Value) -> Result<String, String> {
    let agent = get_string(args, "agent")?;
    let agent_type = project_discovery::AgentType::from_str(&agent)
        .map_err(|error| format!("Invalid agent: {error}"))?;
    pretty(&project_discovery::generate_agent_instructions(
        &get_target(args),
        &agent_type,
    ))
}

#[derive(Debug, Serialize)]
pub(super) struct OnboardingActionRef {
    item_id: String,
    action_id: String,
}

#[derive(Debug, Serialize)]
pub(super) struct OnboardingPlanResult {
    schema_version: &'static str,
    governance_status: GovernanceStatus,
    binding: &'static str,
    plan: ags_onboarding::OnboardingPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    lease: Option<DecisionLeaseEvidence>,
    actions: Vec<OnboardingActionRef>,
}

pub(super) fn tool_onboarding_plan(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
) -> Result<String, String> {
    if args
        .as_object()
        .map(|object| !object.is_empty())
        .unwrap_or(true)
    {
        return Err("ags_onboarding_plan_accepts_no_arguments".to_string());
    }
    session.invalidate();
    let source_root = std::env::var_os("AGS_SOURCE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|root| root.join("manifests/onboarding-public.yaml").is_file())
        })
        .unwrap_or_else(|| binding.target.clone());
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot resolve AGS executable: {error}"))?;
    let third_party = ags_onboarding::manifest::resolve_third_party_manifest(&source_root)?;
    let active_skill_ids = if source_root.join("manifests/skills-registry.yaml").is_file() {
        skill_resolver::build_capability_snapshot_with_roots_and_manifest(
            &source_root,
            &binding.host,
            &skill_resolver::locate_runtime_home(),
            &binding.host_home,
            &third_party,
        )
        .map_err(|error| format!("skill snapshot build failed: {error:?}"))?
        .active_skills
        .into_iter()
        .map(|skill| skill.skill_id)
        .collect::<Vec<_>>()
    } else {
        // Restricted bootstrap bindings may not have a capability authority
        // yet. Fail closed: no skill is claimed active, while native host paths
        // can still be reported as visible-but-not-ready.
        Vec::new()
    };
    let plan = ags_onboarding::assess_public_with_resolution(
        &ags_onboarding::AssessContext {
            source_root: &source_root,
            home: &binding.host_home,
            target: &binding.target,
            host: &binding.host,
            ags_executable: &executable,
            mcp_connected: true,
            host_registered: Some(true),
            registered_mcp_ids: &[],
            active_skill_ids: &active_skill_ids,
        },
        &third_party,
    )?;
    let mut actions = Vec::new();
    for item in &plan.items {
        if let Some(action) = item.action.clone() {
            let held = hold_onboarding_action(session, binding, &plan.plan_hash, &item.id, action);
            actions.push(OnboardingActionRef {
                item_id: item.id.clone(),
                action_id: held.action_id.clone(),
            });
        }
    }
    let lease = session
        .actions
        .values()
        .next()
        .map(|action| action.evidence.clone());
    pretty(&OnboardingPlanResult {
        schema_version: "0.3.0-onboarding-plan-result",
        governance_status: if plan.bootstrap_required {
            GovernanceStatus::NeedsUserDecision
        } else {
            GovernanceStatus::Ok
        },
        binding: if plan.bootstrap_required {
            "bootstrap_required"
        } else {
            "active"
        },
        plan,
        lease,
        actions,
    })
}

pub(super) fn tool_task_validate(args: &serde_json::Value) -> Result<String, String> {
    let task_card = get_string(args, "task_card")?;
    let errors = task_card_validator::validate(&task_card);
    pretty(&serde_json::json!({
        "is_valid": errors.is_empty(),
        "error_count": errors.len(),
        "errors": errors,
    }))
}

pub(super) fn tool_policy_resolve(args: &serde_json::Value) -> Result<String, String> {
    let task_card = get_string(args, "task_card")?;
    let errors = task_card_validator::validate(&task_card);
    if !errors.is_empty() {
        return pretty(&serde_json::json!({
            "resolved": false,
            "validation_error": true,
            "validation_errors": errors,
        }));
    }
    let parsed = task_card_validator::parse_validated(&task_card)
        .map_err(|error| format!("Parse error: {error:?}"))?;
    let input = execution_policy::TaskPolicyInput::from_fields_with_approval(
        &parsed.fields,
        bool_arg(args, "approve_writes"),
        bool_arg(args, "current_task_approval"),
    );
    pretty(&execution_policy::resolve_policy(input))
}

pub(super) fn tool_verify_local(
    args: &serde_json::Value,
    binding: &PreflightBinding,
) -> Result<String, String> {
    if args
        .as_object()
        .map(|object| !object.is_empty())
        .unwrap_or(true)
    {
        return Err("ags_verify_local_is_preflight_bound".to_string());
    }
    pretty(&serde_json::json!({
        "schema_version": "0.3.0-read-only-verification-guidance",
        "governance_status": GovernanceStatus::AdvisoryNoMutation,
        "host": binding.host,
        "target": binding.target.to_string_lossy(),
        "mutation_performed": false,
        "process_launched": false,
        "next_action": {
            "kind": "machine_cli",
            "capability": CliCapabilityId::ProjectVerify,
            "input": TypedCliInput::Empty
        },
        "instruction": "submit the fixed ProjectVerify target through ags_route_request, then consume its connection-held action with ags_apply_action"
    }))
}
