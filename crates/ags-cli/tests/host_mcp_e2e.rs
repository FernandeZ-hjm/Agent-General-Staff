use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
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

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
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
        assert!(
            !line.is_empty(),
            "MCP server closed before responding to {method}"
        );
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
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
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn registered_host_identities_reconnect_and_share_validated_snapshots_across_projects() {
    let environment = TestEnvironment::new();

    for host in HOSTS {
        let written = environment.write_snapshot(host);
        let expected_hash = written["snapshot_hash"].as_str().unwrap();

        {
            let mut first_connection = environment.connect(&environment.project_a);
            first_connection.initialize(host);
            let preflight = first_connection.preflight(host, &environment.project_a);
            assert_eq!(preflight["capability_catalog"]["status"], "ready");
            let snapshot = first_connection.current_host_snapshot();
            assert_eq!(snapshot["host"], *host);
            assert_eq!(snapshot["snapshot_hash"], expected_hash);
        }

        let mut reconnected = environment.connect(&environment.project_b);
        reconnected.initialize(host);
        let preflight = reconnected.preflight(host, &environment.project_b);
        assert_eq!(preflight["capability_catalog"]["status"], "ready");
        assert_eq!(
            Path::new(preflight["target"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            environment.project_b.canonicalize().unwrap()
        );
        let snapshot = reconnected.current_host_snapshot();
        assert_eq!(snapshot["host"], *host);
        assert_eq!(snapshot["snapshot_hash"], expected_hash);
    }
}

#[cfg(unix)]
#[test]
fn running_mcp_detects_replaced_executable_and_reconnect_recovers() {
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

    let replacement = environment._root.path().join("ags-replacement");
    fs::copy(&environment.ags, &replacement).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&replacement)
        .unwrap()
        .write_all(b"\0")
        .unwrap();
    fs::rename(&replacement, &live_executable).unwrap();

    let stale = old_connection.preflight("codex", &environment.project_a);
    assert_eq!(stale["runtime_process"]["status"], "stale");
    assert_eq!(
        stale["capability_catalog"]["status"],
        "runtime_process_stale"
    );
    assert_eq!(stale["capability_catalog"]["refresh_required"], false);
    assert_eq!(stale["capability_catalog"]["requires_host_restart"], true);
    drop(old_connection);

    let restored = environment._root.path().join("ags-restored");
    fs::copy(&environment.ags, &restored).unwrap();
    fs::rename(&restored, &live_executable).unwrap();

    let mut reconnected =
        environment.connect_with_executable(&environment.project_b, &live_executable);
    reconnected.initialize("codex");
    let recovered = reconnected.preflight("codex", &environment.project_b);
    assert_eq!(recovered["capability_catalog"]["status"], "ready");
    assert!(
        recovered.get("runtime_process").is_none(),
        "new process should load the installed executable and recover"
    );
}
