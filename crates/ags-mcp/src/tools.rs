//! AGS MCP governance tools.
//!
//! `ags_route_request` is the only natural-language entry. It returns the
//! canonical `RequestDecision`, resolves Skill targets from a validated
//! ActiveSkillTable snapshot, and invokes MachineCli targets through the real
//! `ags` executable without a shell.

use crate::protocol::ToolListResult;
use request_router::{
    route_request, CliCapabilityId, RequestContext, RequestDecision, RouteTarget, TypedCliInput,
};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const TOOL_PREFLIGHT: &str = "ags_preflight";
pub const TOOL_PROTOCOL_STATUS: &str = "ags_protocol_status";
pub const TOOL_AGENT_INSTRUCTIONS: &str = "ags_agent_instructions";
pub const TOOL_TASK_VALIDATE: &str = "ags_task_validate";
pub const TOOL_POLICY_RESOLVE: &str = "ags_policy_resolve";
pub const TOOL_VERIFY_LOCAL: &str = "ags_verify_local";
pub const TOOL_ROUTE_REQUEST: &str = "ags_route_request";

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
                    "required": ["agent"]
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
                    "required": ["task_card"]
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
                "Run the fixed local AGS verification scope.",
                target_schema(),
            ),
            tool_def(
                TOOL_ROUTE_REQUEST,
                "The unique AGS natural-language requirement router. The host supplies complete conversation context and structured approval evidence; AGS remains stateless.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "request": { "type": "string" },
                        "approved_contract": { "type": "boolean", "default": false },
                        "confirmed_handoff_contract": { "type": "boolean", "default": false },
                        "active_host": { "type": "string" },
                        "target": { "type": "string" }
                    },
                    "required": ["request"]
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

pub fn call_tool(name: &str, arguments: &serde_json::Value) -> Result<String, String> {
    match name {
        TOOL_PREFLIGHT => tool_preflight(arguments),
        TOOL_PROTOCOL_STATUS => tool_protocol_status(arguments),
        TOOL_AGENT_INSTRUCTIONS => tool_agent_instructions(arguments),
        TOOL_TASK_VALIDATE => tool_task_validate(arguments),
        TOOL_POLICY_RESOLVE => tool_policy_resolve(arguments),
        TOOL_VERIFY_LOCAL => tool_verify_local(arguments),
        TOOL_ROUTE_REQUEST => tool_route_request(arguments),
        other => Err(format!("Unknown tool: {other}")),
    }
}

pub fn inject_preflight_defaults(
    tool_name: &str,
    mut arguments: serde_json::Value,
    preflight_agent: Option<&str>,
    preflight_target: Option<&str>,
) -> serde_json::Value {
    if tool_name != TOOL_ROUTE_REQUEST {
        return arguments;
    }
    let Some(object) = arguments.as_object_mut() else {
        return arguments;
    };
    if !object.contains_key("active_host") {
        if let Some(agent) = preflight_agent {
            object.insert("active_host".to_string(), serde_json::json!(agent));
        }
    }
    if !object.contains_key("target") {
        if let Some(target) = preflight_target {
            object.insert("target".to_string(), serde_json::json!(target));
        }
    }
    arguments
}

fn tool_preflight(args: &serde_json::Value) -> Result<String, String> {
    let agent = get_string(args, "agent")?;
    let agent_type = project_discovery::AgentType::from_str(&agent)
        .map_err(|error| format!("Invalid agent: {error}"))?;
    let report = project_discovery::run_session_preflight(&get_target(args), &agent_type);
    let mut value = serde_json::to_value(report).map_err(json_error)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("agent".to_string(), serde_json::json!(agent_type.as_str()));
    }
    pretty(&value)
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

fn tool_verify_local(args: &serde_json::Value) -> Result<String, String> {
    pretty(&ags_verify::run_verify(
        ags_verify::Scope::Local,
        &get_target(args),
    ))
}

#[derive(Debug, Serialize)]
struct RouteOutput {
    decision: RequestDecision,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skill_results: Vec<SkillRouteResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    machine_results: Vec<MachineCliResult>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SkillRouteResult {
    Selected {
        selection: skill_resolver::SkillSelection,
    },
    GovernancePrecondition {
        code: &'static str,
        message: String,
    },
}

#[derive(Debug, Serialize)]
struct MachineCliResult {
    capability: CliCapabilityId,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn tool_route_request(args: &serde_json::Value) -> Result<String, String> {
    tool_route_request_with_runtime_home(args, &skill_resolver::locate_runtime_home())
}

fn tool_route_request_with_runtime_home(
    args: &serde_json::Value,
    runtime_home: &Path,
) -> Result<String, String> {
    let request = get_string(args, "request")?;
    let active_host = args
        .get("active_host")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let target = get_target(args);
    let manifest_root = skill_resolver::locate_manifest_root(&target);
    let decision = route_request(RequestContext {
        request: &request,
        approved_contract: bool_arg(args, "approved_contract"),
        confirmed_handoff_contract: bool_arg(args, "confirmed_handoff_contract"),
    });

    let mut skill_results = Vec::new();
    let mut machine_results = Vec::new();
    for target_route in &decision.targets {
        match target_route {
            RouteTarget::DirectResponse => {}
            RouteTarget::Skill { demand } => {
                match skill_resolver::load_validated_snapshot(
                    &manifest_root,
                    runtime_home,
                    active_host,
                ) {
                    Err(_) => skill_results.push(SkillRouteResult::GovernancePrecondition {
                        code: "skill_snapshot_stale",
                        message: "Skill routing requires a current machine snapshot; run `ags capability snapshot --host <host> --write`.".to_string(),
                    }),
                    Ok((_, table)) => match skill_resolver::resolve_skill(*demand, &table) {
                        Ok(selection) => {
                            skill_results.push(SkillRouteResult::Selected { selection })
                        }
                        Err(_) => skill_results.push(SkillRouteResult::GovernancePrecondition {
                            code: "skill_demand_missing",
                            message: format!(
                                "The current ActiveSkillTable has no exact mapping for {demand:?}."
                            ),
                        }),
                    },
                }
            }
            RouteTarget::MachineCli { capability, input } => {
                machine_results.push(invoke_machine_cli(*capability, input, &target)?);
            }
        }
    }

    pretty(&RouteOutput {
        decision,
        skill_results,
        machine_results,
    })
}

fn invoke_machine_cli(
    capability: CliCapabilityId,
    input: &TypedCliInput,
    target: &Path,
) -> Result<MachineCliResult, String> {
    let executable = std::env::var_os("AGS_CLI_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| "cannot resolve current AGS executable".to_string())?;
    let (arguments, stdin) = machine_invocation(capability, input, target)?;
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
    target: &Path,
) -> Result<(Vec<String>, String), String> {
    let stdin = match input {
        TypedCliInput::RequestText { text } => text.clone(),
        TypedCliInput::TaskCard { content } => content.clone(),
        TypedCliInput::Target { .. } | TypedCliInput::Empty => String::new(),
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
        CliCapabilityId::TaskExecute => vec!["run", "-", "--format", "json"],
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
            return Err("SkillTagsVerify requires a structured task-card input".to_string())
        }
        CliCapabilityId::ReceiptVerify => vec!["receipt", "verify", "-", "--format", "json"],
    };
    Ok((args.into_iter().map(str::to_string).collect(), stdin))
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

    #[test]
    fn exposes_exactly_one_natural_language_router() {
        let tools = list_tools();
        assert_eq!(tools.tools.len(), 7);
        assert!(tools
            .tools
            .iter()
            .any(|tool| tool.name == TOOL_ROUTE_REQUEST));
        let retired = concat!("ags_", "solution", "_check");
        assert!(!tools.tools.iter().any(|tool| tool.name == retired));
    }

    #[test]
    fn direct_response_does_not_read_skill_snapshot_or_invoke_cli() {
        let output = call_tool(
            TOOL_ROUTE_REQUEST,
            &serde_json::json!({ "request": "按已确认结构压缩内容" }),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["decision"]["targets"][0]["kind"], "direct_response");
        assert!(value.get("skill_results").is_none());
        assert!(value.get("machine_results").is_none());
    }

    #[test]
    fn skill_route_fails_closed_when_snapshot_is_missing() {
        let missing_runtime_home =
            std::env::temp_dir().join(format!("ags-mcp-missing-snapshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing_runtime_home);
        let output = tool_route_request_with_runtime_home(
            &serde_json::json!({
                "request": "设计跨模块的新系统架构和架构边界",
                "active_host": "codex",
                "target": env!("CARGO_MANIFEST_DIR")
            }),
            &missing_runtime_home,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value["skill_results"][0]["status"],
            "governance_precondition"
        );
        assert_eq!(value["skill_results"][0]["code"], "skill_snapshot_stale");
    }

    #[test]
    fn machine_cli_mapping_is_fixed_and_shell_free() {
        let (args, stdin) = machine_invocation(
            CliCapabilityId::TaskCompile,
            &TypedCliInput::RequestText {
                text: "contract".to_string(),
            },
            Path::new("."),
        )
        .unwrap();
        assert_eq!(args[0..3], ["task", "compile", "-"]);
        assert_eq!(stdin, "contract");
    }

    #[test]
    fn preflight_defaults_apply_only_to_route_tool() {
        let routed = inject_preflight_defaults(
            TOOL_ROUTE_REQUEST,
            serde_json::json!({ "request": "解释代码" }),
            Some("codex"),
            Some("/repo"),
        );
        assert_eq!(routed["active_host"], "codex");
        assert_eq!(routed["target"], "/repo");

        let untouched = inject_preflight_defaults(
            TOOL_PREFLIGHT,
            serde_json::json!({ "agent": "codex" }),
            Some("claude-code"),
            Some("/repo"),
        );
        assert!(untouched.get("target").is_none());
    }
}
