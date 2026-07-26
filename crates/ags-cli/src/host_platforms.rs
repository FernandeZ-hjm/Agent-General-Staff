//! Human rendering adapter for canonical host-integration facts.

use crate::output::yes_no;

pub(crate) use ags_host_integration::{
    cross_platform_init_plan, AgentPlatformStatus, CrossPlatformInitPlan, AGENT_PLATFORM_SPECS,
};

pub(crate) fn render_cross_platform_init_text(plan: &CrossPlatformInitPlan) -> String {
    let mut lines = vec![
        "Cross-Platform Initialization Wizard".to_string(),
        "====================================".to_string(),
        "Mode: plan-only by default. External MCP registrars are always advice-only; `ags agents govern --apply` may write only AGS-owned native memory adapters.".to_string(),
        match &plan.primary_agent {
            Some(primary) => format!("Primary agent: {primary}"),
            None => "Primary agent: none detected".to_string(),
        },
        String::new(),
        "Detected platforms:".to_string(),
    ];
    for platform in &plan.platforms {
        let detection = if platform.detected {
            "detected"
        } else {
            "not detected"
        };
        let primary = if platform.is_primary {
            " [primary]"
        } else {
            ""
        };
        lines.push(format!(
            "  - {:<14} cli: {:<3} config: {:<3} app: {:<3} ({}){}",
            platform.id,
            yes_no(platform.cli_present),
            yes_no(platform.config_present),
            yes_no(platform.app_present),
            detection,
            primary,
        ));
    }
    lines.push(String::new());

    let detected = plan
        .platforms
        .iter()
        .filter(|platform| platform.detected)
        .collect::<Vec<_>>();
    if detected.is_empty() {
        lines.push(
            "No Agent platforms detected — nothing to sync. Install a host CLI (claude/codex/omp) or rerun setup after onboarding."
                .to_string(),
        );
    } else {
        lines.push("Cross-platform sync plan (no external registrar is run):".to_string());
        for platform in detected {
            lines.push(format!("  → {} ({})", platform.id, platform.display));
            lines.push(format!(
                "      AGS-self MCP entry:      plan — advise host command, AGS never runs it: {}",
                platform.mcp_host_command
            ));
            lines.push(
                "      AGS skill lifecycle:     plan — `ags skill adopt <source>` audits catalog/local/GitHub sources; confirmed `--apply` writes only reviewed machine-private adoption state and planned-host thin indexes"
                    .to_string(),
            );
            lines.push(
                "      Adopted capability sync: plan — via `ags capability sync` (apply writes AGS-owned thin-index)"
                    .to_string(),
            );
            lines.push(format!(
                "      Drift check:             {}",
                platform.drift_check
            ));
        }
    }
    lines.push(String::new());
    lines.push(
        "NOTE: This wizard is plan-only. AGS advises host MCP commands but never executes external registrars/installers; AGS-owned skill thin-index writes go through the confirmation-protected guard. Cross-Agent capability sync/verify is available via the `ags capability` layer (`ags capability sync`, `ags capability verify`)."
            .to_string(),
    );
    lines.join("\n")
}

pub(crate) fn cross_platform_init_json(plan: &CrossPlatformInitPlan) -> serde_json::Value {
    let platforms = plan
        .platforms
        .iter()
        .map(|platform| {
            serde_json::json!({
                "id": platform.id,
                "display": platform.display,
                "cli_present": platform.cli_present,
                "config_present": platform.config_present,
                "app_present": platform.app_present,
                "detected": platform.detected,
                "is_primary": platform.is_primary,
            })
        })
        .collect::<Vec<_>>();
    let sync_plan = plan
        .platforms
        .iter()
        .filter(|platform| platform.detected)
        .map(|platform| {
            serde_json::json!({
                "host": platform.id,
                "ags_self_mcp": "plan",
                "mcp_host_command": platform.mcp_host_command,
                "ags_skill_thin_index": "plan-guarded-apply",
                "adopted_capability_sync": "plan-via-capability-layer",
                "drift_check": platform.drift_check,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "wizard_mode": "plan-only",
        "primary_agent": plan.primary_agent,
        "platforms": platforms,
        "sync_plan": sync_plan,
        "note": "AGS never runs external host registrars/installers; AGS-owned skill thin-index writes go through the confirmation guard. Cross-Agent capability sync/verify is available via the `ags capability` layer.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_host_integration::cross_platform_init_plan_with_detectors;

    #[test]
    fn rendering_preserves_plan_only_contract() {
        let home =
            std::env::temp_dir().join(format!("ags-host-render-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let plan = cross_platform_init_plan_with_detectors(&home, &|_| false, &|_| false);
        assert!(render_cross_platform_init_text(&plan).contains("No Agent platforms detected"));
        let json = cross_platform_init_json(&plan);
        assert_eq!(json["wizard_mode"], "plan-only");
        assert_eq!(json["sync_plan"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn canonical_specs_include_tencent_advise_only_hosts() {
        for host in AGENT_PLATFORM_SPECS
            .iter()
            .filter(|host| matches!(host.id, "workbuddy" | "codebuddy-code"))
        {
            assert!(!host.verify_supported);
            assert!(host.mcp_host_command.contains("ags_preflight"));
        }
    }
}
