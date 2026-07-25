use super::instruction_projection::*;
use super::protocol_audit::*;
use super::rendering::*;
use super::session_preflight::*;
use super::workspace_facts::*;
use ags_host_integration::*;
use std::path::{Path, PathBuf};

const TEST_CLAUDE_MEMORY_SCRIPTS: &[&str] = &[
    "context-memory.sh",
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
    "raw-tool-call-stop-guard.js",
];

// ── Project memory lifecycle closure ────────────────────────────────

fn ml_tmp(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("ags-pd-ml-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn ml_write_profile(target: &Path, slug: &str) {
    std::fs::create_dir_all(target.join("config")).unwrap();
    std::fs::write(
        target.join("config/agent-project-profile.yaml"),
        format!("schema_version: 1\nproject:\n  slug: {slug}\n"),
    )
    .unwrap();
}

fn ml_write_memory_files(home: &Path, slug: &str, archive: bool) {
    let dir = home.join(".agents/memory/projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("context-capsule.md"), "## 项目设计目的\nx\n").unwrap();
    std::fs::write(dir.join("task-memory.md"), "# Task Memory\n").unwrap();
    if archive {
        std::fs::create_dir_all(dir.join("task-archive")).unwrap();
    }
}

fn ml_write_scripts(home: &Path) {
    let dir = home.join(".agents/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    for n in TEST_CLAUDE_MEMORY_SCRIPTS {
        std::fs::write(dir.join(n), "x").unwrap();
    }
}

fn ml_write_settings(target: &Path, start: bool, stop: bool) {
    let claude = target.join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let mut hooks = serde_json::Map::new();
    if start {
        hooks.insert(
                "SessionStart".into(),
                serde_json::json!([
                    { "hooks": [ { "type": "command", "command": "python3 $HOME/.agents/scripts/context-memory-start.py" } ] }
                ]),
            );
    }
    if stop {
        hooks.insert(
                "Stop".into(),
                serde_json::json!([
                    { "hooks": [
                        { "type": "command", "command": "node $HOME/.agents/scripts/raw-tool-call-stop-guard.js" },
                        { "type": "command", "command": "python3 $HOME/.agents/scripts/claude-stop-memory-capture.py" }
                    ] }
                ]),
            );
    }
    let v = serde_json::Value::Object(serde_json::Map::from_iter([(
        "hooks".to_string(),
        serde_json::Value::Object(hooks),
    )]));
    std::fs::write(
        claude.join("settings.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();
}

#[test]
fn memory_lifecycle_full_when_files_hooks_and_scripts_present() {
    let base = ml_tmp("full");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    ml_write_settings(&target, true, true);

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_eq!(ml.status, "full");
    assert!(
        ml.files_present
            && ml.read_wired
            && ml.write_wired
            && ml.scripts_present
            && ml.archive_ready
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_is_host_specific_and_never_reuses_claude_hooks_for_codex() {
    let base = ml_tmp("host-specific");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    ml_write_settings(&target, true, true);

    let claude = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::ClaudeCode);
    let codex = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::Codex);

    assert_eq!(claude.status, "full");
    assert_ne!(
        codex.status, "full",
        "Codex must not inherit Claude Code hook evidence"
    );
    assert_eq!(codex.host, "codex");
    assert!(!codex.read_wired && !codex.write_wired);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_full_for_codex_native_start_and_end_hooks() {
    let base = ml_tmp("codex-full");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    std::fs::write(
            home.join(".codex/hooks.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"python3 ~/.agents/scripts/context-memory-start.py"}]}],"SessionEnd":[{"hooks":[{"command":"python3 ~/.agents/scripts/claude-stop-memory-capture.py"}]}]}}"#,
        )
        .unwrap();

    let lifecycle = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::Codex);
    assert_eq!(lifecycle.status, "full");
    assert_eq!(lifecycle.adapter, "codex-command-hooks");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_full_for_claude_global_hooks() {
    let base = ml_tmp("claude-global-full");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"python3 ~/.agents/scripts/context-memory-start.py"}]}],"Stop":[{"hooks":[{"command":"python3 ~/.agents/scripts/claude-stop-memory-capture.py"}]}]}}"#,
        )
        .unwrap();

    let lifecycle = compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::ClaudeCode);
    assert_eq!(lifecycle.status, "full");
    assert_eq!(lifecycle.adapter, "claude-command-hooks");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_full_for_omp_native_extension() {
    let base = ml_tmp("omp-full");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    let extension = home.join(".omp/agent/extensions/ags-memory-lifecycle.js");
    std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
    std::fs::write(
        &extension,
        r#"// context-memory-start.py claude-stop-memory-capture.py
export default function (pi) {
  pi.on("session_start", async () => {});
  pi.on("before_agent_start", async () => ({ systemPromptAppend: "memory" }));
  pi.on("agent_settled", async () => {});
  pi.on("session_shutdown", async () => {});
}
"#,
    )
    .unwrap();

    let lifecycle =
        compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::from_str("omp").unwrap());
    assert_eq!(lifecycle.status, "full");
    assert_eq!(lifecycle.adapter, "omp-extension");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_rejects_unrelated_omp_extension_file() {
    let base = ml_tmp("omp-unrelated");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    let extension = home.join(".omp/agent/extensions/ags-memory-lifecycle.js");
    std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
    std::fs::write(&extension, "export default function () {}\n").unwrap();

    let lifecycle =
        compute_memory_lifecycle_at_for_host(&target, &home, &AgentType::from_str("omp").unwrap());
    assert_eq!(lifecycle.status, "files-only");
    assert!(!lifecycle.read_wired && !lifecycle.write_wired);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_files_only_when_no_hooks() {
    let base = ml_tmp("filesonly");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    // no settings.json — files exist but nothing is wired

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_eq!(ml.status, "files-only");
    assert!(ml.files_present);
    assert!(!ml.read_wired && !ml.write_wired);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_unbacked_when_hooks_wired_but_scripts_missing() {
    let base = ml_tmp("unbacked");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    // scripts NOT installed → hooks would shell out to nothing
    ml_write_settings(&target, true, true);

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_eq!(ml.status, "unbacked");
    assert!(!ml.scripts_present);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_read_only_when_only_start_wired() {
    let base = ml_tmp("readonly");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", true);
    ml_write_scripts(&home);
    ml_write_settings(&target, true, false);

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_eq!(ml.status, "read-only");
    assert!(ml.read_wired && !ml.write_wired);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_not_full_without_archive() {
    let base = ml_tmp("noarchive");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    ml_write_memory_files(&home, "demo-proj", false);
    ml_write_scripts(&home);
    ml_write_settings(&target, true, true);

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_ne!(ml.status, "full", "must not be full without task-archive/");
    assert!(!ml.archive_ready);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn memory_lifecycle_absent_when_nothing_present() {
    let base = ml_tmp("absent");
    let home = base.join("home");
    let target = base.join("proj");
    std::fs::create_dir_all(&target).unwrap();
    ml_write_profile(&target, "demo-proj");
    // no memory files, no scripts, no settings

    let ml = compute_memory_lifecycle_at(&target, &home);
    assert_eq!(ml.status, "absent");
    assert!(!ml.files_present && !ml.read_wired && !ml.write_wired);
    let _ = std::fs::remove_dir_all(&base);
}

/// Return the repo root path (two levels up from the crate directory).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

// ── Workspace table parsing ────────────────────────────────────────

#[test]
fn test_parse_workspace_table_standard() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Development private suite | /Volumes/Projects/example-private-suite |
| A1 | Private bare repo | /Volumes/Projects/remotes/example-private-suite.git |
| S | Stable private suite | /Volumes/Projects/example-stable-suite |
| B | Public worktree | /Volumes/AI Project/ai-dev-env-bootstrap |
| B1 | Public bare repo | /Volumes/Projects/remotes/example-public-suite.git |
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 5);
    assert_eq!(identities[0].code, "A");
    assert_eq!(identities[0].role, "Development private suite");
    assert_eq!(
        identities[0].path,
        "/Volumes/Projects/example-private-suite"
    );
    assert_eq!(identities[4].code, "B1");
}

#[test]
fn test_parse_workspace_table_blank_line_terminates() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /path/to/a |

Some other text here.
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].code, "A");
}

#[test]
fn test_parse_workspace_table_non_table_line_terminates() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /path/to/a |
## Next Section
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 1);
}

#[test]
fn test_parse_workspace_table_empty() {
    let identities = parse_workspace_table("No table here.");
    assert_eq!(identities.len(), 0);
}

#[test]
fn test_parse_workspace_table_no_header_sep() {
    let content = "\
| Code | Role | Path |
| A | Dev | /path |
";
    let identities = parse_workspace_table(content);
    // No header separator, so no rows parsed
    assert_eq!(identities.len(), 0);
}

// ── AgentType parsing ──────────────────────────────────────────────

#[test]
fn test_agent_type_from_str_valid() {
    assert_eq!(AgentType::from_str("codex").unwrap(), AgentType::Codex);
    assert_eq!(
        AgentType::from_str("claude-code").unwrap(),
        AgentType::ClaudeCode
    );
    assert_eq!(AgentType::from_str("cursor").unwrap(), AgentType::Cursor);
    assert_eq!(
        AgentType::from_str("workbuddy").unwrap(),
        AgentType::Generic("workbuddy".to_string())
    );
    // Tencent Agent host clients normalize to recognized generic ids
    // (no new canonical AgentType variant; casing/spacing folded by
    // normalize_agent_id).
    assert_eq!(
        AgentType::from_str("WorkBuddy").unwrap(),
        AgentType::Generic("workbuddy".to_string())
    );
    assert_eq!(
        AgentType::from_str("CodeBuddy-Code").unwrap(),
        AgentType::Generic("codebuddy-code".to_string())
    );
    assert_eq!(
        AgentType::from_str("Tencent Agent").unwrap(),
        AgentType::Generic("tencent-agent".to_string())
    );
    assert_eq!(
        AgentType::from_str("OMP").unwrap(),
        AgentType::Generic("omp".to_string())
    );
    assert_eq!(
        AgentType::from_str("Oh My Pi").unwrap(),
        AgentType::Generic("oh-my-pi".to_string())
    );
    assert_eq!(
        AgentType::from_str("Claude Desktop Cowork").unwrap(),
        AgentType::Generic("claude-desktop-cowork".to_string())
    );
}

#[test]
fn test_agent_type_from_str_invalid() {
    assert!(AgentType::from_str("").is_err());
    assert!(AgentType::from_str("   ").is_err());
}

#[test]
fn test_agent_type_display_name() {
    assert_eq!(AgentType::Codex.display_name(), "Codex");
    assert_eq!(AgentType::ClaudeCode.display_name(), "Claude Code");
    assert_eq!(AgentType::Cursor.display_name(), "Cursor");
    // Tencent Agent host family gets branded display names while still
    // carried as Generic (no new AgentType variant).
    assert_eq!(
        AgentType::Generic("workbuddy".to_string()).display_name(),
        "Tencent Agent (WorkBuddy)"
    );
    assert_eq!(
        AgentType::Generic("codebuddy-code".to_string()).display_name(),
        "Tencent Agent (CodeBuddy-Code)"
    );
    assert_eq!(
        AgentType::Generic("tencent-agent".to_string()).display_name(),
        "Tencent Agent"
    );
    assert_eq!(
        AgentType::Generic("omp".to_string()).display_name(),
        "Oh My Pi (OMP)"
    );
    // Unknown hosts keep the plain generic fallback — not broken.
    assert_eq!(
        AgentType::Generic("claude-desktop-cowork".to_string()).display_name(),
        "Generic Agent (claude-desktop-cowork)"
    );
}

#[test]
fn test_agent_type_as_str() {
    assert_eq!(AgentType::Codex.as_str(), "codex");
    assert_eq!(AgentType::ClaudeCode.as_str(), "claude-code");
    assert_eq!(AgentType::Cursor.as_str(), "cursor");
    assert_eq!(
        AgentType::Generic("workbuddy".to_string()).as_str(),
        "workbuddy"
    );
}

// ── Agent type serde ───────────────────────────────────────────────

#[test]
fn test_agent_type_serialize() {
    assert_eq!(
        serde_json::to_string(&AgentType::Codex).unwrap(),
        "\"codex\""
    );
    assert_eq!(
        serde_json::to_string(&AgentType::ClaudeCode).unwrap(),
        "\"claude-code\""
    );
    assert_eq!(
        serde_json::to_string(&AgentType::Cursor).unwrap(),
        "\"cursor\""
    );
    assert_eq!(
        serde_json::to_string(&AgentType::Generic("workbuddy".to_string())).unwrap(),
        "\"workbuddy\""
    );
}

#[test]
fn test_agent_type_deserialize() {
    let a: AgentType = serde_json::from_str("\"codex\"").unwrap();
    assert_eq!(a, AgentType::Codex);
    let a: AgentType = serde_json::from_str("\"claude-code\"").unwrap();
    assert_eq!(a, AgentType::ClaudeCode);
    let a: AgentType = serde_json::from_str("\"cursor\"").unwrap();
    assert_eq!(a, AgentType::Cursor);
    let a: AgentType = serde_json::from_str("\"workbuddy\"").unwrap();
    assert_eq!(a, AgentType::Generic("workbuddy".to_string()));
}

// ── IntegrationStatus serde ────────────────────────────────────────

#[test]
fn test_integration_status_serde() {
    assert_eq!(
        serde_json::to_string(&IntegrationStatus::Suite).unwrap(),
        "\"suite\""
    );
    assert_eq!(
        serde_json::to_string(&IntegrationStatus::Integrated).unwrap(),
        "\"integrated\""
    );
    assert_eq!(
        serde_json::to_string(&IntegrationStatus::NotIntegrated).unwrap(),
        "\"not_integrated\""
    );
    assert_eq!(
        serde_json::to_string(&IntegrationStatus::Partial).unwrap(),
        "\"partial\""
    );
}

// ── Project detection against the real AGS repo ────────────────────

#[test]
fn test_detect_ags_suite_repo() {
    let root = repo_root();
    let identity = detect_project(&root);
    assert!(
        identity.is_ags_suite,
        "Running from AGS repo — should detect as suite"
    );
    assert_eq!(identity.integration_status, IntegrationStatus::Suite);
    assert!(!identity.workspace_identities.is_empty());
    // Should have found WORKSPACE.md with at least A, A1, S, B, B1
    assert!(identity.workspace_identities.len() >= 5);
    // Should have found root entry files
    assert!(identity
        .root_entry_files_found
        .contains(&"AGENTS.md".to_string()));
    assert!(identity
        .root_entry_files_found
        .contains(&"CLAUDE.md".to_string()));
    // The development private suite has local memory; the stable suite may
    // not, but both are valid AGS suite roots.
    if matches!(
        identity
            .inferred_role
            .as_ref()
            .map(|role| role.code.as_str()),
        Some("A")
    ) {
        assert!(identity.memory_capsule_path.is_some());
    }
}

#[test]
fn test_detect_temp_dir_not_integrated() {
    let tmp = std::env::temp_dir().join("ags-test-not-integrated");
    let _ = std::fs::create_dir_all(&tmp);
    let identity = detect_project(&tmp);
    assert!(!identity.is_ags_suite);
    assert!(!identity.is_ags_integrated);
    assert_eq!(
        identity.integration_status,
        IntegrationStatus::NotIntegrated
    );
    assert!(!identity.gaps.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_detect_nonexistent_target() {
    let identity = detect_project(Path::new("/tmp/ags-nonexistent-XXXXXX"));
    assert!(!identity.is_ags_suite);
    assert!(!identity.is_ags_integrated);
    assert_eq!(
        identity.integration_status,
        IntegrationStatus::NotIntegrated
    );
}

#[test]
fn test_detect_project_json_output() {
    let identity = detect_project(&repo_root());
    let json = render_json(&identity);
    assert!(json.contains("\"target\""));
    assert!(json.contains("\"integration_status\""));
    assert!(json.contains("\"is_ags_suite\""));
    // Verify parseable
    let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
}

// ── Protocol status tests ──────────────────────────────────────────

#[test]
fn test_protocol_status_ags_repo() {
    let root = repo_root();
    let status = check_protocol_status(&root);
    // Running from AGS repo — most protocol files should be present
    assert!(status.present_count > 5);
    assert!(status.protocol_dir_exists);
    assert!(status.task_card_validator.available);
    // Should have no critical failures in our own repo
    let critical_failures: Vec<_> = status
        .failures
        .iter()
        .filter(|f| f.starts_with("CRITICAL:"))
        .collect();
    assert!(
        critical_failures.is_empty(),
        "AGS repo should have no critical failures: {:?}",
        critical_failures
    );
}

#[test]
fn test_protocol_status_temp_dir() {
    let tmp = std::env::temp_dir().join("ags-test-protocol-status");
    let _ = std::fs::create_dir_all(&tmp);
    let status = check_protocol_status(&tmp);
    assert!(!status.protocol_dir_exists);
    assert!(status.present_count == 0);
    assert!(!status.task_card_validator.available);
    assert!(!status.failures.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_protocol_status_json_output() {
    let status = check_protocol_status(&repo_root());
    let json = render_json(&status);
    assert!(json.contains("\"target\""));
    assert!(json.contains("\"files\""));
    assert!(json.contains("\"present_count\""));
    assert!(json.contains("\"task_card_validator\""));
    // Verify parseable
    let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
}

#[test]
fn test_protocol_status_exit_code_clean() {
    let status = check_protocol_status(&repo_root());
    let code = protocol_status_exit_code(&status);
    // Running from AGS repo — should be clean (0)
    assert_eq!(code, 0, "AGS repo should have exit code 0");
}

#[test]
fn test_protocol_status_exit_code_dirty() {
    let tmp = std::env::temp_dir().join("ags-test-exit-code");
    let _ = std::fs::create_dir_all(&tmp);
    let status = check_protocol_status(&tmp);
    let code = protocol_status_exit_code(&status);
    assert_eq!(code, 1, "Non-AGS repo should have exit code 1");
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Project detect exit code ───────────────────────────────────────

#[test]
fn test_project_detect_exit_code_suite() {
    let identity = detect_project(&repo_root());
    let code = project_detect_exit_code(&identity);
    assert_eq!(code, 0, "Suite repo should have exit code 0");
}

#[test]
fn test_project_detect_exit_code_not_integrated() {
    let tmp = std::env::temp_dir().join("ags-test-exit-code-2");
    let _ = std::fs::create_dir_all(&tmp);
    let identity = detect_project(&tmp);
    let code = project_detect_exit_code(&identity);
    assert_eq!(code, 1, "Non-integrated repo should have exit code 1");
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Agent instructions tests ───────────────────────────────────────

#[test]
fn test_generate_instructions_codex() {
    let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
    assert_eq!(instructions.agent_type, "codex");
    assert_eq!(instructions.agent_display_name, "Codex");
    assert_eq!(
        instructions.permissions.default_permission_mode,
        "execute-and-verify"
    );
    assert!(instructions.is_ags_suite);
    assert!(!instructions.required_reads.is_empty());
    assert!(instructions.instructions_text.contains("## Required Reads"));
    assert!(instructions
        .instructions_text
        .contains("## Stop Conditions"));
    assert!(instructions
        .instructions_text
        .contains("## Verification Commands"));
    assert!(instructions
        .instructions_text
        .contains("same-session modification instruction"));
    assert!(instructions.instructions_text.contains("task-card/handoff"));
}

#[test]
fn test_generate_instructions_claude_code() {
    let root = repo_root();
    let instructions = generate_agent_instructions(&root, &AgentType::ClaudeCode);
    assert_eq!(instructions.agent_type, "claude-code");
    assert_eq!(instructions.agent_display_name, "Claude Code");
    assert_eq!(
        instructions.permissions.default_permission_mode,
        "execute-and-verify"
    );
    assert!(!instructions.required_reads.is_empty());
    assert!(instructions
        .instructions_text
        .contains("Claude Code executes bounded handoff task cards"));
    assert!(instructions
        .instructions_text
        .contains("must not infer same-session direct-edit authority"));
}

#[test]
fn test_generate_instructions_cursor() {
    let instructions = generate_agent_instructions(&repo_root(), &AgentType::Cursor);
    assert_eq!(instructions.agent_type, "cursor");
    assert_eq!(instructions.agent_display_name, "Cursor");
    assert_eq!(
        instructions.permissions.default_permission_mode,
        "execute-and-verify"
    );
    assert!(!instructions.required_reads.is_empty());
    assert!(instructions
        .instructions_text
        .contains("same-session modification authorization"));
    assert!(instructions.instructions_text.contains("ags_route_request"));
    assert!(instructions.instructions_text.contains("HostRouteProposal"));
    assert!(instructions.instructions_text.contains("ags_apply_action"));
    assert!(instructions
        .instructions_text
        .contains("MachineCli is only consumed"));
    assert!(instructions
        .instructions_text
        .contains("confirmed handoff contract"));
}

#[test]
fn test_generate_instructions_generic_agent() {
    let instructions =
        generate_agent_instructions(&repo_root(), &AgentType::Generic("workbuddy".to_string()));
    assert_eq!(instructions.agent_type, "workbuddy");
    assert_eq!(instructions.agent_display_name, "Tencent Agent (WorkBuddy)");
    assert_eq!(
        instructions.permissions.default_permission_mode,
        "execute-and-verify"
    );
    assert!(instructions
        .instructions_text
        .contains("AGS-compatible governed host"));
}

#[test]
fn test_generate_instructions_codebuddy_code() {
    let instructions = generate_agent_instructions(
        &repo_root(),
        &AgentType::Generic("codebuddy-code".to_string()),
    );
    assert_eq!(instructions.agent_type, "codebuddy-code");
    assert_eq!(
        instructions.agent_display_name,
        "Tencent Agent (CodeBuddy-Code)"
    );
    // Recognized Tencent Agent clients keep the governed-host permission
    // profile (no elevated privileges from the name).
    assert_eq!(
        instructions.permissions.default_permission_mode,
        "execute-and-verify"
    );
}

#[test]
fn test_agent_instructions_json_output() {
    let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
    let json = render_json(&instructions);
    assert!(json.contains("\"agent_type\""));
    assert!(json.contains("\"required_reads\""));
    assert!(json.contains("\"stop_conditions\""));
    // Verify parseable
    let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
}

#[test]
fn test_agent_instructions_non_ags_repo() {
    let tmp = std::env::temp_dir().join("ags-test-agent-instructions");
    let _ = std::fs::create_dir_all(&tmp);
    let instructions = generate_agent_instructions(&tmp, &AgentType::ClaudeCode);
    assert!(!instructions.is_ags_suite);
    assert_eq!(
        instructions.integration_status,
        IntegrationStatus::NotIntegrated
    );
    // Should still generate valid instructions
    assert!(!instructions.instructions_text.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Text renderers ─────────────────────────────────────────────────

#[test]
fn test_render_project_identity_text() {
    let identity = detect_project(&repo_root());
    let text = render_project_identity_text(&identity);
    assert!(text.contains("Project Identity"));
    assert!(text.contains("AGS Suite:"));
    assert!(text.contains("Workspace Identities:"));
}

#[test]
fn test_render_protocol_status_text() {
    let status = check_protocol_status(&repo_root());
    let text = render_protocol_status_text(&status);
    assert!(text.contains("Protocol Status"));
    assert!(text.contains("Task-Card Validator:"));
    assert!(text.contains("Risk Boundaries:"));
    assert!(text.contains("Review Requirements:"));
}

#[test]
fn test_render_agent_instructions_text() {
    let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
    let text = render_agent_instructions_text(&instructions);
    assert!(text.contains("# Agent Governance Instructions"));
    assert!(text.contains("## Role"));
    assert!(text.contains("## Required Reads"));
}

// ── Slug derivation ────────────────────────────────────────────────

#[test]
fn test_slug_from_path() {
    assert_eq!(
        slug_from_path(Path::new("/foo/bar/my-project")),
        "my-project"
    );
    assert_eq!(slug_from_path(Path::new("/foo/bar")), "bar");
}

#[test]
fn extract_profile_slug_strips_inline_comments() {
    let base = ml_tmp("slug-comment");
    let target = base.join("proj");
    std::fs::create_dir_all(target.join("config")).unwrap();
    std::fs::write(
        target.join("config/agent-project-profile.yaml"),
        "schema_version: 1\nproject:\n  slug: \"demo\" # this is a comment\n",
    )
    .unwrap();
    assert_eq!(extract_profile_slug(&target).unwrap(), "demo");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn extract_profile_slug_ignores_non_project_slug() {
    let base = ml_tmp("slug-scope");
    let target = base.join("proj");
    std::fs::create_dir_all(target.join("config")).unwrap();
    std::fs::write(
        target.join("config/agent-project-profile.yaml"),
        "schema_version: 1\nslug: top-level-ignored\nproject:\n  slug: real-slug\n",
    )
    .unwrap();
    assert_eq!(extract_profile_slug(&target).unwrap(), "real-slug");
    let _ = std::fs::remove_dir_all(&base);
}

// ── Adversarial: backtick stripping in WORKSPACE.md paths ──────────

#[test]
fn test_workspace_table_strips_backticks_from_paths() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Dev suite | `/Volumes/Projects/example-private-suite` |
| S | Stable | `/Volumes/Projects/example-stable-suite` |
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 2);
    assert_eq!(
        identities[0].path,
        "/Volumes/Projects/example-private-suite"
    );
    assert_eq!(identities[1].path, "/Volumes/Projects/example-stable-suite");
    // Verify no backticks remain
    assert!(!identities[0].path.contains('`'));
    assert!(!identities[1].path.contains('`'));
}

#[test]
fn test_workspace_table_strips_backticks_with_whitespace() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | `  /path/with spaces  ` |
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].path, "/path/with spaces");
    assert!(!identities[0].path.contains('`'));
}

#[test]
fn test_workspace_table_path_without_backticks_unchanged() {
    let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /plain/path |
";
    let identities = parse_workspace_table(content);
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].path, "/plain/path");
}

// ── Adversarial: is_ags_integrated consistency ─────────────────────

#[test]
fn test_is_ags_integrated_consistent_with_status() {
    let tmp = std::env::temp_dir().join("ags-test-integrated-consistency");
    let _ = std::fs::create_dir_all(&tmp);

    // Empty repo: not integrated
    let identity = detect_project(&tmp);
    assert_eq!(
        identity.integration_status,
        IntegrationStatus::NotIntegrated
    );
    assert!(!identity.is_ags_integrated);
    assert!(!identity.is_ags_suite);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_suite_has_is_ags_integrated_true() {
    let root = repo_root();
    let identity = detect_project(&root);
    assert_eq!(identity.integration_status, IntegrationStatus::Suite);
    assert!(identity.is_ags_suite);
    assert!(
        identity.is_ags_integrated,
        "Suite must have is_ags_integrated=true"
    );
}

// ── Adversarial: agent instructions for non-integrated repos ───────

#[test]
fn test_agent_instructions_non_integrated_should_stop() {
    let tmp = std::env::temp_dir().join("ags-test-agent-stop");
    let _ = std::fs::create_dir_all(&tmp);

    let instructions = generate_agent_instructions(&tmp, &AgentType::ClaudeCode);
    assert!(
        instructions.should_stop,
        "Non-integrated repo must set should_stop=true"
    );
    assert!(!instructions.stop_reasons.is_empty());
    assert!(instructions.exit_code != 0);
    // Must contain STOP banner in text
    assert!(instructions.instructions_text.contains("⛔ STOP"));
    assert!(instructions.instructions_text.contains("DO NOT EXECUTE"));
    // Must contain gaps
    assert!(!instructions.integration_gaps.is_empty());
    // Must contain protocol failures
    assert!(!instructions.protocol_failures.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_agent_instructions_suite_does_not_stop() {
    let root = repo_root();
    let instructions = generate_agent_instructions(&root, &AgentType::ClaudeCode);
    assert!(
        !instructions.should_stop,
        "Suite repo must have should_stop=false"
    );
    assert!(instructions.stop_reasons.is_empty());
    assert_eq!(instructions.exit_code, 0);
    // Must NOT contain STOP banner
    assert!(!instructions.instructions_text.contains("⛔ STOP"));
}

#[test]
fn test_agent_instructions_non_integrated_json_has_stop_fields() {
    let tmp = std::env::temp_dir().join("ags-test-agent-json-stop");
    let _ = std::fs::create_dir_all(&tmp);

    let instructions = generate_agent_instructions(&tmp, &AgentType::Codex);
    let json = render_json(&instructions);
    assert!(json.contains("\"should_stop\": true"));
    assert!(json.contains("\"exit_code\": 1"));
    assert!(json.contains("\"stop_reasons\""));
    assert!(json.contains("\"integration_gaps\""));
    assert!(json.contains("\"protocol_failures\""));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Adversarial: target-aware verification commands ─────────────────

#[test]
fn test_verification_commands_rust_project() {
    let tmp = std::env::temp_dir().join("ags-test-verify-rust");
    let _ = std::fs::create_dir_all(&tmp);
    // Create a fake Cargo.toml
    std::fs::write(tmp.join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();

    let commands = detect_verification_commands(&tmp);
    assert!(commands.iter().any(|c| c.contains("cargo fmt")));
    assert!(commands.iter().any(|c| c.contains("cargo test")));
    assert!(commands.iter().any(|c| c.contains("cargo build")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_verification_commands_with_verify_sh() {
    let tmp = std::env::temp_dir().join("ags-test-verify-sh");
    let _ = std::fs::create_dir_all(&tmp);
    let scripts_dir = tmp.join("scripts");
    let _ = std::fs::create_dir_all(&scripts_dir);
    std::fs::write(scripts_dir.join("verify.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let commands = detect_verification_commands(&tmp);
    assert!(commands.iter().any(|c| c.contains("verify.sh")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_verification_commands_empty_project() {
    let tmp = std::env::temp_dir().join("ags-test-verify-empty");
    let _ = std::fs::create_dir_all(&tmp);

    let commands = detect_verification_commands(&tmp);
    // Should return guidance, not false commands
    assert!(!commands.is_empty());
    assert!(!commands.iter().any(|c| c.contains("cargo")));
    assert!(commands.iter().any(|c| c.contains("No project-specific")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_protocol_status_uses_target_aware_verify_commands() {
    let root = repo_root();
    let status = check_protocol_status(&root);
    // AGS repo has Cargo.toml and scripts/verify.sh
    assert!(status
        .verify_requirements
        .iter()
        .any(|c| c.contains("cargo fmt")));
    assert!(status
        .verify_requirements
        .iter()
        .any(|c| c.contains("verify.sh")));
}

#[test]
fn test_protocol_status_empty_repo_verify_commands_guidance() {
    let tmp = std::env::temp_dir().join("ags-test-protocol-verify");
    let _ = std::fs::create_dir_all(&tmp);

    let status = check_protocol_status(&tmp);
    // Empty repo — should give guidance, not false cargo commands
    assert!(!status
        .verify_requirements
        .iter()
        .any(|c| c.contains("cargo")));
    assert!(status
        .verify_requirements
        .iter()
        .any(|c| c.contains("No project-specific")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_agent_instructions_target_aware_verify_commands() {
    let root = repo_root();
    let instructions = generate_agent_instructions(&root, &AgentType::Codex);
    assert!(instructions
        .verification_commands
        .iter()
        .any(|c| c.contains("cargo fmt")));
    assert!(instructions
        .verification_commands
        .iter()
        .any(|c| c.contains("verify.sh")));
}

// ── Known workspace path detection ─────────────────────────────────

#[test]
fn test_known_workspace_path_detection() {
    for (code, role, path) in KNOWN_WORKSPACE_PATHS {
        assert_eq!(
            known_workspace_identity(Path::new(path)),
            Some(WorkspaceIdentity {
                code: (*code).to_string(),
                role: (*role).to_string(),
                path: (*path).to_string(),
            })
        );
    }
}

// ── Session Preflight tests ───────────────────────────────────────

#[test]
fn test_session_preflight_host_matrix() {
    let root = repo_root();
    let cases = [
        (AgentType::Codex, "codex", "Codex"),
        (AgentType::ClaudeCode, "claude-code", "Claude Code"),
        (AgentType::Cursor, "cursor", "Cursor"),
        (
            AgentType::from_str("workbuddy").unwrap(),
            "workbuddy",
            "Tencent Agent (WorkBuddy)",
        ),
        (
            AgentType::from_str("CodeBuddy-Code").unwrap(),
            "codebuddy-code",
            "Tencent Agent (CodeBuddy-Code)",
        ),
        (
            AgentType::from_str("Tencent Agent").unwrap(),
            "tencent-agent",
            "Tencent Agent",
        ),
    ];

    for (agent, canonical, display_name) in cases {
        let preflight = run_session_preflight(&root, &agent);
        assert_eq!(preflight.for_agent, canonical);
        assert_eq!(preflight.agent_display_name, display_name);
        assert!(preflight.is_ags_suite);
        assert_eq!(preflight.integration_status, IntegrationStatus::Suite);
        assert!(preflight.validator_available);
        assert!(!preflight.stop_conditions.is_empty());
        assert!(!preflight.verification_commands.is_empty());
        assert_eq!(preflight.default_permission_mode, "execute-and-verify");
        assert_ne!(preflight.overall_status, PreflightStatus::Stop);
        assert_eq!(preflight.exit_code, 0);
    }
}

#[test]
fn test_session_preflight_non_integrated() {
    let tmp = std::env::temp_dir().join("ags-test-preflight-non-integrated");
    let _ = std::fs::create_dir_all(&tmp);

    let preflight = run_session_preflight(&tmp, &AgentType::ClaudeCode);
    assert!(!preflight.is_ags_suite);
    assert!(!preflight.is_ags_integrated);
    assert_eq!(preflight.overall_status, PreflightStatus::Stop);
    assert!(preflight.should_stop);
    assert!(!preflight.failures.is_empty());
    assert_eq!(preflight.exit_code, 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_session_preflight_json_output() {
    let root = repo_root();
    let preflight = run_session_preflight(&root, &AgentType::Codex);
    let json = render_json(&preflight);
    assert!(json.contains("\"target\""));
    assert!(json.contains("\"for_agent\""));
    assert!(json.contains("\"integration_status\""));
    assert!(json.contains("\"overall_status\""));
    assert!(json.contains("\"stop_conditions\""));
    assert!(json.contains("\"warnings\""));
    assert!(json.contains("\"failures\""));
    assert!(json.contains("\"next_steps\""));
    assert!(json.contains("\"exit_code\""));
    // Verify parseable
    let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
}

#[test]
fn test_session_preflight_text_output() {
    let root = repo_root();
    let preflight = run_session_preflight(&root, &AgentType::Codex);
    let text = render_session_preflight_text(&preflight);
    assert!(text.contains("Session Preflight"));
    assert!(text.contains("Project Identity"));
    assert!(text.contains("Protocol Status"));
    assert!(text.contains("Memory Paths"));
    assert!(text.contains("Stop Conditions"));
    assert!(text.contains("Next Steps"));
    assert!(text.contains("Overall"));
}

#[test]
fn test_session_preflight_has_memory_paths() {
    let root = repo_root();
    let preflight = run_session_preflight(&root, &AgentType::Codex);
    // AGS suite repo may or may not have memory depending on role
    // At minimum, the fields should be populated for an A repo
    let inferred_is_a = preflight
        .inferred_role
        .as_ref()
        .map(|r| r.code == "A")
        .unwrap_or(false);
    if inferred_is_a {
        assert!(preflight.memory_capsule_path.is_some());
        assert!(preflight.memory_capsule_exists == Some(true));
    }
}

#[test]
fn test_session_preflight_exit_code_ok() {
    let root = repo_root();
    let preflight = run_session_preflight(&root, &AgentType::Codex);
    let code = session_preflight_exit_code(&preflight);
    assert_eq!(code, 0);
}

#[test]
fn test_session_preflight_exit_code_stop() {
    let tmp = std::env::temp_dir().join("ags-test-preflight-exit-stop");
    let _ = std::fs::create_dir_all(&tmp);

    let preflight = run_session_preflight(&tmp, &AgentType::ClaudeCode);
    let code = session_preflight_exit_code(&preflight);
    assert_eq!(code, 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_session_preflight_independent_of_skill_governance() {
    // session preflight must work even when governance/ and
    // protocol/skill-governance.md are not present
    let tmp = std::env::temp_dir().join("ags-test-preflight-no-skills");
    let _ = std::fs::create_dir_all(&tmp);
    // Create minimal AGS markers
    std::fs::write(tmp.join("AGENTS.md"), "# AGENTS\n@CLAUDE.md\n").unwrap();
    std::fs::write(tmp.join("CLAUDE.md"), "# CLAUDE\n").unwrap();

    let preflight = run_session_preflight(&tmp, &AgentType::Codex);
    // Must not panic, must produce valid output
    assert!(!preflight.for_agent.is_empty());
    let text = render_session_preflight_text(&preflight);
    assert!(!text.is_empty());
    let json = render_json(&preflight);
    assert!(json.contains("for_agent"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_preflight_status_serde() {
    assert_eq!(
        serde_json::to_string(&PreflightStatus::Ok).unwrap(),
        "\"ok\""
    );
    assert_eq!(
        serde_json::to_string(&PreflightStatus::Warning).unwrap(),
        "\"warning\""
    );
    assert_eq!(
        serde_json::to_string(&PreflightStatus::Stop).unwrap(),
        "\"stop\""
    );
}
