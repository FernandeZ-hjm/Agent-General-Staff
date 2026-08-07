use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HOSTS: &[&str] = &["codex", "claude-code", "cursor", "codebuddy-code", "omp"];

#[cfg(windows)]
fn copy_directory(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).unwrap();
        }
    }
}

fn shell_visible_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
    }
    #[cfg(not(windows))]
    {
        path.into_owned()
    }
}

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
        fs::write(
            runtime.join("install-manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "schema_version": "0.4.13-runtime-install",
                "producer_version": env!("CARGO_PKG_VERSION"),
                "source_root": source_root.to_string_lossy(),
                "target": runtime.to_string_lossy(),
                "lifecycle": {
                    "approved_hosts": [],
                    "selection_source": "setup"
                }
            }))
            .unwrap(),
        )
        .unwrap();
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
        #[cfg(not(windows))]
        command.env("PATH", "/usr/bin:/bin");
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

    fn write_routable_mcp_snapshot(&self, host: &str) -> Value {
        let original: ags_capability_governance::HostCapabilitySnapshot =
            serde_json::from_value(self.write_snapshot(host)).unwrap();
        let mcp = ags_capability_governance::McpCard {
            mcp_id: "context7".to_string(),
            display_name: "context7".to_string(),
            summary: "Current library documentation".to_string(),
            intent_tags: vec!["docs-lookup".to_string()],
            positive_examples: vec!["查一下这个库的最新文档".to_string()],
            negative_examples: vec!["修改当前仓库代码".to_string()],
            tools: vec![
                "get-library-docs".to_string(),
                "resolve-library-id".to_string(),
            ],
            invoke_hint: "context7 MCP".to_string(),
            route_state: "routable".to_string(),
            mutation_surface: "read_only".to_string(),
            availability: ags_capability_governance::AvailabilityState::Ready,
            reason_codes: Vec::new(),
            requires_auth: false,
            auth_state: ags_capability_governance::AuthState::NotRequired,
            health_status: "healthy".to_string(),
        };
        let active = ags_capability_governance::ActiveMcp {
            mcp_id: mcp.mcp_id.clone(),
            invoke_hint: mcp.invoke_hint.clone(),
            allowed_tools: mcp.tools.clone(),
            intent_tags: mcp.intent_tags.clone(),
            mutation_surface: mcp.mutation_surface.clone(),
        };
        let refreshed = ags_capability_governance::HostCapabilitySnapshot::new(
            original.host,
            original.registry_hash,
            original.runtime_hash,
            original.catalog,
            vec![mcp],
            original.third_party_registry_url,
            original.third_party_manifest_hash,
            original.third_party_catalog,
            original.active_skills,
            vec![active],
        )
        .unwrap();
        let path = ags_capability_governance::snapshot_path(&self.runtime, host);
        fs::write(&path, serde_json::to_vec_pretty(&refreshed).unwrap()).unwrap();
        serde_json::to_value(refreshed).unwrap()
    }

    fn install_test_skill(&self, skill_id: &str) {
        let private_canonical = self.source_root.join("global-skills").join(skill_id);
        let public_canonical = self
            .source_root
            .join("templates/command-skills")
            .join(skill_id);
        let external = !private_canonical.is_dir() && !public_canonical.is_dir();
        let canonical = if private_canonical.is_dir() {
            private_canonical
        } else if public_canonical.is_dir() {
            public_canonical
        } else {
            self.home.join(".agents/skills").join(skill_id)
        };
        if external {
            fs::create_dir_all(&canonical).unwrap();
            fs::write(
                canonical.join("SKILL.md"),
                format!(
                    "---\nname: {skill_id}\ndescription: Hermetic external capability fixture.\n---\n\n# {skill_id}\n"
                ),
            )
            .unwrap();
            let registry: serde_yaml::Value = serde_yaml::from_slice(
                &fs::read(self.source_root.join("manifests/skills-registry.yaml")).unwrap(),
            )
            .unwrap();
            for target in registry["route_targets"]
                .as_sequence()
                .into_iter()
                .flatten()
                .filter(|target| {
                    target["routing"]["parent"]["kind"].as_str() == Some("skill")
                        && target["routing"]["parent"]["name"].as_str() == Some(skill_id)
                        && target["routing"]["entrypoint"]["kind"].as_str() == Some("playbook")
                })
            {
                let entrypoint = target["routing"]["entrypoint"]["name"].as_str().unwrap();
                let playbook = canonical.join("playbooks").join(entrypoint);
                fs::create_dir_all(&playbook).unwrap();
                fs::write(
                    playbook.join("PLAYBOOK.md"),
                    format!("# Hermetic {entrypoint} fixture\n"),
                )
                .unwrap();
            }
        }
        for (host, root) in [
            ("claude-code", ".claude/skills"),
            ("codex", ".codex/skills"),
            ("cursor", ".cursor/skills"),
            ("codebuddy-code", ".codebuddy/skills"),
            ("omp", ".omp/agent/skills"),
        ] {
            if external
                && ags_host_integration::platform_spec(host)
                    .is_some_and(|spec| spec.loads_shared_agent_skills)
            {
                continue;
            }
            let skill_dir = self.home.join(root).join(skill_id);
            fs::create_dir_all(skill_dir.parent().unwrap()).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&canonical, &skill_dir).unwrap();
            #[cfg(windows)]
            copy_directory(&canonical, &skill_dir);
        }
    }

    fn install_superpowers_adapter(&self) {
        let mut plan_args = vec![
            "skill".to_string(),
            "install".to_string(),
            "ags-superpowers-adapter".to_string(),
        ];
        for host in HOSTS {
            plan_args.push("--host".to_string());
            plan_args.push((*host).to_string());
        }
        plan_args.extend(["--format".to_string(), "json".to_string()]);
        let planned = self
            .command()
            .current_dir(&self.project_a)
            .args(&plan_args)
            .output()
            .unwrap();
        assert!(
            planned.status.success(),
            "adapter plan failed: stdout={} stderr={}",
            String::from_utf8_lossy(&planned.stdout),
            String::from_utf8_lossy(&planned.stderr)
        );
        let plan: Value = serde_json::from_slice(&planned.stdout).unwrap();

        let mut apply_args = plan_args[..plan_args.len() - 2].to_vec();
        apply_args.push("--plan-hash".to_string());
        apply_args.push(plan["plan_hash"].as_str().unwrap().to_string());
        for risk in plan["required_acknowledgements"].as_array().unwrap() {
            apply_args.push("--ack-risk".to_string());
            apply_args.push(risk.as_str().unwrap().to_string());
        }
        apply_args.extend([
            "--yes".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ]);
        let applied = self
            .command()
            .current_dir(&self.project_a)
            .args(&apply_args)
            .output()
            .unwrap();
        assert!(
            applied.status.success(),
            "adapter apply failed: stdout={} stderr={}",
            String::from_utf8_lossy(&applied.stdout),
            String::from_utf8_lossy(&applied.stderr)
        );
        let receipt: Value = serde_json::from_slice(&applied.stdout).unwrap();
        assert_eq!(receipt["status"], "verified");
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
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
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
        self.route_targets(
            4,
            "sha256:e2e-request",
            "none",
            json!([{
                "kind": "machine_cli",
                "capability": "project_verify",
                "input": {"kind": "empty"}
            }]),
        )
    }

    fn route_targets(
        &mut self,
        id: u64,
        fingerprint: &str,
        execution_authority: &str,
        targets: Value,
    ) -> Value {
        let result = self.request(
            id,
            "tools/call",
            json!({
                "name": "ags_route_request",
                "arguments": {
                    "proposal": {
                        "schema_version": "0.3.6-host-route-proposal",
                        "request_fingerprint": fingerprint,
                        "phase": "execution",
                        "solution_state": "confirmed",
                        "execution_authority": execution_authority,
                        "scope_hash": "sha256:e2e-scope",
                        "targets": targets
                    }
                }
            }),
        );
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn apply(&mut self, id: u64, lease_id: &str, action_id: &str) -> Value {
        self.request(
            id,
            "tools/call",
            json!({
                "name": "ags_apply_action",
                "arguments": {
                    "lease_id": lease_id,
                    "action_id": action_id
                }
            }),
        )
    }

    fn reject_invalid_lease(&mut self, lease_id: &str, action_id: &str) {
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
            "invalid or expired DecisionLease was not rejected: {response}"
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

/// Hermetic protocol fixture used by CI. Host names select AGS adapter
/// behavior; they are not evidence that native host executables are installed
/// or registered.
#[test]
fn mcp_status_and_restart_control_the_workspace_daemon_through_the_cli_seam() {
    let environment = TestEnvironment::new();
    environment.write_snapshot("codex");

    let status_output = environment
        .command()
        .args(["mcp", "status", "--target"])
        .arg(&environment.project_a)
        .output()
        .unwrap();
    assert!(status_output.status.success());
    let stopped: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(stopped["state"], "stopped");

    let first_output = environment
        .command()
        .args(["mcp", "restart", "--target"])
        .arg(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        first_output.status.success(),
        "{}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    let first: Value = serde_json::from_slice(&first_output.stdout).unwrap();
    assert_eq!(first["state"], "running");
    assert_eq!(first["current_binary"], true);
    let first_pid = first["pid"].as_u64().unwrap();

    let second_output = environment
        .command()
        .args(["mcp", "restart", "--target"])
        .arg(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        second_output.status.success(),
        "{}",
        String::from_utf8_lossy(&second_output.stderr)
    );
    let second: Value = serde_json::from_slice(&second_output.stdout).unwrap();
    assert_eq!(second["state"], "running");
    assert_eq!(second["current_binary"], true);
    assert_ne!(second["pid"].as_u64().unwrap(), first_pid);

    let status_output = environment
        .command()
        .args(["mcp", "status", "--target"])
        .arg(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        status_output.status.success(),
        "{}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(status["state"], "running");
    assert_eq!(status["current_binary"], true);
    assert_eq!(status["pid"], second["pid"]);

    #[cfg(unix)]
    {
        let daemon_pid = i32::try_from(second["pid"].as_u64().unwrap()).unwrap();
        let daemon_session = unsafe { libc::getsid(daemon_pid) };
        let caller_session = unsafe { libc::getsid(0) };
        assert_ne!(daemon_session, -1);
        assert_ne!(caller_session, -1);
        assert_ne!(
            daemon_session, caller_session,
            "restart must detach the workspace daemon from the caller session"
        );
    }
}

#[test]
fn cursor_govern_writes_native_hooks_and_reaches_full_lifecycle() {
    let environment = TestEnvironment::new();
    environment.write_snapshot("cursor");

    let output = environment
        .command()
        .current_dir(&environment.project_a)
        .args([
            "agents", "govern", "--agent", "cursor", "--apply", "--format", "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["apply_status"], "memory-adapters-applied");

    let hooks: Value = serde_json::from_slice(
        &fs::read(environment.project_a.join(".cursor/hooks.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(hooks["version"], 1);
    let canonical_project = environment.project_a.canonicalize().unwrap();
    for (native_event, rust_event) in [
        ("sessionStart", "session-start"),
        ("sessionEnd", "session-end"),
        ("stop", "stop-guard"),
    ] {
        assert!(hooks["hooks"][native_event]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["command"]
                .as_str()
                .is_some_and(|command| command.contains(&format!(
                    "host lifecycle --event {rust_event} --host cursor"
                )) && command
                    .contains(&canonical_project.to_string_lossy().to_string()))));
    }

    let verify = environment
        .command()
        .current_dir(&environment.project_a)
        .args(["agents", "verify", "--host", "cursor", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr),
    );
    let verification: Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(
        verification["capability_visibility"]["status"],
        "incomplete"
    );
    assert_eq!(
        verification["memory_lifecycle"]["adapter"],
        "cursor-command-hooks"
    );
    assert_eq!(verification["memory_lifecycle"]["status"], "full");
}

#[test]
fn init_projects_the_approved_host_subset_and_preserves_user_hooks() {
    let environment = TestEnvironment::new();
    let workspace = environment._root.path().join("workspace with spaces");
    fs::create_dir_all(&workspace).unwrap();
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace)
        .status()
        .unwrap()
        .success());
    fs::create_dir_all(workspace.join(".claude")).unwrap();
    fs::write(
        workspace.join(".claude/settings.local.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "Notification": [{
                    "hooks": [{"type": "command", "command": "user-owned-hook"}]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        environment.runtime.join("install-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.4.13-runtime-install",
            "source_root": environment.source_root,
            "lifecycle": {
                "approved_hosts": HOSTS,
                "selection_source": "setup"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = environment
        .command()
        .args(["init", "--target"])
        .arg(&workspace)
        .args(["--mode", "local", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema_version"], "0.4.1-project-init");
    assert_eq!(
        result["lifecycle"]["projected_hosts"]
            .as_array()
            .unwrap()
            .len(),
        HOSTS.len()
    );

    for host in HOSTS {
        let path = ags_lifecycle::lifecycle_projection::workspace_adapter_path(&workspace, host)
            .expect("supported lifecycle host");
        assert!(
            path.is_file(),
            "{host} adapter missing at {}",
            path.display()
        );
    }
    let manifest = ags_lifecycle::lifecycle_projection::load_lifecycle_manifest(
        &ags_lifecycle::lifecycle_projection::lifecycle_manifest_path(&workspace),
    )
    .unwrap();
    assert_eq!(manifest.enabled_hosts.len(), HOSTS.len());

    let claude: Value =
        serde_json::from_slice(&fs::read(workspace.join(".claude/settings.local.json")).unwrap())
            .unwrap();
    assert_eq!(
        claude["hooks"]["Notification"][0]["hooks"][0]["command"],
        "user-owned-hook"
    );
    let exclude = fs::read_to_string(workspace.join(".git/info/exclude")).unwrap();
    for host in HOSTS {
        let adapter =
            ags_lifecycle::lifecycle_projection::workspace_adapter_path(&workspace, host).unwrap();
        let relative = adapter.strip_prefix(&workspace).unwrap().to_string_lossy();
        assert!(
            exclude.contains(&format!("/{relative}").replace('\\', "/")),
            "{host} adapter missing from local overlay"
        );
    }
}

#[test]
fn typed_mcp_route_resolves_to_host_native_dispatch_without_server_action() {
    let environment = TestEnvironment::new();
    let written = environment.write_routable_mcp_snapshot("codex");
    let snapshot_hash = written["snapshot_hash"].as_str().unwrap();
    let mut client = environment.connect(&environment.project_a);
    client.initialize("codex");
    let preflight = client.preflight("codex", &environment.project_a);
    assert_eq!(preflight["capability_catalog"]["status"], "ready");
    let snapshot = client.current_host_snapshot();
    assert!(snapshot["active_mcps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|mcp| mcp["mcp_id"] == "context7"));

    let route = client.route_targets(
        4,
        "sha256:mcp-route",
        "none",
        json!([{
            "kind": "mcp",
            "mcp_id": "context7",
            "tool": "get-library-docs",
            "snapshot_hash": snapshot_hash
        }]),
    );
    assert_eq!(route["governance_status"], "HOST_EXECUTION_REQUIRED");
    assert!(route["lease"].is_null());
    assert_eq!(route["resolved_targets"][0]["kind"], "mcp");
    assert_eq!(route["resolved_targets"][0]["mcp_id"], "context7");
    assert_eq!(route["resolved_targets"][0]["tool"], "get-library-docs");
    assert_eq!(
        route["resolved_targets"][0]["mutation_surface"],
        "read_only"
    );

    let rejected = client.route_targets(
        5,
        "sha256:mcp-unknown-tool",
        "none",
        json!([{
            "kind": "mcp",
            "mcp_id": "context7",
            "tool": "delete-everything",
            "snapshot_hash": snapshot_hash
        }]),
    );
    assert_eq!(rejected["governance_status"], "BLOCKED_BY_POLICY");
    assert!(rejected["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error["code"] == "mcp_selection_rejected"));
}

#[test]
fn hermetic_host_adapters_share_one_workspace_service_but_keep_sessions_and_leases_isolated() {
    let environment = TestEnvironment::new();
    environment.install_test_skill("ags-skill");
    environment.install_superpowers_adapter();

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
        assert!(snapshot["active_skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["skill_id"] == "ags-skill"));
        clients.push(client);
    }
    session_ids.sort();
    session_ids.dedup();
    assert_eq!(session_ids.len(), HOSTS.len());
    for host in HOSTS {
        assert!(
            ags_capability_governance::snapshot_path(&environment.runtime, host).is_file(),
            "static host snapshot is missing for {host}"
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

    let setup_route = clients[0].route_targets(
        4,
        "sha256:command-skill-rejection",
        "none",
        json!([{
            "kind": "skill",
            "skill_id": "ags-setup",
            "snapshot_hash": expected_hashes[0]
        }]),
    );
    assert!(setup_route["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error["code"] == "skill_target_kind_mismatch"));

    let playbook_route = clients[0].route_targets(
        5,
        "sha256:superpowers-playbook-route",
        "none",
        json!([{
            "kind": "skill",
            "skill_id": "superpowers",
            "entrypoint": "verification-before-completion",
            "snapshot_hash": expected_hashes[0]
        }]),
    );
    assert!(
        playbook_route["errors"]
            .as_array()
            .is_none_or(|errors| errors.is_empty()),
        "registered parent playbook route failed: {playbook_route}"
    );
    assert!(playbook_route["resolved_targets"]
        .as_array()
        .is_some_and(|targets| targets.iter().any(|target| {
            target["kind"] == "skill"
                && target["skill_id"] == "superpowers"
                && target["entrypoint"] == "verification-before-completion"
        })));

    let handoff = json!({
        "schema_version": "0.3.6-handoff-contract",
        "task_level": "Light",
        "task": "compile the routed E2E task card",
        "fields": {
            "目标：": "- G-01: compile the card",
            "验收标准：": "- AC-01 -> G-01: compiled",
            "Verification gate:": "- commands:\n  - V-01 -> AC-01: true"
        }
    })
    .to_string();
    let compile_route = clients[0].route_targets(
        6,
        "sha256:skill-plus-task-compile",
        "task_card_handoff",
        json!([
            {
                "kind": "skill",
                "skill_id": "ags-skill",
                "snapshot_hash": expected_hashes[0]
            },
            {
                "kind": "machine_cli",
                "capability": "task_compile",
                "input": {
                    "kind": "confirmed_handoff_contract",
                    "content": handoff,
                    "handoff_source": "explicit_handoff"
                }
            }
        ]),
    );
    assert!(
        compile_route["errors"]
            .as_array()
            .is_none_or(|errors| errors.is_empty()),
        "skill + task_compile route failed: {compile_route}"
    );
    let compile_lease = compile_route["lease"]["lease_id"].as_str().unwrap();
    let compile_action = compile_route["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap();
    let applied = clients[0].apply(7, compile_lease, compile_action);
    assert!(applied["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("0.3.6-task-contract")));

    let superseded_route = clients[0].route_project_verify();
    let superseded_lease = superseded_route["lease"]["lease_id"].as_str().unwrap();
    let superseded_action = superseded_route["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap();
    let preflight_invalidated_route = clients[0].route_project_verify();
    let preflight_invalidated_lease = preflight_invalidated_route["lease"]["lease_id"]
        .as_str()
        .unwrap();
    let preflight_invalidated_action = preflight_invalidated_route["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap();
    assert_ne!(superseded_action, preflight_invalidated_action);
    clients[0].reject_invalid_lease(superseded_lease, superseded_action);
    clients[0].preflight("codex", &environment.project_a);
    clients[0].reject_invalid_lease(preflight_invalidated_lease, preflight_invalidated_action);

    let route = clients[0].route_project_verify();
    let lease_id = route["lease"]["lease_id"].as_str().unwrap();
    let action_id = route["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap();
    clients[1].reject_invalid_lease(lease_id, action_id);
    drop(clients);

    let mut reconnected = environment.connect(&environment.project_a);
    reconnected.initialize("codex");
    let same_workspace = reconnected.preflight("codex", &environment.project_a);
    assert!(!session_ids.iter().any(|session_id| {
        same_workspace["workspace_service"]["session_id"].as_str() == Some(session_id)
    }));
    reconnected.reject_invalid_lease(lease_id, action_id);
    let workspace_a_capability_identity = same_workspace["capability_catalog"]
        ["workspace_identity"]
        .as_str()
        .unwrap()
        .to_string();
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
    assert_ne!(
        preflight["capability_catalog"]["workspace_identity"], workspace_a_capability_identity,
        "different workspaces shared one capability snapshot identity"
    );
    assert!(preflight["capability_catalog"]
        .get("bundle_epoch")
        .is_none());
    let snapshot = other_workspace.current_host_snapshot();
    assert_eq!(snapshot["host"], "codex");
    // Equal content hashes are allowed when the host capabilities are
    // identical; authority and session binding are workspace-local.
    assert_eq!(
        snapshot["snapshot_hash"],
        preflight["capability_catalog"]["snapshot_hash"]
    );
}

#[test]
fn refreshed_snapshot_is_published_only_after_daemon_restart() {
    let environment = TestEnvironment::new();
    let initial = environment.write_snapshot("codex");
    let claude_initial = environment.write_snapshot("claude-code");

    let mut client = environment.connect(&environment.project_a);
    client.initialize("codex");
    let first_preflight = client.preflight("codex", &environment.project_a);
    assert_eq!(first_preflight["capability_catalog"]["status"], "ready");
    assert_eq!(
        first_preflight["capability_catalog"]["snapshot_hash"],
        initial["snapshot_hash"]
    );
    let mut claude = environment.connect(&environment.project_a);
    claude.initialize("claude-code");
    let claude_preflight = claude.preflight("claude-code", &environment.project_a);
    assert_eq!(claude_preflight["capability_catalog"]["status"], "ready");

    environment.install_test_skill("e2e-refresh-skill");
    let refreshed = environment.write_snapshot("codex");
    assert_ne!(
        refreshed["snapshot_hash"], initial["snapshot_hash"],
        "capability mutation did not produce a new snapshot"
    );

    let still_loaded = client.current_host_snapshot();
    assert_eq!(
        still_loaded["snapshot_hash"], initial["snapshot_hash"],
        "a running daemon must keep its once-loaded host snapshot immutable"
    );

    let claude_current = claude.current_host_snapshot();
    assert_eq!(claude_current["host"], "claude-code");
    assert_eq!(
        claude_current["snapshot_hash"], claude_initial["snapshot_hash"],
        "refreshing Codex invalidated the independent Claude host generation"
    );

    drop(client);
    let mut reconnected = environment.connect(&environment.project_a);
    reconnected.initialize("codex");
    let repeated = reconnected.preflight("codex", &environment.project_a);
    assert_eq!(
        repeated["capability_catalog"]["snapshot_hash"], initial["snapshot_hash"],
        "a reconnect to the same daemon must use the already loaded snapshot"
    );
    drop(reconnected);
    drop(claude);

    let restart = environment
        .command()
        .args(["mcp", "restart", "--target"])
        .arg(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        restart.status.success(),
        "{}",
        String::from_utf8_lossy(&restart.stderr)
    );

    let mut after_restart = environment.connect(&environment.project_a);
    after_restart.initialize("codex");
    let published = after_restart.preflight("codex", &environment.project_a);
    assert_eq!(
        published["capability_catalog"]["snapshot_hash"], refreshed["snapshot_hash"],
        "daemon restart must publish the refreshed canonical snapshot"
    );
    assert!(after_restart.route_project_verify()["lease"]["lease_id"].is_string());
}

#[test]
fn direct_workspace_daemon_entrypoint_refuses_a_second_live_owner() {
    let environment = TestEnvironment::new();
    let mut first_command = environment.command();
    first_command.env("AGS_WORKSPACE_IDLE_MS", "30000");
    let mut first = first_command
        .args(["mcp", "workspace-daemon", "--workspace"])
        .arg(&environment.project_a)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let service_dir = environment.runtime.join("workspace-services");
    let canonical_project = environment.project_a.canonicalize().unwrap();
    let first_ready_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let first_registry_endpoint = service_dir.read_dir().ok().and_then(|entries| {
            entries.flatten().find_map(|entry| {
                let registry = fs::read(entry.path())
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())?;
                (registry["pid"].as_u64() == Some(u64::from(first.id()))
                    && registry["workspace"].as_str()
                        == Some(canonical_project.to_string_lossy().as_ref()))
                .then(|| registry["endpoint"].as_str().map(str::to_owned))
                .flatten()
            })
        });
        if first_registry_endpoint
            .as_deref()
            .is_some_and(|endpoint| std::net::TcpStream::connect(endpoint).is_ok())
        {
            break;
        }
        if let Some(status) = first.try_wait().unwrap() {
            let mut stderr = String::new();
            if let Some(mut pipe) = first.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            panic!("first daemon exited before publishing its registry ({status}): {stderr}");
        }
        assert!(
            Instant::now() < first_ready_deadline,
            "first daemon did not publish a reachable owned registry"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut second_command = environment.command();
    second_command.env("AGS_WORKSPACE_IDLE_MS", "30000");
    let mut second = second_command
        .args(["mcp", "workspace-daemon", "--workspace"])
        .arg(&environment.project_a)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut second_status = None;
    let second_exit_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < second_exit_deadline {
        second_status = second.try_wait().unwrap();
        if second_status.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if second_status.is_none() {
        let _ = second.kill();
    }
    let _ = second.wait();
    let mut stderr = String::new();
    if let Some(mut pipe) = second.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let first_status = first.try_wait().unwrap();
    let _ = first.kill();
    let _ = first.wait();
    let mut first_stderr = String::new();
    if let Some(mut pipe) = first.stderr.take() {
        let _ = pipe.read_to_string(&mut first_stderr);
    }

    assert!(
        second_status.is_some_and(|status| !status.success()),
        "second daemon remained alive instead of refusing the owner; first status={first_status:?}; first stderr={first_stderr}; second stderr={stderr}"
    );
    assert!(
        stderr.contains("workspace daemon already active"),
        "{stderr}"
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
fn new_connection_replaces_daemon_when_executable_hash_changes() {
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

#[test]
fn workspace_lifecycle_events_share_the_daemon_and_deduplicate_event_ids() {
    let environment = TestEnvironment::new();
    let memory_dir =
        ags_host_integration::project_memory_dir_at(&environment.project_a, &environment.home);
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("context-capsule.md"),
        "capsule-from-workspace-daemon",
    )
    .unwrap();
    fs::write(
        memory_dir.join("task-memory.md"),
        "task-memory-from-workspace-daemon",
    )
    .unwrap();

    let invoke = |host: &str, event: &str, payload: Value| {
        let mut child = environment
            .command()
            .args([
                "host",
                "lifecycle",
                "--event",
                event,
                "--host",
                host,
                "--target",
            ])
            .arg(&environment.project_a)
            .args(["--input", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };

    for lifecycle in ags_host_integration::lifecycle_specs() {
        let session_id = format!("host-session-{}", lifecycle.host_id);
        let start_event_id = format!("event-start-{}", lifecycle.host_id);
        let started = invoke(
            lifecycle.host_id,
            "session-start",
            json!({"session_id": &session_id, "event_id": start_event_id}),
        );
        assert_eq!(started["schema_version"], "0.4.0-workspace-lifecycle");
        assert_eq!(started["host"], lifecycle.host_id);
        let startup_context = match lifecycle.output {
            ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible
            | ags_host_integration::LifecycleOutputProtocol::CodeBuddy => {
                assert_eq!(
                    started["hookSpecificOutput"]["hookEventName"],
                    "SessionStart"
                );
                started["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
            }
            ags_host_integration::LifecycleOutputProtocol::Cursor => {
                assert!(started.get("additional_context").is_some());
                assert!(started.get("hookSpecificOutput").is_none());
                started["additional_context"].as_str().unwrap()
            }
        };
        assert!(startup_context.contains("capsule-from-workspace-daemon"));
        assert!(startup_context.contains("task-memory-from-workspace-daemon"));

        let stop_event_id = format!("event-stop-{}", lifecycle.host_id);
        let clear_payload = json!({"session_id": &session_id, "event_id": stop_event_id});
        let clear = invoke(lifecycle.host_id, "stop-guard", clear_payload.clone());
        let duplicate = invoke(lifecycle.host_id, "stop-guard", clear_payload);
        assert_eq!(clear["status"], "clear");
        assert!(clear.get("hookSpecificOutput").is_none());
        assert!(clear.get("followup_message").is_none());
        if lifecycle.output == ags_host_integration::LifecycleOutputProtocol::CodeBuddy {
            assert_eq!(clear["continue"], true);
            assert_eq!(clear["suppressOutput"], true);
        }
        assert_eq!(duplicate["duplicate"], true);

        let blocked = invoke(
            lifecycle.host_id,
            "stop-guard",
            json!({
                "session_id": &session_id,
                "event_id": format!("event-blocked-{}", lifecycle.host_id),
                "last_assistant_message": "<invoke tool=\"unsafe\">"
            }),
        );
        assert_eq!(blocked["status"], "blocked");
        match lifecycle.output {
            ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible => {
                assert_eq!(blocked["suppressOutput"], true);
                assert_eq!(blocked["hookSpecificOutput"]["hookEventName"], "Stop");
                assert!(blocked["hookSpecificOutput"]["additionalContext"].is_string());
            }
            ags_host_integration::LifecycleOutputProtocol::CodeBuddy => {
                assert_eq!(blocked["continue"], false);
                assert!(blocked["reason"].is_string());
                assert!(blocked.get("hookSpecificOutput").is_none());
            }
            ags_host_integration::LifecycleOutputProtocol::Cursor => {
                assert!(blocked["followup_message"].is_string());
                assert!(blocked.get("hookSpecificOutput").is_none());
            }
        }

        let end_event_id = format!("event-end-{}", lifecycle.host_id);
        let end_payload = json!({"session_id": &session_id, "event_id": end_event_id});
        let ended = invoke(lifecycle.host_id, "session-end", end_payload.clone());
        let duplicate_end = invoke(lifecycle.host_id, "session-end", end_payload);
        assert_eq!(ended["event"], "session-end");
        assert_eq!(ended["host"], lifecycle.host_id);
        assert_eq!(ended["status"], "skipped");
        assert!(ended.get("hookSpecificOutput").is_none());
        assert_eq!(duplicate_end["duplicate"], true);
    }

    let second_codex_session = invoke(
        "codex",
        "session-end",
        json!({"session_id": "codex-second-session", "event_id": "codex-second-end"}),
    );
    assert_eq!(second_codex_session["status"], "skipped");
    let already_ended = invoke(
        "codex",
        "session-end",
        json!({"session_id": "codex-second-session", "event_id": "codex-late-end"}),
    );
    assert_eq!(already_ended["status"], "already-ended");
    assert_eq!(
        fs::read_dir(environment.project_a.join(".ags/state/lifecycle"))
            .unwrap()
            .count(),
        HOSTS.len() + 1
    );

    let task_card = environment.project_a.join("lifecycle-e2e-task-card.md");
    let launch_plan = environment.project_a.join("lifecycle-e2e-launch-plan.json");
    let delivery_report = environment
        .project_a
        .join("lifecycle-e2e-delivery-report.md");
    let receipt = environment.project_a.join("lifecycle-e2e-receipt.json");
    fs::copy(
        environment.source_root.join("tests/fixtures/valid-full.md"),
        &task_card,
    )
    .unwrap();
    let launch = environment
        .command()
        .arg("run")
        .arg(&task_card)
        .args(["--current-task-approval", "--format", "json"])
        .current_dir(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        launch.status.success(),
        "launch plan failed: {}",
        String::from_utf8_lossy(&launch.stderr)
    );
    fs::write(&launch_plan, &launch.stdout).unwrap();
    let launch: Value = serde_json::from_slice(&launch.stdout).unwrap();
    let task_card_hash = ags_evidence::sha256_hex(&fs::read(&task_card).unwrap());
    let launch_plan_hash = launch["launch_plan_hash"].as_str().unwrap();
    fs::write(
        &delivery_report,
        format!(
            "# 任务交付报告\n\
             \n\
             Closure schema: 1.1\n\
             Contract ID: tc-0123456789abcdef\n\
             task-card-hash: {task_card_hash}\n\
             launch-plan-hash: {launch_plan_hash}\n\
             execution-mode-used: single-writer\n\
             execution-topology-used: single\n\
             delegation-used: none\n\
             状态: completed\n\
             review-gate: passed\n\
             \n\
             ## 目标闭环\n\
             - G-01: done — lifecycle archive E2E completed\n\
             \n\
             ## 验收闭环\n\
             - AC-01: pass — evidence: process boundary preserved closure\n\
             \n\
             ## 验证闭环\n\
             - V-01: pass — host lifecycle process E2E\n\
             \n\
             ## 未闭环项\n\
             - none\n"
        ),
    )
    .unwrap();
    let closed = environment
        .command()
        .args(["task", "close"])
        .arg(&task_card)
        .arg(&launch_plan)
        .arg(&delivery_report)
        .arg("--receipt-out")
        .arg(&receipt)
        .args(["--format", "json"])
        .current_dir(&environment.project_a)
        .output()
        .unwrap();
    assert!(
        closed.status.success(),
        "task close failed: {}",
        String::from_utf8_lossy(&closed.stderr)
    );
    let closed: Value = serde_json::from_slice(&closed.stdout).unwrap();
    assert_eq!(closed["valid"], true);
    let receipt_id = closed["receipt_id"].as_str().unwrap();
    let pointer_dir = environment.project_a.join(".ags/state/closure-pointers");
    assert_eq!(fs::read_dir(&pointer_dir).unwrap().count(), 1);
    let archive_dir = memory_dir.join("task-archive").join(receipt_id);
    assert!(!archive_dir.exists());

    let guard_before_close = invoke(
        "claude-code",
        "stop-guard",
        json!({
            "session_id": "verified-closure-session",
            "event_id": "verified-closure-stop"
        }),
    );
    assert_eq!(guard_before_close["status"], "clear");
    assert_eq!(fs::read_dir(&pointer_dir).unwrap().count(), 1);
    assert!(
        !archive_dir.exists(),
        "per-turn Stop must not archive or close the host session"
    );

    let archived = invoke(
        "claude-code",
        "session-end",
        json!({
            "session_id": "verified-closure-session",
            "event_id": "verified-closure-end"
        }),
    );
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["archive"].as_array().unwrap().len(), 1);
    assert!(archive_dir.join("task-card.md").is_file());
    assert!(archive_dir.join("launch-plan.json").is_file());
    assert!(archive_dir.join("delivery-report.md").is_file());
    assert!(archive_dir.join("receipt.json").is_file());
    assert_eq!(fs::read_dir(&pointer_dir).unwrap().count(), 0);

    let status: Value = serde_json::from_slice(
        &environment
            .command()
            .args(["mcp", "status", "--target"])
            .arg(&environment.project_a)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(status["state"], "running");
    assert_eq!(status["current_binary"], true);
}

#[test]
fn doctor_proves_target_aware_workspace_conformance_and_rejects_fixed_state_drift() {
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    let environment = TestEnvironment::new();
    let canonical_home = environment.home.canonicalize().unwrap();
    let canonical_project_a = environment.project_a.canonicalize().unwrap();
    let canonical_project_b = environment.project_b.canonicalize().unwrap();
    let setup = environment
        .command()
        .env("HOME", &canonical_home)
        .env("USERPROFILE", &canonical_home)
        .args(["setup", "--target"])
        .arg(&environment.runtime)
        .args([
            "--yes",
            "--force",
            "--lifecycle-hosts",
            "claude-code",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        setup.status.success(),
        "setup failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&setup.stdout),
        String::from_utf8_lossy(&setup.stderr)
    );

    let refreshed_init = environment
        .command()
        .env("HOME", &canonical_home)
        .env("USERPROFILE", &canonical_home)
        .env("AGS_HOME", &environment.runtime)
        .current_dir(&canonical_project_a)
        .args(["init", "--target"])
        .arg(&canonical_project_a)
        .args(["--mode", "local", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        refreshed_init.status.success(),
        "post-setup init failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&refreshed_init.stdout),
        String::from_utf8_lossy(&refreshed_init.stderr)
    );
    let refreshed_projection = environment
        .command()
        .env("HOME", &canonical_home)
        .env("USERPROFILE", &canonical_home)
        .env("AGS_HOME", &environment.runtime)
        .current_dir(&canonical_project_b)
        .args(["init", "--target"])
        .arg(&canonical_project_b)
        .args(["--mode", "local", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        refreshed_projection.status.success(),
        "project refresh failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&refreshed_projection.stdout),
        String::from_utf8_lossy(&refreshed_projection.stderr)
    );

    let fake_bin = environment._root.path().join("doctor-bin");
    let old_bin = environment._root.path().join("old-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&old_bin).unwrap();
    let executable_name = if cfg!(windows) { "ags.exe" } else { "ags" };
    let current_ags = fake_bin.join(executable_name);
    #[cfg(unix)]
    symlink(&environment.ags, &current_ags).unwrap();
    #[cfg(windows)]
    fs::copy(&environment.ags, &current_ags).unwrap();
    let old_ags = old_bin.join(executable_name);
    fs::copy(&environment.ags, &old_ags).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&old_ags)
        .unwrap()
        .write_all(b"\nold-e2e-binary")
        .unwrap();

    let fake_codegraph = fake_bin.join(if cfg!(windows) {
        "codegraph.cmd"
    } else {
        "codegraph"
    });
    fs::write(
        &fake_codegraph,
        if cfg!(windows) {
            "@echo off\r\nexit /b 0\r\n"
        } else {
            "#!/bin/sh\nexit 0\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&fake_codegraph, fs::Permissions::from_mode(0o755)).unwrap();
    let fake_node = fake_bin.join(if cfg!(windows) { "node.cmd" } else { "node" });
    fs::write(
        &fake_node,
        if cfg!(windows) {
            "@echo off\r\nexit /b 0\r\n"
        } else {
            "#!/bin/sh\nexit 0\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&fake_node, fs::Permissions::from_mode(0o755)).unwrap();
    let probe_marker = environment._root.path().join("claude-probe-cwd");
    let fake_claude = fake_bin.join(if cfg!(windows) {
        "claude.cmd"
    } else {
        "claude"
    });
    fs::write(
        &fake_claude,
        if cfg!(windows) {
            r#"@echo off
if "%1 %2"=="mcp list" (
  if not "%CD%"=="%FAKE_EXPECTED_WORKSPACE%" if not "%CD%"=="%FAKE_NEUTRAL_HOME%" exit /b 42
  echo list:%CD%>>"%FAKE_CLAUDE_CWD_MARKER%"
  echo ags: %FAKE_AGS_MCP_COMMAND% mcp serve --transport stdio - Connected [workspace]
  echo codegraph: %FAKE_CODEGRAPH_COMMAND% serve --mcp - Connected [user]
  exit /b 0
)
if "%1 %2 %3"=="mcp get ags" (
  if not "%CD%"=="%FAKE_NEUTRAL_HOME%" exit /b 43
  echo get:%CD%:ags>>"%FAKE_CLAUDE_CWD_MARKER%"
  echo ags command: %FAKE_AGS_MCP_COMMAND% mcp serve --transport stdio
  exit /b 0
)
if "%1 %2 %3"=="mcp get codegraph" (
  if not "%CD%"=="%FAKE_NEUTRAL_HOME%" exit /b 43
  echo get:%CD%:codegraph>>"%FAKE_CLAUDE_CWD_MARKER%"
  echo codegraph command: %FAKE_CODEGRAPH_COMMAND% serve --mcp
  exit /b 0
)
exit /b 2
"#
        } else {
            r#"#!/bin/sh
set -eu
if [ "$1" = "mcp" ] && [ "$2" = "list" ]; then
  case "$PWD" in
    "$FAKE_EXPECTED_WORKSPACE"|"$FAKE_NEUTRAL_HOME") ;;
    *)
      echo "claude mcp list ran from forbidden cwd $PWD" >&2
      exit 42
      ;;
  esac
  printf 'list:%s\n' "$PWD" >> "$FAKE_CLAUDE_CWD_MARKER"
  printf 'ags: %s mcp serve --transport stdio - ✓ Connected [workspace]\n' "$FAKE_AGS_MCP_COMMAND"
  printf 'codegraph: %s serve --mcp - ✓ Connected [user]\n' "$FAKE_CODEGRAPH_COMMAND"
  exit 0
fi
if [ "$1" = "mcp" ] && [ "$2" = "get" ] && [ "$3" = "ags" ]; then
  if [ "$PWD" != "$FAKE_NEUTRAL_HOME" ]; then
    echo "claude mcp get ags ran from forbidden cwd $PWD" >&2
    exit 43
  fi
  printf 'get:%s:ags\n' "$PWD" >> "$FAKE_CLAUDE_CWD_MARKER"
  printf 'ags command: %s mcp serve --transport stdio\n' "$FAKE_AGS_MCP_COMMAND"
  exit 0
fi
if [ "$1" = "mcp" ] && [ "$2" = "get" ] && [ "$3" = "codegraph" ]; then
  if [ "$PWD" != "$FAKE_NEUTRAL_HOME" ]; then
    echo "claude mcp get codegraph ran from forbidden cwd $PWD" >&2
    exit 43
  fi
  printf 'get:%s:codegraph\n' "$PWD" >> "$FAKE_CLAUDE_CWD_MARKER"
  printf 'codegraph command: %s serve --mcp\n' "$FAKE_CODEGRAPH_COMMAND"
  exit 0
fi
echo "unsupported fake claude invocation: $*" >&2
exit 2
"#
        },
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&fake_claude, fs::Permissions::from_mode(0o755)).unwrap();

    let path = if cfg!(windows) {
        format!(
            "{};{}",
            fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    } else {
        format!("{}:/usr/bin:/bin", fake_bin.display())
    };
    let expected_workspace_for_shell = shell_visible_path(&canonical_project_a);
    let neutral_home_for_shell = shell_visible_path(&canonical_home);
    let governed = environment
        .command()
        .current_dir(&canonical_project_a)
        .args(["agents", "govern", "--agent", "claude-code", "--target"])
        .arg(&canonical_project_a)
        .args(["--apply", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        governed.status.success(),
        "govern failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&governed.stdout),
        String::from_utf8_lossy(&governed.stderr)
    );

    let write_claude_snapshot = || {
        environment
            .command()
            .env("HOME", &canonical_home)
            .env("USERPROFILE", &canonical_home)
            .env("PATH", &path)
            .env("FAKE_EXPECTED_WORKSPACE", &expected_workspace_for_shell)
            .env("FAKE_NEUTRAL_HOME", &neutral_home_for_shell)
            .env("FAKE_CLAUDE_CWD_MARKER", &probe_marker)
            .env("FAKE_AGS_MCP_COMMAND", &current_ags)
            .env("FAKE_CODEGRAPH_COMMAND", &fake_codegraph)
            .current_dir(&canonical_project_a)
            .args([
                "capability",
                "snapshot",
                "--host",
                "claude-code",
                "--target",
            ])
            .arg(&canonical_project_a)
            .args(["--write", "--format", "json"])
            .output()
            .unwrap()
    };
    let snapshot = write_claude_snapshot();
    assert!(
        snapshot.status.success(),
        "snapshot failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&snapshot.stdout),
        String::from_utf8_lossy(&snapshot.stderr)
    );

    let mut client = environment.connect(&canonical_project_a);
    client.initialize("claude-code");
    let preflight = client.preflight("claude-code", &canonical_project_a);
    assert_eq!(preflight["overall_status"], "ok");
    assert_eq!(preflight["capability_catalog"]["status"], "ready");

    let run_doctor = |registered_ags: &Path| {
        environment
            .command()
            .env("HOME", &canonical_home)
            .env("USERPROFILE", &canonical_home)
            .env("AGS_HOME", &environment.runtime)
            .env("AGS_REMOTE_LATEST_OFFLINE", "1")
            .env("PATH", &path)
            .env("FAKE_EXPECTED_WORKSPACE", &expected_workspace_for_shell)
            .env("FAKE_NEUTRAL_HOME", &neutral_home_for_shell)
            .env("FAKE_CLAUDE_CWD_MARKER", &probe_marker)
            .env("FAKE_AGS_MCP_COMMAND", registered_ags)
            .env("FAKE_CODEGRAPH_COMMAND", &fake_codegraph)
            .current_dir(&canonical_project_b)
            .args(["doctor", "--target"])
            .arg(&canonical_project_a)
            .args(["--format", "json"])
            .output()
            .unwrap()
    };
    let finding_status = |report: &Value, check_name: &str| {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["check_name"] == check_name)
            .unwrap_or_else(|| panic!("missing Doctor finding {check_name}: {report}"))["status"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let _ = fs::remove_file(&probe_marker);
    let current = run_doctor(&current_ags);
    assert!(
        current.status.success(),
        "current Doctor failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&current.stdout),
        String::from_utf8_lossy(&current.stderr)
    );
    let current_report: Value = serde_json::from_slice(&current.stdout).unwrap();
    assert!(
        current_report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["status"] != "fail"),
        "current local conformance contains failures: {current_report}"
    );
    for check in [
        "lifecycle-adapter-claude-code-current",
        "workspace-daemon-current",
        "capability-snapshot-current",
        "mcp-registration-current",
    ] {
        assert_eq!(finding_status(&current_report, check), "pass", "{check}");
    }
    let probe_cwds = fs::read_to_string(&probe_marker).unwrap();
    assert!(
        probe_cwds
            .lines()
            .any(|line| line == format!("list:{neutral_home_for_shell}")),
        "global Claude MCP inspection did not use neutral HOME: {probe_cwds}"
    );
    assert!(
        probe_cwds
            .lines()
            .any(|line| line == format!("list:{expected_workspace_for_shell}")),
        "workspace Claude MCP inspection ignored the explicit target: {probe_cwds}"
    );
    assert!(
        !probe_cwds.contains(&canonical_project_b.to_string_lossy().to_string()),
        "Claude MCP inspection inherited the Doctor caller cwd: {probe_cwds}"
    );

    environment.install_test_skill("doctor-snapshot-drift");
    let source_stale = run_doctor(&current_ags);
    assert_eq!(source_stale.status.code(), Some(1));
    let source_stale_report: Value = serde_json::from_slice(&source_stale.stdout).unwrap();
    assert_eq!(
        finding_status(&source_stale_report, "capability-snapshot-current"),
        "fail"
    );

    let refreshed_snapshot = write_claude_snapshot();
    assert!(
        refreshed_snapshot.status.success(),
        "snapshot refresh failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&refreshed_snapshot.stdout),
        String::from_utf8_lossy(&refreshed_snapshot.stderr)
    );
    let daemon_stale = run_doctor(&current_ags);
    assert_eq!(daemon_stale.status.code(), Some(1));
    let daemon_stale_report: Value = serde_json::from_slice(&daemon_stale.stdout).unwrap();
    assert_eq!(
        finding_status(&daemon_stale_report, "capability-snapshot-current"),
        "fail"
    );

    drop(client);
    let restart = environment
        .command()
        .args(["mcp", "restart", "--target"])
        .arg(&canonical_project_a)
        .output()
        .unwrap();
    assert!(
        restart.status.success(),
        "daemon restart failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    let mut restarted_client = environment.connect(&canonical_project_a);
    restarted_client.initialize("claude-code");
    let restarted_preflight = restarted_client.preflight("claude-code", &canonical_project_a);
    assert_eq!(restarted_preflight["overall_status"], "ok");
    let snapshot_recovered = run_doctor(&current_ags);
    assert!(
        snapshot_recovered.status.success(),
        "Doctor did not recover after snapshot refresh and daemon restart\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&snapshot_recovered.stdout),
        String::from_utf8_lossy(&snapshot_recovered.stderr)
    );

    let runtime_asset = environment.runtime.join("mcp/ags.mcp.json");
    let canonical_runtime_asset = fs::read(&runtime_asset).unwrap();
    fs::write(&runtime_asset, "{}\n").unwrap();
    let runtime_drift = run_doctor(&current_ags);
    assert_eq!(runtime_drift.status.code(), Some(1));
    let runtime_drift_report: Value = serde_json::from_slice(&runtime_drift.stdout).unwrap();
    assert_eq!(
        finding_status(&runtime_drift_report, "runtime-install-content-current"),
        "fail"
    );
    fs::write(&runtime_asset, canonical_runtime_asset).unwrap();
    let runtime_recovered = run_doctor(&current_ags);
    assert!(
        runtime_recovered.status.success(),
        "Doctor did not recover after runtime asset restoration\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&runtime_recovered.stdout),
        String::from_utf8_lossy(&runtime_recovered.stderr)
    );

    let settings_path = canonical_project_a.join(".claude/settings.local.json");
    let canonical_settings = fs::read_to_string(&settings_path).unwrap();
    let json_path = |path: &Path| {
        serde_json::to_string(&path.to_string_lossy())
            .unwrap()
            .trim_matches('"')
            .to_string()
    };
    let wrong_target_settings = canonical_settings.replace(
        &json_path(&canonical_project_a),
        &json_path(&canonical_project_b),
    );
    assert_ne!(wrong_target_settings, canonical_settings);
    fs::write(&settings_path, wrong_target_settings).unwrap();
    let wrong_target = run_doctor(&current_ags);
    assert_eq!(wrong_target.status.code(), Some(1));
    let wrong_target_report: Value = serde_json::from_slice(&wrong_target.stdout).unwrap();
    assert_eq!(
        finding_status(
            &wrong_target_report,
            "lifecycle-adapter-claude-code-current"
        ),
        "fail"
    );

    fs::write(&settings_path, canonical_settings).unwrap();
    let old_registration = run_doctor(&old_ags);
    assert_eq!(old_registration.status.code(), Some(1));
    let old_registration_report: Value = serde_json::from_slice(&old_registration.stdout).unwrap();
    assert_eq!(
        finding_status(&old_registration_report, "mcp-registration-current"),
        "fail"
    );

    drop(restarted_client);
}
