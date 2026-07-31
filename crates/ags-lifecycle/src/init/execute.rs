//! Project initialization request orchestration.

use super::apply::write_project_init_file;
use super::managed_projects::{register_managed_project, ManagedProjectRegistration};
use super::model::{InitFinding, InitReport, PROJECT_INIT_SCHEMA};
use super::overlay::{
    apply_overlay, compute_overlay_plan_with_paths, overlay_json, render_overlay_text,
    untrack_pure_overlay_files, OverlayMode, OverlayPlan,
};
use super::plan::{project_init_plan, sanitize_name, ProjectInitPlan};
use super::render::{render_init_report_text, render_project_init_json, render_project_init_text};
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct InitLifecycleResult {
    pub approved_hosts: Vec<String>,
    pub projected_hosts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InitRequest {
    pub target: PathBuf,
    pub runtime_home: PathBuf,
    pub now: u64,
    pub slug: Option<String>,
    pub dry_run: bool,
    pub mode: String,
    pub approved_lifecycle_hosts: Vec<String>,
}

pub enum InitOutput {
    DryRun {
        plan: ProjectInitPlan,
        overlay: OverlayPlan,
        lifecycle: InitLifecycleResult,
    },
    Applied {
        plan: ProjectInitPlan,
        overlay: OverlayPlan,
        report: InitReport,
        preflight: Box<ags_workspace_facts::SessionPreflight>,
        managed_project_registration: Option<ManagedProjectRegistration>,
        managed_project_receipt: Option<PathBuf>,
        lifecycle: InitLifecycleResult,
    },
}

impl InitOutput {
    fn host_list(hosts: &[String]) -> String {
        if hosts.is_empty() {
            "none".to_string()
        } else {
            hosts.join(", ")
        }
    }

    pub fn render_text(&self) -> String {
        match self {
            Self::DryRun {
                plan,
                overlay,
                lifecycle,
            } => format!(
                "{}\n\n{}\n\nApproved lifecycle hosts: {}",
                render_project_init_text(plan, true),
                render_overlay_text(overlay),
                Self::host_list(&lifecycle.approved_hosts)
            ),
            Self::Applied {
                plan,
                overlay,
                report,
                preflight,
                lifecycle,
                ..
            } => format!(
                "{}\n\n{}\n\nApproved lifecycle hosts: {}\nProjected lifecycle hosts: {}\n\n{}\n\n{}",
                render_project_init_text(plan, false),
                render_overlay_text(overlay),
                Self::host_list(&lifecycle.approved_hosts),
                Self::host_list(&lifecycle.projected_hosts),
                render_init_report_text(report),
                ags_workspace_facts::render_session_preflight_text(preflight)
            ),
        }
    }

    pub fn render_json(&self) -> String {
        let value = match self {
            Self::DryRun {
                plan,
                overlay,
                lifecycle,
            } => {
                let mut value: serde_json::Value =
                    serde_json::from_str(&render_project_init_json(plan, true)).unwrap_or_default();
                if let Some(object) = value.as_object_mut() {
                    object.insert("overlay".to_string(), overlay_json(overlay));
                    object.insert(
                        "lifecycle".to_string(),
                        serde_json::to_value(lifecycle).unwrap_or_default(),
                    );
                }
                value
            }
            Self::Applied {
                plan,
                overlay,
                report,
                preflight,
                managed_project_receipt,
                lifecycle,
                ..
            } => serde_json::json!({
                "schema_version": PROJECT_INIT_SCHEMA,
                "plan": serde_json::from_str::<serde_json::Value>(&render_project_init_json(plan, false)).unwrap_or_default(),
                "overlay": overlay_json(overlay),
                "report": report,
                "preflight": preflight,
                "lifecycle": lifecycle,
                "managed_project_receipt": managed_project_receipt.as_ref().map(|path| path.display().to_string()),
            }),
        };
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    pub fn succeeded(&self) -> bool {
        match self {
            Self::DryRun { .. } => true,
            Self::Applied {
                report, preflight, ..
            } => report.passed() && preflight.exit_code == 0,
        }
    }

    pub fn managed_project_registration(&self) -> Option<&ManagedProjectRegistration> {
        match self {
            Self::Applied {
                managed_project_registration,
                ..
            } => managed_project_registration.as_ref(),
            Self::DryRun { .. } => None,
        }
    }

    pub fn set_managed_project_receipt(&mut self, receipt: Option<PathBuf>) {
        if let Self::Applied {
            managed_project_receipt,
            ..
        } = self
        {
            *managed_project_receipt = receipt;
        }
    }
}

pub fn execute(request: InitRequest) -> Result<InitOutput, String> {
    let overlay_mode = OverlayMode::parse(&request.mode);
    if !request.target.exists() {
        return Err(format!(
            "ags init: target does not exist — {}",
            request.target.display()
        ));
    }
    let plan = project_init_plan(&request.target, request.slug);
    let mut approved_hosts = request.approved_lifecycle_hosts;
    approved_hosts.sort();
    approved_hosts.dedup();
    let adapter_paths = approved_hosts
        .iter()
        .filter_map(|host| crate::lifecycle_projection::workspace_adapter_path(&plan.target, host))
        .collect::<Vec<_>>();
    let overlay =
        compute_overlay_plan_with_paths(&plan.target, &plan.files, &adapter_paths, overlay_mode);
    let planned_lifecycle = InitLifecycleResult {
        approved_hosts: approved_hosts.clone(),
        projected_hosts: Vec::new(),
    };
    if request.dry_run {
        return Ok(InitOutput::DryRun {
            plan,
            overlay,
            lifecycle: planned_lifecycle,
        });
    }

    let mut report = InitReport::new("project-init");
    for directory in &plan.directories {
        match std::fs::create_dir_all(directory) {
            Ok(_) => report.add(InitFinding::pass(
                format!(
                    "project-init-dir-{}",
                    sanitize_name(&directory.to_string_lossy())
                ),
                format!("directory ready: {}", directory.display()),
            )),
            Err(error) => report.add(InitFinding::fail(
                format!(
                    "project-init-dir-{}",
                    sanitize_name(&directory.to_string_lossy())
                ),
                format!("cannot create directory: {}", directory.display()),
                error.to_string(),
            )),
        }
    }
    for file in &plan.files {
        report.add(write_project_init_file(file, &plan.append_files));
    }
    for warning in &plan.warnings {
        report.add(InitFinding::warn(
            format!("project-init-warning-{}", sanitize_name(warning)),
            warning,
            "project init completed with a warning",
        ));
    }
    let mut lifecycle = planned_lifecycle;
    if report.passed() {
        for host in &approved_hosts {
            let result = crate::lifecycle_projection::LifecycleProjection::new(&plan.target, host)
                .and_then(|projection| projection.install().map(|_| projection.path()));
            match result {
                Ok(path) => {
                    lifecycle.projected_hosts.push(host.clone());
                    report.add(InitFinding::pass(
                        format!("project-init-lifecycle-{host}"),
                        format!("workspace lifecycle ready: {}", path.display()),
                    ));
                }
                Err(error) => {
                    report.add(InitFinding::fail(
                        format!("project-init-lifecycle-{host}"),
                        format!("could not install {host} workspace lifecycle"),
                        format!(
                            "{error}; rerun `ags agents govern --agent {host} --target '{}' --apply`",
                            plan.target.display()
                        ),
                    ));
                }
            }
        }
        if !lifecycle.projected_hosts.is_empty() {
            match crate::lifecycle_projection::record_lifecycle_manifest(&plan.target) {
                Ok(path) => report.add(InitFinding::pass(
                    "project-init-lifecycle-manifest",
                    format!("workspace lifecycle manifest current: {}", path.display()),
                )),
                Err(error) => report.add(InitFinding::fail(
                    "project-init-lifecycle-manifest",
                    "could not record workspace lifecycle manifest",
                    error,
                )),
            }
        } else if approved_hosts.is_empty() {
            report.add(InitFinding::info(
                "project-init-lifecycle-hosts",
                "approved lifecycle hosts: none",
            ));
        }
    }
    if overlay_mode == OverlayMode::Local {
        let mut pure_candidates = plan
            .files
            .iter()
            .filter(|file| {
                !matches!(
                    file.path.file_name().and_then(|name| name.to_str()),
                    Some("AGENTS.md" | "CLAUDE.md")
                )
            })
            .map(|file| (file.path.clone(), file.content.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        for host in &lifecycle.projected_hosts {
            if let Ok(projection) =
                crate::lifecycle_projection::LifecycleProjection::new(&plan.target, host)
            {
                if let Ok(rendered) = projection.render(None) {
                    pure_candidates.push((projection.path(), rendered.into_bytes()));
                }
            }
        }
        for finding in untrack_pure_overlay_files(&plan.target, &pure_candidates) {
            report.add(finding);
        }
    }
    for finding in apply_overlay(&overlay) {
        report.add(finding);
    }
    let preflight = ags_workspace_facts::run_session_preflight(
        &plan.target,
        &ags_workspace_facts::AgentType::Codex,
    );
    let managed_project_registration =
        if should_register_project(report.passed(), preflight.exit_code) {
            register_managed_project(
                &request.runtime_home,
                &plan.target,
                &plan.slug,
                request.now,
                &mut report,
            )
        } else {
            None
        };
    let managed_project_receipt = None;
    Ok(InitOutput::Applied {
        plan,
        overlay,
        report,
        preflight: Box::new(preflight),
        managed_project_registration,
        managed_project_receipt,
        lifecycle,
    })
}

pub(crate) fn should_register_project(report_passed: bool, preflight_exit: i32) -> bool {
    report_passed && preflight_exit == 0
}
