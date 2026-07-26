// ── Renderers ───────────────────────────────────────────────────────────────

use super::*;

/// Render a verification report as human-readable text.
pub fn render_text(report: &VerificationReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("AGS Verification Report — scope: {}", report.scope));
    lines.push(format!("Repo: {}", report.repo_root));
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
    serde_json::to_string_pretty(report)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Tests ───────────────────────────────────────────────────────────────────
