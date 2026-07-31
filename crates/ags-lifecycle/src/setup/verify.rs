use super::templates::codex_ags_command_skill_specs;
use super::{
    claude_ags_command_path, codex_ags_named_skill_agent_metadata_path, codex_ags_named_skill_path,
    retired_codex_ags_skill_dirs,
};
use super::{claude_mcp_list_line_at, command_in_path, sanitize_name, AGS_VERSION};
use std::path::Path;

fn add_install_content_conformance(
    report: &mut crate::setup::SetupReport,
    target: &Path,
    home: &Path,
) {
    let manifest_path = target.join("install-manifest.json");
    let manifest = match std::fs::read_to_string(&manifest_path)
        .map_err(|error| error.to_string())
        .and_then(|body| {
            serde_json::from_str::<serde_json::Value>(&body).map_err(|error| error.to_string())
        }) {
        Ok(manifest) => manifest,
        Err(error) => {
            report.add(
                crate::setup::SetupFinding::fail(
                    "private-install-content-current",
                    "installed AGS runtime cannot be compared with the canonical setup plan",
                    format!("{}: {error}", manifest_path.display()),
                )
                .with_conformance(
                    "readable current install manifest and exact AGS-owned runtime assets",
                    "install manifest missing or invalid",
                    "Run `ags setup --yes`, then rerun `ags doctor`.",
                ),
            );
            return;
        }
    };
    let source_root = manifest
        .get("source_root")
        .and_then(serde_json::Value::as_str)
        .map(Path::new);
    let producer = manifest
        .get("producer_version")
        .and_then(serde_json::Value::as_str);
    let schema = manifest
        .get("schema_version")
        .and_then(serde_json::Value::as_str);
    let mut drift = Vec::new();
    if producer != Some(AGS_VERSION) {
        drift.push(format!(
            "producer_version={}",
            producer.unwrap_or("<missing>")
        ));
    }
    if schema != Some(super::PRIVATE_INSTALL_SCHEMA) {
        drift.push(format!("schema_version={}", schema.unwrap_or("<missing>")));
    }
    let Some(source_root) = source_root else {
        drift.push("source_root=<missing>".to_string());
        report.add(install_content_finding(drift));
        return;
    };
    if !source_root.is_dir() {
        drift.push(format!(
            "source_root={} is unavailable",
            source_root.display()
        ));
        report.add(install_content_finding(drift));
        return;
    }
    let plan = super::plan::private_install_plan(source_root, target, home);
    for file in plan
        .files
        .iter()
        .filter(|file| super::apply::codex_skill_thin_index_ancestor(&file.path).is_none())
    {
        match std::fs::read(&file.path) {
            Ok(observed) if observed == file.content.as_bytes() => {}
            Ok(_) => drift.push(format!("{}:content-drift", file.path.display())),
            Err(_) => drift.push(format!("{}:missing", file.path.display())),
        }
    }
    report.add(install_content_finding(drift));
}

fn install_content_finding(drift: Vec<String>) -> crate::setup::SetupFinding {
    if drift.is_empty() {
        return crate::setup::SetupFinding::pass(
            "private-install-content-current",
            "install manifest and AGS-owned runtime assets equal the current setup plan",
        )
        .with_conformance(
            format!(
                "{} / producer {} / exact plan content",
                super::PRIVATE_INSTALL_SCHEMA,
                AGS_VERSION
            ),
            "all AGS-owned runtime assets current",
            "none",
        );
    }
    let total = drift.len();
    let mut observed = drift.into_iter().take(8).collect::<Vec<_>>().join(", ");
    if total > 8 {
        observed.push_str(&format!(", and {} more", total - 8));
    }
    crate::setup::SetupFinding::fail(
        "private-install-content-current",
        "installed AGS runtime differs from the current setup plan",
        observed.clone(),
    )
    .with_conformance(
        format!(
            "{} / producer {} / exact AGS-owned plan content",
            super::PRIVATE_INSTALL_SCHEMA,
            AGS_VERSION
        ),
        observed,
        "Run `ags setup --yes`, then rerun `ags doctor`.",
    )
}

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
fn codex_command_skill_is_current(content: &str, name: &str) -> bool {
    content.contains(&format!("name: \"{name}\""))
        && content.contains("ags session preflight --for")
        && content.contains(AGS_VERSION)
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
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"ags_route_request\",\"arguments\":{\"request\":\"before preflight\"}}}\n"
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
    if !stdout.contains("\"protocolVersion\":\"2024-11-05\"")
        || !stdout.contains(&format!("\"version\":\"{AGS_VERSION}\""))
    {
        return Err("initialize response protocol or server version mismatch".to_string());
    }
    if !stdout.contains("\"id\":2") || !stdout.contains("AGS Initialization Gate") {
        return Err("preflight gate error response missing".to_string());
    }
    Ok(())
}
fn claude_mcp_get_at(server: &str, current_dir: &Path) -> Result<String, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "get", server])
        .current_dir(current_dir)
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
fn add_codegraph_claude_checks(report: &mut crate::setup::SetupReport, home: &Path) {
    match claude_mcp_list_line_at("codegraph", home) {
        Ok(Some(line)) if line.contains("Connected") => {
            report.add(crate::setup::SetupFinding::pass(
                "private-install-claude-code-codegraph-global",
                "Claude Code global MCP includes connected codegraph",
            ))
        }
        Ok(Some(line)) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP codegraph is configured but not connected",
            line,
        )),
        Ok(None) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP does not include codegraph",
            "run `claude mcp add -s user codegraph -- codegraph serve --mcp`",
        )),
        Err(e) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-codegraph-global",
            "cannot verify Claude Code global MCP codegraph entry",
            e,
        )),
    }

    match claude_mcp_get_at("codegraph", home) {
        Ok(detail) if detail.contains("codegraph") && detail.contains("serve --mcp") => {
            report.add(crate::setup::SetupFinding::pass(
                "private-install-claude-code-codegraph-command",
                "Claude Code codegraph MCP uses `codegraph serve --mcp`",
            ));
        }
        Ok(detail) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-codegraph-command",
            "Claude Code codegraph MCP does not use `codegraph serve --mcp`",
            detail,
        )),
        Err(e) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-codegraph-command",
            "cannot inspect Claude Code codegraph MCP command",
            e,
        )),
    }

    match command_in_path("codegraph") {
        Ok(path) => report.add(crate::setup::SetupFinding::pass(
            "private-install-codegraph-cli",
            format!("codegraph CLI available at {path}"),
        )),
        Err(e) => report.add(crate::setup::SetupFinding::fail(
            "private-install-codegraph-cli",
            "codegraph CLI is not available on PATH",
            format!("install codegraph before relying on code intelligence. {e}"),
        )),
    }
}
/// Build the installed-kernel/runtime health report without rendering or
/// exiting so `ags doctor` can use the same diagnostic authority as setup
/// verification.
pub(in crate::setup) fn private_install_health_report(
    target: &Path,
    home: &Path,
    run_mcp_smoke: bool,
) -> crate::setup::SetupReport {
    let mut report = crate::setup::SetupReport::new("private-install-verify");
    add_install_content_conformance(&mut report, target, home);

    let required = [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/codex-planner-recall.json",
        "bin/ags-mcp-stdio.sh",
    ];

    for rel in required {
        let path = target.join(rel);
        if path.exists() {
            report.add(crate::setup::SetupFinding::pass(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("present: {rel}"),
            ));
        } else {
            report.add(crate::setup::SetupFinding::fail(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("missing: {rel}"),
                path.display().to_string(),
            ));
        }
    }

    let claude_command_path = claude_ags_command_path(home);
    if claude_command_path.exists() {
        report.add(crate::setup::SetupFinding::pass(
            "private-install-claude-code-slash-command-present",
            format!("present: {}", claude_command_path.display()),
        ));
        match std::fs::read_to_string(&claude_command_path) {
            Ok(content) if content.contains("ags_preflight") && content.contains(AGS_VERSION) => {
                report.add(crate::setup::SetupFinding::pass(
                    "private-install-claude-code-slash-command-content",
                    "Claude Code /ags command references AGS preflight and current version",
                ));
            }
            Ok(_) => report.add(crate::setup::SetupFinding::fail(
                "private-install-claude-code-slash-command-content",
                "Claude Code /ags command content is stale",
                format!(
                    "expected ags_preflight and version {AGS_VERSION} in {}",
                    claude_command_path.display()
                ),
            )),
            Err(e) => report.add(crate::setup::SetupFinding::fail(
                "private-install-claude-code-slash-command-content",
                "cannot read Claude Code /ags command",
                e.to_string(),
            )),
        }
        match text_file_contains_no_secret_markers(&claude_command_path) {
            Ok(()) => report.add(crate::setup::SetupFinding::pass(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan OK: Claude Code /ags command",
            )),
            Err(e) => report.add(crate::setup::SetupFinding::fail(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan failed: Claude Code /ags command",
                e,
            )),
        }
    } else {
        report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-slash-command-present",
            "missing Claude Code /ags command",
            format!(
                "rerun `ags setup --yes` to create {}",
                claude_command_path.display()
            ),
        ));
    }

    for retired_dir in retired_codex_ags_skill_dirs(home) {
        let check_suffix = sanitize_name(&retired_dir.to_string_lossy());
        if retired_dir.exists() {
            report.add(crate::setup::SetupFinding::fail(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                "retired Codex AGS visible skill still exists",
                format!(
                    "rerun `ags setup --yes --force` to remove {}",
                    retired_dir.display()
                ),
            ));
        } else {
            report.add(crate::setup::SetupFinding::pass(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                format!(
                    "retired Codex AGS visible skill absent: {}",
                    retired_dir.display()
                ),
            ));
        }
    }

    for (name, display_name, _, _, summary) in codex_ags_command_skill_specs() {
        let skill_path = codex_ags_named_skill_path(home, name);
        let check_suffix = sanitize_name(name);
        if skill_path.exists() {
            match std::fs::read_to_string(&skill_path) {
                Ok(content) if codex_command_skill_is_current(&content, name) => {
                    report.add(crate::setup::SetupFinding::pass(
                        format!("private-install-codex-command-skill-{check_suffix}"),
                        format!("Codex command skill present: {name}"),
                    ));
                }
                Ok(_) => report.add(crate::setup::SetupFinding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("Codex command skill content is stale: {name}"),
                    format!("expected {display_name}, {summary}, and version {AGS_VERSION}"),
                )),
                Err(e) => report.add(crate::setup::SetupFinding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("cannot read Codex command skill: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(crate::setup::SetupFinding::fail(
                format!("private-install-codex-command-skill-{check_suffix}"),
                format!("missing Codex command skill: {name}"),
                skill_path.display().to_string(),
            ));
        }

        let metadata_path = codex_ags_named_skill_agent_metadata_path(home, name);
        if metadata_path.exists() {
            match std::fs::read_to_string(&metadata_path) {
                Ok(content) if content.contains(&format!("display_name: \"{display_name}\"")) => {
                    report.add(crate::setup::SetupFinding::pass(
                        format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                        format!("Codex command skill metadata present: {name}"),
                    ));
                }
                Ok(_) => report.add(crate::setup::SetupFinding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("Codex command skill metadata is stale: {name}"),
                    metadata_path.display().to_string(),
                )),
                Err(e) => report.add(crate::setup::SetupFinding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("cannot read Codex command skill metadata: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(crate::setup::SetupFinding::fail(
                format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                format!("missing Codex command skill metadata: {name}"),
                metadata_path.display().to_string(),
            ));
        }
    }

    match claude_mcp_list_line_at("ags", home) {
        Ok(Some(line)) if line.contains("Connected") => {
            report.add(crate::setup::SetupFinding::pass(
                "private-install-claude-code-ags-global",
                "Claude Code global MCP includes connected ags",
            ))
        }
        Ok(Some(line)) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP ags is configured but not connected",
            line,
        )),
        Ok(None) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP does not include ags",
            "run `/ags setup` or `ags setup --yes --register-claude`",
        )),
        Err(e) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-global",
            "cannot verify Claude Code global MCP ags entry",
            e,
        )),
    }

    match (claude_mcp_get_at("ags", home), command_in_path("ags")) {
        (Ok(detail), Ok(ags_path)) if detail.contains(&ags_path) => {
            report.add(crate::setup::SetupFinding::pass(
                "private-install-claude-code-ags-command",
                "Claude Code ags MCP uses installed AGS binary",
            ));
        }
        (Ok(detail), Ok(ags_path)) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-command",
            "Claude Code ags MCP does not use the installed AGS binary",
            format!("expected command: {ags_path}\n{detail}"),
        )),
        (Ok(detail), Err(e)) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-command",
            "cannot confirm installed AGS binary path",
            format!("{e}\n{detail}"),
        )),
        (Err(e), _) => report.add(crate::setup::SetupFinding::fail(
            "private-install-claude-code-ags-command",
            "cannot inspect Claude Code ags MCP command",
            e,
        )),
    }

    add_codegraph_claude_checks(&mut report, home);

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
                Ok(()) => report.add(crate::setup::SetupFinding::pass(
                    format!("private-install-json-{}", sanitize_name(rel)),
                    format!("valid JSON: {rel}"),
                )),
                Err(e) => report.add(crate::setup::SetupFinding::fail(
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
        "hooks/codex-planner-recall.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match text_file_contains_no_secret_markers(&path) {
                Ok(()) => report.add(crate::setup::SetupFinding::pass(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan OK: {rel}"),
                )),
                Err(e) => report.add(crate::setup::SetupFinding::fail(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan failed: {rel}"),
                    e,
                )),
            }
        }
    }

    if run_mcp_smoke {
        match mcp_smoke_current_exe() {
            Ok(()) => report.add(crate::setup::SetupFinding::pass(
                "private-install-mcp-smoke",
                "ags mcp serve stdio smoke OK",
            )),
            Err(e) => report.add(crate::setup::SetupFinding::fail(
                "private-install-mcp-smoke",
                "ags mcp serve stdio smoke failed",
                e,
            )),
        }
    } else {
        report.add(crate::setup::SetupFinding::skip(
            "private-install-mcp-smoke",
            "live MCP smoke is excluded from read-only Doctor; daemon health is inspected without starting or restarting it",
        ));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::super::retired_codex_ags_skill_dirs;
    use super::super::templates::codex_ags_command_skill_specs;
    use super::*;

    fn spec_names() -> Vec<&'static str> {
        codex_ags_command_skill_specs()
            .iter()
            .map(|(name, _, _, _, _)| *name)
            .collect()
    }

    /// The standard Codex front-stage AGS command skills that setup writes and
    /// verify checks are EXACTLY setup / agents / skill / init / doctor.
    /// `ags-capability` is not among them — it is the underlying cross-Agent
    /// `ags capability` CLI, retired from the front-stage command-skill set.
    #[test]
    fn verified_codex_command_skills_are_the_canonical_five() {
        assert_eq!(
            spec_names(),
            vec![
                "ags-setup",
                "ags-agents",
                "ags-skill",
                "ags-init",
                "ags-doctor"
            ]
        );
        assert!(!spec_names().contains(&"ags-capability"));
    }

    #[test]
    fn install_content_conformance_compares_current_plan_not_presence() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime");
        let home = root.path().join("home");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let plan = super::super::plan::private_install_plan(&source, &target, &home);
        for file in plan.files.iter().filter(|file| {
            super::super::apply::codex_skill_thin_index_ancestor(&file.path).is_none()
        }) {
            std::fs::create_dir_all(file.path.parent().unwrap()).unwrap();
            std::fs::write(&file.path, &file.content).unwrap();
        }
        let mut current = crate::setup::SetupReport::new("current");
        add_install_content_conformance(&mut current, &target, &home);
        assert!(current.passed(), "{:?}", current.findings);

        std::fs::write(target.join("mcp/ags.mcp.json"), "{}\n").unwrap();
        let mut drifted = crate::setup::SetupReport::new("drifted");
        add_install_content_conformance(&mut drifted, &target, &home);
        assert_eq!(drifted.total_failed_checks(), 1);
        assert!(drifted.findings[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("mcp/ags.mcp.json:content-drift"));
    }

    /// `ags-capability` is on the retired-Codex-skill list, so the verify gate
    /// reports a stale `~/.codex/skills/ags-capability` entry and setup cleans it.
    /// The active command-skill set and the retired set must stay disjoint.
    #[test]
    fn ags_capability_is_a_retired_codex_skill() {
        let home = std::env::temp_dir().join("ags-verify-retired-codex-skill-home");
        assert!(
            retired_codex_ags_skill_dirs(&home)
                .iter()
                .any(|dir| dir.ends_with("ags-capability")),
            "ags-capability must be retired so setup/verify de-expose the stale Codex entry"
        );
        let active = spec_names();
        for dir in retired_codex_ags_skill_dirs(&home) {
            if let Some(last) = dir.file_name().and_then(|s| s.to_str()) {
                assert!(
                    !active.contains(&last),
                    "retired skill {last} must not also be an active command skill"
                );
            }
        }
    }
}
