use super::*;

/// OMP requires a JavaScript extension. All policy, parsing, hashing and
/// archival behavior remains in the Rust `ags host lifecycle` command.
pub(in crate::setup) const OMP_MEMORY_LIFECYCLE_JS: &str =
    include_str!("../../../../../scripts/ags-memory-lifecycle-omp.js");

/// Marker substrings used for idempotent, structure-preserving hook detection.
pub(super) const MEMORY_CAPTURE_MARKER: &str = "host lifecycle --event session-end";
pub(super) const MEMORY_START_MARKER: &str = "host lifecycle --event session-start";
pub(super) const EVOLVER_MARKER: &str = "evolver-session-end";
pub(super) const RAW_GUARD_MARKER: &str = "host lifecycle --event stop-guard";

pub(crate) fn omp_memory_lifecycle_path(home: &Path) -> PathBuf {
    home.join(".omp/agent/extensions/ags-memory-lifecycle.js")
}

pub(super) fn lifecycle_command(event: &str, host: &str) -> String {
    format!("ags host lifecycle --event {event} --host {host} --target .")
}

pub(in crate::setup) fn memory_capture_command(host: &str) -> String {
    lifecycle_command("session-end", host)
}

pub(in crate::setup) fn memory_start_command(host: &str) -> String {
    lifecycle_command("session-start", host)
}

pub(in crate::setup) fn raw_guard_command(host: &str) -> String {
    lifecycle_command("stop-guard", host)
}

/// Install-file entries for the capture scripts. Added to the base install plan
/// so they appear in `ags setup` dry-run output and are written by the standard
/// atomic install loop.
pub(in crate::setup) fn memory_script_install_files(home: &Path) -> Vec<InstallFile> {
    vec![InstallFile {
        path: omp_memory_lifecycle_path(home),
        description: "Thin OMP event adapter for the Rust AGS lifecycle".to_string(),
        content: OMP_MEMORY_LIFECYCLE_JS.to_string(),
        mode: Some(0o644),
    }]
}
