use super::*;
// ── Project memory lifecycle closure ────────────────────────────────────────
//
// A project is "onboarded with a full memory closure" only when the REQUESTED
// host can READ on session start, WRITE/close on the host's terminal lifecycle
// event, ARCHIVE into task-archive, and invoke the Rust lifecycle kernel.
// The host is part of this interface: one host's evidence must never satisfy
// Codex, Claude Code, Cursor, or OMP. `ags init` owns the project store;
// `ags agents govern --apply` owns
// native host adapters; doctor and preflight consume this same computation.

pub(super) const OMP_MEMORY_EXTENSION: &str = "ags-memory-lifecycle.js";

/// Read / write / archive / verify closure state for a project's memory store.
///
/// `status` is a coarse one-word verdict; the booleans expose the underlying
/// signals so callers can explain exactly which leg of the closure is missing:
///
/// - `full`       — files present and read + write hooks invoke the Rust kernel.
/// - `read-only`  — start injection wired but Stop capture missing.
/// - `write-only` — Stop capture wired but start injection missing.
/// - `files-only` — memory files exist but no hooks are wired (the silent gap
///   `ags init` historically left: onboarded yet unable to read/write memory).
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
    /// SessionStart pipeline invokes the Rust lifecycle kernel (can READ on start).
    pub read_wired: bool,
    /// Native host close pipeline invokes the Rust lifecycle kernel (can WRITE on close).
    pub write_wired: bool,
    /// Native stop/settled event invokes the Rust raw-tool-call guard.
    pub stop_guard_wired: bool,
    /// Every configured hook/extension delegates to the `ags host lifecycle` Rust kernel.
    pub kernel_backed: bool,
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
    stop_guard_wired: bool,
    kernel_backed: bool,
) -> &'static str {
    if !adapter_supported {
        "unsupported"
    } else if !files_present && !read_wired && !write_wired {
        "absent"
    } else if files_present
        && archive_ready
        && read_wired
        && write_wired
        && stop_guard_wired
        && kernel_backed
    {
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
pub fn resolve_project_slug(target: &Path) -> String {
    if let Some(slug) = extract_profile_slug(target) {
        return slug;
    }
    slug_from_path(target)
}

pub fn project_memory_dir_at(target: &Path, home: &Path) -> PathBuf {
    home.join(".agents/memory/projects")
        .join(resolve_project_slug(target))
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

fn command_matches(command: &str, event: &str, host: &str) -> bool {
    command.contains(&format!("host lifecycle --event {event} --host {host}"))
}

pub(super) fn omp_extension_wired(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| {
        std::fs::read_to_string(path).is_ok_and(|body| {
            body.contains("session_start")
                && body.contains("systemPromptAppend")
                && body.contains("agent_settled")
                && body.contains("session_shutdown")
                && body.contains("spawnSync")
                && body.contains("\"host\"")
                && body.contains("\"lifecycle\"")
                && body.contains("\"session-start\"")
                && body.contains("\"session-end\"")
                && body.contains("\"stop-guard\"")
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
    let mem_dir = project_memory_dir_at(target, home);
    let files_present =
        mem_dir.join("context-capsule.md").is_file() && mem_dir.join("task-memory.md").is_file();
    let archive_ready = mem_dir.join("task-archive").is_dir();

    let normalized_host = match agent_type.as_str() {
        "oh-my-pi" => "omp",
        other => other,
    };
    let protocol = platform_spec(normalized_host).and_then(|spec| spec.memory_protocol);
    let (adapter, adapter_supported, read_wired, write_wired, stop_guard_wired, kernel_backed) =
        match protocol {
            Some(MemoryProtocol::ClaudeCommandHooks) => {
                let paths = [
                    home.join(".claude/settings.json"),
                    target.join(".claude/settings.json"),
                ];
                let read = host_hook_commands(&paths, "SessionStart")
                    .iter()
                    .any(|command| command_matches(command, "session-start", normalized_host));
                let write = host_hook_commands(&paths, "Stop")
                    .iter()
                    .any(|command| command_matches(command, "session-end", normalized_host));
                let guard = host_hook_commands(&paths, "Stop")
                    .iter()
                    .any(|command| command_matches(command, "stop-guard", normalized_host));
                (
                    "claude-command-hooks",
                    true,
                    read,
                    write,
                    guard,
                    read || write || guard,
                )
            }
            Some(MemoryProtocol::CodexCommandHooks) => {
                let paths = [
                    home.join(".codex/hooks.json"),
                    target.join(".codex/hooks.json"),
                ];
                let read = host_hook_commands(&paths, "SessionStart")
                    .iter()
                    .any(|command| command_matches(command, "session-start", normalized_host));
                let write = host_hook_commands(&paths, "SessionEnd")
                    .iter()
                    .any(|command| command_matches(command, "session-end", normalized_host));
                let guard = host_hook_commands(&paths, "Stop")
                    .iter()
                    .any(|command| command_matches(command, "stop-guard", normalized_host));
                (
                    "codex-command-hooks",
                    true,
                    read,
                    write,
                    guard,
                    read || write || guard,
                )
            }
            Some(MemoryProtocol::CursorCommandHooks) => {
                let paths = [
                    home.join(".cursor/hooks.json"),
                    target.join(".cursor/hooks.json"),
                ];
                let read = host_hook_commands(&paths, "sessionStart")
                    .iter()
                    .any(|command| command_matches(command, "session-start", normalized_host));
                let write = host_hook_commands(&paths, "sessionEnd")
                    .iter()
                    .any(|command| command_matches(command, "session-end", normalized_host));
                let guard = host_hook_commands(&paths, "stop")
                    .iter()
                    .any(|command| command_matches(command, "stop-guard", normalized_host));
                (
                    "cursor-command-hooks",
                    true,
                    read,
                    write,
                    guard,
                    read || write || guard,
                )
            }
            Some(MemoryProtocol::OmpExtension) => {
                let global = home
                    .join(".omp/agent/extensions")
                    .join(OMP_MEMORY_EXTENSION);
                let project = target.join(".omp/extensions").join(OMP_MEMORY_EXTENSION);
                let wired = omp_extension_wired(&[global, project]);
                ("omp-extension", true, wired, wired, wired, wired)
            }
            None => ("unsupported", false, false, false, false, false),
        };

    let status = derive_memory_status(
        adapter_supported,
        files_present,
        archive_ready,
        read_wired,
        write_wired,
        stop_guard_wired,
        kernel_backed,
    );
    let summary = match status {
        "full" => format!("{normalized_host} memory closure complete: native start + close lifecycle wired and backed; files + archive present"),
        "read-only" => format!("{normalized_host} can read project memory on start, but its native close capture is not wired"),
        "write-only" => format!("{normalized_host} closes/captures memory, but its native start injection is not wired"),
        "files-only" => format!("project memory files exist, but {normalized_host} native read/write hooks are not wired — run `ags agents govern --agent {normalized_host} --apply`"),
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
        stop_guard_wired,
        kernel_backed,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roots(tag: &str) -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("ags-memory-lifecycle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let target = root.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let memory = project_memory_dir_at(&target, &home);
        std::fs::create_dir_all(memory.join("task-archive")).unwrap();
        std::fs::write(memory.join("context-capsule.md"), "capsule").unwrap();
        std::fs::write(memory.join("task-memory.md"), "memory").unwrap();
        (home, target)
    }

    fn write_json(path: &Path, value: serde_json::Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn codex_rejects_a_claude_host_command_and_requires_stop_guard() {
        let (home, target) = test_roots("codex-host");
        write_json(
            &home.join(".codex/hooks.json"),
            serde_json::json!({
                "hooks": {
                    "SessionStart": [{
                        "command": "ags host lifecycle --event session-start --host claude-code --target ."
                    }],
                    "SessionEnd": [{
                        "command": "ags host lifecycle --event session-end --host codex --target ."
                    }],
                    "Stop": [{
                        "command": "ags host lifecycle --event stop-guard --host codex --target ."
                    }]
                }
            }),
        );
        let wrong = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::Codex);
        assert_eq!(wrong.status, "write-only");
        assert!(!wrong.read_wired);

        write_json(
            &home.join(".codex/hooks.json"),
            serde_json::json!({
                "hooks": {
                    "SessionStart": [{
                        "command": "ags host lifecycle --event session-start --host codex --target ."
                    }],
                    "SessionEnd": [{
                        "command": "ags host lifecycle --event session-end --host codex --target ."
                    }],
                    "Stop": [{
                        "command": "ags host lifecycle --event stop-guard --host codex --target ."
                    }]
                }
            }),
        );
        let ready = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::Codex);
        assert_eq!(ready.status, "full");
        assert!(ready.stop_guard_wired);
    }

    #[test]
    fn cursor_native_lowercase_hooks_form_a_full_closure() {
        let (home, target) = test_roots("cursor");
        write_json(
            &home.join(".cursor/hooks.json"),
            serde_json::json!({
                "version": 1,
                "hooks": {
                    "sessionStart": [{
                        "type": "command",
                        "command": "ags host lifecycle --event session-start --host cursor --target ."
                    }],
                    "sessionEnd": [{
                        "type": "command",
                        "command": "ags host lifecycle --event session-end --host cursor --target ."
                    }],
                    "stop": [{
                        "type": "command",
                        "command": "ags host lifecycle --event stop-guard --host cursor --target ."
                    }]
                }
            }),
        );

        let lifecycle = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::Cursor);
        assert_eq!(lifecycle.adapter, "cursor-command-hooks");
        assert_eq!(lifecycle.status, "full");
        assert!(lifecycle.read_wired);
        assert!(lifecycle.write_wired);
        assert!(lifecycle.stop_guard_wired);
    }
}
