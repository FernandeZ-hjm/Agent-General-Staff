use super::*;
// ── Project memory lifecycle closure ────────────────────────────────────────
//
// A project is "onboarded with a full memory closure" only when the REQUESTED
// host can READ on session start, WRITE/close on the host's terminal lifecycle
// event, ARCHIVE into task-archive, and execute the installed bridge scripts.
// The host is part of this interface: Claude evidence must never satisfy Codex
// or OMP. `ags init` owns the project store; `ags agents govern --apply` owns
// native host adapters; doctor and preflight consume this same computation.

pub(super) const MEMORY_START_MARKER: &str = "context-memory-start";
pub(super) const MEMORY_CAPTURE_MARKER: &str = "claude-stop-memory-capture";
pub(super) const COMMON_MEMORY_SCRIPTS: &[&str] = &[
    "context-memory.sh",
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
];
pub(super) const CLAUDE_MEMORY_SCRIPTS: &[&str] = &[
    "context-memory.sh",
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
    "raw-tool-call-stop-guard.js",
];
pub(super) const OMP_MEMORY_EXTENSION: &str = "ags-memory-lifecycle.js";

/// Read / write / archive / verify closure state for a project's memory store.
///
/// `status` is a coarse one-word verdict; the booleans expose the underlying
/// signals so callers can explain exactly which leg of the closure is missing:
///
/// - `full`       — files present, read + write hooks wired, backing scripts installed.
/// - `read-only`  — start injection wired but Stop capture missing.
/// - `write-only` — Stop capture wired but start injection missing.
/// - `files-only` — memory files exist but no hooks are wired (the silent gap
///   `ags init` historically left: onboarded yet unable to read/write memory).
/// - `unbacked`   — a hook is wired but the host capture scripts it invokes are
///   not installed, so the chain is a half-wired no-op (run `ags setup`).
/// - `absent`     — no memory store and no hooks (project not onboarded).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryLifecycle {
    /// Normalized host whose native lifecycle was inspected.
    pub host: String,
    /// Native adapter used for this host, or `unsupported`.
    pub adapter: String,
    /// Whether AGS ships a memory lifecycle adapter for this host.
    pub adapter_supported: bool,
    pub status: String,
    /// capsule + task-memory files exist (the readable project memory).
    pub files_present: bool,
    /// `task-archive/` directory exists (capture target).
    pub archive_ready: bool,
    /// SessionStart pipeline wires `context-memory-start` (can READ on start).
    pub read_wired: bool,
    /// Native host close pipeline wires the compatibility-named
    /// `claude-stop-memory-capture` bridge (can WRITE on close).
    pub write_wired: bool,
    /// Host capture scripts installed under `~/.agents/scripts` (hooks are backed).
    pub scripts_present: bool,
    /// Human one-line summary of the closure state.
    pub summary: String,
}

/// Derive the coarse `status` verdict from the underlying closure signals.
pub(super) fn derive_memory_status(
    adapter_supported: bool,
    files_present: bool,
    archive_ready: bool,
    read_wired: bool,
    write_wired: bool,
    scripts_present: bool,
) -> &'static str {
    if !adapter_supported {
        "unsupported"
    } else if !files_present && !read_wired && !write_wired {
        "absent"
    } else if (read_wired || write_wired) && !scripts_present {
        // A hook is wired but the scripts it shells out to are missing: the
        // chain runs nothing. Surface this as broken, not as a partial success.
        "unbacked"
    } else if files_present && archive_ready && read_wired && write_wired && scripts_present {
        "full"
    } else if read_wired && !write_wired {
        "read-only"
    } else if write_wired && !read_wired {
        "write-only"
    } else {
        // Files present without any hook (the historical `ags init` gap), or
        // the rare hooks-without-files case — either way not a full closure.
        "files-only"
    }
}

/// Resolve a project's memory slug from its profile, falling back to the
/// directory name (mirrors `detect_project`'s slug resolution, standalone so the
/// lifecycle computation does not require a full project detection pass).
pub(super) fn resolve_project_slug(target: &Path) -> String {
    if let Some(slug) = extract_profile_slug(target) {
        return slug;
    }
    slug_from_path(target)
}

/// Extract `project.slug` from `config/agent-project-profile.yaml`.
/// Only matches an indented `slug:` line (under the `project:` section) and
/// strips YAML inline comments (`# …`). Returns `None` on missing/empty.
#[doc(hidden)]
pub fn extract_profile_slug(target: &Path) -> Option<String> {
    let profile = target.join("config/agent-project-profile.yaml");
    let content = std::fs::read_to_string(&profile).ok()?;
    let mut in_project = false;
    for line in content.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_project = line.trim().starts_with("project:");
            continue;
        }
        if !in_project {
            continue;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("slug:") {
            let value = rest.split('#').next().unwrap_or("");
            let slug = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !slug.is_empty() {
                return Some(slug.to_string());
            }
        }
    }
    None
}

fn slug_from_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Collect hook command strings for one `.claude/settings.json` event, tolerating
/// both nested `{hooks:[{command}]}` and flat `{command}` group forms. Returns an
/// empty vec when the file is missing, unreadable, invalid JSON, or has no event.
pub(super) fn settings_event_commands(settings_path: &Path, event: &str) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(settings_path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let mut cmds = Vec::new();
    if let Some(arr) = parsed
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|e| e.as_array())
    {
        for group in arr {
            if let Some(inner) = group.get("hooks").and_then(|h| h.as_array()) {
                for h in inner {
                    if let Some(c) = h.get("command").and_then(|c| c.as_str()) {
                        cmds.push(c.to_string());
                    }
                }
            }
            if let Some(c) = group.get("command").and_then(|c| c.as_str()) {
                cmds.push(c.to_string());
            }
        }
    }
    cmds
}

pub(super) fn capture_scripts_present(home: &Path, required: &[&str]) -> bool {
    let dir = home.join(".agents/scripts");
    required.iter().all(|n| dir.join(n).is_file())
}

/// Backward-compatible Claude Code lifecycle query. New host-aware callers
/// should use [`compute_memory_lifecycle_for_host`].
pub fn compute_memory_lifecycle(target: &Path) -> MemoryLifecycle {
    let home = ags_platform::home_dir_or_temp();
    compute_memory_lifecycle_at_for_host(target, &home, &AgentType::ClaudeCode)
}

/// Backward-compatible Claude Code test seam.
pub fn compute_memory_lifecycle_at(target: &Path, home: &Path) -> MemoryLifecycle {
    compute_memory_lifecycle_at_for_host(target, home, &AgentType::ClaudeCode)
}

/// Compute the closure for the exact host requested by preflight.
pub fn compute_memory_lifecycle_for_host(target: &Path, agent_type: &AgentType) -> MemoryLifecycle {
    let home = ags_platform::home_dir_or_temp();
    compute_memory_lifecycle_at_for_host(target, &home, agent_type)
}

pub(super) fn host_hook_commands(paths: &[PathBuf], event: &str) -> Vec<String> {
    paths
        .iter()
        .flat_map(|path| settings_event_commands(path, event))
        .collect()
}

pub(super) fn omp_extension_wired(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| {
        std::fs::read_to_string(path).is_ok_and(|body| {
            body.contains("session_start")
                && body.contains("systemPromptAppend")
                && body.contains("agent_settled")
                && body.contains("session_shutdown")
                && body.contains(MEMORY_START_MARKER)
                && body.contains(MEMORY_CAPTURE_MARKER)
        })
    })
}

/// Testable host-specific lifecycle core. `home` redirects all machine-local
/// state so tests never inspect or mutate the operator's real host configs.
pub fn compute_memory_lifecycle_at_for_host(
    target: &Path,
    home: &Path,
    agent_type: &AgentType,
) -> MemoryLifecycle {
    let slug = resolve_project_slug(target);
    let mem_dir = home.join(".agents/memory/projects").join(&slug);
    let files_present =
        mem_dir.join("context-capsule.md").is_file() && mem_dir.join("task-memory.md").is_file();
    let archive_ready = mem_dir.join("task-archive").is_dir();

    let normalized_host = match agent_type.as_str() {
        "oh-my-pi" => "omp",
        other => other,
    };
    let (adapter, adapter_supported, read_wired, write_wired, scripts_present) =
        match normalized_host {
            "claude-code" => {
                let paths = [
                    home.join(".claude/settings.json"),
                    target.join(".claude/settings.json"),
                ];
                let read = host_hook_commands(&paths, "SessionStart")
                    .iter()
                    .any(|c| c.contains(MEMORY_START_MARKER));
                let write = host_hook_commands(&paths, "Stop")
                    .iter()
                    .any(|c| c.contains(MEMORY_CAPTURE_MARKER));
                (
                    "claude-command-hooks",
                    true,
                    read,
                    write,
                    capture_scripts_present(home, CLAUDE_MEMORY_SCRIPTS),
                )
            }
            "codex" => {
                let paths = [
                    home.join(".codex/hooks.json"),
                    target.join(".codex/hooks.json"),
                ];
                let read = host_hook_commands(&paths, "SessionStart")
                    .iter()
                    .any(|c| c.contains(MEMORY_START_MARKER));
                let write = host_hook_commands(&paths, "SessionEnd")
                    .iter()
                    .any(|c| c.contains(MEMORY_CAPTURE_MARKER));
                (
                    "codex-command-hooks",
                    true,
                    read,
                    write,
                    capture_scripts_present(home, COMMON_MEMORY_SCRIPTS),
                )
            }
            "omp" => {
                let global = home
                    .join(".omp/agent/extensions")
                    .join(OMP_MEMORY_EXTENSION);
                let project = target.join(".omp/extensions").join(OMP_MEMORY_EXTENSION);
                let wired = omp_extension_wired(&[global, project]);
                (
                    "omp-extension",
                    true,
                    wired,
                    wired,
                    capture_scripts_present(home, COMMON_MEMORY_SCRIPTS) && wired,
                )
            }
            _ => ("unsupported", false, false, false, false),
        };

    let status = derive_memory_status(
        adapter_supported,
        files_present,
        archive_ready,
        read_wired,
        write_wired,
        scripts_present,
    );
    let summary = match status {
        "full" => format!("{normalized_host} memory closure complete: native start + close lifecycle wired and backed; files + archive present"),
        "read-only" => format!("{normalized_host} can read project memory on start, but its native close capture is not wired"),
        "write-only" => format!("{normalized_host} closes/captures memory, but its native start injection is not wired"),
        "files-only" => format!("project memory files exist, but {normalized_host} native read/write hooks are not wired — run `ags agents govern --agent {normalized_host} --apply`"),
        "unbacked" => format!("{normalized_host} lifecycle is wired but AGS bridge scripts are missing — run `ags setup --yes --force`, then govern the host"),
        "unsupported" => format!("AGS has no native memory lifecycle adapter for host `{normalized_host}`; closure cannot be claimed"),
        _ => format!("no project memory store or {normalized_host} native lifecycle wiring"),
    };

    MemoryLifecycle {
        host: normalized_host.to_string(),
        adapter: adapter.to_string(),
        adapter_supported,
        status: status.to_string(),
        files_present,
        archive_ready,
        read_wired,
        write_wired,
        scripts_present,
        summary,
    }
}
