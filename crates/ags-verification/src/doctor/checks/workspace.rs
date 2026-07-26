use super::*;

pub(super) fn yaml_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value
        .as_mapping()
        .and_then(|map| map.get(YamlValue::String(key.to_string())))
}

// ── Public check functions ───────────────────────────────────────────────

/// Run `git status --porcelain` and report uncommitted changes.
pub fn git_status_check(repo_root: &Path) -> Finding {
    match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
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
