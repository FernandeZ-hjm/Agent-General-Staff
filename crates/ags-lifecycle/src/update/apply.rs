//! Effect orchestration for `ags update apply`.

use super::{ProjectInventory, ProjectUpdate, UpdateLane};
use ags_workspace_facts::managed_projects;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct BuildStep {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeUpdate {
    pub report: crate::setup::SetupReport,
}

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub lane: Option<UpdateLane>,
    pub source_root: PathBuf,
    pub runtime_target: PathBuf,
    pub home: PathBuf,
    pub force: bool,
    pub include_optional_extensions: bool,
}

#[derive(Debug, Clone)]
pub struct AdvisedCommand {
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct UpdateVerification {
    pub command: String,
    pub exit_code: i32,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct UpdateWrite {
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub executed_local: bool,
    pub all_ok: bool,
    pub apply_status: &'static str,
    pub applied: bool,
    pub decision: &'static str,
    pub reason: Option<String>,
    pub steps: Vec<BuildStep>,
    pub runtime: Option<RuntimeUpdate>,
    pub projects: Vec<ProjectUpdate>,
    pub project_registry_error: Option<String>,
    pub writes: Vec<UpdateWrite>,
    pub verifications: Vec<UpdateVerification>,
    pub advised: Vec<AdvisedCommand>,
}

fn orchestrate_local_kernel_build(source_root: &Path) -> Vec<BuildStep> {
    let mut steps = Vec::new();
    let dirty = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["status", "--porcelain"])
        .output()
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(false);
    if dirty {
        steps.push(BuildStep {
            label: "git pull --ff-only".to_string(),
            ok: false,
            detail: "source worktree has uncommitted changes; commit/stash before updating"
                .to_string(),
        });
    } else {
        match Command::new("git")
            .arg("-C")
            .arg(source_root)
            .args(["pull", "--ff-only"])
            .output()
        {
            Ok(output) => steps.push(BuildStep {
                label: "git pull --ff-only".to_string(),
                ok: output.status.success(),
                detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
            Err(error) => steps.push(BuildStep {
                label: "git pull --ff-only".to_string(),
                ok: false,
                detail: error.to_string(),
            }),
        }
    }
    match Command::new("cargo")
        .args(["build", "--release", "--manifest-path"])
        .arg(source_root.join("Cargo.toml"))
        .output()
    {
        Ok(output) => {
            let detail = if output.status.success() {
                "built".to_string()
            } else {
                String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .chars()
                    .take(200)
                    .collect()
            };
            steps.push(BuildStep {
                label: "cargo build --release".to_string(),
                ok: output.status.success(),
                detail,
            });
        }
        Err(error) => steps.push(BuildStep {
            label: "cargo build --release".to_string(),
            ok: false,
            detail: error.to_string(),
        }),
    }
    steps
}

pub fn advised_commands(lane: Option<UpdateLane>) -> Vec<AdvisedCommand> {
    UpdateLane::all()
        .into_iter()
        .filter(|candidate| !candidate.auto_executes_locally())
        .filter(|candidate| lane.map(|selected| selected == *candidate).unwrap_or(true))
        .map(|candidate| {
            let command = match candidate {
                UpdateLane::Agents => "ags agents govern",
                UpdateLane::Skills => "ags capability snapshot --write --host <host>",
                UpdateLane::Public => "review public boundary; AGS never publishes by default",
                _ => "",
            };
            AdvisedCommand {
                command: command.to_string(),
                reason: format!("{} lane (advise-only)", candidate.id()),
            }
        })
        .collect()
}

pub fn inspect_projects(source_root: &Path, runtime_home: &Path, apply: bool) -> ProjectInventory {
    let registry = match managed_projects::load(&managed_projects::registry_path(runtime_home)) {
        Ok(registry) => registry,
        Err(error) => {
            return ProjectInventory {
                registry_error: Some(error),
                ..ProjectInventory::default()
            };
        }
    };
    let (existing, stale) = managed_projects::partition_existing(&registry);
    let approved_hosts = crate::setup::approved_lifecycle_hosts(runtime_home).unwrap_or_default();
    let reports = existing
        .iter()
        .map(|project| {
            let report = crate::init::refresh_managed_project(
                Path::new(&project.path),
                &project.slug,
                source_root,
                &approved_hosts,
                apply,
            );
            ProjectUpdate {
                target: report.target,
                slug: report.slug,
                status: report.status,
                drift: report.drift,
                changed_files: report.changed_files,
                unchanged_files: report.unchanged_files,
                blocked_reasons: report.blocked_reasons,
            }
        })
        .collect();
    let stale_reports = stale
        .iter()
        .map(|project| ProjectUpdate {
            target: project.path.clone(),
            slug: project.slug.clone(),
            status: "stale".to_string(),
            drift: true,
            changed_files: Vec::new(),
            unchanged_files: Vec::new(),
            blocked_reasons: vec!["registered project directory is missing".to_string()],
        })
        .collect();
    ProjectInventory {
        registered: registry.projects.len(),
        present: existing.len(),
        stale: stale.len(),
        remote_backed: registry
            .projects
            .iter()
            .filter(|project| managed_projects::is_remote_backed(project))
            .count(),
        reports,
        stale_reports,
        registry_error: None,
    }
}

pub fn execute(request: &ApplyRequest) -> ApplyOutcome {
    let run_core = request
        .lane
        .map(|lane| lane == UpdateLane::Core)
        .unwrap_or(true);
    let run_runtime = request
        .lane
        .map(|lane| lane == UpdateLane::Runtime)
        .unwrap_or(true);
    let run_projects = request
        .lane
        .map(|lane| lane == UpdateLane::Projects)
        .unwrap_or(true);
    let executed_local = run_core || run_runtime || run_projects;

    let steps = if run_core {
        orchestrate_local_kernel_build(&request.source_root)
    } else {
        Vec::new()
    };
    let mut all_ok = steps.iter().all(|step| step.ok);
    let mut verifications = steps
        .iter()
        .map(|step| UpdateVerification {
            command: step.label.clone(),
            exit_code: if step.ok { 0 } else { 1 },
            detail: step.detail.clone(),
        })
        .collect::<Vec<_>>();

    let runtime = if run_runtime && all_ok {
        let result = crate::setup::apply_private(crate::setup::PrivateApplyRequest {
            source_root: &request.source_root,
            target: &request.runtime_target,
            home: &request.home,
            force: request.force,
            include_optional_extensions: request.include_optional_extensions,
            register_claude: false,
            approved_lifecycle_hosts: None,
        });
        let runtime = RuntimeUpdate {
            report: result.report,
        };
        let exit_code = runtime.report.exit_code();
        all_ok &= exit_code == 0;
        verifications.push(UpdateVerification {
            command: "ags setup --yes (runtime/thin-index)".to_string(),
            exit_code,
            detail: "runtime-reapplied".to_string(),
        });
        Some(runtime)
    } else {
        None
    };

    let mut projects = Vec::new();
    let mut project_registry_error = None;
    if run_projects && all_ok {
        let inventory = inspect_projects(&request.source_root, &request.runtime_target, true);
        project_registry_error = inventory.registry_error;
        if project_registry_error.is_some() {
            all_ok = false;
        }
        if !inventory.stale_reports.is_empty() {
            all_ok = false;
        }
        projects.extend(inventory.stale_reports);
        let refreshed_projects = inventory.reports;
        all_ok &= refreshed_projects.iter().all(|project| {
            matches!(
                project.status.as_str(),
                "applied" | "clean" | "suite-authority"
            )
        });
        verifications.extend(refreshed_projects.iter().map(|project| UpdateVerification {
            command: format!("ags update projects refresh {}", project.target),
            exit_code: if matches!(
                project.status.as_str(),
                "applied" | "clean" | "suite-authority"
            ) {
                0
            } else {
                1
            },
            detail: format!(
                "status={} changed={} unchanged={} blocked={}",
                project.status,
                project.changed_files.len(),
                project.unchanged_files.len(),
                project.blocked_reasons.len()
            ),
        }));
        projects.extend(refreshed_projects);
    }

    let writes = projects
        .iter()
        .flat_map(|project| {
            project.changed_files.iter().map(move |path| UpdateWrite {
                path: path.clone(),
                detail: format!("managed project AGS projection: {}", project.slug),
            })
        })
        .collect();

    let (apply_status, applied) = if !executed_local {
        ("advised-only", false)
    } else if all_ok {
        ("applied", true)
    } else {
        ("failed", false)
    };
    let (decision, reason) = match apply_status {
        "advised-only" => (
            "allow",
            Some("advice-only lane selection — no local execution".to_string()),
        ),
        "applied" => ("allow", None),
        _ => ("stop", Some("local kernel build failed".to_string())),
    };

    ApplyOutcome {
        executed_local,
        all_ok,
        apply_status,
        applied,
        decision,
        reason,
        steps,
        runtime,
        projects,
        project_registry_error,
        writes,
        verifications,
        advised: advised_commands(request.lane),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advice_only_lane_is_never_reported_as_applied() {
        let outcome = execute(&ApplyRequest {
            lane: Some(UpdateLane::Agents),
            source_root: PathBuf::from("."),
            runtime_target: PathBuf::from("must-not-be-used"),
            home: PathBuf::from("must-not-be-used"),
            force: false,
            include_optional_extensions: false,
        });
        assert!(!outcome.executed_local);
        assert_eq!(outcome.apply_status, "advised-only");
        assert!(!outcome.applied);
        assert_eq!(outcome.advised.len(), 1);
    }

    #[test]
    fn advised_commands_scope_to_selected_advice_lane() {
        assert!(advised_commands(Some(UpdateLane::Core)).is_empty());
        let agents = advised_commands(Some(UpdateLane::Agents));
        assert_eq!(agents.len(), 1);
        assert!(agents[0].command.contains("ags agents govern"));
        assert_eq!(advised_commands(None).len(), 3);
    }
}
