//! AGS MCP Tools — read-only governance tools exposed via MCP (host initialization adapter).
//!
//! All tools accept JSON arguments and return structured JSON results.
//! No tool writes files, installs hooks, or modifies any configuration.
//!
//! # Initialization Gate
//!
//! `ags_preflight` is the mandatory first call for all AGS scenarios.
//! Hosts MUST complete preflight (MCP or CLI fallback) before invoking
//! phase or mutation-adjacent AGS tools. `ags_agent_instructions` is a
//! read-only bootstrap helper and may be called before preflight so host UIs
//! can discover that preflight is required. `ags_solution_check` is a phase
//! gate, NOT a preflight substitute.
//!
use std::path::PathBuf;

use crate::protocol::ToolListResult;
use serde::Serialize;

pub const TOOL_PREFLIGHT: &str = "ags_preflight";
pub const TOOL_PROTOCOL_STATUS: &str = "ags_protocol_status";
pub const TOOL_AGENT_INSTRUCTIONS: &str = "ags_agent_instructions";
pub const TOOL_TASK_VALIDATE: &str = "ags_task_validate";
pub const TOOL_POLICY_RESOLVE: &str = "ags_policy_resolve";
pub const TOOL_VERIFY_LOCAL: &str = "ags_verify_local";
pub const TOOL_SOLUTION_CHECK: &str = "ags_solution_check";

const LEGACY_TOOL_PREFLIGHT: &str = "ags.preflight";
const LEGACY_TOOL_PROTOCOL_STATUS: &str = "ags.protocol_status";
const LEGACY_TOOL_AGENT_INSTRUCTIONS: &str = "ags.agent_instructions";
const LEGACY_TOOL_TASK_VALIDATE: &str = "ags.task_validate";
const LEGACY_TOOL_POLICY_RESOLVE: &str = "ags.policy_resolve";
const LEGACY_TOOL_VERIFY_LOCAL: &str = "ags.verify_local";
const LEGACY_TOOL_SOLUTION_CHECK: &str = "ags.solution_check";

pub fn is_preflight_tool_name(name: &str) -> bool {
    matches!(name, TOOL_PREFLIGHT | LEGACY_TOOL_PREFLIGHT)
}

pub fn is_preflight_bootstrap_tool_name(name: &str) -> bool {
    matches!(
        name,
        TOOL_PREFLIGHT
            | LEGACY_TOOL_PREFLIGHT
            | TOOL_AGENT_INSTRUCTIONS
            | LEGACY_TOOL_AGENT_INSTRUCTIONS
    )
}

// ── Tool Definitions ─────────────────────────────────────────────────────────

/// Generate MCP `tools/list` response with all available tools.
pub fn list_tools() -> ToolListResult {
    ToolListResult {
        tools: vec![
            tool_def(
                TOOL_PREFLIGHT,
                "MANDATORY FIRST CALL — AGS Initialization Gate. Run AGS session preflight — aggregated agent wake-up check. Combines project detect, protocol status, agent instructions, memory paths, stop conditions, warnings, failures, and next steps into a single read-only report. Must be called before any other AGS tool in AGS scenarios. Does NOT depend on skill governance. If MCP is unavailable, use CLI fallback: `ags session preflight --for <agent>`. If both are unavailable, stop — do not continue AGS scenario tasks.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent": {
                            "type": "string",
                            "description": "Agent identifier. Known examples: codex, claude-code, cursor, tencent-agent, workbuddy, codebuddy-code, cowork. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients. Unknown non-empty identifiers use the generic governed-host profile.",
                            "enum": ["codex", "claude-code", "cursor", "tencent-agent", "workbuddy", "codebuddy-code", "generic"]
                        },
                        "target": {
                            "type": "string",
                            "description": "Target repository path (default: current directory)"
                        }
                    },
                    "required": ["agent"]
                }),
            ),
            tool_def(
                TOOL_PROTOCOL_STATUS,
                "Check AGS protocol file status for a target repository. Reports which protocol files are present or missing, validator entry point, risk boundaries, and verification requirements. Read-only.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": {
                            "type": "string",
                            "description": "Target repository path (default: current directory)"
                        }
                    }
                }),
            ),
            tool_def(
                TOOL_AGENT_INSTRUCTIONS,
                "Export agent-specific project instructions. For Codex/Claude Code/Cursor, returns project-tailored instructions including required reads, stop conditions, and verification commands. For Tencent Agent hosts (WorkBuddy, CodeBuddy-Code), returns AGS global kernel instructions: all development, debugging, review, commit, and task-card work must go through the AGS lifecycle first.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent": {
                            "type": "string",
                            "description": "Agent identifier. Known examples: codex, claude-code, cursor, tencent-agent, workbuddy, codebuddy-code, cowork. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients. Unknown non-empty identifiers use the generic governed-host profile.",
                            "enum": ["codex", "claude-code", "cursor", "tencent-agent", "workbuddy", "codebuddy-code", "generic"]
                        },
                        "target": {
                            "type": "string",
                            "description": "Target repository path (default: current directory)"
                        }
                    },
                    "required": ["agent"]
                }),
            ),
            tool_def(
                TOOL_TASK_VALIDATE,
                "Validate a task card against the AGS canonical format gate. Checks structural format, field values, field combinations, protected paths, contradiction detection, and content quality. Returns validation errors (empty list = valid). Read-only.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task_card": {
                            "type": "string",
                            "description": "Task card markdown text to validate"
                        }
                    },
                    "required": ["task_card"]
                }),
            ),
            tool_def(
                TOOL_POLICY_RESOLVE,
                "Resolve execution policy for a validated task card. Returns effective permission mode, effective parallelism, allowed launch args, downgrade reasons, stop reasons, and confirmation gate requirements. Read-only — never launches a runner. Structured approval signals (never read from the task-card text) are accepted so MCP hosts resolve identical policy to the CLI gate: `approve_writes` (unlocks up to execute-and-verify) and `current_task_approval` (a host-detected live execution instruction; unlocks Heavy edit-with-confirmation only).",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task_card": {
                            "type": "string",
                            "description": "Task card markdown text to resolve policy for"
                        },
                        "approve_writes": {
                            "type": "boolean",
                            "description": "Strong write approval (CLI flag / runner env). Unlocks up to execute-and-verify. Default false."
                        },
                        "current_task_approval": {
                            "type": "boolean",
                            "description": "Structured current-task approval: the host detected an explicit user execution instruction (实现/修复/做完) on the live request. Unlocks Heavy + edit-with-confirmation only (not execute-and-verify). Never derived from task-card text. Default false."
                        }
                    },
                    "required": ["task_card"]
                }),
            ),
            tool_def(
                TOOL_VERIFY_LOCAL,
                "Run AGS local-scope verification checks for a repository. Includes cargo fmt, cargo test, cargo build, fixture validation, YAML checks, and session preflight. Returns structured CheckItem results with pass/fail/skip status. The local gate is fixed-scope and cannot be downgraded by caller input. Read-only.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": {
                            "type": "string",
                            "description": "Target repository path (default: current directory)"
                        }
                    }
                }),
            ),
            tool_def(
                TOOL_SOLUTION_CHECK,
                "Check whether the current phase allows an executable task card. Returns: whether solution formation is still required and whether a task-card instruction is needed (task_card_requested=false blocks executable output with block_reason=task_card_not_requested). It also runs a deterministic prompt-request classifier over `summary` and returns `detected_task_card_request` + `detected_triggers` so the host recognizes \"give me a prompt / generate a task card / hand off to Claude Code\" intent instead of treating it as prose — advisory only: detection alone does NOT authorize a card (the three-gate threshold still requires an explicit task-card instruction). It surfaces two advisory, deterministic routing blocks: `value_route` (效价比路由 — the minimal execution-path form that covers the risk) and `capability_route` (能力路由 — which managed capability to suggest the host wake up for the demand, and whether it is reachable). Both are advisory-only and never change the task level, permission mode, Review gate, or Verification gate; capability_route never auto-invokes anything and never blocks. Optional `active_host` (or `agent`) and `target` shape the capability route for a specific host/repo; when omitted on the MCP server, the values from a successful `ags_preflight` are reused. This is a phase gate, NOT a preflight substitute — preflight must complete first. Read-only.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "User request or current solution summary"
                        },
                        "task_card_requested": {
                            "type": "boolean",
                            "description": "Whether the user has explicitly issued a task-card instruction (\"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.)",
                            "default": false
                        },
                        "active_host": {
                            "type": "string",
                            "description": "Active host the Capability Route targets (e.g. claude-code, codex). Optional. Empty string is host-agnostic (conservative, fail-closed). When omitted, the MCP server reuses the agent recorded by a successful ags_preflight."
                        },
                        "agent": {
                            "type": "string",
                            "description": "Alias for active_host. If both are absent, the MCP server falls back to the preflight agent."
                        },
                        "target": {
                            "type": "string",
                            "description": "Repository path used to read capability manifests for the Capability Route. Optional; resolves the manifest root from this path or any subdirectory. When omitted, the MCP server reuses the target recorded by a successful ags_preflight (default: current directory)."
                        }
                    },
                    "required": ["summary"]
                }),
            ),
        ],
    }
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

// ── Tool Dispatcher ──────────────────────────────────────────────────────────

/// Call a tool by name with the given arguments. Returns a JSON string result.
pub fn call_tool(name: &str, arguments: &serde_json::Value) -> Result<String, String> {
    match name {
        TOOL_PREFLIGHT | LEGACY_TOOL_PREFLIGHT => tool_preflight(arguments),
        TOOL_PROTOCOL_STATUS | LEGACY_TOOL_PROTOCOL_STATUS => tool_protocol_status(arguments),
        TOOL_AGENT_INSTRUCTIONS | LEGACY_TOOL_AGENT_INSTRUCTIONS => {
            tool_agent_instructions(arguments)
        }
        TOOL_TASK_VALIDATE | LEGACY_TOOL_TASK_VALIDATE => tool_task_validate(arguments),
        TOOL_POLICY_RESOLVE | LEGACY_TOOL_POLICY_RESOLVE => tool_policy_resolve(arguments),
        TOOL_VERIFY_LOCAL | LEGACY_TOOL_VERIFY_LOCAL => tool_verify_local(arguments),
        TOOL_SOLUTION_CHECK | LEGACY_TOOL_SOLUTION_CHECK => tool_solution_check(arguments),
        other => Err(format!("Unknown tool: {}", other)),
    }
}

/// Inject preflight-derived context defaults into a tool's arguments.
///
/// Only `ags_solution_check` consumes the `active_host` / `target` context, so
/// every other tool passes through unchanged. Explicit arguments always win: a
/// default is filled ONLY when the corresponding key is absent. An explicitly
/// supplied empty `active_host` (`""`) is a deliberate host-agnostic choice and
/// is left untouched. `agent` is treated as an alias key for `active_host` —
/// when either is already present no host default is injected.
///
/// The server passes the NORMALIZED agent and RESOLVED target it recorded from a
/// successful preflight; this is the only path that fills defaults. Callers that
/// invoke `call_tool` directly without going through the server (e.g. low-level
/// unit tests) get no injection, so an absent host stays host-agnostic
/// (conservative, fail-closed) rather than a fabricated host.
pub fn inject_preflight_defaults(
    tool_name: &str,
    mut arguments: serde_json::Value,
    agent: Option<&str>,
    target: Option<&str>,
) -> serde_json::Value {
    if !matches!(tool_name, TOOL_SOLUTION_CHECK | LEGACY_TOOL_SOLUTION_CHECK) {
        return arguments;
    }
    if let Some(obj) = arguments.as_object_mut() {
        let host_present = obj.contains_key("active_host") || obj.contains_key("agent");
        if !host_present {
            if let Some(a) = agent {
                obj.insert("active_host".to_string(), serde_json::json!(a));
            }
        }
        if !obj.contains_key("target") {
            if let Some(t) = target {
                obj.insert("target".to_string(), serde_json::json!(t));
            }
        }
    }
    arguments
}

// ── Tool Implementations ─────────────────────────────────────────────────────

fn tool_preflight(args: &serde_json::Value) -> Result<String, String> {
    let agent_str = get_string(args, "agent")?;
    let target = get_target(args);

    let agent_type = project_discovery::AgentType::from_str(&agent_str)
        .map_err(|e| format!("Invalid agent: {}", e))?;
    let agent_str_normalized = agent_type.as_str().to_string();
    let mapping_note = if agent_str == agent_str_normalized {
        String::new()
    } else {
        format!(
            "agent '{}' normalized to '{}'",
            agent_str, agent_str_normalized
        )
    };

    let preflight = project_discovery::run_session_preflight(&target, &agent_type);

    #[derive(Serialize)]
    struct PreflightOutput {
        agent: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_mapped_from: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mapping_note: Option<String>,
        target: String,
        integration_status: String,
        is_ags_suite: bool,
        is_ags_integrated: bool,
        protocol_files_found: Vec<String>,
        protocol_files_missing: Vec<String>,
        root_entry_files_found: Vec<String>,
        root_entry_files_missing: Vec<String>,
        validator_available: bool,
        validator_entry: String,
        memory_capsule_path: Option<String>,
        memory_capsule_exists: Option<bool>,
        task_memory_path: Option<String>,
        task_memory_exists: Option<bool>,
        should_stop: bool,
        stop_conditions: Vec<String>,
        verification_commands: Vec<String>,
        default_permission_mode: String,
        overall_status: String,
        warnings: Vec<String>,
        failures: Vec<String>,
        next_steps: Vec<String>,
        exit_code: i32,
        /// Quiet-by-default foreground decision state. Full preflight detail
        /// above stays as audit evidence regardless of this summary.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_status: Option<String>,
    }

    let (from_agent, note) = if agent_str != agent_str_normalized {
        (Some(agent_str), Some(mapping_note))
    } else {
        (None, None)
    };

    let output = PreflightOutput {
        agent: agent_str_normalized,
        agent_mapped_from: from_agent,
        mapping_note: note,
        target: preflight.target.to_string_lossy().to_string(),
        integration_status: format!("{:?}", preflight.integration_status),
        is_ags_suite: preflight.is_ags_suite,
        is_ags_integrated: preflight.is_ags_integrated,
        protocol_files_found: preflight.protocol_files_found,
        protocol_files_missing: preflight.protocol_files_missing,
        root_entry_files_found: preflight.root_entry_files_found,
        root_entry_files_missing: preflight.root_entry_files_missing,
        validator_available: preflight.validator_available,
        validator_entry: preflight.validator_entry,
        memory_capsule_path: preflight
            .memory_capsule_path
            .map(|p| p.to_string_lossy().to_string()),
        memory_capsule_exists: preflight.memory_capsule_exists,
        task_memory_path: preflight
            .task_memory_path
            .map(|p| p.to_string_lossy().to_string()),
        task_memory_exists: preflight.task_memory_exists,
        should_stop: preflight.should_stop,
        stop_conditions: preflight.stop_conditions,
        verification_commands: preflight.verification_commands,
        default_permission_mode: preflight.default_permission_mode,
        overall_status: format!("{:?}", preflight.overall_status),
        warnings: preflight.warnings,
        failures: preflight.failures,
        next_steps: preflight.next_steps,
        exit_code: preflight.exit_code,
        visible_status: Some(
            ags_verify::derive_visible_status(&ags_verify::StatusSignals {
                needs_user_decision: preflight.should_stop,
                ..Default::default()
            })
            .as_str()
            .to_string(),
        ),
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_protocol_status(args: &serde_json::Value) -> Result<String, String> {
    let target = get_target(args);

    let identity = project_discovery::detect_project(&target);
    let status = project_discovery::check_protocol_status(&target);

    #[derive(Serialize)]
    struct ProtocolStatusOutput {
        target: String,
        is_ags_suite: bool,
        integration_status: String,
        protocol_files_status: Vec<ProtocolFile>,
        validator_available: bool,
        validator_entry: String,
        validator_alternate_entry: String,
        present_count: usize,
        missing_count: usize,
        failures: Vec<String>,
        warnings: Vec<String>,
    }

    #[derive(Serialize)]
    struct ProtocolFile {
        name: String,
        present: bool,
        description: String,
        category: String,
    }

    let protocol_files: Vec<ProtocolFile> = status
        .files
        .iter()
        .map(|pf| ProtocolFile {
            name: pf.name.clone(),
            present: pf.present,
            description: pf.description.clone(),
            category: pf.category.clone(),
        })
        .collect();

    let output = ProtocolStatusOutput {
        target: target.to_string_lossy().to_string(),
        is_ags_suite: identity.is_ags_suite,
        integration_status: format!("{:?}", identity.integration_status),
        protocol_files_status: protocol_files,
        validator_available: status.task_card_validator.available,
        validator_entry: status.task_card_validator.entry.clone(),
        validator_alternate_entry: status.task_card_validator.alternate_entry.clone(),
        present_count: status.present_count,
        missing_count: status.missing_count,
        failures: status.failures,
        warnings: status.warnings,
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_agent_instructions(args: &serde_json::Value) -> Result<String, String> {
    let agent_str = get_string(args, "agent")?;
    let target = get_target(args);

    let agent_type = project_discovery::AgentType::from_str(&agent_str)
        .map_err(|e| format!("Invalid agent: {}", e))?;
    let agent_str_normalized = agent_type.as_str().to_string();
    let mapping_note = if agent_str == agent_str_normalized {
        String::new()
    } else {
        format!(
            "agent '{}' normalized to '{}'",
            agent_str, agent_str_normalized
        )
    };

    let instructions = project_discovery::generate_agent_instructions(&target, &agent_type);

    #[derive(Serialize)]
    struct AgentInstructionsOutput {
        agent: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_mapped_from: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mapping_note: Option<String>,
        agent_display_name: String,
        target: String,
        project_name: String,
        is_ags_suite: bool,
        integration_status: String,
        required_reads: Vec<RequiredRead>,
        protocol_entry_points: Vec<String>,
        verification_commands: Vec<String>,
        role_description: String,
        should_stop: bool,
        stop_reasons: Vec<String>,
        stop_conditions: Vec<String>,
        permissions: AgentPerms,
        integration_gaps: Vec<String>,
        protocol_failures: Vec<String>,
        protocol_warnings: Vec<String>,
        exit_code: i32,
        instructions_text: String,
    }

    #[derive(Serialize)]
    struct RequiredRead {
        path: String,
        description: String,
        priority: String,
    }

    #[derive(Serialize)]
    struct AgentPerms {
        default_permission_mode: String,
        default_parallelism: String,
        may_edit_files: bool,
        may_delegate: bool,
        may_install: bool,
    }

    let (from_agent, note) = if agent_str != agent_str_normalized {
        (Some(agent_str), Some(mapping_note))
    } else {
        (None, None)
    };

    let output = AgentInstructionsOutput {
        agent: agent_str_normalized,
        agent_mapped_from: from_agent,
        mapping_note: note,
        agent_display_name: instructions.agent_display_name,
        target: instructions.target.to_string_lossy().to_string(),
        project_name: instructions.project_name,
        is_ags_suite: instructions.is_ags_suite,
        integration_status: format!("{:?}", instructions.integration_status),
        required_reads: instructions
            .required_reads
            .iter()
            .map(|r| RequiredRead {
                path: r.path.clone(),
                description: r.description.clone(),
                priority: r.priority.clone(),
            })
            .collect(),
        protocol_entry_points: instructions.protocol_entry_points,
        verification_commands: instructions.verification_commands,
        role_description: instructions.role_description,
        should_stop: instructions.should_stop,
        stop_reasons: instructions.stop_reasons,
        stop_conditions: instructions.stop_conditions,
        permissions: AgentPerms {
            default_permission_mode: instructions.permissions.default_permission_mode,
            default_parallelism: instructions.permissions.default_parallelism,
            may_edit_files: instructions.permissions.may_edit_files,
            may_delegate: instructions.permissions.may_delegate,
            may_install: instructions.permissions.may_install,
        },
        integration_gaps: instructions.integration_gaps,
        protocol_failures: instructions.protocol_failures,
        protocol_warnings: instructions.protocol_warnings,
        exit_code: instructions.exit_code,
        instructions_text: instructions.instructions_text,
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_task_validate(args: &serde_json::Value) -> Result<String, String> {
    let task_card = get_string(args, "task_card")?;

    let errors = task_card_validator::validate(&task_card);

    #[derive(Serialize)]
    struct ValidateOutput {
        is_valid: bool,
        error_count: usize,
        errors: Vec<String>,
    }

    let output = ValidateOutput {
        is_valid: errors.is_empty(),
        error_count: errors.len(),
        errors,
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_policy_resolve(args: &serde_json::Value) -> Result<String, String> {
    let task_card = get_string(args, "task_card")?;

    // Validate first — policy resolution requires a valid task card
    let errors = task_card_validator::validate(&task_card);
    if !errors.is_empty() {
        #[derive(Serialize)]
        struct PolicyResolveError {
            resolved: bool,
            validation_error: bool,
            validation_errors: Vec<String>,
            hint: String,
        }

        let output = PolicyResolveError {
            resolved: false,
            validation_error: true,
            validation_errors: errors,
            hint: "Task card must pass validation before policy can be resolved. Fix validation errors and retry.".to_string(),
        };

        return serde_json::to_string_pretty(&output)
            .map_err(|e| format!("JSON serialize error: {}", e));
    }

    // Parse and resolve. Structured approval signals are read from explicit
    // args (NEVER from task-card text) and threaded through the same canonical
    // builder the CLI gate uses, so MCP and CLI resolve identical policy.
    let parsed = task_card_validator::parse_validated(&task_card)
        .map_err(|e| format!("Parse error: {:?}", e))?;
    let approve_writes = args
        .get("approve_writes")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let current_task_approval = args
        .get("current_task_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let input = execution_policy::TaskPolicyInput::from_fields_with_approval(
        &parsed.fields,
        approve_writes,
        current_task_approval,
    );
    let policy = execution_policy::resolve_policy(input);

    #[derive(Serialize)]
    struct PolicyResolveOutput {
        resolved: bool,
        executor: String,
        runtime_adapter: String,
        effective_permission_mode: String,
        effective_parallelism: String,
        effective_execution_surface: String,
        allowed_launch_args: Vec<String>,
        stop_before_launch: bool,
        stop_reasons: Vec<serde_json::Value>,
        was_downgraded: bool,
        downgrade_reasons: Vec<serde_json::Value>,
        requires_confirmation_gate: bool,
        execution_effort: String,
        is_exhaustive_mode: bool,
        approval_source: String,
        /// Quiet-by-default foreground decision state. The full downgrade /
        /// stop-reason audit trail above is preserved regardless.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_status: Option<String>,
    }

    // Serialize StopReason enum variants to JSON values
    let stop_reasons: Vec<serde_json::Value> = policy
        .stop_reasons
        .iter()
        .map(|sr| serde_json::to_value(sr).unwrap_or(serde_json::Value::Null))
        .collect();

    // Serialize DowngradeReason structs to JSON values
    let downgrade_reasons: Vec<serde_json::Value> = policy
        .downgrade_reasons
        .iter()
        .map(|dr| serde_json::to_value(dr).unwrap_or(serde_json::Value::Null))
        .collect();

    let output = PolicyResolveOutput {
        resolved: true,
        executor: policy.executor,
        runtime_adapter: policy.runtime_adapter,
        effective_permission_mode: policy.effective_permission_mode.to_string(),
        effective_parallelism: policy.effective_parallelism.to_string(),
        effective_execution_surface: policy.effective_execution_surface,
        allowed_launch_args: policy.allowed_launch_args,
        stop_before_launch: policy.stop_before_launch,
        stop_reasons,
        was_downgraded: policy.was_downgraded,
        downgrade_reasons,
        requires_confirmation_gate: policy.requires_confirmation_gate,
        execution_effort: policy.execution_effort,
        is_exhaustive_mode: policy.is_exhaustive_mode,
        approval_source: policy.approval_source.to_string(),
        visible_status: Some(
            ags_verify::derive_visible_status(&ags_verify::StatusSignals {
                blocked_by_policy: policy.stop_before_launch,
                needs_user_decision: policy.requires_confirmation_gate,
                ..Default::default()
            })
            .as_str()
            .to_string(),
        ),
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_verify_local(args: &serde_json::Value) -> Result<String, String> {
    let target = get_target(args);

    // Fixed local scope. The local verification gate is NOT downgradable by
    // caller input — a caller must never be able to pick a weaker profile and
    // get a "passing" report for source/protocol changes. Diff-aware lane
    // routing is a read-only concern of the push gate's own trusted shell
    // classification, never of this verification endpoint.
    let report = ags_verify::run_verify(ags_verify::Scope::Local, &target);

    serde_json::to_string_pretty(&report).map_err(|e| format!("JSON serialize error: {}", e))
}

fn tool_solution_check(args: &serde_json::Value) -> Result<String, String> {
    let summary = get_string(args, "summary")?;
    let task_card_requested = args
        .get("task_card_requested")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Resolve the active host + target for the advisory Capability Route.
    // Explicit `active_host` wins, then `agent`; the MCP server fills these from
    // a successful preflight when absent (see `inject_preflight_defaults`). An
    // absent or empty host is host-agnostic (conservative, fail-closed) — never a
    // fabricated host. Target resolves the manifest root from itself or any
    // subdirectory; absent → current directory.
    let active_host = args
        .get("active_host")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("agent").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();

    #[derive(Serialize)]
    struct SolutionCheckOutput {
        executable_allowed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        block_reason: Option<String>,
        phase: String,
        task_card_requested: bool,
        /// Deterministic classifier signal: the `summary` text matches a
        /// prompt/task-card request. Advisory only — it does NOT authorize a
        /// task card. The three-gate threshold still requires an explicit
        /// user task-card instruction (`task_card_requested`).
        detected_task_card_request: bool,
        detected_triggers: Vec<String>,
        /// `true` when advisory/consultation intent is detected in the summary.
        #[serde(skip_serializing_if = "Option::is_none")]
        detected_advisory_intent: Option<bool>,
        /// `false` when advisory intent is active and no execution override
        /// clears it. Host must NOT perform write-type tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        mutation_allowed: Option<bool>,
        /// Block reason when advisory intent blocks mutation.
        #[serde(skip_serializing_if = "Option::is_none")]
        advisory_block_reason: Option<String>,
        next_step: String,
        trivial_possible: bool,
        /// Value Route (效价比路由): the minimal execution-path form that still
        /// covers the task's risk, with rejected lighter/heavier alternatives.
        /// Advisory and deterministic — it shapes the path form only and never
        /// changes the Light/Medium/Heavy level, permission mode, Review gate, or
        /// Verification gate. The planner owns the final path.
        value_route: prompt_request_classifier::ValueRoute,
        /// Capability Route (能力路由): which managed capability the host is
        /// ADVISED to wake up for this demand, and whether it is reachable.
        /// Parallel to `value_route` — value_route shapes the execution-path form,
        /// capability_route suggests a third-party capability wakeup. Advisory and
        /// deterministic; additive. It never auto-invokes a skill/MCP/CLI, never
        /// blocks the request, and never changes the task level, permission mode,
        /// Review gate, or Verification gate. Computed for the resolved
        /// `active_host` / `target` (explicit args, else preflight context).
        #[serde(skip_serializing_if = "Option::is_none")]
        capability_route: Option<capability_route::CapabilityRoute>,
        /// Quiet-by-default foreground decision state.
        #[serde(skip_serializing_if = "Option::is_none")]
        visible_status: Option<String>,
    }

    // Deterministic entry intent classification of the summary text. This is
    // advisory: it surfaces when the request *looks like* a task-card/prompt
    // request so the host follows the three-gate threshold instead of treating
    // it as ordinary prose. It does NOT change `executable_allowed` — only an
    // explicit user task-card instruction (`task_card_requested`) authorizes a
    // card.
    let classification = prompt_request_classifier::classify(&summary);
    let detected_task_card_request = classification.is_task_card_request;
    let detected_triggers = classification.matched_triggers.clone();

    // Determine if task sounds trivial (simple typo, trivial fix, etc.)
    let trivial_keywords = ["typo", "typo fix", "missing comma", "fix spelling"];
    let summary_lower = summary.to_lowercase();
    let trivial_possible =
        trivial_keywords.iter().any(|kw| summary_lower.contains(kw)) && summary.len() < 200;

    // Value Route: minimal execution-path form for this solution. Deterministic
    // and advisory — derived from the same classification signals as the entry
    // gate; it does NOT change task level, permission mode, or gates.
    let value_route = prompt_request_classifier::derive_value_route(
        &classification,
        task_card_requested,
        trivial_possible,
    );

    // Capability Route (能力路由): advisory wakeup suggestion for the demand,
    // reachable-or-fallback for the active host. Reads the manifest source of
    // truth at `target`'s manifest root via the shared `capability-route` wiring.
    // Advisory-only and additive — it never blocks, never auto-invokes, and
    // carries no task-level/permission/gate field by construction.
    let cap_route = capability_route::route_request(
        &summary,
        &capability_route::locate_manifest_root(std::path::Path::new(&target)),
        &active_host,
    );

    let (executable_allowed, block_reason, phase) = if !task_card_requested {
        (
            false,
            Some("task_card_not_requested".to_string()),
            "solution_formation",
        )
    } else {
        (true, None, "task_card_requested")
    };

    let next_step = if !task_card_requested {
        let base = "Solution phase is active. If solution is confirmed, user must explicitly issue a task-card instruction (\"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.) before an executable task card can be produced. \"方案 OK\" alone is NOT sufficient — the three-gate threshold is: 方案 OK → 任务卡指令 → 任务分级路由.".to_string();
        if detected_task_card_request {
            format!(
                "NOTE: the summary text matches a prompt/task-card request (triggers: {}). This deterministic detection does NOT authorize a card — the user must still issue an explicit task-card instruction. {}",
                detected_triggers.join(", "),
                base
            )
        } else {
            base
        }
    } else {
        "Task card instruction received. Proceed to task routing (Light/Medium/Heavy) and task card compilation via `ags task compile --task-card-requested`. The final foreground answer must be a canonical `## 任务卡` — self-check with `ags gate output`.".to_string()
    };

    let (advisory_intent, advisory_mutation, advisory_reason) =
        if classification.detected_advisory_intent {
            (
                Some(true),
                Some(classification.mutation_allowed),
                if !classification.mutation_allowed {
                    Some("advisory_intent_no_mutation".to_string())
                } else {
                    None
                },
            )
        } else {
            (None, None, None)
        };

    let output = SolutionCheckOutput {
        executable_allowed,
        block_reason,
        phase: phase.to_string(),
        task_card_requested,
        detected_task_card_request,
        detected_triggers,
        detected_advisory_intent: advisory_intent,
        mutation_allowed: advisory_mutation,
        advisory_block_reason: advisory_reason,
        next_step,
        trivial_possible,
        value_route,
        capability_route: Some(cap_route),
        visible_status: Some(
            ags_verify::derive_visible_status(&ags_verify::StatusSignals {
                advisory_no_mutation: classification.detected_advisory_intent
                    && !classification.mutation_allowed,
                needs_user_decision: !executable_allowed,
                ..Default::default()
            })
            .as_str()
            .to_string(),
        ),
    };

    serde_json::to_string_pretty(&output).map_err(|e| format!("JSON serialize error: {}", e))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Missing required argument: {}", key))
}

fn get_target(args: &serde_json::Value) -> PathBuf {
    args.get("target")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static DISABLE_HOST_PROBES: Once = Once::new();

    fn disable_host_probes_for_tests() {
        DISABLE_HOST_PROBES.call_once(|| {
            std::env::set_var("AGS_DISABLE_HOST_PROBES", "1");
        });
    }

    fn cleanup_local_runtime_artifacts() {
        let private_index_dir = ["g", "ep"].concat();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(private_index_dir);
        let _ = std::fs::remove_dir_all(path);
    }

    fn call_solution_check_json(args: &serde_json::Value) -> serde_json::Value {
        disable_host_probes_for_tests();
        let out = call_tool(TOOL_SOLUTION_CHECK, args).expect("solution_check ok");
        cleanup_local_runtime_artifacts();
        serde_json::from_str(&out).expect("valid json")
    }

    fn run_solution_check(summary: &str, task_card_requested: bool) -> serde_json::Value {
        let args = serde_json::json!({
            "summary": summary,
            "task_card_requested": task_card_requested,
        });
        call_solution_check_json(&args)
    }

    /// Suite root (two levels up from the crate dir) for capability-route tests.
    fn suite_root() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn solution_check_detects_task_card_request_but_does_not_authorize() {
        let v = run_solution_check("给我提示词", false);
        assert_eq!(v["detected_task_card_request"], true);
        assert!(v["detected_triggers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "给我提示词"));
        // Detection alone must NOT authorize a card — three-gate threshold holds.
        assert_eq!(v["executable_allowed"], false);
        assert_eq!(v["block_reason"], "task_card_not_requested");
    }

    #[test]
    fn solution_check_prose_not_detected() {
        let v = run_solution_check("解释这段代码是做什么的", false);
        assert_eq!(v["detected_task_card_request"], false);
        assert!(v["detected_triggers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn solution_check_requested_allows_and_detection_consistent() {
        let v = run_solution_check("按这个方案出任务卡", true);
        assert_eq!(v["executable_allowed"], true);
        assert_eq!(v["detected_task_card_request"], true);
    }

    #[test]
    fn solution_check_exposes_value_route() {
        // Prompt/handoff intent without an instruction → plan-first, with both
        // rejected alternatives and an authority note that disclaims gate change.
        let v = run_solution_check("给我提示词", false);
        let vr = &v["value_route"];
        assert_eq!(vr["recommended_path"], "plan-first");
        assert_eq!(vr["rejected_lighter"]["path"], "direct-edit");
        assert_eq!(vr["rejected_heavier"]["path"], "claude-code-route");
        assert_eq!(vr["requires_user_confirmation"], true);
        assert_eq!(vr["advisory"], true);
        let note = vr["authority_note"]
            .as_str()
            .expect("authority_note string");
        assert!(note.contains("permission mode") && note.contains("Verification gate"));

        // With an explicit instruction → claude-code-route.
        let v2 = run_solution_check("按这个方案出任务卡交给 Claude Code 执行", true);
        assert_eq!(v2["value_route"]["recommended_path"], "claude-code-route");
    }

    #[test]
    fn policy_resolve_emits_visible_status() {
        let card = include_str!("../../../tests/fixtures/valid-full.md");
        let args = serde_json::json!({ "task_card": card });
        let out = call_tool(TOOL_POLICY_RESOLVE, &args).expect("policy resolve ok");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let status = v["visible_status"]
            .as_str()
            .expect("policy resolve must emit visible_status");
        assert!(
            [
                "OK",
                "NEEDS_USER_DECISION",
                "BLOCKED_BY_POLICY",
                "RISK_ESCALATED",
                "DONE_WITH_RECEIPT",
                "ADVISORY_NO_MUTATION"
            ]
            .contains(&status),
            "unexpected visible_status: {status}"
        );
    }

    #[test]
    fn policy_resolve_current_task_approval_unlocks_heavy_edit() {
        // No MCP/CLI policy divergence: the MCP ags_policy_resolve carries the
        // same structured current-task approval signal as the CLI gate. The
        // signal is audit/hint only; task level does not downgrade permission.
        let card = include_str!("../../../tests/fixtures/valid-full.md")
            .replace("任务级别：Light", "任务级别：Heavy")
            .replace(
                "Permission mode: execute-and-verify",
                "Permission mode: edit-with-confirmation",
            )
            .replace(
                "- Light review",
                "- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。",
            );

        // Without approval → still edit-with-confirmation (task level does not downgrade).
        let no = call_tool(
            TOOL_POLICY_RESOLVE,
            &serde_json::json!({ "task_card": card }),
        )
        .expect("policy resolve ok");
        let vno: serde_json::Value = serde_json::from_str(&no).expect("valid json");
        assert_eq!(
            vno["effective_permission_mode"], "edit-with-confirmation",
            "Heavy must NOT be downgraded by task level: {vno}"
        );
        assert_eq!(
            vno["requires_confirmation_gate"], true,
            "Heavy must still require a confirmation gate: {vno}"
        );
        assert_eq!(
            vno["was_downgraded"], false,
            "Heavy edit card must not record a downgrade: {vno}"
        );

        // With current_task_approval → identical resolution.
        let yes = call_tool(
            TOOL_POLICY_RESOLVE,
            &serde_json::json!({ "task_card": card, "current_task_approval": true }),
        )
        .expect("policy resolve ok");
        let vyes: serde_json::Value = serde_json::from_str(&yes).expect("valid json");
        assert_eq!(
            vyes["effective_permission_mode"], "edit-with-confirmation",
            "approval signal must not change the resolved mode: {vyes}"
        );
    }

    #[test]
    fn solution_check_advisory_intent_detected() {
        let v = run_solution_check("评估一下这个方案的风险", false);
        assert_eq!(v["detected_advisory_intent"], true);
        assert_eq!(v["mutation_allowed"], false);
        assert_eq!(v["advisory_block_reason"], "advisory_intent_no_mutation");
        assert_eq!(v["detected_task_card_request"], false);
        assert_eq!(v["visible_status"], "ADVISORY_NO_MUTATION");
    }

    #[test]
    fn solution_check_visible_status_needs_user_when_not_requested() {
        let v = run_solution_check("解释这段代码是做什么的", false);
        assert_eq!(v["visible_status"], "NEEDS_USER_DECISION");
    }

    #[test]
    fn solution_check_visible_status_ok_when_requested() {
        let v = run_solution_check("按这个方案出任务卡", true);
        assert_eq!(v["executable_allowed"], true);
        assert_eq!(v["visible_status"], "OK");
    }

    #[test]
    fn solution_check_advisory_with_override() {
        let v = run_solution_check("评估一下，然后按这个改", false);
        assert_eq!(v["detected_advisory_intent"], true);
        assert_eq!(v["mutation_allowed"], true);
        assert!(v["advisory_block_reason"].is_null());
    }

    #[test]
    fn solution_check_non_advisory_no_advisory_fields() {
        let v = run_solution_check("解释这段代码是做什么的", false);
        assert!(
            v.get("detected_advisory_intent").is_none() || v["detected_advisory_intent"].is_null(),
            "non-advisory should not emit detected_advisory_intent"
        );
    }

    // ── Capability Route (additive, advisory) ────────────────────────────────

    /// `ags_solution_check` exposes BOTH value_route and an advisory
    /// capability_route. A bare `call_tool` (no server injection, no preflight)
    /// stays host-agnostic — never a fabricated host.
    #[test]
    fn solution_check_exposes_capability_route_advisory() {
        let v = run_solution_check("测试挂了，帮我看下", false);
        let cr = &v["capability_route"];
        assert!(!cr.is_null(), "capability_route must be present");
        assert_eq!(cr["advisory"], true);
        assert_eq!(cr["demand_kind"], "debug");
        assert_eq!(
            cr["active_host"], "host-agnostic",
            "no active_host arg + no preflight injection → host-agnostic"
        );
        assert!(
            !v["value_route"].is_null(),
            "value_route must remain present"
        );
    }

    /// Capability Route is advisory-only: it does NOT change the executable gate
    /// decision. The same summary with/without capability data resolves the gate
    /// identically (driven solely by task_card_requested).
    #[test]
    fn solution_check_capability_route_does_not_change_gate() {
        let v = run_solution_check("测试挂了，帮我看下", false);
        assert_eq!(v["executable_allowed"], false);
        assert_eq!(v["block_reason"], "task_card_not_requested");
        // capability_route present but the gate is unaffected by it.
        assert!(!v["capability_route"].is_null());
    }

    /// Explicit `active_host` + `target` in the args drive the route (this is the
    /// path the MCP server uses to inject preflight context).
    #[test]
    fn solution_check_capability_route_uses_explicit_host_and_target() {
        let args = serde_json::json!({
            "summary": "测试挂了，帮我看下",
            "active_host": "claude-code",
            "target": suite_root(),
        });
        let v = call_solution_check_json(&args);
        assert_eq!(v["capability_route"]["active_host"], "claude-code");
        assert_eq!(v["capability_route"]["demand_kind"], "debug");
        // auto-* aliases are retired (route_state: retired → excluded from
        // routing); the debug demand routes to the canonical successor
        // diagnosing-bugs, and the retired auto-debug alias no longer surfaces.
        let names: Vec<&str> = v["capability_route"]["recommendations"]
            .as_array()
            .expect("recommendations array")
            .iter()
            .filter_map(|r| r["capability_name"].as_str())
            .collect();
        assert!(
            names.contains(&"diagnosing-bugs"),
            "debug demand should surface diagnosing-bugs, got {names:?}"
        );
        assert!(
            !names.contains(&"auto-debug"),
            "retired auto-debug alias must not surface, got {names:?}"
        );
    }

    /// The `agent` key is accepted as an alias for `active_host`.
    #[test]
    fn solution_check_capability_route_accepts_agent_alias() {
        let args = serde_json::json!({
            "summary": "测试挂了",
            "agent": "codex",
            "target": suite_root(),
        });
        let out = call_tool(TOOL_SOLUTION_CHECK, &args).expect("solution_check ok");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["capability_route"]["active_host"], "codex");
    }

    /// Ordinary prose with no development demand → capability_route present with
    /// a no-demand status; still advisory, never blocks.
    #[test]
    fn solution_check_capability_route_no_demand_on_prose() {
        let v = run_solution_check("解释这段代码是做什么的", false);
        assert_eq!(v["capability_route"]["status"], "no-demand-detected");
        assert_eq!(v["capability_route"]["advisory"], true);
    }

    /// `inject_preflight_defaults` fills active_host/target only when absent;
    /// explicit values always win, and non-solution-check tools pass through.
    #[test]
    fn inject_preflight_defaults_fills_only_absent_keys() {
        // Absent → filled from preflight context.
        let args = serde_json::json!({"summary": "x"});
        let out =
            inject_preflight_defaults(TOOL_SOLUTION_CHECK, args, Some("codex"), Some("/repo"));
        assert_eq!(out["active_host"], "codex");
        assert_eq!(out["target"], "/repo");

        // Explicit active_host wins; explicit target wins.
        let args = serde_json::json!({"summary": "x", "active_host": "claude-code", "target": "/explicit"});
        let out =
            inject_preflight_defaults(TOOL_SOLUTION_CHECK, args, Some("codex"), Some("/repo"));
        assert_eq!(out["active_host"], "claude-code");
        assert_eq!(out["target"], "/explicit");

        // Explicit empty active_host is a deliberate host-agnostic choice — kept.
        let args = serde_json::json!({"summary": "x", "active_host": ""});
        let out = inject_preflight_defaults(TOOL_SOLUTION_CHECK, args, Some("codex"), None);
        assert_eq!(out["active_host"], "");

        // Non-solution-check tool: pass through unchanged.
        let args = serde_json::json!({"agent": "claude-code"});
        let out = inject_preflight_defaults(TOOL_PREFLIGHT, args, Some("codex"), Some("/repo"));
        assert!(out.get("active_host").is_none(), "preflight args untouched");
        assert!(out.get("target").is_none(), "preflight args untouched");
    }
}
