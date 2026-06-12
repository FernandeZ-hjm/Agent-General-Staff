//! Drift detection engine.
//!
//! Compares parsed markdown files between source and target at the section level,
//! classifies differences, applies allowlists, and produces drift findings.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::allowlist::{self, Allowlist};
use crate::manifest::{self};
use crate::parser;
use crate::types::*;

// ── Public entry point ─────────────────────────────────────────────────

/// Run drift detection for a single target against the source.
pub fn check_target(
    source_root: &Path,
    target_root: &Path,
    target_name: &str,
    target_kind: &ProjectKind,
    allowlist: &Allowlist,
) -> ProjectDrift {
    let manifest = manifest::manifest_for(target_kind);
    let mut drifts: Vec<Drift> = Vec::new();

    // Check only the files required by this target's manifest.
    // Public-full sanitized targets use PUBLIC_MANIFEST: the full public AGS
    // runtime, templates, scripts, and sanitized skeletons.
    for relative in manifest.required_files {
        check_file(source_root, target_root, relative, allowlist, &mut drifts);
    }

    // Check for extra protocol files not in manifest
    for extra in extra_protocol_files(source_root, target_root) {
        let code = error_code::EXTRA_PROTOCOL_FILE;
        if !is_allowed_file_level(allowlist, &extra, &DriftKind::ExtraProtocolFile) {
            drifts.push(Drift::new(
                code,
                DriftKind::ExtraProtocolFile,
                Severity::Warn,
                &extra,
                vec![],
                format!("protocol file not in sync manifest"),
                "review whether this file should be added to the manifest or removed",
            ));
        }
    }

    // Public-full sanitized ships the Rust AGS workspace, but must not ship
    // build output, preinstalled skills, local agent config, or private memory.
    if matches!(target_kind, ProjectKind::PublicCoreOnly) {
        check_public_forbidden_payload(target_root, &mut drifts);
    }

    ProjectDrift {
        project_name: target_name.to_string(),
        project_kind: target_kind.clone(),
        project_root: target_root.to_path_buf(),
        drifts,
    }
}

// ── File-level check ───────────────────────────────────────────────────

fn check_file(
    source_root: &Path,
    target_root: &Path,
    relative: &str,
    allowlist: &Allowlist,
    drifts: &mut Vec<Drift>,
) {
    let source_path = source_root.join(relative);
    let target_path = target_root.join(relative);

    let source_result = read_file(&source_path);
    let target_result = read_file(&target_path);

    match (source_result, target_result) {
        (Ok(source_text), Ok(target_text)) => {
            // Both files exist — do section-level comparison
            let source_parsed = parser::parse(relative, &source_text);
            let target_parsed = parser::parse(relative, &target_text);
            compare_sections(relative, &source_parsed, &target_parsed, allowlist, drifts);
        }
        (Err(_), Ok(_)) => {
            let code = error_code::FILE_MISSING_IN_SOURCE;
            if !is_allowed_file_level(allowlist, relative, &DriftKind::FileMissingInSource) {
                drifts.push(Drift::new(
                    code,
                    DriftKind::FileMissingInSource,
                    Severity::Fail,
                    relative,
                    vec![],
                    format!("file exists in target but not in source: {relative}"),
                    "restore the file in source or add to allowlist",
                ));
            }
        }
        (Ok(_), Err(_)) => {
            let code = error_code::FILE_MISSING_IN_TARGET;
            if is_allowed_file_level(allowlist, relative, &DriftKind::FileMissingInTarget) {
                drifts.push(Drift::new(
                    error_code::LEGAL_REDACTION,
                    DriftKind::LegalRedaction,
                    Severity::Info,
                    relative,
                    vec![],
                    format!("file absent from target (allowlisted): {relative}"),
                    "no action needed — legal redaction",
                ));
            } else {
                drifts.push(Drift::new(
                    code,
                    DriftKind::FileMissingInTarget,
                    Severity::Fail,
                    relative,
                    vec![],
                    format!("file missing in target: {relative}"),
                    "copy the file to target or add to allowlist",
                ));
            }
        }
        (Err(source_err), Err(target_err)) => {
            let code = error_code::FILE_READ_FAILED;
            drifts.push(Drift::new(
                code,
                DriftKind::FileMissingInTarget,
                Severity::Fail,
                relative,
                vec![],
                format!("cannot read source ({source_err}) or target ({target_err})"),
                "check file permissions and existence",
            ));
        }
    }
}

// ── Section-level comparison ───────────────────────────────────────────

fn compare_sections(
    relative: &str,
    source: &parser::ParsedFile,
    target: &parser::ParsedFile,
    allowlist: &Allowlist,
    drifts: &mut Vec<Drift>,
) {
    let source_paths: BTreeSet<_> = source.sections.keys().cloned().collect();
    let target_paths: BTreeSet<_> = target.sections.keys().cloned().collect();

    // Sections in source but missing from target
    for path in source_paths.difference(&target_paths) {
        if is_allowed_section(allowlist, relative, path, &DriftKind::SectionMissing) {
            drifts.push(Drift::new(
                error_code::LEGAL_REDACTION,
                DriftKind::LegalRedaction,
                Severity::Info,
                relative,
                path.clone(),
                format!(
                    "section absent from target (allowlisted): {}",
                    format_section_path(path)
                ),
                "no action needed — legal redaction",
            ));
        } else {
            drifts.push(Drift::new(
                error_code::SECTION_MISSING,
                DriftKind::SectionMissing,
                Severity::Fail,
                relative,
                path.clone(),
                format!("section missing in target: {}", format_section_path(path)),
                &format!(
                    "restore the section in target or add '{}' to the allowlist",
                    format_section_path(path)
                ),
            ));
        }
    }

    // Sections in target but not in source
    for path in target_paths.difference(&source_paths) {
        if is_allowed_section(allowlist, relative, path, &DriftKind::ExtraSection) {
            drifts.push(Drift::new(
                error_code::LEGAL_REDACTION,
                DriftKind::LegalRedaction,
                Severity::Info,
                relative,
                path.clone(),
                format!(
                    "extra section in target (allowlisted): {}",
                    format_section_path(path)
                ),
                "no action needed — legal redaction",
            ));
        } else {
            drifts.push(Drift::new(
                error_code::EXTRA_SECTION,
                DriftKind::ExtraSection,
                Severity::Warn,
                relative,
                path.clone(),
                format!(
                    "extra section in target not in source: {}",
                    format_section_path(path)
                ),
                "review whether this section should be backported to source or removed from target",
            ));
        }
    }

    // Content comparison for sections present in both
    for path in source_paths.intersection(&target_paths) {
        let source_content = source.sections.get(path).unwrap();
        let target_content = target.sections.get(path).unwrap();

        if source_content == target_content {
            continue; // identical
        }

        // Check if the difference is only allowable removable lines
        if content_diff_is_removable_only(allowlist, relative, source_content, target_content) {
            drifts.push(Drift::new(
                error_code::LEGAL_REDACTION,
                DriftKind::LegalRedaction,
                Severity::Info,
                relative,
                path.clone(),
                format!(
                    "content differs in section '{}' but only in allowlisted removable lines",
                    format_section_path(path)
                ),
                "no action needed — legal redaction",
            ));
            continue;
        }

        // Check if content drift is allowed for this section
        if is_allowed_section(allowlist, relative, path, &DriftKind::ContentDrift) {
            drifts.push(Drift::new(
                error_code::LEGAL_REDACTION,
                DriftKind::LegalRedaction,
                Severity::Info,
                relative,
                path.clone(),
                format!(
                    "content drift in section '{}' (allowlisted)",
                    format_section_path(path)
                ),
                "no action needed — legal redaction",
            ));
            continue;
        }

        // Check if the section order matches (structure drift)
        let source_pos = source.section_order.iter().position(|p| p == path);
        let target_pos = target.section_order.iter().position(|p| p == path);
        let same_order = source_pos.zip(target_pos).map_or(true, |(s, t)| s == t);

        let (kind, severity) = if same_order {
            (DriftKind::ContentDrift, Severity::Fail)
        } else {
            (DriftKind::StructureDrift, Severity::Fail)
        };

        drifts.push(Drift::new(
            if same_order {
                error_code::CONTENT_DRIFT
            } else {
                error_code::STRUCTURE_DRIFT
            },
            kind,
            severity,
            relative,
            path.clone(),
            format!(
                "content drift in section '{}': source {} bytes, target {} bytes",
                format_section_path(path),
                source_content.len(),
                target_content.len(),
            ),
            "review and reconcile the content difference between source and target",
        ));
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn read_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "file not found".to_string()
        } else {
            format!("read error: {e}")
        }
    })
}

fn is_allowed_file_level(allowlist: &Allowlist, file: &str, kind: &DriftKind) -> bool {
    allowlist::is_allowed(allowlist, file, &[], kind)
}

fn is_allowed_section(
    allowlist: &Allowlist,
    file: &str,
    path: &[String],
    kind: &DriftKind,
) -> bool {
    allowlist::is_allowed(allowlist, file, path, kind)
}

/// Check if content difference is only due to lines matching removable patterns.
fn content_diff_is_removable_only(
    allowlist: &Allowlist,
    file: &str,
    source_content: &str,
    target_content: &str,
) -> bool {
    // Filter out removable lines from source and compare to target
    let source_filtered: Vec<&str> = source_content
        .lines()
        .filter(|line| !allowlist::is_removable_line(allowlist, file, line))
        .collect();

    let target_lines: Vec<&str> = target_content.lines().collect();

    // Quick check: if lengths differ significantly, not just removable lines
    if (source_filtered.len() as isize - target_lines.len() as isize).abs() > 5 {
        return false;
    }

    // Normalize both and compare
    let source_normalized = source_filtered.join("\n");
    let target_normalized = target_lines.join("\n");

    source_normalized == target_normalized
}

/// Find protocol files in source or target not covered by any manifest.
fn extra_protocol_files(source_root: &Path, target_root: &Path) -> Vec<String> {
    let all_manifest = manifest::all_manifest_files();
    let mut extras = BTreeSet::new();

    for root in [source_root, target_root] {
        let protocol_dir = root.join("protocol");
        if let Ok(entries) = fs::read_dir(&protocol_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(relative) = path.strip_prefix(root) {
                        let rel = relative.to_string_lossy().replace('\\', "/");
                        if rel.starts_with("protocol/") && !all_manifest.contains(rel.as_str()) {
                            extras.insert(rel);
                        }
                    }
                }
            }
        }
    }

    extras.into_iter().collect()
}

fn check_public_forbidden_payload(target_root: &Path, drifts: &mut Vec<Drift>) {
    for forbidden in manifest::PUBLIC_FORBIDDEN_PAYLOAD {
        let candidate = target_root.join(forbidden.trim_end_matches('/'));
        if candidate.exists() && !is_git_ignored(target_root, &candidate) {
            drifts.push(Drift::new(
                error_code::PUBLIC_FORBIDDEN_PAYLOAD,
                DriftKind::PublicForbiddenPayload,
                Severity::Fail,
                forbidden,
                vec![],
                format!("private runtime or build payload present in public-full sanitized target: {forbidden}"),
                "remove this artifact from the public release; public-full may publish the Rust workspace and governance framework, but not build output, local agent state, preinstalled skills, or private memory",
            ));
        }
    }
}

fn is_git_ignored(root: &Path, candidate: &Path) -> bool {
    if !root.join(".git").exists() {
        return false;
    }

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("check-ignore")
        .arg("-q")
        .arg(candidate)
        .status();

    matches!(output, Ok(status) if status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── Fixture helpers ───────────────────────────────────────────

    fn temp_dir(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("drift-test-{name}-{}-{nonce}", std::process::id()));
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
            Heavy tasks need explicit approval, plan-only downgrade, and confirmation gate.\n\
            read-only and plan-only must not produce write-type launch args and must strip parallelism.\n\
            Runners must consume allowed_launch_args and effective_permission_mode.\n";
        let default_fmt = |rel: &str| format!("{rel}\n# Test\n\ndefault content\n");

        for relative in crate::manifest::FULL_MANIFEST.required_files {
            if *relative == "protocol/runtime-adapters.md" {
                write_file(root, relative, safety);
            } else {
                let content = default_fmt(relative);
                write_file(root, relative, &content);
            }
        }
    }

    fn cleanup(source: &Path, target: &Path) {
        for root in [source, target] {
            if let Some(base) = root.parent() {
                let _ = fs::remove_dir_all(base);
            }
        }
    }

    // ── Section-level tests ───────────────────────────────────────

    #[test]
    fn identical_files_produce_no_drifts() {
        let (source, target) = temp_dir("identical");
        write_manifest(&source);
        write_manifest(&target);

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        assert!(
            result.passed(),
            "identical manifests should pass, got {result:?}"
        );
        assert_eq!(result.failures(), 0);
        cleanup(&source, &target);
    }

    #[test]
    fn section_missing_in_target_produces_fail() {
        let source_content = "# Top\n\n## Keep\nkeep\n\n## Remove\nremove\n";
        let target_content = "# Top\n\n## Keep\nkeep\n";
        let (source, target) = temp_dir("section_missing");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/runtime-adapters.md", source_content);
        write_file(&target, "protocol/runtime-adapters.md", target_content);

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        let drift = result.drifts.iter().find(|d| {
            d.kind == DriftKind::SectionMissing && d.file == "protocol/runtime-adapters.md"
        });
        assert!(
            drift.is_some(),
            "expected section missing drift, got none. drifts: {result:?}"
        );
        assert_eq!(drift.unwrap().severity, Severity::Fail);
        cleanup(&source, &target);
    }

    #[test]
    fn content_drift_produces_fail() {
        let source_content = "# Top\n\noriginal content\n";
        let target_content = "# Top\n\nmodified content\n";
        let (source, target) = temp_dir("content_drift");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/runtime-adapters.md", source_content);
        write_file(&target, "protocol/runtime-adapters.md", target_content);

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        let drift = result.drifts.iter().find(|d| {
            d.kind == DriftKind::ContentDrift && d.file == "protocol/runtime-adapters.md"
        });
        assert!(
            drift.is_some(),
            "expected content drift, got none. drifts: {result:?}"
        );
        assert_eq!(drift.unwrap().severity, Severity::Fail);
        cleanup(&source, &target);
    }

    #[test]
    fn public_target_only_checks_public_manifest_files() {
        let (source, target) = temp_dir("public_only_public_manifest");
        write_manifest(&source);
        write_manifest(&target);

        let result = check_target(
            &source,
            &target,
            "public",
            &ProjectKind::PublicCoreOnly,
            &allowlist::default_public_allowlist(),
        );

        // Public target uses PUBLIC_MANIFEST: the public Rust runtime plus
        // public docs, templates, scripts, and empty governance skeletons.
        let checked_files: BTreeSet<&str> = result.drifts.iter().map(|d| d.file.as_str()).collect();
        let public_manifest: BTreeSet<&str> = crate::manifest::PUBLIC_MANIFEST
            .required_files
            .iter()
            .copied()
            .collect();

        // Every drift must be for a file in PUBLIC_MANIFEST, or be a
        // PUBLIC_FORBIDDEN_PAYLOAD drift (runtime/private payload).
        let forbidden: BTreeSet<&str> = crate::manifest::PUBLIC_FORBIDDEN_PAYLOAD
            .iter()
            .copied()
            .collect();
        for file in &checked_files {
            if forbidden.contains(file) {
                continue; // forbidden-payload drift — not in PUBLIC_MANIFEST
            }
            assert!(
                public_manifest.contains(file),
                "public target should not check non-public-manifest file: {file}"
            );
        }

        // Root entry files are part of the public-full manifest.
        for root_file in &[
            "AGENTS.md",
            "CLAUDE.md",
            "WORKSPACE.md",
            "AGENT_SUITE_PROTOCOL.md",
        ] {
            assert!(
                public_manifest.contains(root_file),
                "root file {root_file} must be checked for public-full target"
            );
        }

        cleanup(&source, &target);
    }

    #[test]
    fn public_target_with_identical_public_manifest_passes() {
        let (source, target) = temp_dir("public_identical_manifest");
        // Write only PUBLIC_MANIFEST files to both source and target
        for relative in crate::manifest::PUBLIC_MANIFEST.required_files {
            write_file(
                &source,
                relative,
                &format!("{relative}\n# Test\n\ncontent\n"),
            );
            write_file(
                &target,
                relative,
                &format!("{relative}\n# Test\n\ncontent\n"),
            );
        }

        let result = check_target(
            &source,
            &target,
            "public",
            &ProjectKind::PublicCoreOnly,
            &allowlist::default_public_allowlist(),
        );

        assert!(
            result.passed(),
            "public target with identical public manifest files should pass, got drifts: {:?}",
            result.drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        cleanup(&source, &target);
    }

    #[test]
    fn public_target_with_forbidden_runtime_payload_fails() {
        let (source, target) = temp_dir("public_forbidden_payload");
        for relative in crate::manifest::PUBLIC_MANIFEST.required_files {
            write_file(&source, relative, "same\n# Test\n\ncontent\n");
            write_file(&target, relative, "same\n# Test\n\ncontent\n");
        }
        write_file(&target, "target/release/ags", "binary\n");
        write_file(&target, "skill-packs/personal/demo/SKILL.md", "private\n");

        let result = check_target(
            &source,
            &target,
            "public",
            &ProjectKind::PublicCoreOnly,
            &allowlist::default_public_allowlist(),
        );

        let forbidden: Vec<&Drift> = result
            .drifts
            .iter()
            .filter(|d| d.kind == DriftKind::PublicForbiddenPayload)
            .collect();
        assert!(
            !forbidden.is_empty(),
            "public target must fail when forbidden runtime payload is present: {result:?}"
        );
        assert!(forbidden.iter().all(|d| d.severity == Severity::Fail));
        cleanup(&source, &target);
    }

    #[test]
    fn stable_target_may_contain_rust_workspace_payload() {
        let (source, target) = temp_dir("stable_rust_payload_allowed");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&target, "Cargo.toml", "[workspace]\n");
        write_file(&target, "crates/ags-cli/src/main.rs", "fn main() {}\n");

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        assert!(
            result
                .drifts
                .iter()
                .all(|d| d.kind != DriftKind::PublicForbiddenPayload),
            "stable target should not use public forbidden payload rule: {result:?}"
        );
        cleanup(&source, &target);
    }

    #[test]
    fn allowlisted_missing_section_produces_info() {
        let source_content = "\
# Runtime Adapters

## Default Profiles

### Codex Direct Execution
codex stuff

### Codex Stuff
other codex stuff
";
        let target_content = "\
# Runtime Adapters

## Default Profiles

### Codex Stuff
other codex stuff
";
        let (source, target) = temp_dir("allowlist_section");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/runtime-adapters.md", source_content);
        write_file(&target, "protocol/runtime-adapters.md", target_content);

        let result = check_target(
            &source,
            &target,
            "public",
            &ProjectKind::PublicCoreOnly,
            &allowlist::default_public_allowlist(),
        );

        let drift = result.drifts.iter().find(|d| {
            d.kind == DriftKind::LegalRedaction
                && d.file == "protocol/runtime-adapters.md"
                && d.section_path
                    .contains(&"Codex Direct Execution".to_string())
        });
        assert!(
            drift.is_some(),
            "expected legal redaction for missing section, got: {result:?}"
        );
        assert_eq!(drift.unwrap().severity, Severity::Info);
        cleanup(&source, &target);
    }

    #[test]
    fn missing_target_file_without_allowlist_fails() {
        let (source, target) = temp_dir("no_allowlist_file");
        write_manifest(&source);
        // target: missing CLAUDE.md (remove it after writing manifest)
        write_manifest(&target);
        fs::remove_file(target.join("CLAUDE.md")).unwrap();

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        let drift = result
            .drifts
            .iter()
            .find(|d| d.kind == DriftKind::FileMissingInTarget && d.file == "CLAUDE.md");
        assert!(
            drift.is_some(),
            "expected file missing drift, got: {result:?}"
        );
        assert_eq!(drift.unwrap().severity, Severity::Fail);
        cleanup(&source, &target);
    }

    #[test]
    fn removable_content_lines_are_info_not_fail() {
        let source_content = "\
# Config

Project path: /Volumes/AI Project/something
normal config line
machine-specific setting
";
        let target_content = "\
# Config

normal config line
";
        let (source, target) = temp_dir("removable_lines");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/runtime-adapters.md", source_content);
        write_file(&target, "protocol/runtime-adapters.md", target_content);

        let result = check_target(
            &source,
            &target,
            "public",
            &ProjectKind::PublicCoreOnly,
            &allowlist::default_public_allowlist(),
        );

        // Should have legal redaction (info) for this section, not content drift
        let fails: Vec<_> = result
            .drifts
            .iter()
            .filter(|d| d.severity == Severity::Fail && d.file == "protocol/runtime-adapters.md")
            .collect();
        assert!(
            fails.is_empty(),
            "expected no fails for removable-only diff, got: {fails:?}"
        );

        let infos: Vec<_> = result
            .drifts
            .iter()
            .filter(|d| d.severity == Severity::Info && d.file == "protocol/runtime-adapters.md")
            .collect();
        assert!(!infos.is_empty(), "expected info-level legal redaction");
        cleanup(&source, &target);
    }

    #[test]
    fn extra_section_in_target_is_warn() {
        let source_content = "# Top\n\n## Keep\nkeep\n";
        let target_content = "# Top\n\n## Keep\nkeep\n\n## Extra\nextra\n";
        let (source, target) = temp_dir("extra_section");
        write_manifest(&source);
        write_manifest(&target);
        write_file(&source, "protocol/runtime-adapters.md", source_content);
        write_file(&target, "protocol/runtime-adapters.md", target_content);

        let result = check_target(
            &source,
            &target,
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        let drift = result.drifts.iter().find(|d| {
            d.kind == DriftKind::ExtraSection && d.file == "protocol/runtime-adapters.md"
        });
        assert!(
            drift.is_some(),
            "expected extra section warning, got: {result:?}"
        );
        assert_eq!(drift.unwrap().severity, Severity::Warn);
        cleanup(&source, &target);
    }

    #[test]
    fn source_root_missing_produces_fail() {
        let result = check_target(
            Path::new("/nonexistent/source/path"),
            Path::new("/nonexistent/target/path"),
            "stable",
            &ProjectKind::Stable,
            &allowlist::empty_allowlist(),
        );

        // All manifest files will be "missing in source" since source root doesn't exist
        assert!(!result.drifts.is_empty());
    }

    #[test]
    fn passed_returns_false_when_fails_present() {
        let pd = ProjectDrift {
            project_name: "test".to_string(),
            project_kind: ProjectKind::Stable,
            project_root: Path::new("/tmp").to_path_buf(),
            drifts: vec![Drift::new(
                error_code::CONTENT_DRIFT,
                DriftKind::ContentDrift,
                Severity::Fail,
                "test.md",
                vec!["Test".to_string()],
                "drift",
                "fix it",
            )],
        };
        assert!(!pd.passed());
        assert_eq!(pd.failures(), 1);
    }

    #[test]
    fn passed_returns_true_when_only_warnings_and_infos() {
        let pd = ProjectDrift {
            project_name: "test".to_string(),
            project_kind: ProjectKind::PublicCoreOnly,
            project_root: Path::new("/tmp").to_path_buf(),
            drifts: vec![
                Drift::new(
                    error_code::LEGAL_REDACTION,
                    DriftKind::LegalRedaction,
                    Severity::Info,
                    "CLAUDE.md",
                    vec![],
                    "legal",
                    "none",
                ),
                Drift::new(
                    error_code::EXTRA_PROTOCOL_FILE,
                    DriftKind::ExtraProtocolFile,
                    Severity::Warn,
                    "protocol/extra.md",
                    vec![],
                    "extra",
                    "review",
                ),
            ],
        };
        assert!(pd.passed());
        assert_eq!(pd.failures(), 0);
        assert_eq!(pd.warnings(), 1);
        assert_eq!(pd.infos(), 1);
    }
}
