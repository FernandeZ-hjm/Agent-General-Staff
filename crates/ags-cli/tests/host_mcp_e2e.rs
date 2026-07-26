use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HOSTS: &[&str] = &["codex", "claude-code", "cursor", "omp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeProbeStatus {
    Passed,
    Unavailable,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone)]
enum RegistrationProbe {
    Registered(String),
    ConfiguredOnly(String),
    NotRegistered(String),
    Unsupported(String),
    Failed(String),
}

#[derive(Debug, Serialize)]
struct NativeHostDiagnostic {
    harness_kind: &'static str,
    host: &'static str,
    status: NativeProbeStatus,
    executable: Option<String>,
    version: Option<String>,
    registration_source: Option<String>,
    evidence: String,
}

fn classify_native_probe(
    host_detected: bool,
    registration: RegistrationProbe,
) -> NativeProbeStatus {
    if !host_detected {
        return NativeProbeStatus::Unavailable;
    }
    match registration {
        RegistrationProbe::Registered(_) => NativeProbeStatus::Passed,
        RegistrationProbe::ConfiguredOnly(_) | RegistrationProbe::Unsupported(_) => {
            NativeProbeStatus::Unsupported
        }
        RegistrationProbe::NotRegistered(_) | RegistrationProbe::Failed(_) => {
            NativeProbeStatus::Failed
        }
    }
}

fn first_output_line(output: &std::process::Output) -> Option<String> {
    [&output.stdout[..], &output.stderr[..]]
        .into_iter()
        .flat_map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .map(|line| line.trim().to_string())
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect())
}

fn executable_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("version probe failed: {error}"))?;
    let line = first_output_line(&output).unwrap_or_else(|| "no version output".to_string());
    if output.status.success() {
        Ok(line)
    } else {
        Err(format!("version probe exited {}: {line}", output.status))
    }
}

fn executable_override(host: &str) -> Option<PathBuf> {
    let variable = match host {
        "codex" => "AGS_NATIVE_CODEX_BIN",
        "claude-code" => "AGS_NATIVE_CLAUDE_CODE_BIN",
        "cursor" => "AGS_NATIVE_CURSOR_BIN",
        "omp" => "AGS_NATIVE_OMP_BIN",
        _ => return None,
    };
    std::env::var_os(variable).map(PathBuf::from)
}

fn executable_names(host: &str) -> &'static [&'static str] {
    match host {
        "codex" => &["codex"],
        "claude-code" => &["claude"],
        "cursor" => &["cursor"],
        "omp" => &["omp"],
        _ => &[],
    }
}

fn executable_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            for extension in ["exe", "cmd", "bat", "com"] {
                let candidate = directory.join(format!("{name}.{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn native_executable(host: &str, home: &Path) -> Option<PathBuf> {
    if let Some(path) = executable_override(host) {
        return path.is_file().then_some(path);
    }
    if let Some(path) = executable_on_path(executable_names(host)) {
        return Some(path);
    }
    if host == "cursor" {
        for candidate in [
            PathBuf::from("/Applications/Cursor.app/Contents/Resources/app/bin/cursor"),
            home.join("Applications/Cursor.app/Contents/Resources/app/bin/cursor"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn registered_line(
    executable: &Path,
    arguments: &[&str],
    line_matches: impl Fn(&str) -> bool,
) -> RegistrationProbe {
    let output = match Command::new(executable).args(arguments).output() {
        Ok(output) => output,
        Err(error) => return RegistrationProbe::Failed(error.to_string()),
    };
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        return RegistrationProbe::Failed(
            combined
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("registration probe failed")
                .chars()
                .take(240)
                .collect(),
        );
    }
    match combined.lines().find(|line| line_matches(line.trim())) {
        Some(line) => RegistrationProbe::Registered(line.trim().chars().take(240).collect()),
        None => RegistrationProbe::NotRegistered("AGS registration not found".to_string()),
    }
}

fn json_config_registration(paths: &[PathBuf]) -> RegistrationProbe {
    let mut readable = false;
    for path in paths {
        let content = match fs::read_to_string(path) {
            Ok(content) => {
                readable = true;
                content
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return RegistrationProbe::Failed(format!(
                    "cannot read {}: {error}",
                    path.display()
                ))
            }
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(error) => {
                return RegistrationProbe::Failed(format!(
                    "invalid JSON at {}: {error}",
                    path.display()
                ))
            }
        };
        let registered = value
            .get("mcpServers")
            .or_else(|| value.get("servers"))
            .and_then(|servers| servers.get("ags"))
            .is_some();
        if registered {
            return RegistrationProbe::ConfiguredOnly(format!(
                "AGS entry in {}, but configuration alone is not a live Cursor connection probe",
                path.display()
            ));
        }
    }
    if readable {
        RegistrationProbe::NotRegistered("readable MCP config has no AGS entry".to_string())
    } else {
        RegistrationProbe::Unsupported(
            "no readable Cursor MCP configuration found for a native connection probe".to_string(),
        )
    }
}

fn cursor_agent_executable() -> Option<PathBuf> {
    std::env::var_os("AGS_NATIVE_CURSOR_AGENT_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| executable_on_path(&["cursor-agent"]))
}

fn cursor_live_registration_probe() -> (RegistrationProbe, Option<String>) {
    let Some(cursor_agent) = cursor_agent_executable() else {
        return (
            RegistrationProbe::Unsupported(
                "Cursor IDE is installed, but standalone `cursor-agent` is unavailable; the IDE CLI has no read-only MCP list/connect command and must not be treated as a live probe"
                    .to_string(),
            ),
            Some("standalone `cursor-agent mcp list-tools ags`".to_string()),
        );
    };
    (
        registered_line(&cursor_agent, &["mcp", "list-tools", "ags"], |line| {
            line.contains("ags_preflight")
        }),
        Some(format!("{} mcp list-tools ags", cursor_agent.display())),
    )
}

fn receive_rpc_message(
    receiver: &mpsc::Receiver<Value>,
    deadline: Instant,
) -> Result<Value, String> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| "OMP RPC probe timed out".to_string())?;
    match receiver.recv_timeout(remaining) {
        Ok(message) => Ok(message),
        Err(mpsc::RecvTimeoutError::Timeout) => Err("OMP RPC probe timed out".to_string()),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("OMP RPC stdout closed before the probe completed".to_string())
        }
    }
}

fn omp_log_excerpt(home: &Path) -> String {
    let Ok(entries) = fs::read_dir(home.join(".omp/logs")) else {
        return String::new();
    };
    let mut lines = Vec::new();
    for entry in entries.flatten() {
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        lines.extend(content.lines().filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            let message = value["message"].as_str().unwrap_or_default();
            let path = value["path"].as_str().unwrap_or_default();
            let level = value["level"].as_str().unwrap_or_default();
            (message.to_ascii_lowercase().contains("mcp")
                || path.starts_with("mcp:")
                || level == "error")
                .then(|| line.to_string())
        }));
    }
    lines
        .into_iter()
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
        .chars()
        .take(1_200)
        .collect()
}

fn omp_live_registration_probe(executable: &Path) -> RegistrationProbe {
    let fixture = TestDir::new("native-omp");
    let project = fixture.path().join("project");
    let runtime = fixture.path().join("runtime");
    let home = fixture.path().join("home");
    if let Err(error) = fs::create_dir_all(&project)
        .and_then(|()| fs::create_dir_all(&runtime))
        .and_then(|()| fs::create_dir_all(home.join(".omp/agent")))
    {
        return RegistrationProbe::Failed(format!("cannot prepare isolated OMP probe: {error}"));
    }
    if !Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&project)
        .status()
        .is_ok_and(|status| status.success())
    {
        return RegistrationProbe::Failed(
            "cannot initialize isolated OMP probe workspace".to_string(),
        );
    }
    let ags = PathBuf::from(env!("CARGO_BIN_EXE_ags"));
    let config = json!({
        "mcpServers": {
            "ags_e2e": {
                "command": ags,
                "args": ["mcp", "serve", "--transport", "stdio"]
            }
        }
    });
    let omp_config_dir = project.join(".omp");
    let config_bytes = serde_json::to_vec_pretty(&config).unwrap();
    if let Err(error) = fs::create_dir_all(&omp_config_dir)
        .and_then(|()| fs::write(omp_config_dir.join("mcp.json"), &config_bytes))
    {
        return RegistrationProbe::Failed(format!(
            "cannot write isolated OMP MCP fixture: {error}"
        ));
    }

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut child = match Command::new(executable)
        .args(["--mode", "rpc", "--no-session", "--model", "opus", "--cwd"])
        .arg(&project)
        .current_dir(&project)
        .env("AGS_RUNTIME_HOME", &runtime)
        .env("AGS_SOURCE_ROOT", &source_root)
        .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        // RPC state/tool inspection never invokes the model. A deterministic
        // placeholder only lets isolated OMP startup select its built-in model
        // catalog without reading the operator's real credential store.
        .env("ANTHROPIC_API_KEY", "ags-native-e2e-placeholder")
        .env("OMP_MCP_TIMEOUT_MS", "5000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return RegistrationProbe::Failed(format!("cannot start OMP RPC probe: {error}"))
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stderr_capture = Arc::new(Mutex::new(String::new()));
    let stderr_writer = Arc::clone(&stderr_capture);
    let stderr_reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut content = String::new();
        let _ = stderr.read_to_string(&mut content);
        if let Ok(mut captured) = stderr_writer.lock() {
            *captured = content;
        }
    });
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Ok(message) = serde_json::from_str::<Value>(&line) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    let result = (|| -> Result<(Vec<String>, bool), String> {
        loop {
            let message = receive_rpc_message(&receiver, deadline)?;
            if message["type"] == "ready" {
                break;
            }
        }
        // OMP emits `ready` before its asynchronous MCP discovery/connect
        // tasks have necessarily populated the session tool registry. Poll the
        // read-only state endpoint until the required tools arrive or the
        // single startup deadline expires.
        let required = [
            "mcp__ags_e2e__ags_preflight",
            "mcp__ags_e2e__ags_route_request",
            "mcp__ags_e2e__ags_apply_action",
        ];
        let mut attempt = 0_u32;
        let mut last_names = Vec::new();
        let mut last_discoverable = false;
        while Instant::now() < deadline {
            attempt += 1;
            let request_id = format!("ags-native-omp-probe-{attempt}");
            writeln!(stdin, "{}", json!({"id": request_id, "type": "get_state"}))
                .map_err(|error| format!("cannot write OMP RPC request: {error}"))?;
            stdin
                .flush()
                .map_err(|error| format!("cannot flush OMP RPC request: {error}"))?;
            loop {
                let message = receive_rpc_message(&receiver, deadline)?;
                if message["type"] == "response" && message["id"] == request_id {
                    if message["success"] != true {
                        return Err(format!(
                            "OMP RPC get_state failed: {}",
                            message["error"].as_str().unwrap_or("unknown error")
                        ));
                    }
                    last_names = message["data"]["dumpTools"]
                        .as_array()
                        .ok_or_else(|| "OMP RPC get_state omitted dumpTools".to_string())?
                        .iter()
                        .filter_map(|tool| tool["name"].as_str())
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    let prompt =
                        serde_json::to_string(&message["data"]["systemPrompt"]).unwrap_or_default();
                    last_discoverable = ["ags_preflight", "ags_route_request", "ags_apply_action"]
                        .iter()
                        .all(|tool| prompt.contains(tool));
                    break;
                }
            }
            if required
                .iter()
                .all(|required| last_names.iter().any(|name| name == required))
                || last_discoverable
            {
                return Ok((last_names, last_discoverable));
            }
            thread::sleep(Duration::from_millis(250));
        }
        Ok((last_names, last_discoverable))
    })();
    let _ = child.kill();
    let _ = child.wait();
    drop(receiver);
    let _ = reader.join();
    let _ = stderr_reader.join();
    let stderr = stderr_capture
        .lock()
        .map(|captured| captured.trim().chars().take(600).collect::<String>())
        .unwrap_or_default();
    let log_excerpt = omp_log_excerpt(&home);
    let diagnostic_output = match (stderr.is_empty(), log_excerpt.is_empty()) {
        (true, true) => "(empty)".to_string(),
        (false, true) => stderr,
        (true, false) => log_excerpt,
        (false, false) => format!("{stderr}; log: {log_excerpt}"),
    };

    match result {
        Ok((names, discoverable)) => {
            let required = [
                "mcp__ags_e2e__ags_preflight",
                "mcp__ags_e2e__ags_route_request",
                "mcp__ags_e2e__ags_apply_action",
            ];
            let missing = required
                .iter()
                .filter(|required| !names.iter().any(|name| name == **required))
                .copied()
                .collect::<Vec<_>>();
            if missing.is_empty() || discoverable {
                RegistrationProbe::Registered(format!(
                    "live OMP RPC session loaded AGS preflight/route/apply into its {} surface",
                    if missing.is_empty() {
                        "active tool"
                    } else {
                        "discoverable MCP"
                    }
                ))
            } else {
                RegistrationProbe::NotRegistered(format!(
                    "live OMP RPC session did not expose required AGS tools: {}; observed candidate tools: {}; stderr: {}",
                    missing.join(", "),
                    if names.iter().all(|name| !name.starts_with("mcp__")) {
                        "(none)".to_string()
                    } else {
                        names
                            .iter()
                            .filter(|name| name.starts_with("mcp__"))
                            .take(20)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                    diagnostic_output
                ))
            }
        }
        Err(error) => RegistrationProbe::Failed(format!("{error}; stderr: {}", diagnostic_output)),
    }
}

fn native_registration_probe(
    host: &str,
    executable: Option<&Path>,
    home: &Path,
    project: &Path,
) -> (RegistrationProbe, Option<String>) {
    match (host, executable) {
        ("codex", Some(executable)) => (
            registered_line(executable, &["mcp", "list"], |line| {
                (line.starts_with("ags ") || line.starts_with("ags:")) && line.contains("enabled")
            }),
            Some("codex mcp list".to_string()),
        ),
        ("claude-code", Some(executable)) => (
            registered_line(executable, &["mcp", "list"], |line| {
                line.starts_with("ags:") && line.contains("Connected")
            }),
            Some("claude mcp list".to_string()),
        ),
        ("cursor", _) => {
            let (live, source) = cursor_live_registration_probe();
            if !matches!(live, RegistrationProbe::Unsupported(_)) {
                return (live, source);
            }
            let paths = [
                home.join(".cursor/mcp.json"),
                home.join("Library/Application Support/Cursor/User/mcp.json"),
                project.join(".cursor/mcp.json"),
                project.join(".mcp.json"),
            ];
            let configured = json_config_registration(&paths);
            if matches!(configured, RegistrationProbe::Unsupported(_)) {
                (live, source)
            } else {
                (
                    configured,
                    Some("Cursor MCP configuration (not a live probe)".to_string()),
                )
            }
        }
        ("omp", Some(executable)) => (
            omp_live_registration_probe(executable),
            Some("isolated live `omp --mode rpc` get_state dumpTools/systemPrompt".to_string()),
        ),
        _ => (
            RegistrationProbe::Unsupported(
                "no safe read-only native registration probe is available".to_string(),
            ),
            None,
        ),
    }
}

fn native_host_diagnostic(host: &'static str, home: &Path, project: &Path) -> NativeHostDiagnostic {
    let executable = native_executable(host, home);
    let version = executable.as_deref().map(executable_version);
    let executable_works = version.as_ref().is_some_and(Result::is_ok);
    let (registration, source) =
        native_registration_probe(host, executable.as_deref(), home, project);
    let detected = executable_works
        || (host == "cursor"
            && [
                home.join(".cursor"),
                PathBuf::from("/Applications/Cursor.app"),
            ]
            .iter()
            .any(|path| path.exists()));
    let status = if version.as_ref().is_some_and(Result::is_err) {
        NativeProbeStatus::Failed
    } else {
        classify_native_probe(detected, registration.clone())
    };
    let evidence = match registration {
        RegistrationProbe::Registered(evidence)
        | RegistrationProbe::ConfiguredOnly(evidence)
        | RegistrationProbe::NotRegistered(evidence)
        | RegistrationProbe::Failed(evidence) => evidence,
        RegistrationProbe::Unsupported(evidence) => evidence,
    };
    NativeHostDiagnostic {
        harness_kind: "native",
        host,
        status,
        executable: executable.map(|path| path.display().to_string()),
        version: version.map(|result| result.unwrap_or_else(|error| error)),
        registration_source: source,
        evidence,
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

/// Hermetic protocol fixture used by CI. Host names select AGS adapter
/// behavior; they are not evidence that native host executables are installed
/// or registered. The ignored native harness below owns that evidence.
#[test]
fn hermetic_host_adapters_share_one_workspace_service_but_keep_sessions_and_leases_isolated() {
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
        "different workspaces shared one capability bundle identity"
    );
    assert!(preflight["capability_catalog"]["bundle_epoch"]
        .as_u64()
        .is_some_and(|epoch| epoch > 0));
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
fn native_probe_classification_distinguishes_unavailable_unsupported_and_passed() {
    assert_eq!(
        classify_native_probe(
            false,
            RegistrationProbe::Unsupported("no native probe".to_string())
        ),
        NativeProbeStatus::Unavailable
    );
    assert_eq!(
        classify_native_probe(
            true,
            RegistrationProbe::Unsupported("no native probe".to_string())
        ),
        NativeProbeStatus::Unsupported
    );
    assert_eq!(
        classify_native_probe(
            true,
            RegistrationProbe::Registered("host-native AGS registration".to_string())
        ),
        NativeProbeStatus::Passed
    );
}

/// Opt-in native-host harness.
///
/// This test reads the real host executables and MCP registrations. It is
/// ignored in CI because the hermetic adapter fixture is not evidence that a
/// native Codex/Claude Code/Cursor/OMP installation can see AGS. Operators can
/// require selected hosts to pass with:
///
/// `AGS_NATIVE_HOSTS_REQUIRED=codex,claude-code cargo test -p ags-cli
/// --test host_mcp_e2e native_host_registration_harness -- --ignored --nocapture`
#[test]
#[ignore = "requires native host executables and MCP registrations"]
fn native_host_registration_harness() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let project = std::env::current_dir().unwrap();
    let diagnostics = HOSTS
        .iter()
        .map(|host| native_host_diagnostic(host, &home, &project))
        .collect::<Vec<_>>();
    eprintln!("{}", serde_json::to_string_pretty(&diagnostics).unwrap());

    let required = std::env::var("AGS_NATIVE_HOSTS_REQUIRED")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let known = HOSTS.iter().copied().collect::<BTreeSet<_>>();
    for host in &required {
        assert!(
            known.contains(host.as_str()),
            "unknown required host: {host}"
        );
    }
    for diagnostic in diagnostics {
        if required.contains(diagnostic.host) {
            assert_eq!(
                diagnostic.status,
                NativeProbeStatus::Passed,
                "required native host did not pass: {} ({})",
                diagnostic.host,
                diagnostic.evidence
            );
        }
    }
}

#[test]
fn refreshed_snapshot_rebinds_the_existing_workspace_session() {
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

    let stale_resource = client.request_envelope(
        3,
        "resources/read",
        json!({"uri": "ags://capabilities/current-host"}),
    );
    assert!(stale_resource["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("skill_snapshot_stale")));

    let claude_current = claude.current_host_snapshot();
    assert_eq!(claude_current["host"], "claude-code");
    assert_eq!(
        claude_current["snapshot_hash"], claude_initial["snapshot_hash"],
        "refreshing Codex invalidated the independent Claude host generation"
    );

    let stale_route = client.route_project_verify();
    assert_eq!(stale_route["governance_status"], "BLOCKED_BY_POLICY");
    assert!(stale_route["lease"].is_null());
    assert_eq!(stale_route["errors"][0]["code"], "skill_snapshot_stale");
    assert!(stale_route["errors"][0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("rerun ags_preflight")));

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
            .any(|card| { card["skill_id"].as_str() == Some("e2e-refresh-skill") })),
        "refreshed current-host catalog omitted the new skill"
    );
}

#[test]
fn failed_workspace_bundle_publish_never_becomes_ready_in_memory() {
    let environment = TestEnvironment::new();
    environment.write_snapshot("codex");

    let mut client = environment.connect(&environment.project_a);
    client.initialize("codex");
    let service_dir = environment.runtime.join("workspace-services");
    let registry_path = fs::read_dir(&service_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
                && !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".capabilities.json")
        })
        .unwrap();
    let registry: Value = serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
    let capability_path = service_dir.join(format!(
        "{}.capabilities.json",
        registry["instance_key"].as_str().unwrap()
    ));
    fs::create_dir(&capability_path).unwrap();

    let first = client.preflight("codex", &environment.project_a);
    let second = client.preflight("codex", &environment.project_a);
    for report in [&first, &second] {
        assert_eq!(
            report["capability_catalog"]["status"],
            "capability_unavailable"
        );
        assert_eq!(
            report["capability_catalog"]["error"]["code"],
            "capability_state_persistence_failed"
        );
        assert!(report["capability_catalog"]["snapshot_hash"].is_null());
    }
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
    let held = old_connection.route_project_verify();
    let old_lease = held["lease"]["lease_id"].as_str().unwrap().to_string();
    let old_action = held["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|target| target["action_id"].as_str())
        .unwrap()
        .to_string();
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

    let stale_apply = old_connection.request_envelope(
        5,
        "tools/call",
        json!({
            "name": "ags_apply_action",
            "arguments": {"lease_id": old_lease, "action_id": old_action}
        }),
    );
    assert!(stale_apply["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("runtime_process_stale")));
    let stale_route = old_connection.request_envelope(
        6,
        "tools/call",
        json!({
            "name": "ags_route_request",
            "arguments": {"proposal": {
                "schema_version": "0.3.0-host-route-proposal",
                "request_fingerprint": "sha256:post-upgrade",
                "phase": "direct_response",
                "solution_state": "not_required",
                "execution_authority": "none",
                "scope_hash": "sha256:post-upgrade",
                "targets": [{"kind": "direct_response"}]
            }}
        }),
    );
    assert!(stale_route["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("runtime_process_stale")));
    let stale_resource = old_connection.request_envelope(
        7,
        "resources/read",
        json!({"uri": "ags://capabilities/current-host"}),
    );
    assert!(stale_resource["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("runtime_process_stale")));

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
