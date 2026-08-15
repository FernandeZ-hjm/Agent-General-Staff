use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ags_platform::canonical_workspace_root;

use super::capability_snapshot::WorkspaceState;
use super::registry_ownership::{fresh_id, now_millis, ServicePaths, WorkspaceRegistry};
use super::upgrade_recycle::{
    connect_existing_read_only_at, connect_or_start, reclaim_registry_after_failed_handshake,
};
use super::{
    WorkspaceClientIdentity, WorkspaceCommand, WorkspaceControlClient, WorkspaceServiceInspection,
    WorkspaceSessionContext, WorkspaceSessionHandler, WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION,
};

pub(super) const WIRE_PROTOCOL: &str = "ags-workspace-service/2";
pub const MAX_WORKSPACE_WIRE_FRAME_BYTES: usize = 1024 * 1024;
const PEER_CLOSED_BEFORE_HANDSHAKE: &str = "workspace daemon closed during handshake";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Handshake {
    pub(super) protocol: String,
    pub(super) token: String,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) client: Option<WorkspaceClientIdentity>,
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
    #[serde(default)]
    pub(super) authenticated_session: String,
    #[serde(default)]
    pub(super) project_facts_hash: String,
    #[serde(default)]
    pub(super) registry_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum WorkspaceCommandReply {
    Ok(serde_json::Value),
    Error(String),
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

pub(super) fn dispatch_workspace_command_impl(
    workspace: &Path,
    kind: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace = canonical_workspace_root(workspace)?;
    let (stream, registry) = connect_or_start(&workspace)?;
    dispatch_workspace_command_on(stream, registry, workspace, kind, payload)
}

pub(super) fn inspect_existing_workspace_service_impl(
    workspace: &Path,
) -> Result<Option<WorkspaceServiceInspection>, String> {
    let workspace = canonical_workspace_root(workspace)?;
    let runtime_home = ags_platform::runtime_home();
    inspect_existing_workspace_service_at(&runtime_home, &workspace)
}

pub(super) fn inspect_existing_workspace_service_at(
    runtime_home: &Path,
    workspace: &Path,
) -> Result<Option<WorkspaceServiceInspection>, String> {
    let Some((stream, registry)) = connect_existing_read_only_at(runtime_home, workspace)? else {
        return Ok(None);
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(1))))
        .map_err(|error| format!("workspace daemon inspection timeout setup failed: {error}"))?;
    let expected_identity = registry.instance_key.clone();
    let value = dispatch_workspace_command_on(
        stream,
        registry,
        workspace.to_path_buf(),
        "status",
        serde_json::Value::Null,
    )?;
    let inspection: WorkspaceServiceInspection = serde_json::from_value(value)
        .map_err(|error| format!("workspace daemon inspection invalid: {error}"))?;
    if inspection.schema_version != WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION
        || inspection.canonical_workspace != workspace.to_string_lossy()
        || inspection.workspace_identity != expected_identity
    {
        return Err("workspace daemon inspection identity mismatch".to_string());
    }
    Ok(Some(inspection))
}

fn dispatch_workspace_command_on(
    mut stream: TcpStream,
    registry: WorkspaceRegistry,
    workspace: PathBuf,
    kind: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let command = WorkspaceCommand {
        kind: kind.to_string(),
        payload,
    };
    let handshake = Handshake {
        protocol: WIRE_PROTOCOL.to_string(),
        token: registry.token,
        kind: "workspace-command".to_string(),
        command: Some(
            serde_json::to_string(&command)
                .map_err(|error| format!("workspace command encode failed: {error}"))?,
        ),
        client: None,
        workspace,
    };
    write_json_line(&mut stream, &handshake)?;
    match read_json_line(&mut BufReader::new(stream))? {
        WorkspaceCommandReply::Ok(value) => Ok(value),
        WorkspaceCommandReply::Error(detail) => Err(detail),
    }
}

pub(super) fn connect_workspace_control_client_impl(
    workspace: &Path,
    client: WorkspaceClientIdentity,
) -> Result<WorkspaceControlClient, String> {
    validate_client_identity(&client)?;
    let workspace = canonical_workspace_root(workspace)?;
    let (stream, registry) = connect_or_start(&workspace)?;
    let paths = ServicePaths::new(&ags_platform::runtime_home(), &workspace);
    finish_workspace_session(stream, &registry, &workspace, &paths, client)
}

pub(super) fn finish_workspace_session(
    mut stream: TcpStream,
    registry: &WorkspaceRegistry,
    workspace: &Path,
    paths: &ServicePaths,
    client: WorkspaceClientIdentity,
) -> Result<WorkspaceControlClient, String> {
    let handshake = Handshake {
        protocol: WIRE_PROTOCOL.to_string(),
        token: registry.token.clone(),
        kind: "session".to_string(),
        command: None,
        client: Some(client.clone()),
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
            || ready.authenticated_session.trim().is_empty()
            || !ags_platform::is_sha256(&ready.project_facts_hash)
            || ready.registry_key != registry.instance_key
        {
            return Err("workspace daemon handshake mismatch".to_string());
        }
        Ok(WorkspaceControlClient {
            writer: stream,
            reader: daemon_reader,
            context: WorkspaceSessionContext {
                canonical_workspace: workspace.to_path_buf(),
                workspace_service_identity: ready.daemon_nonce,
                workspace_identity: ready.instance_key,
                project_facts_hash: ready.project_facts_hash,
                registry_key: ready.registry_key,
                authenticated_session: ready.authenticated_session,
                connection_id: client.connection_id,
                host_id: client.host_id,
            },
        })
    })();
    if result.is_err() {
        reclaim_registry_after_failed_handshake(paths, registry);
    }
    result
}

fn validate_client_identity(client: &WorkspaceClientIdentity) -> Result<(), String> {
    let connection = client.connection_id.trim();
    let host = client.host_id.trim();
    if connection.is_empty()
        || host.is_empty()
        || connection.len() > 128
        || host.len() > 64
        || !connection
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || !host.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err("workspace client identity invalid".to_string());
    }
    Ok(())
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
            if connection_error_is_reportable(&error) {
                let _ = writeln!(
                    std::io::stderr(),
                    "[ags-mcp] workspace daemon connection failed: {error}"
                );
            }
        }
    });
    Ok(())
}

pub(super) fn connection_error_is_reportable(error: &str) -> bool {
    error != PEER_CLOSED_BEFORE_HANDSHAKE
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
    if handshake.protocol != WIRE_PROTOCOL
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
                instance_key: registry.instance_key.clone(),
                executable_hash: registry.executable_hash,
                process_start_identity: registry.process_start_identity,
                daemon_nonce: registry.daemon_nonce,
                authenticated_session: String::new(),
                project_facts_hash: state.project_facts_hash(),
                registry_key: registry.instance_key.clone(),
            },
        )?;
        shutdown.store(true, Ordering::Release);
        return Ok(());
    }
    if handshake.kind == "workspace-command" {
        let encoded = handshake
            .command
            .ok_or_else(|| "workspace command handshake has no payload".to_string())?;
        let command: WorkspaceCommand = serde_json::from_str(&encoded)
            .map_err(|error| format!("workspace command payload invalid: {error}"))?;
        let context = super::WorkspaceCommandContext {
            canonical_workspace: registry.workspace.clone(),
            workspace_service_identity: registry.daemon_nonce.clone(),
            authenticated_session: fresh_id("workspace-command-session", &registry.workspace),
        };
        let reply =
            match handler.run_workspace_command(&command.kind, command.payload, state, context) {
                Ok(value) => WorkspaceCommandReply::Ok(value),
                Err(detail) => WorkspaceCommandReply::Error(detail),
            };
        write_json_line(&mut writer, &reply)?;
        return Ok(());
    }
    if handshake.kind != "session" {
        return Err("unsupported workspace daemon handshake".to_string());
    }
    let client = handshake
        .client
        .ok_or_else(|| "workspace session client identity required".to_string())?;
    validate_client_identity(&client)?;
    let session_id = fresh_id("daemon-session", &registry.workspace);
    let startup_executable_hash = registry.executable_hash.clone();
    let project_facts_hash = state.project_facts_hash();
    write_json_line(
        &mut writer,
        &HandshakeResult {
            status: "ready".to_string(),
            workspace: registry.workspace.clone(),
            instance_key: registry.instance_key.clone(),
            executable_hash: registry.executable_hash,
            process_start_identity: registry.process_start_identity,
            daemon_nonce: registry.daemon_nonce.clone(),
            authenticated_session: session_id.clone(),
            project_facts_hash: project_facts_hash.clone(),
            registry_key: registry.instance_key.clone(),
        },
    )?;
    let registry_key = state.instance_key().to_string();
    handler.run(
        reader,
        writer,
        state,
        WorkspaceSessionContext {
            canonical_workspace: registry.workspace,
            workspace_service_identity: registry.daemon_nonce,
            workspace_identity: registry.instance_key,
            project_facts_hash,
            registry_key,
            authenticated_session: session_id,
            connection_id: client.connection_id,
            host_id: client.host_id,
        },
        startup_executable_hash,
    );
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
    let mut buffer = Vec::with_capacity(8 * 1024);
    let line = read_workspace_wire_frame(reader, &mut buffer)?
        .ok_or_else(|| PEER_CLOSED_BEFORE_HANDSHAKE.to_string())?;
    serde_json::from_str(&line).map_err(|error| format!("workspace wire invalid: {error}"))
}

pub fn read_workspace_wire_frame<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<String>, String> {
    buffer.clear();
    let mut saw_input = false;
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("workspace wire read failed: {error}"))?;
        if available.is_empty() {
            if !saw_input {
                return Ok(None);
            }
            break;
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content = newline.map_or(available, |index| &available[..index]);
        if buffer.len().saturating_add(content.len()) > MAX_WORKSPACE_WIRE_FRAME_BYTES {
            buffer.clear();
            reader.consume(consumed);
            return Err(
                "workspace_wire_frame_too_large: workspace wire frame exceeds 1 MiB".to_string(),
            );
        }
        buffer.extend_from_slice(content);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if buffer.last() == Some(&b'\r') {
        buffer.pop();
    }
    String::from_utf8(std::mem::take(buffer))
        .map(Some)
        .map_err(|_| "workspace_wire_frame_invalid_utf8: workspace wire frame is not UTF-8".into())
}
