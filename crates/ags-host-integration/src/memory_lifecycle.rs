use super::*;
// ── Project memory lifecycle closure ────────────────────────────────────────
//
// A project is "onboarded with a full memory closure" only when the REQUESTED
// host can READ on session start, WRITE/close on the host's terminal lifecycle
// event, ARCHIVE into task-archive, and invoke the Rust lifecycle kernel.
// The host is part of this interface: one host's evidence must never satisfy
// Codex, Claude Code, Cursor, or OMP. `ags init` owns the project store;
// `ags govern host-projection` plans
// native host adapters; doctor and preflight consume this same computation.

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
    /// Every configured hook/extension delegates to the `ags-host lifecycle` executable.
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

/// Derive the single-component memory identity from the canonical workspace.
/// Profile metadata is intentionally not an authority input.
pub fn project_memory_key(target: &Path) -> Result<String, String> {
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize memory workspace: {error}"))?;
    let basename = canonical
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workspace".into());
    let mut sanitized = basename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    sanitized.truncate(48);
    let sanitized = sanitized.trim_matches('-');
    let sanitized = if sanitized.is_empty() {
        "workspace"
    } else {
        sanitized
    };
    let digest = ags_platform::sha256_hex(canonical.to_string_lossy().as_bytes());
    Ok(format!("{sanitized}-{}", &digest[..24]))
}

pub fn project_memory_dir_at(target: &Path, home: &Path) -> Result<PathBuf, String> {
    Ok(home
        .join(".agents/memory/projects")
        .join(project_memory_key(target)?))
}

/// Compute the closure for the exact host requested by preflight.
pub fn compute_memory_lifecycle_for_host(target: &Path, agent_type: &AgentType) -> MemoryLifecycle {
    let home = ags_platform::home_dir_or_temp();
    compute_memory_lifecycle_at_for_host(target, &home, agent_type)
}

/// Testable host-specific lifecycle core. `home` redirects all machine-local
/// state so tests never inspect or mutate the operator's real host configs.
pub fn compute_memory_lifecycle_at_for_host(
    target: &Path,
    home: &Path,
    agent_type: &AgentType,
) -> MemoryLifecycle {
    let Ok(mem_dir) = project_memory_dir_at(target, home) else {
        return MemoryLifecycle {
            host: agent_type.as_str().to_string(),
            adapter: "unsupported".to_string(),
            adapter_supported: false,
            status: "absent".to_string(),
            files_present: false,
            archive_ready: false,
            read_wired: false,
            write_wired: false,
            stop_guard_wired: false,
            kernel_backed: false,
            summary: "memory workspace identity is invalid".to_string(),
        };
    };
    let files_present =
        mem_dir.join("context-capsule.md").is_file() && mem_dir.join("task-memory.md").is_file();
    let archive_ready = mem_dir.join("task-archive").is_dir();

    let normalized_host = match agent_type.as_str() {
        "oh-my-pi" => "omp",
        other => other,
    };
    let lifecycle = HostLifecycleCodec::new(target, normalized_host);
    let (adapter, adapter_supported, read_wired, write_wired, stop_guard_wired, kernel_backed) =
        match lifecycle {
            Ok(codec) => {
                let observation = std::fs::read_to_string(codec.path())
                    .ok()
                    .and_then(|body| codec.observe_body(&body).ok());
                let events =
                    observation
                        .map(|value| value.events)
                        .unwrap_or(LifecycleEventObservation {
                            session_start: false,
                            stop_guard: false,
                            session_end: false,
                        });
                (
                    codec.spec().adapter_id,
                    true,
                    events.session_start,
                    events.session_end,
                    events.stop_guard,
                    events.any(),
                )
            }
            Err(_) => ("unsupported", false, false, false, false, false),
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
        "full" => format!(
            "{normalized_host} memory closure complete: native start + close lifecycle wired and backed; files + archive present"
        ),
        "read-only" => format!(
            "{normalized_host} can read project memory on start, but its native close capture is not wired"
        ),
        "write-only" => format!(
            "{normalized_host} closes/captures memory, but its native start injection is not wired"
        ),
        "files-only" => format!(
            "project memory files exist, but {normalized_host} native read/write hooks are not wired — run `ags govern host-projection --host {normalized_host} --surface hybrid`, then apply its action_ref"
        ),
        "unsupported" => format!(
            "AGS has no native memory lifecycle adapter for host `{normalized_host}`; closure cannot be claimed"
        ),
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
        let memory = project_memory_dir_at(&target, &home).unwrap();
        std::fs::create_dir_all(memory.join("task-archive")).unwrap();
        std::fs::write(memory.join("context-capsule.md"), "capsule").unwrap();
        std::fs::write(memory.join("task-memory.md"), "memory").unwrap();
        (home, target)
    }

    #[test]
    fn memory_closure_consumes_the_canonical_codec_observation() {
        let (home, target) = test_roots("codec");
        for agent in [
            AgentType::ClaudeCode,
            AgentType::Codex,
            AgentType::Cursor,
            AgentType::Generic("codebuddy-code".to_string()),
            AgentType::Generic("omp".to_string()),
        ] {
            let codec = HostLifecycleCodec::new(&target, agent.as_str()).unwrap();
            let path = codec.path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let body = match codec.spec().projection_family {
                LifecycleProjectionFamily::OmpExtension => codec.desired_omp_body(),
                _ => serde_json::json!({"hooks": codec.desired_owned_projection()}).to_string(),
            };
            std::fs::write(&path, body).unwrap();
            let lifecycle = compute_memory_lifecycle_at_for_host(&target, &home, &agent);
            assert_eq!(lifecycle.status, "full", "{}", lifecycle.host);
            assert!(lifecycle.kernel_backed);
        }
    }

    #[test]
    fn memory_identity_is_canonical_workspace_bound_and_ignores_profile_slug() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let first = root.path().join("one/shared-name");
        let second = root.path().join("two/shared-name");
        std::fs::create_dir_all(first.join("config")).unwrap();
        std::fs::create_dir_all(second.join("config")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            first.join("config/agent-project-profile.yaml"),
            "project:\n  slug: /tmp/cross-workspace\n",
        )
        .unwrap();
        std::fs::write(
            second.join("config/agent-project-profile.yaml"),
            "project:\n  slug: ../../cross-workspace\n",
        )
        .unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();
        let first_memory = project_memory_dir_at(&first, &home).unwrap();
        let second_memory = project_memory_dir_at(&second, &home).unwrap();
        let trusted_root = home.join(".agents/memory/projects");
        assert!(first_memory.starts_with(&trusted_root));
        assert!(second_memory.starts_with(&trusted_root));
        assert_ne!(first_memory, second_memory);
        assert_eq!(first_memory.parent(), Some(trusted_root.as_path()));
        assert_eq!(second_memory.parent(), Some(trusted_root.as_path()));
        assert_ne!(
            first_memory.file_name().unwrap().to_string_lossy(),
            "cross-workspace"
        );
    }
}
