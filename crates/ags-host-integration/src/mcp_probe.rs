//! Read-only host MCP registration probes.
//!
//! These helpers own host command invocation and output parsing. They never
//! mutate host configuration or invoke an MCP effect.

use crate::{inspect_host_mcp, HostProbeStatus, McpListFormat};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRegistration {
    pub name: String,
    pub active: bool,
    pub evidence: String,
}

/// Resolve an executable through the platform PATH rules.
pub fn command_in_path(command: &str) -> Result<String, String> {
    match ags_platform::find_in_path(command) {
        Some(path) => Ok(path.display().to_string()),
        None => Err(format!("{command} not found in PATH")),
    }
}

pub fn parse_mcp_list(format: McpListFormat, stdout: &str) -> Vec<McpServerRegistration> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, active) = match format {
                McpListFormat::Claude => {
                    let (name, rest) = line.split_once(": ")?;
                    let name = name.trim();
                    if name.is_empty() || name.chars().any(char::is_whitespace) {
                        return None;
                    }
                    (
                        name,
                        rest.contains("Connected") || rest.contains('✔') || rest.contains('✓'),
                    )
                }
                McpListFormat::Codex => {
                    let name = line.split_whitespace().next()?;
                    if name == "Name" || name.chars().all(|character| "-=".contains(character)) {
                        return None;
                    }
                    (name, line.contains("enabled") && !line.contains("disabled"))
                }
                McpListFormat::Omp => {
                    let mut columns = line.split('|').map(str::trim);
                    let name = columns.next()?;
                    let _transport = columns.next()?;
                    let state = columns.next()?;
                    if name.is_empty() {
                        return None;
                    }
                    (name, state.eq_ignore_ascii_case("enabled"))
                }
            };
            Some(McpServerRegistration {
                name: name.to_string(),
                active,
                evidence: line.to_string(),
            })
        })
        .collect()
}

/// Return one matching row from the host's declared protocol surface.
pub fn mcp_server_line(host: &str, server: &str) -> Result<Option<String>, String> {
    let report = inspect_host_mcp(host);
    if report.status == HostProbeStatus::Ready {
        Ok(report.find(server).map(|entry| entry.evidence.clone()))
    } else {
        Err(format!("{}: {}", report.evidence_source, report.evidence))
    }
}

/// Compatibility wrappers around the canonical host probe.
pub fn claude_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    mcp_server_line("claude-code", server)
}

pub fn codex_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    mcp_server_line("codex", server)
}

/// List active MCP server identifiers exposed by a supported host.
pub fn mcp_server_ids(host: &str) -> Result<Vec<String>, String> {
    let report = inspect_host_mcp(host);
    if report.status != HostProbeStatus::Ready {
        if report.status == HostProbeStatus::ProtocolUnsupported {
            return Ok(Vec::new());
        }
        return Err(format!("{}: {}", report.evidence_source, report.evidence));
    }
    let mut ids = report
        .servers
        .into_iter()
        .filter(|entry| entry.active)
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_keep_registration_identity_and_active_state() {
        let claude = parse_mcp_list(
            McpListFormat::Claude,
            "ags: /bin/ags - ✔ Connected\nplugin:memory: node - ✘ Failed\n",
        );
        assert_eq!(claude[0].name, "ags");
        assert!(claude[0].active);
        assert_eq!(claude[1].name, "plugin:memory");
        assert!(!claude[1].active);

        let codex = parse_mcp_list(
            McpListFormat::Codex,
            "Name Command Status\nags ags enabled\nold ags disabled\n",
        );
        assert_eq!(
            codex
                .iter()
                .map(|entry| (entry.name.as_str(), entry.active))
                .collect::<Vec<_>>(),
            vec![("ags", true), ("old", false)]
        );

        let omp = parse_mcp_list(
            McpListFormat::Omp,
            "ags | stdio | enabled | /usr/local/bin/ags [user]\n\
             old | stdio | disabled | old [project]\n",
        );
        assert_eq!(
            omp.iter()
                .map(|entry| (entry.name.as_str(), entry.active))
                .collect::<Vec<_>>(),
            vec![("ags", true), ("old", false)]
        );
    }
}
