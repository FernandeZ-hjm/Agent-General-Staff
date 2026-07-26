use super::*;

/// Canonical capture script bodies, embedded so the installed `ags` binary is
/// self-contained (no dependency on the suite checkout at install time).
pub(in crate::setup) const CONTEXT_MEMORY_SH: &str =
    include_str!("../../../../../scripts/context-memory.sh");
pub(in crate::setup) const CONTEXT_MEMORY_START_PY: &str =
    include_str!("../../../../../scripts/context-memory-start.py");
pub(in crate::setup) const CLAUDE_STOP_MEMORY_CAPTURE_PY: &str =
    include_str!("../../../../../scripts/claude-stop-memory-capture.py");
pub(in crate::setup) const RAW_TOOL_CALL_STOP_GUARD_JS: &str =
    include_str!("../../../../../scripts/raw-tool-call-stop-guard.js");
pub(in crate::setup) const OMP_MEMORY_LIFECYCLE_JS: &str =
    include_str!("../../../../../scripts/ags-memory-lifecycle-omp.js");

/// Marker substrings used for idempotent, structure-preserving hook detection.
pub(super) const MEMORY_CAPTURE_MARKER: &str = "claude-stop-memory-capture";
pub(super) const MEMORY_START_MARKER: &str = "context-memory-start";
pub(super) const RAW_GUARD_MARKER: &str = "raw-tool-call-stop-guard";

/// Host directory the capture scripts are installed into (fork decision:
/// `~/.agents/scripts/`, matching the layout the capture bridge already
/// resolves and the existing machine state).
pub(in crate::setup) fn host_scripts_dir(home: &Path) -> PathBuf {
    home.join(".agents").join("scripts")
}
pub(in crate::setup) fn context_memory_script_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("context-memory.sh")
}
pub(in crate::setup) fn context_memory_start_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("context-memory-start.py")
}
pub(in crate::setup) fn claude_stop_memory_capture_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("claude-stop-memory-capture.py")
}
pub(in crate::setup) fn raw_tool_call_stop_guard_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("raw-tool-call-stop-guard.js")
}
pub(crate) fn omp_memory_lifecycle_path(home: &Path) -> PathBuf {
    home.join(".omp/agent/extensions/ags-memory-lifecycle.js")
}

/// Stop-hook command that runs the project-memory capture bridge. Uses `$HOME`
/// (shell-expanded by the host) so the tracked workspace `settings.json` stays
/// machine-independent.
pub(in crate::setup) fn memory_capture_command() -> String {
    "python3 \"$HOME/.agents/scripts/claude-stop-memory-capture.py\"".to_string()
}
pub(in crate::setup) fn memory_start_command() -> String {
    "python3 \"$HOME/.agents/scripts/context-memory-start.py\"".to_string()
}
pub(crate) fn codex_memory_capture_command() -> String {
    "AGS_MEMORY_HOST=codex python3 \"$HOME/.agents/scripts/claude-stop-memory-capture.py\""
        .to_string()
}
pub(in crate::setup) fn raw_guard_command() -> String {
    "node \"$HOME/.agents/scripts/raw-tool-call-stop-guard.js\"".to_string()
}

/// Install-file entries for the capture scripts. Added to the base install plan
/// so they appear in `ags setup` dry-run output and are written by the standard
/// install loop (which backs up changed files before overwriting).
pub(in crate::setup) fn memory_script_install_files(home: &Path) -> Vec<InstallFile> {
    vec![
        InstallFile {
            path: raw_tool_call_stop_guard_path(home),
            description: "AGS Claude Stop raw tool-call guard".to_string(),
            content: RAW_TOOL_CALL_STOP_GUARD_JS.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: context_memory_script_path(home),
            description: "AGS context-memory product script (status/init/capture)".to_string(),
            content: CONTEXT_MEMORY_SH.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: context_memory_start_path(home),
            description: "AGS Claude SessionStart project-memory injection hook".to_string(),
            content: CONTEXT_MEMORY_START_PY.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: claude_stop_memory_capture_path(home),
            description: "AGS host-neutral project-memory close/capture bridge".to_string(),
            content: CLAUDE_STOP_MEMORY_CAPTURE_PY.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: omp_memory_lifecycle_path(home),
            description: "AGS OMP native project-memory lifecycle extension".to_string(),
            content: OMP_MEMORY_LIFECYCLE_JS.to_string(),
            mode: Some(0o644),
        },
    ]
}
