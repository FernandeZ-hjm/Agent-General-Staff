use crate::lifecycle_projection::migration_backup_path;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleMigrationWorkspace {
    pub canonical_workspace: String,
    pub adapter_path: String,
    pub adapter_ready_now: bool,
    pub adapter_ready_after_apply: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleMigrationPreview {
    pub host: String,
    pub current_workspace: String,
    pub workspace_adapter: String,
    pub managed_workspaces: Vec<LifecycleMigrationWorkspace>,
    pub global_config: String,
    pub global_ags_owned_entry_present: bool,
    pub removal_ready_after_apply: bool,
    pub backup_path: String,
}

#[derive(Debug)]
struct ManagedLifecycleWorkspaceObservation {
    registered: PathBuf,
    canonical: Option<PathBuf>,
    missing: bool,
}

impl ManagedLifecycleWorkspaceObservation {
    fn workspace(&self) -> &Path {
        self.canonical.as_deref().unwrap_or(&self.registered)
    }
}

pub fn lifecycle_migration_preview(
    home: &Path,
    workspace_root: &Path,
    host: &str,
) -> Result<LifecycleMigrationPreview, String> {
    let current = ags_platform::canonical_workspace_root(workspace_root)?;
    let workspace_adapter = crate::lifecycle_projection::workspace_adapter_path(&current, host)
        .ok_or_else(|| format!("unsupported lifecycle host `{host}`"))?;
    let global = crate::lifecycle_projection::global_adapter_path(home, host)
        .ok_or_else(|| format!("unsupported lifecycle host `{host}`"))?;
    let global_ags_owned_entry_present = crate::lifecycle_projection::global_ags_owned(home, host)?;
    let managed_workspaces = managed_lifecycle_workspace_observations(&current)?
        .into_iter()
        .map(|observation| {
            let workspace = observation.workspace();
            let adapter_path = crate::lifecycle_projection::workspace_adapter_path(workspace, host)
                .expect("validated lifecycle host has an adapter path");
            let adapter_ready_now = observation.canonical.as_ref().is_some_and(|workspace| {
                crate::lifecycle_projection::LifecycleProjection::new(workspace, host)
                    .is_ok_and(|projection| projection.observe().current)
            });
            LifecycleMigrationWorkspace {
                canonical_workspace: workspace.to_string_lossy().to_string(),
                adapter_path: adapter_path.to_string_lossy().to_string(),
                adapter_ready_now,
                adapter_ready_after_apply: observation
                    .canonical
                    .as_ref()
                    .is_some_and(|workspace| projection_ready_after_apply(workspace, host)),
            }
        })
        .collect::<Vec<_>>();
    let removal_ready_after_apply = !managed_workspaces.is_empty()
        && managed_workspaces
            .iter()
            .all(|item| item.adapter_ready_after_apply);
    let backup = migration_backup_path(&global);
    Ok(LifecycleMigrationPreview {
        host: host.to_string(),
        current_workspace: current.to_string_lossy().to_string(),
        workspace_adapter: workspace_adapter.to_string_lossy().to_string(),
        managed_workspaces,
        global_config: global.to_string_lossy().to_string(),
        global_ags_owned_entry_present,
        removal_ready_after_apply,
        backup_path: backup.to_string_lossy().to_string(),
    })
}

fn managed_lifecycle_workspace_observations(
    current_workspace: &Path,
) -> Result<Vec<ManagedLifecycleWorkspaceObservation>, String> {
    let runtime = ags_capability_governance::locate_runtime_home();
    let registry_path = ags_workspace_facts::managed_projects::registry_path(&runtime);
    let registry = ags_workspace_facts::managed_projects::load(&registry_path)
        .map_err(|error| format!("{}: {error}", registry_path.display()))?;
    let (projects, missing) = ags_workspace_facts::managed_projects::partition_existing(&registry);
    let mut observations = projects
        .into_iter()
        .map(|project| {
            let registered = PathBuf::from(&project.path);
            ManagedLifecycleWorkspaceObservation {
                canonical: ags_platform::canonical_workspace_root(&registered).ok(),
                registered,
                missing: false,
            }
        })
        .chain(
            missing
                .into_iter()
                .map(|project| ManagedLifecycleWorkspaceObservation {
                    registered: PathBuf::from(&project.path),
                    canonical: None,
                    missing: true,
                }),
        )
        .collect::<Vec<_>>();
    if !observations
        .iter()
        .any(|observation| observation.canonical.as_deref() == Some(current_workspace))
    {
        observations.push(ManagedLifecycleWorkspaceObservation {
            registered: current_workspace.to_path_buf(),
            canonical: Some(current_workspace.to_path_buf()),
            missing: false,
        });
    }
    observations.sort_by(|left, right| left.workspace().cmp(right.workspace()));
    observations.dedup_by(|left, right| left.workspace() == right.workspace());
    Ok(observations)
}

fn projection_ready_after_apply(workspace: &Path, host: &str) -> bool {
    let Ok(projection) = crate::lifecycle_projection::LifecycleProjection::new(workspace, host)
    else {
        return false;
    };
    projection.ready_after_install()
}
/// Explicit `ags agents govern --apply` adapter write. MCP registration remains
/// advice-only; this function changes only AGS-owned lifecycle hooks/extensions
/// and bootstraps the current repository's local memory store.
pub fn apply_host_memory_adapter(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    workspace_root: &Path,
    host: &str,
) {
    let workspace_root = match ags_platform::canonical_workspace_root(workspace_root) {
        Ok(path) => path,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "agents-memory-lifecycle-canonical-workspace",
                "workspace lifecycle target is not canonical",
                error,
            ));
            return;
        }
    };
    let projection =
        match crate::lifecycle_projection::LifecycleProjection::new(&workspace_root, host) {
            Ok(projection) => projection,
            Err(error) => {
                report.add(crate::setup::SetupFinding::fail(
                    "agents-memory-lifecycle-unsupported",
                    format!("no AGS native memory lifecycle adapter for `{host}`"),
                    error,
                ));
                return;
            }
        };
    let projection_path = projection.path();
    let backup = migration_backup_path(&projection_path);
    match projection.install_with_backup(&backup) {
        Ok(crate::lifecycle_projection::ProjectionInstallOutcome::AlreadyCurrent) => {
            report.add(crate::setup::SetupFinding::pass(
                "workspace-lifecycle-projection",
                format!(
                    "{host} workspace lifecycle is already current at {}",
                    projection_path.display()
                ),
            ));
        }
        Ok(crate::lifecycle_projection::ProjectionInstallOutcome::Installed) => {
            report.add(crate::setup::SetupFinding::pass(
                "workspace-lifecycle-projection",
                format!(
                    "installed {host} workspace lifecycle at {}",
                    projection_path.display()
                ),
            ));
        }
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "workspace-lifecycle-projection",
                format!("could not install {host} workspace lifecycle"),
                error,
            ));
            return;
        }
    }
    report.add(bootstrap_workspace_memory(&workspace_root, home));
    if !report.passed() {
        return;
    }
    match crate::lifecycle_projection::record_lifecycle_manifest(&workspace_root) {
        Ok(path) => report.add(crate::setup::SetupFinding::pass(
            "workspace-lifecycle-manifest-current",
            format!("recorded {}", path.display()),
        )),
        Err(error) => report.add(crate::setup::SetupFinding::fail(
            "workspace-lifecycle-manifest-current",
            "could not record workspace lifecycle adapter",
            error,
        )),
    }
    if !report.passed() {
        return;
    }
    migrate_global_adapter_if_safe(report, home, &workspace_root, host);
}

fn migrate_global_adapter_if_safe(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    current_workspace: &Path,
    host: &str,
) {
    if home != ags_platform::home_dir_or_temp() {
        return;
    }
    let Some(global) = crate::lifecycle_projection::global_adapter_path(home, host) else {
        return;
    };
    match crate::lifecycle_projection::global_ags_owned(home, host) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "workspace-lifecycle-global-migration",
                format!("cannot inspect global {host} lifecycle"),
                error,
            ));
            return;
        }
    }
    let observations = match managed_lifecycle_workspace_observations(current_workspace) {
        Ok(observations) => observations,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "workspace-lifecycle-global-migration",
                "managed project registry is unreadable; global lifecycle left unchanged",
                error,
            ));
            return;
        }
    };
    let missing = observations
        .iter()
        .filter(|observation| observation.missing)
        .map(|observation| observation.registered.clone())
        .collect::<Vec<_>>();
    let workspaces = observations
        .iter()
        .filter(|observation| !observation.missing)
        .map(|observation| observation.registered.clone())
        .collect::<Vec<_>>();
    migrate_global_adapter_for_workspaces(report, home, &global, &workspaces, &missing, host);
}

fn migrate_global_adapter_for_workspaces(
    report: &mut crate::setup::SetupReport,
    home: &Path,
    global: &Path,
    workspaces: &[PathBuf],
    missing_workspaces: &[PathBuf],
    host: &str,
) {
    if !missing_workspaces.is_empty() {
        report.add(crate::setup::SetupFinding::fail(
            "workspace-lifecycle-global-migration",
            format!("global {host} lifecycle kept; managed workspace paths are missing"),
            missing_workspaces
                .iter()
                .map(|workspace| workspace.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
        return;
    }
    let failures = workspaces
        .iter()
        .filter_map(|workspace| {
            let result = crate::lifecycle_projection::LifecycleProjection::new(workspace, host)
                .and_then(|projection| {
                    let backup = migration_backup_path(&projection.path());
                    projection.install_with_backup(&backup)
                })
                .and_then(|_| {
                    crate::lifecycle_projection::record_lifecycle_manifest(workspace).map(|_| ())
                });
            result
                .err()
                .map(|error| format!("{}: {error}", workspace.display()))
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        report.add(crate::setup::SetupFinding::fail(
            "workspace-lifecycle-global-migration",
            format!("global {host} lifecycle kept; workspace migration incomplete"),
            failures.join("; "),
        ));
        return;
    }
    let backup = migration_backup_path(global);
    if !backup.exists() {
        if let Err(error) = std::fs::copy(global, &backup) {
            report.add(crate::setup::SetupFinding::fail(
                "workspace-lifecycle-global-migration",
                format!("cannot back up {}", global.display()),
                error.to_string(),
            ));
            return;
        }
    }
    let result = crate::lifecycle_projection::remove_global_ags_owned(home, host).map(|_| ());
    match result {
        Ok(()) => report.add(crate::setup::SetupFinding::pass(
            "workspace-lifecycle-global-migration",
            format!(
                "removed AGS-owned global {host} lifecycle after all managed workspaces were ready; backup: {}",
                backup.display()
            ),
        )),
        Err(error) => report.add(crate::setup::SetupFinding::fail(
            "workspace-lifecycle-global-migration",
            format!("global {host} lifecycle migration failed"),
            error,
        )),
    }
}

/// Bootstrap the current workspace's memory capsule by invoking the installed
/// `ags memory init`. Create-if-missing; the Rust kernel never overwrites the
/// capsule. Fail-closed on the `--register-claude` apply path.
pub(in crate::setup) fn bootstrap_workspace_memory(
    workspace_root: &Path,
    home: &Path,
) -> crate::setup::SetupFinding {
    let check = "setup-memory-capsule-bootstrap";
    let memory_dir = ags_host_integration::project_memory_dir_at(workspace_root, home);
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
    apply_host_memory_adapter(report, home, workspace_root, "claude-code");
}

/// Read-only preview of what `ags setup --yes --register-claude` will do to the
/// workspace memory-capture chain. Rendered in the setup plan / dry-run so the
/// operator can see the hook install/repair before applying.
pub(in crate::setup) fn render_memory_capture_plan(
    _home: &Path,
    workspace_root: &Path,
    register_claude: bool,
) -> String {
    let projection =
        crate::lifecycle_projection::LifecycleProjection::new(workspace_root, "claude-code").ok();
    let observed = projection.as_ref().map(|projection| projection.observe());
    let settings_path = projection
        .as_ref()
        .map(crate::lifecycle_projection::LifecycleProjection::path)
        .unwrap_or_else(|| workspace_root.join(".claude/settings.local.json"));
    let (start_wired, raw_wired, memory_wired) = observed
        .as_ref()
        .map(|observation| {
            (
                observation.events_complete,
                observation.events_complete,
                observation.events_complete,
            )
        })
        .unwrap_or((false, false, false));

    let mut lines = vec!["Memory capture chain (project memory):".to_string()];
    lines.push(
        "  - Rust lifecycle command: ags host lifecycle (SessionStart / SessionEnd / Stop guard)"
            .to_string(),
    );
    lines.push(format!(
        "  - OMP native extension: {}",
        crate::lifecycle_projection::workspace_adapter_path(workspace_root, "omp")
            .unwrap_or_else(|| workspace_root.join(".omp/extensions/ags-lifecycle.js"))
            .display()
    ));
    lines.push(format!(
        "  - Workspace SessionStart config: {}",
        settings_path.display()
    ));
    lines.push(format!(
        "  - Workspace Stop + SessionEnd config: {}",
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
                "  - Action: workspace SessionStart + per-turn Stop Guard + true SessionEnd already wired (idempotent)."
                    .to_string(),
            );
        } else {
            lines.push(
                "  - Action: install the workspace SessionStart, per-turn Stop Guard, and true SessionEnd adapter while preserving unrelated hooks."
                    .to_string(),
            );
        }
        lines.push(
            "  - Capsule: bootstrapped by the Rust memory kernel (create-if-missing; never overwrites context-capsule.md)."
                .to_string(),
        );
    } else {
        lines.push(
            "  - Action: setup leaves optional workspace adapters unchanged. Use `ags agents govern --agent <claude-code|codex|cursor|codebuddy-code|omp> --apply` for explicit workspace host wiring; use --register-claude only for explicit Claude MCP/workspace registration."
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

    fn owned_event_count(value: &serde_json::Value, event: &str, host: &str) -> usize {
        value["hooks"][event]
            .as_array()
            .into_iter()
            .flatten()
            .map(|group| {
                usize::from(group["command"].as_str().is_some_and(|command| {
                    ags_host_integration::lifecycle_command_is_owned(host, command)
                })) + group["hooks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|hook| {
                        hook["command"].as_str().is_some_and(|command| {
                            ags_host_integration::lifecycle_command_is_owned(host, command)
                        })
                    })
                    .count()
            })
            .sum()
    }

    #[test]
    fn migration_preview_does_not_claim_invalid_json_is_apply_ready() {
        let (_home, target) = test_roots("preview-invalid-json");
        let local = target.join(".claude/settings.local.json");
        let shared = target.join(".claude/settings.json");
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, "{invalid").unwrap();
        assert!(!projection_ready_after_apply(&target, "claude-code"));
        std::fs::remove_file(local).unwrap();
        std::fs::write(&shared, "{invalid").unwrap();
        assert!(!projection_ready_after_apply(&target, "claude-code"));
        assert!(!migration_backup_path(&shared).exists());
    }

    #[test]
    fn claude_workspace_migration_removes_shared_ags_hooks_after_local_is_current() {
        let (home, target) = test_roots("claude-shared-migration");
        let shared = target.join(".claude/settings.json");
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        std::fs::write(
            &shared,
            serde_json::to_vec_pretty(&serde_json::json!({
                "mcpServers": {"keep": {"command": "keep-mcp"}},
                "projectOwned": {"keep": true},
                "hooks": {
                    "SessionStart": [{
                        "hooks": [
                            {"type": "command", "command": "user-start"},
                            {
                                "type": "command",
                                "command": "python3 \"$HOME/.agents/scripts/context-memory-start.py\""
                            }
                        ]
                    }],
                    "Stop": [{
                        "hooks": [
                            {"type": "command", "command": "user-stop"},
                            {
                                "type": "command",
                                "command": "set -o pipefail; ags host lifecycle --event stop-guard --host claude-code --target . | jq -c 'if .hookSpecificOutput == null then del(.hookSpecificOutput) else . end'"
                            }
                        ]
                    }],
                    "SessionEnd": [{
                        "hooks": [
                            {"type": "command", "command": "user-end"},
                            {
                                "type": "command",
                                "command": "ags host lifecycle --event session-end --host claude-code --target ."
                            }
                        ]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut report = crate::setup::SetupReport::new("claude-shared-migration");
        apply_host_memory_adapter(&mut report, &home, &target, "claude-code");
        assert!(report.passed(), "{:?}", report.findings);

        let local = read_json(&target.join(".claude/settings.local.json"));
        let shared_after = read_json(&shared);
        for event in ["SessionStart", "Stop", "SessionEnd"] {
            assert_eq!(
                owned_event_count(&local, event, "claude-code")
                    + owned_event_count(&shared_after, event, "claude-code"),
                1,
                "{event} should have exactly one effective AGS lifecycle hook"
            );
        }
        let rendered = shared_after.to_string();
        assert!(rendered.contains("user-start"));
        assert!(rendered.contains("user-stop"));
        assert!(rendered.contains("user-end"));
        assert!(rendered.contains("keep-mcp"));
        assert_eq!(shared_after["projectOwned"]["keep"], true);
        assert!(migration_backup_path(&shared).is_file());
        assert!(
            crate::lifecycle_projection::LifecycleProjection::new(&target, "claude-code")
                .unwrap()
                .observe()
                .current
        );
    }

    #[test]
    fn invalid_shared_claude_config_is_backed_up_before_migration_fails_closed() {
        let (home, target) = test_roots("claude-shared-invalid");
        let shared = target.join(".claude/settings.json");
        let local = target.join(".claude/settings.local.json");
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        std::fs::write(&shared, "{invalid-shared-json").unwrap();

        let mut report = crate::setup::SetupReport::new("claude-shared-invalid");
        apply_host_memory_adapter(&mut report, &home, &target, "claude-code");

        assert!(!report.passed());
        assert!(!local.exists(), "canonical projection must not install");
        assert_eq!(
            std::fs::read_to_string(&shared).unwrap(),
            "{invalid-shared-json"
        );
        assert_eq!(
            std::fs::read_to_string(migration_backup_path(&shared)).unwrap(),
            "{invalid-shared-json"
        );
    }

    #[test]
    fn global_migration_removes_only_ags_owned_hooks() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            serde_json::json!({
                "mcpServers": {"keep": {"command": "keep-mcp"}},
                "hooks": {
                    "SessionStart": [
                        {"command": "ags host lifecycle --event session-start --host claude-code --target ."},
                        {"command": "user-start"}
                    ],
                    "Stop": [{
                        "hooks": [
                            {"command": "ags host lifecycle --event stop-guard --host claude-code --target ."},
                            {"command": "user-stop"}
                        ]
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();

        assert!(
            crate::lifecycle_projection::remove_global_ags_owned(root.path(), "claude-code")
                .unwrap()
        );

        let value = read_json(&settings);
        let rendered = value.to_string();
        assert!(!rendered.contains("ags host lifecycle"));
        assert!(rendered.contains("user-start"));
        assert!(rendered.contains("user-stop"));
        assert!(rendered.contains("keep-mcp"));
    }

    #[test]
    fn global_migration_waits_for_every_live_projection_then_preserves_user_state() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let first = root.path().join("first");
        let second = root.path().join("second");
        for workspace in [&first, &second] {
            std::fs::create_dir_all(workspace.join(".git")).unwrap();
        }
        crate::lifecycle_projection::LifecycleProjection::new(&first, "claude-code")
            .unwrap()
            .install()
            .unwrap();
        let second_config = second.join(".claude/settings.local.json");
        std::fs::create_dir_all(second_config.parent().unwrap()).unwrap();
        std::fs::write(&second_config, "{not-json").unwrap();
        let global = home.join(".claude/settings.json");
        std::fs::create_dir_all(global.parent().unwrap()).unwrap();
        std::fs::write(
            &global,
            serde_json::to_vec_pretty(&serde_json::json!({
                "mcpServers": {"keep": {"command": "keep-mcp"}},
                "hooks": {
                    "SessionStart": [{
                        "hooks": [
                            {"command": "user-start"},
                            {"command": "ags host lifecycle --event session-start --host claude-code --target ."}
                        ]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let workspaces = vec![first.clone(), second.clone()];
        let missing = root.path().join("missing-managed-workspace");
        let migrate = |report: &mut crate::setup::SetupReport, missing: &[PathBuf]| {
            migrate_global_adapter_for_workspaces(
                report,
                &home,
                &global,
                &workspaces,
                missing,
                "claude-code",
            );
        };
        let mut missing_report = crate::setup::SetupReport::new("missing");
        migrate(&mut missing_report, &[missing]);
        assert!(!missing_report.passed());
        assert!(std::fs::read_to_string(&global)
            .unwrap()
            .contains("ags host lifecycle"));
        assert!(!migration_backup_path(&global).exists());

        let mut blocked = crate::setup::SetupReport::new("blocked");
        migrate(&mut blocked, &[]);
        assert!(!blocked.passed());
        assert!(std::fs::read_to_string(&global)
            .unwrap()
            .contains("ags host lifecycle"));
        assert!(!migration_backup_path(&global).exists());
        assert!(migration_backup_path(&second_config).is_file());

        std::fs::write(&second_config, r#"{"user":"keep-second"}"#).unwrap();
        let mut migrated = crate::setup::SetupReport::new("migrated");
        migrate(&mut migrated, &[]);
        assert!(migrated.passed(), "{:?}", migrated.findings);
        let rendered = std::fs::read_to_string(&global).unwrap();
        assert!(!rendered.contains("ags host lifecycle"));
        assert!(rendered.contains("user-start"));
        assert!(rendered.contains("keep-mcp"));
        assert!(migration_backup_path(&global).is_file());
        assert!(std::fs::read_to_string(second_config)
            .unwrap()
            .contains("keep-second"));
    }
}
