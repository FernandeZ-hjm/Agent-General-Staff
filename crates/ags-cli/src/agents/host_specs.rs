use crate::host_platforms::CrossPlatformInitPlan;
use crate::host_probe::{claude_mcp_list_line, codex_mcp_list_line};

/// One row of `ags agents scan`: a detected host + AGS-MCP registration probe.
#[derive(Debug, Clone)]
pub(in crate::agents) struct AgentScanRow {
    pub(crate) id: String,
    pub(crate) display: String,
    pub(crate) cli_present: bool,
    pub(crate) config_present: bool,
    pub(crate) app_present: bool,
    pub(crate) detected: bool,
    pub(crate) is_primary: bool,
    /// Some(true)=registered, Some(false)=not registered, None=not probeable.
    pub(crate) ags_mcp_registered: Option<bool>,
    pub(crate) ags_mcp_evidence: String,
}
/// Build scan rows from a detection plan plus an AGS-MCP registration probe.
/// `probe(host_id)` returns Some((registered, evidence)) for probeable hosts
/// (claude-code / codex) and None for hosts AGS cannot probe (advise instead).
pub(in crate::agents) fn agents_scan_rows(
    plan: &CrossPlatformInitPlan,
    probe: &dyn Fn(&str) -> Option<(bool, String)>,
) -> Vec<AgentScanRow> {
    plan.platforms
        .iter()
        .map(|p| {
            let (registered, evidence) = match probe(&p.id) {
                Some((reg, ev)) => (Some(reg), ev),
                None => (
                    None,
                    "not probeable by AGS — register/verify manually".to_string(),
                ),
            };
            AgentScanRow {
                id: p.id.clone(),
                display: p.display.clone(),
                cli_present: p.cli_present,
                config_present: p.config_present,
                app_present: p.app_present,
                detected: p.detected,
                is_primary: p.is_primary,
                ags_mcp_registered: registered,
                ags_mcp_evidence: evidence,
            }
        })
        .collect()
}
/// The governance chain a host enters once AGS MCP is reachable. Success of
/// `ags agents govern` is the host being able to call `ags_preflight` and then
/// flow through these gates — not AGS writing host config.
pub(in crate::agents) fn agents_governance_chain() -> Vec<&'static str> {
    vec![
        "ags_preflight (host initialization gate)",
        "ags_solution_check (solution/direct-edit/task-card-handoff decision)",
        "ags_task_validate (task-card format gate)",
        "ags_policy_resolve (execution policy)",
        "review gate + verification gate (delivery)",
    ]
}

/// AGS MCP tool surface an operator is choosing to expose when registering the
/// `ags` MCP server in a host. Registration happens at server granularity; this
/// list makes the included tools explicit before the operator acts.
pub(in crate::agents) fn ags_mcp_tool_surface() -> Vec<&'static str> {
    vec![
        "ags_preflight",
        "ags_protocol_status",
        "ags_agent_instructions",
        "ags_task_validate",
        "ags_policy_resolve",
        "ags_solution_check",
        "ags_verify_local",
    ]
}
/// Production probe: claude-code via `claude mcp list`, codex via `codex mcp
/// list`; other hosts not probeable (None → advise).
pub(in crate::agents) fn default_agents_probe(host_id: &str) -> Option<(bool, String)> {
    match host_id {
        "claude-code" => Some(match claude_mcp_list_line("ags") {
            Ok(Some(line)) => (true, line),
            Ok(None) => (false, "not in `claude mcp list`".to_string()),
            Err(e) => (false, format!("claude mcp list unavailable: {e}")),
        }),
        "codex" => Some(match codex_mcp_list_line("ags") {
            Ok(Some(line)) => (true, line),
            Ok(None) => (false, "not in `codex mcp list`".to_string()),
            Err(e) => (false, format!("codex mcp list unavailable: {e}")),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod agents_scan_tests {
    use super::*;
    use crate::host_platforms::cross_platform_init_plan;
    use std::path::PathBuf;

    fn temp_home(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-xplat-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn agents_scan_rows_probe_supported_and_advise_unprobeable() {
        let home = temp_home("agents-scan");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let plan = cross_platform_init_plan(&home, &|c| c == "claude");
        let probe = |id: &str| -> Option<(bool, String)> {
            if id == "claude-code" {
                Some((true, "ags: connected".to_string()))
            } else {
                None
            }
        };
        let rows = agents_scan_rows(&plan, &probe);
        let cc = rows.iter().find(|r| r.id == "claude-code").unwrap();
        assert!(cc.detected);
        assert_eq!(cc.ags_mcp_registered, Some(true));
        let wb = rows.iter().find(|r| r.id == "workbuddy").unwrap();
        assert!(!wb.detected, "tencent host not detected on a bare home");
        assert_eq!(wb.ags_mcp_registered, None, "unprobeable host → advise");
        let _ = std::fs::remove_dir_all(&home);
    }
}
