//! Host identity and hook health (contract v3 §7.5 / §7.11).
//!
//! Any normalized HostId may be admitted (never an allowlist). Registration
//! lives in `ags.toml [hosts]`; `ags doctor` probes hook wiring per host.
//! Hosts without native tool-level hook events (OMP, DSH) degrade to MCP
//! policy and are reported as such — never silently upgraded.

use std::path::Path;

use serde::Serialize;

use crate::config::HostEntry;
use crate::error::{Error, Result};

/// Canonical lowercase-dash form. Rejects empty and non-normalizable ids.
pub fn normalize_host_id(raw: &str) -> Result<String> {
    let collapsed = raw
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty()
        || !collapsed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::new(
            "host_id_invalid",
            format!("`{raw}` does not normalize to a canonical lowercase-dash id"),
        ));
    }
    Ok(match collapsed.as_str() {
        "codex-cli" => "codex".to_string(),
        "claude" => "claude-code".to_string(),
        "codebuddy-code" => "codebuddy".to_string(),
        _ => collapsed,
    })
}

/// Hook wiring file per host (relative to the workspace root). DSH has no
/// hook file and rides MCP; its status derives from its surface.
pub const HOST_HOOK_PATHS: &[(&str, &str)] = &[
    ("claude-code", ".claude/settings.json"),
    ("codex", ".codex/hooks.json"),
    ("cursor", ".cursor/hooks.json"),
    ("codebuddy", ".codebuddy/settings.local.json"),
    ("omp", ".omp/extensions/ags-policy.js"),
];

#[derive(Debug, Clone, Serialize)]
pub struct HookStatus {
    pub host: String,
    pub surface: String,
    /// True when a hook file exists and references `ags-policy` or the
    /// legacy lifecycle adapter, or the host rides MCP as its policy channel.
    pub wired: bool,
    pub mode: String, // "cli" | "mcp" | "hooks" | "unwired"
    /// Configured dispatch capability: the host can spawn child agents for
    /// delegation (CodeBuddy multi-agent, Codex spawn_agent, Claude Task).
    pub dispatch: bool,
}

pub fn hook_health(root: &Path, hosts: &[HostEntry]) -> Vec<HookStatus> {
    hosts
        .iter()
        .map(|entry| {
            let surface = entry.surface.clone();
            let id = normalize_host_id(&entry.id).unwrap_or_else(|_| entry.id.clone());
            if matches!(surface.as_str(), "cli" | "mcp") {
                let mode = if surface == "cli" { "cli" } else { "mcp" };
                return HookStatus {
                    host: id,
                    surface,
                    wired: true,
                    mode: mode.to_string(),
                    dispatch: entry.dispatch,
                };
            }
            let file = HOST_HOOK_PATHS
                .iter()
                .find(|(h, _)| *h == id)
                .map(|(_, f)| root.join(f));
            let wired = file
                .as_ref()
                .map(|f| f.is_file() && mentions_policy(f))
                .unwrap_or(false);
            HookStatus {
                host: id,
                surface,
                wired,
                mode: if wired {
                    "hooks".to_string()
                } else {
                    "unwired".to_string()
                },
                dispatch: entry.dispatch,
            }
        })
        .collect()
}

fn mentions_policy(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains("ags-policy") || text.contains("ags-host"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normalization_is_strict() {
        assert_eq!(normalize_host_id("  Claude Code ").unwrap(), "claude-code");
        assert_eq!(normalize_host_id("CodeBuddy Code").unwrap(), "codebuddy");
        assert_eq!(normalize_host_id("codex-cli").unwrap(), "codex");
        assert!(normalize_host_id("").is_err());
        assert!(normalize_host_id("bad/id").is_err());
    }

    #[test]
    fn mcp_hosts_are_wired_via_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let hosts = vec![
            HostEntry {
                id: "dsh".to_string(),
                surface: "mcp".to_string(),
                dispatch: false,
            },
            HostEntry {
                id: "codex".to_string(),
                surface: "hybrid".to_string(),
                dispatch: true,
            },
        ];
        let status = hook_health(tmp.path(), &hosts);
        assert_eq!(status[0].mode, "mcp");
        assert_eq!(status[1].mode, "unwired");
    }

    #[test]
    fn arbitrary_cli_and_mcp_hosts_need_no_adapter() {
        let tmp = tempfile::tempdir().unwrap();
        let hosts = vec![
            HostEntry {
                id: "future-host".to_string(),
                surface: "cli".to_string(),
                dispatch: true,
            },
            HostEntry {
                id: "another-host".to_string(),
                surface: "mcp".to_string(),
                dispatch: false,
            },
        ];
        let status = hook_health(tmp.path(), &hosts);
        assert!(status.iter().all(|host| host.wired));
        assert_eq!(status[0].mode, "cli");
        assert_eq!(status[1].mode, "mcp");
    }

    #[test]
    fn hook_file_with_policy_is_wired() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        fs::write(
            tmp.path().join(".codex/hooks.json"),
            r#"{"hooks": {"PreToolUse": [{"command": "ags-policy"}]}}"#,
        )
        .unwrap();
        let hosts = vec![HostEntry {
            id: "codex".to_string(),
            surface: "hybrid".to_string(),
            dispatch: true,
        }];
        let status = hook_health(tmp.path(), &hosts);
        assert!(status[0].wired);
        assert_eq!(status[0].mode, "hooks");
    }
}
