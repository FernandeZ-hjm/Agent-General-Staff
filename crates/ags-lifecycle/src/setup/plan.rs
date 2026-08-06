use super::apply::codex_skill_thin_index_ancestor;
use super::project_protocol_files;
use super::templates::{
    claude_ags_command_content, codex_ags_command_skill_specs, host_entry_policy_content,
};
use super::{
    claude_ags_command_path, codex_ags_named_skill_path, retired_ags_memory_script_paths,
    retired_codex_ags_skill_dirs, RUNTIME_INSTALL_SCHEMA,
};
use super::{sanitize_name, shell_quote, InstallFile};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(in crate::setup) struct RuntimeInstallPlan {
    pub(crate) profile: String,
    pub(crate) source_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) approved_lifecycle_hosts: Vec<String>,
    pub(crate) files: Vec<InstallFile>,
    pub(crate) cleanup_paths: Vec<PathBuf>,
    pub(crate) suite_skill_projection:
        Result<crate::suite_skill_projection::PreparedSuiteSkillProjection, String>,
}
pub(in crate::setup) fn runtime_install_plan(
    source_root: &Path,
    target: &Path,
    home: &Path,
) -> RuntimeInstallPlan {
    let approved = super::approved_lifecycle_hosts(target).unwrap_or_default();
    let selection_source = super::lifecycle_selection_source(target);
    runtime_install_plan_with_hosts(source_root, target, home, &approved, &selection_source)
}

pub(in crate::setup) fn runtime_install_plan_with_hosts(
    source_root: &Path,
    target: &Path,
    home: &Path,
    approved_lifecycle_hosts: &[String],
    lifecycle_selection_source: &str,
) -> RuntimeInstallPlan {
    match super::configured_suite_skill_authority_root(target) {
        Ok(authority) => runtime_install_plan_with_hosts_and_authority(
            source_root,
            target,
            home,
            approved_lifecycle_hosts,
            lifecycle_selection_source,
            authority.as_deref(),
        ),
        Err(error) => {
            let mut plan = runtime_install_plan_with_hosts_and_authority(
                source_root,
                target,
                home,
                approved_lifecycle_hosts,
                lifecycle_selection_source,
                None,
            );
            plan.suite_skill_projection = Err(error);
            plan
        }
    }
}

pub(in crate::setup) fn runtime_install_plan_with_hosts_and_authority(
    source_root: &Path,
    target: &Path,
    home: &Path,
    approved_lifecycle_hosts: &[String],
    lifecycle_selection_source: &str,
    suite_skill_authority_root: Option<&Path>,
) -> RuntimeInstallPlan {
    let serialized_target = serde_json::to_string(&target.to_string_lossy()).unwrap_or_default();
    let config_target = serialized_target
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or_default();
    let ags_mcp_json = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "ags": {
                "command": "ags",
                "args": ["mcp", "serve", "--transport", "stdio"],
                "env": {
                    "AGS_RUNTIME_HOME": target.to_string_lossy()
                }
            }
        },
        "initialization_gate": {
            "mandatory_first_tool": "ags_preflight",
            "failed_preflight_opens_gate": false
        }
    }))
    .unwrap_or_default()
        + "\n";

    let codex_snippet = r#"# AGS MCP host initialization adapter
# Merge this snippet into ~/.codex/config.toml after review.
[mcp_servers.ags]
command = "ags"
args = ["mcp", "serve", "--transport", "stdio"]

[mcp_servers.ags.env]
AGS_RUNTIME_HOME = "__TARGET__"

"#
    .replace("__TARGET__", config_target);

    let claude_snippet = r#"{
  "mcpServers": {
    "ags": {
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "AGS_RUNTIME_HOME": "__TARGET__"
      }
    }
  }
}
"#
    .replace("__TARGET__", config_target);

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

    let launcher = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport AGS_RUNTIME_HOME={}\nexec ags mcp serve --transport stdio\n",
        shell_quote(target)
    );

    let claude_selected = approved_lifecycle_hosts
        .iter()
        .any(|host| host == "claude-code");
    let codex_selected = approved_lifecycle_hosts.iter().any(|host| host == "codex");
    let mut host_commands = serde_json::Map::new();
    if claude_selected {
        host_commands.insert(
            "claude_code".to_string(),
            serde_json::json!({
                "slash_command": "/ags",
                "path": claude_ags_command_path(home).to_string_lossy()
            }),
        );
    }
    if codex_selected {
        host_commands.insert(
            "codex".to_string(),
            serde_json::json!({
                "command_skills": codex_ags_command_skill_specs()
                    .iter()
                    .map(|(name, _, _, _, _)| codex_ags_named_skill_path(home, name).to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
                "retired_visible_skills": retired_codex_ags_skill_dirs(home)
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            }),
        );
    }

    let manifest = serde_json::json!({
        "schema_version": RUNTIME_INSTALL_SCHEMA,
        "producer_version": env!("CARGO_PKG_VERSION"),
        "profile": "runtime",
        "source_root": source_root.to_string_lossy(),
        "target": target.to_string_lossy(),
        "mcp": {
            "server": "ags",
            "command": "ags mcp serve --transport stdio",
            "mandatory_first_tool": "ags_preflight"
        },
        "lifecycle": {
            "approved_hosts": approved_lifecycle_hosts,
            "selection_source": lifecycle_selection_source
        },
        "suite_skill_projection": {
            "mode": if suite_skill_authority_root.is_some() { "required_authority_root" } else { "suite_source_root" },
            "required_authority_root": suite_skill_authority_root.map(|root| root.to_string_lossy().to_string()),
            "hosts": approved_lifecycle_hosts,
        },
        "host_snippets": serde_json::json!([
            "hosts/codex.config.snippet.toml",
            "hosts/claude-code.mcp.snippet.json",
            "hosts/tencent-agent.mcp.snippet.json",
            "hosts/workbuddy.mcp.snippet.json",
            "hosts/codebuddy-code.mcp.snippet.json"
        ]),
        "host_commands": host_commands,
        "created_by": "ags setup",
    });

    let readme = format!(
        "# AGS Runtime\n\n\
This directory was generated by `ags setup`.\n\n\
## Commands\n\n\
- MCP server: `ags mcp serve --transport stdio`\n\
- Doctor: `ags doctor`\n\
- Runtime check: `ags doctor --target {}`\n\n\
## Host snippets\n\n\
Review files in `hosts/` before merging them into host-specific global config.\n\
AGS scenarios must call `ags_preflight` before any other AGS tool.\n\n\
## Selected Hosts\n\n\
`ags setup --yes` projects required suite command Skills only to the approved Host set recorded in `install-manifest.json`.\n\
Run setup again with `--lifecycle-hosts` to add or remove Host projections. A completed setup always retains at least one selected Host.\n\n\
",
        target.display()
    );

    let mut files = vec![
        InstallFile {
            path: target.join("install-manifest.json"),
            description: "machine-readable runtime install manifest".to_string(),
            content: serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n",
            mode: None,
        },
        InstallFile {
            path: target.join("README.md"),
            description: "operator notes for this runtime home".to_string(),
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
            description: "Claude Code MCP registration snippet".to_string(),
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
            path: target.join("bin/ags-mcp-stdio.sh"),
            description: "portable launcher for AGS MCP stdio server".to_string(),
            content: launcher,
            mode: Some(0o755),
        },
    ];
    if claude_selected {
        files.push(InstallFile {
            path: claude_ags_command_path(home),
            description: "Claude Code user slash command for AGS governance".to_string(),
            content: claude_ags_command_content(),
            mode: None,
        });
    }

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
                path: home.join(".agents/rules").join(name),
                description: description.to_string(),
                content,
                mode: None,
            });
        }
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

    let mut suite_skill_projection =
        crate::suite_skill_projection::plan_required_suite_skill_projection(
            source_root,
            target,
            home,
            &crate::suite_skill_projection::SuiteSkillProjectionPolicy {
                required_authority_root: suite_skill_authority_root.map(Path::to_path_buf),
                target_hosts: approved_lifecycle_hosts.to_vec(),
            },
        );

    match super::global_entry::planned_ags_global_entry(target) {
        Ok(content) => files.push(InstallFile {
            path: target.join("ags-global-entry.md"),
            description: "AGS managed global entry block".to_string(),
            content,
            mode: None,
        }),
        Err(error) => suite_skill_projection = Err(error),
    }

    let mut cleanup_paths = retired_codex_ags_skill_dirs(home)
        .into_iter()
        .chain(retired_ags_memory_script_paths(home))
        .collect::<Vec<_>>();
    let previous_hosts = super::approved_lifecycle_hosts(target).unwrap_or_default();
    if previous_hosts.iter().any(|host| host == "claude-code") && !claude_selected {
        cleanup_paths.push(claude_ags_command_path(home));
    }

    RuntimeInstallPlan {
        profile: "runtime".to_string(),
        source_root: source_root.to_path_buf(),
        target: target.to_path_buf(),
        approved_lifecycle_hosts: approved_lifecycle_hosts.to_vec(),
        files,
        cleanup_paths,
        suite_skill_projection,
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
pub(in crate::setup) fn render_runtime_plan_json(plan: &RuntimeInstallPlan) -> String {
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
    let cleanup_paths: Vec<_> = plan
        .cleanup_paths
        .iter()
        .map(|path| {
            serde_json::json!({
                "path": path.to_string_lossy(),
                "status": if path.exists() { "would-remove" } else { "absent" },
            })
        })
        .collect();
    let suite_skill_projection = match &plan.suite_skill_projection {
        Ok(projection) => serde_json::to_value(projection)
            .unwrap_or_else(|error| serde_json::json!({"error": error.to_string()})),
        Err(error) => serde_json::json!({"error": error}),
    };

    let output = serde_json::json!({
        "schema_version": RUNTIME_INSTALL_SCHEMA,
        "profile": plan.profile,
        "source_root": plan.source_root.to_string_lossy(),
        "target": plan.target.to_string_lossy(),
        "write_mode": "plan-only",
        "approved_lifecycle_hosts": plan.approved_lifecycle_hosts,
        "files": files,
        "cleanup_paths": cleanup_paths,
        "suite_skill_projection": suite_skill_projection,
        "host_config_policy": "MCP snippets are generated only; Claude Code /ags command and Codex AGS command skills are installed on apply",
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}
pub(in crate::setup) fn render_runtime_plan_text(plan: &RuntimeInstallPlan) -> String {
    let mut lines = vec![
        format!("AGS Runtime Install Plan {}", RUNTIME_INSTALL_SCHEMA),
        format!("Profile: {}", plan.profile),
        format!("Source:  {}", plan.source_root.display()),
        format!("Target:  {}", plan.target.display()),
        format!(
            "Lifecycle hosts: {}",
            if plan.approved_lifecycle_hosts.is_empty() {
                "none".to_string()
            } else {
                plan.approved_lifecycle_hosts.join(", ")
            }
        ),
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
    if !plan.cleanup_paths.is_empty() {
        lines.push(String::new());
        lines.push("Cleanup:".to_string());
        for (i, path) in plan.cleanup_paths.iter().enumerate() {
            let status = if path.exists() {
                "would-remove"
            } else {
                "absent"
            };
            lines.push(format!("  {}. [{}] {}", i + 1, status, path.display()));
        }
    }
    lines.push(String::new());
    lines.push("Required suite Skill projection:".to_string());
    match &plan.suite_skill_projection {
        Ok(projection) => {
            lines.push(format!(
                "  authority: {}",
                projection.authority_root.display()
            ));
            lines.push(format!("  hosts: {}", projection.hosts.join(", ")));
            lines.push(format!("  operations: {}", projection.operations.len()));
            for finding in &projection.blocking_findings {
                lines.push(format!("  BLOCKED: {finding}"));
            }
        }
        Err(error) => lines.push(format!("  BLOCKED: {error}")),
    }
    lines.push(String::new());
    lines.push(
        "Host config policy: MCP snippets only; required suite Skills are projected only to the approved Host set on apply."
            .to_string(),
    );
    lines.push("Apply with: ags setup --yes".to_string());
    lines.join("\n")
}
/// Does `dir` look like an AGS-generated Codex command-skill body? True when it
/// has a `SKILL.md` whose front-matter `name` matches the directory and whose
/// body routes through AGS preflight. Used to decide whether a retired host
/// entry can be auto-quarantined.
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
///   - a real directory AGS recognizably generated is removed;
///   - a real entry with unrecognized (possibly user-edited) content is left in
///     place unless `force`, in which case it is removed.
///
pub(crate) fn cleanup_install_entry(path: &Path, force: bool) -> crate::setup::SetupFinding {
    let id = format!("cleanup-{}", sanitize_name(&path.to_string_lossy()));
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return crate::setup::SetupFinding::pass(id, format!("absent: {}", path.display()));
    };

    // A retired thin-index link is removable only when its raw target has the
    // exact AGS suite shape `.../global-skills/<same-id>`. Merely being a
    // symlink is not ownership proof.
    if meta.file_type().is_symlink() {
        let owned = std::fs::read_link(path)
            .ok()
            .is_some_and(|target| retired_symlink_has_suite_identity(path, &target));
        if !owned {
            return crate::setup::SetupFinding::fail(
                id,
                format!(
                    "retired skill symlink is not recognizably AGS-owned: {}",
                    path.display()
                ),
                "leaving it untouched; inspect the target and remove or rebind it explicitly",
            );
        }
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
    if !is_ags_generated_codex_skill_dir(path)
        && !is_ags_generated_claude_command_file(path)
        && !force
    {
        return crate::setup::SetupFinding::fail(
            id,
            format!(
                "retired skill entry has unrecognized (possibly user-edited) content: {}",
                path.display()
            ),
            "not modifying it automatically — remove it manually, or rerun `ags setup --yes --force` to delete it",
        );
    }

    match if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    } {
        Ok(()) => crate::setup::SetupFinding::pass(
            id,
            format!("removed retired AGS entry: {}", path.display()),
        ),
        Err(e) => crate::setup::SetupFinding::fail(
            id,
            format!("retire failed: {}", path.display()),
            e.to_string(),
        ),
    }
}

pub(crate) fn is_ags_generated_claude_command_file(path: &Path) -> bool {
    let suffix = Path::new(".claude/commands/ags.md");
    path.ends_with(suffix)
        && std::fs::read_to_string(path).is_ok_and(|content| {
            content.contains("description: AGS one-command setup")
                && content.contains("Current AGS version expected by this command:")
        })
}

fn retired_symlink_has_suite_identity(link: &Path, raw_target: &Path) -> bool {
    let Some(skill_id) = link.file_name() else {
        return false;
    };
    let target = if raw_target.is_absolute() {
        raw_target.to_path_buf()
    } else {
        link.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(raw_target)
    };
    target.file_name() == Some(skill_id)
        && target
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "global-skills")
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
