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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Hash a file without exposing filesystem details to domain crates.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("file hash read failed for {}: {error}", path.display()))?;
    Ok(sha256(bytes))
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
    set_private_file(temp.path());
    temp.persist(path)
        .map_err(|error| format!("atomic replace failed: {}", error.error))?;
    set_private_file(path);
    Ok(())
}

fn set_private_file(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600));
    }
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
            command.args(["/D", "/C"]).arg(resolved);
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
