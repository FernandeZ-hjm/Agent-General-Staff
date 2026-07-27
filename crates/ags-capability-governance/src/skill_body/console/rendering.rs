use super::*;
#[allow(unused_imports)]
use super::{inventory::*, model::*};
// ── Rendering ────────────────────────────────────────────────────────────────

pub fn render_inventory_json(result: &ManagedInventoryResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

pub fn render_inventory_text(result: &ManagedInventoryResult) -> String {
    let mut lines = Vec::new();
    lines.push("Skill & MCP Management Console — Inventory".to_string());
    lines.push("==========================================".to_string());
    lines.push(format!("Schema: {}", result.schema_version));
    lines.push(format!("Hosts:  {}", result.hosts.join(", ")));
    lines.push(String::new());
    let s = &result.summary;
    lines.push(format!(
        "Summary: total {} (skills {}, mcps {}, suite-interfaces {}, cli-backed {}); canonical {}, claude-visible {}, risk-flagged {}",
        s.total, s.skills, s.mcps, s.suite_interfaces, s.cli_backed, s.canonical_present, s.claude_visible, s.risk_flagged
    ));
    lines
        .push("(canonical = AGS holds the one body; per-host = thin-index visibility)".to_string());
    lines.push(String::new());
    for c in &result.capabilities {
        // Per-host thin-index visibility, e.g. "claude-code:Visible codex:NotVisible".
        let hosts: String = c
            .host_visibility
            .iter()
            .map(|v| format!("{}:{:?}", v.host, v.status))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!(
            "  [{}] {} — managed:{} canonical:{} health:{:?} | {}",
            kind_str(&c.kind),
            c.name,
            managed_status_str(&c.managed_status),
            if c.canonical_present {
                "present"
            } else {
                "absent"
            },
            c.health_status,
            hosts,
        ));
        for r in &c.risk_notes {
            lines.push(format!("      ⚠ {r}"));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

pub fn render_verify_json(result: &HostVerifyResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

pub fn render_verify_text(result: &HostVerifyResult) -> String {
    let mut lines = Vec::new();
    lines.push("Host Visibility Verify".to_string());
    lines.push("======================".to_string());
    lines.push(format!("Host:      {}", result.host));
    lines.push(format!("Supported: {}", result.supported));
    lines.push(format!("Status:    {}", result.status));
    if result.supported {
        let s = &result.summary;
        lines.push(format!(
            "Summary:   total {} (visible {}, not-visible {}, degraded {}); expected {}, failed {}, all_visible {}",
            s.total, s.visible, s.not_visible, s.degraded, s.expected, s.failed, s.all_visible
        ));
        lines.push(String::new());
        for c in &result.checks {
            let exp = if c.expected { " (expected)" } else { "" };
            lines.push(format!(
                "  [{}] {} — {:?}{}",
                c.kind, c.name, c.visibility, exp
            ));
            for e in &c.evidence {
                lines.push(format!("      {e}"));
            }
        }
        for (label, drift) in [
            ("host", result.thin_index_drift.as_ref()),
            ("shared", result.shared_thin_index_drift.as_ref()),
        ] {
            if let Some(drift) = drift {
                lines.push(format!(
                    "  {label} thin-index drift: dir={}, broken={}, has_drift={}",
                    drift.skills_dir, drift.broken_symlinks, drift.has_drift
                ));
                for sample in &drift.drift_samples {
                    lines.push(format!("      {sample}"));
                }
            }
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}
