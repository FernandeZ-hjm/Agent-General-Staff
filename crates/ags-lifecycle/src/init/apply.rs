//! Project initialization file-write transaction.

use super::model::{InitFile, InitFinding, AGS_VERSION};
use super::plan::{append_content_present, sanitize_name};

pub(crate) fn write_project_init_file(
    file: &InitFile,
    append_candidates: &[InitFile],
) -> InitFinding {
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

    if file.path.exists() {
        if let Some(append) = append_candidates
            .iter()
            .find(|candidate| candidate.path == file.path)
        {
            match std::fs::read_to_string(&file.path) {
                Ok(existing) if append_content_present(&existing, &append.content) => {
                    return InitFinding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(existing)
                    if existing.contains("Agent Governance Suite")
                        || existing.contains(&format!("AGS {AGS_VERSION}")) =>
                {
                    return InitFinding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(_) => {
                    if let Err(e) = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&file.path)
                        .and_then(|mut f| {
                            use std::io::Write;
                            f.write_all(append.content.as_bytes())
                        })
                    {
                        return InitFinding::fail(
                            format!(
                                "project-init-{}",
                                sanitize_name(&file.path.to_string_lossy())
                            ),
                            format!("append failed: {}", file.path.display()),
                            e.to_string(),
                        );
                    }
                    return InitFinding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("appended AGS block: {}", file.path.display()),
                    );
                }
                Err(e) => {
                    return InitFinding::fail(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("read failed: {}", file.path.display()),
                        e.to_string(),
                    );
                }
            }
        }

        return InitFinding::pass(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("kept existing: {}", file.path.display()),
        );
    }

    if let Err(e) = std::fs::write(&file.path, &file.content) {
        return InitFinding::fail(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("write failed: {}", file.path.display()),
            e.to_string(),
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
        format!("written: {}", file.path.display()),
    )
}
