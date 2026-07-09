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
fn project_init_plan(target: &Path, slug: Option<String>) -> ProjectInitPlan {
    let canonical = guard_path(target);
    let slug = slug.unwrap_or_else(|| default_project_slug(&canonical));
    let memory_dir = project_memory_dir(&slug);
    let protocol_dir = project_template_protocol_dir();
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
        "\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}.\n\n- Run `ags doctor --target .` to diagnose local governance health.\n- AGS MCP hosts must call `ags_preflight` before other AGS tools.\n- CLI fallback: `ags session preflight --for <agent-id> --target .`.\n- Known agents get tailored instructions; unknown non-empty agent ids use the generic governed-host profile.\n- Protocol entry points: `AGENT_SUITE_PROTOCOL.md`, `CLAUDE.md`, `protocol/agent-task-protocol.md`, `protocol/task-routing.md`, and `protocol/cursor-skill-index.md`.\n- Task cards must be validated with the task-card-validator via `bash scripts/validate.sh <task-card>`.\n"
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
            "# CLAUDE.md\n\nThis project is governed by Agent Governance Suite {AGS_VERSION}.\n\nBefore task execution, run AGS preflight through MCP (`ags_preflight`) or CLI fallback:\n\n```bash\nags session preflight --for claude-code --target .\n```\n\nDo not classify tasks from raw requests. Follow solution formation, user confirmation, task-card request gate, execution contract, routing, gate, verification, and receipt rules from `protocol/agent-task-protocol.md`. Select skills and capability wakeups using `protocol/cursor-skill-index.md` and `protocol/task-routing.md`.\n"
        ),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join("CLAUDE.md"),
        description: "append AGS execution protocol block to existing CLAUDE.md".to_string(),
        content: format!("\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}. Run `ags_preflight` through MCP or `ags session preflight --for claude-code --target .` before execution. Follow `protocol/agent-task-protocol.md`, `protocol/task-routing.md`, and `protocol/cursor-skill-index.md`.\n"),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "ignore AGS local runtime data".to_string(),
        content: "# AGS local runtime data\nassets/ags/\n".to_string(),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "append AGS local runtime ignore rules".to_string(),
        content: "\n# AGS local runtime data\nassets/ags/\n".to_string(),
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
    medium: edit-with-confirmation
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
    fn project_init_plan_ignores_local_runtime_assets() {
        let target = unique_temp_project("ags-project-init-ignore-plan");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");
        assert!(gitignore.content.contains("assets/ags/"));
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
        let content = std::fs::read_to_string(&gitignore_path).unwrap();
        assert_eq!(content.matches("assets/ags/").count(), 1);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_gitignore_dry_run_status_is_idempotent() {
        let target = unique_temp_project("ags-project-init-ignore-status");
        std::fs::create_dir_all(&target).unwrap();
        let gitignore_path = target.join(".gitignore");
        std::fs::write(
            &gitignore_path,
            "/target/\n\n# AGS local runtime data\nassets/ags/\n",
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
}
