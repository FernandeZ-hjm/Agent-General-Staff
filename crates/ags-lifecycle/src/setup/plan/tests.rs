use super::super::templates::{
    claude_ags_command_content, codex_ags_command_skill_agent_metadata_content,
    codex_ags_command_skill_content, codex_ags_command_skill_specs,
};
use super::super::AGS_VERSION;
use super::super::{
    claude_ags_command_path, codex_ags_named_skill_path, retired_codex_ags_skill_dirs,
};
use super::*;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn private_install_plan_excludes_evomap_by_default() {
    let target = std::env::temp_dir().join("ags-private-install-plan-default-test");
    let home = std::env::temp_dir().join("ags-private-install-plan-default-home");
    let plan = private_install_plan(&workspace_root(), &target, &home);
    assert!(!plan
        .files
        .iter()
        .any(|file| file.path.ends_with("mcp/gep.mcp.json")));
    assert!(plan
        .files
        .iter()
        .any(|file| file.path == claude_ags_command_path(&home)));
    let host_entry_policy = plan
        .files
        .iter()
        .find(|file| file.path.ends_with("hosts/host-entry-policy.md"))
        .expect("host entry policy must be installed");
    assert!(host_entry_policy.content.contains("HostRouteProposal"));
    assert!(host_entry_policy.content.contains("RouteResolution"));
    assert!(host_entry_policy.content.contains("host Plan mode"));
    assert!(host_entry_policy.content.contains("task_card_hash"));
    assert!(!host_entry_policy.content.contains("RequestDecision"));
    let manifest = plan
        .files
        .iter()
        .find(|file| file.path.ends_with("install-manifest.json"))
        .expect("manifest file must be generated");
    assert!(manifest.content.contains("\"slash_command\": \"/ags\""));
    assert!(manifest.content.contains("ags-setup"));
    assert!(manifest.content.contains("ags-init"));
    assert!(manifest.content.contains("ags-skill"));
    assert!(manifest.content.contains("ags-agents"));
    assert!(manifest.content.contains(".claude/commands/ags.md"));
    assert!(!manifest.content.contains(".codex/skills/ags/SKILL.md"));
    for name in ["ags-core.md", "ags-task-handoff.md", "host-operations.md"] {
        assert!(
            plan.files.iter().any(|file| file.path.ends_with(name)),
            "global rule module must be installed: {name}"
        );
    }
    for (name, _, _, _, _) in codex_ags_command_skill_specs() {
        assert!(plan
            .files
            .iter()
            .any(|file| file.path == codex_ags_named_skill_path(&home, name)));
    }
    for retired_dir in retired_codex_ags_skill_dirs(&home) {
        assert!(plan.cleanup_dirs.iter().any(|dir| dir == &retired_dir));
    }
}

/// Codex front-stage command skills are EXACTLY the canonical five
/// (setup / agents / skill / init / doctor). `ags-capability` must NOT be a
/// visible command skill — it is retired into `retired_visible_skills` while
/// the underlying `ags capability` CLI remains.
#[test]
fn codex_visible_command_skills_are_exactly_the_canonical_five() {
    let target = std::env::temp_dir().join("ags-private-install-plan-five-set-test");
    let home = std::env::temp_dir().join("ags-private-install-plan-five-set-home");
    let plan = private_install_plan(&workspace_root(), &target, &home);

    // 1. The spec list itself is exactly the canonical five, in order.
    let spec_names: Vec<&str> = codex_ags_command_skill_specs()
        .iter()
        .map(|(name, _, _, _, _)| *name)
        .collect();
    assert_eq!(
        spec_names,
        vec![
            "ags-setup",
            "ags-agents",
            "ags-skill",
            "ags-init",
            "ags-doctor"
        ],
        "Codex front-stage command skills must be exactly setup/agents/skill/init/doctor"
    );
    assert!(
        !spec_names.contains(&"ags-capability"),
        "ags-capability must not be a front-stage Codex command skill"
    );

    // 2. The install manifest's codex.command_skills mirror the five and
    //    exclude ags-capability; ags-capability appears only as retired.
    let manifest = plan
        .files
        .iter()
        .find(|file| file.path.ends_with("install-manifest.json"))
        .expect("manifest file must be generated");
    let json: serde_json::Value =
        serde_json::from_str(&manifest.content).expect("manifest is valid JSON");
    let command_skills = json["host_commands"]["codex"]["command_skills"]
        .as_array()
        .expect("codex.command_skills is an array");
    assert_eq!(
        command_skills.len(),
        5,
        "exactly five visible Codex command skills"
    );
    for expected in [
        "ags-setup",
        "ags-agents",
        "ags-skill",
        "ags-init",
        "ags-doctor",
    ] {
        assert!(
            command_skills
                .iter()
                .any(|p| p.as_str().is_some_and(|s| s.contains(expected))),
            "command_skills must include {expected}"
        );
    }
    assert!(
        !command_skills
            .iter()
            .any(|p| p.as_str().is_some_and(|s| s.contains("ags-capability"))),
        "ags-capability must NOT be a visible Codex command skill"
    );
    let retired = json["host_commands"]["codex"]["retired_visible_skills"]
        .as_array()
        .expect("codex.retired_visible_skills is an array");
    assert!(
        retired
            .iter()
            .any(|p| p.as_str().is_some_and(|s| s.contains("ags-capability"))),
        "ags-capability must be listed among retired_visible_skills"
    );

    // 3. No generated install file targets an ags-capability skill body.
    assert!(
        !plan
            .files
            .iter()
            .any(|f| f.path == codex_ags_named_skill_path(&home, "ags-capability")),
        "setup must not generate an ags-capability Codex skill body"
    );

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn tencent_agent_host_snippets_register_ags_mcp() {
    // Tencent Agent / WorkBuddy / CodeBuddy-Code are platform-host MCP
    // integration snippets. They register AGS MCP only; they do not create
    // runtime adapters or change execution-policy authority.
    let target = std::env::temp_dir().join("ags-tencent-snippet-struct-test");
    let home = std::env::temp_dir().join("ags-tencent-snippet-struct-home");
    let plan = private_install_plan(&workspace_root(), &target, &home);
    for name in [
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
    ] {
        let file = plan
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap_or_else(|| panic!("missing host MCP snippet: {name}"));
        let json: serde_json::Value = serde_json::from_str(&file.content)
            .unwrap_or_else(|e| panic!("{name} must be valid JSON: {e}"));
        let entry = json
            .get("mcpServers")
            .and_then(|servers| servers.get("ags"))
            .unwrap_or_else(|| panic!("{name} must expose mcpServers.ags"));
        assert_eq!(
            entry.get("mandatory_first_tool").and_then(|v| v.as_str()),
            Some("ags_preflight"),
            "{name} must register ags_preflight as mandatory_first_tool"
        );
        assert_eq!(
            entry.get("command").and_then(|v| v.as_str()),
            Some("ags"),
            "{name} ags entry must launch the `ags` command"
        );
    }
}

#[test]
fn claude_ags_command_mentions_preflight_and_current_version() {
    let content = claude_ags_command_content();
    assert!(content.contains("ags_preflight"));
    assert!(content.contains("ags session preflight --for claude-code --target ."));
    assert!(content.contains("ags setup --yes --force --register-claude"));
    assert!(content.contains("ags init --target ."));
    assert!(content.contains("/ags setup"));
    assert!(content.contains("/ags init"));
    assert!(content.contains("strictly read-only `ags_route_request`"));
    assert!(content.contains("typed `HostRouteProposal`"));
    assert!(content.contains("`ags_apply_action`"));
    assert!(content.contains("Explicit handoff"));
    assert!(content.contains("confirmed-handoff-contract"));
    assert!(content.contains("solution work is unresolved or reopened"));
    assert!(content.contains("direct edit stays host-native"));
    assert!(content.contains(AGS_VERSION));
}

#[test]
fn codex_ags_command_skills_mention_top_level_routes() {
    for (name, display_name, _, _, summary) in codex_ags_command_skill_specs() {
        let content = codex_ags_command_skill_content(name, display_name, summary);
        let route = name.strip_prefix("ags-").unwrap_or(name);
        assert!(content.contains(&format!("name: \"{name}\"")));
        assert!(content.contains(&format!("/ags {route}")));
        assert!(content.contains("ags session preflight --for codex --target ."));
        assert!(content.contains("明确要求任务卡/交接"));
        assert!(content.contains("handoff contract 已独立确认"));
        assert!(content.contains("未决或重开的 solution work"));
        assert!(content.contains(AGS_VERSION));
        assert!(content.contains("必须先执行"));
    }
}

#[test]
fn codex_ags_skill_metadata_uses_command_shaped_display_names() {
    for (_, display_name, short_description, default_prompt, _) in codex_ags_command_skill_specs() {
        let metadata = codex_ags_command_skill_agent_metadata_content(
            display_name,
            short_description,
            default_prompt,
        );
        assert!(display_name.starts_with("AGS "));
        assert!(short_description
            .chars()
            .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)));
        assert!(metadata.contains(&format!("display_name: \"{display_name}\"")));
        assert!(metadata.contains(short_description));
        assert!(metadata.contains(default_prompt));
    }
}

#[test]
fn public_install_plan_excludes_maintainer_only_extensions() {
    let target = std::env::temp_dir().join("ags-public-install-plan-extension-test");
    let home = std::env::temp_dir().join("ags-public-install-plan-extension-home");
    let plan = private_install_plan(&workspace_root(), &target, &home);
    assert!(plan.files.iter().all(|file| {
        let path = file.path.to_string_lossy();
        !path.contains("optional-peer") && !path.contains("maintainer-extension")
    }));
}

/// Adversarial-review fix: a retired thin-index symlink is unlinked only —
/// the canonical body it points at is never touched (no blind remove_dir_all).
#[cfg(unix)]
#[test]
fn cleanup_retire_unlinks_symlink_without_touching_canonical() {
    let base = std::env::temp_dir().join(format!("ags-cleanup-symlink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let canonical = base.join("repo/global-skills/ags-capability");
    let host = base.join("home/.codex/skills/ags-capability");
    std::fs::create_dir_all(&canonical).unwrap();
    std::fs::create_dir_all(host.parent().unwrap()).unwrap();
    std::fs::write(canonical.join("SKILL.md"), "canonical body\n").unwrap();
    std::os::unix::fs::symlink(&canonical, &host).unwrap();

    let finding = cleanup_install_dir(&host, false, 7);
    assert_eq!(finding.status, crate::setup::SetupCheckStatus::Pass);
    assert!(finding.message.contains("unlinked thin-index symlink"));
    assert!(std::fs::symlink_metadata(&host).is_err(), "symlink removed");
    assert_eq!(
        std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
        "canonical body\n",
        "canonical body must be untouched"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// An AGS-generated retired dir is MOVED to a timestamped backup (reversible),
/// never deleted.
#[test]
fn cleanup_retire_quarantines_ags_generated_dir_reversibly() {
    let base = std::env::temp_dir().join(format!("ags-cleanup-ags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("ags-capability");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: \"ags-capability\"\n---\nags session preflight --for codex --target .\n",
    )
    .unwrap();

    let finding = cleanup_install_dir(&dir, false, 42);
    assert_eq!(finding.status, crate::setup::SetupCheckStatus::Pass);
    assert!(finding.message.contains("quarantined to backup"));
    assert!(!dir.exists(), "original retired entry moved aside");
    let backup = base.join("ags-capability.retired.bak.42");
    assert!(backup.join("SKILL.md").is_file(), "backup is reversible");
    let _ = std::fs::remove_dir_all(&base);
}

/// Unrecognized (possibly user-edited) content is left in place without
/// --force; with --force it is quarantined to backup, never deleted.
#[test]
fn cleanup_retire_refuses_unrecognized_content_without_force() {
    let base = std::env::temp_dir().join(format!("ags-cleanup-user-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("ags-capability");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: mine\n---\nmy custom skill\n",
    )
    .unwrap();

    let finding = cleanup_install_dir(&dir, false, 1);
    assert_eq!(finding.status, crate::setup::SetupCheckStatus::Fail);
    assert!(
        dir.join("SKILL.md").is_file(),
        "user content must be left untouched without --force"
    );

    let finding = cleanup_install_dir(&dir, true, 2);
    assert_eq!(finding.status, crate::setup::SetupCheckStatus::Pass);
    assert!(!dir.exists());
    assert!(base
        .join("ags-capability.retired.bak.2")
        .join("SKILL.md")
        .is_file());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cleanup_retire_absent_is_pass() {
    let base = std::env::temp_dir().join(format!("ags-cleanup-absent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let finding = cleanup_install_dir(&base.join("nope"), false, 1);
    assert_eq!(finding.status, crate::setup::SetupCheckStatus::Pass);
    assert!(finding.message.contains("absent"));
}
