//! Shared cross-platform host-detection wizard (setup + agents).

use crate::output::yes_no;
use std::path::Path;

pub(crate) struct AgentPlatformSpec {
    pub(crate) id: &'static str,
    pub(crate) display: &'static str,
    /// Binaries probed on PATH to infer the host is installed.
    pub(crate) cli_names: &'static [&'static str],
    /// Home-relative config dirs whose presence also implies the host.
    pub(crate) config_subdirs: &'static [&'static str],
    /// macOS app bundles whose presence implies a GUI/IDE host is installed.
    pub(crate) app_bundles: &'static [&'static str],
    /// Host MCP-registration command AGS *advises* (and never runs).
    pub(crate) mcp_host_command: &'static str,
    /// Whether `ags skill verify --host <id>` currently supports this host.
    pub(crate) verify_supported: bool,
}
pub(crate) const AGENT_PLATFORM_SPECS: &[AgentPlatformSpec] = &[
    AgentPlatformSpec {
        id: "claude-code",
        display: "Claude Code",
        cli_names: &["claude"],
        config_subdirs: &[".claude"],
        app_bundles: &[],
        mcp_host_command: "claude mcp add ags -- ags mcp serve --transport stdio",
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "codex",
        display: "Codex",
        cli_names: &["codex"],
        config_subdirs: &[".codex"],
        app_bundles: &[],
        mcp_host_command: "codex mcp add ags -- ags mcp serve --transport stdio",
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "cursor",
        display: "Cursor",
        cli_names: &["cursor"],
        config_subdirs: &[".cursor"],
        app_bundles: &["Cursor.app"],
        mcp_host_command: "configure AGS MCP in Cursor settings (reserved)",
        verify_supported: false,
    },
    // Tencent Agent host clients. CLI / config names are best-effort inferences;
    // when neither is present detection degrades to "not detected" + advise.
    // AGS MCP registration uses underscore tool names (ags_preflight, etc.).
    AgentPlatformSpec {
        id: "workbuddy",
        display: "Tencent Agent (WorkBuddy)",
        cli_names: &["workbuddy", "workbuddy-ide"],
        config_subdirs: &[".workbuddy"],
        app_bundles: &["WorkBuddy.app", "WorkBuddy IDE.app"],
        mcp_host_command: "register AGS MCP in WorkBuddy host config (exposes ags_preflight / ags_agent_instructions / ags_task_validate / ags_policy_resolve); AGS never runs the registrar",
        verify_supported: false,
    },
    AgentPlatformSpec {
        id: "codebuddy-code",
        display: "Tencent Agent (CodeBuddy-Code)",
        cli_names: &["codebuddy", "codebuddy-code"],
        config_subdirs: &[".codebuddy"],
        app_bundles: &["CodeBuddy CN.app", "CodeBuddy Code.app", "CodeBuddy.app"],
        mcp_host_command: "register AGS MCP in CodeBuddy-Code host config (exposes ags_preflight / ags_agent_instructions / ags_task_validate / ags_policy_resolve); AGS never runs the registrar",
        verify_supported: false,
    },
];
#[derive(Debug, Clone)]
pub(crate) struct AgentPlatformStatus {
    pub(crate) id: String,
    pub(crate) display: String,
    pub(crate) cli_present: bool,
    pub(crate) config_present: bool,
    pub(crate) app_present: bool,
    pub(crate) detected: bool,
    pub(crate) is_primary: bool,
    pub(crate) mcp_host_command: String,
    pub(crate) drift_check: String,
}
pub(crate) struct CrossPlatformInitPlan {
    pub(crate) primary_agent: Option<String>,
    pub(crate) platforms: Vec<AgentPlatformStatus>,
}
/// Detect Agent platforms and build the cross-platform init plan.
///
/// Pure over its inputs: `home` supplies the config-dir root and `present`
/// reports whether a host CLI is on PATH. Production passes the real home and
/// `ags_platform::is_on_path`; tests inject a temp home and a mock predicate
/// so detection never touches the real host configuration.
pub(crate) fn cross_platform_init_plan(
    home: &Path,
    present: &dyn Fn(&str) -> bool,
) -> CrossPlatformInitPlan {
    cross_platform_init_plan_with_detectors(home, present, &|bundle| {
        application_bundle_present(home, bundle)
    })
}
pub(crate) fn cross_platform_init_plan_with_detectors(
    home: &Path,
    command_present: &dyn Fn(&str) -> bool,
    app_present: &dyn Fn(&str) -> bool,
) -> CrossPlatformInitPlan {
    let mut platforms: Vec<AgentPlatformStatus> = AGENT_PLATFORM_SPECS
        .iter()
        .map(|spec| {
            let cli_present = spec.cli_names.iter().any(|name| command_present(name));
            let config_present = spec
                .config_subdirs
                .iter()
                .any(|dir| home.join(dir).is_dir());
            let app_present = spec.app_bundles.iter().any(|bundle| app_present(bundle));
            let drift_check = if spec.verify_supported {
                format!("ags capability verify --host {}", spec.id)
            } else {
                format!("ags capability verify --host {} (reserved)", spec.id)
            };
            AgentPlatformStatus {
                id: spec.id.to_string(),
                display: spec.display.to_string(),
                cli_present,
                config_present,
                app_present,
                detected: cli_present || config_present || app_present,
                is_primary: false,
                mcp_host_command: spec.mcp_host_command.to_string(),
                drift_check,
            }
        })
        .collect();

    // Primary = strongest install signal first, then first detected.
    let primary_idx = platforms
        .iter()
        .position(|p| p.cli_present && p.config_present)
        .or_else(|| {
            platforms
                .iter()
                .position(|p| p.config_present && p.app_present)
        })
        .or_else(|| platforms.iter().position(|p| p.config_present))
        .or_else(|| platforms.iter().position(|p| p.detected));
    if let Some(idx) = primary_idx {
        platforms[idx].is_primary = true;
    }
    let primary_agent = primary_idx.map(|i| platforms[i].id.clone());

    CrossPlatformInitPlan {
        primary_agent,
        platforms,
    }
}
pub(crate) fn render_cross_platform_init_text(plan: &CrossPlatformInitPlan) -> String {
    let mut lines = vec![
        "Cross-Platform Initialization Wizard".to_string(),
        "====================================".to_string(),
        "Mode: plan-only (AGS never runs an external host registrar/installer here).".to_string(),
        match &plan.primary_agent {
            Some(p) => format!("Primary agent: {p}"),
            None => "Primary agent: none detected".to_string(),
        },
        String::new(),
        "Detected platforms:".to_string(),
    ];
    for p in &plan.platforms {
        let det = if p.detected {
            "detected"
        } else {
            "not detected"
        };
        let primary = if p.is_primary { " [primary]" } else { "" };
        lines.push(format!(
            "  - {:<14} cli: {:<3} config: {:<3} app: {:<3} ({}){}",
            p.id,
            yes_no(p.cli_present),
            yes_no(p.config_present),
            yes_no(p.app_present),
            det,
            primary,
        ));
    }
    lines.push(String::new());

    let detected: Vec<&AgentPlatformStatus> =
        plan.platforms.iter().filter(|p| p.detected).collect();
    if detected.is_empty() {
        lines.push(
            "No Agent platforms detected — nothing to sync. Install a host CLI (claude/codex) or rerun setup after onboarding."
                .to_string(),
        );
    } else {
        lines.push(
            "Cross-platform sync plan (plan-only; nothing is written or registered here):"
                .to_string(),
        );
        for p in detected {
            lines.push(format!("  → {} ({})", p.id, p.display));
            lines.push(format!(
                "      AGS-self MCP entry:      plan — advise host command, AGS never runs it: {}",
                p.mcp_host_command
            ));
            lines.push(
                "      AGS skill lifecycle:     plan — `ags skill adopt <skill-id>` writes only the machine-private overlay when confirmed with `--apply`"
                    .to_string(),
            );
            lines.push(
                "      Adopted capability sync: plan — via `ags capability sync` (apply writes AGS-owned thin-index)"
                    .to_string(),
            );
            lines.push(format!("      Drift check:             {}", p.drift_check));
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
    let platforms: Vec<_> = plan
        .platforms
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display": p.display,
                "cli_present": p.cli_present,
                "config_present": p.config_present,
                "app_present": p.app_present,
                "detected": p.detected,
                "is_primary": p.is_primary,
            })
        })
        .collect();
    let sync_plan: Vec<_> = plan
        .platforms
        .iter()
        .filter(|p| p.detected)
        .map(|p| {
            serde_json::json!({
                "host": p.id,
                "ags_self_mcp": "plan",
                "mcp_host_command": p.mcp_host_command,
                "ags_skill_thin_index": "plan-guarded-apply",
                "adopted_capability_sync": "plan-via-capability-layer",
                "drift_check": p.drift_check,
            })
        })
        .collect();
    serde_json::json!({
        "wizard_mode": "plan-only",
        "primary_agent": plan.primary_agent,
        "platforms": platforms,
        "sync_plan": sync_plan,
        "note": "AGS never runs external host registrars/installers; AGS-owned skill thin-index writes go through the confirmation guard. Cross-Agent capability sync/verify is available via the `ags capability` layer.",
    })
}
fn application_bundle_present(home: &Path, bundle_name: &str) -> bool {
    [
        Path::new("/Applications").join(bundle_name),
        Path::new("/System/Applications").join(bundle_name),
        home.join("Applications").join(bundle_name),
    ]
    .iter()
    .any(|p| p.is_dir())
}
#[cfg(test)]
mod cross_platform_init_tests {
    use super::{
        cross_platform_init_json, cross_platform_init_plan_with_detectors,
        render_cross_platform_init_text, AGENT_PLATFORM_SPECS,
    };
    use std::path::PathBuf;

    fn temp_home(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-xplat-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }
    fn test_plan(
        home: &std::path::Path,
        command_present: &dyn Fn(&str) -> bool,
    ) -> super::CrossPlatformInitPlan {
        cross_platform_init_plan_with_detectors(home, command_present, &|_| false)
    }

    #[test]
    fn claude_detected_via_cli_and_config_is_primary() {
        let home = temp_home("claude");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let plan = test_plan(&home, &|c| c == "claude");
        let claude = plan
            .platforms
            .iter()
            .find(|p| p.id == "claude-code")
            .unwrap();
        assert!(claude.cli_present && claude.config_present && claude.detected);
        assert!(claude.is_primary);
        assert_eq!(plan.primary_agent.as_deref(), Some("claude-code"));
        // codex/cursor absent here.
        assert!(
            !plan
                .platforms
                .iter()
                .find(|p| p.id == "codex")
                .unwrap()
                .detected
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_platforms_detected_yields_empty_sync_plan() {
        let home = temp_home("none");
        let plan = test_plan(&home, &|_| false);
        assert!(plan.primary_agent.is_none());
        assert!(plan.platforms.iter().all(|p| !p.detected));
        let text = render_cross_platform_init_text(&plan);
        assert!(text.contains("No Agent platforms detected"));
        let json = cross_platform_init_json(&plan);
        assert_eq!(json["sync_plan"].as_array().unwrap().len(), 0);
        assert_eq!(json["wizard_mode"], "plan-only");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn agent_specs_include_tencent_hosts_advise_only() {
        let ids: Vec<&str> = AGENT_PLATFORM_SPECS.iter().map(|s| s.id).collect();
        assert!(ids.contains(&"workbuddy"));
        assert!(ids.contains(&"codebuddy-code"));
        for s in AGENT_PLATFORM_SPECS {
            if s.id == "workbuddy" || s.id == "codebuddy-code" {
                assert!(!s.verify_supported, "tencent hosts are reserved for verify");
                assert!(
                    s.mcp_host_command.contains("ags_preflight"),
                    "tencent advise uses underscore tool names"
                );
            }
        }
    }

    #[test]
    fn partial_signals_still_detect_and_pick_primary() {
        // Codex CLI present (no config); Claude config present (no CLI).
        let home = temp_home("partial");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let plan = test_plan(&home, &|c| c == "codex");
        let claude = plan
            .platforms
            .iter()
            .find(|p| p.id == "claude-code")
            .unwrap();
        let codex = plan.platforms.iter().find(|p| p.id == "codex").unwrap();
        assert!(claude.detected && !claude.cli_present && claude.config_present);
        assert!(codex.detected && codex.cli_present && !codex.config_present);
        // Neither has both signals → primary falls to first detected (claude-code).
        assert_eq!(plan.primary_agent.as_deref(), Some("claude-code"));
        // JSON sync plan covers both detected hosts and never auto-runs registrars.
        let json = cross_platform_init_json(&plan);
        let sync = json["sync_plan"].as_array().unwrap();
        assert_eq!(sync.len(), 2);
        assert_eq!(sync[1]["host"], "codex");
        assert!(sync[1]["mcp_host_command"]
            .as_str()
            .unwrap()
            .starts_with("codex mcp add"));
        assert_eq!(sync[0]["ags_skill_thin_index"], "plan-guarded-apply");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cursor_drift_check_is_marked_reserved() {
        let home = temp_home("cursor");
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        let plan = test_plan(&home, &|_| false);
        let cursor = plan.platforms.iter().find(|p| p.id == "cursor").unwrap();
        assert!(cursor.detected); // config dir present
        assert!(cursor.drift_check.contains("reserved"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codebuddy_code_detects_homebrew_cli_alias() {
        let home = temp_home("codebuddy-cli");
        let plan = test_plan(&home, &|c| c == "codebuddy-code");
        let codebuddy = plan
            .platforms
            .iter()
            .find(|p| p.id == "codebuddy-code")
            .unwrap();
        assert!(codebuddy.cli_present);
        assert!(codebuddy.detected);
        assert!(!codebuddy.config_present);
        assert!(!codebuddy.app_present);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codebuddy_code_detects_macos_app_bundle_without_cli_or_config() {
        let home = temp_home("codebuddy-app");
        let plan = cross_platform_init_plan_with_detectors(&home, &|_| false, &|bundle| {
            bundle == "CodeBuddy CN.app"
        });
        let workbuddy = plan.platforms.iter().find(|p| p.id == "workbuddy").unwrap();
        let codebuddy = plan
            .platforms
            .iter()
            .find(|p| p.id == "codebuddy-code")
            .unwrap();
        assert!(
            !workbuddy.detected,
            "CodeBuddy app must not report WorkBuddy"
        );
        assert!(codebuddy.app_present);
        assert!(codebuddy.detected);
        assert_eq!(plan.primary_agent.as_deref(), Some("codebuddy-code"));
        let json = cross_platform_init_json(&plan);
        let codebuddy_json = json["platforms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "codebuddy-code")
            .unwrap();
        assert_eq!(codebuddy_json["app_present"], true);
        let _ = std::fs::remove_dir_all(&home);
    }
}
