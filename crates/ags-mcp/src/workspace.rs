//! Workspace-scoped AGS daemon and thin stdio adapter.
//!
//! One daemon is keyed only by the canonical workspace path. MCP hosts are
//! clients of that daemon; each TCP connection receives an independent
//! governance session inside the daemon.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::server::{run_mcp_session, RuntimeProcessIdentity};
use crate::tools::{CapabilityCatalogSource, PreflightBinding};

const REGISTRY_SCHEMA: &str = "0.3.0-workspace-service";
const WIRE_SCHEMA: &str = "ags-workspace-service/1";
const DEFAULT_IDLE_MS: u64 = 30 * 60 * 1000;
const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceRegistry {
    schema_version: String,
    workspace: PathBuf,
    instance_key: String,
    endpoint: String,
    token: String,
    pid: u32,
    executable_hash: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handshake {
    protocol: String,
    token: String,
    kind: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    command: Option<String>,
    workspace: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct HandshakeResult {
    status: String,
    workspace: PathBuf,
    instance_key: String,
    executable_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceCapabilityBundle {
    schema_version: String,
    workspace: PathBuf,
    snapshots: HashMap<String, skill_resolver::HostCapabilitySnapshot>,
}

struct SessionActivity {
    active_sessions: Arc<AtomicUsize>,
    last_activity: Arc<AtomicU64>,
}

impl SessionActivity {
    fn begin(active_sessions: Arc<AtomicUsize>, last_activity: Arc<AtomicU64>) -> Self {
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

/// Shared state owned by the unique daemon for one canonical workspace.
#[derive(Debug)]
pub(crate) struct WorkspaceState {
    root: PathBuf,
    instance_key: String,
    runtime_home: PathBuf,
    enforce_root: bool,
    snapshots: RwLock<HashMap<String, skill_resolver::HostCapabilitySnapshot>>,
}

impl WorkspaceState {
    pub(crate) fn new(root: PathBuf, runtime_home: PathBuf) -> Self {
        let instance_key = workspace_key(&root);
        let snapshots = load_capability_bundle(&runtime_home, &root);
        Self {
            root,
            instance_key,
            runtime_home,
            enforce_root: true,
            snapshots: RwLock::new(snapshots),
        }
    }

    #[cfg(test)]
    pub(crate) fn standalone() -> Arc<Self> {
        let root = canonical_workspace_root(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        )
        .unwrap_or_else(|_| PathBuf::from("."));
        Arc::new(Self {
            instance_key: workspace_key(&root),
            root,
            runtime_home: skill_resolver::locate_runtime_home(),
            enforce_root: false,
            snapshots: RwLock::new(HashMap::new()),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn instance_key(&self) -> &str {
        &self.instance_key
    }

    pub(crate) fn is_daemon_owned(&self) -> bool {
        self.enforce_root
    }

    pub(crate) fn target_matches(&self, target: &Path) -> bool {
        !self.enforce_root
            || canonical_workspace_root(target).is_ok_and(|target| target == self.root)
    }

    /// Load and validate the current host catalog through the workspace daemon,
    /// then atomically replace the daemon-owned snapshot view.
    pub(crate) fn read_catalog(
        &self,
        binding: &PreflightBinding,
    ) -> Result<skill_resolver::HostCapabilitySnapshot, String> {
        self.load_validated_catalog(binding)
            .map(|(snapshot, _)| snapshot)
    }

    fn load_validated_catalog(
        &self,
        binding: &PreflightBinding,
    ) -> Result<
        (
            skill_resolver::HostCapabilitySnapshot,
            skill_resolver::ActiveSkillTable,
        ),
        String,
    > {
        let target = canonical_workspace_root(&binding.target)?;
        if self.enforce_root && target != self.root {
            return Err(format!(
                "workspace_target_mismatch: service={} requested={}",
                self.root.display(),
                target.display()
            ));
        }
        let authority = skill_resolver::resolve_capability_authority_root(
            &binding.target,
            &self.runtime_home,
            std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
        )
        .map_err(|error| error.to_string())?;
        let expected = skill_resolver::build_capability_snapshot_with_roots(
            &authority,
            &binding.host,
            &self.runtime_home,
            &binding.host_home,
        )
        .map_err(|_| "skill_snapshot_stale".to_string())?;

        let mut snapshots = self
            .snapshots
            .write()
            .map_err(|_| "workspace_snapshot_lock_poisoned".to_string())?;
        let cached_is_current = snapshots.get(&binding.host).is_some_and(|cached| {
            cached
                .validate(
                    &binding.host,
                    &expected.registry_hash,
                    &expected.overlay_hash,
                    &expected.runtime_hash,
                )
                .is_ok()
                && cached.catalog_hash == expected.catalog_hash
                && cached.active_table_hash == expected.active_table_hash
                && cached.snapshot_hash == expected.snapshot_hash
        });
        if cached_is_current {
            let table = skill_resolver::ActiveSkillTable::new(
                expected.host.clone(),
                expected.snapshot_hash.clone(),
                expected.active_skills.clone(),
            )
            .map_err(|_| "skill_snapshot_stale".to_string())?;
            return Ok((expected, table));
        }

        let (snapshot, table) = skill_resolver::load_validated_snapshot_with_roots(
            &authority,
            &self.runtime_home,
            &binding.host,
            &binding.host_home,
        )
        .map_err(|_| "skill_snapshot_stale".to_string())?;
        let changed = snapshots
            .get(&binding.host)
            .is_none_or(|current| current.snapshot_hash != snapshot.snapshot_hash);
        if changed {
            snapshots.insert(binding.host.clone(), snapshot.clone());
            self.persist_capability_bundle(&snapshots)?;
        }
        Ok((snapshot, table))
    }

    fn persist_capability_bundle(
        &self,
        snapshots: &HashMap<String, skill_resolver::HostCapabilitySnapshot>,
    ) -> Result<(), String> {
        let paths = ServicePaths::new(&self.runtime_home, &self.root)?;
        ensure_private_dir(&paths.dir)?;
        let bundle = WorkspaceCapabilityBundle {
            schema_version: "0.3.0-workspace-capabilities".to_string(),
            workspace: self.root.clone(),
            snapshots: snapshots.clone(),
        };
        atomic_write_json(&paths.capabilities, &bundle)
    }
}

impl CapabilityCatalogSource for WorkspaceState {
    fn capability_reference(&self, target: &Path, host: &str) -> serde_json::Value {
        let binding = PreflightBinding {
            host: host.to_string(),
            target: target.to_path_buf(),
            host_home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        match self.read_catalog(&binding) {
            Ok(snapshot) => serde_json::json!({
                "uri": crate::tools::CURRENT_HOST_CAPABILITIES_URI,
                "status": "ready",
                "snapshot_hash": snapshot.snapshot_hash,
                "refresh_required": false
            }),
            Err(_) => serde_json::json!({
                "uri": crate::tools::CURRENT_HOST_CAPABILITIES_URI,
                "status": "snapshot_stale",
                "snapshot_hash": null,
                "refresh_required": true,
                "refresh": {
                    "argv": [
                        "ags",
                        "capability",
                        "snapshot",
                        "--host",
                        host,
                        "--target",
                        target.to_string_lossy(),
                        "--write"
                    ],
                    "requires_repreflight": true
                }
            }),
        }
    }

    fn load_validated_snapshot(
        &self,
        binding: &PreflightBinding,
    ) -> Result<
        (
            skill_resolver::HostCapabilitySnapshot,
            skill_resolver::ActiveSkillTable,
        ),
        String,
    > {
        self.load_validated_catalog(binding)
    }
}

fn load_capability_bundle(
    runtime_home: &Path,
    workspace: &Path,
) -> HashMap<String, skill_resolver::HostCapabilitySnapshot> {
    let Ok(paths) = ServicePaths::new(runtime_home, workspace) else {
        return HashMap::new();
    };
    let Ok(bytes) = fs::read(paths.capabilities) else {
        return HashMap::new();
    };
    let Ok(bundle) = serde_json::from_slice::<WorkspaceCapabilityBundle>(&bytes) else {
        return HashMap::new();
    };
    if bundle.schema_version != "0.3.0-workspace-capabilities" || bundle.workspace != workspace {
        return HashMap::new();
    }
    bundle.snapshots
}

#[derive(Debug)]
struct ServicePaths {
    dir: PathBuf,
    registry: PathBuf,
    lock: PathBuf,
    capabilities: PathBuf,
    diagnostics: PathBuf,
}

struct StartLock {
    file: fs::File,
    path: PathBuf,
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl ServicePaths {
    fn new(runtime_home: &Path, workspace: &Path) -> Result<Self, String> {
        let dir = runtime_home.join("workspace-services");
        let key = workspace_key(workspace);
        Ok(Self {
            registry: dir.join(format!("{key}.json")),
            lock: dir.join(format!("{key}.lock")),
            capabilities: dir.join(format!("{key}.capabilities.json")),
            diagnostics: dir.join(format!("{key}.log")),
            dir,
        })
    }
}

pub fn run_stdio_adapter() -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| format!("current_dir failed: {error}"))?;
    let workspace = canonical_workspace_root(&cwd)?;
    let service_paths = ServicePaths::new(&skill_resolver::locate_runtime_home(), &workspace)?;
    let (mut stream, registry) = connect_or_start(&workspace)?;
    let session_id = fresh_id("session", &workspace);
    let handshake = Handshake {
        protocol: WIRE_SCHEMA.to_string(),
        token: registry.token.clone(),
        kind: "session".to_string(),
        session_id: Some(session_id),
        command: None,
        workspace: workspace.clone(),
    };
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
    {
        return Err("workspace daemon handshake mismatch".to_string());
    }

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

pub fn run_workspace_daemon(workspace: &Path) -> Result<(), String> {
    let workspace = canonical_workspace_root(workspace)?;
    let runtime_home = skill_resolver::locate_runtime_home();
    let paths = ServicePaths::new(&runtime_home, &workspace)?;
    ensure_private_dir(&paths.dir)?;

    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|error| format!("bind failed: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("nonblocking failed: {error}"))?;
    let endpoint = listener
        .local_addr()
        .map_err(|error| format!("local_addr failed: {error}"))?
        .to_string();
    let executable_hash = current_executable_hash()?;
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: workspace.clone(),
        instance_key: workspace_key(&workspace),
        endpoint,
        token: fresh_id("token", &workspace),
        pid: std::process::id(),
        executable_hash: executable_hash.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    atomic_write_json(&paths.registry, &registry)?;

    let state = Arc::new(WorkspaceState::new(workspace.clone(), runtime_home));
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
                stream
                    .set_nonblocking(false)
                    .map_err(|error| format!("accepted stream blocking mode failed: {error}"))?;
                let activity = SessionActivity::begin(
                    Arc::clone(&active_sessions),
                    Arc::clone(&last_activity),
                );
                let registry = registry.clone();
                let state = Arc::clone(&state);
                let shutdown = Arc::clone(&shutdown);
                let _connection = std::thread::spawn(move || {
                    let _activity = activity;
                    if let Err(error) = handle_connection(stream, registry, state, shutdown) {
                        let _ = writeln!(
                            std::io::stderr(),
                            "[ags-mcp] workspace daemon connection failed: {error}"
                        );
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("workspace daemon accept failed: {error}")),
        }
        if active_sessions.load(Ordering::Acquire) == 0
            && now_millis().saturating_sub(last_activity.load(Ordering::Acquire)) >= idle_ms
        {
            break;
        }
    }

    remove_registry_if_owned(&paths.registry, &registry.token);
    Ok(())
}

fn handle_connection(
    stream: TcpStream,
    registry: WorkspaceRegistry,
    state: Arc<WorkspaceState>,
    shutdown: Arc<AtomicBool>,
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
            },
        )?;
        shutdown.store(true, Ordering::Release);
        return Ok(());
    }
    if handshake.kind != "session" {
        return Err("unsupported workspace daemon handshake".to_string());
    }
    let session_id = handshake
        .session_id
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "workspace session_id required".to_string())?;
    write_json_line(
        &mut writer,
        &HandshakeResult {
            status: "ready".to_string(),
            workspace: registry.workspace,
            instance_key: registry.instance_key,
            executable_hash: registry.executable_hash,
        },
    )?;
    run_mcp_session(
        reader,
        writer,
        state,
        session_id,
        RuntimeProcessIdentity::capture(),
    );
    Ok(())
}

fn connect_or_start(workspace: &Path) -> Result<(TcpStream, WorkspaceRegistry), String> {
    let runtime_home = skill_resolver::locate_runtime_home();
    let paths = ServicePaths::new(&runtime_home, workspace)?;
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
                }
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

fn connect_registered(
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
        Err(error) => {
            if process_is_alive(registry.pid) {
                Err(format!(
                    "workspace daemon pid {} is alive but endpoint {} is unreachable: {error}",
                    registry.pid, registry.endpoint
                ))
            } else {
                remove_registry_if_owned(&paths.registry, &registry.token);
                Ok(None)
            }
        }
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
    if registry.executable_hash == executable_hash {
        return Ok(());
    }
    request_shutdown(&registry)?;
    wait_for_shutdown(paths, &registry)
}

fn request_shutdown(registry: &WorkspaceRegistry) -> Result<(), String> {
    let mut stream = match TcpStream::connect(&registry.endpoint) {
        Ok(stream) => stream,
        Err(_error) if !process_is_alive(registry.pid) => return Ok(()),
        Err(error) => {
            return Err(format!(
                "stale workspace daemon pid {} is alive but unreachable: {error}",
                registry.pid
            ))
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
    if result.status != "stopping" {
        return Err("stale workspace daemon refused shutdown".to_string());
    }
    Ok(())
}

fn wait_for_shutdown(paths: &ServicePaths, registry: &WorkspaceRegistry) -> Result<(), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    while process_is_alive(registry.pid) {
        if Instant::now() >= deadline {
            return Err("stale workspace daemon did not stop".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    remove_registry_if_owned(&paths.registry, &registry.token);
    Ok(())
}

fn read_registry(path: &Path) -> Result<Option<WorkspaceRegistry>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("workspace registry read failed: {error}")),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("workspace registry invalid: {error}"))
}

fn remove_registry_if_owned(path: &Path, token: &str) {
    if read_registry(path)
        .ok()
        .flatten()
        .is_some_and(|registry| registry.token == token)
    {
        let _ = fs::remove_file(path);
    }
}

fn reclaim_stale_lock(path: &Path) {
    let owner = fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    let abandoned_empty_lock = owner.is_none()
        && fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= START_TIMEOUT);
    if owner.is_some_and(|pid| !process_is_alive(pid)) || abandoned_empty_lock {
        let _ = fs::remove_file(path);
    }
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "workspace state path has no parent".to_string())?;
    ensure_private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("workspace state stage failed: {error}"))?;
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("JSON encode failed: {error}"))?;
    temp.write_all(&bytes)
        .and_then(|_| temp.flush())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|error| format!("workspace state write failed: {error}"))?;
    set_private_file(temp.path());
    temp.persist(path)
        .map_err(|error| format!("workspace state replace failed: {}", error.error))?;
    set_private_file(path);
    Ok(())
}

fn write_json_line<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| format!("workspace wire encode failed: {error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("workspace wire write failed: {error}"))
}

fn read_json_line<T: for<'de> Deserialize<'de>>(reader: &mut impl BufRead) -> Result<T, String> {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| format!("workspace wire read failed: {error}"))?;
    if read == 0 {
        return Err("workspace daemon closed during handshake".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("workspace wire invalid: {error}"))
}

fn canonical_workspace_root(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("workspace canonicalization failed: {error}"))?;
    for candidate in canonical.ancestors() {
        if candidate.join(".git").exists() {
            return Ok(candidate.to_path_buf());
        }
    }
    Ok(canonical)
}

fn workspace_key(workspace: &Path) -> String {
    let digest = Sha256::digest(workspace.to_string_lossy().as_bytes());
    format!("{digest:x}")
}

fn current_executable_hash() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("current_exe failed: {error}"))?;
    let bytes =
        fs::read(&executable).map_err(|error| format!("executable hash read failed: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn fresh_id(prefix: &str, workspace: &Path) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut random = [0_u8; 32];
    let random_status = getrandom::fill(&mut random)
        .map(|_| format!("{random:x?}"))
        .unwrap_or_else(|_| "os-random-unavailable".to_string());
    let basis = format!(
        "{prefix}\n{}\n{}\n{}\n{sequence}\n{random_status}",
        workspace.display(),
        std::process::id(),
        now_millis()
    );
    format!("{prefix}-{:x}", Sha256::digest(basis.as_bytes()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
            })
    }
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("workspace state mkdir failed: {error}"))?;
    if fs::symlink_metadata(path)
        .map_err(|error| format!("workspace state metadata failed: {error}"))?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "workspace state directory must not be a symlink: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("workspace state chmod failed: {error}"))?;
    }
    Ok(())
}

fn set_private_file(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(_path, fs::Permissions::from_mode(0o600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_identity_is_only_the_canonical_workspace_path() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let child = workspace.join("nested");
        let sibling = root.path().join("other");
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(sibling.join(".git")).unwrap();

        let canonical = canonical_workspace_root(&child).unwrap();
        let state = WorkspaceState::new(canonical.clone(), root.path().join("runtime"));
        assert_eq!(state.instance_key(), workspace_key(&canonical));
        assert!(state.target_matches(&child));
        assert!(!state.target_matches(&sibling));
    }

    #[test]
    fn abandoned_start_lock_is_reclaimed() {
        let root = tempfile::tempdir().unwrap();
        let lock = root.path().join("workspace.lock");
        fs::write(&lock, u32::MAX.to_string()).unwrap();
        reclaim_stale_lock(&lock);
        assert!(!lock.exists());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_state_directory_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let link = root.path().join("link");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &link).unwrap();
        assert!(ensure_private_dir(&link)
            .unwrap_err()
            .contains("must not be a symlink"));
    }
}
