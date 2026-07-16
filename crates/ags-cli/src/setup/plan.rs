use super::{
    claude_ags_command_path, codex_ags_named_skill_agent_metadata_path, codex_ags_named_skill_path,
    retired_codex_ags_skill_dirs, PRIVATE_INSTALL_SCHEMA,
};
use crate::context::{home_dir, sanitize_name, shell_quote};
use crate::file_plan::InstallFile;
use crate::project_templates::{portable_validate_script, project_protocol_files};
use crate::setup::apply::codex_skill_thin_index_ancestor;
use crate::setup::templates::{
    claude_ags_command_content, codex_ags_command_skill_agent_metadata_content,
    codex_ags_command_skill_content, codex_ags_command_skill_specs,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(in crate::setup) struct PrivateInstallPlan {
    pub(crate) profile: String,
    pub(crate) source_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) files: Vec<InstallFile>,
    pub(crate) cleanup_dirs: Vec<PathBuf>,
}
pub(in crate::setup) fn private_install_plan(
    source_root: &Path,
    target: &Path,
) -> PrivateInstallPlan {
    let ags_mcp_json = r#"{
  "mcpServers": {
    "ags": {
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "AGS_RUNTIME_HOME": "__TARGET__"
      }
    },
    "codegraph": {
      "command": "codegraph",
      "args": ["serve", "--mcp"]
    }
  },
  "initialization_gate": {
    "mandatory_first_tool": "ags_preflight",
    "failed_preflight_opens_gate": false
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let codex_snippet = r#"# AGS MCP host initialization adapter
# Merge this snippet into ~/.codex/config.toml after review.
[mcp_servers.ags]
command = "ags"
args = ["mcp", "serve", "--transport", "stdio"]

[mcp_servers.ags.env]
AGS_RUNTIME_HOME = "__TARGET__"

[mcp_servers.codegraph]
command = "codegraph"
args = ["serve", "--mcp"]
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let claude_snippet = r#"{
  "mcpServers": {
    "ags": {
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "AGS_RUNTIME_HOME": "__TARGET__"
      }
    },
    "codegraph": {
      "command": "codegraph",
      "args": ["serve", "--mcp"]
    }
  },
  "hooks": {
    "Stop": [
      {
        "command": "node __TARGET__/hooks/claude-code-executor-stop.js",
        "timeout": 8
      }
    ]
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    // Tencent Agent is the platform family; WorkBuddy and CodeBuddy-Code are
    // host clients. These snippets are host-platform MCP registrations for AGS,
    // not task-card runtime adapters and not execution-policy authority.
    let host_platform_mcp_snippet = |client_note: &str| -> String {
        format!(
            r#"{{
  "mcpServers": {{
    "ags": {{
      "role": "host_initialization_adapter",
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "mandatory_first_tool": "ags_preflight",
      "_comment": "{client_note}"
    }}
  }}
}}
"#
        )
    };
    let tencent_agent_snippet = host_platform_mcp_snippet(
        "Tencent Agent platform MCP registration for AGS. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients sharing this AGS MCP entry.",
    );
    let workbuddy_snippet = host_platform_mcp_snippet(
        "WorkBuddy (Tencent Agent host client) platform MCP registration for AGS.",
    );
    let codebuddy_code_snippet = host_platform_mcp_snippet(
        "CodeBuddy-Code (Tencent Agent host client) platform MCP registration for AGS.",
    );

    let profile = r#"schema_version: "2.7-public-runtime-profile"
profiles:
  claude-code-executor:
    role: "executor"
    first_tool: "ags_preflight"
    hooks: []
    note: "Public edition records AGS governance posture only; no private runtime hooks are bundled."
  planner:
    role: "planner"
    first_tool: "ags_preflight"
    advisory_recall: "disabled"
    note: "Use AGS preflight and solution formation; public edition does not bundle local recall hooks."
"#
    .to_string();

    let claude_hook = r#"#!/usr/bin/env node
// AGS public edition no-op Stop hook.
// Private runtime hooks are not bundled in the public release.
process.exit(0);
"#
    .to_string();

    let codex_hook = r#"{
  "schema_version": "2.7-public-hook-placeholder",
  "hooks": [],
  "boundary": "Public edition does not bundle local planner recall hooks; use AGS preflight first."
}
"#
    .to_string();

    let launcher = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport AGS_RUNTIME_HOME={}\nexec ags mcp serve --transport stdio\n",
        shell_quote(target)
    );

    let manifest = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": "private",
        "source_root": source_root.to_string_lossy(),
        "target": target.to_string_lossy(),
        "mcp": {
            "server": "ags",
            "command": "ags mcp serve --transport stdio",
            "mandatory_first_tool": "ags_preflight"
        },
        "host_snippets": serde_json::json!([
            "hosts/codex.config.snippet.toml",
            "hosts/claude-code.mcp.snippet.json",
            "hosts/tencent-agent.mcp.snippet.json",
            "hosts/workbuddy.mcp.snippet.json",
            "hosts/codebuddy-code.mcp.snippet.json"
        ]),
        "host_commands": {
            "claude_code": {
                "slash_command": "/ags",
                "path": claude_ags_command_path().to_string_lossy().replace('\\', "/")
            },
            "codex": {
                "command_skills": codex_ags_command_skill_specs()
                    .iter()
                    .map(|(name, _, _, _, _)| codex_ags_named_skill_path(name).to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>(),
                "retired_visible_skills": retired_codex_ags_skill_dirs()
                    .iter()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>()
            }
        },
        "created_by": "ags setup",
    });

    let readme = format!(
        "# AGS Private Runtime\n\n\
This directory was generated by `ags setup`.\n\n\
## Commands\n\n\
- MCP server: `ags mcp serve --transport stdio`\n\
- Doctor: `ags doctor`\n\
- Runtime check: `ags doctor --target {}`\n\n\
## Host snippets\n\n\
Review files in `hosts/` before merging them into host-specific global config.\n\
AGS scenarios must call `ags_preflight` before any other AGS tool.\n\n\
## Claude Code slash command\n\n\
The one-line installer seeds `/ags`; `ags setup --yes` refreshes it at `~/.claude/commands/ags.md`.\n\
Use `/ags setup` to initialize this machine and `/ags init` to onboard the current project.\n\
Diagnostics remain available as `/ags preflight` and `/ags doctor`; verification gates drive `ags verify` internally.\n\n\
## Codex skills\n\n\
`ags setup --yes` installs visible top-level command skills: `$ags-setup`, `$ags-agents`, `$ags-skill`, `$ags-init`, and `$ags-doctor`.\n\
Retired visible skills (`$ags`, `$ags-preflight`, `$ags-verify`, `$ags-capability`) are removed from the Codex skill list during setup.\n\
`ags capability` remains the Cross-Agent visibility/sync CLI and is no longer installed as a visible Codex command skill.\n\
`ags verify` remains a kernel/CI verification command and is not installed as a visible Codex skill.\n\
Each command skill routes through AGS preflight before acting.\n",
        target.display()
    );

    let mut files = vec![
        InstallFile {
            path: target.join("install-manifest.json"),
            description: "machine-readable private runtime install manifest".to_string(),
            content: serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n",
            mode: None,
        },
        InstallFile {
            path: target.join("README.md"),
            description: "operator notes for this private runtime home".to_string(),
            content: readme,
            mode: None,
        },
        InstallFile {
            path: target.join("mcp/ags.mcp.json"),
            description: "generic MCP registration snippet for AGS host adapter".to_string(),
            content: ags_mcp_json,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/codex.config.snippet.toml"),
            description: "Codex MCP config snippet".to_string(),
            content: codex_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/claude-code.mcp.snippet.json"),
            description: "Claude Code MCP and Stop hook snippet".to_string(),
            content: claude_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/tencent-agent.mcp.snippet.json"),
            description: "Tencent Agent platform MCP registration snippet for AGS".to_string(),
            content: tencent_agent_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/workbuddy.mcp.snippet.json"),
            description: "WorkBuddy platform MCP registration snippet for AGS".to_string(),
            content: workbuddy_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/codebuddy-code.mcp.snippet.json"),
            description: "CodeBuddy-Code platform MCP registration snippet for AGS".to_string(),
            content: codebuddy_code_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("manifests/runtime-profiles.yaml"),
            description: "private runtime profile with local-safe defaults".to_string(),
            content: profile,
            mode: None,
        },
        InstallFile {
            path: target.join("hooks/claude-code-executor-stop.js"),
            description: "Claude Code executor Stop hook".to_string(),
            content: claude_hook,
            mode: Some(0o755),
        },
        InstallFile {
            path: target.join("hooks/codex-planner-recall.json"),
            description: "Codex/Cursor planner hook template".to_string(),
            content: codex_hook,
            mode: None,
        },
        InstallFile {
            path: target.join("bin/ags-mcp-stdio.sh"),
            description: "portable launcher for AGS MCP stdio server".to_string(),
            content: launcher,
            mode: Some(0o755),
        },
        InstallFile {
            path: claude_ags_command_path(),
            description: "Claude Code user slash command for AGS governance".to_string(),
            content: claude_ags_command_content(),
            mode: None,
        },
        InstallFile {
            path: target.join("project-templates/scripts/validate.sh"),
            description: "portable project task-card validator wrapper".to_string(),
            content: portable_validate_script(),
            mode: Some(0o755),
        },
    ];
    files.extend(super::memory::memory_script_install_files(&home_dir()));

    for (name, display_name, short_description, default_prompt, summary) in
        codex_ags_command_skill_specs()
    {
        files.push(InstallFile {
            path: codex_ags_named_skill_path(name),
            description: format!("Codex AGS command skill: {name}"),
            content: codex_ags_command_skill_content(name, display_name, summary),
            mode: None,
        });
        files.push(InstallFile {
            path: codex_ags_named_skill_agent_metadata_path(name),
            description: format!("Codex AGS command skill UI metadata: {name}"),
            content: codex_ags_command_skill_agent_metadata_content(
                display_name,
                short_description,
                default_prompt,
            ),
            mode: None,
        });
    }

    for name in project_protocol_files() {
        let src = source_root.join("protocol").join(name);
        if let Ok(content) = std::fs::read_to_string(&src) {
            files.push(InstallFile {
                path: target.join("project-templates/protocol").join(name),
                description: format!("project onboarding protocol template: protocol/{name}"),
                content,
                mode: None,
            });
        }
    }

    PrivateInstallPlan {
        profile: "private".to_string(),
        source_root: source_root.to_path_buf(),
        target: target.to_path_buf(),
        files,
        cleanup_dirs: retired_codex_ags_skill_dirs(),
    }
}

fn install_file_status(file: &InstallFile) -> &'static str {
    if codex_skill_thin_index_ancestor(&file.path).is_some() {
        return "thin-index-symlink";
    }
    match std::fs::read(&file.path) {
        Ok(existing) if existing == file.content.as_bytes() => "unchanged",
        Ok(_) => "would-replace",
        Err(_) => "would-create",
    }
}
pub(in crate::setup) fn render_private_plan_json(plan: &PrivateInstallPlan) -> String {
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path.to_string_lossy(),
                "description": file.description,
                "mode": file.mode.map(|m| format!("{m:o}")),
                "status": install_file_status(file),
            })
        })
        .collect();
    let cleanup_dirs: Vec<_> = plan
        .cleanup_dirs
        .iter()
        .map(|path| {
            serde_json::json!({
                "path": path.to_string_lossy(),
                "status": if path.exists() { "would-retire" } else { "absent" },
            })
        })
        .collect();

    let output = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": plan.profile,
        "source_root": plan.source_root.to_string_lossy(),
        "target": plan.target.to_string_lossy(),
        "write_mode": "plan-only",
        "files": files,
        "cleanup_dirs": cleanup_dirs,
        "host_config_policy": "MCP snippets are generated only; Claude Code /ags command and Codex AGS command skills are installed on apply",
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}
pub(in crate::setup) fn render_private_plan_text(plan: &PrivateInstallPlan) -> String {
    let mut lines = vec![
        format!(
            "AGS Private Runtime Install Plan {}",
            PRIVATE_INSTALL_SCHEMA
        ),
        format!("Profile: {}", plan.profile),
        format!("Source:  {}", plan.source_root.display()),
        format!("Target:  {}", plan.target.display()),
        "Mode:    plan-only".to_string(),
        String::new(),
        "Files:".to_string(),
    ];
    for (i, file) in plan.files.iter().enumerate() {
        let mode = file
            .mode
            .map(|m| format!(" mode={m:o}"))
            .unwrap_or_default();
        lines.push(format!(
            "  {}. [{}{}] {} — {}",
            i + 1,
            install_file_status(file),
            mode,
            file.path.display(),
            file.description
        ));
    }
    if !plan.cleanup_dirs.is_empty() {
        lines.push(String::new());
        lines.push("Cleanup:".to_string());
        for (i, dir) in plan.cleanup_dirs.iter().enumerate() {
            let status = if dir.exists() {
                "would-retire"
            } else {
                "absent"
            };
            lines.push(format!("  {}. [{}] {}", i + 1, status, dir.display()));
        }
    }
    lines.push(String::new());
    lines.push(
        "Host config policy: MCP snippets only; Claude Code /ags command and Codex AGS command skills are installed on apply."
            .to_string(),
    );
    lines.push("Apply with: ags setup --yes".to_string());
    lines.push(
        "One-command Claude Code initialization: /ags setup (runs setup with Claude MCP registration)"
            .to_string(),
    );
    lines.join("\n")
}
/// Does `dir` look like an AGS-generated Codex command-skill body? True when it
/// has a `SKILL.md` whose front-matter `name` matches the directory and whose
/// body routes through AGS preflight — the shape `codex_ags_command_skill_content`
/// emits. Used to decide whether a retired host entry can be auto-quarantined.
fn is_ags_generated_codex_skill_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(dir.join("SKILL.md")) else {
        return false;
    };
    text.contains(&format!("name: \"{name}\""))
        && text.contains("ags session preflight --for codex")
}

/// Retire a (possibly stale) Codex AGS command-skill host entry safely. This is
/// the cleanup path for `retired_codex_ags_skill_dirs`; it never does a blind
/// `remove_dir_all`:
///   - a thin-index symlink is unlinked only (the canonical body is untouched);
///   - a real directory AGS recognizably generated is moved to a timestamped
///     backup (reversible quarantine), not deleted;
///   - a real entry with unrecognized (possibly user-edited) content is left in
///     place unless `force`, in which case it is also quarantined to a backup.
pub(in crate::setup) fn cleanup_install_dir(
    path: &Path,
    force: bool,
    backup_stamp: u64,
) -> suite_doctor::Finding {
    let id = format!("cleanup-{}", sanitize_name(&path.to_string_lossy()));
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return suite_doctor::Finding::pass(id, format!("absent: {}", path.display()));
    };

    if meta.file_type().is_symlink() {
        return match std::fs::remove_file(path) {
            Ok(()) => suite_doctor::Finding::pass(
                id,
                format!("unlinked thin-index symlink: {}", path.display()),
            ),
            Err(e) => suite_doctor::Finding::fail(
                id,
                format!("unlink failed: {}", path.display()),
                e.to_string(),
            ),
        };
    }

    if !is_ags_generated_codex_skill_dir(path) && !force {
        return suite_doctor::Finding::fail(
            id,
            format!(
                "retired skill entry has unrecognized (possibly user-edited) content: {}",
                path.display()
            ),
            "not modifying it automatically — back it up and remove manually, or rerun `ags setup --yes --force` to quarantine it to a backup",
        );
    }

    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("skill");
    let backup = path.with_file_name(format!("{file_name}.retired.bak.{backup_stamp}"));
    match std::fs::rename(path, &backup) {
        Ok(()) => suite_doctor::Finding::pass(
            id,
            format!(
                "retired (quarantined to backup): {} -> {}",
                path.display(),
                backup.display()
            ),
        ),
        Err(e) => suite_doctor::Finding::fail(
            id,
            format!("retire failed: {}", path.display()),
            e.to_string(),
        ),
    }
}

// ── Global Entry Protocol Templates (setup gate, 五段链路第 1 段) ─────────────
//
// AGS surfaces the AGS-relevant global entry protocol templates as a mandatory
// `ags setup` section so setup can never claim completion without checking them.
// Three classes: AGS-self global kernel (staged under the runtime target,
// confirm-gated by --yes), host global entries (advise-only — AGS never writes
// host config), and project-init entries (owned by `ags init`).
#[cfg(test)]
mod install_plan_tests {
    use super::*;
    use crate::context::AGS_VERSION;
    use crate::setup::templates::{
        claude_ags_command_content, codex_ags_command_skill_agent_metadata_content,
        codex_ags_command_skill_content, codex_ags_command_skill_specs,
    };
    use crate::setup::{
        claude_ags_command_path, codex_ags_named_skill_path, retired_codex_ags_skill_dirs,
    };
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    #[test]
    fn private_install_plan_has_core_files() {
        let target = std::env::temp_dir().join("ags-private-install-plan-default-test");
        let plan = private_install_plan(&workspace_root(), &target);
        assert!(plan
            .files
            .iter()
            .any(|file| file.path == claude_ags_command_path()));
        let manifest = plan
            .files
            .iter()
            .find(|file| file.path.ends_with("install-manifest.json"))
            .expect("manifest file must be generated");
        assert!(manifest.content.contains("\"slash_command\": \"/ags\""));
        assert!(manifest.content.contains("ags-setup"));
        assert!(manifest.content.contains("ags-agents"));
        assert!(manifest.content.contains("ags-init"));
        assert!(manifest.content.contains("ags-skill"));
        assert!(manifest.content.contains(".claude/commands/ags.md"));
        assert!(!manifest.content.contains(".codex/skills/ags/SKILL.md"));
        for (name, _, _, _, _) in codex_ags_command_skill_specs() {
            assert!(plan
                .files
                .iter()
                .any(|file| file.path == codex_ags_named_skill_path(name)));
        }
        for retired_dir in retired_codex_ags_skill_dirs() {
            assert!(plan.cleanup_dirs.iter().any(|dir| dir == &retired_dir));
        }
    }

    /// Codex front-stage command skills are exactly the canonical five
    /// (setup / agents / skill / init / doctor). `ags-capability` must not be a
    /// visible command skill — it is retired into `retired_visible_skills` while
    /// the underlying `ags capability` CLI remains.
    #[test]
    fn codex_visible_command_skills_are_exactly_the_canonical_five() {
        let target = std::env::temp_dir().join("ags-public-install-plan-five-set-test");
        let plan = private_install_plan(&workspace_root(), &target);

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

        let manifest = plan
            .files
            .iter()
            .find(|file| file.path.ends_with("install-manifest.json"))
            .expect("manifest file must be generated");
        let json: serde_json::Value =
            serde_json::from_str(&manifest.content).expect("manifest is valid JSON");
        let command_skills = json["host_commands"]["codex"]["command_skills"]
            .as_array()
            .expect("codex command_skills array");
        let command_skill_text = command_skills
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "ags-setup",
            "ags-agents",
            "ags-skill",
            "ags-init",
            "ags-doctor",
        ] {
            assert!(
                command_skill_text.contains(expected),
                "manifest command_skills missing {expected}: {command_skill_text}"
            );
        }
        assert!(
            !command_skill_text.contains("ags-capability"),
            "manifest command_skills must exclude ags-capability: {command_skill_text}"
        );

        let retired_skills = json["host_commands"]["codex"]["retired_visible_skills"]
            .as_array()
            .expect("codex retired_visible_skills array");
        let retired_text = retired_skills
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            retired_text.contains("ags-capability"),
            "ags-capability must be in retired_visible_skills: {retired_text}"
        );
    }

    #[test]
    fn tencent_agent_host_snippets_register_ags_mcp() {
        // Tencent Agent / WorkBuddy / CodeBuddy-Code are platform-host MCP
        // integration snippets. They register AGS MCP only; they do not create
        // runtime adapters or change execution-policy authority.
        let target = std::env::temp_dir().join("ags-tencent-snippet-struct-test");
        let plan = private_install_plan(&workspace_root(), &target);
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
        assert!(content.contains("MCP `ags_route_request`"));
        assert!(content.contains("structured `RequestDecision`"));
        assert!(content.contains("decision selects task compilation"));
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
        for (_, display_name, short_description, default_prompt, _) in
            codex_ags_command_skill_specs()
        {
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

    #[cfg(unix)]
    #[test]
    fn cleanup_retire_unlinks_symlink_without_touching_canonical() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("ags-cleanup-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let canonical = base.join("canonical");
        let link = base.join("ags-capability");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(canonical.join("SKILL.md"), "canonical").unwrap();
        symlink(&canonical, &link).unwrap();

        let finding = cleanup_install_dir(&link, false, 123);
        assert_eq!(
            finding.status,
            suite_doctor::CheckStatus::Pass,
            "{finding:?}"
        );
        assert!(!link.exists(), "symlink should be unlinked");
        assert!(
            canonical.join("SKILL.md").exists(),
            "canonical body must not be touched"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_retire_quarantines_ags_generated_dir_reversibly() {
        let base =
            std::env::temp_dir().join(format!("ags-cleanup-generated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("ags-capability");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: \"ags-capability\"\n---\nags session preflight --for codex --target .\n",
        )
        .unwrap();

        let finding = cleanup_install_dir(&dir, false, 456);
        assert_eq!(
            finding.status,
            suite_doctor::CheckStatus::Pass,
            "{finding:?}"
        );
        assert!(!dir.exists(), "original dir should be moved");
        assert!(
            base.join("ags-capability.retired.bak.456")
                .join("SKILL.md")
                .exists(),
            "backup quarantine should keep contents"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_retire_refuses_unrecognized_content_without_force() {
        let base = std::env::temp_dir().join(format!("ags-cleanup-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("ags-capability");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "user edited content").unwrap();

        let finding = cleanup_install_dir(&dir, false, 789);
        assert_eq!(
            finding.status,
            suite_doctor::CheckStatus::Fail,
            "{finding:?}"
        );
        assert!(dir.join("SKILL.md").exists(), "user content must remain");
        assert!(
            !base.join("ags-capability.retired.bak.789").exists(),
            "no backup should be created without force"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_retire_absent_is_pass() {
        let path = std::env::temp_dir().join(format!("ags-cleanup-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let finding = cleanup_install_dir(&path, false, 1);
        assert_eq!(
            finding.status,
            suite_doctor::CheckStatus::Pass,
            "{finding:?}"
        );
    }
}
