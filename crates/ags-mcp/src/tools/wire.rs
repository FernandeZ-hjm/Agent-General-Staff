use super::*;
#[allow(unused_imports)]
use super::{apply::*, decision::*, preflight::*};

pub const TOOL_PREFLIGHT: &str = "ags_preflight";
pub const TOOL_PROTOCOL_STATUS: &str = "ags_protocol_status";
pub const TOOL_AGENT_INSTRUCTIONS: &str = "ags_agent_instructions";
pub const TOOL_ONBOARDING_PLAN: &str = "ags_onboarding_plan";
pub const TOOL_TASK_VALIDATE: &str = "ags_task_validate";
pub const TOOL_POLICY_RESOLVE: &str = "ags_policy_resolve";
pub const TOOL_ROUTE_REQUEST: &str = "ags_route_request";
pub const TOOL_APPLY_ACTION: &str = "ags_apply_action";
pub const TOOL_MAINTENANCE_STATUS: &str = "ags_maintenance_status";
pub const TOOL_MAINTENANCE_PLAN: &str = "ags_maintenance_plan";
pub const TOOL_MAINTENANCE_APPLY: &str = "ags_maintenance_apply";
pub const TOOL_MAINTENANCE_VERIFY: &str = "ags_maintenance_verify";
pub const TOOL_MAINTENANCE_RECOVER: &str = "ags_maintenance_recover";

pub const CURRENT_HOST_CAPABILITIES_URI: &str = "ags://capabilities/current-host";

pub use ags_session::{CapabilityCatalogSource, PreflightBinding};

#[derive(Debug, Clone, Serialize)]
pub(super) struct SkillOutcomeBinding {
    pub(super) request_fingerprint: String,
    pub(super) skill_id: String,
    pub(super) entrypoint: Option<String>,
}

#[derive(Debug)]
pub(super) enum HeldActionKind {
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
    Onboarding {
        plan_hash: String,
        item_id: String,
        action: ags_lifecycle::OnboardingAction,
    },
}

#[derive(Debug)]
pub(crate) struct HeldAction {
    pub(super) evidence: DecisionLeaseEvidence,
    pub(super) action_id: String,
    pub(super) kind: HeldActionKind,
    pub(super) consumed: bool,
}

/// Per-daemon-client-session route state. The session crate owns isolation and
/// generation invalidation; the MCP decision adapter owns the held payload.
pub(crate) type RoutingSession = ags_session::SessionActionStore<HeldAction>;

pub fn is_preflight_tool_name(name: &str) -> bool {
    name == TOOL_PREFLIGHT
}

pub fn is_preflight_bootstrap_tool_name(name: &str) -> bool {
    matches!(name, TOOL_PREFLIGHT | TOOL_AGENT_INSTRUCTIONS)
}

pub fn is_onboarding_bootstrap_tool_name(name: &str) -> bool {
    matches!(name, TOOL_ONBOARDING_PLAN | TOOL_APPLY_ACTION)
}

pub fn list_tools() -> ToolListResult {
    let mut skill_host_ids = ags_host_integration::supported_skill_hosts()
        .map(str::to_string)
        .collect::<Vec<_>>();
    skill_host_ids.push("all".to_string());
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
                TOOL_ONBOARDING_PLAN,
                "Read-only deterministic public onboarding assessment. In bootstrap_required mode it creates one-shot, daemon-client-session-local action references for individually applyable items.",
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
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
                TOOL_ROUTE_REQUEST,
                "Read-only typed request governance. The host interprets conversation context and submits exact Skill, MCP, direct-response, or machine targets; AGS validates them and creates daemon-client-session-local action references where required. Third-party MCP invocation remains host-native.",
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
                                "schema_version": { "type": "string", "const": "0.3.6-host-route-proposal" },
                                "request_fingerprint": { "type": "string" },
                                "phase": { "type": "string", "enum": ["direct_response", "solution_formation", "execution"] },
                                "solution_state": { "type": "string", "enum": ["not_required", "open", "confirmed"] },
                                "execution_authority": { "type": "string", "enum": ["none", "direct_edit", "task_card_handoff"] },
                                "scope_hash": { "type": "string" },
                                "targets": {
                                    "type": "array",
                                    "minItems": 0,
                                    "maxItems": 3,
                                    "items": {
                                        "oneOf": [
                                            { "$ref": "#/$defs/DirectResponseTarget" },
                                            { "$ref": "#/$defs/SkillTarget" },
                                            { "$ref": "#/$defs/McpTarget" },
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
                        "McpTarget": {
                            "type": "object",
                            "required": ["kind", "mcp_id", "snapshot_hash"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": { "const": "mcp" },
                                "mcp_id": { "type": "string" },
                                "tool": { "type": "string" },
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
                                        "skill_adopt",
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
                                        "content": { "type": "string", "minLength": 1 },
                                        "handoff_source": {
                                            "type": "string",
                                            "enum": ["explicit_handoff", "host_plan_mode"],
                                            "default": "explicit_handoff"
                                        }
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
                                    "required": ["kind", "source", "host", "apply"],
                                    "additionalProperties": false,
                                    "properties": {
                                        "kind": { "const": "skill_adopt" },
                                        "source": { "type": "string", "minLength": 1, "maxLength": 4096 },
                                        "host": {
                                            "type": "string",
                                            "enum": skill_host_ids
                                        },
                                        "apply": { "type": "boolean" }
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
            tool_def(
                TOOL_MAINTENANCE_STATUS,
                "Read one immutable maintenance plan bound to this MCP connection.",
                maintenance::plan_hash_schema(),
            ),
            tool_def(
                TOOL_MAINTENANCE_PLAN,
                "Create a typed, expiring, connection-bound Skill maintenance plan. Planning may resolve a remote source but never applies it.",
                maintenance::maintenance_plan_schema(),
            ),
            tool_def(
                TOOL_MAINTENANCE_APPLY,
                "Apply exactly one current connection-bound maintenance plan after explicit per-finding acknowledgements.",
                maintenance::plan_hash_schema(),
            ),
            tool_def(
                TOOL_MAINTENANCE_VERIFY,
                "Verify immutable body, host projection, active snapshots and exact Skill visibility for a maintenance plan.",
                maintenance::plan_hash_schema(),
            ),
            tool_def(
                TOOL_MAINTENANCE_RECOVER,
                "Recover the prior immutable Skill revision or remove a newly installed Skill using the plan recovery point.",
                maintenance::plan_hash_schema(),
            ),
        ],
    }
}

pub(super) fn target_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": { "target": { "type": "string" } }
    })
}

pub(super) fn tool_def(
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
    capability_source: &dyn CapabilityCatalogSource,
) -> Result<String, String> {
    match name {
        TOOL_PREFLIGHT => tool_preflight(arguments, capability_source),
        TOOL_PROTOCOL_STATUS => tool_protocol_status(arguments),
        TOOL_AGENT_INSTRUCTIONS => tool_agent_instructions(arguments),
        TOOL_ONBOARDING_PLAN => {
            tool_onboarding_plan(arguments, required_binding(binding)?, routing_session)
        }
        TOOL_TASK_VALIDATE => tool_task_validate(arguments),
        TOOL_POLICY_RESOLVE => tool_policy_resolve(arguments),
        TOOL_ROUTE_REQUEST => tool_route_request_with_source(
            arguments,
            required_binding(binding)?,
            routing_session,
            capability_source,
        ),
        TOOL_APPLY_ACTION => tool_apply_action(
            arguments,
            required_binding(binding)?,
            routing_session,
            &ags_platform::runtime_home(),
        ),
        TOOL_MAINTENANCE_STATUS => maintenance::tool_maintenance_status(
            arguments,
            required_binding(binding)?,
            routing_session,
        ),
        TOOL_MAINTENANCE_PLAN => maintenance::tool_maintenance_plan(
            arguments,
            required_binding(binding)?,
            routing_session,
        ),
        TOOL_MAINTENANCE_APPLY => maintenance::tool_maintenance_apply(
            arguments,
            required_binding(binding)?,
            routing_session,
        ),
        TOOL_MAINTENANCE_VERIFY => maintenance::tool_maintenance_verify(
            arguments,
            required_binding(binding)?,
            routing_session,
        ),
        TOOL_MAINTENANCE_RECOVER => maintenance::tool_maintenance_recover(
            arguments,
            required_binding(binding)?,
            routing_session,
        ),
        other => Err(format!("Unknown tool: {other}")),
    }
}

pub(super) fn required_binding(
    binding: Option<&PreflightBinding>,
) -> Result<&PreflightBinding, String> {
    binding.ok_or_else(|| "preflight_binding_missing".to_string())
}

#[cfg(test)]
mod maintenance_wire_tests {
    use super::*;

    #[test]
    fn maintenance_tools_are_typed_and_listed_once() {
        let tools = list_tools().tools;
        for expected in [
            TOOL_MAINTENANCE_STATUS,
            TOOL_MAINTENANCE_PLAN,
            TOOL_MAINTENANCE_APPLY,
            TOOL_MAINTENANCE_VERIFY,
            TOOL_MAINTENANCE_RECOVER,
        ] {
            let matching = tools
                .iter()
                .filter(|definition| definition.name == expected)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "{expected} must be listed exactly once");
            assert_eq!(matching[0].inputSchema["type"], "object");
            assert_eq!(
                matching[0].inputSchema["additionalProperties"],
                serde_json::Value::Bool(false)
            );
        }
    }
}
