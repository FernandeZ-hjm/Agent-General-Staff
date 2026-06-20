use super::{
    claude_ags_command_path, codex_ags_named_skill_agent_metadata_path, codex_ags_named_skill_path,
    retired_codex_ags_skill_dirs, PRIVATE_INSTALL_SCHEMA,
};
use crate::context::{sanitize_name, shell_quote};
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
    capability_route_mode: capability_route::EnrollmentMode,
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

    let profile = std::fs::read_to_string(
        source_root.join("manifests/templates/runtime-profiles.template.yaml"),
    )
    .unwrap_or_default()
    .replace("http://127.0.0.1:PORT", "http://127.0.0.1:19821");

    let claude_hook = std::fs::read_to_string(
        source_root.join("manifests/templates/hooks/claude-code-executor-stop.template.js"),
    )
    .unwrap_or_default();
    let codex_hook = std::fs::read_to_string(
        source_root.join("manifests/templates/hooks/codex-planner-recall.template.json"),
    )
    .unwrap_or_default();

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
`ags setup --yes` installs visible top-level command skills: `$ags-setup`, `$ags-init`, `$ags-skill`, `$ags-capability`, and `$ags-doctor`.\n\
Retired visible skills (`$ags`, `$ags-preflight`, `$ags-verify`) are removed from the Codex skill list during setup.\n\
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
            path: capability_route::enrollment_file_path(target),
            description: format!(
                "machine-local Capability Route enrollment evidence (mode={}); never a tracked manifest, no credential",
                capability_route_mode.as_str()
            ),
            content: capability_route::render_enrollment_json(capability_route_mode, "ags setup"),
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

// ── Capability Route Enrollment (setup wizard, 五段链路第 1 段) ────────────────
//
// AGS surfaces a plan-only enrollment choice: which managed capabilities this
// machine opts into Capability Route. The choice is machine-local runtime
// evidence written to `<target>/capability-route/enrollment.json` on `--yes` — it
// is never a tracked manifest, and AGS never auto-installs, registers, logs in,
// or reads credentials for any capability. Non-interactive: the mode comes from
// the `--capability-route` flag (default suite-only); CI never blocks on stdin.

pub(in crate::setup) fn capability_route_enrollment_json(
    mode: capability_route::EnrollmentMode,
    target: &Path,
) -> serde_json::Value {
    let modes: Vec<serde_json::Value> = capability_route::EnrollmentMode::all()
        .iter()
        .map(|m| {
            serde_json::json!({
                "mode": m.as_str(),
                "routes": m.description(),
                "selected": *m == mode,
                "default": *m == capability_route::EnrollmentMode::SuiteOnly,
            })
        })
        .collect();
    // Routing reads enrollment from the runtime home it resolves itself
    // (AGS_RUNTIME_HOME → AGS_HOME → ~/.ags/runtime). Warn when this setup
    // target is NOT that path: the evidence would be written somewhere routing
    // won't read (fail-closed advisory degraded, never unsafe).
    let routing_read_home = capability_route::locate_runtime_home();
    let mut obj = serde_json::json!({
        "selected_mode": mode.as_str(),
        "default_mode": capability_route::EnrollmentMode::SuiteOnly.as_str(),
        "evidence_path": capability_route::enrollment_file_path(target).to_string_lossy(),
        "routing_read_path": capability_route::enrollment_file_path(&routing_read_home).to_string_lossy(),
        "schema_version": capability_route::ENROLLMENT_SCHEMA,
        "write_mode": "plan-only (writes on --yes)",
        "modes": modes,
        "boundary": "AGS records the routing-membership choice only. It never auto-installs, registers, logs in, or reads credentials for any capability. auth_status is runtime-derived and is never written here as configured.",
        "fail_closed": "Missing or malformed evidence resolves to off (advisory degraded); Capability Route never blocks the user request or changes any gate.",
        "set_with": "ags setup --capability-route <off|suite-only|adopted|review-all> --yes",
    });
    if target != routing_read_home {
        obj["target_routing_note"] = serde_json::json!(
            "This --target differs from the runtime home routing reads (AGS_RUNTIME_HOME → AGS_HOME → ~/.ags/runtime). Routing will not see enrollment written here unless AGS_RUNTIME_HOME (or AGS_HOME) points at this target. Fail-closed: routing degrades to advisory, never blocks."
        );
    }
    obj
}

pub(in crate::setup) fn render_capability_route_enrollment_text(
    mode: capability_route::EnrollmentMode,
    target: &Path,
) -> String {
    let mut lines = vec![
        "Capability Route Enrollment (五段链路第 1 段 · machine-local runtime evidence)"
            .to_string(),
        format!(
            "  Selected mode: {} (default: {})",
            mode.as_str(),
            capability_route::EnrollmentMode::SuiteOnly.as_str()
        ),
        format!(
            "  Evidence file: {}  [plan-only; writes on --yes]",
            capability_route::enrollment_file_path(target).display()
        ),
        "  Modes:".to_string(),
    ];
    for m in capability_route::EnrollmentMode::all() {
        let marker = if m == mode { "→" } else { " " };
        lines.push(format!(
            "    {} {:<11} {}",
            marker,
            m.as_str(),
            m.description()
        ));
    }
    lines.push(
        "  Boundary: AGS records the routing-membership choice only — never auto-installs,"
            .to_string(),
    );
    lines.push(
        "            registers, logs in, or reads credentials. auth_status is runtime-derived,"
            .to_string(),
    );
    lines.push("            never written here as configured.".to_string());
    lines.push(
        "  Fail-closed: missing/malformed evidence ⇒ off (advisory degraded); never blocks."
            .to_string(),
    );
    let routing_read_home = capability_route::locate_runtime_home();
    if target != routing_read_home {
        lines.push(format!(
            "  NOTE: routing reads enrollment from {} (AGS_RUNTIME_HOME → AGS_HOME → default);",
            capability_route::enrollment_file_path(&routing_read_home).display()
        ));
        lines.push(
            "        this --target differs, so routing won't see evidence written here unless"
                .to_string(),
        );
        lines.push(
            "        AGS_RUNTIME_HOME/AGS_HOME points at it (fail-closed advisory).".to_string(),
        );
    }
    lines.push(
        "  Set with: ags setup --capability-route <off|suite-only|adopted|review-all> --yes"
            .to_string(),
    );
    lines.join("\n")
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
    lines.push(
        "One-command Claude Code initialization: /ags setup (runs setup with Claude MCP registration)"
            .to_string(),
    );
    lines.join("\n")
}
pub(in crate::setup) fn cleanup_install_dir(path: &Path) -> suite_doctor::Finding {
    if !path.exists() {
        return suite_doctor::Finding::pass(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("absent: {}", path.display()),
        );
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => suite_doctor::Finding::pass(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("removed: {}", path.display()),
        ),
        Err(e) => suite_doctor::Finding::fail(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("remove failed: {}", path.display()),
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
        let plan = private_install_plan(
            &workspace_root(),
            &target,
            capability_route::EnrollmentMode::SuiteOnly,
        );
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
        assert!(manifest.content.contains("ags-init"));
        assert!(manifest.content.contains("ags-skill"));
        assert!(manifest.content.contains("ags-capability"));
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

    #[test]
    fn tencent_agent_host_snippets_register_ags_mcp() {
        // Tencent Agent / WorkBuddy / CodeBuddy-Code are platform-host MCP
        // integration snippets. They register AGS MCP only; they do not create
        // runtime adapters or change execution-policy authority.
        let target = std::env::temp_dir().join("ags-tencent-snippet-struct-test");
        let plan = private_install_plan(
            &workspace_root(),
            &target,
            capability_route::EnrollmentMode::SuiteOnly,
        );
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
}
