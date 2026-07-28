//! Report rendering for suite diagnostic reports.
//!
//! Supports both human-readable text output and machine-readable JSON output,
//! shared by doctor and verification adapters.

use super::types::*;

/// Render a health report as human-readable text.
pub fn render_text(report: &HealthReport) -> String {
    let mut out = String::new();

    // Header
    let status = if report.passed() { "PASS" } else { "FAIL" };
    out.push_str(&format!(
        "═══ Suite Diagnostic Report ═══\n\
         Title:   {title}\n\
         Status:  {status}\n\
         Checks:  {total} total — {pass} pass, {fail} fail, {warn} warn, {skip} skip\n",
        title = report.title,
        total = report.total(),
        pass = report.total_passed_checks(),
        fail = report.total_failed_checks(),
        warn = report.total_warned_checks(),
        skip = report.total_skipped(),
    ));

    if report.findings.is_empty() {
        out.push_str("(no checks run)\n");
        return out;
    }

    // Per-check findings
    for (i, finding) in report.findings.iter().enumerate() {
        out.push('\n');
        out.push_str(&format!(
            "── Check {n}: {name} ──\n\
             Status:   {status}\n\
             Severity: {severity}\n\
             Message:  {message}\n",
            n = i + 1,
            name = finding.check_name,
            status = finding.status,
            severity = finding.severity,
            message = finding.message,
        ));
        if let Some(ref detail) = finding.detail {
            out.push_str(&format!("Detail:   {detail}\n"));
        }
    }

    // Summary footer
    out.push_str("\n───\n");
    if report.passed() {
        out.push_str("All checks passed.\n");
    } else {
        let fail_names: Vec<&str> = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Fail)
            .map(|f| f.check_name.as_str())
            .collect();
        out.push_str(&format!(
            "{} check(s) failed: {}\n",
            fail_names.len(),
            fail_names.join(", ")
        ));
    }

    out
}

/// Render a health report as JSON.
pub fn render_json(report: &HealthReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{ \"error\": \"{e}\" }}"))
}
