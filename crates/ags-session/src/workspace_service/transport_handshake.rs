use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ags_platform::canonical_workspace_root;

use super::capability_snapshot::WorkspaceState;
use super::registry_ownership::{fresh_id, now_millis, ServicePaths, WorkspaceRegistry};
use super::upgrade_recycle::{connect_or_start, reclaim_registry_after_failed_handshake};
use super::WorkspaceSessionHandler;

pub(super) const WIRE_SCHEMA: &str = "ags-workspace-service/1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Handshake {
    pub(super) protocol: String,
    pub(super) token: String,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) command: Option<String>,
    pub(super) workspace: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HandshakeResult {
    pub(super) status: String,
    pub(super) workspace: PathBuf,
    pub(super) instance_key: String,
    pub(super) executable_hash: String,
    #[serde(default)]
    pub(super) process_start_identity: String,
    #[serde(default)]
    pub(super) daemon_nonce: String,
}

pub(super) struct SessionActivity {
    active_sessions: Arc<AtomicUsize>,
    last_activity: Arc<AtomicU64>,
}

impl SessionActivity {
    pub(super) fn begin(active_sessions: Arc<AtomicUsize>, last_activity: Arc<AtomicU64>) -> Self {
        active_sessions.fetch_add(1, Ordering::AcqRel);
        last_activity.store(now_millis(), Ordering::Release);
        Self {
            active_sessions,
            last_activity,
        }
    }
}

impl Drop for SessionActivity {
    fn drop(&mut self) {
        self.active_sessions.fetch_sub(1, Ordering::AcqRel);
        self.last_activity.store(now_millis(), Ordering::Release);
    }
}

pub(super) fn run_stdio_adapter_impl() -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| format!("current_dir failed: {error}"))?;
    let workspace = canonical_workspace_root(&cwd)?;
    let service_paths = ServicePaths::new(
        &ags_capability_governance::locate_runtime_home(),
        &workspace,
    );
    let mut first_error = None;
    let (mut stream, mut daemon_reader) = loop {
        match connect_workspace_session(&workspace) {
            Ok(session) => break session,
            Err(error) if first_error.is_none() => {
                first_error = Some(error);
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(format!(
                    "workspace daemon session handshake failed after retry: {}; first attempt: {}",
                    error,
                    first_error.unwrap_or_else(|| "unknown".to_string())
                ));
            }
        }
    };

    let _request_thread = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in BufReader::new(stdin.lock()).lines() {
            let Ok(line) = line else {
                break;
            };
            if stream
                .write_all(line.as_bytes())
                .and_then(|_| stream.write_all(b"\n"))
                .and_then(|_| stream.flush())
                .is_err()
            {
                break;
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Write);
    });

    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let read = daemon_reader
            .read_line(&mut line)
            .map_err(|error| format!("daemon read failed: {error}"))?;
        if read == 0 {
            let diagnostic = fs::read_to_string(&service_paths.diagnostics).unwrap_or_default();
            if !diagnostic.contains("stdin read error:")
                && !diagnostic.contains("panicked at")
                && !diagnostic.contains("workspace daemon connection failed:")
            {
                return Ok(());
            }
            return Err(format!(
                "workspace daemon closed the session; diagnostic log {}:\n{}",
                service_paths.diagnostics.display(),
                diagnostic.trim()
            ));
        }
        output
            .write_all(line.as_bytes())
            .and_then(|_| output.flush())
            .map_err(|error| format!("stdout proxy failed: {error}"))?;
    }
}

fn connect_workspace_session(
    workspace: &Path,
) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let (stream, registry) = connect_or_start(workspace)?;
    let paths = ServicePaths::new(&ags_capability_governance::locate_runtime_home(), workspace);
    finish_workspace_session(stream, &registry, workspace, &paths)
}

pub(super) fn finish_workspace_session(
    mut stream: TcpStream,
    registry: &WorkspaceRegistry,
    workspace: &Path,
    paths: &ServicePaths,
) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let handshake = Handshake {
        protocol: WIRE_SCHEMA.to_string(),
        token: registry.token.clone(),
        kind: "session".to_string(),
        command: None,
        workspace: workspace.to_path_buf(),
    };
    let result = (|| {
        write_json_line(&mut stream, &handshake)?;
        let mut daemon_reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|error| format!("daemon stream clone failed: {error}"))?,
        );
        let ready: HandshakeResult = read_json_line(&mut daemon_reader)?;
        if ready.status != "ready"
            || ready.workspace != workspace
            || ready.instance_key != registry.instance_key
            || ready.executable_hash != registry.executable_hash
            || (!registry.process_start_identity.is_empty()
                && ready.process_start_identity != registry.process_start_identity)
            || (!registry.daemon_nonce.is_empty() && ready.daemon_nonce != registry.daemon_nonce)
        {
            return Err("workspace daemon handshake mismatch".to_string());
        }
        Ok((stream, daemon_reader))
    })();
    if result.is_err() {
        reclaim_registry_after_failed_handshake(paths, registry);
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_workspace_connection(
    stream: TcpStream,
    registry: &WorkspaceRegistry,
    state: &Arc<WorkspaceState>,
    shutdown: &Arc<AtomicBool>,
    handler: &Arc<dyn WorkspaceSessionHandler>,
    active_sessions: &Arc<AtomicUsize>,
    last_activity: &Arc<AtomicU64>,
) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("accepted stream blocking mode failed: {error}"))?;
    let activity = SessionActivity::begin(Arc::clone(active_sessions), Arc::clone(last_activity));
    let registry = registry.clone();
    let state = Arc::clone(state);
    let shutdown = Arc::clone(shutdown);
    let handler = Arc::clone(handler);
    let _connection = std::thread::spawn(move || {
        let _activity = activity;
        if let Err(error) = handle_connection(stream, registry, state, shutdown, handler) {
            let _ = writeln!(
                std::io::stderr(),
                "[ags-mcp] workspace daemon connection failed: {error}"
            );
        }
    });
    Ok(())
}

pub(super) fn handle_connection(
    stream: TcpStream,
    registry: WorkspaceRegistry,
    state: Arc<WorkspaceState>,
    shutdown: Arc<AtomicBool>,
    handler: Arc<dyn WorkspaceSessionHandler>,
) -> Result<(), String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("stream clone failed: {error}"))?,
    );
    let handshake: Handshake = read_json_line(&mut reader)?;
    if handshake.protocol != WIRE_SCHEMA
        || handshake.token != registry.token
        || canonical_workspace_root(&handshake.workspace)? != registry.workspace
    {
        return Err("workspace daemon authentication failed".to_string());
    }
    let mut writer = stream;
    if handshake.kind == "control" && handshake.command.as_deref() == Some("shutdown") {
        write_json_line(
            &mut writer,
            &HandshakeResult {
                status: "stopping".to_string(),
                workspace: registry.workspace,
                instance_key: registry.instance_key,
                executable_hash: registry.executable_hash,
                process_start_identity: registry.process_start_identity,
                daemon_nonce: registry.daemon_nonce,
            },
        )?;
        shutdown.store(true, Ordering::Release);
        return Ok(());
    }
    if handshake.kind != "session" {
        return Err("unsupported workspace daemon handshake".to_string());
    }
    let session_id = fresh_id("daemon-session", &registry.workspace);
    let startup_executable_hash = registry.executable_hash.clone();
    write_json_line(
        &mut writer,
        &HandshakeResult {
            status: "ready".to_string(),
            workspace: registry.workspace,
            instance_key: registry.instance_key,
            executable_hash: registry.executable_hash,
            process_start_identity: registry.process_start_identity,
            daemon_nonce: registry.daemon_nonce,
        },
    )?;
    handler.run(reader, writer, state, session_id, startup_executable_hash);
    Ok(())
}

pub(super) fn write_json_line<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| format!("workspace wire encode failed: {error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("workspace wire write failed: {error}"))
}

pub(super) fn read_json_line<T: for<'de> Deserialize<'de>>(
    reader: &mut impl BufRead,
) -> Result<T, String> {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| format!("workspace wire read failed: {error}"))?;
    if read == 0 {
        return Err("workspace daemon closed during handshake".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("workspace wire invalid: {error}"))
}
