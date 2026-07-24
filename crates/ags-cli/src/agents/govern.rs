use crate::agents::host_specs::{agents_governance_chain, ags_mcp_tool_surface};
use crate::context::home_dir;
use crate::host_platforms::{cross_platform_init_plan, AgentPlatformStatus};

/// `ags agents govern` — plan AGS MCP onboarding and, only with `--apply`,
/// install the selected host's AGS-owned native memory lifecycle adapter.
/// External MCP registrars remain advice-only.
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
    let mut apply_report = suite_doctor::HealthReport::new("agents-govern-apply");
    let supported_memory_hosts = ["claude-code", "codex", "omp"];
    if apply {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let stamp = crate::context::unix_timestamp();
        for target in &targets {
            if supported_memory_hosts.contains(&target.id.as_str()) {
                crate::setup::memory::apply_host_memory_adapter(
                    &mut apply_report,
                    &home,
                    &cwd,
                    &target.id,
                    stamp,
                );
            } else if agent.is_some() {
                apply_report.add(suite_doctor::Finding::fail(
                    "agents-memory-lifecycle-unsupported",
                    format!("no native memory lifecycle adapter for {}", target.id),
                    "Supported adapters: claude-code, codex, omp.",
                ));
            }
        }
    }
    let mut receipt_path = None;
    if apply {
        let passed = apply_report.passed();
        let target_ids = targets
            .iter()
            .map(|target| target.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let verification_command = match agent {
            Some(host) => format!("ags agents govern --agent {host} --apply"),
            None => "ags agents govern --apply".to_string(),
        };
        let advised = targets
            .iter()
            .map(|target| receipt::ReceiptAdvised {
                command: target.mcp_host_command.clone(),
                reason: "External MCP registration remains operator-controlled.".to_string(),
            })
            .collect();
        let action_receipt = receipt::build_action_receipt(
            "agents-govern-apply",
            Some(&target_ids),
            receipt::GateResult {
                decision: if passed { "allow" } else { "stop" }.to_string(),
                reason: if passed {
                    None
                } else {
                    Some("one or more host memory adapter writes failed".to_string())
                },
            },
            vec![],
            vec![],
            advised,
            vec![receipt::VerificationResult {
                command: verification_command,
                exit_code: apply_report.exit_code(),
                output_hash: receipt::sha256_hex(
                    suite_doctor::render_text(&apply_report).as_bytes(),
                ),
            }],
            receipt::RollbackPlan::manual_confirm(vec![]),
            if passed { "applied" } else { "failed" },
            passed,
        );
        match crate::receipt_bridge::emit_ags_action_receipt(&action_receipt) {
            Ok(path) => receipt_path = Some(path),
            Err(error) => apply_report.add(suite_doctor::Finding::fail(
                "agents-govern-action-receipt",
                "host memory adapter receipt could not be written",
                error,
            )),
        }
    }

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
                "mode": if apply { "apply" } else { "dry-run" },
                "apply_requested": apply,
                "apply_status": if apply { if apply_report.passed() { "memory-adapters-applied" } else { "memory-adapter-apply-failed" } } else { "advised-only" },
                "applied": apply,
                "selection_required": !apply,
                "governance_chain": chain,
                "registration_granularity": "mcp-server",
                "registers_server": "ags",
                "mandatory_first_tool": "ags_preflight",
                "mcp_tools": tool_surface,
                "hosts": host_plans,
                "memory_adapter_report": if apply { serde_json::to_value(&apply_report).unwrap_or_default() } else { serde_json::Value::Null },
                "receipt": receipt_path.as_ref().map(|path| path.display().to_string()),
                "note": "MCP registration remains advice-only. --apply writes only AGS-owned Claude/Codex hooks or the OMP extension for the selected supported host.",
            }))
            .unwrap()
        );
    } else {
        println!(
            "Agent Governance ({})",
            if apply {
                "memory-adapter apply"
            } else {
                "plan-only"
            }
        );
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
            println!("\n{}", suite_doctor::render_text(&apply_report));
            if let Some(path) = &receipt_path {
                println!("{}", receipt::render_action_receipt_summary_line(path));
            }
        }
        println!(
            "\nNOTE: MCP registration is advice-only. --apply changes only AGS-owned native memory lifecycle wiring."
        );
    }
    if apply && !apply_report.passed() {
        std::process::exit(apply_report.exit_code());
    }
}
