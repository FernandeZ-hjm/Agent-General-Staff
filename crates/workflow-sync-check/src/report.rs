//! Report rendering for structured drift reports.
//!
//! Supports both human-readable text output and machine-readable JSON output.

use crate::types::*;

/// Render a drift report as human-readable text.
pub fn render_text(report: &DriftReport) -> String {
    let mut out = String::new();

    // Header
    let status = if report.passed() { "PASS" } else { "FAIL" };
    out.push_str(&format!(
        "═══ Protocol Sync Drift Report ═══\n\
         Status:  {status}\n\
         Source:  {source} ({source_name})\n\
         Targets: {target_count}\n\
         Drifts:  {total} total — {fails} fail, {warns} warn, {infos} info\n",
        source = report.source_root.display(),
        source_name = report.source_name,
        target_count = report.projects.len(),
        total = report.total_drifts(),
        fails = report.total_failures(),
        warns = report.total_warnings(),
        infos = report.total_infos(),
    ));

    // Per-project sections
    for project in &report.projects {
        out.push('\n');
        out.push_str(&format!(
            "── {name} ({kind}) ──\n\
             Path:   {root}\n\
             Result: {pstatus}\n\
             Drifts: {fail} fail, {warn} warn, {info} info\n",
            name = project.project_name,
            kind = project.project_kind,
            root = project.project_root.display(),
            pstatus = if project.passed() { "PASS" } else { "FAIL" },
            fail = project.failures(),
            warn = project.warnings(),
            info = project.infos(),
        ));

        if project.drifts.is_empty() {
            out.push_str("  (no differences)\n");
            continue;
        }

        // Group drifts by file
        let mut files: Vec<&str> = project.drifts.iter().map(|d| d.file.as_str()).collect();
        files.sort();
        files.dedup();

        for file in &files {
            let file_drifts: Vec<&Drift> =
                project.drifts.iter().filter(|d| d.file == *file).collect();

            out.push_str(&format!(
                "\n  File: {file} ({count} finding(s))\n",
                count = file_drifts.len()
            ));

            for drift in &file_drifts {
                let section = if drift.section_path.is_empty() {
                    "(file-level)".to_string()
                } else {
                    format_section_path(&drift.section_path)
                };

                out.push_str(&format!(
                    "    [{severity}] [{code}] {kind}\n\
                     Section: {section}\n\
                     Message: {message}\n\
                     Action:  {action}\n\n",
                    severity = drift.severity,
                    code = drift.code,
                    kind = drift.kind,
                    section = section,
                    message = drift.message,
                    action = drift.suggested_action,
                ));
            }
        }
    }

    // Summary footer
    out.push_str("───\n");
    if report.passed() {
        out.push_str("No dangerous drift detected.\n");
    } else {
        out.push_str(&format!(
            "{} target(s) have dangerous drift. Review findings above.\n",
            report.projects.iter().filter(|p| !p.passed()).count()
        ));
    }

    out
}

/// Render a drift report as JSON.
pub fn render_json(report: &DriftReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{ \"error\": \"{e}\" }}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_report() -> DriftReport {
        DriftReport {
            source_root: PathBuf::from("/tmp/source"),
            source_name: "private".to_string(),
            projects: vec![
                ProjectDrift {
                    project_name: "stable".to_string(),
                    project_kind: ProjectKind::Stable,
                    project_root: PathBuf::from("/tmp/stable"),
                    drifts: vec![
                        Drift::new(
                            error_code::CONTENT_DRIFT,
                            DriftKind::ContentDrift,
                            Severity::Fail,
                            "protocol/runtime-adapters.md",
                            vec!["Generic Fields".into(), "Executor".into()],
                            "content drift: source 120 bytes, target 115 bytes",
                            "review and reconcile the content difference",
                        ),
                        Drift::new(
                            error_code::SECTION_MISSING,
                            DriftKind::SectionMissing,
                            Severity::Fail,
                            "protocol/task-routing.md",
                            vec!["Heavy Tasks".into()],
                            "section missing in target: Heavy Tasks",
                            "restore the section in target",
                        ),
                    ],
                },
                ProjectDrift {
                    project_name: "public".to_string(),
                    project_kind: ProjectKind::PublicCoreOnly,
                    project_root: PathBuf::from("/tmp/public"),
                    drifts: vec![Drift::new(
                        error_code::LEGAL_REDACTION,
                        DriftKind::LegalRedaction,
                        Severity::Info,
                        "CLAUDE.md",
                        vec![],
                        "file absent from target (allowlisted)",
                        "no action needed",
                    )],
                },
            ],
        }
    }

    #[test]
    fn text_report_includes_status_and_files() {
        let report = sample_report();
        let text = render_text(&report);

        assert!(text.contains("FAIL"));
        assert!(text.contains("private"));
        assert!(text.contains("stable"));
        assert!(text.contains("public"));
        assert!(text.contains("DRIFT_CONTENT_DRIFT"));
        assert!(text.contains("DRIFT_LEGAL_REDACTION"));
        assert!(text.contains("Executor"));
        assert!(text.contains("Heavy Tasks"));
    }

    #[test]
    fn text_passed_report_shows_pass() {
        let report = DriftReport {
            source_root: PathBuf::from("/tmp/src"),
            source_name: "private".into(),
            projects: vec![ProjectDrift {
                project_name: "stable".into(),
                project_kind: ProjectKind::Stable,
                project_root: PathBuf::from("/tmp/stable"),
                drifts: vec![],
            }],
        };
        let text = render_text(&report);
        assert!(text.contains("PASS"));
    }

    #[test]
    fn json_report_is_valid_json() {
        let report = sample_report();
        let json = render_json(&report);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["source_name"], "private");
        assert_eq!(parsed["projects"].as_array().unwrap().len(), 2);
    }
}
