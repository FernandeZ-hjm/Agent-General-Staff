//! Project initialization request orchestration.

use super::apply::write_project_init_file;
use super::model::{InitFinding, InitReport, PROJECT_INIT_SCHEMA};
use super::overlay::{
    apply_overlay, compute_overlay_plan, overlay_json, render_overlay_text, OverlayMode,
    OverlayPlan,
};
use super::plan::{project_init_plan, sanitize_name, ProjectInitPlan};
use super::render::{render_init_report_text, render_project_init_json, render_project_init_text};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct InitRequest {
    pub target: PathBuf,
    pub slug: Option<String>,
    pub dry_run: bool,
    pub mode: String,
    pub migrate_tracked_overlay: bool,
}

pub enum InitOutput {
    DryRun {
        plan: ProjectInitPlan,
        overlay: OverlayPlan,
    },
    Applied {
        plan: ProjectInitPlan,
        overlay: OverlayPlan,
        report: InitReport,
        preflight: Box<ags_workspace_facts::SessionPreflight>,
        managed_project_receipt: Option<PathBuf>,
    },
}

impl InitOutput {
    pub fn render_text(&self) -> String {
        match self {
            Self::DryRun { plan, overlay } => format!(
                "{}\n\n{}",
                render_project_init_text(plan, true),
                render_overlay_text(overlay)
            ),
            Self::Applied {
                plan,
                overlay,
                report,
                preflight,
                ..
            } => format!(
                "{}\n\n{}\n\n{}\n\n{}",
                render_project_init_text(plan, false),
                render_overlay_text(overlay),
                render_init_report_text(report),
                ags_workspace_facts::render_session_preflight_text(preflight)
            ),
        }
    }

    pub fn render_json(&self) -> String {
        let value = match self {
            Self::DryRun { plan, overlay } => {
                let mut value: serde_json::Value =
                    serde_json::from_str(&render_project_init_json(plan, true)).unwrap_or_default();
                if let Some(object) = value.as_object_mut() {
                    object.insert("overlay".to_string(), overlay_json(overlay));
                }
                value
            }
            Self::Applied {
                plan,
                overlay,
                report,
                preflight,
                managed_project_receipt,
            } => serde_json::json!({
                "schema_version": PROJECT_INIT_SCHEMA,
                "plan": serde_json::from_str::<serde_json::Value>(&render_project_init_json(plan, false)).unwrap_or_default(),
                "overlay": overlay_json(overlay),
                "report": report,
                "preflight": preflight,
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
}

pub fn execute(
    request: InitRequest,
    mut register_project: impl FnMut(&Path, &str, &mut InitReport) -> Option<PathBuf>,
) -> Result<InitOutput, String> {
    let overlay_mode = OverlayMode::parse(&request.mode);
    if request.migrate_tracked_overlay && overlay_mode == OverlayMode::Shared {
        return Err(
            "ags init: --migrate-tracked-overlay requires --mode local (shared/tracked overlays stay committed)"
                .to_string(),
        );
    }
    if !request.target.exists() {
        return Err(format!(
            "ags init: target does not exist — {}",
            request.target.display()
        ));
    }
    let plan = project_init_plan(&request.target, request.slug);
    let overlay = compute_overlay_plan(
        &plan.target,
        &plan.files,
        overlay_mode,
        request.migrate_tracked_overlay,
    );
    if request.dry_run {
        return Ok(InitOutput::DryRun { plan, overlay });
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
    for finding in apply_overlay(&overlay) {
        report.add(finding);
    }
    let preflight = ags_workspace_facts::run_session_preflight(
        &plan.target,
        &ags_workspace_facts::AgentType::Codex,
    );
    let managed_project_receipt = if should_register_project(report.passed(), preflight.exit_code) {
        register_project(&plan.target, &plan.slug, &mut report)
    } else {
        None
    };
    Ok(InitOutput::Applied {
        plan,
        overlay,
        report,
        preflight: Box::new(preflight),
        managed_project_receipt,
    })
}

pub(crate) fn should_register_project(report_passed: bool, preflight_exit: i32) -> bool {
    report_passed && preflight_exit == 0
}
