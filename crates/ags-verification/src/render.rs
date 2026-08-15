// ── Renderers ───────────────────────────────────────────────────────────────

use super::*;

/// Render a verification report as human-readable text.
pub fn render_text(report: &VerificationReport) -> String {
    if report.passed() {
        let mut lines = vec![
            format!("AGS check: PASS ({})", report.scope),
            format!("Workspace: {}", report.repo_root),
            format!("Project tests run: {}", report.project_tests_run),
        ];
        let warnings = report
            .items
            .iter()
            .filter(|item| item.severity == Severity::Warn)
            .collect::<Vec<_>>();
        if warnings.is_empty() {
            lines.push(format!(
                "Checks: {} passed, {} skipped",
                report.summary.passed, report.summary.skipped
            ));
            lines.push("Receipt: governance check only".to_string());
        } else {
            lines.push(format!(
                "Warnings: {}",
                warnings
                    .iter()
                    .map(|item| format!("{} — {}", item.id, single_line(&item.evidence)))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
            lines.push(format!(
                "Remediation: {}",
                warnings
                    .iter()
                    .map(|item| item.remediation.as_deref().unwrap_or("none"))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        return lines.join("\n");
    }

    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("AGS Verification Report — scope: {}", report.scope));
    lines.push(format!("Repo: {}", report.repo_root));
    lines.push(format!("Project tests run: {}", report.project_tests_run));
    lines.push(String::new());

    // Sort items: failures first, then passes, then skips
    let mut sorted = report.items.clone();
    sorted.sort_by_key(|i| {
        (
            match i.status {
                CheckStatus::Fail => 0u8,
                CheckStatus::Pass => 1,
                CheckStatus::Skip => 2,
            },
            match i.severity {
                Severity::Error => 0u8,
                Severity::Warn => 1,
                Severity::Info => 2,
            },
        )
    });

    for item in &sorted {
        let status_icon = match item.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Fail => match item.severity {
                Severity::Error => "FAIL",
                Severity::Warn => "WARN",
                Severity::Info => "FAIL",
            },
            CheckStatus::Skip => "SKIP",
        };

        lines.push(format!(
            "[{}] {} — {}",
            status_icon,
            item.id,
            item.evidence.lines().next().unwrap_or("")
        ));

        if item.status == CheckStatus::Fail {
            if let Some(ref rem) = item.remediation {
                lines.push(format!("      remediation: {}", rem));
            }
            if let Some(ref cmd) = item.command {
                lines.push(format!("      command: {}", cmd));
            }
        }

        // For multi-line evidence, show remaining lines indented
        let evidence_lines: Vec<&str> = item.evidence.lines().collect();
        if evidence_lines.len() > 1 {
            for line in &evidence_lines[1..] {
                if !line.is_empty() {
                    lines.push(format!("      {}", line));
                }
            }
        }
    }

    lines.push(String::new());
    lines.push("─".repeat(50));
    lines.push(format!(
        "Summary: {} total, {} passed, {} failed ({} errors, {} warnings), {} skipped",
        report.summary.total,
        report.summary.passed,
        report.summary.failed,
        report.summary.errors,
        report.summary.warnings,
        report.summary.skipped,
    ));

    if report.passed() {
        lines.push("Verdict: PASS".to_string());
    } else {
        lines.push("Verdict: FAIL".to_string());
    }

    lines.join("\n")
}

/// Render a verification report as JSON.
pub fn render_json(report: &VerificationReport) -> String {
    let full = serde_json::to_string_pretty(report)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e));
    if crate::check_json_output_budget(full.as_bytes()).is_ok() {
        return full;
    }
    let blocked = serde_json::json!({
        "schema_version": "ags://schema/contract/v2/check-output-blocked",
        "status": "blocked",
        "error_code": "details_storage_required",
        "scope": report.scope,
        "workspace": report.repo_root,
        "project_tests_run": report.project_tests_run,
        "summary": report.summary,
        "full_output_sha256": ags_platform::sha256(full.as_bytes()),
        "remediation": "configure a content-addressed details store and rerun",
    });
    serde_json::to_string(&blocked)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Tests ───────────────────────────────────────────────────────────────────
