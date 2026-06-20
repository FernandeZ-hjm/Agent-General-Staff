use crate::agents::host_specs::{agents_scan_rows, default_agents_probe};
use crate::context::home_dir;
use crate::host_platforms::cross_platform_init_plan;

/// `ags agents scan` — read-only inventory of local Agent hosts + AGS MCP.
pub(in crate::agents) fn cmd_agents_scan(format: &str) {
    let home = home_dir();
    let plan = cross_platform_init_plan(&home, &|c| ags_platform::is_on_path(c));
    let rows = agents_scan_rows(&plan, &default_agents_probe);
    if format == "json" {
        let hosts: Vec<_> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "display": r.display,
                    "cli_present": r.cli_present,
                    "config_present": r.config_present,
                    "app_present": r.app_present,
                    "detected": r.detected,
                    "is_primary": r.is_primary,
                    "ags_mcp_registered": r.ags_mcp_registered,
                    "ags_mcp_evidence": r.ags_mcp_evidence,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "agents scan",
                "primary_agent": plan.primary_agent,
                "hosts": hosts,
            }))
            .unwrap()
        );
    } else {
        println!("Agent Hosts (read-only scan)");
        match &plan.primary_agent {
            Some(p) => println!("Primary agent: {p}"),
            None => println!("Primary agent: none detected"),
        }
        for r in &rows {
            let det = if r.detected {
                "detected"
            } else {
                "not detected"
            };
            let primary = if r.is_primary { " [primary]" } else { "" };
            let mcp = match r.ags_mcp_registered {
                Some(true) => "AGS MCP: registered",
                Some(false) => "AGS MCP: not registered",
                None => "AGS MCP: advise (not probeable)",
            };
            println!(
                "  - {:<14} {:<12} cli:{:<3} config:{:<3} app:{:<3} {}{}",
                r.id,
                det,
                if r.cli_present { "yes" } else { "no" },
                if r.config_present { "yes" } else { "no" },
                if r.app_present { "yes" } else { "no" },
                mcp,
                primary
            );
        }
        println!("\nNext: `ags agents govern` to plan AGS MCP onboarding (advise-only).");
    }
}
