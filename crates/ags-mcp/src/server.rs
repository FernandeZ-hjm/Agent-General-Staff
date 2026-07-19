//! stdio JSON-RPC server loop for AGS MCP.
//!
//! Reads JSON-RPC messages from stdin, dispatches to tool/resource/prompt
//! handlers, and writes JSON-RPC responses to stdout. Stderr is reserved
//! for logging and must never contain JSON-RPC messages.
//!
//! # Initialization Gate (Hard Enforcement)
//!
//! After MCP `initialize`, the server tracks per-connection preflight state.
//! All `tools/call` requests (except `ags_preflight` itself) and phase-gated
//! `prompts/get` requests are blocked until `ags_preflight` completes.
//! `tools/list`, static protocol resources, and `prompts/list` are always
//! allowed. The current-host capability resource is read-only but remains
//! preflight-bound because it represents one specific host/target pair.

use std::io::{BufRead, BufReader, Write};

use crate::protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, PromptsCapability, ResourcesCapability,
    ServerCapabilities, ServerInfo, ToolsCapability, MCP_VERSION, SERVER_NAME, SERVER_VERSION,
};
use crate::{prompts, resources, tools};

// ── Preflight State ─────────────────────────────────────────────────────────

/// Per-connection preflight state for the AGS Initialization Gate.
///
/// After MCP `initialize`, the server requires `ags_preflight` (MCP tool)
/// or CLI fallback before any other governed tool or phase-gated prompt.
/// State is scoped to the stdio connection — it is destroyed when the
/// connection ends.
#[derive(Debug)]
struct PreflightState {
    preflight_completed: bool,
    preflight_agent: Option<String>,
    /// Resolved target path from the successful preflight RESULT (never raw call
    /// arguments). It is the mandatory target binding for route/apply; callers
    /// cannot override it in either tool.
    preflight_target: Option<String>,
    routing_session: tools::RoutingSession,
}

impl PreflightState {
    fn new() -> Self {
        Self {
            preflight_completed: false,
            preflight_agent: None,
            preflight_target: None,
            routing_session: tools::RoutingSession::default(),
        }
    }

    /// Record a successful preflight. `agent` is the NORMALIZED agent and
    /// `target` is the RESOLVED target — both taken from the preflight result
    /// JSON, not the raw call arguments.
    fn mark_completed(&mut self, agent: Option<String>, target: Option<String>) {
        self.preflight_completed = true;
        self.preflight_agent = agent;
        self.preflight_target = target;
    }

    fn binding(&self) -> Option<tools::PreflightBinding> {
        if !self.preflight_completed {
            return None;
        }
        Some(tools::PreflightBinding {
            host: self.preflight_agent.clone()?,
            target: self.preflight_target.as_deref()?.into(),
            host_home: std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
        })
    }
}

fn is_successful_preflight_result(result: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(result) else {
        return false;
    };

    let exit_code_ok = value
        .get("exit_code")
        .and_then(|v| v.as_i64())
        .is_some_and(|code| code == 0);
    let should_stop = value
        .get("should_stop")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let failures_empty = value
        .get("failures")
        .and_then(|v| v.as_array())
        .is_some_and(|failures| failures.is_empty());

    exit_code_ok && !should_stop && failures_empty
}

/// Extract the normalized agent and resolved target from a successful preflight
/// result JSON. These come from the preflight OUTPUT (normalized agent, resolved
/// target path), never from the raw call arguments, so later phase tools reuse
/// the same context AGS actually resolved.
fn preflight_context_from_result(result: &str) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(result) else {
        return (None, None);
    };
    let agent = value
        .get("agent")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let target = value
        .get("target")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (agent, target)
}

/// Prompts that enter an AGS lifecycle phase and therefore require preflight.
const PHASE_GATED_PROMPTS: &[&str] = &["ags_solution_phase", "ags_task_card_request_gate"];

/// Error message returned when a gated operation is attempted before preflight.
const PREFLIGHT_GATE_ERROR: &str =
    "AGS Initialization Gate: ags_preflight must be called first on the ags MCP server. \
     Use MCP: call ags_preflight tool with agent parameter. \
     CLI fallback: run `ags session preflight --for <agent> [--target <path>]`. \
     If both are unavailable, stop — do not continue AGS scenario tasks.";

// ── Server Loop ─────────────────────────────────────────────────────────────

/// Run the MCP server loop on stdio.
///
/// Reads line-delimited JSON-RPC messages from stdin, dispatches each to
/// the appropriate handler, and writes the response to stdout. Returns
/// when stdin is closed or an unrecoverable error occurs.
pub fn run_mcp_server() {
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut initialized = false;
    let mut preflight = PreflightState::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                log_error(&format!("stdin read error: {}", e));
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try parsing as request (has `id`) or notification (no `id`)
        match serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(req) => {
                // Messages without an `id` are notifications — do not respond
                if req.id.is_none() {
                    if req.method == "notifications/initialized" {
                        // Client confirms initialization complete — no response needed
                    } else {
                        log_error(&format!("unhandled notification: {}", req.method));
                    }
                    continue;
                }

                let response = if !initialized && req.method != "initialize" {
                    JsonRpcResponse::error(
                        req.id,
                        -32002,
                        "Not initialized — send initialize request first",
                    )
                } else {
                    dispatch_request(&req, &mut initialized, &mut preflight)
                };
                write_response(&response);
            }
            Err(_) => {
                // Try parsing as notification
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(val) => {
                        let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("");
                        if method == "notifications/initialized" {
                            // Client confirms initialization complete — no response needed
                        } else {
                            log_error(&format!("unhandled notification: {}", method));
                        }
                    }
                    Err(e) => {
                        log_error(&format!("cannot parse message: {} — raw: {}", e, trimmed));
                        // Write a parse error response without an id
                        let err = JsonRpcResponse::error(None, -32700, "Parse error");
                        write_response(&err);
                    }
                }
            }
        }
    }
}

// ── Request Dispatch ────────────────────────────────────────────────────────

fn dispatch_request(
    req: &JsonRpcRequest,
    initialized: &mut bool,
    preflight: &mut PreflightState,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => handle_initialize(req, initialized, preflight),
        "tools/list" => handle_tools_list(req),
        "tools/call" => handle_tools_call(req, preflight),
        "resources/list" => handle_resources_list(req),
        "resources/read" => handle_resources_read(req, preflight),
        "prompts/list" => handle_prompts_list(req),
        "prompts/get" => handle_prompts_get(req, preflight),
        "ping" => JsonRpcResponse::success(req.id.clone(), serde_json::json!({})),
        _ => JsonRpcResponse::method_not_found(req.id.clone()),
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

fn handle_initialize(
    req: &JsonRpcRequest,
    initialized: &mut bool,
    preflight: &mut PreflightState,
) -> JsonRpcResponse {
    let result = InitializeResult {
        protocolVersion: MCP_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability {
                listChanged: Some(false),
            }),
            resources: Some(ResourcesCapability {
                subscribe: Some(false),
                listChanged: Some(false),
            }),
            prompts: Some(PromptsCapability {
                listChanged: Some(false),
            }),
        },
        serverInfo: ServerInfo {
            name: SERVER_NAME.to_string(),
            version: SERVER_VERSION.to_string(),
        },
    };

    *initialized = true;
    // Reset preflight state on re-initialize (new connection semantics)
    *preflight = PreflightState::new();

    let json_result = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
    JsonRpcResponse::success(req.id.clone(), json_result)
}

/// `tools/list` — always allowed (discovery operation, no preflight required).
fn handle_tools_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    let tools = tools::list_tools();
    let result = serde_json::to_value(&tools).unwrap_or(serde_json::Value::Null);
    JsonRpcResponse::success(req.id.clone(), result)
}

/// `tools/call` — `ags_preflight` and read-only bootstrap instructions are
/// allowed before preflight; phase/mutation-adjacent tools require preflight.
fn handle_tools_call(req: &JsonRpcRequest, preflight: &mut PreflightState) -> JsonRpcResponse {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return JsonRpcResponse::invalid_params(req.id.clone(), "params required"),
    };

    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return JsonRpcResponse::invalid_params(req.id.clone(), "params.name required");
        }
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // ── Initialization Gate: block non-preflight tools before preflight ──
    if !tools::is_preflight_bootstrap_tool_name(tool_name) && !preflight.preflight_completed {
        return JsonRpcResponse::error(req.id.clone(), -32000, PREFLIGHT_GATE_ERROR);
    }

    // Every preflight attempt invalidates actions from the preceding binding,
    // even when the new preflight ultimately reports a stop condition.
    if tools::is_preflight_tool_name(tool_name) {
        preflight.routing_session.invalidate();
    }

    let binding = preflight.binding();

    match tools::call_tool(
        tool_name,
        &arguments,
        binding.as_ref(),
        &mut preflight.routing_session,
    ) {
        Ok(result) => {
            // Mark preflight as completed only when the preflight report itself
            // is clean. A successful JSON-RPC tool call may still report
            // overall_status=Stop / exit_code=1 for an ungoverned target.
            if tools::is_preflight_tool_name(tool_name) && is_successful_preflight_result(&result) {
                // Use the NORMALIZED agent + RESOLVED target from the preflight
                // result JSON, not the raw call arguments.
                let (agent, target) = preflight_context_from_result(&result);
                preflight.mark_completed(agent, target);
                log_error(&format!(
                    "preflight completed for agent: {} target: {}",
                    preflight.preflight_agent.as_deref().unwrap_or("unknown"),
                    preflight.preflight_target.as_deref().unwrap_or("unknown"),
                ));
            }

            let content = vec![serde_json::json!({
                "type": "text",
                "text": result,
            })];
            let response = serde_json::json!({ "content": content });
            JsonRpcResponse::success(req.id.clone(), response)
        }
        Err(e) => JsonRpcResponse::internal_error(req.id.clone(), &e),
    }
}

/// `resources/list` — always allowed (discovery operation).
fn handle_resources_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    let res = resources::list_resources();
    let result = serde_json::to_value(&res).unwrap_or(serde_json::Value::Null);
    JsonRpcResponse::success(req.id.clone(), result)
}

/// `resources/read` — static protocol documentation is always allowed; the
/// current-host capability catalog requires the successful preflight binding.
fn handle_resources_read(req: &JsonRpcRequest, preflight: &PreflightState) -> JsonRpcResponse {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return JsonRpcResponse::invalid_params(req.id.clone(), "params required"),
    };

    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => {
            return JsonRpcResponse::invalid_params(req.id.clone(), "params.uri required");
        }
    };

    if uri == tools::CURRENT_HOST_CAPABILITIES_URI {
        let Some(binding) = preflight.binding() else {
            return JsonRpcResponse::error(req.id.clone(), -32000, PREFLIGHT_GATE_ERROR);
        };
        return match tools::read_current_host_catalog(
            &binding,
            &skill_resolver::locate_runtime_home(),
        ) {
            Ok(snapshot) => {
                let text = serde_json::to_string_pretty(&snapshot)
                    .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"));
                let result = crate::protocol::ResourceReadResult {
                    contents: vec![crate::protocol::ResourceContent {
                        uri: tools::CURRENT_HOST_CAPABILITIES_URI.to_string(),
                        mimeType: Some("application/json".to_string()),
                        text,
                    }],
                };
                let value = serde_json::to_value(result).unwrap_or(serde_json::Value::Null);
                JsonRpcResponse::success(req.id.clone(), value)
            }
            Err(error) => JsonRpcResponse::internal_error(req.id.clone(), &error),
        };
    }

    match resources::read_resource(uri) {
        Ok(result) => {
            let val = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
            JsonRpcResponse::success(req.id.clone(), val)
        }
        Err(e) => JsonRpcResponse::internal_error(req.id.clone(), &e),
    }
}

/// `prompts/list` — always allowed (discovery operation).
fn handle_prompts_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    let p = prompts::list_prompts();
    let result = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
    JsonRpcResponse::success(req.id.clone(), result)
}

/// `prompts/get` — reference prompts allowed without preflight;
/// phase-entry prompts require preflight.
fn handle_prompts_get(req: &JsonRpcRequest, preflight: &PreflightState) -> JsonRpcResponse {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return JsonRpcResponse::invalid_params(req.id.clone(), "params required"),
    };

    let prompt_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return JsonRpcResponse::invalid_params(req.id.clone(), "params.name required");
        }
    };

    // ── Initialization Gate: block phase-gated prompts before preflight ──
    if PHASE_GATED_PROMPTS.contains(&prompt_name) && !preflight.preflight_completed {
        return JsonRpcResponse::error(req.id.clone(), -32000, PREFLIGHT_GATE_ERROR);
    }

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    match prompts::get_prompt(prompt_name, &arguments) {
        Ok(result) => {
            let val = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
            JsonRpcResponse::success(req.id.clone(), val)
        }
        Err(e) => JsonRpcResponse::internal_error(req.id.clone(), &e),
    }
}

// ── I/O helpers ──────────────────────────────────────────────────────────────

fn write_response(response: &JsonRpcResponse) {
    let json = serde_json::to_string(response).unwrap_or_else(|e| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32603,"message":"Serialization error: {}"}}}}"#,
            e
        )
    });
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", json);
    let _ = stdout.flush();
}

fn log_error(msg: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "[ags-mcp] {}", msg);
    let _ = stderr.flush();
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcRequest;
    use serde_json::json;

    /// Build a minimal JSON-RPC request for testing handlers directly.
    fn make_request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: method.to_string(),
            params,
        }
    }

    fn has_error(response: &JsonRpcResponse) -> bool {
        response.error.is_some()
    }

    fn is_success(response: &JsonRpcResponse) -> bool {
        response.result.is_some() && response.error.is_none()
    }

    fn error_contains(response: &JsonRpcResponse, needle: &str) -> bool {
        response
            .error
            .as_ref()
            .map(|e| e.message.contains(needle))
            .unwrap_or(false)
    }

    fn suite_root() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("suite root should canonicalize")
            .to_string_lossy()
            .to_string()
    }

    // ── tools/call gate tests ───────────────────────────────────────────

    #[test]
    fn tools_list_always_allowed() {
        let req = make_request("tools/list", None);
        let resp = handle_tools_list(&req);
        assert!(is_success(&resp), "tools/list must always succeed");
    }

    #[test]
    fn tools_list_exposes_schema_safe_tool_names() {
        let req = make_request("tools/list", None);
        let resp = handle_tools_list(&req);
        let tools = resp
            .result
            .as_ref()
            .and_then(|result| result.get("tools"))
            .and_then(|tools| tools.as_array())
            .expect("tools/list result must contain tools array");

        let names: Vec<&str> = tools
            .iter()
            .map(|tool| {
                tool.get("name")
                    .and_then(|name| name.as_str())
                    .expect("each tool must have a string name")
            })
            .collect();

        assert_eq!(names.len(), 8, "AGS MCP should expose exactly 8 tools");
        assert!(names.contains(&tools::TOOL_PREFLIGHT));
        assert!(
            names.iter().all(|name| !name.contains('.')),
            "tools/list must not expose dotted tool names: {:?}",
            names
        );
    }

    #[test]
    fn preflight_tool_allowed_before_preflight() {
        let mut preflight = PreflightState::new();
        let params = json!({
            "name": "ags_preflight",
            "arguments": {"agent": "claude-code", "target": suite_root()}
        });
        let req = make_request("tools/call", Some(params));
        let resp = handle_tools_call(&req, &mut preflight);
        assert!(
            is_success(&resp),
            "preflight must be allowed before preflight"
        );
        assert!(
            preflight.preflight_completed,
            "preflight state must be marked completed"
        );
        assert_eq!(
            preflight.preflight_agent.as_deref(),
            Some("claude-code"),
            "preflight agent must be recorded"
        );
        // Target must be recorded from the RESOLVED preflight result, not raw args.
        let recorded_target = preflight
            .preflight_target
            .as_deref()
            .expect("preflight target must be recorded from the result");
        assert_eq!(
            recorded_target,
            suite_root(),
            "recorded target should be the resolved suite root"
        );
    }

    #[test]
    fn failed_preflight_does_not_open_gate() {
        let mut preflight = PreflightState::new();
        let missing_target = std::env::temp_dir()
            .join("ags-mcp-missing-preflight-target")
            .join("does-not-exist");
        let params = json!({
            "name": "ags_preflight",
            "arguments": {
                "agent": "codex",
                "target": missing_target.to_string_lossy()
            }
        });
        let req = make_request("tools/call", Some(params));
        let resp = handle_tools_call(&req, &mut preflight);
        assert!(is_success(&resp), "failed preflight still returns a report");
        assert!(
            !preflight.preflight_completed,
            "failed preflight must not open the gate"
        );

        let gated_params = json!({"name": "ags_route_request", "arguments": {"request": "after failed preflight"}});
        let gated_req = make_request("tools/call", Some(gated_params));
        let gated_resp = handle_tools_call(&gated_req, &mut preflight);
        assert!(
            has_error(&gated_resp),
            "gated tools must remain blocked after failed preflight"
        );
        assert!(error_contains(&gated_resp, "Initialization Gate"));
    }

    #[test]
    fn non_preflight_tool_blocked_before_preflight() {
        let mut preflight = PreflightState::new();
        let params = json!({"name": "ags_route_request", "arguments": {"request": "test"}});
        let req = make_request("tools/call", Some(params));
        let resp = handle_tools_call(&req, &mut preflight);
        assert!(
            has_error(&resp),
            "ags_route_request must be blocked before preflight"
        );
        assert!(
            error_contains(&resp, "Initialization Gate"),
            "error must mention Initialization Gate"
        );
    }

    #[test]
    fn agent_instructions_allowed_before_preflight_without_opening_gate() {
        let mut preflight = PreflightState::new();
        let params = json!({
            "name": "ags_agent_instructions",
            "arguments": {"agent": "workbuddy", "target": suite_root()}
        });
        let req = make_request("tools/call", Some(params));
        let resp = handle_tools_call(&req, &mut preflight);
        assert!(
            is_success(&resp),
            "ags_agent_instructions must be available as a read-only bootstrap helper"
        );
        assert!(
            !preflight.preflight_completed,
            "agent instructions must not satisfy the initialization gate"
        );

        let gated_params =
            json!({"name": "ags_route_request", "arguments": {"request": "still gated"}});
        let gated_req = make_request("tools/call", Some(gated_params));
        let gated_resp = handle_tools_call(&gated_req, &mut preflight);
        assert!(
            has_error(&gated_resp),
            "phase tools must remain blocked until ags_preflight succeeds"
        );
    }

    #[test]
    fn non_preflight_tool_allowed_after_preflight() {
        let mut preflight = PreflightState::new();
        preflight.mark_completed(Some("claude-code".to_string()), None);

        let params = json!({"name": "ags_protocol_status", "arguments": {}});
        let req = make_request("tools/call", Some(params));
        let resp = handle_tools_call(&req, &mut preflight);
        assert!(
            is_success(&resp),
            "ags_protocol_status must be allowed after preflight"
        );
    }

    #[test]
    fn preflight_repeated_call_updates_state() {
        let mut preflight = PreflightState::new();

        // First preflight
        let target = suite_root();
        let params1 = json!({
            "name": "ags_preflight",
            "arguments": {"agent": "codex", "target": target}
        });
        let req1 = make_request("tools/call", Some(params1));
        let _ = handle_tools_call(&req1, &mut preflight);
        assert_eq!(preflight.preflight_agent.as_deref(), Some("codex"));

        // Second preflight with different agent
        let params2 = json!({
            "name": "ags_preflight",
            "arguments": {"agent": "claude-code", "target": suite_root()}
        });
        let req2 = make_request("tools/call", Some(params2));
        let resp2 = handle_tools_call(&req2, &mut preflight);
        assert!(is_success(&resp2), "repeated preflight must succeed");
        assert_eq!(
            preflight.preflight_agent.as_deref(),
            Some("claude-code"),
            "agent must be updated on repeat preflight"
        );
    }

    // ── prompts/get gate tests ──────────────────────────────────────────

    #[test]
    fn reference_prompt_allowed_before_preflight() {
        let preflight = PreflightState::new();
        let params = json!({"name": "ags_global_kernel"});
        let req = make_request("prompts/get", Some(params));
        let resp = handle_prompts_get(&req, &preflight);
        assert!(
            is_success(&resp),
            "ags_global_kernel reference prompt must be allowed before preflight"
        );
    }

    #[test]
    fn delivery_report_prompt_allowed_before_preflight() {
        let preflight = PreflightState::new();
        let params = json!({"name": "ags_delivery_report"});
        let req = make_request("prompts/get", Some(params));
        let resp = handle_prompts_get(&req, &preflight);
        assert!(
            is_success(&resp),
            "ags_delivery_report reference prompt must be allowed before preflight"
        );
    }

    #[test]
    fn solution_phase_prompt_blocked_before_preflight() {
        let preflight = PreflightState::new();
        let params = json!({"name": "ags_solution_phase", "arguments": {"user_request": "test"}});
        let req = make_request("prompts/get", Some(params));
        let resp = handle_prompts_get(&req, &preflight);
        assert!(
            has_error(&resp),
            "ags_solution_phase must be blocked before preflight"
        );
        assert!(error_contains(&resp, "Initialization Gate"));
    }

    #[test]
    fn task_card_request_gate_prompt_blocked_before_preflight() {
        let preflight = PreflightState::new();
        let params = json!({"name": "ags_task_card_request_gate"});
        let req = make_request("prompts/get", Some(params));
        let resp = handle_prompts_get(&req, &preflight);
        assert!(
            has_error(&resp),
            "ags_task_card_request_gate must be blocked before preflight"
        );
    }

    #[test]
    fn solution_phase_prompt_allowed_after_preflight() {
        let mut preflight = PreflightState::new();
        preflight.mark_completed(Some("claude-code".to_string()), None);

        let params = json!({"name": "ags_solution_phase", "arguments": {"user_request": "test"}});
        let req = make_request("prompts/get", Some(params));
        let resp = handle_prompts_get(&req, &preflight);
        assert!(
            is_success(&resp),
            "ags_solution_phase must be allowed after preflight"
        );
    }

    // ── resources/read boundaries ───────────────────────────────────────

    #[test]
    fn resources_read_always_allowed() {
        let req = make_request(
            "resources/read",
            Some(json!({"uri": "ags://global-kernel"})),
        );
        let preflight = PreflightState::new();
        let resp = handle_resources_read(&req, &preflight);
        assert!(is_success(&resp), "resources/read must always succeed");
    }

    #[test]
    fn current_host_catalog_requires_preflight() {
        let req = make_request(
            "resources/read",
            Some(json!({"uri": tools::CURRENT_HOST_CAPABILITIES_URI})),
        );
        let resp = handle_resources_read(&req, &PreflightState::new());
        assert!(has_error(&resp));
        assert!(error_contains(&resp, "Initialization Gate"));
    }

    // ── initialize resets preflight state ───────────────────────────────

    #[test]
    fn initialize_resets_preflight_state() {
        let mut initialized = false;
        let mut preflight = PreflightState::new();
        preflight.mark_completed(Some("codex".to_string()), Some("/tmp/x".to_string()));

        let req = make_request(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1.0"}
            })),
        );
        let resp = handle_initialize(&req, &mut initialized, &mut preflight);

        assert!(is_success(&resp), "initialize must succeed");
        assert!(initialized, "initialized flag must be set");
        assert!(
            !preflight.preflight_completed,
            "preflight state must be reset on initialize"
        );
        assert!(
            preflight.preflight_agent.is_none(),
            "preflight agent must be cleared on initialize"
        );
        assert!(
            preflight.preflight_target.is_none(),
            "preflight target must be cleared on initialize"
        );
    }

    // ── route_request is bound to preflight context ─────────────────────

    #[test]
    fn route_request_uses_preflight_agent_and_target() {
        let mut preflight = PreflightState::new();
        let pf_params = json!({
            "name": "ags_preflight",
            "arguments": {"agent": "codex", "target": suite_root()}
        });
        let pf_req = make_request("tools/call", Some(pf_params));
        let pf_resp = handle_tools_call(&pf_req, &mut preflight);
        assert!(is_success(&pf_resp), "preflight must succeed");
        assert_eq!(preflight.preflight_agent.as_deref(), Some("codex"));
        assert!(preflight.preflight_target.is_some());

        let sc_params = json!({
            "name": "ags_route_request",
            "arguments": {"proposal": {
                "schema_version": "0.3.0-host-route-proposal",
                "request_fingerprint": "sha256:req",
                "phase": "direct_response",
                "solution_state": "not_required",
                "execution_authority": "none",
                "scope_hash": "sha256:scope",
                "targets": [{"kind": "direct_response"}]
            }}
        });
        let sc_req = make_request("tools/call", Some(sc_params));
        let sc_resp = handle_tools_call(&sc_req, &mut preflight);
        assert!(
            is_success(&sc_resp),
            "route_request must succeed after preflight"
        );

        let text = sc_resp
            .result
            .as_ref()
            .and_then(|r| r.get("content"))
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .expect("route_request must return text content");
        let v: serde_json::Value = serde_json::from_str(text).expect("valid json");

        assert_eq!(v["host"], "codex");
        assert_eq!(v["target"], preflight.preflight_target.as_deref().unwrap());
        assert_eq!(v["resolved_targets"][0]["kind"], "direct_response");
    }

    #[test]
    fn route_request_rejects_explicit_binding_override() {
        let mut preflight = PreflightState::new();
        preflight.mark_completed(Some("codex".to_string()), Some(suite_root()));

        let sc_params = json!({
            "name": "ags_route_request",
            "arguments": {
                "active_host": "claude-code",
                "proposal": {
                    "schema_version": "0.3.0-host-route-proposal",
                    "request_fingerprint": "sha256:req",
                    "phase": "direct_response",
                    "solution_state": "not_required",
                    "execution_authority": "none",
                    "scope_hash": "sha256:scope",
                    "targets": [{"kind": "direct_response"}]
                }
            }
        });
        let sc_req = make_request("tools/call", Some(sc_params));
        let sc_resp = handle_tools_call(&sc_req, &mut preflight);
        assert!(has_error(&sc_resp));
        assert!(error_contains(&sc_resp, "preflight_binding_conflict"));
    }
}
