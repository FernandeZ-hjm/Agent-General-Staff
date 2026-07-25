use super::*;
#[allow(unused_imports)]
use super::{apply::*, preflight::*, wire::*};
#[cfg(test)]
pub(super) fn tool_route_request(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
) -> Result<String, String> {
    tool_route_request_with_source(args, binding, session, runtime_home, None)
}

pub(super) fn tool_route_request_with_source(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
    capability_source: Option<&dyn CapabilityCatalogSource>,
) -> Result<String, String> {
    // Every route attempt starts a new decision generation, including malformed
    // or legacy input. A caller cannot probe a new route shape while retaining
    // an older effectful lease.
    session.invalidate();
    if args.get("request").is_some() {
        return Err("legacy_raw_request_unsupported".to_string());
    }
    if args.get("active_host").is_some() || args.get("target").is_some() {
        return Err("preflight_binding_conflict".to_string());
    }
    let unexpected_fields = args
        .as_object()
        .map(|object| {
            object
                .keys()
                .filter(|key| key.as_str() != "proposal")
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !unexpected_fields.is_empty() {
        return Err(serde_json::json!({
            "code": "typed_proposal_unexpected_fields",
            "fields": unexpected_fields,
        })
        .to_string());
    }
    let proposal_value = match args.get("proposal") {
        Some(proposal) => proposal,
        None => {
            return Err(serde_json::json!({
                "code": "typed_proposal_missing_fields",
                "fields": ["proposal"],
            })
            .to_string());
        }
    };
    let required_fields = [
        "schema_version",
        "request_fingerprint",
        "phase",
        "solution_state",
        "execution_authority",
        "scope_hash",
        "targets",
    ];
    let missing_fields = required_fields
        .iter()
        .filter(|field| proposal_value.get(**field).is_none())
        .copied()
        .collect::<Vec<_>>();
    if !missing_fields.is_empty() {
        return Err(serde_json::json!({
            "code": "typed_proposal_missing_fields",
            "fields": missing_fields,
        })
        .to_string());
    }
    let proposal: HostRouteProposal = serde_json::from_value(proposal_value.clone())
        .map_err(|error| format!("invalid_typed_proposal: {error}"))?;

    let proposal_id = proposal_hash(&proposal);
    let decision_id = stable_id(
        "decision",
        &proposal_id,
        &session.connection_nonce,
        session.generation,
    );
    if let Err(errors) = validate_proposal(&proposal) {
        return pretty(&RouteResolution {
            schema_version: ROUTE_RESOLUTION_SCHEMA_VERSION.to_string(),
            governance_status: GovernanceStatus::BlockedByPolicy,
            decision_id,
            proposal_hash: proposal_id,
            host: binding.host.clone(),
            target: binding.target.to_string_lossy().into_owned(),
            resolved_targets: Vec::new(),
            lease: None,
            errors,
        });
    }

    let needs_capability_authority = proposal.targets.iter().any(|target| {
        matches!(
            target,
            ProposalTarget::Skill(_) | ProposalTarget::MachineCli(_)
        )
    });
    let (authority_root, registry_hash) = if needs_capability_authority {
        let root = match skill_resolver::resolve_capability_authority_root(
            &binding.target,
            runtime_home,
            std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
        ) {
            Ok(root) => root,
            Err(error) => {
                return blocked_route(
                    binding,
                    decision_id,
                    proposal_id,
                    ProposalError::new(
                        "capability_authority_unresolved",
                        "targets",
                        error.to_string(),
                    ),
                );
            }
        };
        let registry_bytes = match std::fs::read(root.join("manifests/skills-registry.yaml")) {
            Ok(bytes) => bytes,
            Err(error) => {
                return blocked_route(
                    binding,
                    decision_id,
                    proposal_id,
                    ProposalError::new(
                        "capability_registry_unavailable",
                        "targets",
                        format!("capability registry read failed: {error}"),
                    ),
                );
            }
        };
        let registry_hash = skill_resolver::sha256(&registry_bytes);
        (Some(root), registry_hash)
    } else {
        (None, "sha256:not-applicable".to_string())
    };

    // Resolve every read-only dependency before creating any held action. This
    // prevents target ordering from leaving an action behind when a later
    // exact skill selection fails.
    let skill_target = proposal.targets.iter().find_map(|target| match target {
        ProposalTarget::Skill(skill) => Some(skill),
        _ => None,
    });
    let current_snapshot = if let Some(root) = authority_root.as_deref() {
        let loaded = capability_source.map_or_else(
            || {
                skill_resolver::load_validated_snapshot_with_roots(
                    root,
                    runtime_home,
                    &binding.host,
                    &binding.host_home,
                )
                .map_err(|_| "skill_snapshot_stale".to_string())
            },
            |source| source.load_validated_snapshot(binding),
        );
        match loaded {
            Ok(snapshot) => snapshot,
            Err(_) => {
                return blocked_route(
                    binding,
                    decision_id,
                    proposal_id,
                    ProposalError::new(
                        "skill_snapshot_stale",
                        "targets",
                        "the preflight-bound host capability snapshot is unavailable or stale",
                    ),
                );
            }
        }
    } else {
        return finish_route_without_governed_targets(
            binding,
            session,
            proposal,
            decision_id,
            proposal_id,
            registry_hash,
        );
    };
    let (snapshot, table) = current_snapshot;
    let (selected_skill, snapshot_hash) = if let Some(skill) = skill_target {
        if skill.snapshot_hash != snapshot.snapshot_hash {
            return blocked_route(
                binding,
                decision_id,
                proposal_id,
                ProposalError::new(
                    "skill_snapshot_stale",
                    "targets.snapshot_hash",
                    "the proposal snapshot_hash does not match the current host snapshot",
                ),
            );
        }
        let selection = match skill_resolver::resolve_skill(
            &skill.skill_id,
            skill.entrypoint.as_deref(),
            &snapshot.snapshot_hash,
            &table,
        ) {
            Ok(selection) => selection,
            Err(error) => {
                return blocked_route(
                    binding,
                    decision_id,
                    proposal_id,
                    ProposalError::new(
                        "skill_selection_rejected",
                        "targets.skill_id",
                        format!("exact skill selection rejected: {error:?}"),
                    ),
                );
            }
        };
        (Some(selection), snapshot.snapshot_hash)
    } else {
        (None, snapshot.snapshot_hash)
    };

    let skill_outcome = selected_skill
        .as_ref()
        .map(|selection| SkillOutcomeBinding {
            request_fingerprint: proposal.request_fingerprint.clone(),
            skill_id: selection.skill_id.clone(),
            entrypoint: selection.entrypoint.clone(),
        });
    let machine_policy = proposal.targets.iter().find_map(|target| match target {
        ProposalTarget::MachineCli(machine) => Some(machine_policy_hash(
            machine.capability,
            &machine.input,
            skill_outcome.as_ref(),
        )),
        _ => None,
    });
    let machine_policy_hash = match machine_policy.transpose() {
        Ok(policy) => policy,
        Err(message) => {
            return blocked_route(
                binding,
                decision_id,
                proposal_id,
                ProposalError::new("machine_policy_rejected", "targets.input", message),
            );
        }
    };

    let action_context = ActionHoldContext {
        binding,
        proposal: &proposal,
        decision_id: &decision_id,
        proposal_id: &proposal_id,
        registry_hash: &registry_hash,
        snapshot_hash: &snapshot_hash,
    };
    let mut resolved_targets = Vec::new();
    for target in &proposal.targets {
        match target {
            ProposalTarget::DirectResponse {} => {
                resolved_targets.push(ResolvedTarget::DirectResponse)
            }
            ProposalTarget::Skill(_) => {
                let selection = selected_skill
                    .as_ref()
                    .expect("skill target was resolved before action creation");
                resolved_targets.push(ResolvedTarget::Skill {
                    skill_id: selection.skill_id.clone(),
                    invoke_hint: selection.invoke_hint.clone(),
                    entrypoint: selection.entrypoint.clone(),
                });
            }
            ProposalTarget::MachineCli(machine) => {
                let action = hold_action(
                    session,
                    &action_context,
                    HeldActionKind::Machine {
                        capability: machine.capability,
                        input: machine.input.clone(),
                        skill_outcome: skill_outcome.clone(),
                    },
                    machine_policy_hash
                        .as_deref()
                        .expect("machine target has a resolved policy hash"),
                );
                resolved_targets.push(ResolvedTarget::ServerHeldAction {
                    action_id: action.action_id.clone(),
                    action_kind: ServerHeldActionKind::MachineCli,
                    capability: Some(machine.capability),
                });
            }
        }
    }

    if proposal.execution_authority == ExecutionAuthority::DirectEdit {
        let action_id = stable_id(
            "host",
            &proposal_id,
            &session.connection_nonce,
            session.generation,
        );
        resolved_targets.push(ResolvedTarget::HostNativeDirectEdit { action_id });
    }

    // A skill-only route receives one controlled outcome action regardless of
    // phase. This closes the lifecycle loop for solution-method skills as well
    // as direct edits. A coexisting MachineCli action remains the sole action
    // for that decision; one lease consumption invalidates the whole decision.
    let has_machine_action = proposal
        .targets
        .iter()
        .any(|target| matches!(target, ProposalTarget::MachineCli(_)));
    if !has_machine_action {
        if let Some(selection) = selected_skill.as_ref() {
            let outcome_action = hold_action(
                session,
                &action_context,
                HeldActionKind::RecordOutcome {
                    request_fingerprint: proposal.request_fingerprint.clone(),
                    skill_id: selection.skill_id.clone(),
                    entrypoint: selection.entrypoint.clone(),
                },
                &outcome_policy_hash(
                    &proposal.request_fingerprint,
                    &selection.skill_id,
                    selection.entrypoint.as_deref(),
                ),
            );
            resolved_targets.push(ResolvedTarget::ServerHeldAction {
                action_id: outcome_action.action_id.clone(),
                action_kind: ServerHeldActionKind::SkillOutcome,
                capability: None,
            });
        }
    }

    let lease = session
        .actions
        .values()
        .next()
        .map(|action| action.evidence.clone());
    let status = if proposal.execution_authority == ExecutionAuthority::DirectEdit {
        GovernanceStatus::HostExecutionRequired
    } else {
        GovernanceStatus::Ok
    };
    pretty(&RouteResolution {
        schema_version: ROUTE_RESOLUTION_SCHEMA_VERSION.to_string(),
        governance_status: status,
        decision_id,
        proposal_hash: proposal_id,
        host: binding.host.clone(),
        target: binding.target.to_string_lossy().into_owned(),
        resolved_targets,
        lease,
        errors: Vec::new(),
    })
}

pub(super) fn finish_route_without_governed_targets(
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    proposal: HostRouteProposal,
    decision_id: String,
    proposal_id: String,
    _registry_hash: String,
) -> Result<String, String> {
    let mut resolved_targets = proposal
        .targets
        .iter()
        .filter_map(|target| {
            matches!(target, ProposalTarget::DirectResponse {})
                .then_some(ResolvedTarget::DirectResponse)
        })
        .collect::<Vec<_>>();
    if proposal.execution_authority == ExecutionAuthority::DirectEdit {
        resolved_targets.push(ResolvedTarget::HostNativeDirectEdit {
            action_id: stable_id(
                "host",
                &proposal_id,
                &session.connection_nonce,
                session.generation,
            ),
        });
    }
    pretty(&RouteResolution {
        schema_version: ROUTE_RESOLUTION_SCHEMA_VERSION.to_string(),
        governance_status: if proposal.execution_authority == ExecutionAuthority::DirectEdit {
            GovernanceStatus::HostExecutionRequired
        } else {
            GovernanceStatus::Ok
        },
        decision_id,
        proposal_hash: proposal_id,
        host: binding.host.clone(),
        target: binding.target.to_string_lossy().into_owned(),
        resolved_targets,
        lease: None,
        errors: Vec::new(),
    })
}

pub(super) fn blocked_route(
    binding: &PreflightBinding,
    decision_id: String,
    proposal_id: String,
    error: ProposalError,
) -> Result<String, String> {
    pretty(&RouteResolution {
        schema_version: ROUTE_RESOLUTION_SCHEMA_VERSION.to_string(),
        governance_status: GovernanceStatus::BlockedByPolicy,
        decision_id,
        proposal_hash: proposal_id,
        host: binding.host.clone(),
        target: binding.target.to_string_lossy().into_owned(),
        resolved_targets: Vec::new(),
        lease: None,
        errors: vec![error],
    })
}

pub(super) struct ActionHoldContext<'a> {
    binding: &'a PreflightBinding,
    proposal: &'a HostRouteProposal,
    decision_id: &'a str,
    proposal_id: &'a str,
    registry_hash: &'a str,
    snapshot_hash: &'a str,
}

pub(super) fn machine_policy_hash(
    capability: CliCapabilityId,
    input: &TypedCliInput,
    skill_outcome: Option<&SkillOutcomeBinding>,
) -> Result<String, String> {
    validate_machine_input(capability, input)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let admission = match (capability, input) {
        (
            CliCapabilityId::TaskPrepareExecution | CliCapabilityId::PolicyResolve,
            TypedCliInput::TaskCard { content },
        ) => {
            let parsed = task_card_validator::parse_validated(content)
                .map_err(|errors| format!("task_card_validation_failed: {}", errors.join("; ")))?;
            let policy = execution_policy::resolve_policy(
                execution_policy::TaskPolicyInput::from_fields(&parsed.fields),
            );
            if capability == CliCapabilityId::TaskPrepareExecution && policy.stop_before_launch {
                return Err(format!(
                    "task_execution_policy_stopped: {}",
                    serde_json::to_string(&policy.stop_reasons).unwrap_or_default()
                ));
            }
            serde_json::json!({
                "contract": "ags-machine-policy-v1",
                "capability": capability,
                "resolved_policy": policy,
                "skill_outcome": skill_outcome,
            })
        }
        _ => serde_json::json!({
            "contract": "ags-closed-machine-admission-v1",
            "capability": capability,
            "input_kind": typed_input_kind(input),
            "skill_outcome": skill_outcome,
        }),
    };
    serde_json::to_vec(&admission)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("cannot serialize resolved machine policy: {error}"))
}

pub(super) fn typed_input_kind(input: &TypedCliInput) -> &'static str {
    match input {
        TypedCliInput::ConfirmedHandoffContract { .. } => "confirmed_handoff_contract",
        TypedCliInput::TaskCard { .. } => "task_card",
        TypedCliInput::Receipt { .. } => "receipt",
        TypedCliInput::SkillAdopt { .. } => "skill_adopt",
        TypedCliInput::Empty => "empty",
    }
}

pub(super) fn outcome_policy_hash(
    request_fingerprint: &str,
    skill_id: &str,
    entrypoint: Option<&str>,
) -> String {
    sha256(
        serde_json::to_string(&(
            "ags-skill-outcome-policy-v1",
            request_fingerprint,
            skill_id,
            entrypoint,
        ))
        .unwrap_or_default()
        .as_bytes(),
    )
}

pub(super) fn hold_action<'a>(
    session: &'a mut RoutingSession,
    context: &ActionHoldContext<'_>,
    kind: HeldActionKind,
    policy_hash: &str,
) -> &'a HeldAction {
    let serialized = match &kind {
        HeldActionKind::Machine {
            capability,
            input,
            skill_outcome,
        } => serde_json::to_string(&(capability, input, skill_outcome)).unwrap_or_default(),
        HeldActionKind::RecordOutcome {
            request_fingerprint,
            skill_id,
            entrypoint,
        } => {
            serde_json::to_string(&(request_fingerprint, skill_id, entrypoint)).unwrap_or_default()
        }
        HeldActionKind::Onboarding {
            plan_hash,
            item_id,
            action,
        } => serde_json::to_string(&(plan_hash, item_id, action)).unwrap_or_default(),
    };
    let action_id = stable_id(
        "action",
        &format!("{}\n{serialized}", context.proposal_id),
        &session.connection_nonce,
        session.generation,
    );
    let lease_id = stable_id(
        "lease",
        context.proposal_id,
        &session.connection_nonce,
        session.generation,
    );
    let evidence = DecisionLeaseEvidence {
        lease_id,
        decision_id: context.decision_id.to_string(),
        proposal_hash: context.proposal_id.to_string(),
        scope_hash: context.proposal.scope_hash.clone(),
        host: context.binding.host.clone(),
        target: context.binding.target.to_string_lossy().into_owned(),
        registry_hash: context.registry_hash.to_string(),
        snapshot_hash: context.snapshot_hash.to_string(),
        policy_hash: policy_hash.to_string(),
    };
    session.actions.insert(
        action_id.clone(),
        HeldAction {
            evidence,
            action_id: action_id.clone(),
            policy_hash: policy_hash.to_string(),
            kind,
            consumed: false,
        },
    );
    session.actions.get(&action_id).expect("inserted action")
}

pub(super) fn onboarding_policy_hash(
    plan_hash: &str,
    item_id: &str,
    action: &ags_onboarding::OnboardingAction,
) -> String {
    ags_onboarding::action_hash(plan_hash, item_id, action)
}

pub(super) fn hold_onboarding_action<'a>(
    session: &'a mut RoutingSession,
    binding: &PreflightBinding,
    plan_hash: &str,
    item_id: &str,
    action: ags_onboarding::OnboardingAction,
) -> &'a HeldAction {
    let policy_hash = onboarding_policy_hash(plan_hash, item_id, &action);
    let action_id = stable_id(
        "onboarding-action",
        &format!("{plan_hash}\n{item_id}\n{policy_hash}"),
        &session.connection_nonce,
        session.generation,
    );
    let lease_id = stable_id(
        "onboarding-lease",
        plan_hash,
        &session.connection_nonce,
        session.generation,
    );
    let evidence = DecisionLeaseEvidence {
        lease_id,
        decision_id: stable_id(
            "onboarding-decision",
            plan_hash,
            &session.connection_nonce,
            session.generation,
        ),
        proposal_hash: plan_hash.to_string(),
        scope_hash: sha256(binding.target.to_string_lossy().as_bytes()),
        host: binding.host.clone(),
        target: binding.target.to_string_lossy().into_owned(),
        registry_hash: plan_hash.to_string(),
        snapshot_hash: "sha256:bootstrap-not-applicable".to_string(),
        policy_hash: policy_hash.clone(),
    };
    session.actions.insert(
        action_id.clone(),
        HeldAction {
            evidence,
            action_id: action_id.clone(),
            policy_hash,
            kind: HeldActionKind::Onboarding {
                plan_hash: plan_hash.to_string(),
                item_id: item_id.to_string(),
                action,
            },
            consumed: false,
        },
    );
    session.actions.get(&action_id).expect("inserted action")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OutcomeInput {
    pub(super) status: skill_resolver::SkillOutcome,
    #[serde(default)]
    pub(super) quality: Option<u8>,
}

#[derive(Debug, Serialize)]
pub(super) struct OnboardingExecutionResult {
    pub(super) item_id: String,
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) receipt_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ApplyResult {
    pub(super) schema_version: &'static str,
    pub(super) governance_status: GovernanceStatus,
    pub(super) lease_id: String,
    pub(super) action_id: String,
    pub(super) consumed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) machine_result: Option<MachineCliResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) onboarding_result: Option<OnboardingExecutionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) outcome_event_id: Option<String>,
    pub(super) requires_repreflight: bool,
}
