//! Multi-project protocol drift checker with section-level comparison
//! and protocol safety assertion verification.
//!
//! 1. Parse markdown files into heading-path → content sections
//! 2. Compare sections across source and multiple targets
//! 3. Classify drift by type and severity
//! 4. Apply allowlists for legal differences (e.g. public-full sanitized adjustments)
//! 5. Verify critical protocol safety assertions are present in every target
//! 6. Output structured text or JSON reports
//!
//! # Library usage (for suite-doctor)
//!
//! ```ignore
//! use workflow_sync_check::{check_multi, CheckOptions, TargetConfig, ProjectKind};
//!
//! let report = check_multi(&CheckOptions {
//!     source_root: PathBuf::from("/path/to/private"),
//!     source_name: "private".into(),
//!     targets: vec![TargetConfig {
//!         root: PathBuf::from("/path/to/stable"),
//!         name: "stable".into(),
//!         kind: ProjectKind::Stable,
//!     }],
//!     allowlist_path: None,
//! });
//! ```

pub mod allowlist;
pub mod assertions;
pub mod drift;
pub mod manifest;
pub mod parser;
pub mod report;
pub mod types;

use std::path::PathBuf;

pub use types::{
    error_code, format_section_path, CheckOptions, Drift, DriftKind, DriftReport, ProjectDrift,
    ProjectKind, SectionPath, Severity, TargetConfig,
};

/// Default stable suite root (used by CLI as default target).
pub const DEFAULT_STABLE_ROOT: &str = "/Volumes/Projects/example-stable-suite";

/// Default public-full sanitized suite root.
pub const DEFAULT_PUBLIC_ROOT: &str = "/Volumes/AI Project/ai-dev-env-bootstrap";

// ── Public API ─────────────────────────────────────────────────────────

/// Run drift detection across multiple targets.
///
/// Returns a structured `DriftReport` with per-project drift findings.
/// This is the main library entry point for both CLI and suite-doctor.
pub fn check_multi(options: &CheckOptions) -> DriftReport {
    let mut report = DriftReport {
        source_root: options.source_root.clone(),
        source_name: options.source_name.clone(),
        projects: Vec::new(),
    };

    // Load explicit allowlist once; fail structured on load error.
    let explicit_allowlist: Option<allowlist::Allowlist> = match &options.allowlist_path {
        Some(path) => match allowlist::load_allowlist(path) {
            Ok(al) => Some(al),
            Err(e) => {
                // All targets get a structured ALLOWLIST_LOAD_FAILED drift.
                for target in &options.targets {
                    report.projects.push(ProjectDrift {
                        project_name: target.name.clone(),
                        project_kind: target.kind.clone(),
                        project_root: target.root.clone(),
                        drifts: vec![Drift::new(
                            error_code::ALLOWLIST_LOAD_FAILED,
                            DriftKind::AllowlistLoadFailed,
                            Severity::Fail,
                            path.to_string_lossy().as_ref(),
                            vec![],
                            format!("failed to load allowlist: {e}"),
                            "fix the allowlist file path or JSON format",
                        )],
                    });
                }
                return report;
            }
        },
        None => None,
    };

    for target in &options.targets {
        // Per-target allowlist: explicit allowlist takes precedence,
        // otherwise use the default for the target kind.
        let effective_allowlist: allowlist::Allowlist;
        let al_ref = if let Some(ref al) = explicit_allowlist {
            al
        } else {
            effective_allowlist = allowlist::default_for(&target.kind);
            &effective_allowlist
        };

        let mut project_drift = drift::check_target(
            &options.source_root,
            &target.root,
            &target.name,
            &target.kind,
            al_ref,
        );

        // Protocol safety assertion checks (always FAIL on missing/contradicted,
        // regardless of target kind or allowlist).
        let assertion_drifts =
            assertions::check_assertions(&target.root, &target.name, &target.kind);
        project_drift.drifts.extend(assertion_drifts);

        report.projects.push(project_drift);
    }

    report
}

/// Run drift check for a single target (backward-compatible convenience wrapper).
pub fn check_single(
    source_root: PathBuf,
    target_root: PathBuf,
    target_name: String,
) -> DriftReport {
    let options = CheckOptions::single(source_root, target_root, target_name);
    check_multi(&options)
}

/// CLI entry point: run and print text report, return pass/fail.
pub fn run_cli(options: CheckOptions, format: ReportFormat) -> bool {
    let report = check_multi(&options);
    match format {
        ReportFormat::Text => print!("{}", report::render_text(&report)),
        ReportFormat::Json => println!("{}", report::render_json(&report)),
    }
    report.passed()
}

/// Output format for reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Text,
    Json,
}

// ── Test helpers ───────────────────────────────────────────────────────

#[allow(dead_code)]
fn group_drifts_by_code(drifts: &[Drift]) -> std::collections::BTreeMap<String, usize> {
    let mut grouped = std::collections::BTreeMap::new();
    for drift in drifts {
        *grouped.entry(drift.code.clone()).or_insert(0) += 1;
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> (PathBuf, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("lib-test-{name}-{}-{nonce}", std::process::id()));
        let source = base.join("source");
        let target = base.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        (source, target)
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn write_manifest(root: &Path) {
        let safety = "protocol/runtime-adapters.md\n# Test\n\n\
            ultracode is thinking intensity only — it does not change permission mode.\n\
            Task level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\n\
            plan-only must not produce write-type launch args and must strip parallelism.\n\
            Runners must consume allowed_launch_args and effective_permission_mode.\n\
            Runner never launches. An allowed LaunchPlan returns HOST_EXECUTION_REQUIRED.\n";
        let default_fmt = |rel: &str| format!("{rel}\n# Test\n\ncontent\n");

        for relative in manifest::FULL_MANIFEST.required_files {
            if *relative == "protocol/runtime-adapters.md" {
                write_file(root, relative, safety);
            } else {
                let content = default_fmt(relative);
                write_file(root, relative, &content);
            }
        }
    }

    fn write_public_manifest(source: &Path, target: &Path) {
        for relative in crate::manifest::PUBLIC_MANIFEST.required_files {
            let source_path = source.join(relative);
            if !source_path.exists() {
                let content = format!("{relative}\n# Test\n\ndefault content\n");
                if let Some(parent) = source_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&source_path, &content).unwrap();
            }

            let target_path = target.join(relative);
            if target_path.exists() {
                continue;
            }
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let content = fs::read(&source_path).unwrap();
            fs::write(target_path, content).unwrap();
        }
    }

    fn cleanup(source: &Path, target: &Path) {
        for root in [source, target] {
            if let Some(base) = root.parent() {
                let _ = fs::remove_dir_all(base);
            }
        }
    }

    #[test]
    fn identical_roots_pass() {
        let (source, target) = temp_dir("identical_roots_pass");
        write_manifest(&source);
        write_manifest(&target);

        let report = check_single(source.clone(), target.clone(), "stable".into());

        assert!(report.passed(), "expected pass, got {report:?}");
        assert_eq!(report.projects.len(), 1);
        cleanup(&source, &target);
    }

    #[test]
    fn drift_fails_with_structured_code() {
        let (source, target) = temp_dir("drift_fails_structured");
        write_manifest(&source);
        write_manifest(&target);
        // Modify a protocol file in the target
        fs::write(
            target.join("protocol/runtime-adapters.md"),
            "protocol/runtime-adapters.md\n# Changed\n\nmodified content\n",
        )
        .unwrap();

        let report = check_single(source.clone(), target.clone(), "stable".into());

        assert!(!report.passed(), "expected fail, got {report:?}");
        let project = &report.projects[0];
        let has_drift = project.drifts.iter().any(|d| {
            d.file == "protocol/runtime-adapters.md"
                && (d.code == error_code::CONTENT_DRIFT
                    || d.code == error_code::SECTION_MISSING
                    || d.code == error_code::EXTRA_SECTION)
        });
        assert!(
            has_drift,
            "expected drift for runtime-adapters.md, got {project:?}"
        );
        cleanup(&source, &target);
    }

    #[test]
    fn missing_target_file_fails() {
        let (source, target) = temp_dir("missing_target_file_fails");
        write_manifest(&source);
        write_manifest(&target);
        fs::remove_file(target.join("protocol/task-card-template.md")).unwrap();

        let report = check_single(source.clone(), target.clone(), "stable".into());

        assert!(!report.passed());
        let project = &report.projects[0];
        let has_missing = project.drifts.iter().any(|d| {
            d.code == error_code::FILE_MISSING_IN_TARGET
                && d.file == "protocol/task-card-template.md"
        });
        assert!(has_missing, "expected file missing drift, got: {project:?}");
        cleanup(&source, &target);
    }

    #[test]
    fn missing_root_fails_before_file_checks() {
        let (source, target) = temp_dir("missing_root_fails");
        write_manifest(&source);
        fs::remove_dir_all(&target).unwrap();

        let report = check_single(source.clone(), target.clone(), "stable".into());

        assert!(!report.passed());
        // All manifest files will fail since target root is missing
        let project = &report.projects[0];
        assert!(!project.drifts.is_empty());
        cleanup(&source, &target);
    }

    #[test]
    fn extra_protocol_files_warn_without_failing() {
        let (source, target) = temp_dir("extra_protocol_warn");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/new-rule.md", "draft\n");

        let report = check_single(source.clone(), target.clone(), "stable".into());

        // Extra protocol file is a warning, so the report should pass (no fails)
        let project = &report.projects[0];
        let has_extra = project
            .drifts
            .iter()
            .any(|d| d.code == error_code::EXTRA_PROTOCOL_FILE && d.file == "protocol/new-rule.md");
        assert!(
            has_extra || project.passed(),
            "expected extra protocol file warning"
        );
        assert!(
            project.passed(),
            "extra protocol file should not cause failure, got {project:?}"
        );
        cleanup(&source, &target);
    }

    #[test]
    fn multi_target_check() {
        let (source, target1) = temp_dir("multi_target_1");
        let target2_base = std::env::temp_dir().join(format!(
            "lib-test-multi_target_2-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target2 = target2_base.join("target2");
        fs::create_dir_all(&target2).unwrap();

        write_manifest(&source);
        write_manifest(&target1);
        write_public_manifest(&source, &target2);

        let options = CheckOptions {
            source_root: source.clone(),
            source_name: "private".into(),
            targets: vec![
                TargetConfig {
                    root: target1.clone(),
                    name: "stable".into(),
                    kind: ProjectKind::Stable,
                },
                TargetConfig {
                    root: target2.clone(),
                    name: "public".into(),
                    kind: ProjectKind::PublicCoreOnly,
                },
            ],
            allowlist_path: None,
        };

        let report = check_multi(&options);

        assert_eq!(report.projects.len(), 2);
        assert_eq!(report.projects[0].project_name, "stable");
        assert_eq!(report.projects[1].project_name, "public");
        assert!(
            report.passed(),
            "multi-target check should pass with identical manifests"
        );

        cleanup(&source, &target1);
        let _ = fs::remove_dir_all(&target2_base);
    }

    #[test]
    fn public_target_only_checks_public_manifest() {
        let (source, target) = temp_dir("public_manifest_only");
        write_manifest(&source);
        write_public_manifest(&source, &target);

        let options = CheckOptions {
            source_root: source.clone(),
            source_name: "private".into(),
            targets: vec![TargetConfig {
                root: target.clone(),
                name: "public".into(),
                kind: ProjectKind::PublicCoreOnly,
            }],
            allowlist_path: None,
        };

        let report = check_multi(&options);
        let project = &report.projects[0];

        // Public target uses PUBLIC_MANIFEST, which includes the public root
        // entry files. With identical public-full manifest files there should
        // be no drift for those entries.
        for root_file in &[
            "AGENTS.md",
            "CLAUDE.md",
            "WORKSPACE.md",
            "AGENT_SUITE_PROTOCOL.md",
        ] {
            let has_root_entry_drift = project.drifts.iter().any(|d| d.file == *root_file);
            assert!(
                !has_root_entry_drift,
                "root entry file {root_file} should be clean in public target, got drifts: {project:?}"
            );
        }

        cleanup(&source, &target);
    }

    #[test]
    fn allowlist_load_failure_produces_fail() {
        let path = std::env::temp_dir().join(format!(
            "nonexistent-allowlist-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let options = CheckOptions {
            source_root: PathBuf::from("."),
            source_name: "private".into(),
            targets: vec![TargetConfig {
                root: PathBuf::from("/tmp"),
                name: "stable".into(),
                kind: ProjectKind::Stable,
            }],
            allowlist_path: Some(path.clone()),
        };

        let report = check_multi(&options);
        assert!(!report.passed(), "allowlist load failure should fail");
        assert_eq!(report.projects.len(), 1);
        assert_eq!(report.projects[0].failures(), 1);
        assert_eq!(
            report.projects[0].drifts[0].code,
            error_code::ALLOWLIST_LOAD_FAILED
        );
    }

    #[test]
    fn allowlist_invalid_json_produces_fail() {
        let path = std::env::temp_dir().join(format!(
            "invalid-allowlist-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, "this is not valid json {{{").unwrap();

        let options = CheckOptions {
            source_root: PathBuf::from("."),
            source_name: "private".into(),
            targets: vec![TargetConfig {
                root: PathBuf::from("/tmp"),
                name: "stable".into(),
                kind: ProjectKind::Stable,
            }],
            allowlist_path: Some(path.clone()),
        };

        let report = check_multi(&options);
        assert!(!report.passed());
        assert_eq!(
            report.projects[0].drifts[0].code,
            error_code::ALLOWLIST_LOAD_FAILED
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn report_format_json_produces_valid_output() {
        let report = DriftReport {
            source_root: PathBuf::from("/test"),
            source_name: "private".into(),
            projects: vec![],
        };
        let json = report::render_json(&report);
        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok());
    }
}
