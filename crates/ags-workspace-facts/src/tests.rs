use super::instruction_projection::*;
use super::protocol_audit::*;
use super::rendering::*;
use super::session_preflight::*;
use super::workspace_facts::*;
use ags_host_integration::*;
use std::path::{Path, PathBuf};

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

fn write_memory(home: &Path, project: &Path, archive: bool) {
    let dir = home
        .join(".agents/memory/projects")
        .join(project_memory_key(project).unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("context-capsule.md"), "# Context\n").unwrap();
    std::fs::write(dir.join("task-memory.md"), "# Tasks\n").unwrap();
    if archive {
        std::fs::create_dir_all(dir.join("task-archive")).unwrap();
    }
}

fn write_command_hooks(project: &Path, host: &str, start: bool, stop: bool) {
    let codec = HostLifecycleCodec::new(project, host).unwrap();
    let spec = codec.spec();
    let mut hooks = codec.desired_owned_projection();
    if !start {
        hooks.remove(spec.native_events.session_start);
    }
    if !stop {
        hooks.remove(spec.native_events.stop_guard);
        hooks.remove(spec.native_events.session_end);
    }
    let path = codec.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({"hooks": hooks})).unwrap(),
    )
    .unwrap();
}

#[test]
fn memory_lifecycle_state_matrix() {
    for (tag, memory, start, stop, archive, expected) in [
        ("absent", false, false, false, false, "absent"),
        ("files", true, false, false, true, "files-only"),
        ("read", true, true, false, true, "read-only"),
        ("full", true, true, true, true, "full"),
    ] {
        let root = temp(tag);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        write_profile(&project, tag);
        if memory {
            write_memory(&home, &project, archive);
        }
        if start || stop {
            write_command_hooks(&project, "claude-code", start, stop);
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
    write_memory(&home, &project, true);
    write_command_hooks(&project, "claude-code", true, true);

    assert_eq!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::ClaudeCode).status,
        "full"
    );
    assert_ne!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::Codex).status,
        "full"
    );

    write_command_hooks(&project, "codex", true, true);
    assert_eq!(
        compute_memory_lifecycle_at_for_host(&project, &home, &AgentType::Codex).status,
        "full"
    );

    let omp_codec = HostLifecycleCodec::new(&project, "omp").unwrap();
    let extension = omp_codec.path();
    std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
    std::fs::write(extension, omp_codec.desired_omp_body()).unwrap();
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
fn public_suite_identity_does_not_depend_on_machine_path_rows() {
    let root = temp("public-suite");
    std::fs::create_dir_all(root.join("crates")).unwrap();
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::create_dir_all(root.join("protocol")).unwrap();
    std::fs::write(root.join("WORKSPACE.md"), "# Public AGS workspace\n").unwrap();
    std::fs::write(root.join("AGENT_SUITE_PROTOCOL.md"), "# Protocol\n").unwrap();
    std::fs::write(root.join("AGENTS.md"), "# AGENTS\n").unwrap();
    std::fs::write(root.join("CLAUDE.md"), "# CLAUDE\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/ags-cli\"]\n",
    )
    .unwrap();
    std::fs::write(root.join("manifests/suite.yaml"), "suite: {}\n").unwrap();
    for protocol in [
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
    ] {
        std::fs::write(root.join("protocol").join(protocol), "# Protocol\n").unwrap();
    }

    let identity = detect_project(&root);
    assert!(identity.workspace_identities.is_empty());
    assert!(identity.is_ags_suite);
    assert_eq!(identity.integration_status, IntegrationStatus::Suite);
    let preflight = run_session_preflight(&root, &AgentType::ClaudeCode);
    assert_eq!(preflight.exit_code, 0, "{:?}", preflight.failures);

    std::fs::remove_file(root.join("manifests/suite.yaml")).unwrap();
    assert!(!detect_project(&root).is_ags_suite);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn instruction_projection_preserves_each_host_contract() {
    for (agent, canonical, marker) in [
        (AgentType::Codex, "codex", "ags_decide"),
        (
            AgentType::ClaudeCode,
            "claude-code",
            "bounded handoff task cards",
        ),
        (AgentType::Cursor, "cursor", "OperationRequest"),
        (
            AgentType::from_str("workbuddy").unwrap(),
            "workbuddy",
            "contract-v2 CLI/MCP surfaces",
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
        .any(|command| command.contains("ags check governance")));
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\n").unwrap();
    let commands = detect_verification_commands(&root);
    for expected in [
        "cargo fmt",
        "cargo test",
        "cargo build",
        "ags check governance",
    ] {
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
