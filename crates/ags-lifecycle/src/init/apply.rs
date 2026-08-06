//! Project initialization file-write transaction.

use super::managed_projects::desired_project_file_content;
use super::model::{InitFile, InitFinding};
use super::plan::{sanitize_name, ProjectInitPlan};

pub(crate) fn write_project_init_file(plan: &ProjectInitPlan, file: &InitFile) -> InitFinding {
    if let Some(parent) = file.path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return InitFinding::fail(
                format!(
                    "project-init-{}",
                    sanitize_name(&file.path.to_string_lossy())
                ),
                format!("cannot create directory {}", parent.display()),
                e.to_string(),
            );
        }
    }

    let after = match desired_project_file_content(plan, file) {
        Ok(Some(after)) => after,
        Ok(None) => {
            return InitFinding::pass(
                format!(
                    "project-init-{}",
                    sanitize_name(&file.path.to_string_lossy())
                ),
                format!("unchanged: {}", file.path.display()),
            )
        }
        Err(error) => {
            return InitFinding::fail(
                format!(
                    "project-init-{}",
                    sanitize_name(&file.path.to_string_lossy())
                ),
                format!("planning failed: {}", file.path.display()),
                error,
            )
        }
    };

    if let Err(error) = std::fs::write(&file.path, &after) {
        return InitFinding::fail(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("write failed: {}", file.path.display()),
            error.to_string(),
        );
    }

    #[cfg(unix)]
    if let Some(mode) = file.mode {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&file.path) {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(&file.path, perms);
        }
    }

    InitFinding::pass(
        format!(
            "project-init-{}",
            sanitize_name(&file.path.to_string_lossy())
        ),
        format!("projected: {}", file.path.display()),
    )
}
