use crate::agents::host_specs::{agents_governance_chain, ags_mcp_tool_surface};
use crate::context::home_dir;
use crate::host_platforms::{cross_platform_init_plan, AgentPlatformStatus};

/// `ags agents govern` — advise-only AGS MCP onboarding plan. AGS never writes
/// host config, never runs external registrars, and never writes a receipt:
/// advised-only choices must stay visible in stdout / the host conversation so
/// the operator can choose which hosts to register.
pub(in crate::agents) fn cmd_agents_govern(agent: Option<&str>, apply: bool, format: &str) {
    let home = home_dir();
    let plan = cross_platform_init_plan(&home, &|c| ags_platform::is_on_path(c));
    let targets: Vec<&AgentPlatformStatus> = plan
        .platforms
        .iter()
        .filter(|p| match agent {
            Some(a) => p.id == a,
            None => p.detected,
        })
        .collect();
    let chain = agents_governance_chain();
    let tool_surface = ags_mcp_tool_surface();

    if format == "json" {
        let host_plans: Vec<_> = targets
            .iter()
            .map(|p| {
                serde_json::json!({
                    "host": p.id,
                    "display": p.display,
                    "detected": p.detected,
                    "advised_mcp_registration": p.mcp_host_command,
                    "registers_server": "ags",
                    "mandatory_first_tool": "ags_preflight",
                    "mcp_tools": tool_surface,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "agents govern",
                "mode": if apply { "confirm-view" } else { "dry-run" },
                "apply_requested": apply,
                "apply_status": if apply { "advice-only-no-write" } else { "advised-only" },
                "applied": false,
                "selection_required": true,
                "governance_chain": chain,
                "registration_granularity": "mcp-server",
                "registers_server": "ags",
                "mandatory_first_tool": "ags_preflight",
                "mcp_tools": tool_surface,
                "hosts": host_plans,
                "note": "advise-only — AGS never runs claude mcp add / codex mcp / lark-cli, never writes host config, and never writes a receipt for advice. Choose hosts/tools in the conversation, then run the selected host registrar explicitly.",
            }))
            .unwrap()
        );
    } else {
        println!("Agent Governance (advise-only)");
        if targets.is_empty() {
            println!("  No target hosts (none detected; pass --agent <id> to target one).");
        }
        for p in &targets {
            println!("  → {} ({})", p.id, p.display);
            println!("      advise: {}", p.mcp_host_command);
            println!("      server: ags");
            println!("      mandatory first tool: ags_preflight");
            println!("      tools: {}", tool_surface.join(", "));
        }
        println!("\nGovernance chain (success = host can call ags_preflight, then flow through):");
        for step in &chain {
            println!("  - {step}");
        }
        if apply {
            println!(
                "\nSelection required: no files were written. Choose which host registrations to run."
            );
        }
        println!(
            "\nNOTE: advise-only. AGS never runs external registrars, writes host config, or writes receipts for advice."
        );
    }
}
