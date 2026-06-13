//! Diagnostic check functions for suite-doctor.
//!
//! Each check runs a lightweight diagnostic and returns a `Finding`.
//! Checks that shell out are behind simple functions so they can be
//! replaced with stubs in tests — no dynamic dispatch needed.

use crate::types::{Finding, HealthReport};
use serde_yaml::Value as YamlValue;
use std::path::Path;
use std::process::Command;

// ── Evolver / EvoMap checks ────────────────────────────────────────────────

fn is_public_edition(repo_root: &Path) -> bool {
    let workspace = repo_root.join("WORKSPACE.md");
    let claude = repo_root.join("CLAUDE.md");
    for path in [workspace, claude] {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if raw.contains("Public Edition") || raw.contains("public distributable edition") {
                return true;
            }
        }
    }
    false
}

/// Check that `.claude/hooks/evolver-session-end.js` is present on disk.
pub fn evolver_stop_hook_file_present(repo_root: &Path) -> Finding {
    let path = repo_root.join(".claude/hooks/evolver-session-end.js");
    if path.exists() {
        Finding::pass(
            "evolver_stop_hook_file_present",
            ".claude/hooks/evolver-session-end.js found",
        )
    } else if is_public_edition(repo_root) {
        Finding::skip(
            "evolver_stop_hook_file_present",
            "public edition does not require an installed private Stop hook",
        )
    } else {
        Finding::fail(
            "evolver_stop_hook_file_present",
            ".claude/hooks/evolver-session-end.js missing",
            format!("Expected at: {}", path.display()),
        )
    }
}

/// Run `node --check` on the evolver Stop hook script.
pub fn evolver_stop_hook_syntax_ok(repo_root: &Path) -> Finding {
    let path = repo_root.join(".claude/hooks/evolver-session-end.js");
    if !path.exists() {
        return Finding::skip(
            "evolver_stop_hook_syntax_ok",
            "hook file not present — skipping syntax check",
        );
    }
    match Command::new("node")
        .args(["--check", ".claude/hooks/evolver-session-end.js"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Finding::pass(
                    "evolver_stop_hook_syntax_ok",
                    "evolver-session-end.js syntax check passed",
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Finding::fail(
                    "evolver_stop_hook_syntax_ok",
                    "evolver-session-end.js syntax error",
                    format!("node --check failed: {}", stderr.trim()),
                )
            }
        }
        Err(e) => Finding::fail(
            "evolver_stop_hook_syntax_ok",
            "node not available",
            format!("Cannot run node --check: {e}"),
        ),
    }
}

/// Check that the wired Stop hook uses task-bound archive evidence and does not
/// persist local evidence paths into Evolver events.
pub fn evolver_stop_hook_semantics_safe(repo_root: &Path) -> Finding {
    let path = repo_root.join(".claude/hooks/evolver-session-end.js");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Finding::skip(
                "evolver_stop_hook_semantics_safe",
                "public edition checks the portable Stop hook template instead of an installed private hook",
            );
        }
        return Finding::skip(
            "evolver_stop_hook_semantics_safe",
            "hook file not present — skipping semantic safety check",
        );
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::fail(
                "evolver_stop_hook_semantics_safe",
                "cannot read evolver-session-end.js",
                format!("{e}"),
            );
        }
    };

    let mut issues = Vec::new();
    if raw.contains("latestTaskArchiveEvidence") {
        issues.push("uses latestTaskArchiveEvidence instead of task-bound archive lookup");
    }
    if !raw.contains("function resolveTaskId") {
        issues.push("missing resolveTaskId task binding");
    }
    if !raw.contains("function taskArchiveEvidence") {
        issues.push("missing taskArchiveEvidence task-bound archive lookup");
    }
    if !raw.contains("entry.name === taskId")
        || !raw.contains("entry.name.startsWith(`${taskId}-`)")
    {
        issues.push("archive directory matching is not strict taskId / taskId-hyphen");
    }
    if !raw.contains("(taskId ? taskArchiveEvidence(projectDir, taskId) : null)") {
        issues.push("archive evidence is not gated by explicit taskId");
    }
    if raw.contains("path: input.transcript_path") || raw.contains("evidence_path: evidence.path") {
        issues.push("persists local transcript/archive evidence paths");
    }
    if !raw.contains("reference_id: evidence.task_id || ''") || !raw.contains("evidence_path: ''") {
        issues.push("does not store opaque reference_id with blank evidence_path");
    }

    if issues.is_empty() {
        Finding::pass(
            "evolver_stop_hook_semantics_safe",
            "evolver-session-end.js uses task-bound evidence and sanitized method events",
        )
    } else {
        Finding::fail(
            "evolver_stop_hook_semantics_safe",
            "evolver-session-end.js semantic safety check failed",
            format!("Issues: {}", issues.join("; ")),
        )
    }
}

/// Check that `.claude/settings.json` wires the evolver Stop hook.
///
/// This is a **blocking** check: if the hook is not wired, the finding
/// is Fail (not Warn).  The user's requirement is that Stop hook wiring
/// MUST be confirmed before the doctor passes.
pub fn claude_code_stop_hook_wired(repo_root: &Path) -> Finding {
    let path = repo_root.join(".claude/settings.json");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Finding::skip(
                "claude_code_stop_hook_wired",
                "public edition does not require private .claude Stop hook wiring",
            );
        }
        return Finding::fail(
            "claude_code_stop_hook_wired",
            ".claude/settings.json not found",
            "Create .claude/settings.json with Stop hook pointing to evolver-session-end.js",
        );
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::fail(
                "claude_code_stop_hook_wired",
                "cannot read .claude/settings.json",
                format!("{e}"),
            );
        }
    };

    // Parse JSON and walk hooks.Stop[*].hooks[*].command for
    // "evolver-session-end.js".
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Finding::fail(
                "claude_code_stop_hook_wired",
                ".claude/settings.json is not valid JSON",
                format!("{e}"),
            );
        }
    };

    let stop_hooks = match parsed.get("hooks").and_then(|h| h.get("Stop")) {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => {
            return Finding::fail(
                "claude_code_stop_hook_wired",
                "Stop hook not found in .claude/settings.json",
                "Add hooks.Stop array with evolver-session-end.js command",
            );
        }
    };

    let wired = stop_hooks.iter().any(|group| {
        group
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| c.contains("evolver-session-end.js"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });

    if wired {
        Finding::pass(
            "claude_code_stop_hook_wired",
            "Stop hook wired to evolver-session-end.js",
        )
    } else {
        Finding::fail(
            "claude_code_stop_hook_wired",
            "evolver-session-end.js not found in Stop hooks",
            "Ensure .claude/settings.json hooks.Stop includes a command referencing evolver-session-end.js",
        )
    }
}

/// Check that `manifests/runtime-profiles.yaml` exists and contains the
/// expected top-level keys.
pub fn runtime_profile_declared(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/runtime-profiles.yaml");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Finding::skip(
                "runtime_profile_declared",
                "public edition ships runtime profile templates, not installed private runtime-profiles.yaml",
            );
        }
        return Finding::fail(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml not found",
            format!(
                "Expected at: {}. Create it with claude-code-executor and planner profiles.",
                path.display()
            ),
        );
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::fail(
                "runtime_profile_declared",
                "cannot read manifests/runtime-profiles.yaml",
                format!("{e}"),
            );
        }
    };

    let parsed: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Finding::fail(
                "runtime_profile_declared",
                "manifests/runtime-profiles.yaml is not valid YAML",
                format!("{e}"),
            );
        }
    };

    let has_schema = yaml_get(&parsed, "schema_version").is_some();
    let profiles = yaml_get(&parsed, "profiles");
    let has_claude_code = profiles
        .and_then(|profiles| yaml_get(profiles, "claude-code-executor"))
        .is_some();
    let has_planner = profiles
        .and_then(|profiles| yaml_get(profiles, "planner"))
        .is_some();

    if has_schema && has_claude_code && has_planner {
        Finding::pass(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml present with claude-code-executor and planner profiles",
        )
    } else {
        let mut missing = Vec::new();
        if !has_schema {
            missing.push("schema_version");
        }
        if !has_claude_code {
            missing.push("claude-code-executor profile");
        }
        if !has_planner {
            missing.push("planner profile");
        }
        Finding::fail(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml missing required profiles",
            format!("Missing: {}", missing.join(", ")),
        )
    }
}

/// Check that `manifests/mcp-registry.yaml` has a `gep` MCP entry with
/// `status: adopted`.  This is an **info** check — it never blocks.
pub fn mcp_registry_gep_adopted(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Finding::info(
                "mcp_registry_gep_adopted",
                "public edition does not require private mcp-registry.yaml",
            );
        }
        return Finding::warn(
            "mcp_registry_gep_adopted",
            "manifests/mcp-registry.yaml not found",
            format!("Expected at: {}", path.display()),
        );
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::warn(
                "mcp_registry_gep_adopted",
                "cannot read manifests/mcp-registry.yaml",
                format!("{e}"),
            );
        }
    };

    let parsed: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Finding::warn(
                "mcp_registry_gep_adopted",
                "manifests/mcp-registry.yaml is not valid YAML",
                format!("{e}"),
            );
        }
    };

    let gep_status = yaml_get(&parsed, "mcps")
        .and_then(|mcps| mcps.as_sequence())
        .and_then(|mcps| {
            mcps.iter().find_map(|entry| {
                let name = yaml_get(entry, "name").and_then(|value| value.as_str());
                if name == Some("gep") {
                    yaml_get(entry, "status").and_then(|value| value.as_str())
                } else {
                    None
                }
            })
        });

    let has_gep = gep_status.is_some();
    let has_adopted = gep_status == Some("adopted");

    if has_gep && has_adopted {
        Finding::info(
            "mcp_registry_gep_adopted",
            "GEP MCP registered and adopted in mcp-registry.yaml",
        )
    } else if has_gep {
        Finding::warn(
            "mcp_registry_gep_adopted",
            "GEP MCP found but status is not adopted",
            "Review manifests/mcp-registry.yaml gep entry",
        )
    } else {
        Finding::info(
            "mcp_registry_gep_adopted",
            "GEP MCP not registered in mcp-registry.yaml",
        )
    }
}

fn yaml_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value
        .as_mapping()
        .and_then(|map| map.get(YamlValue::String(key.to_string())))
}

// ── Public check functions ───────────────────────────────────────────────

/// Run `git status --porcelain` and report uncommitted changes.
pub fn git_status_check(repo_root: &Path) -> Finding {
    match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Finding::fail(
                    "git-status",
                    "git status failed",
                    format!(
                        "git exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let changed: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
            if changed.is_empty() {
                Finding::pass("git-status", "working tree clean")
            } else {
                Finding::warn(
                    "git-status",
                    format!("{} uncommitted file(s)", changed.len()),
                    format!("Changed: {}", changed.join(", ")),
                )
            }
        }
        Err(e) => Finding::fail(
            "git-status",
            "git not available",
            format!("Failed to run git: {e}"),
        ),
    }
}

/// Run `cargo fmt --check` and report formatting issues.
pub fn cargo_fmt_check(repo_root: &Path) -> Finding {
    match Command::new("cargo")
        .args(["fmt", "--check"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Finding::pass("cargo-fmt", "cargo fmt --check passed")
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Finding::fail(
                    "cargo-fmt",
                    "cargo fmt --check failed",
                    format!("Run `cargo fmt` to fix. {}", stderr.trim()),
                )
            }
        }
        Err(e) => Finding::fail(
            "cargo-fmt",
            "cargo not available",
            format!("Failed to run cargo: {e}"),
        ),
    }
}

/// Check that key workspace files exist on disk.
pub fn workspace_structure_check(repo_root: &Path) -> Vec<Finding> {
    let required = [
        ("Cargo.toml", "workspace root manifest"),
        ("crates", "crates directory"),
        ("scripts/verify.sh", "verification script"),
    ];

    let mut findings = Vec::new();
    for (rel_path, label) in &required {
        let full = repo_root.join(rel_path);
        if full.exists() {
            findings.push(Finding::pass(
                format!("structure-{}", rel_path.replace('/', "-")),
                format!("{label} present"),
            ));
        } else {
            findings.push(Finding::warn(
                format!("structure-{}", rel_path.replace('/', "-")),
                format!("{label} missing"),
                format!("Expected at: {}", full.display()),
            ));
        }
    }
    findings
}

// ── EvoMap proxy / template checks ────────────────────────────────────────

/// Check EvoMap proxy health via authenticated `/proxy/status`.
///
/// Reads `proxy.url` and `proxy.token` from `~/.evolver/settings.json`,
/// sends a GET to `{url}/proxy/status` with `Authorization: Bearer {token}`.
///
/// This check is **always degradable**: missing settings file, unreachable
/// proxy, auth failure, or `last_sync_at=null` never produce a blocking
/// Fail — only Skip or Warn.  Output is sanitized: the token value is
/// never printed.
pub fn evolver_proxy_health_check() -> Finding {
    let home = match ags_platform::home_dir() {
        Some(h) => h,
        None => {
            return Finding::skip(
                "evolver_proxy_health",
                "home directory not set — cannot locate ~/.evolver/settings.json",
            );
        }
    };

    let settings_path = home.join(".evolver/settings.json");
    if !settings_path.exists() {
        return Finding::skip(
            "evolver_proxy_health",
            "~/.evolver/settings.json not found — EvoMap proxy not configured on this machine",
        );
    }

    let raw = match std::fs::read_to_string(&settings_path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::skip(
                "evolver_proxy_health",
                format!("cannot read ~/.evolver/settings.json: {e}"),
            );
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Finding::warn(
                "evolver_proxy_health",
                "~/.evolver/settings.json is not valid JSON",
                format!("Parse error: {e}"),
            );
        }
    };

    let proxy_url = match parsed
        .get("proxy")
        .and_then(|p| p.get("url"))
        .and_then(|v| v.as_str())
    {
        Some(url) => url,
        None => {
            return Finding::skip(
                "evolver_proxy_health",
                "proxy.url not found in ~/.evolver/settings.json",
            );
        }
    };

    let proxy_token = match parsed
        .get("proxy")
        .and_then(|p| p.get("token"))
        .and_then(|v| v.as_str())
    {
        Some(tok) => tok,
        None => {
            return Finding::warn(
                "evolver_proxy_health",
                "proxy.token not found in ~/.evolver/settings.json",
                "EvoMap proxy is configured but auth token is missing.",
            );
        }
    };

    // Sanitized display: report only that a token is configured and its
    // length.  Never include token content (prefix, suffix, or fragment)
    // — even partial token disclosure weakens the auth boundary for no
    // operational gain.
    let token_label = format!("token_present=true, len={}", proxy_token.len());

    let health_url = format!("{}/proxy/status", proxy_url.trim_end_matches('/'));

    match http_get_json(&health_url, proxy_token) {
        Ok((status_code, body)) => {
            if status_code == 200 {
                let last_sync = body
                    .get("last_sync_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("null");
                let msg = format!(
                    "EvoMap proxy reachable and authenticated ({token_label}, last_sync_at={last_sync})"
                );
                Finding::pass("evolver_proxy_health", msg)
            } else if status_code == 401 || status_code == 403 {
                Finding::warn(
                    "evolver_proxy_health",
                    format!(
                        "EvoMap proxy auth failed (HTTP {status_code}) — token may be expired ({token_label})"
                    ),
                    "Check proxy.token in ~/.evolver/settings.json and restart the proxy.",
                )
            } else {
                Finding::warn(
                    "evolver_proxy_health",
                    format!("EvoMap proxy returned unexpected HTTP {status_code} ({token_label})"),
                    "Check EvoMap proxy status and logs.",
                )
            }
        }
        Err(e) => Finding::warn(
            "evolver_proxy_health",
            format!("EvoMap proxy not reachable at {health_url} ({token_label})"),
            format!("Connection error: {e}"),
        ),
    }
}

/// Minimal HTTP GET + JSON parse using raw TCP (no dependency on curl/reqwest).
///
/// Returns `(status_code, parsed_json_body)` or an error string.
fn http_get_json(url_str: &str, bearer_token: &str) -> Result<(u16, serde_json::Value), String> {
    // Parse URL: http://host:port/path
    let (host, port, path) = parse_http_url(url_str)?;

    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};

    // Resolve hostname (supports both IP and localhost) via ToSocketAddrs.
    let sock_addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {host}:{port}: {e}"))?
        .collect();
    if sock_addrs.is_empty() {
        return Err(format!("no addresses resolved for {host}:{port}"));
    }

    let mut last_error = None;
    let mut stream = None;
    for addr in &sock_addrs {
        match TcpStream::connect_timeout(addr, std::time::Duration::from_secs(5)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => {
                last_error = Some(format!("{addr}: {e}"));
            }
        }
    }
    let mut stream = stream.ok_or_else(|| {
        format!(
            "TCP connect to {host}:{port}: {}",
            last_error.unwrap_or_else(|| "all resolved addresses failed".to_string())
        )
    })?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {bearer_token}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read error: {e}"))?;

    let response_str =
        String::from_utf8(response).map_err(|e| format!("invalid UTF-8 in response: {e}"))?;

    // Parse HTTP status line
    let status_line = response_str.lines().next().ok_or("empty response")?;
    let status_code = parse_http_status(status_line)?;

    // Find JSON body after headers
    let body_start = response_str
        .find("\r\n\r\n")
        .map(|i| i + 4)
        .or_else(|| response_str.find("\n\n").map(|i| i + 2))
        .ok_or("no HTTP body found")?;
    let body = &response_str[body_start..];

    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON in proxy response: {e}"))?;

    Ok((status_code, json))
}

fn parse_http_url(url_str: &str) -> Result<(&str, u16, String), String> {
    let without_scheme = url_str
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// URLs supported, got: {url_str}"))?;
    let (host_port, path) = without_scheme
        .split_once('/')
        .unwrap_or((without_scheme, "/"));
    let (host, port_str) = host_port.split_once(':').unwrap_or((host_port, "80"));
    let port: u16 = port_str
        .parse()
        .map_err(|_| format!("invalid port: {port_str}"))?;
    let path_with_slash = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Ok((host, port, path_with_slash))
}

fn parse_http_status(status_line: &str) -> Result<u16, String> {
    // "HTTP/1.1 200 OK" → 200
    let parts: Vec<&str> = status_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("invalid HTTP status line: {status_line}"));
    }
    parts[1]
        .parse::<u16>()
        .map_err(|_| format!("invalid HTTP status code: {}", parts[1]))
}

/// Check that `manifests/templates/runtime-profiles.template.yaml` exists.
pub fn runtime_profile_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/runtime-profiles.template.yaml");
    if path.exists() {
        Finding::info(
            "runtime_profile_template_exists",
            "manifests/templates/runtime-profiles.template.yaml found",
        )
    } else {
        Finding::warn(
            "runtime_profile_template_exists",
            "manifests/templates/runtime-profiles.template.yaml missing",
            format!(
                "Portable runtime profile template not found at {}. Bootstrap and migration may lack EvoMap profile support.",
                path.display()
            ),
        )
    }
}

/// Check that `manifests/templates/hooks/codex-planner-recall.template.json` exists.
pub fn codex_planner_hook_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/hooks/codex-planner-recall.template.json");
    if path.exists() {
        Finding::info(
            "codex_planner_hook_template_exists",
            "manifests/templates/hooks/codex-planner-recall.template.json found",
        )
    } else {
        Finding::warn(
            "codex_planner_hook_template_exists",
            "manifests/templates/hooks/codex-planner-recall.template.json missing",
            format!(
                "Codex planner hook template not found at {}. Planner pre-solution recall setup may be incomplete.",
                path.display()
            ),
        )
    }
}

/// Check that `manifests/templates/hooks/claude-code-executor-stop.template.js`
/// exists and passes `node --check`.
pub fn claude_code_stop_hook_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/hooks/claude-code-executor-stop.template.js");
    if !path.exists() {
        return Finding::warn(
            "claude_code_stop_hook_template_exists",
            "manifests/templates/hooks/claude-code-executor-stop.template.js missing",
            format!(
                "Claude Code Stop hook template not found at {}. Bootstrap and migration may lack hook support.",
                path.display()
            ),
        );
    }

    match Command::new("node")
        .args([
            "--check",
            "manifests/templates/hooks/claude-code-executor-stop.template.js",
        ])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Finding::info(
                    "claude_code_stop_hook_template_exists",
                    "manifests/templates/hooks/claude-code-executor-stop.template.js found and syntax OK",
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Finding::warn(
                    "claude_code_stop_hook_template_exists",
                    "claude-code-executor-stop.template.js syntax error",
                    format!("node --check failed: {}", stderr.trim()),
                )
            }
        }
        Err(e) => Finding::warn(
            "claude_code_stop_hook_template_exists",
            "node not available",
            format!("Cannot run node --check on template: {e}"),
        ),
    }
}

/// Run all default suite-doctor checks and populate a `HealthReport`.
///
/// The `repo_root` is typically the current working directory or a
/// configured suite root.
pub fn run_checks(report: &mut HealthReport, repo_root: &Path) {
    report.add(git_status_check(repo_root));
    report.add(cargo_fmt_check(repo_root));
    for finding in workspace_structure_check(repo_root) {
        report.add(finding);
    }
    // ── Evolver / EvoMap checks ────────────────────────────────────────
    report.add(evolver_stop_hook_file_present(repo_root));
    report.add(evolver_stop_hook_syntax_ok(repo_root));
    report.add(evolver_stop_hook_semantics_safe(repo_root));
    report.add(claude_code_stop_hook_wired(repo_root));
    report.add(runtime_profile_declared(repo_root));
    report.add(mcp_registry_gep_adopted(repo_root));
    // ── EvoMap proxy health (always degradable) ────────────────────────
    report.add(evolver_proxy_health_check());
    // ── Portable template checks ───────────────────────────────────────
    report.add(runtime_profile_template_exists(repo_root));
    report.add(codex_planner_hook_template_exists(repo_root));
    report.add(claude_code_stop_hook_template_exists(repo_root));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CheckStatus, Severity};

    // ── workspace_structure_check (pure Rust, no shell-out) ───────────

    #[test]
    fn workspace_structure_finds_cargo_toml() {
        // Use the workspace root (where Cargo.toml lives).
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let findings = workspace_structure_check(repo_root);

        // At minimum we should have 3 findings
        assert_eq!(findings.len(), 3);
        // Cargo.toml should be present → pass
        let cargo_finding = findings
            .iter()
            .find(|f| f.check_name == "structure-Cargo.toml")
            .unwrap();
        assert_eq!(cargo_finding.severity, Severity::Info);
    }

    #[test]
    fn workspace_structure_reports_missing_path() {
        let tmp = std::env::temp_dir().join("ags-suite-doctor-nonexistent-test");
        // Ensure it doesn't exist
        let _ = std::fs::remove_dir_all(&tmp);
        let findings = workspace_structure_check(&tmp);

        // All 3 should be missing → warn
        assert_eq!(findings.len(), 3);
        for f in &findings {
            assert_eq!(f.severity, Severity::Warn);
            assert!(f.message.contains("missing"));
        }
    }

    // ── Finding constructors smoke ────────────────────────────────────

    #[test]
    fn git_status_pass_has_correct_shape() {
        let f = Finding::pass("git-status", "working tree clean");
        assert_eq!(f.check_name, "git-status");
        assert_eq!(f.severity, Severity::Info);
        assert!(f.detail.is_none());
    }

    #[test]
    fn cargo_fmt_fail_has_detail() {
        let f = Finding::fail("cargo-fmt", "failed", "run cargo fmt");
        assert_eq!(f.severity, Severity::Fail);
        assert_eq!(f.detail.as_deref(), Some("run cargo fmt"));
    }

    // ── Evolver / EvoMap checks ────────────────────────────────────────

    #[test]
    fn evolver_stop_hook_file_present_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = evolver_stop_hook_file_present(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Skip);
        } else {
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.check_name, "evolver_stop_hook_file_present");
    }

    #[test]
    fn evolver_stop_hook_file_missing_in_empty_dir() {
        let tmp = std::env::temp_dir().join("ags-evolver-hook-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = evolver_stop_hook_file_present(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn evolver_stop_hook_semantics_safe_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = evolver_stop_hook_semantics_safe(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Skip);
        } else {
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.check_name, "evolver_stop_hook_semantics_safe");
    }

    #[test]
    fn evolver_stop_hook_semantics_safe_blocks_latest_archive_lookup() {
        let tmp = std::env::temp_dir().join("ags-stop-hook-unsafe-semantics-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".claude/hooks")).unwrap();
        std::fs::write(
            tmp.join(".claude/hooks/evolver-session-end.js"),
            r#"
function latestTaskArchiveEvidence(projectDir) { return projectDir; }
function methodEventFromEvidence(evidence) {
  return { evidence_path: evidence.path || '' };
}
const evidence = latestTaskArchiveEvidence(process.cwd());
"#,
        )
        .unwrap();

        let f = evolver_stop_hook_semantics_safe(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert!(f
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("latestTaskArchiveEvidence"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_wired_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = claude_code_stop_hook_wired(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Skip);
        } else {
            // Private/stable runtime should wire the hook.
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.check_name, "claude_code_stop_hook_wired");
    }

    #[test]
    fn claude_code_stop_hook_wired_blocking_on_missing_file() {
        let tmp = std::env::temp_dir().join("ags-stop-hook-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = claude_code_stop_hook_wired(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_wired_blocking_on_bad_json() {
        let tmp = std::env::temp_dir().join("ags-stop-hook-bad-json-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp.join(".claude")).unwrap();
        std::fs::write(&tmp.join(".claude/settings.json"), "not json").unwrap();
        let f = claude_code_stop_hook_wired(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_wired_blocking_on_missing_stop_key() {
        let tmp = std::env::temp_dir().join("ags-stop-hook-no-stop-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp.join(".claude")).unwrap();
        std::fs::write(
            &tmp.join(".claude/settings.json"),
            r#"{"hooks": {"SessionStart": []}}"#,
        )
        .unwrap();
        let f = claude_code_stop_hook_wired(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn runtime_profile_declared_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = runtime_profile_declared(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Skip);
        } else {
            // Private/stable runtime should declare installed profiles.
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.check_name, "runtime_profile_declared");
    }

    #[test]
    fn runtime_profile_declared_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-runtime-profile-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = runtime_profile_declared(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn runtime_profile_declared_fails_on_invalid_yaml() {
        let tmp = std::env::temp_dir().join("ags-runtime-profile-invalid-yaml-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("manifests")).unwrap();
        std::fs::write(
            tmp.join("manifests/runtime-profiles.yaml"),
            "schema_version: \"1.0\"\nprofiles: [claude-code-executor\n",
        )
        .unwrap();
        let f = runtime_profile_declared(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mcp_registry_gep_adopted_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = mcp_registry_gep_adopted(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Pass);
            assert!(f.message.contains("public edition"));
        } else {
            // GEP is adopted — expect Pass (info severity)
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(f.check_name, "mcp_registry_gep_adopted");
    }

    #[test]
    fn public_edition_runtime_checks_are_degradable() {
        let tmp = std::env::temp_dir().join("ags-public-doctor-degradable-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("WORKSPACE.md"),
            "# Agent General Staff 2.0 — Public Edition Workspace\n",
        )
        .unwrap();

        assert_eq!(
            evolver_stop_hook_file_present(&tmp).status,
            CheckStatus::Skip
        );
        assert_eq!(
            evolver_stop_hook_semantics_safe(&tmp).status,
            CheckStatus::Skip
        );
        assert_eq!(claude_code_stop_hook_wired(&tmp).status, CheckStatus::Skip);
        assert_eq!(runtime_profile_declared(&tmp).status, CheckStatus::Skip);
        assert_eq!(mcp_registry_gep_adopted(&tmp).status, CheckStatus::Pass);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mcp_registry_gep_adopted_warn_on_missing_file() {
        let tmp = std::env::temp_dir().join("ags-mcp-registry-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = mcp_registry_gep_adopted(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── EvoMap proxy health check tests ────────────────────────────────

    #[test]
    fn evolver_proxy_health_check_never_produces_fail() {
        // This check must ALWAYS be degradable — it never returns Fail severity.
        let f = evolver_proxy_health_check();
        assert!(
            f.severity != Severity::Fail,
            "evolver_proxy_health_check must never produce Fail (got {:?})",
            f.severity
        );
        // Status must be Pass, Skip, or Warn — never Fail
        assert!(
            matches!(
                f.status,
                CheckStatus::Pass | CheckStatus::Skip | CheckStatus::Warn
            ),
            "evolver_proxy_health_check must never return Fail status"
        );
    }

    #[test]
    fn evolver_proxy_health_check_output_is_sanitized() {
        let f = evolver_proxy_health_check();
        // The message must not contain a raw 64-char hex token
        let hex_token_pattern = |s: &str| {
            let chars: Vec<char> = s.chars().collect();
            chars.len() >= 64 && chars.iter().all(|c| c.is_ascii_hexdigit())
        };
        assert!(
            !hex_token_pattern(&f.message),
            "evolver_proxy_health_check message must not contain raw token: {}",
            f.message
        );
        if let Some(ref detail) = f.detail {
            assert!(
                !hex_token_pattern(detail),
                "evolver_proxy_health_check detail must not contain raw token: {detail}"
            );
        }
    }

    #[test]
    fn http_parse_url_valid() {
        let (host, port, path) = parse_http_url("http://127.0.0.1:19821/proxy/status").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 19821);
        assert_eq!(path, "/proxy/status");
    }

    #[test]
    fn http_parse_url_default_port() {
        let (host, port, path) = parse_http_url("http://localhost/health").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 80);
        assert_eq!(path, "/health");
    }

    #[test]
    fn http_parse_url_rejects_https() {
        assert!(parse_http_url("https://example.com/status").is_err());
    }

    #[test]
    fn http_parse_status_valid() {
        assert_eq!(parse_http_status("HTTP/1.1 200 OK").unwrap(), 200);
        assert_eq!(parse_http_status("HTTP/1.1 401 Unauthorized").unwrap(), 401);
    }

    #[test]
    fn http_parse_url_accepts_localhost_hostname() {
        let (host, port, path) = parse_http_url("http://localhost:19821/proxy/status").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 19821);
        assert_eq!(path, "/proxy/status");
    }

    // ── Template existence tests ───────────────────────────────────────

    #[test]
    fn runtime_profile_template_exists_when_file_present() {
        let tmp = std::env::temp_dir().join("ags-runtime-template-present-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("manifests/templates")).unwrap();
        std::fs::write(
            tmp.join("manifests/templates/runtime-profiles.template.yaml"),
            "schema_version: \"1.0\"\nprofiles: []\n",
        )
        .unwrap();

        let f = runtime_profile_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Pass);
        assert_eq!(f.check_name, "runtime_profile_template_exists");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn runtime_profile_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = runtime_profile_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn codex_planner_hook_template_exists_when_file_present() {
        let tmp = std::env::temp_dir().join("ags-planner-template-present-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("manifests/templates/hooks")).unwrap();
        std::fs::write(
            tmp.join("manifests/templates/hooks/codex-planner-recall.template.json"),
            "{}\n",
        )
        .unwrap();

        let f = codex_planner_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Pass);
        assert_eq!(f.check_name, "codex_planner_hook_template_exists");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn codex_planner_hook_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-planner-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = codex_planner_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_template_exists_when_file_present() {
        let tmp = std::env::temp_dir().join("ags-stop-template-present-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("manifests/templates/hooks")).unwrap();
        std::fs::write(
            tmp.join("manifests/templates/hooks/claude-code-executor-stop.template.js"),
            "console.log('ok');\n",
        )
        .unwrap();

        let f = claude_code_stop_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Pass);
        assert_eq!(f.check_name, "claude_code_stop_hook_template_exists");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-stop-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = claude_code_stop_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
