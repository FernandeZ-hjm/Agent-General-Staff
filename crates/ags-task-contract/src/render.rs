//! Stable text and JSON presentation for compile reports.

use crate::compile::CompileReport;

/// Render only the compiled task card as plain text (no report wrapper).
/// Returns an empty string if the card is empty.
pub fn render_card_text(report: &CompileReport) -> String {
    report.compiled_task_card.clone()
}

/// Render a CompileReport as human-readable text.
pub fn render_report_text(report: &CompileReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("M4 Task Compiler Report".to_string());
    lines.push("========================".to_string());
    lines.push(format!("Schema version: {}", report.schema_version));
    lines.push(format!("Contract format: {}", report.contract_format));
    lines.push(format!("Check only:     {}", report.check_only));
    lines.push(format!(
        "Task card requested: {}",
        if report.task_card_requested {
            "YES"
        } else {
            "NO"
        }
    ));
    lines.push(format!(
        "Host Plan-mode final: {}",
        if report.host_plan_mode_final {
            "YES"
        } else {
            "NO"
        }
    ));
    lines.push(format!("Handoff source: {}", report.handoff_source));
    lines.push(format!(
        "Handoff contract confirmed: {}",
        if report.confirmed_handoff_contract {
            "YES"
        } else {
            "NO"
        }
    ));
    lines.push(format!(
        "Executable allowed:  {}",
        if report.executable_allowed {
            "YES"
        } else {
            "NO"
        }
    ));
    if let Some(ref reason) = report.block_reason {
        lines.push(format!("Block reason:   {}", reason));
    }
    lines.push(format!(
        "Validation:     {}",
        if report.validation_passed {
            "PASS"
        } else {
            "FAIL"
        }
    ));
    lines.push(String::new());

    if !report.missing_slots.is_empty() {
        lines.push("MISSING SLOTS:".to_string());
        for slot in &report.missing_slots {
            lines.push(format!("  - {}", slot));
        }
        lines.push(String::new());
    }

    if !report.assumptions.is_empty() {
        lines.push("Assumptions:".to_string());
        for a in &report.assumptions {
            lines.push(format!("  - {}", a));
        }
        lines.push(String::new());
    }

    if !report.deprecations.is_empty() {
        lines.push("Deprecations:".to_string());
        for notice in &report.deprecations {
            lines.push(format!("  - {notice}"));
        }
        lines.push(String::new());
    }

    if !report.validation_errors.is_empty() {
        lines.push("Validation Errors:".to_string());
        for e in &report.validation_errors {
            lines.push(format!("  - {}", e));
        }
        lines.push(String::new());
    }

    lines.push("Slot Sources:".to_string());
    for slot in &report.slot_sources {
        lines.push(format!(
            "  {:25} ← {:20} ({})",
            slot.field,
            slot.source.as_str(),
            if slot.value.chars().count() > 60 {
                let truncated: String = slot.value.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                slot.value.clone()
            }
        ));
    }
    lines.push(String::new());

    if report.executable_allowed && !report.compiled_task_card.is_empty() {
        lines.push("Compiled Task Card:".to_string());
        lines.push("-------------------".to_string());
        lines.push(report.compiled_task_card.clone());
    }

    lines.join("\n")
}

/// Render a CompileReport as JSON.
pub fn render_report_json(report: &CompileReport) -> String {
    serde_json::to_string_pretty(report)
        .unwrap_or_else(|e| format!("{{\"error\": \"JSON serialization failed: {}\"}}", e))
}
