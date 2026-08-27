//! `ags-mcp` — the Thin AGS MCP adapter (contract v3).
//!
//! Exactly two tools: `ags_decide` (submit one typed operation; read-only
//! operations return results, sealed operations return a single-use
//! action_ref) and `ags_apply` (consume one action_ref once). The adapter
//! owns stdio and routing only; every decision lives in `ags-kernel`.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut workspace_flag: Option<PathBuf> = None;
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace" => {
                i += 1;
                workspace_flag = args.get(i).map(PathBuf::from);
            }
            other => {
                if other == "daemon" {
                    // child mode: roots come from the environment
                    if let Ok(list) = std::env::var("AGS_MCP_ROOTS") {
                        roots = list
                            .split(':')
                            .filter(|p| !p.is_empty())
                            .map(PathBuf::from)
                            .collect();
                    }
                }
            }
        }
        i += 1;
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})
                );
                continue;
            }
        };
        if let Some(response) = handle(&request, &workspace_flag, &roots) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

fn handle(request: &Value, workspace_flag: &Option<PathBuf>, roots: &[PathBuf]) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let respond = |result: Value| -> Option<Value> {
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    };
    let error = |code: i64, message: &str| -> Option<Value> {
        Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    };
    match method {
        "initialize" => respond(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "ags", "version": env!("AGS_PRODUCT_VERSION"), "build": env!("AGS_BUILD_ID")},
        })),
        "ping" => respond(json!({})),
        "notifications/initialized"
        | "notifications/cancelled"
        | "notifications/roots/list_changed" => None,
        "tools/list" => respond(json!({
            "tools": [
                tool("ags_decide", "Submit one typed Operation and receive a result or sealed action_ref.", json!({
                    "type": "object",
                    "required": ["operation", "request"],
                    "properties": {
                        "operation": {"type": "string"},
                        "request": {"type": "object"},
                    },
                })),
                tool("ags_apply", "Consume one connection-bound action_ref exactly once. Workspace resolves like ags_decide: explicit request.workspace wins, then the --workspace flag.", json!({
                    "type": "object",
                    "required": ["action_ref"],
                    "properties": {
                        "action_ref": {"type": "string"},
                        "workspace": {"type": "string"},
                    },
                })),
            ]
        })),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            match name {
                "ags_decide" => {
                    let operation = arguments
                        .get("operation")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let request = arguments.get("request").cloned().unwrap_or(json!({}));
                    match decide(operation, &request, workspace_flag, roots) {
                        Ok(result) => respond(json!({
                            "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default()}],
                            "isError": false,
                        })),
                        Err(e) => respond(json!({
                            "content": [{"type": "text", "text": json!({"error": {"code": e.code, "message": e.message}}).to_string()}],
                            "isError": true,
                        })),
                    }
                }
                "ags_apply" => {
                    let action_ref = arguments
                        .get("action_ref")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match apply(action_ref, &arguments, workspace_flag, roots) {
                        Ok(result) => respond(json!({
                            "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default()}],
                            "isError": false,
                        })),
                        Err(e) => respond(json!({
                            "content": [{"type": "text", "text": json!({"error": {"code": e.code, "message": e.message}}).to_string()}],
                            "isError": true,
                        })),
                    }
                }
                _ => error(-32602, &format!("unknown tool `{name}`")),
            }
        }
        _ => error(-32601, &format!("unknown method `{method}`")),
    }
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

fn resolve_binding(
    request: &Value,
    workspace_flag: &Option<PathBuf>,
    roots: &[PathBuf],
) -> ags_kernel::Result<ags_kernel::workspace::WorkspaceBinding> {
    // Canonical v3 form: `request.workspace` at the top level. The contract-v2
    // `request.context.workspace` form is also accepted so hosts that cached
    // the old schema keep working (drift-proofing, see sync-on-update).
    let explicit = request
        .get("workspace")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| {
            request
                .get("context")
                .and_then(|c| c.get("workspace"))
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
        });
    let cwd = request
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let explicit = explicit.or_else(|| workspace_flag.clone());
    ags_kernel::workspace::resolve(explicit.as_deref(), roots, &cwd)
}

fn decide(
    operation: &str,
    request: &Value,
    workspace_flag: &Option<PathBuf>,
    roots: &[PathBuf],
) -> ags_kernel::Result<Value> {
    match operation {
        "route" => {
            // Deterministic skill matcher (read-only): unique best hit wins,
            // ties are rejected; verified reflects machine-lock integrity.
            let input = request
                .get("input")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ags_kernel::Error::new("route_input_missing", "route requires input")
                })?
                .to_string();
            let view = ags_kernel::route::load_route_view()?;
            let result = ags_kernel::route::match_route(&view, &input);
            Ok(json!({
                "operation": "route",
                "kind": "read-only",
                "result": {
                    "input": input,
                    "skill": result.skill,
                    "candidate": result.candidate,
                    "ambiguous": result.ambiguous,
                    "hits": result.hits,
                    "verified": result.verified,
                },
            }))
        }
        "skill.list" => {
            let skills = ags_kernel::skills::list_installed()?;
            Ok(json!({
                "operation": operation,
                "kind": "read-only",
                "result": { "count": skills.len(), "skills": skills },
            }))
        }
        "skill.recommend" => {
            let query = request.get("query").and_then(|v| v.as_str());
            let skills = ags_kernel::skills::recommendations(query)?;
            Ok(json!({
                "operation": operation,
                "kind": "read-only",
                "result": { "query": query, "count": skills.len(), "skills": skills },
            }))
        }
        "doctor" => {
            let binding = resolve_binding(request, workspace_flag, roots)?;
            let config = ags_kernel::config::Config::load(&binding.root)?;
            let lint = config.lint();
            let hosts = ags_kernel::hosts::hook_health(&binding.root, &config.hosts);
            let routes = ags_kernel::capabilities::CapabilitiesLock::load(&binding)?
                .check_routes(&binding.root);
            let capability_audit_clean = routes.iter().all(|r| r.status == "exact");
            let evidence_read =
                ags_kernel::evidence::EvidenceLog::new(binding.evidence_dir.clone()).read_all();
            let chain_ok = evidence_read
                .as_ref()
                .map(|events| ags_kernel::evidence::EvidenceLog::verify_chain(events).is_ok())
                .unwrap_or(false);
            let evidence_error = evidence_read.err().map(|e| e.message);
            let install_ok = ags_kernel::sync::install_info().is_ok();
            let (entry_drift, drift_error) = match ags_kernel::sync::drift_report() {
                Ok(d) => (d, None),
                Err(e) => (Vec::new(), Some(e.message)),
            };
            let (bodies_drift, bodies_error) = match ags_kernel::sync::bodies_drift() {
                Ok(d) => (d, None),
                Err(e) => (Vec::new(), Some(e.message)),
            };
            let (git_projection_drift, git_projection_error) =
                match ags_kernel::git_projection::drift(&binding.root) {
                    Ok(d) => (d, None),
                    Err(e) => (Vec::new(), Some(e.message)),
                };
            let healthy = lint.is_empty()
                && !hosts.is_empty()
                && hosts.iter().all(|h| h.wired)
                && chain_ok
                && install_ok
                && drift_error.is_none()
                && bodies_error.is_none()
                && git_projection_error.is_none()
                && entry_drift.is_empty()
                && bodies_drift.is_empty()
                && git_projection_drift.is_empty();
            let experience =
                ags_kernel::host_projection::experience_status(&binding.root, &config)?;
            let experience_healthy = experience
                .get("healthy")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let git_projection_repair = (!git_projection_drift.is_empty()
                || git_projection_error.is_some())
            .then(|| format!("ags update --workspace {:?}", binding.root));
            Ok(json!({
                "operation": operation,
                "kind": "read-only",
                "result": {
                    "version": env!("AGS_PRODUCT_VERSION"),
                    "build": env!("AGS_BUILD_ID"),
                    "workspace": binding.slug,
                    "healthy": healthy,
                    "core_healthy": healthy,
                    "experience_healthy": experience_healthy,
                    "experience": experience,
                    "install_ok": install_ok,
                    "hosts_configured": !hosts.is_empty(),
                    "lint_findings": lint,
                    "hosts": hosts,
                    "capability_routes": routes,
                    "capability_audit_clean": capability_audit_clean,
                    "evidence_chain_ok": chain_ok,
                    "evidence_error": evidence_error,
                    "entry_drift": entry_drift,
                    "drift_error": drift_error,
                    "third_party_bodies_drift": bodies_drift,
                    "bodies_drift_error": bodies_error,
                    "git_projection_drift": git_projection_drift,
                    "git_projection_error": git_projection_error,
                    "git_projection_repair": git_projection_repair,
                },
            }))
        }
        "check" | "log" | "status" | "schema" => {
            let binding = resolve_binding(request, workspace_flag, roots)?;
            let config = ags_kernel::config::Config::load(&binding.root)?;
            let lint = config.lint();
            let routes = ags_kernel::capabilities::CapabilitiesLock::load(&binding)?
                .check_routes(&binding.root);
            let capability_audit_clean = routes.iter().all(|r| r.status == "exact");
            let chain_ok = ags_kernel::evidence::EvidenceLog::verify_chain(
                &ags_kernel::evidence::EvidenceLog::new(binding.evidence_dir.clone())
                    .read_all()
                    .unwrap_or_default(),
            )
            .is_ok();
            Ok(json!({
                "operation": operation,
                "kind": "read-only",
                "result": {
                    "workspace": binding.slug,
                    "lint_findings": lint,
                    "capability_routes": routes,
                    "capability_audit_clean": capability_audit_clean,
                    "evidence_chain_ok": chain_ok,
                },
            }))
        }
        other => {
            let binding = resolve_binding(request, workspace_flag, roots)?;
            let config = ags_kernel::config::Config::load(&binding.root)?;
            if other == "govern.host.register"
                || ags_kernel::matrix::evaluate_op(&config, other)
                    == ags_kernel::matrix::Decision::Sealed
            {
                let store = ags_kernel::seal::SealStore::new(&binding);
                let mut payload = request.clone();
                if other == "govern.host.register" {
                    let object = payload.as_object_mut().ok_or_else(|| {
                        ags_kernel::Error::new(
                            "host_register_request_invalid",
                            "host registration request must be an object",
                        )
                    })?;
                    object.insert("surface".to_string(), serde_json::json!("mcp"));
                }
                if other == "govern.skill.install" {
                    let id = payload
                        .get("skill_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            ags_kernel::Error::new(
                                "skill_install_id_missing",
                                "request requires skill_id",
                            )
                        })?;
                    let path = payload.get("path").and_then(Value::as_str).ok_or_else(|| {
                        ags_kernel::Error::new(
                            "skill_install_path_missing",
                            "request requires path",
                        )
                    })?;
                    let acknowledgements: Vec<String> = payload
                        .get("acknowledged_risks")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    payload = ags_kernel::skill_adoption::prepare_install(
                        &binding,
                        id,
                        path,
                        &acknowledgements,
                    )?;
                    if payload.get("ready").and_then(Value::as_bool) != Some(true) {
                        return Ok(json!({
                            "operation": other,
                            "kind": "transaction",
                            "state": "blocked",
                            "result": payload,
                        }));
                    }
                }
                let action = store.seal_plan(other, &payload, &binding)?;
                Ok(json!({
                    "operation": other,
                    "kind": "transaction",
                    "state": "planned",
                    "action_ref": action.token,
                    "plan_hash": action.plan_hash,
                }))
            } else {
                Err(ags_kernel::Error::new(
                    "operation_unknown",
                    format!("`{other}` is not in the sealed registry; read-only operations: doctor/check/log/status/schema/route/skill.list/skill.recommend"),
                ))
            }
        }
    }
}

fn apply(
    action_ref: &str,
    arguments: &Value,
    workspace_flag: &Option<PathBuf>,
    roots: &[PathBuf],
) -> ags_kernel::Result<Value> {
    // Same resolution order as decide: explicit request.workspace, then the
    // --workspace flag — so a multi-root connection can apply in the exact
    // binding the plan was sealed for.
    let binding = resolve_binding(arguments, workspace_flag, roots)?;
    let store = ags_kernel::seal::SealStore::new(&binding);
    let root = binding.root.clone();
    let real = ags_kernel::workspace::bind(&root);
    let receipt =
        store.apply_with_result(action_ref, &binding, |plan| match plan.operation.as_str() {
            "init" => ags_kernel::effects::init_effect(&root, &plan.payload).map(Into::into),
            other => {
                let binding = real.as_ref().map_err(|e| e.clone())?;
                ags_kernel::effects::run(other, &plan.payload, binding)
            }
        })?;
    let mut output = json!({
        "operation": receipt.operation,
        "state": receipt.state,
        "receipt_id": receipt.receipt_id,
        "observed_write_set": receipt.observed_write_set,
    });
    if let Some(result) = receipt.result {
        output["result"] = result;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_host_registration_derives_mcp_surface() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"govern.skill.install\", \"govern.skill.remove\", \"govern.host_projection\", \"govern.delegation.issue\", \"update\"]\n",
        )
        .unwrap();
        let request = json!({
            "workspace": root,
            "id": "future-host",
            "dispatch": true,
        });
        let result = decide(
            "govern.host.register",
            &request,
            &None,
            std::slice::from_ref(&root),
        )
        .unwrap();
        let token = result["action_ref"].as_str().unwrap();
        let binding = ags_kernel::workspace::bind(&root).unwrap();
        let plan = ags_kernel::seal::SealStore::new(&binding)
            .load_plan(token)
            .unwrap();
        assert_eq!(plan.payload["surface"], "mcp");
    }
}
