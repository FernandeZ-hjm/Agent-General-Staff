//! Stable text and JSON rendering for project initialization.

use super::model::{InitCheckStatus, InitReport, InitSeverity, PROJECT_INIT_SCHEMA};
use super::plan::{project_file_status, ProjectInitPlan};

pub(crate) fn render_init_report_text(report: &InitReport) -> String {
    let status = if report.passed() { "PASS" } else { "FAIL" };
    let passed = report
        .findings
        .iter()
        .filter(|finding| finding.status == InitCheckStatus::Pass)
        .count();
    let failed = report
        .findings
        .iter()
        .filter(|finding| finding.status == InitCheckStatus::Fail)
        .count();
    let warned = report
        .findings
        .iter()
        .filter(|finding| finding.status == InitCheckStatus::Warn)
        .count();
    let mut output = format!(
        "═══ Suite Diagnostic Report ═══\nTitle:   {}\nStatus:  {status}\nChecks:  {} total — {passed} pass, {failed} fail, {warned} warn, 0 skip\n",
        report.title,
        report.findings.len()
    );
    if report.findings.is_empty() {
        output.push_str("(no checks run)\n");
        return output;
    }
    for (index, finding) in report.findings.iter().enumerate() {
        output.push_str(&format!(
            "\n── Check {}: {} ──\nStatus:   {}\nSeverity: {}\nMessage:  {}\n",
            index + 1,
            finding.check_name,
            finding.status,
            finding.severity,
            finding.message
        ));
        if let Some(detail) = &finding.detail {
            output.push_str(&format!("Detail:   {detail}\n"));
        }
    }
    output.push_str("\n───\n");
    if report.passed() {
        output.push_str("All checks passed.\n");
    } else {
        let failures: Vec<&str> = report
            .findings
            .iter()
            .filter(|finding| finding.severity == InitSeverity::Fail)
            .map(|finding| finding.check_name.as_str())
            .collect();
        output.push_str(&format!(
            "{} check(s) failed: {}\n",
            failures.len(),
            failures.join(", ")
        ));
    }
    output
}
pub(crate) fn render_project_init_text(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let mut lines = vec![
        format!("AGS Project Init Plan {}", PROJECT_INIT_SCHEMA),
        format!("Target: {}", plan.target.display()),
        format!("Slug:   {}", plan.slug),
        format!("Memory: {}", plan.memory_dir.display()),
        format!("Mode:   {}", if dry_run { "dry-run" } else { "apply" }),
        String::new(),
        "Directories:".to_string(),
    ];
    for dir in &plan.directories {
        let status = if dir.exists() {
            "exists"
        } else {
            "would-create"
        };
        lines.push(format!("  - [{status}] {}", dir.display()));
    }
    lines.push(String::new());
    lines.push("Files:".to_string());
    for file in &plan.files {
        lines.push(format!(
            "  - [{}] {} — {}",
            project_file_status(file, &plan.append_files),
            file.path.display(),
            file.description
        ));
    }
    if !plan.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &plan.warnings {
            lines.push(format!("  ! {warning}"));
        }
    }
    lines.join("\n")
}
pub(crate) fn render_project_init_json(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let directories: Vec<_> = plan
        .directories
        .iter()
        .map(|dir| {
            serde_json::json!({
                "path": dir.to_string_lossy(),
                "status": if dir.exists() { "exists" } else { "would-create" },
            })
        })
        .collect();
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path.to_string_lossy(),
                "description": file.description,
                "status": project_file_status(file, &plan.append_files),
                "mode": file.mode.map(|m| format!("{m:o}")),
            })
        })
        .collect();
    let output = serde_json::json!({
        "schema_version": PROJECT_INIT_SCHEMA,
        "target": plan.target.to_string_lossy(),
        "slug": plan.slug,
        "memory_dir": plan.memory_dir.to_string_lossy(),
        "dry_run": dry_run,
        "directories": directories,
        "files": files,
        "warnings": plan.warnings,
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}
