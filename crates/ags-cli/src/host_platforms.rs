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
            "No Agent platforms detected. Install a host CLI (claude/codex/omp) or rerun setup after onboarding."
                .to_string(),
        );
    } else {
        lines.push("Cross-platform setup plan (no external registrar is run):".to_string());
        for platform in detected {
            lines.push(format!("  → {} ({})", platform.id, platform.display));
            lines.push(format!(
                "      AGS-self MCP entry:      plan — advise host command, AGS never runs it: {}",
                platform.mcp_host_command
            ));
            lines.push(
                "      Skill catalog:           read-only inventory; source changes happen only in an explicit reviewed install/update"
                    .to_string(),
            );
            lines.push(
                "      Static capability state: refresh once with `ags capability snapshot --write --host <host>` after that install/update"
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
        "NOTE: This wizard is plan-only. AGS advises host MCP commands but never executes external registrars/installers. Capability inventory and verification are read-only; static snapshots refresh only after an explicit setup/update."
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
    let capability_plan = plan
        .platforms
        .iter()
        .filter(|platform| platform.detected)
        .map(|platform| {
            serde_json::json!({
                "host": platform.id,
                "ags_self_mcp": "plan",
                "mcp_host_command": platform.mcp_host_command,
                "skill_catalog": "read-only",
                "static_snapshot": "explicit-refresh-after-install-or-update",
                "drift_check": platform.drift_check,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "wizard_mode": "plan-only",
        "primary_agent": plan.primary_agent,
        "platforms": platforms,
        "capability_plan": capability_plan,
        "note": "AGS never runs external host registrars/installers. Capability inventory and verification are read-only; static snapshots refresh only after explicit setup/update.",
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
        assert_eq!(json["capability_plan"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(home);
    }
}
