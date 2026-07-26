//! Read-only host MCP registration probes.
//!
//! These helpers own host command invocation and output parsing. They never
//! mutate host configuration or invoke an MCP effect.

/// Resolve an executable through the platform PATH rules.
pub fn command_in_path(command: &str) -> Result<String, String> {
    match ags_platform::find_in_path(command) {
        Some(path) => Ok(path.display().to_string()),
        None => Err(format!("{command} not found in PATH")),
    }
}

/// Return the matching Claude Code MCP registration row.
pub fn claude_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "list"])
        .output()
        .map_err(|error| error.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{server}:")))
            .map(|line| line.trim().to_string()))
    } else {
        Err(combined.trim().to_string())
    }
}

/// Return the matching Codex MCP registration row.
pub fn codex_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("codex")
        .args(["mcp", "list"])
        .output()
        .map_err(|error| error.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined
            .lines()
            .find(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with(&format!("{server}:"))
                    || trimmed.starts_with(&format!("{server} "))
            })
            .map(|line| line.trim().to_string()))
    } else {
        Err(combined.trim().to_string())
    }
}

/// List active MCP server identifiers exposed by a supported host.
pub fn mcp_server_ids(host: &str) -> Result<Vec<String>, String> {
    let (program, args): (&str, &[&str]) = match host {
        "claude-code" => ("claude", &["mcp", "list"]),
        "codex" => ("codex", &["mcp", "list"]),
        _ => return Ok(Vec::new()),
    };
    let output = std::process::Command::new(program)
        .args(args)
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
    let mut ids = combined
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("Name ") || line.starts_with("Checking ") {
                return None;
            }
            let active = if host == "claude-code" {
                line.contains("✔ Connected")
            } else {
                line.split_whitespace().any(|field| field == "enabled")
            };
            if !active {
                return None;
            }
            let candidate = if host == "claude-code" {
                line.split_once(':').map(|(name, _)| name)
            } else {
                line.split_whitespace().next()
            }?;
            (!candidate.is_empty()
                && candidate.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "-_:.".contains(character)
                }))
            .then(|| candidate.to_string())
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}
