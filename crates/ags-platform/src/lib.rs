//! Cross-platform path and command-lookup helpers for the AGS suite.
//!
//! Centralizes the OS-specific assumptions the rest of the suite used to make
//! inline (reading `$HOME`, hardcoding `/tmp`, shelling out to `which`) so the
//! core CLI and libraries stay portable across Unix and Windows.
//!
//! Zero third-party dependencies — `std` only.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
        let mut joined = drive;
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
}
