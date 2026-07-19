//! AGS MCP governance tools.
//!
//! `ags_route_request` consumes a typed host proposal and is strictly
//! read-only. `ags_apply_action` is the only MCP tool in this module allowed to
//! launch a process or append a machine-local usage event.

use crate::protocol::ToolListResult;
use request_governance::{
    proposal_hash, sha256, validate_machine_input, validate_proposal, CliCapabilityId,
    DecisionLeaseEvidence, ExecutionAuthority, GovernanceStatus, HostRouteProposal, ProposalError,
    ProposalTarget, ResolvedTarget, RouteResolution, ServerHeldActionKind, TypedCliInput,
    ROUTE_RESOLUTION_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

pub const TOOL_PREFLIGHT: &str = "ags_preflight";
pub const TOOL_PROTOCOL_STATUS: &str = "ags_protocol_status";
pub const TOOL_AGENT_INSTRUCTIONS: &str = "ags_agent_instructions";
pub const TOOL_TASK_VALIDATE: &str = "ags_task_validate";
pub const TOOL_POLICY_RESOLVE: &str = "ags_policy_resolve";
pub const TOOL_VERIFY_LOCAL: &str = "ags_verify_local";
pub const TOOL_ROUTE_REQUEST: &str = "ags_route_request";
pub const TOOL_APPLY_ACTION: &str = "ags_apply_action";

pub const CURRENT_HOST_CAPABILITIES_URI: &str = "ags://capabilities/current-host";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightBinding {
    pub host: String,
    pub target: PathBuf,
    pub host_home: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct SkillOutcomeBinding {
    request_fingerprint: String,
    skill_id: String,
    entrypoint: Option<String>,
}

#[derive(Debug)]
enum HeldActionKind {
    Machine {
        capability: CliCapabilityId,
        input: TypedCliInput,
        skill_outcome: Option<SkillOutcomeBinding>,
    },
    RecordOutcome {
        request_fingerprint: String,
        skill_id: String,
        entrypoint: Option<String>,
    },
}

#[derive(Debug)]
struct HeldAction {
    evidence: DecisionLeaseEvidence,
    action_id: String,
    policy_hash: String,
    kind: HeldActionKind,
    consumed: bool,
}

/// Per-MCP-connection route state. A new preflight or route clears all prior
/// actions; callers can never carry an action across connections.
#[derive(Debug)]
pub struct RoutingSession {
    connection_nonce: String,
    generation: u64,
    actions: HashMap<String, HeldAction>,
}

impl Default for RoutingSession {
    fn default() -> Self {
        static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            connection_nonce: sha256(
                format!("connection\n{}\n{now}\n{sequence}", std::process::id()).as_bytes(),
            ),
            generation: 0,
            actions: HashMap::new(),
        }
    }
}

impl RoutingSession {
    pub fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.actions.clear();
    }
}

pub fn is_preflight_tool_name(name: &str) -> bool {
    name == TOOL_PREFLIGHT
}

pub fn is_preflight_bootstrap_tool_name(name: &str) -> bool {
    matches!(name, TOOL_PREFLIGHT | TOOL_AGENT_INSTRUCTIONS)
}

pub fn list_tools() -> ToolListResult {
    ToolListResult {
        tools: vec![
            tool_def(
                TOOL_PREFLIGHT,
                "MANDATORY FIRST CALL. Run AGS session preflight for the active host and repository.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent": { "type": "string" },
                        "target": { "type": "string" }
                    },
                    "required": ["agent"],
                    "additionalProperties": false
                }),
            ),
            tool_def(
                TOOL_PROTOCOL_STATUS,
                "Read AGS protocol status for a repository.",
                target_schema(),
            ),
            tool_def(
                TOOL_AGENT_INSTRUCTIONS,
                "Read host-specific AGS instructions.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent": { "type": "string" },
                        "target": { "type": "string" }
                    },
                    "required": ["agent"]
                }),
            ),
            tool_def(
                TOOL_TASK_VALIDATE,
                "Validate a canonical task card.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "task_card": { "type": "string" } },
                    "required": ["task_card"],
                    "additionalProperties": false
                }),
            ),
            tool_def(
                TOOL_POLICY_RESOLVE,
                "Resolve policy for a validated task card.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task_card": { "type": "string" },
                        "approve_writes": { "type": "boolean", "default": false },
                        "current_task_approval": { "type": "boolean", "default": false }
                    },
                    "required": ["task_card"]
                }),
            ),
            tool_def(
                TOOL_VERIFY_LOCAL,
                "Read-only compatibility guidance for the fixed local verification action. No command is launched; execution requires a typed ProjectVerify route followed by ags_apply_action.",
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            ),
            tool_def(
                TOOL_ROUTE_REQUEST,
                "Read-only typed request governance. The host interprets conversation context and submits an exact proposal; AGS validates it and creates connection-local action references.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "proposal": { "$ref": "#/$defs/HostRouteProposal" }
                    },
                    "required": ["proposal"],
                    "additionalProperties": false,
                    "$defs": {
                        "HostRouteProposal": {
                            "type": "object",
                            "required": ["schema_version", "request_fingerprint", "phase", "solution_state", "execution_authority", "scope_hash", "targets"],
                            "additionalProperties": false,
                            "properties": {
                                "schema_version": { "type": "string", "const": "0.3.0-host-route-proposal" },
                                "request_fingerprint": { "type": "string" },
                                "phase": { "type": "string", "enum": ["direct_response", "solution_formation", "execution"] },
                                "solution_state": { "type": "string", "enum": ["not_required", "open", "confirmed"] },
                                "execution_authority": { "type": "string", "enum": ["none", "direct_edit", "task_card_handoff"] },
                                "scope_hash": { "type": "string" },
                                "targets": {
                                    "type": "array",
                                    "minItems": 0,
                                    "maxItems": 2,
                                    "items": {
                                        "oneOf": [
                                            { "$ref": "#/$defs/DirectResponseTarget" },
                                            { "$ref": "#/$defs/SkillTarget" },
                                            { "$ref": "#/$defs/MachineCliTarget" }
                                        ]
                                    }
                                }
                            }
                        },
                        "DirectResponseTarget": {
                            "type": "object",
                            "required": ["kind"],
                            "additionalProperties": false,
                            "properties": { "kind": { "const": "direct_response" } }
                        },
                        "SkillTarget": {
                            "type": "object",
                            "required": ["kind", "skill_id", "snapshot_hash"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": { "const": "skill" },
                                "skill_id": { "type": "string" },
                                "entrypoint": { "type": "string" },
                                "snapshot_hash": { "type": "string" }
                            }
                        },
                        "MachineCliTarget": {
                            "type": "object",
                            "required": ["kind", "capability", "input"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": { "const": "machine_cli" },
                                "capability": {
                                    "type": "string",
                                    "enum": [
                                        "task_compile",
                                        "task_prepare_execution",
                                        "task_validate",
                                        "policy_resolve",
                                        "project_verify",
                                        "skill_tags_verify",
                                        "receipt_verify"
                                    ]
                                },
                                "input": { "$ref": "#/$defs/TypedCliInput" }
                            }
                        },
                        "TypedCliInput": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "required": ["kind", "content"],
                                    "additionalProperties": false,
                                    "properties": {
                                        "kind": { "const": "confirmed_handoff_contract" },
                                        "content": { "type": "string", "minLength": 1 }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["kind", "content"],
                                    "additionalProperties": false,
                                    "properties": {
                                        "kind": { "const": "task_card" },
                                        "content": { "type": "string", "minLength": 1 }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["kind", "content"],
                                    "additionalProperties": false,
                                    "properties": {
                                        "kind": { "const": "receipt" },
                                        "content": { "type": "string", "minLength": 1 }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["kind"],
                                    "additionalProperties": false,
                                    "properties": { "kind": { "const": "empty" } }
                                }
                            ]
                        }
                    }
                }),
            ),
            tool_def(
                TOOL_APPLY_ACTION,
                "Consume one server-held action from the current DecisionLease. Callers cannot resubmit or alter capability, input or argv.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "lease_id": { "type": "string" },
                        "action_id": { "type": "string" },
                        "outcome": {
                            "type": "object",
                            "properties": {
                                "status": { "type": "string", "enum": ["succeeded", "failed", "abandoned"] },
                                "quality": { "type": "integer", "minimum": 0, "maximum": 100 }
                            },
                            "required": ["status"],
                            "additionalProperties": false
                        }
                    },
                    "required": ["lease_id", "action_id"],
                    "additionalProperties": false
                }),
            ),
        ],
    }
}

fn target_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": { "target": { "type": "string" } }
    })
}

fn tool_def(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
) -> crate::protocol::ToolDef {
    crate::protocol::ToolDef {
        name: name.to_string(),
        description: Some(description.to_string()),
        inputSchema: input_schema,
    }
}

pub fn call_tool(
    name: &str,
    arguments: &serde_json::Value,
    binding: Option<&PreflightBinding>,
    routing_session: &mut RoutingSession,
) -> Result<String, String> {
    match name {
        TOOL_PREFLIGHT => tool_preflight(arguments),
        TOOL_PROTOCOL_STATUS => tool_protocol_status(arguments),
        TOOL_AGENT_INSTRUCTIONS => tool_agent_instructions(arguments),
        TOOL_TASK_VALIDATE => tool_task_validate(arguments),
        TOOL_POLICY_RESOLVE => tool_policy_resolve(arguments),
        TOOL_VERIFY_LOCAL => tool_verify_local(arguments, required_binding(binding)?),
        TOOL_ROUTE_REQUEST => tool_route_request(
            arguments,
            required_binding(binding)?,
            routing_session,
            &skill_resolver::locate_runtime_home(),
        ),
        TOOL_APPLY_ACTION => tool_apply_action(
            arguments,
            required_binding(binding)?,
            routing_session,
            &skill_resolver::locate_runtime_home(),
        ),
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn required_binding(binding: Option<&PreflightBinding>) -> Result<&PreflightBinding, String> {
    binding.ok_or_else(|| "preflight_binding_missing".to_string())
}

fn tool_preflight(args: &serde_json::Value) -> Result<String, String> {
    let agent = get_string(args, "agent")?;
    let agent_type = project_discovery::AgentType::from_str(&agent)
        .map_err(|error| format!("Invalid agent: {error}"))?;
    let target = get_target(args);
    let report = project_discovery::run_session_preflight(&target, &agent_type);
    let resolved_target = report.target.clone();
    let mut value = serde_json::to_value(report).map_err(json_error)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("agent".to_string(), serde_json::json!(agent_type.as_str()));
        let capability = capability_reference(&resolved_target, agent_type.as_str());
        object.insert("capability_catalog".to_string(), capability);
    }
    pretty(&value)
}

fn capability_reference(target: &Path, host: &str) -> serde_json::Value {
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
    serde_json::json!({
        "uri": CURRENT_HOST_CAPABILITIES_URI,
        "status": if loaded.is_some() { "ready" } else { "snapshot_stale" },
        "snapshot_hash": loaded.as_ref().map(|(snapshot, _)| snapshot.snapshot_hash.clone())
    })
}

fn tool_protocol_status(args: &serde_json::Value) -> Result<String, String> {
    pretty(&project_discovery::check_protocol_status(&get_target(args)))
}

fn tool_agent_instructions(args: &serde_json::Value) -> Result<String, String> {
    let agent = get_string(args, "agent")?;
    let agent_type = project_discovery::AgentType::from_str(&agent)
        .map_err(|error| format!("Invalid agent: {error}"))?;
    pretty(&project_discovery::generate_agent_instructions(
        &get_target(args),
        &agent_type,
    ))
}

fn tool_task_validate(args: &serde_json::Value) -> Result<String, String> {
    let task_card = get_string(args, "task_card")?;
    let errors = task_card_validator::validate(&task_card);
    pretty(&serde_json::json!({
        "is_valid": errors.is_empty(),
        "error_count": errors.len(),
        "errors": errors,
    }))
}

fn tool_policy_resolve(args: &serde_json::Value) -> Result<String, String> {
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

fn tool_verify_local(
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

fn tool_route_request(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
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
        match skill_resolver::load_validated_snapshot_with_roots(
            root,
            runtime_home,
            &binding.host,
            &binding.host_home,
        ) {
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

fn finish_route_without_governed_targets(
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

fn blocked_route(
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

struct ActionHoldContext<'a> {
    binding: &'a PreflightBinding,
    proposal: &'a HostRouteProposal,
    decision_id: &'a str,
    proposal_id: &'a str,
    registry_hash: &'a str,
    snapshot_hash: &'a str,
}

fn machine_policy_hash(
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

fn typed_input_kind(input: &TypedCliInput) -> &'static str {
    match input {
        TypedCliInput::ConfirmedHandoffContract { .. } => "confirmed_handoff_contract",
        TypedCliInput::TaskCard { .. } => "task_card",
        TypedCliInput::Receipt { .. } => "receipt",
        TypedCliInput::Empty => "empty",
    }
}

fn outcome_policy_hash(
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

fn hold_action<'a>(
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeInput {
    status: skill_resolver::SkillOutcome,
    #[serde(default)]
    quality: Option<u8>,
}

#[derive(Debug, Serialize)]
struct ApplyResult {
    schema_version: &'static str,
    governance_status: GovernanceStatus,
    lease_id: String,
    action_id: String,
    consumed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_result: Option<MachineCliResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_event_id: Option<String>,
}

fn tool_apply_action(
    args: &serde_json::Value,
    binding: &PreflightBinding,
    session: &mut RoutingSession,
    runtime_home: &Path,
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
    }
    // A valid lease/action pair is one-shot even when the requested apply is
    // rejected later. This prevents a caller from probing bindings or hashes
    // and replaying the same action after changing its environment.
    for held in session.actions.values_mut() {
        if held.evidence.lease_id == lease_id {
            held.consumed = true;
        }
    }
    let action = session
        .actions
        .get(&action_id)
        .expect("validated held action remains connection-local");
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
    if action.evidence.host != binding.host || Path::new(&action.evidence.target) != binding.target
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
    };
    if policy_hash != action.policy_hash || policy_hash != action.evidence.policy_hash {
        return Err("decision_lease_policy_hash_mismatch".to_string());
    }
    let authority_root = skill_resolver::resolve_capability_authority_root(
        &binding.target,
        runtime_home,
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    )
    .map_err(|error| error.to_string())?;
    let registry = std::fs::read(authority_root.join("manifests/skills-registry.yaml"))
        .map_err(|error| error.to_string())?;
    if skill_resolver::sha256(&registry) != action.evidence.registry_hash {
        return Err("decision_lease_registry_hash_mismatch".to_string());
    }
    let (snapshot, _) = skill_resolver::load_validated_snapshot_with_roots(
        &authority_root,
        runtime_home,
        &binding.host,
        &binding.host_home,
    )
    .map_err(|_| "skill_snapshot_stale".to_string())?;
    if snapshot.snapshot_hash != action.evidence.snapshot_hash {
        return Err("decision_lease_snapshot_hash_mismatch".to_string());
    }

    let (machine_result, outcome_event_id, status) = match &action.kind {
        HeldActionKind::Machine {
            capability,
            input,
            skill_outcome,
        } => {
            let outcome = match (skill_outcome, args.get("outcome")) {
                (Some(_), Some(value)) => Some(
                    serde_json::from_value::<OutcomeInput>(value.clone())
                        .map_err(|error| format!("invalid_outcome: {error}"))?,
                ),
                (None, Some(_)) => return Err("outcome_not_allowed_for_machine_action".to_string()),
                (_, None) => None,
            };
            let result = invoke_machine_cli(*capability, input, &binding.host, &binding.target)?;
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
            (Some(result), outcome_event_id, status)
        }
        HeldActionKind::RecordOutcome {
            request_fingerprint,
            skill_id,
            entrypoint,
        } => {
            let outcome: OutcomeInput = serde_json::from_value(
                args.get("outcome")
                    .cloned()
                    .ok_or_else(|| "outcome_required".to_string())?,
            )
            .map_err(|error| format!("invalid_outcome: {error}"))?;
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
            (None, Some(event_id), GovernanceStatus::DoneWithReceipt)
        }
    };
    pretty(&ApplyResult {
        schema_version: "0.3.0-apply-result",
        governance_status: status,
        lease_id,
        action_id,
        consumed: true,
        machine_result,
        outcome_event_id,
    })
}

fn append_outcome_event(
    runtime_home: &Path,
    binding: &PreflightBinding,
    action: &HeldAction,
    generation: u64,
    skill: &SkillOutcomeBinding,
    outcome: OutcomeInput,
    connection_nonce: &str,
) -> Result<String, String> {
    let event_id = stable_id("outcome", &action.action_id, connection_nonce, generation);
    let event = skill_resolver::SkillUsageEvent {
        schema_version: skill_resolver::SKILL_USAGE_EVENT_SCHEMA_VERSION.to_string(),
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
    skill_resolver::append_usage_event(runtime_home, &binding.host, &event)?;
    Ok(event_id)
}

#[derive(Debug, Serialize)]
struct MachineCliResult {
    capability: CliCapabilityId,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn invoke_machine_cli(
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

fn machine_invocation(
    capability: CliCapabilityId,
    input: &TypedCliInput,
    host: &str,
    target: &Path,
) -> Result<(Vec<String>, String), String> {
    validate_machine_input(capability, input)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let stdin = match input {
        TypedCliInput::ConfirmedHandoffContract { content }
        | TypedCliInput::TaskCard { content }
        | TypedCliInput::Receipt { content } => content.clone(),
        TypedCliInput::Empty => String::new(),
    };
    let args = match capability {
        CliCapabilityId::TaskCompile => vec![
            "task",
            "compile",
            "-",
            "--format",
            "json",
            "--output",
            "report",
            "--task-card-requested",
            "--confirmed-handoff-contract",
        ],
        CliCapabilityId::TaskPrepareExecution => vec!["run", "-", "--format", "json"],
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

pub fn read_current_host_catalog(
    binding: &PreflightBinding,
    runtime_home: &Path,
) -> Result<skill_resolver::HostCapabilitySnapshot, String> {
    let authority = skill_resolver::resolve_capability_authority_root(
        &binding.target,
        runtime_home,
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    )
    .map_err(|error| error.to_string())?;
    let (snapshot, _) =
        skill_resolver::load_validated_snapshot(&authority, runtime_home, &binding.host)
            .map_err(|_| "skill_snapshot_stale".to_string())?;
    Ok(snapshot)
}

fn stable_id(prefix: &str, basis: &str, connection_nonce: &str, generation: u64) -> String {
    let digest = sha256(format!("{connection_nonce}\n{generation}\n{basis}").as_bytes());
    format!(
        "{prefix}-{}",
        digest
            .trim_start_matches("sha256:")
            .get(..20)
            .unwrap_or("invalid")
    )
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn get_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing required string argument: {key}"))
}

fn bool_arg(args: &serde_json::Value, key: &str) -> bool {
    args.get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn get_target(args: &serde_json::Value) -> PathBuf {
    args.get("target")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn pretty<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(json_error)
}

fn json_error(error: serde_json::Error) -> String {
    format!("JSON serialize error: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use request_governance::{
        ExecutionAuthority, ProposalPhase, SolutionState, HOST_ROUTE_PROPOSAL_SCHEMA_VERSION,
    };

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
目标：生成宿主执行所需的 LaunchPlan\n\
非目标：不在 AGS Runner 内执行宿主任务\n\
验证：\ncargo test -p ags-mcp\n\
Verification gate:\n- commands: cargo test -p ags-mcp\n\
交付：\n返回 host_execution_required\n"
            .to_string()
    }

    fn machine_fixture(tag: &str) -> (PathBuf, PreflightBinding, PathBuf, PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("ags-mcp-machine-{tag}-{}", std::process::id()));
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
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$@\" > \"$AGS_PROCESS_SPY\"\n",
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
        assert_eq!(tools.tools.len(), 8);
        let route = tools
            .tools
            .iter()
            .find(|tool| tool.name == TOOL_ROUTE_REQUEST)
            .expect("route tool");
        let capabilities = route.inputSchema["$defs"]["MachineCliTarget"]["properties"]
            ["capability"]["enum"]
            .as_array()
            .expect("capability enum");
        assert!(capabilities
            .iter()
            .any(|value| value == "task_prepare_execution"));
        assert!(capabilities.iter().all(|value| value != "task_execute"));
        assert!(route.inputSchema["$defs"]["TypedCliInput"]["oneOf"]
            .as_array()
            .is_some_and(|variants| variants.len() == 4));
        assert!(tools
            .tools
            .iter()
            .any(|tool| tool.name == TOOL_APPLY_ACTION));
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
            tool_verify_local(&serde_json::json!({"target": "/tmp/other"}), &binding())
                .unwrap_err(),
            "ags_verify_local_is_preflight_bound"
        );
    }

    #[test]
    fn machine_mapping_is_fixed_and_shell_free() {
        let (args, stdin) = machine_invocation(
            CliCapabilityId::TaskCompile,
            &TypedCliInput::ConfirmedHandoffContract {
                content: "任务：contract".to_string(),
            },
            "codex",
            Path::new("."),
        )
        .unwrap();
        assert_eq!(args[0..3], ["task", "compile", "-"]);
        assert_eq!(stdin, "任务：contract");
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
    fn tamper_binding_and_registry_fail_closed_and_consume_action() {
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
        assert_eq!(
            tool_apply_action(
                &serde_json::json!({"lease_id": lease_id, "action_id": action_id}),
                &PreflightBinding {
                    host: "codex".to_string(),
                    target: root,
                    host_home: home.clone(),
                },
                &mut session,
                &runtime,
            )
            .unwrap_err(),
            "decision_lease_invalid_or_consumed"
        );
        assert_eq!(
            skill_resolver::load_usage_events(&runtime, "codex").len(),
            2
        );

        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(base);
    }
}
