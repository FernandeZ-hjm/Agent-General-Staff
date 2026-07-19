//! `ags init` lifecycle (五段链路第 4 段).

mod overlay;

use crate::context::{
    default_private_runtime_home, guard_path, home_dir, sanitize_name, unix_timestamp, AGS_VERSION,
};
use crate::file_plan::InstallFile;
use crate::init::overlay::{
    apply_overlay, compute_overlay_plan, overlay_json, render_overlay_text, OverlayMode,
};
use crate::output::yaml_string;
use crate::project_templates::{portable_validate_script, project_protocol_files};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::{Path, PathBuf};

const PROJECT_INIT_SCHEMA: &str = "2.4-project-init";
const AGS_MANAGED_BEGIN: &str = "<!-- BEGIN AGS MANAGED BLOCK -->";
const AGS_MANAGED_END: &str = "<!-- END AGS MANAGED BLOCK -->";

fn managed_block_text(desired: &str) -> String {
    format!("{AGS_MANAGED_BEGIN}\n{}\n{AGS_MANAGED_END}", desired.trim())
}

/// Replace only the AGS-owned section of a user-owned entry file. Legacy
/// unmarked sections are migrated once; ambiguous project-owned sections fail
/// closed instead of being overwritten.
fn merge_managed_project_block(existing: &str, desired: &str) -> Result<String, String> {
    let replacement = managed_block_text(desired);
    if let Some(begin) = existing.find(AGS_MANAGED_BEGIN) {
        let Some(end_rel) = existing[begin..].find(AGS_MANAGED_END) else {
            return Err("AGS managed block begin marker has no end marker".to_string());
        };
        let end = begin + end_rel + AGS_MANAGED_END.len();
        return Ok(format!(
            "{}{}{}",
            &existing[..begin],
            replacement,
            &existing[end..]
        ));
    }

    let heading = "## Agent Governance Suite";
    if let Some(begin) = existing.find(heading) {
        let section_tail = &existing[begin + heading.len()..];
        let end = section_tail
            .find("\n## ")
            .map(|offset| begin + heading.len() + offset)
            .unwrap_or(existing.len());
        let legacy = &existing[begin..end];
        if !legacy.contains("This project is governed by AGS")
            && !legacy.contains("This project is governed by Agent Governance Suite")
        {
            return Err("existing Agent Governance Suite section is not AGS-managed".to_string());
        }
        return Ok(format!(
            "{}{}{}",
            &existing[..begin],
            replacement,
            &existing[end..]
        ));
    }

    let separator = if existing.is_empty() || existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!("{existing}{separator}{replacement}\n"))
}
fn default_project_slug(target: &Path) -> String {
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("project");
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}
fn project_memory_dir(slug: &str) -> PathBuf {
    home_dir()
        .join(".agents")
        .join("memory")
        .join("projects")
        .join(slug)
}
fn project_template_protocol_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let suite_protocol = cwd.join("protocol");
    if suite_protocol.join("agent-task-protocol.md").exists() {
        return Some(suite_protocol);
    }

    if let Some(runtime_home) = std::env::var_os("AGS_RUNTIME_HOME").map(PathBuf::from) {
        let dir = runtime_home.join("project-templates/protocol");
        if dir.join("agent-task-protocol.md").exists() {
            return Some(dir);
        }
    }

    let dir = default_private_runtime_home().join("project-templates/protocol");
    if dir.join("agent-task-protocol.md").exists() {
        Some(dir)
    } else {
        None
    }
}
#[derive(Debug, Clone)]
struct ProjectInitPlan {
    target: PathBuf,
    slug: String,
    memory_dir: PathBuf,
    files: Vec<InstallFile>,
    append_files: Vec<InstallFile>,
    directories: Vec<PathBuf>,
    warnings: Vec<String>,
}
fn project_init_plan_with_protocol(
    target: &Path,
    slug: Option<String>,
    protocol_dir: Option<PathBuf>,
) -> ProjectInitPlan {
    let canonical = guard_path(target);
    let slug = slug.unwrap_or_else(|| default_project_slug(&canonical));
    let memory_dir = project_memory_dir(&slug);
    let mut files = Vec::new();
    let mut append_files = Vec::new();
    let mut directories = vec![
        canonical.join("config"),
        canonical.join("protocol"),
        canonical.join("scripts"),
        memory_dir.join("task-archive"),
        memory_dir.join("sessions"),
    ];
    let mut warnings = Vec::new();

    let ags_block = format!(
        "\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}.\n\n- Run `ags doctor --target .` to diagnose local governance health.\n- AGS MCP hosts must call `ags_preflight` before other AGS tools.\n- CLI fallback: `ags session preflight --for <agent-id> --target .`.\n- Read `ags://capabilities/current-host`; the host uses complete context to create a typed `HostRouteProposal`.\n- `ags_route_request` is strictly read-only; it validates exclusive DirectResponse or at most one exact SkillTarget plus one MachineCliTarget.\n- `ags_apply_action` is the only effectful MCP tool and consumes a connection-bound lease/action reference.\n- Skill Resolver, Compiler, Policy, Gate, and Runner never parse natural language.\n- Task-card generation requires both an explicit handoff request and a confirmed handoff contract.\n- Existing `## 任务卡` input validates before policy/gate/LaunchPlan.\n- Runner returns `HOST_EXECUTION_REQUIRED`; it does not execute or verify the task.\n- Task-card permission has exactly two modes: `plan-only` and `execute-and-verify`; Heavy adds an independent review gate.\n- Protocol entry points: `AGENT_SUITE_PROTOCOL.md`, `protocol/agent-task-protocol.md`, `protocol/task-routing.md`.\n"
    );

    files.push(InstallFile {
        path: canonical.join("AGENTS.md"),
        description: "agent entrypoint with AGS governance reference".to_string(),
        content: format!("# AGENTS.md\n\n@CLAUDE.md\n{ags_block}"),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join("AGENTS.md"),
        description: "append AGS governance block to existing AGENTS.md".to_string(),
        content: ags_block.clone(),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("CLAUDE.md"),
        description: "Claude Code AGS execution protocol entrypoint".to_string(),
        content: format!(
            "# CLAUDE.md\n\nThis project is governed by Agent Governance Suite {AGS_VERSION}.\n\nBefore task execution, call MCP `ags_preflight` or use `ags session preflight --for claude-code --target .`. Read `ags://capabilities/current-host`, use complete conversation context to create a typed HostRouteProposal, and submit it to strictly read-only `ags_route_request`. DirectResponse is exclusive; one exact SkillTarget and one MachineCliTarget may coexist. Only `ags_apply_action` consumes a connection-held action. Skill Resolver, Compiler, Policy, Gate, and Runner do not parse raw language. Existing `## 任务卡` input validates before policy/gate/LaunchPlan. New task-card generation requires an explicit handoff request plus a confirmed handoff contract. Follow `protocol/agent-task-protocol.md`.\n"
        ),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join("CLAUDE.md"),
        description: "append AGS execution protocol block to existing CLAUDE.md".to_string(),
        content: format!("\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}. Run `ags_preflight` before execution, read `ags://capabilities/current-host`, and let the host create a typed HostRouteProposal from complete context. `ags_route_request` is strictly read-only; only `ags_apply_action` consumes connection-held fixed actions. DirectResponse is exclusive; an exact SkillTarget and MachineCliTarget may coexist. Skill Resolver, Compiler, Policy, Gate, and Runner consume structured input only. Existing task cards validate before LaunchPlan; new task-card generation requires explicit handoff intent plus a confirmed contract. Follow `protocol/agent-task-protocol.md`.\n"),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "ignore machine-local AGS runtime data".to_string(),
        content: "# Machine-local AGS runtime data\n/capability-snapshot/\n/skill-registry/\n/skill-usage/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n".to_string(),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "append machine-local AGS runtime ignore rules".to_string(),
        content: "\n# Machine-local AGS runtime data\n/capability-snapshot/\n/skill-registry/\n/skill-usage/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n".to_string(),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("AGENT_SUITE_PROTOCOL.md"),
        description: "project-local AGS protocol pointer".to_string(),
        content: format!("# AGENT_SUITE_PROTOCOL.md\n\nThis project is integrated with Agent Governance Suite {AGS_VERSION}.\n\nCanonical governance entry points:\n\n- `AGENTS.md`\n- `CLAUDE.md`\n- `protocol/agent-task-protocol.md`\n- `protocol/task-routing.md`\n- `protocol/cursor-skill-index.md`\n- `config/agent-project-profile.yaml`\n\nHosts must call AGS preflight before AGS-governed work.\n"),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("WORKSPACE.md"),
        description: "project-local AGS workspace marker".to_string(),
        content: format!(
            "# WORKSPACE.md\n\n| Code | Role | Path |\n|---|---|---|\n| P | AGS-integrated project | {} |\n\nThis file marks the repository as an AGS-managed project, not an AGS suite root.\n",
            canonical.display()
        ),
        mode: None,
    });

    let profile = format!(
        r#"schema_version: 1
project:
  name: {}
  slug: {}
  type: {}
  primary_languages: []
  primary_runtime: {}

defaults:
  executor: {}
  runtime_adapter: {}
  execution_surface: {}
  # Omitted-field defaults only: use these values only when a generated task card
  # does not declare `Permission mode:`. Task level is a risk/review tier, not an
  # execution cap. A Heavy card that explicitly declares execute-and-verify remains
  # executable and still goes through the Heavy Review gate.
  permission_mode_by_level:
    light: execute-and-verify
    medium: execute-and-verify
    heavy: plan-only
  parallelism: none

verification:
  default_commands:
    - ags doctor --target .
  smoke_commands: []
  expensive_commands: []
  evidence_required:
    - command
    - exit_code

risk:
  high_risk_paths:
    - AGENTS.md
    - CLAUDE.md
    - AGENT_SUITE_PROTOCOL.md
    - config/agent-project-profile.yaml
    - protocol/
  protected_paths:
    - $HOME/.agents/memory/projects/{}/context-capsule.md
  destructive_actions_require_confirmation: true
  heavy_triggers:
    - protocol changes
    - hook installation
    - production wiring
  stop_conditions:
    - Do not overwrite user-owned files without explicit confirmation.

workflow:
  governance_docs:
    - AGENTS.md
    - CLAUDE.md
    - AGENT_SUITE_PROTOCOL.md
    - protocol/agent-task-protocol.md
    - protocol/task-routing.md
    - protocol/cursor-skill-index.md
  context_memory_capsule: {}
  task_memory: {}
  task_archive: {}
  default_review_policy: Codex review before release
  delivery_report: protocol/agent-task-protocol.md

user_preferences:
  interaction_style: {}
  ask_before:
    - destructive commands
    - hook installation
    - dependency installation
  do_not_do:
    - overwrite project memory design purpose automatically
"#,
        yaml_string(
            canonical
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("project")
        ),
        yaml_string(&slug),
        yaml_string("ags-integrated-project"),
        yaml_string("project-defined"),
        yaml_string("codex"),
        yaml_string("ags-mcp-or-cli-fallback"),
        yaml_string("local-workspace"),
        slug,
        yaml_string(&memory_dir.join("context-capsule.md").to_string_lossy()),
        yaml_string(&memory_dir.join("task-memory.md").to_string_lossy()),
        yaml_string(&memory_dir.join("task-archive").to_string_lossy()),
        yaml_string("concise, evidence-first, ask before high-risk writes"),
    );
    files.push(InstallFile {
        path: canonical.join("config/agent-project-profile.yaml"),
        description: "AGS project profile".to_string(),
        content: profile,
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("scripts/validate.sh"),
        description: "portable project task-card validator wrapper".to_string(),
        content: portable_validate_script(),
        mode: Some(0o755),
    });

    files.push(InstallFile {
        path: memory_dir.join("context-capsule.md"),
        description: "manual project memory capsule".to_string(),
        content: format!(
            "# Context Capsule: {slug}\n\nManual-maintained stable project memory.\n\n## 项目设计目的\n\nTODO: describe this project's purpose. This section is human-maintained and must not be overwritten by automated capture.\n\n## Stable Facts\n\n- Project path: `{}`\n- Memory dir: `{}`\n\n## 自动记忆入口\n\n- Task memory: `{}`\n- Task archive: `{}`\n- Sessions: `{}`\n",
            canonical.display(),
            memory_dir.display(),
            memory_dir.join("task-memory.md").display(),
            memory_dir.join("task-archive").display(),
            memory_dir.join("sessions").display(),
        ),
        mode: None,
    });
    files.push(InstallFile {
        path: memory_dir.join("task-memory.md"),
        description: "task continuity memory entrypoint".to_string(),
        content: format!(
            "# Task Memory: {slug}\n\nNo AGS task archives have been captured yet.\n\nThe manual project charter remains in `context-capsule.md`.\n"
        ),
        mode: None,
    });

    if let Some(protocol_dir) = protocol_dir {
        for name in project_protocol_files() {
            let src = protocol_dir.join(name);
            match std::fs::read_to_string(&src) {
                Ok(content) => files.push(InstallFile {
                    path: canonical.join("protocol").join(name),
                    description: format!("AGS protocol file: protocol/{name}"),
                    content,
                    mode: None,
                }),
                Err(e) => warnings.push(format!(
                    "cannot read protocol template {}: {}",
                    src.display(),
                    e
                )),
            }
        }
    } else {
        warnings.push(
            "no AGS protocol templates found; run `ags setup --yes` or invoke init from the AGS suite root"
                .to_string(),
        );
    }

    directories.sort();
    directories.dedup();

    ProjectInitPlan {
        target: canonical,
        slug,
        memory_dir,
        files,
        append_files,
        directories,
        warnings,
    }
}

fn project_init_plan(target: &Path, slug: Option<String>) -> ProjectInitPlan {
    project_init_plan_with_protocol(target, slug, project_template_protocol_dir())
}
fn project_file_status(file: &InstallFile, append_candidates: &[InstallFile]) -> &'static str {
    if !file.path.exists() {
        return "would-create";
    }
    if append_candidates
        .iter()
        .any(|candidate| candidate.path == file.path)
    {
        if let Ok(existing) = std::fs::read_to_string(&file.path) {
            if append_candidates.iter().any(|candidate| {
                candidate.path == file.path && existing.contains(candidate.content.trim())
            }) || existing.contains("Agent Governance Suite")
                || existing.contains(&format!("AGS {AGS_VERSION}"))
            {
                "exists"
            } else {
                "would-append"
            }
        } else {
            "exists"
        }
    } else {
        "exists"
    }
}
fn render_project_init_text(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let mut lines = vec![
        format!("AGS Project Init Plan {}", PROJECT_INIT_SCHEMA),
        format!("Target: {}", plan.target.display()),
        format!("Slug:   {}", plan.slug),
        format!("Memory: {}", plan.memory_dir.display()),
        format!("Mode:   {}", if dry_run { "dry-run" } else { "apply" }),
        String::new(),
        "Directories:".to_string(),
    ];
    for dir in &plan.directories {
        let status = if dir.exists() {
            "exists"
        } else {
            "would-create"
        };
        lines.push(format!("  - [{status}] {}", dir.display()));
    }
    lines.push(String::new());
    lines.push("Files:".to_string());
    for file in &plan.files {
        lines.push(format!(
            "  - [{}] {} — {}",
            project_file_status(file, &plan.append_files),
            file.path.display(),
            file.description
        ));
    }
    if !plan.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &plan.warnings {
            lines.push(format!("  ! {warning}"));
        }
    }
    lines.join("\n")
}
fn render_project_init_json(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let directories: Vec<_> = plan
        .directories
        .iter()
        .map(|dir| {
            serde_json::json!({
                "path": dir.to_string_lossy(),
                "status": if dir.exists() { "exists" } else { "would-create" },
            })
        })
        .collect();
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path.to_string_lossy(),
                "description": file.description,
                "status": project_file_status(file, &plan.append_files),
                "mode": file.mode.map(|m| format!("{m:o}")),
            })
        })
        .collect();
    let output = serde_json::json!({
        "schema_version": PROJECT_INIT_SCHEMA,
        "target": plan.target.to_string_lossy(),
        "slug": plan.slug,
        "memory_dir": plan.memory_dir.to_string_lossy(),
        "dry_run": dry_run,
        "directories": directories,
        "files": files,
        "warnings": plan.warnings,
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}
fn write_project_init_file(
    file: &InstallFile,
    append_candidates: &[InstallFile],
) -> suite_doctor::Finding {
    if let Some(parent) = file.path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return suite_doctor::Finding::fail(
                format!(
                    "project-init-{}",
                    sanitize_name(&file.path.to_string_lossy())
                ),
                format!("cannot create directory {}", parent.display()),
                e.to_string(),
            );
        }
    }

    if file.path.exists() {
        if let Some(append) = append_candidates
            .iter()
            .find(|candidate| candidate.path == file.path)
        {
            match std::fs::read_to_string(&file.path) {
                Ok(existing) if existing.contains(append.content.trim()) => {
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(existing)
                    if existing.contains("Agent Governance Suite")
                        || existing.contains(&format!("AGS {AGS_VERSION}")) =>
                {
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(_) => {
                    if let Err(e) = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&file.path)
                        .and_then(|mut f| {
                            use std::io::Write;
                            f.write_all(append.content.as_bytes())
                        })
                    {
                        return suite_doctor::Finding::fail(
                            format!(
                                "project-init-{}",
                                sanitize_name(&file.path.to_string_lossy())
                            ),
                            format!("append failed: {}", file.path.display()),
                            e.to_string(),
                        );
                    }
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("appended AGS block: {}", file.path.display()),
                    );
                }
                Err(e) => {
                    return suite_doctor::Finding::fail(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("read failed: {}", file.path.display()),
                        e.to_string(),
                    );
                }
            }
        }

        return suite_doctor::Finding::pass(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("kept existing: {}", file.path.display()),
        );
    }

    if let Err(e) = std::fs::write(&file.path, &file.content) {
        return suite_doctor::Finding::fail(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("write failed: {}", file.path.display()),
            e.to_string(),
        );
    }

    #[cfg(unix)]
    if let Some(mode) = file.mode {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&file.path) {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(&file.path, perms);
        }
    }

    suite_doctor::Finding::pass(
        format!(
            "project-init-{}",
            sanitize_name(&file.path.to_string_lossy())
        ),
        format!("written: {}", file.path.display()),
    )
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedProjectRefresh {
    pub target: String,
    pub slug: String,
    pub status: String,
    pub drift: bool,
    pub changed_files: Vec<String>,
    pub unchanged_files: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

struct PendingProjectWrite {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: Option<u32>,
}

fn is_project_memory_file(plan: &ProjectInitPlan, path: &Path) -> bool {
    path.starts_with(&plan.memory_dir)
}

fn is_entry_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("AGENTS.md" | "CLAUDE.md")
    )
}

fn is_generated_full_entry(path: &Path, content: &str) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("CLAUDE.md") => {
            content.starts_with("# CLAUDE.md\n\nThis project is governed by Agent Governance Suite")
        }
        Some("AGENTS.md") => content.starts_with("# AGENTS.md\n\n@CLAUDE.md"),
        _ => false,
    }
}

fn desired_project_file_content(
    plan: &ProjectInitPlan,
    file: &InstallFile,
) -> Result<Option<Vec<u8>>, String> {
    let existing = match std::fs::read(&file.path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", file.path.display())),
    };
    if is_project_memory_file(plan, &file.path) && existing.is_some() {
        return Ok(None);
    }
    let Some(before) = existing.as_deref() else {
        return Ok(Some(file.content.as_bytes().to_vec()));
    };

    let desired = if let Some(append) = plan
        .append_files
        .iter()
        .find(|candidate| candidate.path == file.path)
    {
        let text = std::str::from_utf8(before)
            .map_err(|e| format!("{} is not UTF-8: {e}", file.path.display()))?;
        if is_entry_file(&file.path) {
            if is_generated_full_entry(&file.path, text) {
                file.content.clone()
            } else {
                merge_managed_project_block(text, &append.content)?
            }
        } else if text.contains(append.content.trim()) {
            text.to_string()
        } else {
            format!("{}{}", text, append.content)
        }
    } else {
        file.content.clone()
    };
    if desired.as_bytes() == before {
        Ok(None)
    } else {
        Ok(Some(desired.into_bytes()))
    }
}

/// Inspect or refresh one registered project through a single deep interface.
/// User-owned entry files are changed only inside the AGS managed section;
/// project memory is create-only; AGS-owned protocol/template files are exact.
pub(crate) fn refresh_managed_project(
    target: &Path,
    slug: &str,
    source_root: &Path,
    apply: bool,
) -> ManagedProjectRefresh {
    let canonical = guard_path(target);
    if project_discovery::detect_project(&canonical).is_ags_suite {
        return ManagedProjectRefresh {
            target: canonical.display().to_string(),
            slug: slug.to_string(),
            status: "suite-authority".to_string(),
            drift: false,
            changed_files: Vec::new(),
            unchanged_files: Vec::new(),
            blocked_reasons: Vec::new(),
        };
    }

    let plan = project_init_plan_with_protocol(
        &canonical,
        Some(slug.to_string()),
        Some(source_root.join("protocol")),
    );
    let mut pending = Vec::new();
    let mut unchanged = Vec::new();
    let mut blocked = plan.warnings.clone();
    for file in &plan.files {
        match desired_project_file_content(&plan, file) {
            Ok(Some(after)) => {
                let before = std::fs::read(&file.path).ok();
                pending.push(PendingProjectWrite {
                    path: file.path.clone(),
                    before,
                    after,
                    mode: file.mode,
                });
            }
            Ok(None) => unchanged.push(file.path.display().to_string()),
            Err(e) => blocked.push(e),
        }
    }
    let changed_files: Vec<String> = pending
        .iter()
        .map(|write| write.path.display().to_string())
        .collect();
    let drift = !pending.is_empty() || !blocked.is_empty();
    if !apply || !blocked.is_empty() {
        return ManagedProjectRefresh {
            target: canonical.display().to_string(),
            slug: slug.to_string(),
            status: if !blocked.is_empty() {
                "blocked"
            } else if pending.is_empty() {
                "clean"
            } else {
                "planned"
            }
            .to_string(),
            drift,
            changed_files,
            unchanged_files: unchanged,
            blocked_reasons: blocked,
        };
    }

    let mut applied: Vec<&PendingProjectWrite> = Vec::new();
    for write in &pending {
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = write.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&write.path, &write.after)?;
            let _requested_mode = write.mode;
            #[cfg(unix)]
            if let Some(mode) = _requested_mode {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&write.path)?.permissions();
                permissions.set_mode(mode);
                std::fs::set_permissions(&write.path, permissions)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            for previous in applied.iter().rev() {
                if let Some(before) = &previous.before {
                    let _ = std::fs::write(&previous.path, before);
                } else {
                    let _ = std::fs::remove_file(&previous.path);
                }
            }
            blocked.push(format!(
                "write failed {}: {e}; prior writes rolled back",
                write.path.display()
            ));
            return ManagedProjectRefresh {
                target: canonical.display().to_string(),
                slug: slug.to_string(),
                status: "failed".to_string(),
                drift: true,
                changed_files,
                unchanged_files: unchanged,
                blocked_reasons: blocked,
            };
        }
        applied.push(write);
    }

    ManagedProjectRefresh {
        target: canonical.display().to_string(),
        slug: slug.to_string(),
        status: if pending.is_empty() {
            "clean"
        } else {
            "applied"
        }
        .to_string(),
        drift: false,
        changed_files,
        unchanged_files: unchanged,
        blocked_reasons: Vec::new(),
    }
}
fn cmd_project_init(
    target: &Path,
    slug: Option<String>,
    dry_run: bool,
    format: &str,
    mode: OverlayMode,
    migrate: bool,
) {
    if !target.exists() {
        eprintln!("ags init: target does not exist — {}", target.display());
        std::process::exit(1);
    }
    let plan = project_init_plan(target, slug);
    let overlay = compute_overlay_plan(&plan.target, &plan.files, mode, migrate);
    if dry_run {
        match format {
            "json" => {
                let mut value: serde_json::Value =
                    serde_json::from_str(&render_project_init_json(&plan, true))
                        .unwrap_or_default();
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("overlay".to_string(), overlay_json(&overlay));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            }
            _ => {
                println!("{}", render_project_init_text(&plan, true));
                println!();
                println!("{}", render_overlay_text(&overlay));
            }
        }
        return;
    }

    let mut report = suite_doctor::HealthReport::new("project-init");
    for dir in &plan.directories {
        match std::fs::create_dir_all(dir) {
            Ok(_) => report.add(suite_doctor::Finding::pass(
                format!("project-init-dir-{}", sanitize_name(&dir.to_string_lossy())),
                format!("directory ready: {}", dir.display()),
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                format!("project-init-dir-{}", sanitize_name(&dir.to_string_lossy())),
                format!("cannot create directory: {}", dir.display()),
                e.to_string(),
            )),
        }
    }
    for file in &plan.files {
        report.add(write_project_init_file(file, &plan.append_files));
    }
    for warning in &plan.warnings {
        report.add(suite_doctor::Finding::warn(
            format!("project-init-warning-{}", sanitize_name(warning)),
            warning,
            "project init completed with a warning",
        ));
    }
    for finding in apply_overlay(&overlay) {
        report.add(finding);
    }

    let preflight = project_discovery::run_session_preflight(
        &plan.target,
        &project_discovery::AgentType::Codex,
    );

    // Register ONLY a successfully onboarded project (all init writes + overlay
    // passed AND preflight is clean). A failed / partial init must not enter the
    // managed-projects registry. (init owns project memory + registration;
    // skill governance never touches project memory.)
    let managed_project_receipt = if should_register_project(report.passed(), preflight.exit_code) {
        register_managed_project(&plan.target, &plan.slug, &mut report)
    } else {
        None
    };
    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PROJECT_INIT_SCHEMA,
                "plan": serde_json::from_str::<serde_json::Value>(&render_project_init_json(&plan, false)).unwrap_or_default(),
                "overlay": overlay_json(&overlay),
                "report": report,
                "preflight": preflight,
                "managed_project_receipt": managed_project_receipt.as_ref().map(|p| p.display().to_string()),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", render_project_init_text(&plan, false));
            println!();
            println!("{}", render_overlay_text(&overlay));
            println!();
            println!("{}", suite_doctor::render_text(&report));
            println!();
            println!(
                "{}",
                project_discovery::render_session_preflight_text(&preflight)
            );
        }
    }
    if !report.passed() || preflight.exit_code != 0 {
        std::process::exit(1);
    }
}

// ── Local governance overlay (.git/info/exclude management) ─────────────────
//
// `ags init` defaults to a `local` overlay: the AGS governance files it writes
// into a repository are added to `.git/info/exclude` so they are git-ignored
// locally and never show up as committable changes. `--mode shared|tracked`
// opts into a committed overlay (no exclude). `--migrate-tracked-overlay`
// untracks already-tracked AGS-owned files via `git rm --cached` (keeping the
// working copy). Shared files the repository may own (AGENTS.md / CLAUDE.md /
// .gitignore) are never auto-untracked.
/// A project enters the managed-projects registry ONLY when init fully
/// succeeded: every write / overlay finding passed AND preflight was clean.
/// A failed or partial init must never be recorded as managed state.
fn should_register_project(report_passed: bool, preflight_exit: i32) -> bool {
    report_passed && preflight_exit == 0
}
/// Register `target` in the managed-projects registry after a successful init.
/// Append/dedupe on canonical path; preserves first-registration time. Marks
/// GitHub/remote-backed repos (origin present) so downstream sync stays
/// local-plan-only. Adds a Finding to the init report and emits a receipt.
/// Returns the receipt path on a registry write, None on no-op / malformed.
fn register_managed_project(
    target: &Path,
    slug: &str,
    report: &mut suite_doctor::HealthReport,
) -> Option<PathBuf> {
    use crate::managed_projects as mp;
    let runtime_home = default_private_runtime_home();
    let reg_path = mp::registry_path(&runtime_home);
    let mut reg = match mp::load(&reg_path) {
        Ok(r) => r,
        Err(e) => {
            report.add(suite_doctor::Finding::warn(
                "managed-project-registry",
                "managed-projects.yaml is malformed; reporting drift instead of overwriting",
                e,
            ));
            return None;
        }
    };
    let canon = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let path_str = canon.display().to_string();
    let is_git = std::process::Command::new("git")
        .arg("-C")
        .arg(&canon)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let origin = if is_git {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&canon)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    let vcs = if is_git {
        mp::ProjectVcs::Git
    } else {
        mp::ProjectVcs::None
    };
    let entry = mp::describe_project(
        path_str.clone(),
        slug.to_string(),
        unix_timestamp(),
        vcs,
        origin,
    );
    let change = mp::upsert(&mut reg, entry);
    if change == mp::RegistryChange::Unchanged {
        report.add(suite_doctor::Finding::pass(
            "managed-project-registry",
            format!("already registered: {path_str}"),
        ));
        return None;
    }
    if let Some(parent) = reg_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&reg_path, mp::render_yaml(&reg)) {
        report.add(suite_doctor::Finding::warn(
            "managed-project-registry",
            "could not write managed-projects.yaml",
            e.to_string(),
        ));
        return None;
    }
    let verb = if change == mp::RegistryChange::Added {
        "added"
    } else {
        "refreshed"
    };
    report.add(suite_doctor::Finding::pass(
        "managed-project-registry",
        format!("registered in managed-projects.yaml: {path_str} ({verb})"),
    ));
    let ar = receipt::build_action_receipt(
        "init-register-project",
        Some(&path_str),
        receipt::GateResult {
            decision: "allow".to_string(),
            reason: Some("ags init managed-project registration".to_string()),
        },
        vec![],
        vec![receipt::ReceiptWrite {
            op: "overwrite".to_string(),
            path: reg_path.display().to_string(),
            from: None,
            backup: None,
            detail: format!("managed-projects.yaml upsert ({verb})"),
        }],
        vec![],
        vec![],
        receipt::RollbackPlan::backup_restore(vec![]),
        "applied",
        true,
    );
    emit_ags_action_receipt(&ar).ok()
}

// ── ags agents — Agent host governance (五段心智第 2 段) ──────────────────────

pub(crate) fn run(
    target: &Path,
    slug: Option<String>,
    dry_run: bool,
    format: &str,
    mode: &str,
    migrate_tracked_overlay: bool,
) {
    let overlay_mode = OverlayMode::parse(mode);
    if migrate_tracked_overlay && overlay_mode == OverlayMode::Shared {
        eprintln!(
            "ags init: --migrate-tracked-overlay requires --mode local (shared/tracked overlays stay committed)"
        );
        std::process::exit(1);
    }
    cmd_project_init(
        target,
        slug,
        dry_run,
        format,
        overlay_mode,
        migrate_tracked_overlay,
    )
}
#[cfg(test)]
mod project_init_relocated_tests {
    use super::*;

    #[test]
    fn managed_project_block_refresh_preserves_user_content() {
        let existing = "# Project rules\n\nKeep this.\n\n## Agent Governance Suite\n\nThis project is governed by AGS 0.2.6.\n\n## Project-specific tail\n\nKeep tail.\n";
        let desired = "## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n";

        let merged = merge_managed_project_block(existing, desired).expect("managed block");

        assert!(merged.contains("Keep this."));
        assert!(merged.contains("Keep tail."));
        assert!(merged.contains("AGS 0.3.0"));
        assert!(!merged.contains("AGS 0.2.6"));
        assert_eq!(merged.matches("## Agent Governance Suite").count(), 1);
    }

    #[test]
    fn managed_project_block_refresh_rejects_ambiguous_unowned_section() {
        let existing =
            "# Project rules\n\n## Agent Governance Suite\n\nCustom project-owned prose.\n";
        let desired = "## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n";

        assert!(merge_managed_project_block(existing, desired).is_err());
    }

    #[test]
    fn managed_project_refresh_policy_preserves_memory_and_refreshes_owned_files() {
        let base =
            std::env::temp_dir().join(format!("ags-project-refresh-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("project");
        let memory_dir = base.join("memory");
        let agents = InstallFile {
            path: target.join("AGENTS.md"),
            description: "entry".to_string(),
            content: "# AGENTS.md\n\n## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n".to_string(),
            mode: None,
        };
        let append = InstallFile {
            path: agents.path.clone(),
            description: "managed block".to_string(),
            content: "## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n"
                .to_string(),
            mode: None,
        };
        let protocol = InstallFile {
            path: target.join("protocol/task-routing.md"),
            description: "owned".to_string(),
            content: "current protocol\n".to_string(),
            mode: None,
        };
        let memory = InstallFile {
            path: memory_dir.join("context-capsule.md"),
            description: "memory".to_string(),
            content: "default memory\n".to_string(),
            mode: None,
        };
        for path in [&agents.path, &protocol.path, &memory.path] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &agents.path,
            "# User rules\n\nKeep me.\n\n## Agent Governance Suite\n\nThis project is governed by AGS 0.2.6.\n",
        )
        .unwrap();
        std::fs::write(&protocol.path, "stale protocol\n").unwrap();
        std::fs::write(&memory.path, "user memory\n").unwrap();
        let plan = ProjectInitPlan {
            target,
            slug: "test".to_string(),
            memory_dir,
            files: vec![agents.clone(), protocol.clone(), memory.clone()],
            append_files: vec![append],
            directories: Vec::new(),
            warnings: Vec::new(),
        };

        let entry = desired_project_file_content(&plan, &agents)
            .unwrap()
            .expect("stale managed block");
        let entry = String::from_utf8(entry).unwrap();
        assert!(entry.contains("Keep me."));
        assert!(entry.contains("AGS 0.3.0"));
        assert_eq!(
            desired_project_file_content(&plan, &protocol).unwrap(),
            Some(b"current protocol\n".to_vec())
        );
        assert_eq!(desired_project_file_content(&plan, &memory).unwrap(), None);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn managed_project_refresh_uses_explicit_protocol_source() {
        let base =
            std::env::temp_dir().join(format!("ags-project-refresh-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("project");
        let protocol_source = base.join("canonical-protocol");
        std::fs::create_dir_all(&target).unwrap();
        for name in project_protocol_files() {
            std::fs::create_dir_all(&protocol_source).unwrap();
            std::fs::write(protocol_source.join(name), format!("canonical {name}\n")).unwrap();
        }

        let plan = project_init_plan_with_protocol(
            &target,
            Some("test".to_string()),
            Some(protocol_source),
        );
        let routing = plan
            .files
            .iter()
            .find(|file| file.path.ends_with("protocol/task-routing.md"))
            .expect("task-routing projection");
        assert_eq!(routing.content, "canonical task-routing.md\n");

        let _ = std::fs::remove_dir_all(&base);
    }
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_project(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{suffix}", std::process::id()))
    }

    #[test]
    fn should_register_project_only_on_full_success() {
        // A project enters the registry only when init writes/overlay passed AND
        // preflight was clean — a failed/partial init must not be registered.
        assert!(should_register_project(true, 0));
        assert!(!should_register_project(false, 0));
        assert!(!should_register_project(true, 1));
        assert!(!should_register_project(false, 1));
    }

    #[test]
    fn project_init_entry_files_encode_request_decision_contract() {
        let target = unique_temp_project("ags-project-init-entry-contract");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);

        let entry_files: Vec<&InstallFile> = plan
            .files
            .iter()
            .chain(plan.append_files.iter())
            .filter(|file| file.path.ends_with("AGENTS.md") || file.path.ends_with("CLAUDE.md"))
            .collect();
        assert_eq!(entry_files.len(), 4, "create and append entry surfaces");

        for file in &entry_files {
            let lower = file.content.to_lowercase();
            assert!(file.content.contains("`ags_preflight`"));
            assert!(file.content.contains("`ags_route_request`"));
            assert!(file
                .content
                .replace('`', "")
                .contains("typed HostRouteProposal"));
            assert!(file.content.contains("strictly read-only"));
            assert!(file.content.contains("DirectResponse"));
            assert!(file.content.contains("SkillTarget"));
            assert!(file.content.contains("ags_apply_action"));
            assert!(file.content.contains("MachineCli"));
            assert!(file.content.contains("Skill Resolver"));
            assert!(file.content.contains("Compiler"));
            assert!(file.content.contains("Policy"));
            assert!(file.content.contains("Gate"));
            assert!(file.content.contains("Runner"));
            assert!(lower.contains("confirmed"));
            assert!(lower.contains("task-card generation"));
            assert!(lower.contains("existing"));
            assert!(lower.contains("validat"));
            assert!(lower.contains("before"));
        }

        for file in entry_files
            .iter()
            .filter(|file| file.path.ends_with("AGENTS.md"))
        {
            assert!(file.content.contains("exactly two modes"));
            assert!(file.content.contains("`plan-only`"));
            assert!(file.content.contains("`execute-and-verify`"));
        }

        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_plan_ignores_gep_runtime_assets() {
        let target = unique_temp_project("ags-project-init-ignore-plan");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");
        assert!(gitignore.content.contains("/capability-snapshot/"));
        assert!(gitignore.content.contains("/skill-registry/"));
        assert!(gitignore.content.contains("/skill-usage/"));
        assert!(gitignore.content.contains("/decision-leases/"));
        assert!(gitignore.content.contains("/auth-state/"));
        assert!(gitignore.content.contains("/receipts/"));
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_gitignore_append_is_idempotent() {
        let target = unique_temp_project("ags-project-init-ignore-idempotent");
        std::fs::create_dir_all(&target).unwrap();
        let gitignore_path = target.join(".gitignore");
        std::fs::write(&gitignore_path, "/target/\n").unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");

        let first = write_project_init_file(gitignore, &plan.append_files);
        let second = write_project_init_file(gitignore, &plan.append_files);

        assert_eq!(first.status, suite_doctor::CheckStatus::Pass);
        assert_eq!(second.status, suite_doctor::CheckStatus::Pass);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_gitignore_dry_run_status_is_idempotent() {
        let target = unique_temp_project("ags-project-init-ignore-status");
        std::fs::create_dir_all(&target).unwrap();
        let gitignore_path = target.join(".gitignore");
        std::fs::write(
            &gitignore_path,
            "/target/\n\n# Machine-local AGS runtime data\n/capability-snapshot/\n/skill-registry/\n/skill-usage/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n",
        )
        .unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");

        assert_eq!(project_file_status(gitignore, &plan.append_files), "exists");
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_plan_includes_memory_capsule_task_memory_and_archive() {
        let target = unique_temp_project("ags-project-init-memory-plan");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);
        assert!(
            plan.files
                .iter()
                .any(|f| f.path.ends_with("context-capsule.md")),
            "init plan must create the memory capsule"
        );
        assert!(
            plan.files
                .iter()
                .any(|f| f.path.ends_with("task-memory.md")),
            "init plan must create task-memory.md"
        );
        assert!(
            plan.directories.iter().any(|d| d.ends_with("task-archive")),
            "init plan must create the task-archive directory"
        );
        let _ = std::fs::remove_dir_all(target);
    }

    /// G5/G8: the memory capsule and task-memory are NOT append-managed, so the
    /// init writer's keep-existing branch protects them from being overwritten.
    #[test]
    fn memory_capsule_is_not_append_managed() {
        let target = unique_temp_project("ags-project-init-capsule-protected");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);
        let capsule = plan
            .files
            .iter()
            .find(|f| f.path.ends_with("context-capsule.md"))
            .expect("capsule planned");
        assert!(
            !plan.append_files.iter().any(|c| c.path == capsule.path),
            "capsule must not be append-managed (would risk overwrite)"
        );
        let _ = std::fs::remove_dir_all(target);
    }

    /// G5/G8: re-running init keeps an existing capsule byte-for-byte (the
    /// keep-existing path), so a human-edited `## 项目设计目的` is never clobbered.
    #[test]
    fn write_project_init_file_keeps_existing_non_append_file() {
        let dir = unique_temp_project("ags-init-keep-existing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("context-capsule.md");
        let human = "# Context Capsule\n\n## 项目设计目的\nHUMAN-ONLY-SENTINEL\n";
        std::fs::write(&path, human).unwrap();
        let file = InstallFile {
            path: path.clone(),
            description: "capsule".to_string(),
            content: "GENERATED — should not overwrite\n".to_string(),
            mode: None,
        };
        let finding = write_project_init_file(&file, &[]);
        assert_eq!(finding.status, suite_doctor::CheckStatus::Pass);
        assert!(finding.message.contains("kept existing"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), human);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_project_init_file_creates_missing_file() {
        let dir = unique_temp_project("ags-init-create-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory/context-capsule.md");
        let file = InstallFile {
            path: path.clone(),
            description: "capsule".to_string(),
            content: "# fresh capsule\n".to_string(),
            mode: None,
        };
        let finding = write_project_init_file(&file, &[]);
        assert_eq!(finding.status, suite_doctor::CheckStatus::Pass);
        assert!(finding.message.contains("written"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# fresh capsule\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
