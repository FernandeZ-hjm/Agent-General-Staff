use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ags_platform::{atomic_write, executable_content_hash};

pub(super) const REGISTRY_SCHEMA: &str = "ags-workspace-registry/1";
pub(super) const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WorkspaceRegistry {
    pub(super) schema_version: String,
    pub(super) workspace: PathBuf,
    pub(super) instance_key: String,
    pub(super) endpoint: String,
    pub(super) token: String,
    pub(super) pid: u32,
    pub(super) executable_hash: String,
    #[serde(default)]
    pub(super) process_start_identity: String,
    #[serde(default)]
    pub(super) daemon_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WorkspaceOwner {
    pub(super) pid: u32,
    pub(super) token: String,
    pub(super) workspace: PathBuf,
    #[serde(default)]
    pub(super) executable_hash: String,
    #[serde(default)]
    pub(super) process_start_identity: String,
    #[serde(default)]
    pub(super) daemon_nonce: String,
}

#[derive(Debug)]
pub(super) struct ServicePaths {
    pub(super) dir: PathBuf,
    pub(super) registry: PathBuf,
    pub(super) lock: PathBuf,
    pub(super) owner: PathBuf,
    pub(super) diagnostics: PathBuf,
}

impl ServicePaths {
    pub(super) fn new(runtime_home: &Path, workspace: &Path) -> Self {
        let dir = runtime_home.join("workspace-services");
        let key = workspace_key(workspace);
        Self {
            registry: dir.join(format!("{key}.json")),
            lock: dir.join(format!("{key}.lock")),
            owner: dir.join(format!("{key}.owner")),
            diagnostics: dir.join(format!("{key}.log")),
            dir,
        }
    }
}

pub(super) struct StartLock {
    pub(super) file: fs::File,
    pub(super) path: PathBuf,
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
pub(super) struct WorkspaceOwnerGuard {
    path: PathBuf,
    token: String,
    pub(super) owner: WorkspaceOwner,
}

impl Drop for WorkspaceOwnerGuard {
    fn drop(&mut self) {
        if read_owner(&self.path)
            .ok()
            .flatten()
            .is_some_and(|owner| owner.token == self.token)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn acquire_workspace_owner(
    paths: &ServicePaths,
    workspace: &Path,
) -> Result<WorkspaceOwnerGuard, String> {
    let token = fresh_id("owner", workspace);
    for _ in 0..2 {
        let owner = WorkspaceOwner {
            pid: std::process::id(),
            token: token.clone(),
            workspace: workspace.to_path_buf(),
            executable_hash: current_executable_hash()?,
            process_start_identity: current_process_start_identity()
                .ok_or_else(|| "process start identity unavailable".to_string())?,
            daemon_nonce: fresh_id("daemon", workspace),
        };
        match publish_workspace_owner(paths, &owner, &token) {
            Ok(()) => {
                return Ok(WorkspaceOwnerGuard {
                    path: paths.owner.clone(),
                    token,
                    owner,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = read_owner(&paths.owner)?;
                if existing.as_ref().is_some_and(|owner| {
                    owner.workspace == workspace && owner_matches_process(owner)
                }) {
                    return Err("workspace daemon already active".to_string());
                }
                if let Some(existing) = existing {
                    if existing.workspace != workspace {
                        return Err("workspace daemon owner identity mismatch".to_string());
                    }
                }
                fs::remove_file(&paths.owner)
                    .map_err(|remove| format!("stale workspace owner reclaim failed: {remove}"))?;
            }
            Err(error) => return Err(format!("workspace daemon owner lock failed: {error}")),
        }
    }
    Err("workspace daemon owner lock unavailable".to_string())
}

fn publish_workspace_owner(
    paths: &ServicePaths,
    owner: &WorkspaceOwner,
    token: &str,
) -> Result<(), std::io::Error> {
    let candidate = paths.dir.join(format!(
        ".{}.owner-candidate",
        token.strip_prefix("owner-").unwrap_or(token)
    ));
    let publish = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)?;
        serde_json::to_writer(&mut file, owner).map_err(std::io::Error::other)?;
        file.flush()?;
        file.sync_all()?;
        set_private_file(&candidate);
        fs::hard_link(&candidate, &paths.owner)
    })();
    let _ = fs::remove_file(&candidate);
    publish
}

pub(super) fn read_owner(path: &Path) -> Result<Option<WorkspaceOwner>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("workspace owner read failed: {error}")),
    };
    // Atomic hard-link publication means an incomplete record cannot belong to
    // a live current daemon. Empty or truncated stale records are reclaimable.
    Ok(serde_json::from_slice(&bytes).ok())
}

pub(super) fn publish_registry(path: &Path, registry: &WorkspaceRegistry) -> Result<(), String> {
    if let Some(existing) = read_registry(path)? {
        if existing.token != registry.token && registry_matches_process(&existing) {
            return Err(format!(
                "workspace registry already owned by live pid {}",
                existing.pid
            ));
        }
        remove_registry_if_owned(path, &existing.token);
    }
    atomic_write_json(path, registry)
}

pub(super) fn read_registry(path: &Path) -> Result<Option<WorkspaceRegistry>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("workspace registry read failed: {error}")),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("workspace registry invalid: {error}"))
}

pub(super) fn remove_registry_if_owned(path: &Path, token: &str) {
    if read_registry(path)
        .ok()
        .flatten()
        .is_some_and(|registry| registry.token == token)
    {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn reclaim_stale_lock(path: &Path) {
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

pub(super) fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("JSON encode failed: {error}"))?;
    atomic_write(path, &bytes).map_err(|error| format!("workspace state {error}"))
}

pub(super) fn workspace_key(workspace: &Path) -> String {
    let digest = Sha256::digest(workspace.to_string_lossy().as_bytes());
    format!("{digest:x}")
}

pub(super) fn current_executable_hash() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("current_exe failed: {error}"))?;
    executable_content_hash(&executable)
}

pub(super) fn fresh_id(prefix: &str, workspace: &Path) -> String {
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

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(super) fn current_process_start_identity() -> Option<String> {
    process_start_identity(std::process::id())
}

pub(super) fn owner_matches_process(owner: &WorkspaceOwner) -> bool {
    if owner.process_start_identity.is_empty() {
        return false;
    }
    process_start_identity(owner.pid)
        .is_some_and(|identity| identity == owner.process_start_identity)
}

pub(super) fn registry_matches_process(registry: &WorkspaceRegistry) -> bool {
    if registry.process_start_identity.is_empty() {
        return false;
    }
    process_start_identity(registry.pid)
        .is_some_and(|identity| identity == registry.process_start_identity)
}

fn process_start_identity(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        if !process_is_alive(pid) {
            return None;
        }
        let output = Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let started = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!started.is_empty()).then(|| format!("ps-lstart:{started}"))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
        use windows_sys::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // SAFETY: OpenProcess returns either a null handle or an owned process
        // handle. Every non-null handle is closed below, and GetProcessTimes is
        // given valid pointers to initialized FILETIME storage.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let success =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        unsafe {
            CloseHandle(handle);
        }
        if success == 0 {
            return None;
        }
        let started =
            (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        Some(format!("filetime:{started}"))
    }
}

fn process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
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

pub(super) fn ensure_private_dir(path: &Path) -> Result<(), String> {
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

pub(super) fn set_private_file(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(_path, fs::Permissions::from_mode(0o600));
    }
}
