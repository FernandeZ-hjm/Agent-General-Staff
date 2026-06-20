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
