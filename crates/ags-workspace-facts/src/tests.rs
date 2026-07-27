use super::instruction_projection::*;
use super::protocol_audit::*;
use super::rendering::*;
use super::session_preflight::*;
use super::workspace_facts::*;
use ags_host_integration::*;
use std::path::{Path, PathBuf};

const MEMORY_SCRIPTS: &[&str] = &[
    "context-memory.sh",
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
    "raw-tool-call-stop-guard.js",
];

fn temp(tag: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("ags-workspace-facts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write_profile(project: &Path, slug: &str) {
    std::fs::create_dir_all(project.join("config")).unwrap();
    std::fs::write(
        project.join("config/agent-project-profile.yaml"),
        format!("schema_version: 1\nproject:\n  slug: {slug}\n"),
    )
    .unwrap();
}

fn write_memory(home: &Path, slug: &str, archive: bool) {
    let dir = home.join(".agents/memory/projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("context-capsule.md"), "# Context\n").unwrap();
    std::fs::write(dir.join("task-memory.md"), "# Tasks\n").unwrap();
    if archive {
        std::fs::create_dir_all(dir.join("task-archive")).unwrap();
    }
}

fn write_scripts(home: &Path) {
    let dir = home.join(".agents/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    for name in MEMORY_SCRIPTS {
        std::fs::write(dir.join(name), "x").unwrap();
    }
}

fn write_claude_hooks(project: &Path, start: bool, stop: bool) {
    let mut hooks = serde_json::Map::new();
    if start {
        hooks.insert(
            "SessionStart".into(),
            serde_json::json!([{"hooks":[{"command":"python3 $HOME/.agents/scripts/context-memory-start.py"}]}]),
        );
    }
    if stop {
        hooks.insert(
            "Stop".into(),
            serde_json::json!([{"hooks":[
                {"command":"node $HOME/.agents/scripts/raw-tool-call-stop-guard.js"},
                {"command":"python3 $HOME/.agents/scripts/claude-stop-memory-capture.py"}
            ]}]),
        );
    }
    let path = project.join(".claude/settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({"hooks": hooks})).unwrap(),
    )
    .unwrap();
}

#[test]
fn memory_lifecycle_state_matrix() {
    for (tag, memory, scripts, start, stop, archive, expected) in [
        ("absent", false, false, false, false, false, "absent"),
        ("files", true, true, false, false, true, "files-only"),
        ("read", true, true, true, false, true, "read-only"),
        ("unbacked", true, false, true, true, true, "unbacked"),
        ("full", true, true, true, true, true, "full"),
    ] {
        let root = temp(tag);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        write_profile(&project, tag);
        if memory {
            write_memory(&home, tag, archive);
        }
        if scripts {
            write_scripts(&home);
        }
        if start || stop {
            write_claude_hooks(&project, start, stop);
        }
        assert_eq!(
            compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::ClaudeCode).status,
            expected,
            "{tag}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn memory_lifecycle_is_host_specific_and_supports_codex_and_omp() {
    let root = temp("hosts");
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    write_profile(&project, "hosts");
    write_memory(&home, "hosts", true);
    write_scripts(&home);
    write_claude_hooks(&project, true, true);

    assert_eq!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::ClaudeCode).status,
        "full"
    );
    assert_ne!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::Codex).status,
        "full"
    );

    let codex = home.join(".codex/hooks.json");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(
        codex,
        r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"context-memory-start.py"}]}],"SessionEnd":[{"hooks":[{"command":"claude-stop-memory-capture.py"}]}]}}"#,
    )
    .unwrap();
    assert_eq!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::Codex).status,
        "full"
    );

    let extension = home.join(".omp/agent/extensions/ags-memory-lifecycle.js");
    std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
    std::fs::write(
        extension,
        r#"// context-memory-start.py claude-stop-memory-capture.py
export default function (pi) {
  pi.on("session_start", async () => {});
  pi.on("before_agent_start", async () => ({ systemPromptAppend: "memory" }));
  pi.on("agent_settled", async () => {});
  pi.on("session_shutdown", async () => {});
}"#,
    )
    .unwrap();
    assert_eq!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::from_str("omp").unwrap())
            .status,
        "full"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_table_parser_is_closed_and_strips_markdown() {
    let valid = "| Code | Role | Path |\n|---|---|---|\n| A | Dev | `/tmp/a` |\n| S | Stable | /tmp/s |\n\nignored";
    let rows = parse_workspace_table(valid);
    assert_eq!(
        rows.iter()
            .map(|row| (&*row.code, &*row.path))
            .collect::<Vec<_>>(),
        vec![("A", "/tmp/a"), ("S", "/tmp/s")]
    );
    for invalid in ["no table", "| Code | Role | Path |\n| A | Dev | /tmp/a |"] {
        assert!(parse_workspace_table(invalid).is_empty());
    }
}

#[test]
fn agent_type_parse_display_and_serde_share_one_matrix() {
    for (raw, canonical, display) in [
        ("codex", "codex", "Codex"),
        ("Claude Code", "claude-code", "Claude Code"),
        ("cursor", "cursor", "Cursor"),
        ("OMP", "omp", "Oh My Pi (OMP)"),
        ("WorkBuddy", "workbuddy", "Tencent Agent (WorkBuddy)"),
        (
            "CodeBuddy-Code",
            "codebuddy-code",
            "Tencent Agent (CodeBuddy-Code)",
        ),
    ] {
        let agent = AgentType::from_str(raw).unwrap();
        assert_eq!(agent.as_str(), canonical);
        assert_eq!(agent.display_name(), display);
        let encoded = serde_json::to_string(&agent).unwrap();
        let decoded: AgentType = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, agent);
    }
    assert!(AgentType::from_str(" ").is_err());
}

#[test]
fn detection_and_protocol_status_distinguish_suite_from_empty_project() {
    let suite = detect_project(&repo_root());
    assert!(suite.is_ags_suite && suite.is_ags_integrated);
    assert_eq!(project_detect_exit_code(&suite), 0);
    serde_json::from_str::<serde_json::Value>(&render_json(&suite)).unwrap();

    let root = temp("empty");
    let empty = detect_project(&root);
    assert_eq!(empty.integration_status, IntegrationStatus::NotIntegrated);
    assert_eq!(project_detect_exit_code(&empty), 1);
    let status = check_protocol_status(&root);
    assert!(!status.protocol_dir_exists);
    assert_eq!(protocol_status_exit_code(&status), 1);
    let _ = std::fs::remove_dir_all(root);

    let suite_status = check_protocol_status(&repo_root());
    assert!(suite_status.protocol_dir_exists && suite_status.task_card_validator.available);
    assert_eq!(protocol_status_exit_code(&suite_status), 0);
}

#[test]
fn instruction_projection_preserves_each_host_contract() {
    for (agent, canonical, marker) in [
        (AgentType::Codex, "codex", "ags_route_request"),
        (
            AgentType::ClaudeCode,
            "claude-code",
            "bounded handoff task cards",
        ),
        (AgentType::Cursor, "cursor", "HostRouteProposal"),
        (
            AgentType::from_str("workbuddy").unwrap(),
            "workbuddy",
            "AGS-compatible governed host",
        ),
    ] {
        let instructions = generate_agent_instructions(&repo_root(), &agent);
        assert_eq!(instructions.agent_type, canonical);
        assert!(
            instructions.instructions_text.contains(marker),
            "{canonical}"
        );
        assert!(!instructions.required_reads.is_empty());
        assert!(!instructions.verification_commands.is_empty());
        assert!(!instructions.should_stop);
    }
}

#[test]
fn verification_command_detection_is_target_aware() {
    let root = temp("verify");
    assert!(detect_verification_commands(&root)
        .iter()
        .any(|command| command.contains("No project-specific")));
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/verify.sh"), "#!/bin/sh\n").unwrap();
    let commands = detect_verification_commands(&root);
    for expected in ["cargo fmt", "cargo test", "cargo build", "verify.sh"] {
        assert!(
            commands.iter().any(|command| command.contains(expected)),
            "{expected}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_preflight_host_matrix_and_rendering() {
    for (agent, canonical) in [
        (AgentType::Codex, "codex"),
        (AgentType::ClaudeCode, "claude-code"),
        (AgentType::Cursor, "cursor"),
        (AgentType::from_str("OMP").unwrap(), "omp"),
        (AgentType::from_str("WorkBuddy").unwrap(), "workbuddy"),
    ] {
        let preflight = run_session_preflight(&repo_root(), &agent);
        assert_eq!(preflight.for_agent, canonical);
        assert_eq!(preflight.integration_status, IntegrationStatus::Suite);
        assert_ne!(preflight.overall_status, PreflightStatus::Stop);
        assert_eq!(session_preflight_exit_code(&preflight), 0);
        serde_json::from_str::<serde_json::Value>(&render_json(&preflight)).unwrap();
        assert!(render_session_preflight_text(&preflight).contains("Session Preflight"));
    }
}

#[test]
fn non_integrated_preflight_fails_closed_without_skill_state() {
    let root = temp("stop");
    std::fs::write(root.join("AGENTS.md"), "# AGENTS\n").unwrap();
    let preflight = run_session_preflight(&root, &AgentType::Codex);
    assert_eq!(preflight.overall_status, PreflightStatus::Stop);
    assert!(preflight.should_stop);
    assert_eq!(session_preflight_exit_code(&preflight), 1);
    assert!(!preflight.failures.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
