use super::*;
use crate::protocol::JsonRpcRequest;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct RuntimeFixture {
    path: PathBuf,
}

impl RuntimeFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ags-mcp-runtime-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::write(&path, b"runtime-v1").expect("runtime fixture should be writable");
        Self { path }
    }

    fn identity(&self) -> RuntimeProcessIdentity {
        RuntimeProcessIdentity::capture_from_path(self.path.clone())
    }

    fn replace(&self) {
        fs::write(&self.path, b"runtime-v2").expect("runtime fixture should be replaceable");
    }

    fn remove(&self) {
        fs::remove_file(&self.path).expect("runtime fixture should be removable");
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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

fn response_text(response: &JsonRpcResponse) -> &str {
    response
        .result
        .as_ref()
        .and_then(|result| result.get("content"))
        .and_then(|content| content.as_array())
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(|text| text.as_str())
        .expect("tool response should contain text")
}

// ── tools/call gate tests ───────────────────────────────────────────

#[test]
fn runtime_identity_detects_replaced_executable() {
    let fixture = RuntimeFixture::new();
    let identity = fixture.identity();
    for _ in 0..8 {
        assert!(
            identity.stale_evidence().is_none(),
            "unchanged executable must be current"
        );
    }
    let steady_state_hash_reads = if cfg!(unix) { 0 } else { 8 };
    assert_eq!(
        identity.full_hash_reads(),
        steady_state_hash_reads,
        "runtime identity must use the platform's configured verification strategy"
    );

    fixture.replace();
    let evidence = identity
        .stale_evidence()
        .expect("replaced executable must be detected");
    assert_eq!(identity.full_hash_reads(), steady_state_hash_reads + 1);
    assert_ne!(evidence.started_hash, evidence.installed_hash);
    assert_eq!(evidence.executable, fixture.path);
}

#[cfg(unix)]
#[test]
fn runtime_identity_rehashes_once_when_metadata_changes_without_content_drift() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = RuntimeFixture::new();
    let identity = fixture.identity();
    let mut permissions = fs::metadata(&fixture.path)
        .expect("runtime fixture metadata should be readable")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&fixture.path, permissions)
        .expect("runtime fixture permissions should be replaceable");

    assert!(
        identity.stale_evidence().is_none(),
        "metadata-only drift with identical bytes must remain current"
    );
    assert_eq!(identity.full_hash_reads(), 1);
    assert!(identity.stale_evidence().is_none());
    assert_eq!(
        identity.full_hash_reads(),
        1,
        "verified metadata identity should be cached"
    );
}

#[test]
fn runtime_identity_fails_closed_when_executable_path_disappears() {
    let fixture = RuntimeFixture::new();
    let identity = fixture.identity();
    fixture.remove();

    let evidence = identity
        .stale_evidence()
        .expect("missing executable path must be treated as stale");
    assert!(evidence.installed_hash.starts_with("unavailable:"));
}

#[test]
fn stale_runtime_preflight_requires_reconnect_and_keeps_gate_closed() {
    let fixture = RuntimeFixture::new();
    let mut preflight = PreflightState::with_runtime_process(fixture.identity());
    fixture.replace();

    let params = json!({
        "name": "ags_preflight",
        "arguments": {"agent": "codex", "target": suite_root()}
    });
    let req = make_request("tools/call", Some(params));
    let response = handle_tools_call(&req, &mut preflight);
    assert!(is_success(&response), "preflight should return a report");

    let report: serde_json::Value =
        serde_json::from_str(response_text(&response)).expect("preflight report should be JSON");
    assert_eq!(report["runtime_process"]["status"], "stale");
    assert_eq!(report["runtime_process"]["restart_required"], true);
    assert_eq!(
        report["capability_catalog"]["status"],
        "runtime_process_stale"
    );
    assert_eq!(report["capability_catalog"]["refresh_required"], false);
    assert_eq!(report["capability_catalog"]["requires_host_restart"], true);
    assert!(
        report["capability_catalog"].get("refresh").is_none(),
        "runtime staleness must not suggest snapshot refresh"
    );
    assert!(
        report["next_steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step
                .as_str()
                .is_some_and(|step| step.contains("Do not refresh")))),
        "next steps must distinguish restart from snapshot refresh"
    );
    assert!(
        !preflight.governance.is_preflight_completed()
            && !preflight.governance.is_bootstrap_required(),
        "stale runtime must keep every governed binding closed"
    );

    let gated_params =
        json!({"name": "ags_route_request", "arguments": {"request": "still gated"}});
    let gated_req = make_request("tools/call", Some(gated_params));
    let gated_response = handle_tools_call(&gated_req, &mut preflight);
    assert!(has_error(&gated_response));
    assert!(error_contains(&gated_response, "Initialization Gate"));
}

#[test]
fn replaced_runtime_blocks_route_before_it_mutates_decision_state() {
    let fixture = RuntimeFixture::new();
    let mut preflight = PreflightState::with_runtime_process(fixture.identity());
    preflight.mark_completed(
        Some("codex".to_string()),
        Some(suite_root()),
        Some(ags_session::CapabilityReference::SnapshotStale),
    );
    let generation = preflight.governance.action_store().generation;
    fixture.replace();

    let req = make_request(
        "tools/call",
        Some(json!({
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
        })),
    );
    let response = handle_tools_call(&req, &mut preflight);

    assert!(error_contains(&response, "runtime_process_stale"));
    assert_eq!(
        preflight.governance.action_store().generation,
        generation,
        "runtime admission must reject before route invalidates or creates leases"
    );
}

#[test]
fn replaced_runtime_blocks_apply_before_lease_lookup_or_consumption() {
    let fixture = RuntimeFixture::new();
    let mut preflight = PreflightState::with_runtime_process(fixture.identity());
    preflight.mark_completed(
        Some("codex".to_string()),
        Some(suite_root()),
        Some(ags_session::CapabilityReference::SnapshotStale),
    );
    let generation = preflight.governance.action_store().generation;
    fixture.replace();

    let req = make_request(
        "tools/call",
        Some(json!({
            "name": "ags_apply_action",
            "arguments": {"lease_id": "old", "action_id": "old"}
        })),
    );
    let response = handle_tools_call(&req, &mut preflight);

    assert!(error_contains(&response, "runtime_process_stale"));
    assert_eq!(preflight.governance.action_store().generation, generation);
}

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

    assert_eq!(names.len(), 9, "AGS MCP should expose exactly 9 tools");
    assert!(names.contains(&tools::TOOL_PREFLIGHT));
    assert!(names.contains(&tools::TOOL_ONBOARDING_PLAN));
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
        preflight.governance.is_preflight_completed(),
        "preflight state must be marked completed"
    );
    assert_eq!(
        preflight.governance.preflight_agent(),
        Some("claude-code"),
        "preflight agent must be recorded"
    );
    // Target must be recorded from the RESOLVED preflight result, not raw args.
    let recorded_target = preflight
        .governance
        .preflight_target()
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
        !preflight.governance.is_preflight_completed(),
        "failed preflight must not open the gate"
    );
    assert!(
        preflight.governance.is_bootstrap_required(),
        "unintegrated targets should receive only the restricted bootstrap binding"
    );

    let gated_params =
        json!({"name": "ags_route_request", "arguments": {"request": "after failed preflight"}});
    let gated_req = make_request("tools/call", Some(gated_params));
    let gated_resp = handle_tools_call(&gated_req, &mut preflight);
    assert!(
        has_error(&gated_resp),
        "gated tools must remain blocked after failed preflight"
    );
    assert!(error_contains(&gated_resp, "Initialization Gate"));

    let onboarding_params = json!({"name": "ags_onboarding_plan", "arguments": {}});
    let onboarding_req = make_request("tools/call", Some(onboarding_params));
    let onboarding_resp = handle_tools_call(&onboarding_req, &mut preflight);
    assert!(
        is_success(&onboarding_resp),
        "restricted bootstrap binding should allow the read-only onboarding plan"
    );
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
        !preflight.governance.is_preflight_completed(),
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
    preflight.mark_completed(Some("claude-code".to_string()), None, None);

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
    assert_eq!(preflight.governance.preflight_agent(), Some("codex"));

    // Second preflight with different agent
    let params2 = json!({
        "name": "ags_preflight",
        "arguments": {"agent": "claude-code", "target": suite_root()}
    });
    let req2 = make_request("tools/call", Some(params2));
    let resp2 = handle_tools_call(&req2, &mut preflight);
    assert!(is_success(&resp2), "repeated preflight must succeed");
    assert_eq!(
        preflight.governance.preflight_agent(),
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
    preflight.mark_completed(Some("claude-code".to_string()), None, None);

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

#[test]
fn current_host_catalog_preserves_typed_unavailable_diagnostic() {
    let mut preflight = PreflightState::new();
    preflight.mark_completed(
        Some("codex".to_string()),
        Some(suite_root()),
        Some(ags_session::CapabilityReference::Unavailable {
            diagnostic: ags_session::CapabilityDiagnostic {
                code: ags_session::CapabilityDiagnosticCode::StatePersistenceFailed,
                detail: "atomic publication failed".to_string(),
            },
        }),
    );
    let req = make_request(
        "resources/read",
        Some(json!({"uri": tools::CURRENT_HOST_CAPABILITIES_URI})),
    );

    let response = handle_resources_read(&req, &preflight);

    assert!(error_contains(
        &response,
        "capability_state_persistence_failed"
    ));
    assert!(
        !error_contains(&response, "skill_snapshot_stale"),
        "typed unavailable state must not collapse to stale"
    );
}

#[test]
fn replaced_runtime_blocks_bound_resource_before_catalog_access() {
    let fixture = RuntimeFixture::new();
    let mut preflight = PreflightState::with_runtime_process(fixture.identity());
    preflight.mark_completed(
        Some("codex".to_string()),
        Some(suite_root()),
        Some(ags_session::CapabilityReference::SnapshotStale),
    );
    fixture.replace();
    let req = make_request(
        "resources/read",
        Some(json!({"uri": tools::CURRENT_HOST_CAPABILITIES_URI})),
    );

    let response = handle_resources_read(&req, &preflight);

    assert!(error_contains(&response, "runtime_process_stale"));
    assert!(!error_contains(&response, "skill_snapshot_stale"));
}

// ── initialize resets preflight state ───────────────────────────────

#[test]
fn initialize_resets_preflight_state() {
    let mut initialized = false;
    let mut preflight = PreflightState::new();
    let started_hash = preflight.runtime_process.started_hash.clone();
    preflight.mark_completed(Some("codex".to_string()), Some("/tmp/x".to_string()), None);

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
        !preflight.governance.is_preflight_completed(),
        "preflight state must be reset on initialize"
    );
    assert!(
        preflight.governance.preflight_agent().is_none(),
        "preflight agent must be cleared on initialize"
    );
    assert!(
        preflight.governance.preflight_target().is_none(),
        "preflight target must be cleared on initialize"
    );
    assert_eq!(
        preflight.runtime_process.started_hash, started_hash,
        "initialize must preserve the process-start executable identity"
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
    assert_eq!(preflight.governance.preflight_agent(), Some("codex"));
    assert!(preflight.governance.preflight_target().is_some());

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
    assert_eq!(
        v["target"],
        preflight.governance.preflight_target().unwrap()
    );
    assert_eq!(v["resolved_targets"][0]["kind"], "direct_response");
}

#[test]
fn route_request_rejects_explicit_binding_override() {
    let mut preflight = PreflightState::new();
    preflight.mark_completed(Some("codex".to_string()), Some(suite_root()), None);

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
