use super::*;
#[allow(unused_imports)]
use super::{decision::*, preflight::*, wire::*};
#[cfg(test)]
pub(super) fn tool_apply_action(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
) -> Result<String, String> {
    tool_apply_action_with_source(args, binding, session, runtime_home, None)
}

pub(super) fn tool_apply_action_with_source(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
    capability_source: Option<&dyn CapabilityCatalogSource>,
) -> Result<String, String> {
    let lease_id = get_string(args, "lease_id")?;
    let action_id = get_string(args, "action_id")?;
    let generation = session.generation;
    {
        let action = session
            .actions
            .get(&action_id)
            .ok_or_else(|| "decision_lease_invalid_or_expired".to_string())?;
        if action.consumed || action.evidence.lease_id != lease_id {
            return Err("decision_lease_invalid_or_consumed".to_string());
        }
        validate_apply_shape(args, &action.kind)?;

        if action.evidence.host != binding.host
            || Path::new(&action.evidence.target) != binding.target
        {
            return Err("preflight_binding_conflict".to_string());
        }
        let policy_hash = match &action.kind {
            HeldActionKind::Machine {
                capability,
                input,
                skill_outcome,
            } => machine_policy_hash(*capability, input, skill_outcome.as_ref())?,
            HeldActionKind::RecordOutcome {
                request_fingerprint,
                skill_id,
                entrypoint,
            } => outcome_policy_hash(request_fingerprint, skill_id, entrypoint.as_deref()),
            HeldActionKind::Onboarding {
                plan_hash,
                item_id,
                action,
            } => onboarding_policy_hash(plan_hash, item_id, action),
        };
        if policy_hash != action.policy_hash || policy_hash != action.evidence.policy_hash {
            return Err("decision_lease_policy_hash_mismatch".to_string());
        }
        if !matches!(action.kind, HeldActionKind::Onboarding { .. }) {
            if capability_source.is_some() {
                let failure = binding
                    .capability
                    .as_ref()
                    .and_then(ags_session::CapabilityReference::as_failure)
                    .or_else(|| {
                        binding
                            .capability
                            .is_none()
                            .then_some(ags_session::CapabilityLoadFailure::SnapshotStale)
                    });
                if let Some(error) = failure {
                    return Err(error.into_legacy_error());
                }
            }
            let authority_root = ags_capability_governance::resolve_capability_authority_root(
                &binding.target,
                runtime_home,
                std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
            )
            .map_err(|error| error.to_string())?;
            let registry = std::fs::read(authority_root.join("manifests/skills-registry.yaml"))
                .map_err(|error| error.to_string())?;
            if ags_capability_governance::sha256(&registry) != action.evidence.registry_hash {
                return Err("decision_lease_registry_hash_mismatch".to_string());
            }
            let catalog = capability_source
                .map_or_else(
                    || {
                        ags_session::LocalCapabilityCatalogSource::new(runtime_home.to_path_buf())
                            .load_validated_snapshot(binding)
                    },
                    |source| source.load_validated_snapshot(binding),
                )
                .map_err(ags_session::CapabilityLoadFailure::into_legacy_error)?;
            if capability_source.is_some()
                && binding
                    .capability
                    .as_ref()
                    .and_then(ags_session::CapabilityReference::ready_binding)
                    != Some(&catalog.binding)
            {
                return Err("decision_lease_preflight_capability_binding_stale".to_string());
            }
            if catalog.snapshot.snapshot_hash != action.evidence.snapshot_hash {
                return Err("decision_lease_snapshot_hash_mismatch".to_string());
            }
        }
    }

    // Only the effect boundary consumes the one-shot lease. Everything above is
    // admission and can be retried after correcting transient bindings, hashes,
    // catalogs, snapshots, or input. Once invocation begins, success or failure
    // is final and replay stays fail-closed for every action on this lease.
    for held in session.actions.values_mut() {
        if held.evidence.lease_id == lease_id {
            held.consumed = true;
        }
    }
    let action = session
        .actions
        .get(&action_id)
        .expect("validated held action remains daemon-client-session-local");

    let (machine_result, onboarding_result, outcome_event_id, status, requires_repreflight) =
        match &action.kind {
            HeldActionKind::Machine {
                capability,
                input,
                skill_outcome,
            } => {
                let outcome = match (skill_outcome, args.get("outcome")) {
                    (Some(_), Some(value)) => Some(
                        serde_json::from_value::<OutcomeInput>(value.clone())
                            .expect("apply shape validated machine outcome"),
                    ),
                    (None, Some(_)) => unreachable!("apply shape rejected unexpected outcome"),
                    (_, None) => None,
                };
                let result = invoke_machine_cli(
                    *capability,
                    input,
                    &binding.host,
                    &binding.target,
                    runtime_home,
                )?;
                let status = if result.success {
                    if capability.is_handoff_capability() {
                        GovernanceStatus::HostExecutionRequired
                    } else {
                        GovernanceStatus::Ok
                    }
                } else {
                    GovernanceStatus::BlockedByPolicy
                };
                let outcome_event_id = match (skill_outcome, outcome) {
                    (Some(skill), Some(outcome)) => Some(append_outcome_event(
                        runtime_home,
                        binding,
                        action,
                        generation,
                        skill,
                        outcome,
                        &session.connection_nonce,
                    )?),
                    _ => None,
                };
                (Some(result), None, outcome_event_id, status, false)
            }
            HeldActionKind::RecordOutcome {
                request_fingerprint,
                skill_id,
                entrypoint,
            } => {
                let outcome: OutcomeInput = serde_json::from_value(
                    args.get("outcome")
                        .cloned()
                        .expect("apply shape required an outcome"),
                )
                .expect("apply shape validated recorded outcome");
                let event_id = append_outcome_event(
                    runtime_home,
                    binding,
                    action,
                    generation,
                    &SkillOutcomeBinding {
                        request_fingerprint: request_fingerprint.clone(),
                        skill_id: skill_id.clone(),
                        entrypoint: entrypoint.clone(),
                    },
                    outcome,
                    &session.connection_nonce,
                )?;
                (
                    None,
                    None,
                    Some(event_id),
                    GovernanceStatus::DoneWithReceipt,
                    false,
                )
            }
            HeldActionKind::Onboarding {
                item_id,
                action: onboarding_action,
                ..
            } => {
                debug_assert!(args.get("outcome").is_none());
                let result = invoke_onboarding_action(item_id, onboarding_action)?;
                let mut result = result;
                result.receipt_path =
                    emit_onboarding_receipt(runtime_home, binding, action, &result).ok();
                let status = if result.success && result.receipt_path.is_some() {
                    GovernanceStatus::DoneWithReceipt
                } else if result.success {
                    GovernanceStatus::Ok
                } else {
                    GovernanceStatus::BlockedByPolicy
                };
                (None, Some(result), None, status, true)
            }
        };
    pretty(&ApplyResult {
        schema_version: "0.3.0-apply-result",
        governance_status: status,
        lease_id,
        action_id,
        consumed: true,
        machine_result,
        onboarding_result,
        outcome_event_id,
        requires_repreflight,
    })
}

pub(super) fn validate_apply_shape(
    args: &serde_json::Value,
    kind: &HeldActionKind,
) -> Result<(), String> {
    let unexpected_fields = args
        .as_object()
        .map(|object| {
            object
                .keys()
                .any(|key| !matches!(key.as_str(), "lease_id" | "action_id" | "outcome"))
        })
        .unwrap_or(true);
    if unexpected_fields {
        return Err("held_action_tampering_rejected".to_string());
    }

    match (kind, args.get("outcome")) {
        (
            HeldActionKind::Machine {
                skill_outcome: Some(_),
                ..
            }
            | HeldActionKind::RecordOutcome { .. },
            Some(value),
        ) => {
            serde_json::from_value::<OutcomeInput>(value.clone())
                .map_err(|error| format!("invalid_outcome: {error}"))?;
            Ok(())
        }
        (HeldActionKind::RecordOutcome { .. }, None) => Err("outcome_required".to_string()),
        (
            HeldActionKind::Machine {
                skill_outcome: None,
                ..
            },
            Some(_),
        ) => Err("outcome_not_allowed_for_machine_action".to_string()),
        (HeldActionKind::Onboarding { .. }, Some(_)) => {
            Err("outcome_not_allowed_for_onboarding_action".to_string())
        }
        _ => Ok(()),
    }
}

pub(super) fn append_outcome_event(
    runtime_home: &Path,
    binding: &PreflightBinding,
    action: &HeldAction,
    generation: u64,
    skill: &SkillOutcomeBinding,
    outcome: OutcomeInput,
    connection_nonce: &str,
) -> Result<String, String> {
    let event_id = stable_id("outcome", &action.action_id, connection_nonce, generation);
    let event = ags_capability_governance::SkillUsageEvent {
        schema_version: ags_capability_governance::SKILL_USAGE_EVENT_SCHEMA_VERSION.to_string(),
        event_id: event_id.clone(),
        timestamp_unix: unix_timestamp(),
        request_fingerprint: skill.request_fingerprint.clone(),
        proposal_id: action.evidence.proposal_hash.clone(),
        decision_id: action.evidence.decision_id.clone(),
        lease_id: action.evidence.lease_id.clone(),
        skill_id: skill.skill_id.clone(),
        entrypoint: skill.entrypoint.clone(),
        outcome: outcome.status,
        quality: outcome.quality,
    };
    ags_capability_governance::append_usage_event(runtime_home, &binding.host, &event)?;
    Ok(event_id)
}

pub(super) fn invoke_onboarding_action(
    item_id: &str,
    action: &ags_lifecycle::OnboardingAction,
) -> Result<OnboardingExecutionResult, String> {
    let executable = std::env::var_os("AGS_CLI_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| "cannot resolve current AGS executable".to_string())?;
    let output = ags_lifecycle::execute_action(action, &executable)?;
    Ok(OnboardingExecutionResult {
        item_id: item_id.to_string(),
        success: output.success,
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
        receipt_path: None,
    })
}

pub(super) fn emit_onboarding_receipt(
    runtime_home: &Path,
    binding: &PreflightBinding,
    held: &HeldAction,
    result: &OnboardingExecutionResult,
) -> Result<String, String> {
    let rollback = match &held.kind {
        HeldActionKind::Onboarding { action, .. } => {
            let steps = ags_lifecycle::rollback_advice(action)
                .into_iter()
                .map(|advice| ags_evidence::RollbackStep {
                    affected_path: advice.affected_path,
                    inverse_op: "manual-confirm".to_string(),
                    backup_path: None,
                    inverse_command: advice.inverse_command,
                    detail: advice.detail,
                })
                .collect();
            ags_evidence::RollbackPlan::manual_confirm(steps)
        }
        _ => ags_evidence::RollbackPlan::none(),
    };
    let receipt = ags_evidence::build_action_receipt(
        "mcp-onboarding-apply",
        Some(&binding.target.to_string_lossy()),
        ags_evidence::GateResult {
            decision: if result.success { "allow" } else { "stop" }.to_string(),
            reason: (!result.success).then(|| format!("onboarding item {} failed", result.item_id)),
        },
        vec![],
        vec![],
        vec![],
        vec![ags_evidence::VerificationResult {
            command: format!("ags_apply_action onboarding item {}", result.item_id),
            exit_code: result.exit_code.unwrap_or(1),
            output_hash: ags_evidence::sha256_hex(
                format!("{}\n{}", result.stdout, result.stderr).as_bytes(),
            ),
        }],
        rollback,
        if result.success { "applied" } else { "failed" },
        result.success,
    );
    ags_evidence::emit_action_receipt(&runtime_home.join("receipts"), &receipt)
        .map(|path| path.display().to_string())
        .map_err(|error| format!("onboarding receipt failed for {}: {error}", held.action_id))
}

#[derive(Debug, Serialize)]
pub(super) struct MachineCliResult {
    pub(super) capability: CliCapabilityId,
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) fn invoke_machine_cli(
    capability: CliCapabilityId,
    input: &TypedCliInput,
    host: &str,
    target: &Path,
    _runtime_home: &Path,
) -> Result<MachineCliResult, String> {
    let executable = std::env::var_os("AGS_CLI_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| "cannot resolve current AGS executable".to_string())?;
    let (arguments, stdin) = machine_invocation(capability, input, host, target)?;
    let mut child = Command::new(executable)
        .args(&arguments)
        .current_dir(target)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("MachineCli spawn failed: {error}"))?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin.as_bytes())
            .map_err(|error| format!("MachineCli stdin failed: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("MachineCli wait failed: {error}"))?;
    Ok(MachineCliResult {
        capability,
        success: output.status.success(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub(super) fn machine_invocation(
    capability: CliCapabilityId,
    input: &TypedCliInput,
    host: &str,
    target: &Path,
) -> Result<(Vec<String>, String), String> {
    validate_machine_input(capability, input)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let stdin = match input {
        TypedCliInput::ConfirmedHandoffContract { content, .. }
        | TypedCliInput::TaskCard { content }
        | TypedCliInput::Receipt { content } => content.clone(),
        TypedCliInput::SkillAdopt { .. } | TypedCliInput::Empty => String::new(),
    };
    let args = match capability {
        CliCapabilityId::TaskCompile => {
            let handoff_flag = match input {
                TypedCliInput::ConfirmedHandoffContract {
                    handoff_source: TaskCardHandoffSource::HostPlanMode,
                    ..
                } => "--host-plan-mode-final",
                _ => "--task-card-requested",
            };
            vec![
                "task",
                "compile",
                "-",
                "--format",
                "json",
                "--output",
                "report",
                handoff_flag,
                "--confirmed-handoff-contract",
            ]
        }
        CliCapabilityId::TaskPrepareExecution => {
            vec!["run", "-", "--check-only", "--format", "json"]
        }
        CliCapabilityId::TaskValidate => vec!["task", "validate", "-"],
        CliCapabilityId::PolicyResolve => vec!["policy", "resolve", "-", "--format", "json"],
        CliCapabilityId::ProjectVerify => {
            return Ok((
                vec![
                    "verify".to_string(),
                    "--scope".to_string(),
                    "local".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    "--target".to_string(),
                    target.to_string_lossy().into_owned(),
                ],
                stdin,
            ));
        }
        CliCapabilityId::SkillTagsVerify => {
            return Ok((
                vec![
                    "gate".to_string(),
                    "skill-tags".to_string(),
                    "-".to_string(),
                    "--target".to_string(),
                    target.to_string_lossy().into_owned(),
                    "--for".to_string(),
                    host.to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                ],
                stdin,
            ));
        }
        CliCapabilityId::SkillAdopt => {
            let TypedCliInput::SkillAdopt {
                source,
                host,
                apply,
            } = input
            else {
                unreachable!("validated SkillAdopt input kind");
            };
            let mut args = vec![
                "skill".to_string(),
                "adopt".to_string(),
                "--host".to_string(),
                host.clone(),
                "--format".to_string(),
                "json".to_string(),
            ];
            if *apply {
                args.push("--apply".to_string());
            }
            args.push("--".to_string());
            args.push(source.clone());
            return Ok((args, stdin));
        }
        CliCapabilityId::ReceiptVerify => vec!["receipt", "verify", "-", "--format", "json"],
    };
    Ok((args.into_iter().map(str::to_string).collect(), stdin))
}

pub(super) fn stable_id(
    prefix: &str,
    basis: &str,
    connection_nonce: &str,
    generation: u64,
) -> String {
    let digest = sha256(format!("{connection_nonce}\n{generation}\n{basis}").as_bytes());
    format!(
        "{prefix}-{}",
        digest
            .trim_start_matches("sha256:")
            .get(..20)
            .unwrap_or("invalid")
    )
}

pub(super) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn get_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing required string argument: {key}"))
}

pub(super) fn bool_arg(args: &serde_json::Value, key: &str) -> bool {
    args.get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(super) fn get_target(args: &serde_json::Value) -> PathBuf {
    args.get("target")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn pretty<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(json_error)
}

pub(super) fn json_error(error: serde_json::Error) -> String {
    format!("JSON serialize error: {error}")
}
