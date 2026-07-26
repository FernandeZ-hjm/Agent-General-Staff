//! Thin `ags init` CLI adapter (五段链路第 4 段).

use crate::context::{default_private_runtime_home, unix_timestamp};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::{Path, PathBuf};

pub(crate) use ags_lifecycle::init::ManagedProjectRefresh;

fn register_managed_project(
    target: &Path,
    slug: &str,
    report: &mut ags_lifecycle::init::InitReport,
) -> Option<PathBuf> {
    use crate::managed_projects as mp;
    let registry_path = mp::registry_path(&default_private_runtime_home());
    let mut registry = match mp::load(&registry_path) {
        Ok(registry) => registry,
        Err(error) => {
            report.add(ags_lifecycle::init::InitFinding::warn(
                "managed-project-registry",
                "managed-projects.yaml is malformed; reporting drift instead of overwriting",
                error,
            ));
            return None;
        }
    };
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let path = canonical.display().to_string();
    let is_git = std::process::Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let origin = if is_git {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&canonical)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    } else {
        None
    };
    let vcs = if is_git {
        mp::ProjectVcs::Git
    } else {
        mp::ProjectVcs::None
    };
    let entry = mp::describe_project(
        path.clone(),
        slug.to_string(),
        unix_timestamp(),
        vcs,
        origin,
    );
    let change = mp::upsert(&mut registry, entry);
    if change == mp::RegistryChange::Unchanged {
        report.add(ags_lifecycle::init::InitFinding::pass(
            "managed-project-registry",
            format!("already registered: {path}"),
        ));
        return None;
    }
    if let Some(parent) = registry_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&registry_path, mp::render_yaml(&registry)) {
        report.add(ags_lifecycle::init::InitFinding::warn(
            "managed-project-registry",
            "could not write managed-projects.yaml",
            error.to_string(),
        ));
        return None;
    }
    let verb = if change == mp::RegistryChange::Added {
        "added"
    } else {
        "refreshed"
    };
    report.add(ags_lifecycle::init::InitFinding::pass(
        "managed-project-registry",
        format!("registered in managed-projects.yaml: {path} ({verb})"),
    ));
    let receipt = ags_evidence::build_action_receipt(
        "init-register-project",
        Some(&path),
        ags_evidence::GateResult {
            decision: "allow".to_string(),
            reason: Some("ags init managed-project registration".to_string()),
        },
        vec![],
        vec![ags_evidence::ReceiptWrite {
            op: "overwrite".to_string(),
            path: registry_path.display().to_string(),
            from: None,
            backup: None,
            detail: format!("managed-projects.yaml upsert ({verb})"),
        }],
        vec![],
        vec![],
        ags_evidence::RollbackPlan::backup_restore(vec![]),
        "applied",
        true,
    );
    emit_ags_action_receipt(&receipt).ok()
}

pub(crate) fn refresh_managed_project(
    target: &Path,
    slug: &str,
    source_root: &Path,
    apply: bool,
) -> ManagedProjectRefresh {
    ags_lifecycle::init::refresh_managed_project(target, slug, source_root, apply)
}

pub(crate) fn run(
    target: &Path,
    slug: Option<String>,
    dry_run: bool,
    format: &str,
    mode: &str,
    migrate_tracked_overlay: bool,
) {
    let request = ags_lifecycle::init::InitRequest {
        target: target.to_path_buf(),
        slug,
        dry_run,
        mode: mode.to_string(),
        migrate_tracked_overlay,
    };
    match ags_lifecycle::init::execute(request, register_managed_project) {
        Ok(output) => {
            if format == "json" {
                println!("{}", output.render_json());
            } else {
                println!("{}", output.render_text());
            }
            if !output.succeeded() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
