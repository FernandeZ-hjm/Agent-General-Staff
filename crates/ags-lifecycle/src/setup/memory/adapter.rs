use super::*;
use super::{assets::*, merge::*, wire::*};

/// Explicit `ags agents govern --apply` adapter write. MCP registration remains
/// advice-only; this function changes only AGS-owned lifecycle hooks/extensions
/// and bootstraps the current repository's local memory store.
pub fn apply_host_memory_adapter(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    workspace_root: &Path,
    host: &str,
) {
    let protocol = ags_host_integration::platform_spec(host).and_then(|spec| spec.memory_protocol);
    let supported = match protocol {
        Some(ags_host_integration::MemoryProtocol::ClaudeCommandHooks) => {
            let settings = home.join(".claude/settings.json");
            report.add(wire_workspace_memory_start(
                &settings,
                &memory_start_command("claude-code"),
            ));
            report.add(wire_workspace_memory_capture(
                &settings,
                &memory_capture_command("claude-code"),
                &raw_guard_command("claude-code"),
            ));
            true
        }
        Some(ags_host_integration::MemoryProtocol::CodexCommandHooks) => {
            report.add(wire_codex_memory_lifecycle(&home.join(".codex/hooks.json")));
            true
        }
        Some(ags_host_integration::MemoryProtocol::CursorCommandHooks) => {
            report.add(wire_cursor_memory_lifecycle(
                &home.join(".cursor/hooks.json"),
            ));
            true
        }
        Some(ags_host_integration::MemoryProtocol::OmpExtension) => {
            report.add(ensure_omp_memory_extension(home));
            true
        }
        None => {
            report.add(crate::setup::SetupFinding::fail(
                "agents-memory-lifecycle-unsupported",
                format!("no AGS native memory lifecycle adapter for `{host}`"),
                "Supported adapters: claude-code, codex, cursor, omp.",
            ));
            false
        }
    };
    if supported {
        report.add(bootstrap_workspace_memory_with(workspace_root, home, None));
    }
}

/// Bootstrap the current workspace's memory capsule by invoking the installed
/// `ags memory init`. Create-if-missing; the Rust kernel never overwrites the
/// capsule. Fail-closed on the `--register-claude` apply path: a missing script
/// or shell failure is a blocking `fail` (the operator asked to wire the chain),
/// not an advisory warn. `memory_root` overrides the default store (tests only).
pub(in crate::setup) fn bootstrap_workspace_memory_with(
    workspace_root: &Path,
    home: &Path,
    memory_root: Option<&Path>,
) -> crate::setup::SetupFinding {
    let check = "setup-memory-capsule-bootstrap";
    let memory_dir = memory_root.map_or_else(
        || ags_host_integration::project_memory_dir_at(workspace_root, home),
        |root| root.join(ags_host_integration::resolve_project_slug(workspace_root)),
    );
    match ags_evidence::memory::init(&memory_dir) {
        Ok(_) => crate::setup::SetupFinding::pass(
            check,
            format!(
                "workspace memory capsule ready for {} (capsule never overwritten)",
                workspace_root.display()
            ),
        ),
        Err(e) => crate::setup::SetupFinding::fail(
            check,
            "could not initialize Rust memory store",
            e.to_string(),
        ),
    }
}

/// Register-claude apply step: wire the workspace Stop pipeline and bootstrap
/// the workspace memory capsule. `home` resolves the installed script path;
/// `workspace_root` is the current AGS suite/workspace whose `.claude` config
/// and memory are bootstrapped.
pub(in crate::setup) fn add_workspace_memory_capture(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    workspace_root: &Path,
) {
    add_workspace_memory_capture_inner(report, home, workspace_root, None);
}

pub(super) fn add_workspace_memory_capture_inner(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    workspace_root: &Path,
    memory_root: Option<&Path>,
) {
    let settings_path = workspace_root.join(".claude").join("settings.json");
    report.add(wire_workspace_memory_start(
        &settings_path,
        &memory_start_command("claude-code"),
    ));
    report.add(wire_workspace_memory_capture(
        &settings_path,
        &memory_capture_command("claude-code"),
        &raw_guard_command("claude-code"),
    ));
    report.add(bootstrap_workspace_memory_with(
        workspace_root,
        home,
        memory_root,
    ));
}

/// Read-only preview of what `ags setup --yes --register-claude` will do to the
/// workspace memory-capture chain. Rendered in the setup plan / dry-run so the
/// operator can see the hook install/repair before applying.
pub(in crate::setup) fn render_memory_capture_plan(
    home: &Path,
    workspace_root: &Path,
    register_claude: bool,
) -> String {
    let settings_path = workspace_root.join(".claude").join("settings.json");
    let (start_wired, raw_wired, memory_wired) = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|v| {
            let start_wired = v
                .get("hooks")
                .and_then(|h| h.get("SessionStart"))
                .and_then(|s| s.as_array())
                .map(|start| hooks_contain(start, MEMORY_START_MARKER))
                .unwrap_or(false);
            let (raw_wired, memory_wired) = v
                .get("hooks")
                .and_then(|h| h.get("Stop"))
                .and_then(|s| s.as_array())
                .map(|stop| {
                    (
                        hooks_contain(stop, RAW_GUARD_MARKER),
                        hooks_contain(stop, MEMORY_CAPTURE_MARKER),
                    )
                })
                .unwrap_or((false, false));
            (start_wired, raw_wired, memory_wired)
        })
        .unwrap_or((false, false, false));

    let mut lines = vec!["Memory capture chain (project memory):".to_string()];
    lines.push(format!(
        "  - Rust lifecycle command: ags host lifecycle (SessionStart / SessionEnd / Stop guard)"
    ));
    lines.push(format!(
        "  - OMP native extension: {}",
        omp_memory_lifecycle_path(home).display()
    ));
    lines.push(format!(
        "  - Workspace SessionStart config: {}",
        settings_path.display()
    ));
    lines.push(format!(
        "  - Workspace Stop config: {}",
        settings_path.display()
    ));
    lines.push(format!(
        "  - Current state: project memory start hook {}",
        if start_wired { "WIRED" } else { "MISSING" }
    ));
    lines.push(format!(
        "  - Current state: raw guard {}",
        if raw_wired { "WIRED" } else { "MISSING" }
    ));
    lines.push(format!(
        "  - Current state: project memory capture {}",
        if memory_wired { "WIRED" } else { "MISSING" }
    ));
    if register_claude {
        if start_wired && raw_wired && memory_wired {
            lines.push(
                "  - Action: scripts refreshed; SessionStart + Stop pipelines already wired (idempotent)."
                    .to_string(),
            );
        } else {
            lines.push(
                "  - Action: install scripts + repair SessionStart injection and Stop pipeline (raw guard → project memory capture), backing up the prior settings.json."
                    .to_string(),
            );
        }
        lines.push(
            "  - Capsule: bootstrapped by the Rust memory kernel (create-if-missing; never overwrites context-capsule.md)."
                .to_string(),
        );
    } else {
        lines.push(
            "  - Action: setup refreshes the OMP extension. Use `ags agents govern --agent <claude-code|codex|cursor|omp> --apply` for explicit native host wiring; use --register-claude only for explicit Claude MCP/workspace registration."
                .to_string(),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roots(tag: &str) -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("ags-host-adapter-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let target = root.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        (home, target)
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn all_four_host_adapters_write_exact_rust_commands_and_preserve_assets() {
        let (home, target) = test_roots("all-hosts");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/hooks.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "hooks": {
                    "Stop": [{"command": "node .codex/hooks/evolver-session-end.js"}]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::write(
            home.join(".cursor/hooks.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "project_owned": {"keep": true},
                "hooks": {"sessionStart": [{"command": "echo keep-cursor"}]}
            }))
            .unwrap(),
        )
        .unwrap();

        let memory = ags_host_integration::project_memory_dir_at(&target, &home);
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("context-capsule.md"), "protected capsule").unwrap();

        for host in ["claude-code", "codex", "cursor", "omp"] {
            let mut report = crate::setup::SetupReport::new(host);
            apply_host_memory_adapter(&mut report, &home, &target, host);
            assert!(report.passed(), "{host}: {:?}", report.findings);
            let agent = ags_host_integration::AgentType::from_str(host).unwrap();
            let lifecycle =
                ags_host_integration::compute_memory_lifecycle_at_for_host(&target, &home, &agent);
            assert_eq!(lifecycle.status, "full", "{host}: {lifecycle:?}");
        }

        let codex = read_json(&home.join(".codex/hooks.json"));
        assert!(codex
            .to_string()
            .contains("host lifecycle --event session-start --host codex"));
        assert!(codex
            .to_string()
            .contains("host lifecycle --event session-end --host codex"));
        assert!(codex
            .to_string()
            .contains("host lifecycle --event stop-guard --host codex"));
        assert!(codex.to_string().contains("evolver-session-end"));
        assert!(!codex
            .to_string()
            .contains("session-start --host claude-code"));

        let cursor = read_json(&home.join(".cursor/hooks.json"));
        assert_eq!(cursor["project_owned"]["keep"], true);
        assert!(cursor.to_string().contains("echo keep-cursor"));
        for event in ["session-start", "session-end", "stop-guard"] {
            assert!(cursor
                .to_string()
                .contains(&format!("host lifecycle --event {event} --host cursor")));
        }

        let claude = read_json(&home.join(".claude/settings.json"));
        for event in ["session-start", "session-end", "stop-guard"] {
            assert!(claude.to_string().contains(&format!(
                "host lifecycle --event {event} --host claude-code"
            )));
        }

        let omp = std::fs::read_to_string(omp_memory_lifecycle_path(&home)).unwrap();
        for event in ["session-start", "session-end", "stop-guard"] {
            assert!(omp.contains(event));
        }
        assert_eq!(
            std::fs::read_to_string(memory.join("context-capsule.md")).unwrap(),
            "protected capsule"
        );

        for host in ["claude-code", "codex", "cursor", "omp"] {
            let mut report = crate::setup::SetupReport::new(host);
            apply_host_memory_adapter(&mut report, &home, &target, host);
            assert!(report.passed(), "{host}: {:?}", report.findings);
        }
    }
}
