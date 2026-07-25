use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HOSTS: &[&str] = &["codex", "claude-code", "cursor", "omp"];

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ags-host-mcp-e2e-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TestEnvironment {
    _root: TestDir,
    home: PathBuf,
    runtime: PathBuf,
    project_a: PathBuf,
    project_b: PathBuf,
    source_root: PathBuf,
    ags: PathBuf,
}

impl TestEnvironment {
    fn new() -> Self {
        let root = TestDir::new("runtime");
        let home = root.path().join("home");
        let runtime = root.path().join("runtime");
        let project_a = root.path().join("project-a");
        let project_b = root.path().join("project-b");
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let ags = PathBuf::from(env!("CARGO_BIN_EXE_ags"));

        for path in [&home, &runtime, &project_a, &project_b] {
            fs::create_dir_all(path).unwrap();
        }
        let environment = Self {
            _root: root,
            home,
            runtime,
            project_a,
            project_b,
            source_root,
            ags,
        };
        environment.initialize_project(&environment.project_a);
        environment.initialize_project(&environment.project_b);
        environment
    }

    fn command(&self) -> Command {
        self.command_with_executable(&self.ags)
    }

    fn command_with_executable(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("AGS_RUNTIME_HOME", &self.runtime)
            .env("AGS_SOURCE_ROOT", &self.source_root)
            .env("AGS_WORKSPACE_IDLE_MS", "1000")
            .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1");
        command
    }

    fn initialize_project(&self, project: &Path) {
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(project)
            .status()
            .unwrap();
        assert!(git.success(), "git init failed for {}", project.display());

        let output = self
            .command()
            .args(["init", "--target"])
            .arg(project)
            .args(["--mode", "local", "--format", "json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "ags init failed for {}: {}",
            project.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_snapshot(&self, host: &str) -> Value {
        let output = self
            .command()
            .args(["capability", "snapshot", "--host", host, "--target"])
            .arg(&self.project_a)
            .args(["--write", "--format", "json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "snapshot failed for {host}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let snapshot: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(snapshot["host"], host);
        assert!(
            snapshot["active_skills"].is_array(),
            "{host} snapshot omitted its active skill table"
        );
        snapshot
    }

    fn install_test_skill(&self, skill_id: &str) {
        let skill_dir = self.home.join(".agents").join("skills").join(skill_id);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {skill_id}\ndescription: Hermetic capability refresh fixture.\n---\n\n# {skill_id}\n"
            ),
        )
        .unwrap();
    }

    fn connect(&self, cwd: &Path) -> McpClient {
        self.connect_with_executable(cwd, &self.ags)
    }

    fn connect_with_executable(&self, cwd: &Path, executable: &Path) -> McpClient {
        let mut child = self
            .command_with_executable(executable)
            .args(["mcp", "serve", "--transport", "stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        McpClient {
            child,
            stdin,
            stdout,
        }
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        // Thin adapters may disconnect before their workspace daemons reach the
        // short test-only idle timeout.
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
}

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn request_envelope(&mut self, id: u64, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let mut stderr = String::new();
        if line.is_empty() {
            let _ = self.child.wait();
            if let Some(mut child_stderr) = self.child.stderr.take() {
                let _ = child_stderr.read_to_string(&mut stderr);
            }
        }
        assert!(
            !line.is_empty(),
            "MCP server closed before responding to {method}: {stderr}"
        );
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let response = self.request_envelope(id, method, params);
        assert!(
            response.get("error").is_none() || response["error"].is_null(),
            "MCP {method} failed: {response}"
        );
        response["result"].clone()
    }

    fn initialize(&mut self, host: &str) {
        let result = self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": host, "version": "e2e"}
            }),
        );
        assert_eq!(result["serverInfo"]["name"], "ags-mcp");
    }

    fn preflight(&mut self, host: &str, project: &Path) -> Value {
        let result = self.request(
            2,
            "tools/call",
            json!({
                "name": "ags_preflight",
                "arguments": {
                    "agent": host,
                    "target": project
                }
            }),
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn current_host_snapshot(&mut self) -> Value {
        let result = self.request(
            3,
            "resources/read",
            json!({"uri": "ags://capabilities/current-host"}),
        );
        let text = result["contents"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn route_project_verify(&mut self) -> Value {
        let result = self.request(
            4,
            "tools/call",
            json!({
                "name": "ags_route_request",
                "arguments": {
                    "proposal": {
                        "schema_version": "0.3.0-host-route-proposal",
                        "request_fingerprint": "sha256:e2e-request",
                        "phase": "execution",
                        "solution_state": "confirmed",
                        "execution_authority": "none",
                        "scope_hash": "sha256:e2e-scope",
                        "targets": [{
                            "kind": "machine_cli",
                            "capability": "project_verify",
                            "input": {"kind": "empty"}
                        }]
                    }
                }
            }),
        );
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn reject_foreign_lease(&mut self, lease_id: &str, action_id: &str) {
        let response = self.request_envelope(
            5,
            "tools/call",
            json!({
                "name": "ags_apply_action",
                "arguments": {
                    "lease_id": lease_id,
                    "action_id": action_id
                }
            }),
        );
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("decision_lease_invalid_or_expired")),
            "foreign DecisionLease was not rejected: {response}"
        );
    }

    #[cfg(unix)]
    fn wait_for_exit(&mut self) -> bool {
        for _ in 0..40 {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn registered_hosts_share_one_workspace_service_but_keep_sessions_and_leases_isolated() {
    let environment = TestEnvironment::new();

    let mut expected_hashes = Vec::new();
    for host in HOSTS {
        let written = environment.write_snapshot(host);
        expected_hashes.push(written["snapshot_hash"].as_str().unwrap().to_string());
    }

    let mut clients = Vec::new();
    let mut workspace_key = None;
    let mut session_ids = Vec::new();
    for (host, expected_hash) in HOSTS.iter().zip(&expected_hashes) {
        let mut client = environment.connect(&environment.project_a);
        client.initialize(host);
        let preflight = client.preflight(host, &environment.project_a);
        assert_eq!(preflight["capability_catalog"]["status"], "ready");
        assert_eq!(preflight["workspace_service"]["status"], "ready");
        let current_key = preflight["workspace_service"]["instance_key"]
            .as_str()
            .unwrap()
            .to_string();
        if let Some(expected_key) = &workspace_key {
            assert_eq!(&current_key, expected_key);
        } else {
            workspace_key = Some(current_key);
        }
        session_ids.push(
            preflight["workspace_service"]["session_id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        let snapshot = client.current_host_snapshot();
        assert_eq!(snapshot["host"], *host);
        assert_eq!(snapshot["snapshot_hash"], *expected_hash);
        clients.push(client);
    }
    session_ids.sort();
    session_ids.dedup();
    assert_eq!(session_ids.len(), HOSTS.len());
    let capability_bundle = environment.runtime.join("workspace-services").join(format!(
        "{}.capabilities.json",
        workspace_key.as_ref().unwrap()
    ));
    let bundle: Value = serde_json::from_slice(&fs::read(capability_bundle).unwrap()).unwrap();
    assert_eq!(
        Path::new(bundle["workspace"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        environment.project_a.canonicalize().unwrap()
    );
    for host in HOSTS {
        assert!(
            bundle["snapshots"].get(*host).is_some(),
            "workspace capability bundle omitted {host}"
        );
    }
    let service_registry = environment
        .runtime
        .join("workspace-services")
        .join(format!("{}.json", workspace_key.as_ref().unwrap()));
    let shared_pid = serde_json::from_slice::<Value>(&fs::read(&service_registry).unwrap())
        .unwrap()["pid"]
        .as_u64()
        .unwrap();

    let route = clients[0].route_project_verify();
    let lease_id = route["lease"]["lease_id"].as_str().unwrap();
    let action_id = route["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap();
    clients[1].reject_foreign_lease(lease_id, action_id);
    drop(clients);

    let mut reconnected = environment.connect(&environment.project_a);
    reconnected.initialize("codex");
    let same_workspace = reconnected.preflight("codex", &environment.project_a);
    assert_eq!(
        same_workspace["workspace_service"]["instance_key"],
        workspace_key.unwrap()
    );
    let reconnect_pid = serde_json::from_slice::<Value>(&fs::read(&service_registry).unwrap())
        .unwrap()["pid"]
        .as_u64()
        .unwrap();
    assert_eq!(
        reconnect_pid, shared_pid,
        "client disconnect unexpectedly killed the workspace daemon"
    );
    drop(reconnected);
    std::thread::sleep(std::time::Duration::from_millis(1300));

    let mut recycled = environment.connect(&environment.project_a);
    recycled.initialize("codex");
    let recycled_preflight = recycled.preflight("codex", &environment.project_a);
    assert_eq!(
        recycled_preflight["workspace_service"]["instance_key"],
        same_workspace["workspace_service"]["instance_key"]
    );
    let recycled_pid = serde_json::from_slice::<Value>(&fs::read(&service_registry).unwrap())
        .unwrap()["pid"]
        .as_u64()
        .unwrap();
    assert_ne!(
        recycled_pid, shared_pid,
        "idle workspace daemon was not recycled"
    );

    let mut other_workspace = environment.connect(&environment.project_b);
    other_workspace.initialize("codex");
    let preflight = other_workspace.preflight("codex", &environment.project_b);
    assert_ne!(
        preflight["workspace_service"]["instance_key"],
        same_workspace["workspace_service"]["instance_key"]
    );
    assert_eq!(
        Path::new(preflight["target"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        environment.project_b.canonicalize().unwrap()
    );
    let snapshot = other_workspace.current_host_snapshot();
    assert_eq!(snapshot["host"], "codex");
    assert_eq!(snapshot["snapshot_hash"], expected_hashes[0]);
}

#[test]
fn refreshed_snapshot_rebinds_the_existing_workspace_session() {
    let environment = TestEnvironment::new();
    let initial = environment.write_snapshot("codex");

    let mut client = environment.connect(&environment.project_a);
    client.initialize("codex");
    let first_preflight = client.preflight("codex", &environment.project_a);
    assert_eq!(first_preflight["capability_catalog"]["status"], "ready");
    assert_eq!(
        first_preflight["capability_catalog"]["snapshot_hash"],
        initial["snapshot_hash"]
    );

    environment.install_test_skill("e2e-refresh-skill");
    let refreshed = environment.write_snapshot("codex");
    assert_ne!(
        refreshed["snapshot_hash"], initial["snapshot_hash"],
        "capability mutation did not produce a new snapshot"
    );

    let rebound = client.preflight("codex", &environment.project_a);
    assert_eq!(rebound["capability_catalog"]["status"], "ready");
    assert_eq!(
        rebound["capability_catalog"]["snapshot_hash"],
        refreshed["snapshot_hash"]
    );
    let current = client.current_host_snapshot();
    assert_eq!(current["snapshot_hash"], refreshed["snapshot_hash"]);
    assert!(
        current["catalog"].as_array().is_some_and(|cards| cards
            .iter()
            .any(|card| card["skill_id"].as_str() == Some("e2e-refresh-skill"))),
        "refreshed current-host catalog omitted the new skill"
    );
}

#[cfg(unix)]
#[test]
fn crashed_workspace_daemon_is_replaced_without_reusing_its_session() {
    let environment = TestEnvironment::new();
    environment.write_snapshot("codex");

    let mut client = environment.connect(&environment.project_a);
    client.initialize("codex");
    let first = client.preflight("codex", &environment.project_a);
    let instance_key = first["workspace_service"]["instance_key"].as_str().unwrap();
    let first_session = first["workspace_service"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let registry_path = environment
        .runtime
        .join("workspace-services")
        .join(format!("{instance_key}.json"));
    let first_pid = serde_json::from_slice::<Value>(&fs::read(&registry_path).unwrap()).unwrap()
        ["pid"]
        .as_u64()
        .unwrap();

    assert!(Command::new("kill")
        .args(["-TERM", &first_pid.to_string()])
        .status()
        .unwrap()
        .success());
    for _ in 0..40 {
        if !Command::new("kill")
            .args(["-0", &first_pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(client);

    let mut recovered = environment.connect(&environment.project_a);
    recovered.initialize("codex");
    let second = recovered.preflight("codex", &environment.project_a);
    assert_eq!(
        second["workspace_service"]["instance_key"],
        first["workspace_service"]["instance_key"]
    );
    assert_ne!(
        second["workspace_service"]["session_id"].as_str().unwrap(),
        first_session
    );
    let second_pid = serde_json::from_slice::<Value>(&fs::read(&registry_path).unwrap()).unwrap()
        ["pid"]
        .as_u64()
        .unwrap();
    assert_ne!(second_pid, first_pid);
}

#[cfg(unix)]
#[test]
fn executable_upgrade_stops_the_old_workspace_daemon_before_restart() {
    use std::fs::OpenOptions;

    let environment = TestEnvironment::new();
    environment.write_snapshot("codex");

    let live_executable = environment._root.path().join("ags-live");
    fs::copy(&environment.ags, &live_executable).unwrap();

    let mut old_connection =
        environment.connect_with_executable(&environment.project_a, &live_executable);
    old_connection.initialize("codex");
    let ready = old_connection.preflight("codex", &environment.project_a);
    assert_eq!(ready["capability_catalog"]["status"], "ready");
    let instance_key = ready["workspace_service"]["instance_key"].as_str().unwrap();
    let registry_path = environment
        .runtime
        .join("workspace-services")
        .join(format!("{instance_key}.json"));
    let old_registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    let old_pid = old_registry["pid"].as_u64().unwrap();
    let old_hash = old_registry["executable_hash"]
        .as_str()
        .unwrap()
        .to_string();

    let replacement = environment._root.path().join("ags-replacement");
    fs::copy(&environment.ags, &replacement).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&replacement)
        .unwrap()
        .write_all(b"\0")
        .unwrap();
    fs::rename(&replacement, &live_executable).unwrap();

    let mut reconnected =
        environment.connect_with_executable(&environment.project_a, &live_executable);
    reconnected.initialize("codex");
    let recovered = reconnected.preflight("codex", &environment.project_a);
    assert_eq!(recovered["capability_catalog"]["status"], "ready");
    assert_eq!(
        recovered["workspace_service"]["instance_key"],
        ready["workspace_service"]["instance_key"]
    );
    assert_ne!(
        recovered["workspace_service"]["session_id"],
        ready["workspace_service"]["session_id"]
    );
    let new_registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    assert_ne!(new_registry["pid"].as_u64().unwrap(), old_pid);
    assert_ne!(new_registry["executable_hash"].as_str().unwrap(), old_hash);
    assert!(
        old_connection.wait_for_exit(),
        "old stdio adapter did not exit after its workspace daemon stopped"
    );
    assert!(
        !Command::new("kill")
            .args(["-0", &old_pid.to_string()])
            .status()
            .is_ok_and(|status| status.success()),
        "old workspace daemon still exists after the new daemon became ready"
    );
    drop(old_connection);
}
