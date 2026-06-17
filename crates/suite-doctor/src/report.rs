//! Report rendering for suite diagnostic reports.
//!
//! Supports both human-readable text output and machine-readable JSON output,
//! following the same pattern as `workflow_sync_check::report`.

use crate::types::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> HealthReport {
        let mut report = HealthReport::new("Suite Diagnostics v2.6.0");
        report.add(Finding::pass("cargo-fmt", "cargo fmt --check passed"));
        report.add(Finding::fail(
            "cargo-test",
            "2 test(s) failed",
            "Run `cargo test` for details. Failing: test_a, test_b",
        ));
        report.add(Finding::warn(
            "git-status",
            "uncommitted changes in workspace",
            "3 files modified: Cargo.toml, src/lib.rs, README.md",
        ));
        report.add(Finding::info("suite-version", "suite-doctor v2.6.0"));
        report.add(Finding::skip(
            "network-check",
            "skipped: no network prerequisites configured",
        ));
        report
    }

    fn empty_report() -> HealthReport {
        HealthReport::new("Empty Report")
    }

    fn all_pass_report() -> HealthReport {
        let mut report = HealthReport::new("All Pass Report");
        report.add(Finding::pass("check-a", "ok"));
        report.add(Finding::pass("check-b", "ok"));
        report
    }

    // ── Text rendering ────────────────────────────────────────────────

    #[test]
    fn text_report_includes_title_and_status() {
        let report = sample_report();
        let text = render_text(&report);

        assert!(text.contains("Suite Diagnostic Report"));
        assert!(text.contains("Suite Diagnostics v2.6.0"));
        assert!(text.contains("FAIL"));
        assert!(text.contains("1 fail"));
        assert!(text.contains("2 pass"));
    }

    #[test]
    fn text_report_includes_all_findings() {
        let report = sample_report();
        let text = render_text(&report);

        assert!(text.contains("cargo-fmt"));
        assert!(text.contains("cargo-test"));
        assert!(text.contains("git-status"));
        assert!(text.contains("suite-version"));
        assert!(text.contains("network-check"));
        assert!(text.contains("PASS"));
        assert!(text.contains("SKIP"));
    }

    #[test]
    fn text_report_includes_detail_for_fail_and_warn() {
        let report = sample_report();
        let text = render_text(&report);

        assert!(text.contains("Run `cargo test` for details"));
        assert!(text.contains("3 files modified"));
    }

    #[test]
    fn text_passed_report_shows_pass() {
        let report = all_pass_report();
        let text = render_text(&report);

        assert!(text.contains("PASS"));
        assert!(text.contains("All checks passed"));
        assert!(!text.contains("FAIL"));
    }

    #[test]
    fn text_empty_report_shows_no_checks() {
        let report = empty_report();
        let text = render_text(&report);

        assert!(text.contains("(no checks run)"));
    }

    #[test]
    fn text_report_summary_lists_failed_checks() {
        let report = sample_report();
        let text = render_text(&report);

        assert!(text.contains("check(s) failed: cargo-test"));
    }

    // ── JSON rendering ────────────────────────────────────────────────

    #[test]
    fn json_report_is_valid_json() {
        let report = sample_report();
        let json = render_json(&report);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["title"], "Suite Diagnostics v2.6.0");
        assert_eq!(parsed["findings"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn json_uses_canonical_status_values() {
        let report = sample_report();
        let json = render_json(&report);

        assert!(json.contains("\"pass\""));
        assert!(json.contains("\"fail\""));
        assert!(json.contains("\"warn\""));
        assert!(json.contains("\"skip\""));
        // Must NOT contain Rust variant names
        assert!(!json.contains("\"Pass\""));
        assert!(!json.contains("\"Fail\""));
        assert!(!json.contains("\"Warn\""));
        assert!(!json.contains("\"Skip\""));
    }

    #[test]
    fn json_uses_canonical_severity_values() {
        let report = sample_report();
        let json = render_json(&report);

        assert!(json.contains("\"info\""));
        // Must NOT contain Rust variant names
        assert!(!json.contains("\"Info\""));
    }

    #[test]
    fn json_detail_is_null_when_absent() {
        let report = all_pass_report();
        let json = render_json(&report);

        // Pass findings have no detail → should serialize as null
        assert!(json.contains("\"detail\": null"));
    }

    #[test]
    fn json_passed_report_has_correct_exit_code() {
        let report = all_pass_report();
        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    // ── Report aggregation ─────────────────────────────────────────────

    #[test]
    fn health_report_aggregates_counts_correctly() {
        let report = sample_report();

        assert_eq!(report.total(), 5);
        assert_eq!(report.total_failures(), 1); // cargo-test
        assert_eq!(report.total_warnings(), 1); // git-status
        assert_eq!(report.total_infos(), 3); // cargo-fmt pass, suite-version info, network-check skip
        assert_eq!(report.total_skipped(), 1); // network-check
        assert_eq!(report.total_passed_checks(), 2); // cargo-fmt + suite-version
        assert_eq!(report.total_failed_checks(), 1); // cargo-test
        assert_eq!(report.total_warned_checks(), 1); // git-status
    }

    #[test]
    fn health_report_passed_is_false_when_failures_present() {
        let report = sample_report();
        assert!(!report.passed());
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn health_report_passed_is_true_when_only_warnings_and_infos() {
        let mut report = HealthReport::new("warn-only");
        report.add(Finding::warn("w1", "warning", "detail"));
        report.add(Finding::info("i1", "info"));

        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn health_report_passed_is_true_when_empty() {
        let report = empty_report();
        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn health_report_passed_is_true_when_all_skip() {
        let mut report = HealthReport::new("skip-only");
        report.add(Finding::skip("s1", "not configured"));
        report.add(Finding::skip("s2", "not applicable"));

        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    // ── Severity ordering ──────────────────────────────────────────────

    #[test]
    fn severity_ordering_info_lowest_fail_highest() {
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Fail);
        assert!(Severity::Fail > Severity::Info);
    }

    #[test]
    fn check_status_is_ok() {
        assert!(CheckStatus::Pass.is_ok());
        assert!(CheckStatus::Skip.is_ok());
        assert!(!CheckStatus::Fail.is_ok());
        assert!(!CheckStatus::Warn.is_ok());
    }

    // ── Finding constructors ───────────────────────────────────────────

    #[test]
    fn finding_pass_has_correct_fields() {
        let f = Finding::pass("fmt", "all good");
        assert_eq!(f.check_name, "fmt");
        assert_eq!(f.status, CheckStatus::Pass);
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(f.message, "all good");
        assert!(f.detail.is_none());
    }

    #[test]
    fn finding_fail_has_correct_fields() {
        let f = Finding::fail("test", "failed", "details here");
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        assert_eq!(f.detail.as_deref(), Some("details here"));
    }
}
