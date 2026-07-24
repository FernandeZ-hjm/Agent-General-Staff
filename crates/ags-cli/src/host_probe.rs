//! Shared host-probing helpers (PATH lookup + host MCP-registration probes).
//!
//! Kept module-neutral so `ags agents` host governance does not depend on the
//! `ags setup` verify lifecycle. These helpers only read host state; they never
//! mutate host configuration or run external registrars.

pub(crate) fn command_in_path(command: &str) -> Result<String, String> {
    // Cross-platform PATH lookup (replaces shelling out to `which`, which is
    // absent on native Windows). On Windows this also honours `%PATHEXT%`.
    match ags_platform::find_in_path(command) {
        Some(path) => Ok(path.display().to_string()),
        None => Err(format!("{command} not found in PATH")),
    }
}
pub(crate) fn claude_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "list"])
        .output()
        .map_err(|e| e.to_string())?;
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
/// Probe whether AGS MCP is registered in Codex, mirroring `claude_mcp_list_line`.
pub(crate) fn codex_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("codex")
        .args(["mcp", "list"])
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined
            .lines()
            .find(|line| {
                let t = line.trim_start();
                t.starts_with(&format!("{server}:")) || t.starts_with(&format!("{server} "))
            })
            .map(|line| line.trim().to_string()))
    } else {
        Err(combined.trim().to_string())
    }
}

pub(crate) fn mcp_server_ids(host: &str) -> Result<Vec<String>, String> {
    let (program, args): (&str, &[&str]) = match host {
        "claude-code" => ("claude", &["mcp", "list"]),
        "codex" | "omp" => ("codex", &["mcp", "list"]),
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
