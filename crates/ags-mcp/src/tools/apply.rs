use super::*;
#[allow(unused_imports)]
use super::{decision::*, preflight::*, wire::*};
pub(super) fn tool_apply_action(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
) -> Result<String, String> {
    let lease_id = get_string(args, "lease_id")?;
    let action_id = get_string(args, "action_id")?;
    {
        let action = session
            .get(&action_id)
            .ok_or_else(|| "decision_lease_invalid_or_expired".to_string())?;
        if action.consumed || action.evidence.lease_id != lease_id {
            return Err("decision_lease_invalid_or_consumed".to_string());
        }
        validate_apply_shape(args, &action.kind)?;
    }

    // Only the effect boundary consumes the one-shot lease. Everything above is
    // admission and can be retried after correcting transient bindings, hashes,
    // catalogs, snapshots, or input. Once invocation begins, success or failure
    // is final and replay stays fail-closed for every action on this lease.
    for held in session.values_mut() {
        if held.evidence.lease_id == lease_id {
            held.consumed = true;
        }
    }
    let action = session
        .get(&action_id)
        .expect("validated held action remains daemon-client-session-local");
    let (machine_result, onboarding_result, outcome_accepted, status, requires_repreflight) =
        match &action.kind {
            HeldActionKind::Machine {
                capability,
                input,
                skill_outcome,
            } => {
                let outcome_accepted = skill_outcome.is_some() && args.get("outcome").is_some();
                let result =
                    invoke_machine_cli(*capability, input, &binding.host, &binding.target)?;
                let status = if result.success {
                    if capability.is_handoff_capability() {
                        GovernanceStatus::HostExecutionRequired
                    } else {
                        GovernanceStatus::Ok
                    }
                } else {
                    GovernanceStatus::BlockedByPolicy
                };
                (Some(result), None, outcome_accepted, status, false)
            }
            HeldActionKind::RecordOutcome { .. } => (None, None, true, GovernanceStatus::Ok, false),
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
                (None, Some(result), false, status, true)
            }
        };
    pretty(&ApplyResult {
        schema_version: "0.3.4-apply-result",
        governance_status: status,
        lease_id,
        action_id,
        consumed: true,
        machine_result,
        onboarding_result,
        outcome_accepted,
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
            let outcome = serde_json::from_value::<OutcomeInput>(value.clone())
                .map_err(|error| format!("invalid_outcome: {error}"))?;
            if outcome.quality.is_some_and(|quality| quality > 100) {
                return Err("invalid_outcome: quality must be in 0..=100".to_string());
            }
            let _ = outcome.status;
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
        TypedCliInput::Empty => String::new(),
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
        CliCapabilityId::ReceiptVerify => vec!["receipt", "verify", "-", "--format", "json"],
    };
    Ok((args.into_iter().map(str::to_string).collect(), stdin))
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
