use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(binary: &Path, cwd: &Path, runtime: &Path, host: &str, roots: &[PathBuf]) -> Self {
        let mut child = Command::new(binary)
            .arg("stdio")
            .current_dir(cwd)
            .env("AGS_RUNTIME_HOME", runtime)
            .env("AGS_WORKSPACE_IDLE_MS", "200")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let initialized = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"roots": {"listChanged": true}},
                "clientInfo": {"name": host, "version": "1.0"}
            }),
        );
        assert_eq!(initialized["result"]["serverInfo"]["version"], "0.4.20");
        client.notify("notifications/initialized", json!({}));
        let roots_request = client.read_message();
        assert_eq!(roots_request["method"], "roots/list", "{roots_request}");
        client.respond(
            roots_request["id"].clone(),
            json!({
                "roots": roots
                    .iter()
                    .map(|path| json!({"uri": format!("file://{}", path.display())}))
                    .collect::<Vec<_>>()
            }),
        );
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        serde_json::to_writer(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
        self.read_message()
    }

    fn notify(&mut self, method: &str, params: Value) {
        serde_json::to_writer(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
        .unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn respond(&mut self, id: Value, result: Value) {
        serde_json::to_writer(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        )
        .unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read_message(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "adapter closed before responding");
        serde_json::from_str(&line).unwrap()
    }

    fn decide(&mut self, workspace: Option<&Path>, mut operation: Value) -> Value {
        if let Some(workspace) = workspace {
            operation["request"]["context"]["workspace"] = json!(workspace);
        }
        tool_value(self.request(
            "tools/call",
            json!({"name": "ags_decide", "arguments": {"operation": operation}}),
        ))
    }

    fn apply(&mut self, action_ref: &str) -> Value {
        self.request(
            "tools/call",
            json!({
                "name": "ags_apply",
                "arguments": {"action_ref": action_ref}
            }),
        )
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_value(response: Value) -> Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    response["result"]["structuredContent"].clone()
}

fn tool_error(response: Value) -> String {
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["isError"], true, "{response}");
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

fn workspace(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    std::fs::create_dir_all(path.join("config")).unwrap();
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&path)
        .status()
        .unwrap()
        .success());
    std::fs::write(path.join("AGENTS.md"), "# governed\n").unwrap();
    std::fs::write(
        path.join("config/agent-project-profile.yaml"),
        "schema_version: ags://schema/contract/v2/project-profile\n",
    )
    .unwrap();
    path.canonicalize().unwrap()
}

fn doctor() -> Value {
    json!({"operation": "doctor", "request": {"context": {}, "scope": "all"}})
}

fn setup() -> Value {
    json!({"operation": "setup", "request": {"context": {}, "approved_hosts": []}})
}

fn host_test() -> Value {
    json!({
        "operation": "test",
        "request": {"context": {}, "profile": "smoke", "executor": "host"}
    })
}

fn agent_register(host_id: &str, surface: &str) -> Value {
    json!({
        "operation": "agent.register",
        "request": {
            "context": {},
            "host_id": host_id,
            "surface": surface
        }
    })
}

fn capability_snapshot(host_id: &str) -> Value {
    json!({
        "operation": "govern.capability.snapshot",
        "request": {"context": {}, "host_id": host_id, "replace_all": false}
    })
}

fn skill_install(
    skill_id: &str,
    source: &Path,
    routing_metadata: &Path,
    target_host: &str,
) -> Value {
    json!({
        "operation": "govern.skill.install",
        "request": {
            "context": {},
            "skill_id": skill_id,
            "source": {
                "kind": "local",
                "uri": source,
                "requested_ref": null,
                "tracking_ref": null,
                "subdir": null
            },
            "routing_metadata": routing_metadata,
            "target_hosts": [target_host],
            "update_policy": "notify",
            "risk_acknowledgements": ["catalog_unreviewed"]
        }
    })
}

fn wait_for_registry(runtime: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::fs::read_dir(runtime.join("workspace-services"))
            .ok()
            .is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
            })
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn daemon_home_cwd_and_global_connection_route_without_binding_drift() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("runtime");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let a = workspace(temp.path(), "a");
    let b = workspace(temp.path(), "b");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ags-mcp"));

    let mut daemon_a = Command::new(&binary)
        .args(["daemon", "--workspace"])
        .arg(&a)
        .current_dir(&home)
        .env("AGS_RUNTIME_HOME", &runtime)
        .env("AGS_WORKSPACE_IDLE_MS", "200")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if !wait_for_registry(&runtime) {
        let status = daemon_a.try_wait().unwrap();
        if status.is_none() {
            let _ = daemon_a.kill();
            let _ = daemon_a.wait();
        }
        let mut stderr = String::new();
        daemon_a
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!("workspace daemon registry was not published; status={status:?}; stderr={stderr}");
    }

    let mut project_client =
        McpClient::spawn(&binary, &a, &runtime, "Hermes", std::slice::from_ref(&a));
    let omitted = project_client.decide(None, doctor());
    assert_eq!(omitted["state"], "no-change");
    let explicit_relative = project_client.decide(Some(Path::new(".")), doctor());
    assert_eq!(explicit_relative["state"], "no-change");
    let mut explicit_b = doctor();
    explicit_b["request"]["context"]["workspace"] = json!(&b);
    let explicit_b = tool_value(project_client.request(
        "tools/call",
        json!({"name": "ags_decide", "arguments": {"operation": explicit_b}}),
    ));
    assert_eq!(explicit_b["state"], "no-change");
    assert_eq!(explicit_b["result"]["canonical_workspace"], json!(b));

    let mut global = McpClient::spawn(&binary, &a, &runtime, "Hermes", &[a.clone(), b.clone()]);
    for target in [&a, &b, &a] {
        let decision = global.decide(Some(target), doctor());
        assert_eq!(decision["state"], "no-change");
    }

    let planned = global.decide(Some(&a), setup());
    assert_eq!(planned["state"], "planned");
    let action_ref = planned["action_ref"].as_str().unwrap();

    let mut other_workspace =
        McpClient::spawn(&binary, &b, &runtime, "Hermes", std::slice::from_ref(&b));
    assert!(tool_error(other_workspace.apply(action_ref)).contains("action_ref_invalid"));

    let mut other_connection =
        McpClient::spawn(&binary, &a, &runtime, "Hermes", std::slice::from_ref(&a));
    assert!(tool_error(other_connection.apply(action_ref)).contains("action_ref_invalid"));

    let mut other_host = McpClient::spawn(&binary, &a, &runtime, "Codex", std::slice::from_ref(&a));
    assert!(tool_error(other_host.apply(action_ref)).contains("action_ref_invalid"));

    let applied = tool_value(global.apply(action_ref));
    assert_eq!(applied["state"], "receipted");
    assert!(tool_error(global.apply(action_ref)).contains("action_ref_invalid"));

    drop(other_host);
    drop(other_workspace);
    drop(other_connection);
    drop(global);
    drop(project_client);
    let _ = daemon_a.wait();
}

#[test]
fn daemon_rejects_apply_after_governance_facts_change() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("runtime");
    let a = workspace(temp.path(), "a");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ags-mcp"));
    let mut client = McpClient::spawn(&binary, &a, &runtime, "Hermes", std::slice::from_ref(&a));

    let planned = client.decide(Some(&a), setup());
    assert_eq!(planned["state"], "planned");
    let action_ref = planned["action_ref"].as_str().unwrap();
    std::fs::write(
        a.join("AGENTS.md"),
        "# governed facts changed after decide\n",
    )
    .unwrap();

    assert!(tool_error(client.apply(action_ref)).contains("workspace_binding_stale"));
}

#[test]
fn host_delegated_grant_exposes_one_bounded_typed_execution_instruction() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("runtime");
    let a = workspace(temp.path(), "a");
    std::fs::write(
        a.join("config/agent-project-profile.yaml"),
        "schema_version: ags://schema/contract/v2/project-profile\nverification:\n  project_tests:\n    smoke: { program: /usr/bin/true, argv: [], cwd: ., env: {}, timeout_ms: 5000, allowed_write_paths: [] }\n    standard: { program: /usr/bin/true, argv: [], cwd: ., env: {}, timeout_ms: 5000, allowed_write_paths: [] }\n    full: { program: /usr/bin/true, argv: [], cwd: ., env: {}, timeout_ms: 5000, allowed_write_paths: [] }\n",
    )
    .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ags-mcp"));
    let mut client = McpClient::spawn(&binary, &a, &runtime, "Hermes", std::slice::from_ref(&a));

    let planned = client.decide(Some(&a), host_test());
    assert_eq!(planned["state"], "planned", "{planned}");
    assert_eq!(planned["kind"], "host-delegated", "{planned}");
    let action_ref = planned["action_ref"].as_str().unwrap();
    let grant = tool_value(client.apply(action_ref));
    assert_eq!(grant["state"], "awaiting-outcome", "{grant}");
    assert!(grant["outcome_token"].as_str().is_some(), "{grant}");
    let details_uri = grant["details_uri"].as_str().unwrap().to_string();
    assert!(grant["byte_length"].as_u64().is_some(), "{grant}");
    let resources = client.request("resources/list", json!({}));
    assert_eq!(resources["result"]["resources"][0]["uri"], details_uri);
    let resource = client.request("resources/read", json!({"uri": details_uri}));
    assert!(resource.get("error").is_none(), "{resource}");
    let text = resource["result"]["contents"][0]["text"].as_str().unwrap();
    assert_eq!(text.len() as u64, grant["byte_length"].as_u64().unwrap());
    let instruction: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        instruction["schema_version"],
        "ags://schema/contract/v2/host-execution-instruction"
    );
    assert_eq!(instruction["action_ref"], action_ref);
    for required in [
        "binding_hash",
        "plan_hash",
        "policy_hash",
        "instruction_digest",
    ] {
        assert!(
            instruction[required].as_str().is_some(),
            "{required}: {instruction}"
        );
    }
    assert_eq!(instruction["action"]["kind"], "command");
    for required in [
        "program",
        "argv",
        "cwd",
        "env",
        "timeout_ms",
        "allowed_write_paths",
    ] {
        assert!(
            !instruction["action"][required].is_null(),
            "{required}: {instruction}"
        );
    }
    let encoded = serde_json::to_string(&instruction).unwrap();
    for forbidden in ["natural_language", "prompt", "shell_command"] {
        assert!(!encoded.contains(forbidden), "{forbidden}: {encoded}");
    }
}

#[test]
fn generic_hermes_registration_is_an_ags_owned_transaction_not_host_delegation() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("runtime");
    let a = workspace(temp.path(), "a");
    std::fs::create_dir_all(a.join("manifests")).unwrap();
    std::fs::write(a.join("manifests/skills-registry.yaml"), "skills: []\n").unwrap();
    std::fs::write(
        a.join("manifests/mcp-registry.yaml"),
        "suite_interfaces: []\n",
    )
    .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ags-mcp"));

    // The authenticated executor and the registration subject are deliberately
    // distinct identities. Generic host admission must never be inferred from
    // the adapter executable name or an official-host allowlist.
    let mut client = McpClient::spawn(
        &binary,
        &a,
        &runtime,
        "Codex Executor",
        std::slice::from_ref(&a),
    );
    let planned = client.decide(Some(&a), agent_register("Hermes Agent.v2", "hybrid"));
    assert_eq!(planned["state"], "planned", "{planned}");
    assert_eq!(planned["kind"], "transaction", "{planned}");
    assert!(planned["plan"].is_object(), "{planned}");
    assert!(planned.get("outcome_token").is_none(), "{planned}");

    let applied = tool_value(client.apply(planned["action_ref"].as_str().unwrap()));
    assert_eq!(applied["state"], "receipted", "{applied}");

    let registration_path = runtime.join("hosts/hermes-agent-v2/registration.json");
    let registration: Value =
        serde_json::from_slice(&std::fs::read(&registration_path).unwrap()).unwrap();
    assert_eq!(
        registration["schema_version"],
        "ags://schema/contract/v2/host-registration"
    );
    assert_eq!(registration["host_id"], "hermes-agent-v2");
    assert_eq!(registration["surface"], "hybrid");
    assert_eq!(registration["contract_version"], "2");
    assert_eq!(
        registration["governed_operations"],
        json!(["ags_decide", "ags_apply"])
    );
    assert!(registration["official_adapter"].is_null());
    let registration_hash = registration["registration_hash"].as_str().unwrap();
    assert_eq!(registration_hash.len(), 71);
    assert!(registration_hash.starts_with("sha256:"));

    let base_snapshot = client.decide(Some(&a), capability_snapshot("hermes-agent-v2"));
    assert_eq!(base_snapshot["state"], "planned", "{base_snapshot}");
    let base_snapshot = tool_value(client.apply(base_snapshot["action_ref"].as_str().unwrap()));
    assert_eq!(base_snapshot["state"], "receipted", "{base_snapshot}");

    let source = a.join("fixture-skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: hermes-fixture\ndescription: Generic Hermes route fixture.\n---\n",
    )
    .unwrap();
    std::fs::write(source.join("LICENSE"), "MIT License\n").unwrap();
    let routing = a.join("fixture-routing.yaml");
    std::fs::write(
        &routing,
        "summary: Verify Generic Hermes activation.\nintent_tags: [hermes-fixture]\npositive_examples: [Use the Hermes fixture]\nnegative_examples: [Do unrelated work]\n",
    )
    .unwrap();
    let install = client.decide(
        Some(&a),
        skill_install("hermes-fixture", &source, &routing, "hermes-agent-v2"),
    );
    assert_eq!(install["state"], "planned", "{install}");
    let installed = tool_value(client.apply(install["action_ref"].as_str().unwrap()));
    assert_eq!(installed["state"], "receipted", "{installed}");

    let snapshot_path = runtime.join("stable-capabilities/snapshots/hermes-agent-v2.json");
    let snapshot: Value = serde_json::from_slice(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(
        snapshot["schema_version"],
        "ags://schema/contract/v2/host-capability-snapshot"
    );
    assert_eq!(snapshot["host"], "hermes-agent-v2");
    assert_eq!(snapshot["surface"], "hybrid");
    for required in [
        "host_registration_hash",
        "runtime_observation_hash",
        "installed_skill_index_hash",
        "input_set_hash",
        "snapshot_hash",
    ] {
        let digest = snapshot[required].as_str().unwrap();
        assert_eq!(digest.len(), 71, "{required}");
        assert!(digest.starts_with("sha256:"), "{required}");
    }
    let active = snapshot["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|skill| skill["skill_id"] == "hermes-fixture")
        .unwrap();
    assert!(active["body_ref"]["body_revision"].as_str().is_some());
    assert_eq!(active["body_ref"]["source_digest"], active["source_hash"]);
    assert!(active["body_ref"]["runtime_uri"]
        .as_str()
        .unwrap()
        .starts_with("ags://runtime/skills/hermes-fixture/"));
}
