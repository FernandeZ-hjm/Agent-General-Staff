use std::fs::OpenOptions;
use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ags_platform::canonical_workspace_root;

use super::capability_snapshot::WorkspaceState;
use super::registry_ownership::{
    acquire_workspace_owner, current_executable_hash, ensure_private_dir, fresh_id, now_millis,
    publish_registry, read_registry, reclaim_stale_lock, registry_matches_process,
    remove_registry_if_owned, set_private_file, workspace_key, ServicePaths, StartLock,
    WorkspaceRegistry, REGISTRY_SCHEMA, START_TIMEOUT,
};
use super::transport_handshake::{
    read_json_line, spawn_workspace_connection, write_json_line, Handshake, HandshakeResult,
    WIRE_SCHEMA,
};
use super::WorkspaceSessionHandler;

const DEFAULT_IDLE_MS: u64 = 30 * 60 * 1000;
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn run_workspace_daemon_impl(
    workspace: &Path,
    handler: Arc<dyn WorkspaceSessionHandler>,
) -> Result<(), String> {
    let workspace = canonical_workspace_root(workspace)?;
    let runtime_home = ags_capability_governance::locate_runtime_home();
    let paths = ServicePaths::new(&runtime_home, &workspace);
    ensure_private_dir(&paths.dir)?;
    let owner = acquire_workspace_owner(&paths, &workspace)?;

    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|error| format!("bind failed: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("nonblocking failed: {error}"))?;
    let endpoint = listener
        .local_addr()
        .map_err(|error| format!("local_addr failed: {error}"))?
        .to_string();
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: workspace.clone(),
        instance_key: workspace_key(&workspace),
        endpoint,
        token: fresh_id("token", &workspace),
        pid: std::process::id(),
        executable_hash: owner.owner.executable_hash.clone(),
        process_start_identity: owner.owner.process_start_identity.clone(),
        daemon_nonce: owner.owner.daemon_nonce.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let state = Arc::new(WorkspaceState::new(workspace.clone(), runtime_home)?);
    publish_registry(&paths.registry, &registry)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let active_sessions = Arc::new(AtomicUsize::new(0));
    let last_activity = Arc::new(AtomicU64::new(now_millis()));
    let idle_ms = std::env::var("AGS_WORKSPACE_IDLE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_IDLE_MS);

    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                spawn_workspace_connection(
                    stream,
                    &registry,
                    &state,
                    &shutdown,
                    &handler,
                    &active_sessions,
                    &last_activity,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(error) => return Err(format!("workspace daemon accept failed: {error}")),
        }
        if active_sessions.load(Ordering::Acquire) == 0
            && now_millis().saturating_sub(last_activity.load(Ordering::Acquire)) >= idle_ms
        {
            match listener.accept() {
                Ok((stream, _)) => {
                    spawn_workspace_connection(
                        stream,
                        &registry,
                        &state,
                        &shutdown,
                        &handler,
                        &active_sessions,
                        &last_activity,
                    )?;
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("workspace daemon final accept failed: {error}")),
            }
        }
    }

    remove_registry_if_owned(&paths.registry, &registry.token);
    drop(owner);
    Ok(())
}

pub(super) fn connect_or_start(workspace: &Path) -> Result<(TcpStream, WorkspaceRegistry), String> {
    let runtime_home = ags_capability_governance::locate_runtime_home();
    let paths = ServicePaths::new(&runtime_home, workspace);
    ensure_private_dir(&paths.dir)?;
    let fast_connected = connect_registered(&paths, workspace)?;
    let executable_hash = current_executable_hash()?;

    if let Some((stream, registry)) = fast_connected {
        if registry.executable_hash == executable_hash {
            return Ok((stream, registry));
        }
        drop(stream);
    }

    let deadline = Instant::now() + START_TIMEOUT;
    let mut lock = loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.lock)
        {
            Ok(file) => {
                break StartLock {
                    file,
                    path: paths.lock.clone(),
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(connected) = connect_current(&paths, workspace, &executable_hash)? {
                    return Ok(connected);
                }
                reclaim_stale_lock(&paths.lock);
                if Instant::now() >= deadline {
                    return Err("workspace daemon start lock timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("workspace daemon lock failed: {error}")),
        }
    };
    set_private_file(&paths.lock);
    writeln!(lock.file, "{}", std::process::id())
        .and_then(|_| lock.file.flush())
        .map_err(|error| format!("workspace daemon lock write failed: {error}"))?;

    retire_mismatched_daemon(&paths, workspace, &executable_hash)?;
    if let Some(connected) = connect_current(&paths, workspace, &executable_hash)? {
        drop(lock);
        return Ok(connected);
    }

    let daemon_log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&paths.diagnostics)
        .map_err(|error| format!("workspace daemon log open failed: {error}"))?;
    set_private_file(&paths.diagnostics);
    let mut daemon = Command::new(
        std::env::current_exe()
            .map_err(|error| format!("cannot resolve current executable: {error}"))?,
    )
    .args(["mcp", "workspace-daemon", "--workspace"])
    .arg(workspace)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::from(daemon_log))
    .spawn()
    .map_err(|error| format!("workspace daemon spawn failed: {error}"))?;
    let _daemon_reaper = std::thread::spawn(move || {
        let _ = daemon.wait();
    });

    let started = loop {
        if let Some(connected) = connect_current(&paths, workspace, &executable_hash)? {
            break connected;
        }
        if Instant::now() >= deadline {
            drop(lock);
            return Err("workspace daemon did not become ready".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(lock);
    Ok(started)
}

fn connect_current(
    paths: &ServicePaths,
    workspace: &Path,
    executable_hash: &str,
) -> Result<Option<(TcpStream, WorkspaceRegistry)>, String> {
    let Some((stream, registry)) = connect_registered(paths, workspace)? else {
        return Ok(None);
    };
    if registry.executable_hash != executable_hash {
        drop(stream);
        return Ok(None);
    }
    Ok(Some((stream, registry)))
}

pub(super) fn connect_registered(
    paths: &ServicePaths,
    workspace: &Path,
) -> Result<Option<(TcpStream, WorkspaceRegistry)>, String> {
    let Some(registry) = read_registry(&paths.registry)? else {
        return Ok(None);
    };
    if registry.schema_version != REGISTRY_SCHEMA || registry.workspace != workspace {
        return Err("workspace daemon registry identity mismatch".to_string());
    }
    match TcpStream::connect(&registry.endpoint) {
        Ok(stream) => Ok(Some((stream, registry))),
        Err(_)
            if registry.process_start_identity.is_empty()
                || !registry_matches_process(&registry) =>
        {
            // A v0.3.1 registry has no process-start identity. A reused PID
            // therefore cannot prove that the old daemon still owns this
            // unreachable endpoint. New registries are reclaimed only after
            // their PID + process-start identity no longer matches.
            remove_registry_if_owned(&paths.registry, &registry.token);
            Ok(None)
        }
        Err(error) => Err(format!(
            "workspace daemon pid {} is alive but endpoint {} is unreachable: {error}",
            registry.pid, registry.endpoint
        )),
    }
}

pub(super) fn reclaim_registry_after_failed_handshake(
    paths: &ServicePaths,
    registry: &WorkspaceRegistry,
) {
    // A successful TCP connect only proves that some process owns the port.
    // Keep the fast path free of OS process probes, then reclaim the registry
    // only when a failed authenticated handshake also proves that its recorded
    // process identity is no longer authoritative.
    if registry.process_start_identity.is_empty() || !registry_matches_process(registry) {
        remove_registry_if_owned(&paths.registry, &registry.token);
    }
}

fn retire_mismatched_daemon(
    paths: &ServicePaths,
    workspace: &Path,
    executable_hash: &str,
) -> Result<(), String> {
    let Some(registry) = read_registry(&paths.registry)? else {
        return Ok(());
    };
    if registry.schema_version != REGISTRY_SCHEMA || registry.workspace != workspace {
        return Err("workspace daemon registry identity mismatch".to_string());
    }
    if !registry_matches_process(&registry) {
        remove_registry_if_owned(&paths.registry, &registry.token);
        return Ok(());
    }
    if registry.executable_hash == executable_hash {
        return Ok(());
    }
    if request_shutdown(&registry)? {
        wait_for_shutdown(paths, &registry)
    } else {
        remove_registry_if_owned(&paths.registry, &registry.token);
        Ok(())
    }
}

fn request_shutdown(registry: &WorkspaceRegistry) -> Result<bool, String> {
    let mut stream = match TcpStream::connect(&registry.endpoint) {
        Ok(stream) => stream,
        Err(_error)
            if !registry_matches_process(registry)
                || registry.process_start_identity.is_empty() =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(format!(
                "stale workspace daemon pid {} is alive but unreachable: {error}",
                registry.pid
            ));
        }
    };
    write_json_line(
        &mut stream,
        &Handshake {
            protocol: WIRE_SCHEMA.to_string(),
            token: registry.token.clone(),
            kind: "control".to_string(),
            session_id: None,
            command: Some("shutdown".to_string()),
            workspace: registry.workspace.clone(),
        },
    )?;
    let mut reader = BufReader::new(stream);
    let result: HandshakeResult = read_json_line(&mut reader)?;
    if result.status != "stopping"
        || result.workspace != registry.workspace
        || result.instance_key != registry.instance_key
        || result.executable_hash != registry.executable_hash
        || (!registry.process_start_identity.is_empty()
            && result.process_start_identity != registry.process_start_identity)
        || (!registry.daemon_nonce.is_empty() && result.daemon_nonce != registry.daemon_nonce)
    {
        return Err("stale workspace daemon refused shutdown".to_string());
    }
    Ok(true)
}

fn wait_for_shutdown(paths: &ServicePaths, registry: &WorkspaceRegistry) -> Result<(), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    while registry_matches_process(registry) {
        if Instant::now() >= deadline {
            return Err("stale workspace daemon did not stop".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    remove_registry_if_owned(&paths.registry, &registry.token);
    Ok(())
}
