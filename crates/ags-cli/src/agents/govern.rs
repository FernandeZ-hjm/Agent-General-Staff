use crate::context::home_dir;
use crate::host_platforms::{cross_platform_init_plan, AgentPlatformStatus};
use ags_host_integration::{agents_governance_chain, ags_mcp_tool_surface};

/// `ags agents govern` — plan AGS MCP onboarding and, only with `--apply`,
/// install the selected host's AGS-owned native memory lifecycle adapter.
/// External MCP registrars remain advice-only.
pub(in crate::agents) fn cmd_agents_govern(
    agent: Option<&str>,
    workspace: &std::path::Path,
    apply: bool,
    format: &str,
) {
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
    let workspace = ags_platform::canonical_workspace_root(workspace).unwrap_or_else(|error| {
        eprintln!("ags agents govern: {error}");
        std::process::exit(1);
    });
    let mut apply_report = ags_verification::doctor::HealthReport::new("agents-govern-apply");
    if apply {
        for target in &targets {
            if ags_host_integration::platform_spec(&target.id)
                .and_then(|spec| spec.lifecycle)
                .is_some()
            {
                ags_lifecycle::setup::apply_host_memory_adapter(
                    &mut apply_report,
                    &home,
                    &workspace,
                    &target.id,
                );
            } else if agent.is_some() {
                let supported = ags_host_integration::lifecycle_specs()
                    .map(|spec| spec.host_id)
                    .collect::<Vec<_>>()
                    .join(", ");
                apply_report.add(ags_verification::doctor::Finding::fail(
                    "agents-memory-lifecycle-unsupported",
                    format!("no native memory lifecycle adapter for {}", target.id),
                    format!("Supported adapters: {supported}."),
                ));
            }
        }
        if apply_report.passed() {
            let runtime_home = ags_capability_governance::locate_runtime_home();
            let approved = targets
                .iter()
                .filter(|target| {
                    ags_host_integration::platform_spec(&target.id)
                        .and_then(|spec| spec.lifecycle)
                        .is_some()
                })
                .map(|target| target.id.clone())
                .collect::<Vec<_>>();
            match ags_lifecycle::setup::add_approved_lifecycle_hosts(&runtime_home, &approved) {
                Ok(hosts) => apply_report.add(ags_verification::doctor::Finding::pass(
                    "lifecycle-host-approval-current",
                    format!("approved lifecycle hosts: {}", hosts.join(", ")),
                )),
                Err(error) => apply_report.add(ags_verification::doctor::Finding::fail(
                    "lifecycle-host-approval-current",
                    "could not persist lifecycle host approval",
                    error,
                )),
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
            Some(host) => format!(
                "ags agents govern --agent {host} --target '{}' --apply",
                workspace.display()
            ),
            None => format!(
                "ags agents govern --target '{}' --apply",
                workspace.display()
            ),
        };
        let advised = targets
            .iter()
            .map(|target| ags_evidence::ReceiptAdvised {
                command: target.mcp_host_command.clone(),
                reason: "External MCP registration remains operator-controlled.".to_string(),
            })
            .collect();
        let action_receipt = ags_evidence::build_action_receipt(
            "agents-govern-apply",
            Some(&target_ids),
            ags_evidence::GateResult {
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
            vec![ags_evidence::VerificationResult {
                command: verification_command,
                exit_code: apply_report.exit_code(),
                output_hash: ags_evidence::sha256_hex(
                    ags_verification::doctor::render_text(&apply_report).as_bytes(),
                ),
            }],
            if passed { "applied" } else { "failed" },
            passed,
        );
        match crate::receipt_bridge::emit_ags_action_receipt(&action_receipt) {
            Ok(path) => receipt_path = Some(path),
            Err(error) => apply_report.add(ags_verification::doctor::Finding::fail(
                "agents-govern-action-receipt",
                "host memory adapter receipt could not be written",
                error,
            )),
        }
    }

    let host_plans: Vec<_> = targets
        .iter()
        .map(|p| {
            let migration_preview =
                ags_lifecycle::setup::lifecycle_migration_preview(&home, &workspace, &p.id).ok();
            serde_json::json!({
                "host": p.id,
                "display": p.display,
                "detected": p.detected,
                "advised_mcp_registration": p.mcp_host_command,
                "registers_server": "ags",
                "mandatory_first_tool": "ags_preflight",
                "mcp_tools": tool_surface,
                "lifecycle_migration_preview": migration_preview,
            })
        })
        .collect();
    let output = serde_json::json!({
        "command": "agents govern",
        "target": workspace,
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
        "memory_adapter_report": if apply { serde_json::to_value(&apply_report).expect("serializable doctor report") } else { serde_json::Value::Null },
        "receipt": receipt_path.as_ref().map(|path| path.display().to_string()),
        "note": "MCP registration remains advice-only. --apply writes only AGS-owned workspace lifecycle adapters for Claude Code, Codex, Cursor, CodeBuddy, or OMP.",
    });
    crate::output::emit(format, &output, || {
        let mut lines = vec![format!(
            "Agent Governance ({})",
            if apply {
                "memory-adapter apply"
            } else {
                "plan-only"
            }
        )];
        if targets.is_empty() {
            lines
                .push("  No target hosts (none detected; pass --agent <id> to target one).".into());
        }
        for p in &targets {
            lines.push(format!("  → {} ({})", p.id, p.display));
            lines.push(format!("      advise: {}", p.mcp_host_command));
            lines.push("      server: ags".into());
            lines.push("      mandatory first tool: ags_preflight".into());
            lines.push(format!("      tools: {}", tool_surface.join(", ")));
            if let Ok(preview) =
                ags_lifecycle::setup::lifecycle_migration_preview(&home, &workspace, &p.id)
            {
                lines.push(format!(
                    "      lifecycle: {} managed workspace(s), global AGS entry={}, removal ready after apply={}",
                    preview.managed_workspaces.len(),
                    preview.global_ags_owned_entry_present,
                    preview.removal_ready_after_apply
                ));
                lines.push(format!("      backup if removed: {}", preview.backup_path));
            }
        }
        lines.push(
            "\nGovernance chain (success = host can call ags_preflight, then flow through):".into(),
        );
        for step in &chain {
            lines.push(format!("  - {step}"));
        }
        if apply {
            lines.push(format!(
                "\n{}",
                ags_verification::doctor::render_text(&apply_report)
            ));
            if let Some(path) = &receipt_path {
                lines.push(ags_evidence::render_action_receipt_summary_line(path));
            }
        }
        lines.push("\nNOTE: MCP registration is advice-only. The dry-run includes managed-workspace migration readiness; --apply changes only AGS-owned native memory lifecycle wiring.".into());
        lines.join("\n")
    });
    if apply && !apply_report.passed() {
        std::process::exit(apply_report.exit_code());
    }
}
