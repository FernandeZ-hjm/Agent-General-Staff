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
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, PromptsCapability, ResourcesCapability,
    ServerCapabilities, ServerInfo, ToolsCapability, MCP_VERSION, SERVER_NAME, SERVER_VERSION,
};
use crate::{prompts, resources, tools};
use ags_session::WorkspaceState;

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
    runtime_process: RuntimeProcessIdentity,
}

impl PreflightState {
    #[cfg(test)]
    fn new() -> Self {
        Self::with_runtime_process(RuntimeProcessIdentity::capture())
    }

    #[cfg(test)]
    fn with_runtime_process(runtime_process: RuntimeProcessIdentity) -> Self {
        Self {
            governance: ags_session::WorkspaceClientSession::standalone(host_home()),
            runtime_process,
        }
    }

    fn for_workspace(
        workspace: Arc<WorkspaceState>,
        session_id: String,
        runtime_process: RuntimeProcessIdentity,
    ) -> Self {
        Self {
            governance: ags_session::WorkspaceClientSession::new(
                workspace,
                session_id,
                host_home(),
            ),
            runtime_process,
        }
    }

    /// Clear per-connection governance state while retaining the identity of the
    /// executable loaded when this process started. Re-initializing a stdio
    /// connection must not make an old process look like a newly loaded binary.
    fn reset_session(&mut self) {
        self.governance.reset();
    }

    /// Record a successful preflight. `agent` is the NORMALIZED agent and
    /// `target` is the RESOLVED target — both taken from the preflight result
    /// JSON, not the raw call arguments.
    fn mark_completed(
        &mut self,
        agent: Option<String>,
        target: Option<String>,
        capability: Option<ags_session::CapabilityReference>,
    ) {
        self.governance.mark_completed(agent, target, capability);
    }

    fn mark_bootstrap_required(&mut self, agent: Option<String>, target: Option<String>) {
        self.governance.mark_bootstrap_required(agent, target);
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

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProcessIdentity {
    executable: Option<PathBuf>,
    started_hash: Option<String>,
    observation: Arc<Mutex<RuntimeExecutableObservation>>,
    #[cfg(test)]
    full_hash_reads: Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Debug)]
struct RuntimeExecutableObservation {
    /// File identity whose bytes produced `started_hash`.
    ///
    /// On Unix this includes inode/device plus content-relevant timestamps.
    /// Other platforms deliberately leave this unset and retain the conservative
    /// full-hash check on every governed request.
    fingerprint: Option<ExecutableFingerprint>,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutableFingerprint {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Debug, Clone)]
struct RuntimeProcessStaleEvidence {
    executable: PathBuf,
    started_hash: String,
    installed_hash: String,
}

impl RuntimeProcessIdentity {
    pub(crate) fn capture() -> Self {
        let executable = std::env::current_exe().ok();
        let (started_hash, fingerprint) = executable
            .as_deref()
            .map(capture_executable)
            .unwrap_or((None, None));
        Self {
            executable,
            started_hash,
            observation: Arc::new(Mutex::new(RuntimeExecutableObservation { fingerprint })),
            #[cfg(test)]
            full_hash_reads: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    #[cfg(test)]
    fn capture_from_path(executable: PathBuf) -> Self {
        let (started_hash, fingerprint) = capture_executable(&executable);
        Self {
            executable: Some(executable),
            started_hash,
            observation: Arc::new(Mutex::new(RuntimeExecutableObservation { fingerprint })),
            full_hash_reads: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn stale_evidence(&self) -> Option<RuntimeProcessStaleEvidence> {
        let executable = self.executable.clone()?;
        let Some(started_hash) = self.started_hash.clone() else {
            return Some(RuntimeProcessStaleEvidence {
                executable,
                started_hash: "unavailable:startup-hash-read-failed".to_string(),
                installed_hash: "unverified".to_string(),
            });
        };
        let before = match executable_fingerprint(&executable) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return Some(RuntimeProcessStaleEvidence {
                    executable,
                    started_hash,
                    installed_hash: format!("unavailable:{error}"),
                });
            }
        };
        let mut observation = self
            .observation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if before.is_some() && before == observation.fingerprint {
            return None;
        }

        #[cfg(test)]
        self.full_hash_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let installed_hash = match executable_sha256(&executable) {
            Some(hash) => hash,
            None => {
                return Some(RuntimeProcessStaleEvidence {
                    executable,
                    started_hash,
                    installed_hash: "unavailable:hash-read-failed".to_string(),
                });
            }
        };
        let after = executable_fingerprint(&executable).ok().flatten();
        if before != after {
            return Some(RuntimeProcessStaleEvidence {
                executable,
                started_hash,
                installed_hash: format!("unstable:{installed_hash}"),
            });
        }
        if installed_hash == started_hash {
            observation.fingerprint = after;
            return None;
        }
        Some(RuntimeProcessStaleEvidence {
            executable,
            started_hash,
            installed_hash,
        })
    }

    #[cfg(test)]
    fn full_hash_reads(&self) -> u64 {
        self.full_hash_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn capture_executable(path: &Path) -> (Option<String>, Option<ExecutableFingerprint>) {
    let before = executable_fingerprint(path).ok().flatten();
    let hash = executable_sha256(path);
    let after = executable_fingerprint(path).ok().flatten();
    let fingerprint = (before == after).then_some(after).flatten();
    (hash, fingerprint)
}

#[cfg(unix)]
fn executable_fingerprint(path: &Path) -> std::io::Result<Option<ExecutableFingerprint>> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)?;
    Ok(Some(ExecutableFingerprint {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }))
}

#[cfg(not(unix))]
fn executable_fingerprint(path: &Path) -> std::io::Result<Option<ExecutableFingerprint>> {
    // No std API exposes a portable file identity strong enough to prove that
    // a path still names the bytes hashed at daemon start. Keep the existing
    // full-hash-per-call behavior on these platforms.
    let _ = std::fs::metadata(path)?;
    Ok(None)
}

#[cfg(not(unix))]
type ExecutableFingerprint = ();

fn executable_sha256(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(bytes);
    Some(format!("sha256:{digest:x}"))
}

fn attach_runtime_process_stale(result: &str, evidence: &RuntimeProcessStaleEvidence) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(result) else {
        return result.to_string();
    };
    let Some(object) = value.as_object_mut() else {
        return result.to_string();
    };

    object.insert(
        "runtime_process".to_string(),
        serde_json::json!({
            "status": "stale",
            "executable": evidence.executable,
            "started_hash": evidence.started_hash,
            "installed_hash": evidence.installed_hash,
            "restart_required": true
        }),
    );

    let capability = object
        .entry("capability_catalog".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let Some(capability) = capability.as_object_mut() {
        capability.insert(
            "status".to_string(),
            serde_json::json!("runtime_process_stale"),
        );
        capability.insert("refresh_required".to_string(), serde_json::json!(false));
        capability.insert("requires_host_restart".to_string(), serde_json::json!(true));
        capability.remove("refresh");
    }

    let already_stopped = object
        .get("overall_status")
        .and_then(|status| status.as_str())
        .is_some_and(|status| status == "stop");
    if !already_stopped {
        object.insert("overall_status".to_string(), serde_json::json!("warning"));
        object.insert(
            "governance_status".to_string(),
            serde_json::json!("NEEDS_USER_DECISION"),
        );
    }

    let warning = "AGS workspace daemon is stale after an executable upgrade; the host may still reply directly outside AGS, but all AGS-governed routing, actions, and resources are blocked until the host reconnects through the stdio adapter.";
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
        step.as_str().is_none_or(|text| {
            !text.contains("All clear")
                && !text.contains("may execute tasks")
                && !text.contains("Capability snapshot refresh")
                && !text.contains("capability_catalog.refresh.argv")
        })
    });
    let mut next_steps = vec![
        serde_json::json!(
            "⚠ Reconnect the AGS MCP stdio adapter so it stops the old workspace daemon before starting the installed executable."
        ),
        serde_json::json!(
            "  Do not refresh the capability snapshot for this condition; rerun ags_preflight after reconnecting."
        ),
    ];
    next_steps.append(&mut existing_steps);
    object.insert(
        "next_steps".to_string(),
        serde_json::Value::Array(next_steps),
    );

    serde_json::to_string_pretty(&value).unwrap_or_else(|_| result.to_string())
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
    let runtime_process_current = value
        .get("runtime_process")
        .and_then(|runtime| runtime.get("status"))
        .and_then(|status| status.as_str())
        .is_none_or(|status| status != "stale");

    exit_code_ok && !should_stop && failures_empty && runtime_process_current
}

fn is_runtime_process_stale_result(result: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .and_then(|value| {
            value
                .get("runtime_process")
                .and_then(|runtime| runtime.get("status"))
                .and_then(|status| status.as_str())
                .map(str::to_string)
        })
        .is_some_and(|status| status == "stale")
}

fn runtime_process_stale_error(
    id: Option<serde_json::Value>,
    preflight: &PreflightState,
) -> Option<JsonRpcResponse> {
    let evidence = preflight.runtime_process.stale_evidence()?;
    Some(JsonRpcResponse::error(
        id,
        -32001,
        &format!(
            "runtime_process_stale: AGS executable changed from {} to {} at {}; reconnect through the stdio adapter before using governed tools or resources",
            evidence.started_hash,
            evidence.installed_hash,
            evidence.executable.display()
        ),
    ))
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
    runtime_process: RuntimeProcessIdentity,
) {
    let mut initialized = false;
    let mut preflight = PreflightState::for_workspace(workspace, session_id, runtime_process);

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
    // Reset connection governance state while preserving the executable
    // identity captured when this process started.
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

    // Check the loaded executable before any governed adapter can invalidate a
    // decision generation, mint a lease, read a bound resource, or cross the
    // apply consumption point. Discovery and host-instruction bootstrap remain
    // available; preflight retains its structured stale report.
    if tool_name != tools::TOOL_PREFLIGHT && tool_name != tools::TOOL_AGENT_INSTRUCTIONS {
        if let Some(response) = runtime_process_stale_error(req.id.clone(), preflight) {
            return response;
        }
    }

    // Every preflight attempt invalidates actions from the preceding binding,
    // even when the new preflight ultimately reports a stop condition.
    if tools::is_preflight_tool_name(tool_name) {
        preflight.governance.invalidate_actions();
    }

    let binding = preflight.binding();
    let workspace = Arc::clone(preflight.governance.workspace());
    let capability_source: Option<&dyn tools::CapabilityCatalogSource> =
        workspace.is_daemon_owned().then_some(workspace.as_ref());

    match tools::call_tool(
        tool_name,
        &arguments,
        binding.as_ref(),
        preflight.governance.action_store_mut(),
        capability_source,
    ) {
        Ok(result) => {
            let result = if tools::is_preflight_tool_name(tool_name) {
                if let Some(evidence) = preflight.runtime_process.stale_evidence() {
                    preflight.reset_session();
                    attach_runtime_process_stale(&result, &evidence)
                } else {
                    result
                }
            } else {
                result
            };
            let result = if tools::is_preflight_tool_name(tool_name) {
                attach_workspace_service(&result, preflight)
            } else {
                result
            };

            // Mark preflight as completed only when the preflight report itself
            // is clean. A successful JSON-RPC tool call may still report
            // overall_status=Stop / exit_code=1 for an ungoverned target.
            if tools::is_preflight_tool_name(tool_name) && is_runtime_process_stale_result(&result)
            {
                // The process cannot safely establish any governed binding
                // after its on-disk executable changes.
            } else if tools::is_preflight_tool_name(tool_name)
                && is_successful_preflight_result(&result)
            {
                // Use the NORMALIZED agent + RESOLVED target from the preflight
                // result JSON, not the raw call arguments.
                let (agent, target, capability) = preflight_context_from_result(&result);
                preflight.mark_completed(agent, target, capability);
                log_error(&format!(
                    "preflight completed for agent: {} target: {}",
                    preflight.governance.preflight_agent().unwrap_or("unknown"),
                    preflight.governance.preflight_target().unwrap_or("unknown"),
                ));
            } else if tools::is_preflight_tool_name(tool_name)
                && is_bootstrap_required_preflight_result(&result)
            {
                let (agent, target, _) = preflight_context_from_result(&result);
                preflight.mark_bootstrap_required(agent, target);
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
        if let Some(response) = runtime_process_stale_error(req.id.clone(), preflight) {
            return response;
        }
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
    if PHASE_GATED_PROMPTS.contains(&prompt_name) {
        if let Some(response) = runtime_process_stale_error(req.id.clone(), preflight) {
            return response;
        }
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
mod tests;
