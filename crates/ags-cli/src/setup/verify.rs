use super::{
    claude_ags_command_path, codex_ags_named_skill_agent_metadata_path, codex_ags_named_skill_path,
    retired_codex_ags_skill_dirs, PRIVATE_INSTALL_SCHEMA,
};
use crate::context::{private_install_target, sanitize_name, AGS_VERSION};
use crate::host_probe::{claude_mcp_list_line, command_in_path};
use crate::setup::templates::codex_ags_command_skill_specs;
use std::path::{Path, PathBuf};

fn json_file_ok(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str::<serde_json::Value>(&text)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
fn text_file_contains_no_secret_markers(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if has_token_like_secret(&text, "Bearer ", 20) {
        return Err("contains token-like Bearer secret".to_string());
    }
    if has_token_like_secret(&text, "sk-", 20) {
        return Err("contains token-like sk secret".to_string());
    }
    Ok(())
}
fn has_token_like_secret(text: &str, prefix: &str, min_tail: usize) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(prefix) {
        let tail_start = start + offset + prefix.len();
        let tail = &text[tail_start..];
        let token_len = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if token_len >= min_tail {
            return true;
        }
        start = tail_start;
    }
    false
}
fn mcp_smoke_current_exe() -> Result<(), String> {
    use std::io::Write;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .args(["mcp", "serve", "--transport", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ags-install-verify\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"ags_solution_check\",\"arguments\":{\"summary\":\"before preflight\"}}}\n"
    );
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("\"id\":1") || !stdout.contains("\"result\"") {
        return Err("initialize response missing".to_string());
    }
    if !stdout.contains("\"id\":2") || !stdout.contains("AGS Initialization Gate") {
        return Err("preflight gate error response missing".to_string());
    }
    Ok(())
}
fn claude_mcp_get(server: &str) -> Result<String, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "get", server])
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined)
    } else {
        Err(combined.trim().to_string())
    }
}
fn add_codegraph_claude_checks(report: &mut suite_doctor::HealthReport) {
    match claude_mcp_list_line("codegraph") {
        Ok(Some(line)) if line.contains("Connected") => report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP includes connected codegraph",
        )),
        Ok(Some(line)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP codegraph is configured but not connected",
            line,
        )),
        Ok(None) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP does not include codegraph",
            "run `claude mcp add -s user codegraph -- codegraph serve --mcp`",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "cannot verify Claude Code global MCP codegraph entry",
            e,
        )),
    }

    match claude_mcp_get("codegraph") {
        Ok(detail) if detail.contains("codegraph") && detail.contains("serve --mcp") => {
            report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-codegraph-command",
                "Claude Code codegraph MCP uses `codegraph serve --mcp`",
            ));
        }
        Ok(detail) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-command",
            "Claude Code codegraph MCP does not use `codegraph serve --mcp`",
            detail,
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-command",
            "cannot inspect Claude Code codegraph MCP command",
            e,
        )),
    }

    match command_in_path("codegraph") {
        Ok(path) => report.add(suite_doctor::Finding::pass(
            "private-install-codegraph-cli",
            format!("codegraph CLI available at {path}"),
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-codegraph-cli",
            "codegraph CLI is not available on PATH",
            format!("install codegraph before relying on code intelligence. {e}"),
        )),
    }
}
pub(crate) fn cmd_private_verify(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags verify: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let mut report = suite_doctor::HealthReport::new("private-install-verify");

    let required = [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
        "bin/ags-mcp-stdio.sh",
    ];

    for rel in required {
        let path = target.join(rel);
        if path.exists() {
            report.add(suite_doctor::Finding::pass(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("present: {rel}"),
            ));
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("missing: {rel}"),
                path.display().to_string(),
            ));
        }
    }

    let claude_command_path = claude_ags_command_path();
    if claude_command_path.exists() {
        report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-slash-command-present",
            format!("present: {}", claude_command_path.display()),
        ));
        match std::fs::read_to_string(&claude_command_path) {
            Ok(content) if content.contains("ags_preflight") && content.contains(AGS_VERSION) => {
                report.add(suite_doctor::Finding::pass(
                    "private-install-claude-code-slash-command-content",
                    "Claude Code /ags command references AGS preflight and current version",
                ));
            }
            Ok(_) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-content",
                "Claude Code /ags command content is stale",
                format!(
                    "expected ags_preflight and version {AGS_VERSION} in {}",
                    claude_command_path.display()
                ),
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-content",
                "cannot read Claude Code /ags command",
                e.to_string(),
            )),
        }
        match text_file_contains_no_secret_markers(&claude_command_path) {
            Ok(()) => report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan OK: Claude Code /ags command",
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan failed: Claude Code /ags command",
                e,
            )),
        }
    } else {
        report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-slash-command-present",
            "missing Claude Code /ags command",
            format!(
                "rerun `ags setup --yes` to create {}",
                claude_command_path.display()
            ),
        ));
    }

    for retired_dir in retired_codex_ags_skill_dirs() {
        let check_suffix = sanitize_name(&retired_dir.to_string_lossy());
        if retired_dir.exists() {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                "retired Codex AGS visible skill still exists",
                format!(
                    "rerun `ags setup --yes --force` to remove {}",
                    retired_dir.display()
                ),
            ));
        } else {
            report.add(suite_doctor::Finding::pass(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                format!(
                    "retired Codex AGS visible skill absent: {}",
                    retired_dir.display()
                ),
            ));
        }
    }

    for (name, display_name, _, _, summary) in codex_ags_command_skill_specs() {
        let skill_path = codex_ags_named_skill_path(name);
        let check_suffix = sanitize_name(name);
        if skill_path.exists() {
            match std::fs::read_to_string(&skill_path) {
                Ok(content)
                    if content.contains(&format!("name: \"{name}\""))
                        && content.contains("ags session preflight --for codex")
                        && content.contains(AGS_VERSION) =>
                {
                    report.add(suite_doctor::Finding::pass(
                        format!("private-install-codex-command-skill-{check_suffix}"),
                        format!("Codex command skill present: {name}"),
                    ));
                }
                Ok(_) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("Codex command skill content is stale: {name}"),
                    format!("expected {display_name}, {summary}, and version {AGS_VERSION}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("cannot read Codex command skill: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-codex-command-skill-{check_suffix}"),
                format!("missing Codex command skill: {name}"),
                skill_path.display().to_string(),
            ));
        }

        let metadata_path = codex_ags_named_skill_agent_metadata_path(name);
        if metadata_path.exists() {
            match std::fs::read_to_string(&metadata_path) {
                Ok(content) if content.contains(&format!("display_name: \"{display_name}\"")) => {
                    report.add(suite_doctor::Finding::pass(
                        format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                        format!("Codex command skill metadata present: {name}"),
                    ));
                }
                Ok(_) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("Codex command skill metadata is stale: {name}"),
                    metadata_path.display().to_string(),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("cannot read Codex command skill metadata: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                format!("missing Codex command skill metadata: {name}"),
                metadata_path.display().to_string(),
            ));
        }
    }

    match claude_mcp_list_line("ags") {
        Ok(Some(line)) if line.contains("Connected") => report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP includes connected ags",
        )),
        Ok(Some(line)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP ags is configured but not connected",
            line,
        )),
        Ok(None) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP does not include ags",
            "run `/ags setup` or `ags setup --yes --register-claude`",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "cannot verify Claude Code global MCP ags entry",
            e,
        )),
    }

    match (claude_mcp_get("ags"), command_in_path("ags")) {
        (Ok(detail), Ok(ags_path)) if detail.contains(&ags_path) => {
            report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-ags-command",
                "Claude Code ags MCP uses installed AGS binary",
            ));
        }
        (Ok(detail), Ok(ags_path)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "Claude Code ags MCP does not use the installed AGS binary",
            format!("expected command: {ags_path}\n{detail}"),
        )),
        (Ok(detail), Err(e)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "cannot confirm installed AGS binary path",
            format!("{e}\n{detail}"),
        )),
        (Err(e), _) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "cannot inspect Claude Code ags MCP command",
            e,
        )),
    }

    add_codegraph_claude_checks(&mut report);

    for rel in [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "hooks/codex-planner-recall.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match json_file_ok(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-json-{}", sanitize_name(rel)),
                    format!("valid JSON: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-json-{}", sanitize_name(rel)),
                    format!("invalid JSON: {rel}"),
                    e,
                )),
            }
        }
    }

    for rel in [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match text_file_contains_no_secret_markers(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan OK: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan failed: {rel}"),
                    e,
                )),
            }
        }
    }

    match std::process::Command::new("node")
        .arg("--check")
        .arg(target.join("hooks/claude-code-executor-stop.js"))
        .output()
    {
        Ok(output) if output.status.success() => report.add(suite_doctor::Finding::pass(
            "private-install-node-check",
            "node --check hook OK",
        )),
        Ok(output) => report.add(suite_doctor::Finding::fail(
            "private-install-node-check",
            "node --check hook failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Err(e) => report.add(suite_doctor::Finding::warn(
            "private-install-node-check",
            "node unavailable; skipped hook syntax check",
            e.to_string(),
        )),
    }

    match mcp_smoke_current_exe() {
        Ok(()) => report.add(suite_doctor::Finding::pass(
            "private-install-mcp-smoke",
            "ags mcp serve stdio smoke OK",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-mcp-smoke",
            "ags mcp serve stdio smoke failed",
            e,
        )),
    }

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PRIVATE_INSTALL_SCHEMA,
                "profile": profile,
                "target": target.to_string_lossy(),
                "report": report,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => println!("{}", suite_doctor::render_text(&report)),
    }
    std::process::exit(report.exit_code());
}
