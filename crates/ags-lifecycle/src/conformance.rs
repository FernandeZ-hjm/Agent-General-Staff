use crate::lifecycle_projection::{
    HostLifecycleProjection, WorkspaceLifecycleObservation, LIFECYCLE_MANIFEST_SCHEMA_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceCheck {
    pub check_name: String,
    pub status: ConformanceStatus,
    pub message: String,
    pub expected: String,
    pub observed: String,
    pub remediation: String,
}

impl ConformanceCheck {
    fn pass(name: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            check_name: name.into(),
            status: ConformanceStatus::Pass,
            message: message.clone(),
            expected: "current canonical state".to_string(),
            observed: message,
            remediation: "none".to_string(),
        }
    }

    fn fail(
        name: impl Into<String>,
        message: impl Into<String>,
        expected: impl Into<String>,
        observed: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            check_name: name.into(),
            status: ConformanceStatus::Fail,
            message: message.into(),
            expected: expected.into(),
            observed: observed.into(),
            remediation: remediation.into(),
        }
    }

    fn skip(name: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            check_name: name.into(),
            status: ConformanceStatus::Skip,
            message: message.clone(),
            expected: "optional host disabled or not installed".to_string(),
            observed: message,
            remediation: "none unless this host should be enabled".to_string(),
        }
    }
}

/// Render the shared lifecycle finding vocabulary from a previously collected
/// observation. Doctor uses this function to avoid repeating filesystem probes.
pub fn conformance_checks(observation: &WorkspaceLifecycleObservation) -> Vec<ConformanceCheck> {
    let mut checks = vec![manifest_check(
        &observation.effective_hosts,
        observation.manifest_current,
    )];
    checks.extend(observation.projections.iter().map(projection_check));
    checks.push(if observation.legacy_markers.is_empty() {
        ConformanceCheck::pass(
            "lifecycle-legacy-commands-absent",
            "no legacy or relative lifecycle commands detected",
        )
    } else {
        ConformanceCheck::fail(
            "lifecycle-legacy-commands-absent",
            "legacy lifecycle commands remain effective",
            "no legacy command markers",
            observation.legacy_markers.join(", "),
            "Migrate managed workspaces, then remove only AGS-owned user-level entries.",
        )
    });
    let mut duplicate_details = observation.duplicate_events.clone();
    duplicate_details.extend(
        observation
            .global_ags_owned_hosts
            .iter()
            .map(|host| format!("{host}:user-level-hook")),
    );
    checks.push(if duplicate_details.is_empty() {
        ConformanceCheck::pass(
            "lifecycle-duplicates-absent",
            "no duplicate effective AGS lifecycle hooks",
        )
    } else {
        ConformanceCheck::fail(
            "lifecycle-duplicates-absent",
            "duplicate or user-level lifecycle hooks detected",
            "one workspace-owned AGS hook per enabled host event",
            duplicate_details.join(", "),
            "Keep workspace projections and remove only AGS-owned user-level entries.",
        )
    });
    checks
}

fn manifest_check(effective_hosts: &[String], current: bool) -> ConformanceCheck {
    if effective_hosts.is_empty() && current {
        return ConformanceCheck::skip(
            "workspace-lifecycle-manifest-current",
            "no workspace lifecycle adapters are enabled",
        );
    }
    if current {
        ConformanceCheck::pass(
            "workspace-lifecycle-manifest-current",
            format!(
                "{} effective host adapter(s) have a current receipt",
                effective_hosts.len()
            ),
        )
    } else {
        ConformanceCheck::fail(
            "workspace-lifecycle-manifest-current",
            "workspace lifecycle receipt is missing, stale, or disagrees with desired state",
            format!(
                "{} recording the current path and desired hash for {}",
                LIFECYCLE_MANIFEST_SCHEMA_VERSION,
                effective_hosts.join(", ")
            ),
            "manifest does not match the canonical projection generator",
            "Re-apply each enabled workspace lifecycle adapter.",
        )
    }
}

fn projection_check(projection: &HostLifecycleProjection) -> ConformanceCheck {
    let name = format!("lifecycle-adapter-{}-current", projection.host);
    if projection.current {
        ConformanceCheck::pass(
            name,
            format!("{} is current", projection.config_path.display()),
        )
    } else {
        ConformanceCheck::fail(
            name,
            "workspace lifecycle adapter drift detected",
            format!(
                "hash={}, three events, canonical target",
                projection.desired_hash
            ),
            format!(
                "hash={:?}, file_present={}, events_complete={}, canonical_target={}, detail={}",
                projection.observed_hash,
                projection.file_present,
                projection.events_complete,
                projection.canonical_target,
                projection.detail
            ),
            format!(
                "Run `ags agents govern --agent {} --apply --target '{}'`.",
                projection.host,
                projection
                    .config_path
                    .ancestors()
                    .find(|path| path.join(".git").exists())
                    .unwrap_or(&projection.config_path)
                    .display()
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_macbook_start_hook_is_legacy_and_not_current() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let home = root.path().join("home");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"command":"python3 \"$HOME/.agents/scripts/context-memory-start.py\""}]}}"#,
        )
        .unwrap();
        let observation =
            crate::lifecycle_projection::observe_workspace_lifecycle(&workspace, &home, &[])
                .unwrap();
        let checks = conformance_checks(&observation);
        assert_eq!(
            checks
                .iter()
                .find(|check| check.check_name == "lifecycle-legacy-commands-absent")
                .unwrap()
                .status,
            ConformanceStatus::Fail
        );
        assert_eq!(
            checks
                .iter()
                .find(|check| check.check_name == "lifecycle-adapter-claude-code-current")
                .unwrap()
                .status,
            ConformanceStatus::Fail
        );
    }
}
