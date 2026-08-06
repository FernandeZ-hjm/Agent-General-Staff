//! Cross-platform path and command-lookup helpers for the AGS suite.
//!
//! Centralizes the OS-specific assumptions the rest of the suite used to make
//! inline (reading `$HOME`, hardcoding `/tmp`, shelling out to `which`) so the
//! core CLI and libraries stay portable across Unix and Windows.
//!
//! This crate is the only place where AGS domain crates should need to know
//! about operating-system path, file replacement, executable, or hashing
//! details.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;

use sha2::{Digest, Sha256};

/// Resolve the current user's home directory in a cross-platform way.
///
/// - Unix: `$HOME`.
/// - Windows: `%USERPROFILE%`, then `%HOMEDRIVE%%HOMEPATH%`, then `%APPDATA%`.
///
/// Returns `None` when the environment does not describe a home location, so
/// callers can pick an explicit fallback instead of silently substituting an
/// unrelated path (the old inline code fell back to `/tmp` or a hardcoded
/// machine-specific user directory).
pub fn home_dir() -> Option<PathBuf> {
    home_dir_impl()
}

#[cfg(windows)]
fn home_dir_impl() -> Option<PathBuf> {
    if let Some(p) = non_empty_var_os("USERPROFILE") {
        return Some(PathBuf::from(p));
    }
    if let (Some(drive), Some(path)) = (non_empty_var_os("HOMEDRIVE"), non_empty_var_os("HOMEPATH"))
    {
        let mut joined = OsString::from(drive);
        joined.push(path);
        return Some(PathBuf::from(joined));
    }
    non_empty_var_os("APPDATA").map(PathBuf::from)
}

#[cfg(not(windows))]
fn home_dir_impl() -> Option<PathBuf> {
    non_empty_var_os("HOME").map(PathBuf::from)
}

fn non_empty_var_os(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

/// Resolve the home directory, falling back to the OS temp dir when the
/// environment does not describe one. Keeps path construction deterministic
/// and free of hardcoded machine-specific fallbacks.
pub fn home_dir_or_temp() -> PathBuf {
    home_dir().unwrap_or_else(temp_root)
}

/// Cross-platform temporary-directory root (`std::env::temp_dir`).
pub fn temp_root() -> PathBuf {
    std::env::temp_dir()
}

/// Canonical machine-local AGS runtime root.
///
/// This is the only environment/default resolution used by domain modules.
/// Callers that need a child location use [`RuntimeLayout`] instead of
/// reconstructing path fragments or re-reading environment variables.
pub fn runtime_home() -> PathBuf {
    if let Some(path) = non_empty_var_os("AGS_RUNTIME_HOME") {
        return PathBuf::from(path);
    }
    if let Some(path) = non_empty_var_os("AGS_HOME") {
        return PathBuf::from(path);
    }
    runtime_home_at(&home_dir().unwrap_or_else(|| PathBuf::from(".")))
}

pub fn runtime_home_at(home: &Path) -> PathBuf {
    home.join(".ags").join("private-runtime")
}

/// Typed path projection for all machine-local AGS state.
///
/// Product/source manifests remain in the suite checkout. Installed
/// capabilities, immutable bodies, plans, receipts and host snapshots live
/// here and are owned by the stable runtime, so S can be the machine-effective
/// authority without becoming a source-code fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLayout {
    root: PathBuf,
}

impl RuntimeLayout {
    pub fn discover() -> Self {
        Self::new(runtime_home())
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn install_manifest(&self) -> PathBuf {
        self.root.join("install-manifest.json")
    }

    pub fn stable_capabilities(&self) -> PathBuf {
        self.root.join("stable-capabilities")
    }

    pub fn installed_skills(&self) -> PathBuf {
        self.stable_capabilities().join("installed-skills.json")
    }

    pub fn skill_bodies(&self) -> PathBuf {
        self.stable_capabilities().join("bodies")
    }

    pub fn capability_snapshots(&self) -> PathBuf {
        self.stable_capabilities().join("snapshots")
    }

    pub fn suite_projection_state(&self) -> PathBuf {
        self.stable_capabilities()
            .join("suite-skill-projection.json")
    }

    pub fn maintenance(&self) -> PathBuf {
        self.root.join("maintenance")
    }

    pub fn maintenance_lock(&self) -> PathBuf {
        self.maintenance().join("transaction.lock")
    }

    pub fn managed_projects(&self) -> PathBuf {
        self.root.join("managed-projects.yaml")
    }

    pub fn workspace_services(&self) -> PathBuf {
        self.root.join("workspace-services")
    }
}

/// Process-scoped exclusive lock shared by every maintenance subject. A
/// single lock prevents setup, Skill, snapshot and recovery transactions from
/// mutating the same runtime concurrently.
pub struct MaintenanceLock {
    path: PathBuf,
    token: String,
}

impl MaintenanceLock {
    pub fn acquire(runtime_home: &Path) -> Result<Self, String> {
        let path = RuntimeLayout::new(runtime_home).maintenance_lock();
        let parent = path
            .parent()
            .ok_or_else(|| "maintenance lock has no parent".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        let token = format!("pid:{}:{}", std::process::id(), unix_nanos());
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file
                        .write_all(token.as_bytes())
                        .and_then(|_| file.sync_all())
                    {
                        let _ = std::fs::remove_file(&path);
                        return Err(format!("cannot initialize maintenance lock: {error}"));
                    }
                    return Ok(Self { path, token });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_owner_is_stale(&path)? {
                        std::fs::remove_file(&path).map_err(|remove_error| {
                            format!("cannot clear stale lock {}: {remove_error}", path.display())
                        })?;
                        continue;
                    }
                    return Err("runtime maintenance is locked by another process".to_string());
                }
                Err(error) => {
                    return Err(format!("cannot acquire {}: {error}", path.display()));
                }
            }
        }
        Err("cannot acquire maintenance lock after stale-lock retry".to_string())
    }
}

impl Drop for MaintenanceLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|contents| contents == self.token) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn lock_owner_is_stale(path: &Path) -> Result<bool, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read maintenance lock {}: {error}", path.display()))?;
    let Some(pid) = contents
        .strip_prefix("pid:")
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return Ok(false);
    };
    if pid == std::process::id() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("cannot inspect maintenance lock owner: {error}"))?;
        Ok(!status.success())
    }
    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

/// Normalize an existing or not-yet-created path without requiring its final
/// component to exist. The nearest existing ancestor is canonicalized and the
/// missing suffix is restored. This replaces the three formerly independent
/// guard/source-root normalizers.
pub fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }

    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        if let Some(name) = existing.file_name() {
            missing.push(name.to_os_string());
        }
        let Some(parent) = existing.parent() else {
            return absolute;
        };
        existing = parent;
    }

    let mut normalized = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for component in missing.iter().rev() {
        normalized.push(component);
    }
    normalized
}

/// Resolve a path to the canonical workspace root.
///
/// A nested path belongs to the nearest ancestor containing `.git`; otherwise
/// the canonicalized path itself is the workspace. This identity deliberately
/// excludes the host so Codex, Claude Code, Cursor, and OMP share one service.
pub fn canonical_workspace_root(path: &Path) -> Result<PathBuf, String> {
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

/// Stable SHA-256 digest with the suite's canonical prefix.
pub fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes.as_ref()))
}

/// Stable SHA-256 digest without a scheme prefix.
///
/// Task-card and receipt identifiers historically use the bare hexadecimal
/// form, while capability authorities use [`sha256`]. Keeping both encodings
/// here prevents domain modules from depending on one another only for hash
/// formatting.
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_sha256_hex)
}

/// Accept full Git object ids for both SHA-1 and SHA-256 repositories.
pub fn is_git_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Hash a file without exposing filesystem details to domain crates.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("file hash read failed for {}: {error}", path.display()))?;
    Ok(sha256(bytes))
}

/// Copy a regular-file directory tree while rejecting symlinks and special
/// files at every level. Capability candidates and one-way state migration
/// share this boundary instead of maintaining subtly different walkers.
pub fn copy_regular_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| format!("cannot inspect {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "copy source must be a real directory: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    let mut entries = std::fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", from.display()))?;
        if kind.is_symlink() {
            return Err(format!("symlink_refused: {}", from.display()));
        }
        if kind.is_dir() {
            copy_regular_tree(&from, &to)?;
        } else if kind.is_file() {
            let bytes = std::fs::read(&from)
                .map_err(|error| format!("cannot read {}: {error}", from.display()))?;
            atomic_write(&to, &bytes)?;
        } else {
            return Err(format!("special_file_refused: {}", from.display()));
        }
    }
    Ok(())
}

/// Fast cryptographic digest for request-time executable integrity checks.
///
/// Unlike artifact hashes, this digest is recomputed over the complete file on
/// every governed request. BLAKE3 keeps that fail-closed check cheap enough to
/// remain on the request path without relying on filesystem metadata.
pub fn executable_content_hash(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "executable hash read failed for {}: {error}",
            path.display()
        )
    })?;
    Ok(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
}

/// Atomically replace a file with fully flushed bytes in the same directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "atomic write path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("atomic write mkdir failed: {error}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("atomic write stage failed: {error}"))?;
    temp.write_all(bytes)
        .and_then(|_| temp.flush())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|error| format!("atomic write failed: {error}"))?;
    set_private_file(temp.path())?;
    temp.persist(path)
        .map_err(|error| format!("atomic replace failed: {}", error.error))?;
    Ok(())
}

fn set_private_file(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("atomic write chmod failed: {error}"))?;
    }
    Ok(())
}

/// Look up an executable on `PATH`, returning the first match.
///
/// On Windows the lookup also tries the extensions listed in `%PATHEXT%`
/// (defaulting to `.COM;.EXE;.BAT;.CMD`), so `find_in_path("ags")` resolves
/// `ags.exe` / `ags.cmd` / `ags.bat`. This replaces shelling out to `which`,
/// which does not exist on native Windows.
pub fn find_in_path(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH");
    find_in_path_within(cmd, path_var.as_deref())
}

/// Build a native process command, resolving Windows PATHEXT launchers.
///
/// `CreateProcess` cannot execute `.cmd` or `.bat` files directly, so those
/// launchers use the system command interpreter. Callers append arguments to
/// the returned command exactly as they would for a native executable.
pub fn command_for_program(program: &str) -> Command {
    #[cfg(windows)]
    {
        let resolved = find_in_path(program).unwrap_or_else(|| PathBuf::from(program));
        let batch = resolved
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            });
        if batch {
            let mut command =
                Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
            command.args(["/D", "/C", "call"]).arg(resolved);
            return command;
        }
        Command::new(resolved)
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

/// Whether an executable named `cmd` is resolvable on `PATH`.
pub fn is_on_path(cmd: &str) -> bool {
    find_in_path(cmd).is_some()
}

fn find_in_path_within(cmd: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let candidates = path_candidate_names(cmd);
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in &candidates {
            let full = dir.join(name);
            if is_executable_file(&full) {
                return Some(full);
            }
        }
    }
    None
}

#[cfg(windows)]
fn path_candidate_names(cmd: &str) -> Vec<PathBuf> {
    let has_ext = Path::new(cmd).extension().is_some();
    let mut names = Vec::new();
    if has_ext {
        names.push(PathBuf::from(cmd));
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for ext in exts.split(';').filter(|e| !e.is_empty()) {
        names.push(PathBuf::from(format!("{cmd}{ext}")));
    }
    if !has_ext {
        names.push(PathBuf::from(cmd));
    }
    names
}

#[cfg(not(windows))]
fn path_candidate_names(cmd: &str) -> Vec<PathBuf> {
    vec![PathBuf::from(cmd)]
}

#[cfg(unix)]
fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(p: &Path) -> bool {
    // On Windows executability is governed by file extension (handled in
    // `path_candidate_names`); an existing regular file is sufficient here.
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_root_is_non_empty() {
        assert!(!temp_root().as_os_str().is_empty());
    }

    #[test]
    fn home_dir_or_temp_never_empty() {
        assert!(!home_dir_or_temp().as_os_str().is_empty());
    }

    #[test]
    fn runtime_layout_has_one_stable_machine_fact_root() {
        let layout = RuntimeLayout::new(PathBuf::from("runtime"));
        assert_eq!(
            layout.installed_skills(),
            PathBuf::from("runtime/stable-capabilities/installed-skills.json")
        );
        assert_eq!(
            layout.skill_bodies(),
            PathBuf::from("runtime/stable-capabilities/bodies")
        );
        assert_eq!(
            layout.capability_snapshots(),
            PathBuf::from("runtime/stable-capabilities/snapshots")
        );
        assert_eq!(
            layout.suite_projection_state(),
            PathBuf::from("runtime/stable-capabilities/suite-skill-projection.json")
        );
        assert_eq!(layout.maintenance(), PathBuf::from("runtime/maintenance"));
    }

    #[test]
    fn normalize_path_preserves_a_missing_suffix() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("missing/child");
        let expected = root.path().canonicalize().unwrap().join("missing/child");
        assert_eq!(normalize_path(&candidate), expected);
    }

    #[cfg(not(windows))]
    #[test]
    fn home_dir_reflects_home_env_on_unix() {
        if let Some(h) = non_empty_var_os("HOME") {
            assert_eq!(home_dir(), Some(PathBuf::from(h)));
        }
    }

    #[test]
    fn find_in_path_rejects_unknown_binary() {
        assert!(find_in_path("ags-definitely-not-a-real-binary-xyz-123").is_none());
    }

    #[test]
    fn find_in_path_within_handles_missing_path() {
        assert!(find_in_path_within("anything", None).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_within_locates_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_root().join(format!("ags-platform-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("ags-fake-tool");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let mut perm = std::fs::metadata(&bin).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&bin, perm).unwrap();

        let path_var = dir.as_os_str().to_os_string();
        let found = find_in_path_within("ags-fake-tool", Some(path_var.as_os_str()));

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, Some(bin));
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_within_skips_non_executable() {
        let dir = temp_root().join(format!("ags-platform-nonexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ags-not-exec");
        std::fs::write(&f, b"data\n").unwrap(); // regular file, no +x bit
        let path_var = dir.as_os_str().to_os_string();
        let found = find_in_path_within("ags-not-exec", Some(path_var.as_os_str()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found.is_none());
    }

    #[test]
    fn atomic_write_replaces_complete_content() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bundle.json");
        std::fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new-complete-bundle").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new-complete-bundle");
    }

    #[test]
    fn atomic_write_failure_does_not_disguise_a_directory_as_success() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("bundle.json");
        std::fs::create_dir_all(&destination).unwrap();
        let error = atomic_write(&destination, b"candidate").unwrap_err();
        assert!(error.contains("atomic replace failed"));
        assert!(destination.is_dir());
    }
}
