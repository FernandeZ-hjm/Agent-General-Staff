use super::*;

// ── Public check functions ───────────────────────────────────────────────

/// Run `git status --porcelain` and report uncommitted changes.
pub fn git_status_check(repo_root: &Path) -> Finding {
    git_status_check_with_command(repo_root, std::ffi::OsStr::new("git"))
}

fn git_status_check_with_command(repo_root: &Path, git: &std::ffi::OsStr) -> Finding {
    let repository = match Command::new(git)
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo_root)
        .env("LC_ALL", "C")
        .output()
    {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == "true" =>
        {
            true
        }
        Ok(output)
            if String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("not a git repository") =>
        {
            return Finding::skip(
                "git-status",
                "target is not a Git worktree; Git status is not applicable",
            );
        }
        Ok(output) => {
            return Finding::fail(
                "git-status",
                "Git repository detection failed",
                format!(
                    "git exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Finding::skip(
                "git-status",
                "Git CLI is not installed; Git status is not applicable",
            );
        }
        Err(error) => {
            return Finding::fail(
                "git-status",
                "Git repository detection failed",
                format!("Failed to run git: {error}"),
            );
        }
    };
    debug_assert!(repository);
    match Command::new(git)
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .env("LC_ALL", "C")
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Finding::fail(
                    "git-status",
                    "git status failed",
                    format!(
                        "git exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let changed: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
            if changed.is_empty() {
                Finding::pass("git-status", "working tree clean")
            } else {
                Finding::warn(
                    "git-status",
                    format!("{} uncommitted file(s)", changed.len()),
                    format!("Changed: {}", changed.join(", ")),
                )
            }
        }
        Err(e) => Finding::fail(
            "git-status",
            "git not available",
            format!("Failed to run git: {e}"),
        ),
    }
}

pub(super) fn project_integration_check(
    identity: &ags_workspace_facts::ProjectIdentity,
) -> Finding {
    use ags_workspace_facts::IntegrationStatus;

    match identity.integration_status {
        IntegrationStatus::Suite => Finding::pass(
            "project-integration",
            "target is the AGS suite authority workspace",
        ),
        IntegrationStatus::Integrated => Finding::pass(
            "project-integration",
            "target is registered with a complete AGS integration identity",
        ),
        IntegrationStatus::Partial => Finding::fail(
            "project-integration",
            "target has a partial AGS integration",
            format!("Gaps: {}", identity.gaps.join("; ")),
        ),
        IntegrationStatus::NotIntegrated => Finding::fail(
            "project-integration",
            "target is not managed by AGS",
            "Run `ags init --target <project>` before using it as a governed project.",
        ),
    }
}

pub(super) fn project_protocol_check(repo_root: &Path) -> Finding {
    let status = ags_workspace_facts::check_protocol_status(repo_root);
    if !status.failures.is_empty() {
        Finding::fail(
            "project-protocol",
            "AGS protocol or validator projection is incomplete",
            status.failures.join("; "),
        )
    } else if !status.warnings.is_empty() {
        Finding::warn(
            "project-protocol",
            format!(
                "AGS protocol projection is usable with {} warning(s)",
                status.warnings.len()
            ),
            status.warnings.join("; "),
        )
    } else {
        Finding::pass(
            "project-protocol",
            format!(
                "AGS protocol projection complete ({}/{} files, validator available)",
                status.present_count,
                status.files.len()
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_git_workspace_is_not_applicable() {
        let target = tempfile::tempdir().unwrap();
        let finding = git_status_check(target.path());
        assert_eq!(finding.status, ags_lifecycle::setup::SetupCheckStatus::Skip);
        assert_eq!(finding.check_name, "git-status");
    }

    #[test]
    fn missing_git_is_not_applicable() {
        let target = tempfile::tempdir().unwrap();
        let finding = git_status_check_with_command(
            target.path(),
            std::ffi::OsStr::new("ags-definitely-missing-git"),
        );
        assert_eq!(finding.status, ags_lifecycle::setup::SetupCheckStatus::Skip);
    }

    #[test]
    fn real_git_worktree_with_changes_warns() {
        let target = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(target.path())
            .status()
            .unwrap()
            .success());
        std::fs::write(target.path().join("untracked.txt"), "fixture\n").unwrap();
        let finding = git_status_check(target.path());
        assert_eq!(finding.status, ags_lifecycle::setup::SetupCheckStatus::Warn);
    }

    #[cfg(unix)]
    #[test]
    fn detected_repository_status_failure_is_a_product_failure() {
        use std::os::unix::fs::PermissionsExt;
        let target = tempfile::tempdir().unwrap();
        let fake = target.path().join("git-fixture");
        std::fs::write(
            &fake,
            "#!/bin/sh\nif [ \"$1\" = rev-parse ]; then echo true; exit 0; fi\necho injected-status-failure >&2\nexit 2\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake, permissions).unwrap();
        let finding = git_status_check_with_command(target.path(), fake.as_os_str());
        assert_eq!(finding.status, ags_lifecycle::setup::SetupCheckStatus::Fail);
        assert!(finding
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("injected-status-failure"));
    }
}
