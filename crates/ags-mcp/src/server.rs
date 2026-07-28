//! Daemon-side JSON-RPC session loop for AGS MCP.
//!
//! Reads JSON-RPC messages from one workspace-daemon client stream, dispatches
//! to tool/resource/prompt handlers, and writes responses to that stream.
//! The separate stdio adapter only proxies bytes.
//!
//! # Initialization Gate (Hard Enforcement)
//!
//! After MCP `initialize`, the server tracks per-connection preflight state.
//! All `tools/call` requests (except `ags_preflight` itself) and phase-gated
//! `prompts/get` requests are blocked until `ags_preflight` completes.
//! `tools/list`, static protocol resources, and `prompts/list` are always
//! allowed. The current-host capability resource is read-only but remains
//! preflight-bound because it represents one specific host/target pair.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, PromptsCapability, ResourcesCapability,
    ServerCapabilities, ServerInfo, ToolsCapability, MCP_VERSION, SERVER_NAME, SERVER_VERSION,
};
use crate::{prompts, resources, tools};
use ags_session::WorkspaceState;

const RUNTIME_IDENTITY_ERROR: &str =
    "mcp_runtime_identity_stale_reconnect_required: AGS executable content changed";

#[derive(Debug)]
struct RuntimeIdentity {
    executable: PathBuf,
    startup_hash: String,
}

impl RuntimeIdentity {
    fn current(startup_hash: String) -> Result<Self, String> {
        let executable =
            std::env::current_exe().map_err(|error| format!("current_exe failed: {error}"))?;
        Ok(Self {
            executable,
            startup_hash,
        })
    }

    #[cfg(test)]
    fn from_path(executable: PathBuf) -> Result<Self, String> {
        let startup_hash = ags_platform::executable_content_hash(&executable)?;
        Ok(Self {
            executable,
            startup_hash,
        })
    }

    /// Deliberately hash the complete file for every governed request. No
    /// inode/size/timestamp shortcut is permitted on this security boundary.
    fn verify(&self) -> Result<(), String> {
        let current_hash = ags_platform::executable_content_hash(&self.executable)?;
        if current_hash == self.startup_hash {
            Ok(())
        } else {
            Err(RUNTIME_IDENTITY_ERROR.to_string())
        }
    }
}

// ── Preflight State ─────────────────────────────────────────────────────────

/// Per-connection preflight state for the AGS Initialization Gate.
///
/// After MCP `initialize`, the server requires `ags_preflight` (MCP tool)
/// or CLI fallback before any other governed tool or phase-gated prompt.
/// State is scoped to one daemon client session and is destroyed when that
/// client disconnects. The workspace daemon itself may outlive every client.
#[derive(Debug)]
struct PreflightState {
    governance: ags_session::WorkspaceClientSession<tools::HeldAction>,
}

impl PreflightState {
    fn for_workspace(workspace: Arc<WorkspaceState>, session_id: String) -> Self {
        Self {
            governance: ags_session::WorkspaceClientSession::new(
                workspace,
                session_id,
                host_home(),
            ),
        }
    }

    /// Clear per-connection governance state.
    fn reset_session(&mut self) {
        self.governance.reset();
    }

    /// Record a successful preflight. `agent` is the NORMALIZED agent and
    /// `target` is the RESOLVED target — both taken from the preflight result
    /// JSON, not the raw call arguments.
    fn mark_completed(
        &mut self,
        agent: String,
        target: String,
        capability: Option<ags_session::CapabilityReference>,
    ) {
        self.governance.bind_ready(agent, target, capability);
    }

    fn mark_bootstrap_required(&mut self, agent: String, target: String) {
        self.governance.bind_bootstrap_required(agent, target);
    }

    fn binding(&self) -> Option<tools::PreflightBinding> {
        self.governance.binding()
    }
}

fn host_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
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
fn preflight_context_from_result(
    result: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<ags_session::CapabilityReference>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(result) else {
        return (None, None, None);
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
    let capability = value.get("capability_catalog").and_then(|catalog| {
        match catalog.get("status").and_then(|status| status.as_str())? {
            "ready" => Some(ags_session::CapabilityReference::Ready {
                binding: ags_session::CapabilityBinding {
                    workspace_identity: catalog.get("workspace_identity")?.as_str()?.to_string(),
                    snapshot_hash: catalog.get("snapshot_hash")?.as_str()?.to_string(),
                },
            }),
            "snapshot_stale" => Some(ags_session::CapabilityReference::SnapshotStale),
            "capability_unavailable" => {
                let error = catalog.get("error")?;
                let code = match error.get("code")?.as_str()? {
                    "capability_snapshot_read_failed" => {
                        ags_session::CapabilityDiagnosticCode::SnapshotReadFailed
                    }
                    "capability_snapshot_corrupt" => {
                        ags_session::CapabilityDiagnosticCode::SnapshotCorrupt
                    }
                    "capability_snapshot_integrity_failed" => {
                        ags_session::CapabilityDiagnosticCode::SnapshotIntegrityFailed
                    }
                    "capability_snapshot_invalid" => {
                        ags_session::CapabilityDiagnosticCode::SnapshotInvalid
                    }
                    "capability_workspace_target_invalid" => {
                        ags_session::CapabilityDiagnosticCode::WorkspaceTargetInvalid
                    }
                    "capability_state_lock_unavailable" => {
                        ags_session::CapabilityDiagnosticCode::StateLockUnavailable
                    }
                    "capability_source_unavailable" => {
                        ags_session::CapabilityDiagnosticCode::SourceUnavailable
                    }
                    _ => return None,
                };
                Some(ags_session::CapabilityReference::Unavailable {
                    diagnostic: ags_session::CapabilityDiagnostic {
                        code,
                        detail: error.get("detail")?.as_str()?.to_string(),
                    },
                })
            }
            _ => None,
        }
    });
    (agent, target, capability)
}

fn is_bootstrap_required_preflight_result(result: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .and_then(|value| {
            value
                .get("integration_status")
                .and_then(|status| status.as_str())
                .map(str::to_string)
        })
        .is_some_and(|status| status == "not_integrated")
}

fn attach_workspace_service(result: &str, preflight: &PreflightState) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(result) else {
        return result.to_string();
    };
    let Some(object) = value.as_object_mut() else {
        return result.to_string();
    };
    let requested_target = object
        .get("target")
        .and_then(|target| target.as_str())
        .unwrap_or("<missing>")
        .to_string();
    let target_matches = requested_target != "<missing>"
        && preflight
            .governance
            .workspace()
            .target_matches(Path::new(&requested_target));
    object.insert(
        "workspace_service".to_string(),
        serde_json::json!({
            "status": if target_matches { "ready" } else { "target_mismatch" },
            "workspace": preflight.governance.workspace().root(),
            "instance_key": preflight.governance.workspace().instance_key(),
            "session_id": preflight.governance.session_id(),
        }),
    );
    if !target_matches {
        object.insert("overall_status".to_string(), serde_json::json!("stop"));
        object.insert("should_stop".to_string(), serde_json::json!(true));
        object.insert("exit_code".to_string(), serde_json::json!(1));
        let failure = format!(
            "workspace_target_mismatch: service={} requested={requested_target}",
            preflight.governance.workspace().root().display()
        );
        let failures = object
            .entry("failures".to_string())
            .or_insert_with(|| serde_json::json!([]));
        if let Some(failures) = failures.as_array_mut() {
            failures.push(serde_json::json!(failure));
        }
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| result.to_string())
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

/// Run one MCP session over a daemon-owned reader/writer pair.
///
/// Workspace state is shared across clients, while initialization, preflight
/// binding and DecisionLease storage remain session-local.
pub(crate) fn run_mcp_session<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    workspace: Arc<WorkspaceState>,
    session_id: String,
    startup_executable_hash: String,
) {
    let mut initialized = false;
    let mut preflight = PreflightState::for_workspace(workspace, session_id);
    let runtime_identity = RuntimeIdentity::current(startup_executable_hash);

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

                let response = if request_requires_runtime_identity(&req)
                    && runtime_identity
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(RuntimeIdentity::verify)
                        .is_err()
                {
                    preflight.reset_session();
                    JsonRpcResponse::error(req.id.clone(), -32001, RUNTIME_IDENTITY_ERROR)
                } else if !initialized && req.method != "initialize" {
                    JsonRpcResponse::error(
                        req.id,
                        -32002,
                        "Not initialized — send initialize request first",
                    )
                } else {
                    dispatch_request(&req, &mut initialized, &mut preflight)
                };
                write_response(&mut writer, &response);
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
                        write_response(&mut writer, &err);
                    }
                }
            }
        }
    }
}

fn request_requires_runtime_identity(req: &JsonRpcRequest) -> bool {
    match req.method.as_str() {
        "tools/call" => true,
        "resources/read" => {
            req.params
                .as_ref()
                .and_then(|params| params.get("uri"))
                .and_then(|uri| uri.as_str())
                == Some(tools::CURRENT_HOST_CAPABILITIES_URI)
        }
        "prompts/get" => req
            .params
            .as_ref()
            .and_then(|params| params.get("name"))
            .and_then(|name| name.as_str())
            .is_some_and(|name| PHASE_GATED_PROMPTS.contains(&name)),
        _ => false,
    }
}

#[cfg(test)]
mod runtime_identity_tests {
    use super::*;

    #[test]
    fn equal_length_fast_replacement_is_detected_by_full_content_hash() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("ags");
        std::fs::write(&executable, b"aaaa").unwrap();
        let identity = RuntimeIdentity::from_path(executable.clone()).unwrap();
        std::fs::write(executable, b"bbbb").unwrap();
        assert_eq!(identity.verify().unwrap_err(), RUNTIME_IDENTITY_ERROR);
    }

    #[test]
    fn only_governed_requests_require_runtime_identity_verification() {
        let request = |method: &str, params: Option<serde_json::Value>| JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: method.to_string(),
            params,
        };

        assert!(!request_requires_runtime_identity(&request(
            "initialize",
            None
        )));
        assert!(!request_requires_runtime_identity(&request(
            "resources/list",
            None
        )));
        assert!(request_requires_runtime_identity(&request(
            "tools/call",
            Some(serde_json::json!({"name": tools::TOOL_PREFLIGHT}))
        )));
        assert!(request_requires_runtime_identity(&request(
            "resources/read",
            Some(serde_json::json!({"uri": tools::CURRENT_HOST_CAPABILITIES_URI}))
        )));
        assert!(request_requires_runtime_identity(&request(
            "prompts/get",
            Some(serde_json::json!({"name": PHASE_GATED_PROMPTS[0]}))
        )));
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
    // Reset connection governance state.
    preflight.reset_session();

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
    let allowed_in_bootstrap = preflight.governance.is_bootstrap_required()
        && tools::is_onboarding_bootstrap_tool_name(tool_name);
    if !tools::is_preflight_bootstrap_tool_name(tool_name)
        && !preflight.governance.is_preflight_completed()
        && !allowed_in_bootstrap
    {
        return JsonRpcResponse::error(req.id.clone(), -32000, PREFLIGHT_GATE_ERROR);
    }

    // Every preflight attempt invalidates actions from the preceding binding,
    // even when the new preflight ultimately reports a stop condition.
    if tools::is_preflight_tool_name(tool_name) {
        preflight.governance.invalidate_actions();
    }

    let binding = preflight.binding();
    let workspace = Arc::clone(preflight.governance.workspace());

    match tools::call_tool(
        tool_name,
        &arguments,
        binding.as_ref(),
        preflight.governance.action_store_mut(),
        workspace.as_ref(),
    ) {
        Ok(result) => {
            let result = if tools::is_preflight_tool_name(tool_name) {
                attach_workspace_service(&result, preflight)
            } else {
                result
            };

            // Mark preflight as completed only when the preflight report itself
            // is clean. A successful JSON-RPC tool call may still report
            // overall_status=Stop / exit_code=1 for an ungoverned target.
            if tools::is_preflight_tool_name(tool_name) && is_successful_preflight_result(&result) {
                // Use the NORMALIZED agent + RESOLVED target from the preflight
                // result JSON, not the raw call arguments.
                let (agent, target, capability) = preflight_context_from_result(&result);
                if let (Some(agent), Some(target)) = (agent, target) {
                    preflight.mark_completed(agent, target, capability);
                    log_error(&format!(
                        "preflight completed for agent: {} target: {}",
                        preflight.governance.preflight_agent().unwrap_or("unknown"),
                        preflight.governance.preflight_target().unwrap_or("unknown"),
                    ));
                } else {
                    preflight.reset_session();
                    log_error("preflight result omitted normalized binding");
                }
            } else if tools::is_preflight_tool_name(tool_name)
                && is_bootstrap_required_preflight_result(&result)
            {
                let (agent, target, _) = preflight_context_from_result(&result);
                if let (Some(agent), Some(target)) = (agent, target) {
                    preflight.mark_bootstrap_required(agent, target);
                } else {
                    preflight.reset_session();
                }
                if preflight.governance.is_bootstrap_required() {
                    log_error("preflight established restricted bootstrap_required binding");
                }
            }

            if tool_name == tools::TOOL_APPLY_ACTION
                && serde_json::from_str::<serde_json::Value>(&result)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("requires_repreflight")
                            .and_then(|flag| flag.as_bool())
                    })
                    .unwrap_or(false)
            {
                preflight.reset_session();
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
        return match preflight.governance.workspace().read_catalog(&binding) {
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
    if PHASE_GATED_PROMPTS.contains(&prompt_name) && !preflight.governance.is_preflight_completed()
    {
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

fn write_response(writer: &mut impl Write, response: &JsonRpcResponse) {
    let json = serde_json::to_string(response).unwrap_or_else(|e| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32603,"message":"Serialization error: {}"}}}}"#,
            e
        )
    });
    let _ = writeln!(writer, "{}", json);
    let _ = writer.flush();
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

    #[test]
    fn tools_call_requires_preflight() {
        let workspace = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let state = Arc::new(
            WorkspaceState::new(
                workspace.path().canonicalize().unwrap(),
                runtime.path().to_path_buf(),
            )
            .unwrap(),
        );
        let mut preflight = PreflightState::for_workspace(state, "test-session".to_string());
        let mut initialized = true;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": tools::TOOL_ROUTE_REQUEST,
                "arguments": {}
            })),
        };

        let response = dispatch_request(&request, &mut initialized, &mut preflight);
        let error = response
            .error
            .expect("preflight gate must reject tools/call");
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, PREFLIGHT_GATE_ERROR);
    }
}
