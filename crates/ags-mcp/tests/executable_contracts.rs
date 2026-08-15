use std::process::Command;

fn production_rust_sources(root: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            production_rust_sources(&path, output);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs")
            && path.components().any(|part| part.as_os_str() == "src")
        {
            output.push(path);
        }
    }
}

fn contains_complete_token(source: &str, token: &str) -> bool {
    source.match_indices(token).any(|(offset, _)| {
        let before = source[..offset].chars().next_back();
        let after = source[offset + token.len()..].chars().next();
        let is_identifier = |character: Option<char>| {
            character.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        };
        !is_identifier(before) && !is_identifier(after)
    })
}

#[test]
fn standalone_mcp_executable_owns_stdio_and_private_daemon_modes() {
    let invalid = Command::new(env!("CARGO_BIN_EXE_ags-mcp"))
        .arg("serve")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    let stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(stderr.contains("ags-mcp [stdio | daemon --workspace <path>]"));
    assert!(!stderr.contains("ags mcp"));
}

#[test]
fn standalone_host_executable_owns_lifecycle_callback_shape() {
    let invalid = Command::new(env!("CARGO_BIN_EXE_ags-host"))
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    let stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(stderr.contains("ags-host lifecycle"));
    assert!(stderr.contains("--workspace <path>"));
    assert!(!stderr.contains("ags host"));
}

#[test]
fn host_lifecycle_has_no_parallel_workspace_command_or_raw_host_identity() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let host = std::fs::read_to_string(manifest.join("src/bin/ags-host.rs")).unwrap();
    let daemon = std::fs::read_to_string(manifest.join("src/lib.rs")).unwrap();
    let product_cli = std::fs::read_to_string(manifest.join("../ags-cli/src/main.rs")).unwrap();

    assert!(!host.contains("dispatch_workspace_command"));
    assert!(!host.contains("workspace_lifecycle::LifecycleEnvelope"));
    assert!(host.contains("WorkspaceControlRequest"));
    assert!(host.contains("OperationRequest::HostLifecycleSessionStart"));
    assert!(host.contains("OperationRequest::HostLifecycleSessionEnd"));
    assert!(host.contains("OperationRequest::HostLifecycleStopGuard"));
    assert!(host.contains("outcome_token"));
    assert!(host.contains("HostOutcomeInput"));
    assert!(host.contains("HostExecutionInstruction"));
    assert!(host.contains("DetailsReadRequest"));
    assert!(host.contains("instruction_digest"));
    assert!(host.contains("observed_write_set"));
    assert!(host.contains("generic_agent.host_id.as_str()"));
    assert!(!host.contains("Command::new(\"ags\")"));
    assert!(!host.contains("sh\", \"-c"));
    assert!(!daemon.contains("if kind == \"lifecycle\""));
    assert!(!daemon.contains("workspace_lifecycle::LifecycleKernel"));
    assert!(!daemon.contains("if outcome.is_some()"));
    assert!(daemon.contains("AuthenticatedHostOutcome::from_artifact"));
    assert!(
        !product_cli.contains("HostExecutionInstruction"),
        "ordinary ags CLI must not auto-execute HostDelegated instructions"
    );
}

#[test]
fn production_rust_has_no_retired_mcp_tool_names() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let retired = ["ags_policy", "_resolve"].concat();
    let mut sources = Vec::new();
    production_rust_sources(&repo.join("crates"), &mut sources);
    let offenders = sources
        .into_iter()
        .filter(|path| std::fs::read_to_string(path).unwrap().contains(&retired))
        .collect::<Vec<_>>();
    assert!(offenders.is_empty(), "{retired} remains in {offenders:?}");
}

#[test]
fn production_rust_has_no_retired_agent_registration_or_skill_wire() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let retired = [
        ["AgentRegistration", "Receipt"].concat(),
        ["Decision", "Lease"].concat(),
        ["HostRoute", "Proposal"].concat(),
        ["CliCapability", "Id"].concat(),
        ["GovernSkill", "Adopt"].concat(),
        ["SkillAdopt", "Request"].concat(),
        ["skill", "_adopt"].concat(),
    ];
    let mut sources = Vec::new();
    production_rust_sources(&repo.join("crates"), &mut sources);
    let offenders = sources
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            retired
                .iter()
                .find(|token| contains_complete_token(&source, token))
                .map(|token| (path, token.clone()))
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "retired registration/Skill wire remains in {offenders:?}"
    );
}

#[test]
fn production_docs_have_no_retired_mcp_tool_names() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let retired = [
        ["ags_", "preflight"].concat(),
        ["ags_", "route_request"].concat(),
        ["ags_", "apply_action"].concat(),
        ["ags_", "policy_resolve"].concat(),
        ["ags_", "task_validate"].concat(),
        ["ags_", "maintenance_status"].concat(),
        ["ags_", "maintenance_plan"].concat(),
        ["ags_", "maintenance_apply"].concat(),
        ["ags_", "maintenance_verify"].concat(),
        ["ags_", "maintenance_recover"].concat(),
        ["ags_", "protocol_status"].concat(),
        ["ags_", "agent_instructions"].concat(),
        ["ags_", "onboarding_plan"].concat(),
    ];
    let tracked = Command::new("git")
        .args(["ls-files", "-z", "--", "*.md"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(tracked.status.success());
    let offenders = tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| repo.join(std::str::from_utf8(path).unwrap()))
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let contents = std::fs::read_to_string(&path).unwrap();
            retired
                .iter()
                .find(|token| contents.contains(token.as_str()))
                .map(|token| (path, token.clone()))
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "retired MCP tools remain in {offenders:?}"
    );
}

#[test]
fn production_docs_have_no_retired_skill_adopt_surface() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let tracked = Command::new("git")
        .args(["ls-files", "-z", "--", "*.md"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(tracked.status.success());
    let retired = ["govern skill adopt", "skill_adopt"];
    let offenders = tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| repo.join(std::str::from_utf8(path).unwrap()))
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let contents = std::fs::read_to_string(&path).unwrap();
            retired
                .iter()
                .find(|token| contents.contains(**token))
                .map(|token| (path, *token))
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "retired Skill surface remains in {offenders:?}"
    );
}

#[test]
fn private_control_transport_has_no_unbounded_read_line() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mcp = std::fs::read_to_string(manifest.join("src/lib.rs")).unwrap();
    let mcp = mcp.split("#[cfg(test)]").next().unwrap();
    let session = std::fs::read_to_string(
        manifest.join("../ags-session/src/workspace_service/transport_handshake.rs"),
    )
    .unwrap();
    let token = ["read", "_line("].concat();
    assert!(!mcp.contains(&token), "ags-mcp private socket is unbounded");
    assert!(
        !session.contains(&token),
        "workspace handshake/reply path is unbounded"
    );
}
