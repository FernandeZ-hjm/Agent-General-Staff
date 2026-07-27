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
    let supported = match host {
        "claude-code" => {
            let settings = home.join(".claude/settings.json");
            report.add(wire_workspace_memory_start(
                &settings,
                &memory_start_command(),
            ));
            report.add(wire_workspace_memory_capture(
                &settings,
                &memory_capture_command(),
            ));
            true
        }
        "codex" => {
            report.add(wire_codex_memory_lifecycle(&home.join(".codex/hooks.json")));
            true
        }
        "omp" => {
            report.add(ensure_omp_memory_extension(home));
            true
        }
        other => {
            report.add(crate::setup::SetupFinding::fail(
                "agents-memory-lifecycle-unsupported",
                format!("no AGS native memory lifecycle adapter for `{other}`"),
                "Supported adapters: claude-code, codex, omp.",
            ));
            false
        }
    };
    if supported {
        report.add(bootstrap_workspace_memory_with(
            &context_memory_script_path(home),
            workspace_root,
            None,
        ));
    }
}

/// Bootstrap the current workspace's memory capsule by invoking the installed
/// `context-memory.sh init`. Create-if-missing; the script never overwrites the
/// capsule. Fail-closed on the `--register-claude` apply path: a missing script
/// or shell failure is a blocking `fail` (the operator asked to wire the chain),
/// not an advisory warn. `memory_root` overrides the default store (tests only).
pub(in crate::setup) fn bootstrap_workspace_memory_with(
    script_path: &Path,
    workspace_root: &Path,
    memory_root: Option<&Path>,
) -> crate::setup::SetupFinding {
    let check = "setup-memory-capsule-bootstrap";
    if !script_path.is_file() {
        return crate::setup::SetupFinding::fail(
            check,
            "context-memory.sh not installed — capsule bootstrap skipped",
            format!("expected installed script at {}", script_path.display()),
        );
    }
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(script_path)
        .arg("init")
        .arg("--repo")
        .arg(workspace_root);
    if let Some(root) = memory_root {
        cmd.env("MEMORY_ROOT", root);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => crate::setup::SetupFinding::pass(
            check,
            format!(
                "workspace memory capsule ready for {} (capsule never overwritten)",
                workspace_root.display()
            ),
        ),
        Ok(out) => crate::setup::SetupFinding::fail(
            check,
            "context-memory.sh init reported a problem",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => crate::setup::SetupFinding::fail(
            check,
            "could not run context-memory.sh init",
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
        &memory_start_command(),
    ));
    report.add(wire_workspace_memory_capture(
        &settings_path,
        &memory_capture_command(),
    ));
    let script_path = context_memory_script_path(home);
    report.add(bootstrap_workspace_memory_with(
        &script_path,
        workspace_root,
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
        "  - Shared scripts: {} , {} , {} , {}",
        raw_tool_call_stop_guard_path(home).display(),
        context_memory_script_path(home).display(),
        context_memory_start_path(home).display(),
        claude_stop_memory_capture_path(home).display()
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
            "  - Capsule: bootstrapped via context-memory.sh init (create-if-missing; never overwrites context-capsule.md)."
                .to_string(),
        );
    } else {
        lines.push(
            "  - Action: setup refreshes shared scripts + OMP extension. Use `ags agents govern --agent <claude-code|codex|omp> --apply` for explicit native host wiring; use --register-claude only for explicit Claude MCP/workspace registration."
                .to_string(),
        );
    }
    lines.join("\n")
}
