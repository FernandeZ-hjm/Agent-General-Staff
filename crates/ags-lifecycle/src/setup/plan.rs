use super::apply::codex_skill_thin_index_ancestor;
use super::templates::{
    claude_ags_command_content, codex_ags_command_skill_agent_metadata_content,
    codex_ags_command_skill_content, codex_ags_command_skill_specs, host_entry_policy_content,
};
use super::{
    claude_ags_command_path, codex_ags_named_skill_agent_metadata_path, codex_ags_named_skill_path,
    retired_codex_ags_skill_dirs, PRIVATE_INSTALL_SCHEMA,
};
use super::{portable_validate_script, project_protocol_files};
use super::{sanitize_name, shell_quote, InstallFile};
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
    home: &Path,
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
                "path": claude_ags_command_path(home).to_string_lossy().replace('\\', "/")
            },
            "codex": {
                "command_skills": codex_ags_command_skill_specs()
                    .iter()
                    .map(|(name, _, _, _, _)| codex_ags_named_skill_path(home, name).to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>(),
                "retired_visible_skills": retired_codex_ags_skill_dirs(home)
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
            path: target.join("hosts/host-entry-policy.md"),
            description: "canonical AGS 0.3 host entry and OMP Plan single-card policy".to_string(),
            content: host_entry_policy_content(),
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
            path: claude_ags_command_path(home),
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
    files.extend(super::memory::memory_script_install_files(
        &home.to_path_buf(),
    ));

    // AGS-owned global rule modules. Host-global AGENTS.md / CLAUDE.md stay
    // operator-controlled and may reference these concise, stable modules.
    for (name, description) in [
        ("ags-core.md", "AGS concise global core rules"),
        (
            "ags-task-handoff.md",
            "AGS task-card and Plan handoff rules",
        ),
        (
            "host-operations.md",
            "AGS remote, GUI, install, and temporary-file rules",
        ),
    ] {
        if let Ok(content) =
            std::fs::read_to_string(source_root.join("templates/global-entry").join(name))
        {
            files.push(InstallFile {
                path: home.to_path_buf().join(".agents/rules").join(name),
                description: description.to_string(),
                content,
                mode: None,
            });
        }
    }

    for (name, display_name, short_description, default_prompt, summary) in
        codex_ags_command_skill_specs()
    {
        files.push(InstallFile {
            path: codex_ags_named_skill_path(home, name),
            description: format!("Codex AGS command skill: {name}"),
            content: codex_ags_command_skill_content(name, display_name, summary),
            mode: None,
        });
        files.push(InstallFile {
            path: codex_ags_named_skill_agent_metadata_path(home, name),
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
        cleanup_dirs: retired_codex_ags_skill_dirs(home),
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
                "status": if path.exists() { "would-remove" } else { "absent" },
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
                "would-remove"
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

/// Retire a (possibly stale) Codex AGS command-skill host entry SAFELY. This is
/// the cleanup path for `retired_codex_ags_skill_dirs`; it never does a blind
/// `remove_dir_all`:
///   - a thin-index symlink is unlinked only (the canonical body is untouched);
///   - a real directory AGS recognizably generated is MOVED to a timestamped
///     backup (reversible quarantine), not deleted;
///   - a real entry with unrecognized (possibly user-edited) content is left in
///     place unless `force`, in which case it is also quarantined to a backup.
///
/// Nothing is ever irreversibly deleted.
pub(in crate::setup) fn cleanup_install_dir(
    path: &Path,
    force: bool,
    backup_stamp: u64,
) -> crate::setup::SetupFinding {
    let id = format!("cleanup-{}", sanitize_name(&path.to_string_lossy()));
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return crate::setup::SetupFinding::pass(id, format!("absent: {}", path.display()));
    };

    // Thin-index symlink: unlink only — removing the link never touches the
    // canonical body it points at (also clears a dangling symlink).
    if meta.file_type().is_symlink() {
        return match std::fs::remove_file(path) {
            Ok(()) => crate::setup::SetupFinding::pass(
                id,
                format!("unlinked thin-index symlink: {}", path.display()),
            ),
            Err(e) => crate::setup::SetupFinding::fail(
                id,
                format!("unlink failed: {}", path.display()),
                e.to_string(),
            ),
        };
    }

    // Real entry with unrecognized content and no --force → do not touch it.
    if !is_ags_generated_codex_skill_dir(path) && !force {
        return crate::setup::SetupFinding::fail(
            id,
            format!(
                "retired skill entry has unrecognized (possibly user-edited) content: {}",
                path.display()
            ),
            "not modifying it automatically — back it up and remove manually, or rerun `ags setup --yes --force` to quarantine it to a backup",
        );
    }

    // Quarantine to a timestamped backup instead of deleting (reversible).
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("skill");
    let backup = path.with_file_name(format!("{file_name}.retired.bak.{backup_stamp}"));
    match std::fs::rename(path, &backup) {
        Ok(()) => crate::setup::SetupFinding::pass(
            id,
            format!(
                "retired (quarantined to backup): {} -> {}",
                path.display(),
                backup.display()
            ),
        ),
        Err(e) => crate::setup::SetupFinding::fail(
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
mod tests;
