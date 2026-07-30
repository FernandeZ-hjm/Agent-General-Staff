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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub evidence: String,
}

/// Inspect CodeBuddy's documented JSON registration when the optional
/// standalone `codebuddy` CLI is unavailable.
pub fn inspect_codebuddy_mcp_config_at(
    workspace: &std::path::Path,
    home: &std::path::Path,
) -> Option<crate::HostMcpReport> {
    let candidates = [
        (workspace.join(".mcp.json"), "workspace"),
        (home.join(".codebuddy/.mcp.json"), "user"),
        (home.join(".codebuddy/mcp.json"), "user"),
    ];
    let (path, scope) = candidates.into_iter().find(|(path, _)| path.is_file())?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(crate::HostMcpReport {
                host: "codebuddy-code".to_string(),
                status: HostProbeStatus::ConnectionFailed,
                evidence_source: "CodeBuddy MCP config".to_string(),
                servers: Vec::new(),
                evidence: format!("cannot read {}: {error}", path.display()),
            });
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return Some(crate::HostMcpReport {
                host: "codebuddy-code".to_string(),
                status: HostProbeStatus::ConnectionFailed,
                evidence_source: "CodeBuddy MCP config".to_string(),
                servers: Vec::new(),
                evidence: format!("invalid JSON in {}: {error}", path.display()),
            });
        }
    };
    let servers = value
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|servers| servers.iter())
        .map(|(name, entry)| {
            let command = entry
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let args = entry
                .get("args")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flat_map(|args| args.iter())
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let active = !entry
                .get("disabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                && entry
                    .get("enabled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
            McpServerRegistration {
                name: name.clone(),
                active,
                command,
                args,
                transport: Some(
                    if entry
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                    {
                        "http"
                    } else {
                        "stdio"
                    }
                    .to_string(),
                ),
                scope: Some(scope.to_string()),
                evidence: format!("registered in {}", path.display()),
            }
        })
        .collect();
    Some(crate::HostMcpReport {
        host: "codebuddy-code".to_string(),
        status: HostProbeStatus::Ready,
        evidence_source: "CodeBuddy MCP config".to_string(),
        servers,
        evidence: format!("read-only registration from {}", path.display()),
    })
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
            let (name, active, command, args, transport, scope) = match format {
                McpListFormat::Claude => {
                    let (name, rest) = line.split_once(": ")?;
                    let name = name.trim();
                    if name.is_empty() || name.chars().any(char::is_whitespace) {
                        return None;
                    }
                    let invocation = rest
                        .split(" - ")
                        .next()
                        .unwrap_or(rest)
                        .split_whitespace()
                        .collect::<Vec<_>>();
                    (
                        name,
                        rest.contains("Connected") || rest.contains('✔') || rest.contains('✓'),
                        invocation.first().map(|value| (*value).to_string()),
                        invocation
                            .iter()
                            .skip(1)
                            .map(|value| (*value).to_string())
                            .collect(),
                        Some("stdio".to_string()),
                        registration_scope(rest),
                    )
                }
                McpListFormat::Codex => {
                    let columns = line.split_whitespace().collect::<Vec<_>>();
                    let name = *columns.first()?;
                    if name == "Name" || name.chars().all(|character| "-=".contains(character)) {
                        return None;
                    }
                    let command = columns.get(1).map(|value| (*value).to_string());
                    let status_index = columns
                        .iter()
                        .position(|value| *value == "enabled" || *value == "disabled")
                        .unwrap_or(columns.len());
                    (
                        name,
                        line.contains("enabled") && !line.contains("disabled"),
                        command,
                        columns
                            .iter()
                            .take(status_index)
                            .skip(2)
                            .map(|value| (*value).to_string())
                            .collect(),
                        Some("stdio".to_string()),
                        registration_scope(line),
                    )
                }
                McpListFormat::Omp => {
                    let mut columns = line.split('|').map(str::trim);
                    let name = columns.next()?;
                    let transport = columns.next()?;
                    let state = columns.next()?;
                    let invocation = columns
                        .next()
                        .unwrap_or_default()
                        .split_whitespace()
                        .filter(|value| !value.starts_with('['))
                        .collect::<Vec<_>>();
                    if name.is_empty() {
                        return None;
                    }
                    (
                        name,
                        state.eq_ignore_ascii_case("enabled"),
                        invocation.first().map(|value| (*value).to_string()),
                        invocation
                            .iter()
                            .skip(1)
                            .map(|value| (*value).to_string())
                            .collect(),
                        Some(transport.to_ascii_lowercase()),
                        registration_scope(line),
                    )
                }
            };
            Some(McpServerRegistration {
                name: name.to_string(),
                active,
                command,
                args,
                transport,
                scope,
                evidence: line.to_string(),
            })
        })
        .collect()
}

fn registration_scope(line: &str) -> Option<String> {
    ["user", "project", "workspace", "local"]
        .into_iter()
        .find(|scope| line.contains(&format!("[{scope}]")))
        .map(str::to_string)
}

/// Return one matching row from the host's declared protocol surface.
pub fn mcp_server_line(host: &str, server: &str) -> Result<Option<String>, String> {
    let report = inspect_host_mcp(host);
    mcp_server_line_from_report(report, server)
}

pub fn mcp_server_line_at(
    host: &str,
    server: &str,
    current_dir: &std::path::Path,
) -> Result<Option<String>, String> {
    let report = crate::inspect_host_mcp_at(host, current_dir);
    mcp_server_line_from_report(report, server)
}

fn mcp_server_line_from_report(
    report: crate::HostMcpReport,
    server: &str,
) -> Result<Option<String>, String> {
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

pub fn claude_mcp_list_line_at(
    server: &str,
    current_dir: &std::path::Path,
) -> Result<Option<String>, String> {
    mcp_server_line_at("claude-code", server, current_dir)
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
    fn codebuddy_config_is_a_strict_read_only_probe_fallback() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".codebuddy")).unwrap();
        std::fs::write(
            home.join(".codebuddy/.mcp.json"),
            serde_json::json!({
                "mcpServers": {
                    "ags": {
                        "command": "/usr/local/bin/ags",
                        "args": ["mcp", "serve", "--transport", "stdio"]
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let report = inspect_codebuddy_mcp_config_at(&workspace, &home).unwrap();
        assert_eq!(report.status, HostProbeStatus::Ready);
        assert_eq!(report.evidence_source, "CodeBuddy MCP config");
        let registration = report.find("ags").unwrap();
        assert!(registration.active);
        assert_eq!(registration.command.as_deref(), Some("/usr/local/bin/ags"));
        assert_eq!(registration.args, ["mcp", "serve", "--transport", "stdio"]);
        assert_eq!(registration.transport.as_deref(), Some("stdio"));
        assert_eq!(registration.scope.as_deref(), Some("user"));
    }

    #[test]
    fn parsers_keep_registration_identity_and_active_state() {
        let claude = parse_mcp_list(
            McpListFormat::Claude,
            "ags: /bin/ags mcp serve - ✔ Connected\nplugin:memory: node old - ✘ Failed\n",
        );
        assert_eq!(claude[0].name, "ags");
        assert!(claude[0].active);
        assert_eq!(claude[0].command.as_deref(), Some("/bin/ags"));
        assert_eq!(claude[0].args, ["mcp", "serve"]);
        assert_eq!(claude[0].transport.as_deref(), Some("stdio"));
        assert_eq!(claude[1].name, "plugin:memory");
        assert!(!claude[1].active);

        let codex = parse_mcp_list(
            McpListFormat::Codex,
            "Name Command Args Status\nags ags mcp serve enabled\nold ags mcp serve disabled\n",
        );
        assert_eq!(
            codex
                .iter()
                .map(|entry| (entry.name.as_str(), entry.active))
                .collect::<Vec<_>>(),
            vec![("ags", true), ("old", false)]
        );
        assert_eq!(codex[0].command.as_deref(), Some("ags"));
        assert_eq!(codex[0].args, ["mcp", "serve"]);

        let omp = parse_mcp_list(
            McpListFormat::Omp,
            "ags | stdio | enabled | /usr/local/bin/ags mcp serve [user]\n\
             old | stdio | disabled | old mcp serve [project]\n",
        );
        assert_eq!(
            omp.iter()
                .map(|entry| (entry.name.as_str(), entry.active))
                .collect::<Vec<_>>(),
            vec![("ags", true), ("old", false)]
        );
        assert_eq!(omp[0].transport.as_deref(), Some("stdio"));
        assert_eq!(omp[0].scope.as_deref(), Some("user"));
        assert_eq!(omp[0].args, ["mcp", "serve"]);
    }
}
