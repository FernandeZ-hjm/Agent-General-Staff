//! Read-only host MCP registration probes.
//!
//! These helpers own host command invocation and output parsing. They never
//! mutate host configuration or invoke an MCP effect.

use crate::{platform_spec, McpListFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
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
            };
            Some(McpServerRegistration {
                name: name.to_string(),
                active,
                evidence: line.to_string(),
            })
        })
        .collect()
}

/// Return one matching row from a host's own live registrar surface.
///
/// Inherited configuration sources such as OMP's Codex registry are excluded:
/// they may support capability visibility, but cannot prove a live OMP host.
pub fn mcp_server_line(host: &str, server: &str) -> Result<Option<String>, String> {
    let spec = platform_spec(host).ok_or_else(|| format!("unknown host: {host}"))?;
    let probe = spec
        .mcp_probe
        .filter(|probe| probe.live_runtime_probe)
        .ok_or_else(|| format!("{host} has no live MCP registrar probe"))?;
    let output = std::process::Command::new(probe.program)
        .args(probe.args)
        .output()
        .map_err(|error| error.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(parse_mcp_list(probe.format, &combined)
            .into_iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(server))
            .map(|entry| entry.evidence))
    } else {
        Err(combined.trim().to_string())
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
    let Some(probe) = platform_spec(host)
        .and_then(|spec| spec.mcp_probe)
        .filter(|probe| probe.live_runtime_probe)
    else {
        return Ok(Vec::new());
    };
    let output = std::process::Command::new(probe.program)
        .args(probe.args)
        .output()
        .map_err(|error| error.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        return Err(combined.trim().to_string());
    }
    let mut ids = parse_mcp_list(probe.format, &combined)
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
    }
}
