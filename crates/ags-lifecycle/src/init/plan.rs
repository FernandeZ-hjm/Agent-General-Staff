//! Pure/read-only project initialization planning.

use super::model::{InitFile, AGS_VERSION};
use std::path::{Path, PathBuf};

pub(crate) fn guard_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        if let Some(name) = existing.file_name() {
            missing.push(name.to_os_string());
        }
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return absolute,
        }
    }
    let mut normalized = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for component in missing.iter().rev() {
        normalized.push(component);
    }
    normalized
}

pub(crate) fn sanitize_name(path: &str) -> String {
    path.trim_matches('/')
        .replace(['/', '\\', '.'], "-")
        .trim_matches('-')
        .to_string()
}

fn home_dir() -> PathBuf {
    ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn default_private_runtime_home() -> PathBuf {
    if let Some(path) = std::env::var_os("AGS_HOME") {
        return PathBuf::from(path);
    }
    home_dir().join(".ags").join("private-runtime")
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

pub(crate) fn project_protocol_files() -> &'static [&'static str] {
    &[
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
        "task-routing.md",
        "project-profile.md",
        "context-memory.md",
        "cursor-skill-index.md",
    ]
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
    if let Some(source_root) = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from) {
        let dir = source_root.join("protocol");
        if dir.join("agent-task-protocol.md").exists() {
            return Some(dir);
        }
    }
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
pub struct ProjectInitPlan {
    pub(crate) target: PathBuf,
    pub(crate) slug: String,
    pub(crate) memory_dir: PathBuf,
    pub(crate) files: Vec<InitFile>,
    pub(crate) append_files: Vec<InitFile>,
    pub(crate) directories: Vec<PathBuf>,
    pub(crate) warnings: Vec<String>,
}
pub(crate) fn project_init_plan_with_protocol(
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
        memory_dir.join("task-archive"),
        memory_dir.join("sessions"),
    ];
    let mut warnings = Vec::new();

    let ags_block = format!(
        "\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}.\n\n- Before AGS work call MCP `ags_preflight`, or `ags session preflight --for <agent-id> --target .`.\n- Read `ags://capabilities/current-host`; the host submits a typed `HostRouteProposal` to read-only `ags_route_request`.\n- Existing `## 任务卡` input validates before execution; explicit handoff requires a confirmed contract.\n- `ags_apply_action` is the only effectful AGS MCP tool.\n- Diagnose with `ags doctor --target .`.\n- Read details when relevant: `AGENT_SUITE_PROTOCOL.md`, `protocol/agent-task-protocol.md`, `protocol/task-routing.md`, `protocol/runtime-adapters.md`, `protocol/context-memory.md`.\n"
    );

    files.push(InitFile {
        path: canonical.join("AGENTS.md"),
        description: "agent entrypoint with AGS governance reference".to_string(),
        content: format!("# AGENTS.md\n{ags_block}"),
        mode: None,
    });
    append_files.push(InitFile {
        path: canonical.join("AGENTS.md"),
        description: "append AGS governance block to existing AGENTS.md".to_string(),
        content: ags_block.clone(),
        mode: None,
    });

    files.push(InitFile {
        path: canonical.join("CLAUDE.md"),
        description: "Claude Code AGS execution protocol entrypoint".to_string(),
        content: format!(
            "# CLAUDE.md\n\n@AGENTS.md\n\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}. Claude Code consumes a validated task card or an explicitly bounded direct-edit request. It must not infer task level, execution mode, review gate, or verification gate from raw language. Follow `protocol/agent-task-protocol.md` and `protocol/runtime-adapters.md` when relevant.\n"
        ),
        mode: None,
    });
    append_files.push(InitFile {
        path: canonical.join("CLAUDE.md"),
        description: "append AGS execution protocol block to existing CLAUDE.md".to_string(),
        content: format!("\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}. Claude Code consumes validated task cards or explicitly bounded direct edits and does not infer governance gates from raw language. Follow `AGENTS.md`, `protocol/agent-task-protocol.md`, and `protocol/runtime-adapters.md` when relevant.\n"),
        mode: None,
    });

    files.push(InitFile {
        path: canonical.join(".gitignore"),
        description: "ignore AGS/GEP local runtime data".to_string(),
        content: "# AGS/GEP local runtime data\nassets/gep/\n/capability-snapshot/\n/skill-registry/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n".to_string(),
        mode: None,
    });
    append_files.push(InitFile {
        path: canonical.join(".gitignore"),
        description: "append AGS/GEP local runtime ignore rules".to_string(),
        content: "\n# AGS/GEP local runtime data\nassets/gep/\n/capability-snapshot/\n/skill-registry/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n".to_string(),
        mode: None,
    });

    files.push(InitFile {
        path: canonical.join("AGENT_SUITE_PROTOCOL.md"),
        description: "project-local AGS protocol pointer".to_string(),
        content: format!("# AGENT_SUITE_PROTOCOL.md\n\nThis project is integrated with Agent Governance Suite {AGS_VERSION}.\n\nCanonical governance entry points:\n\n- `AGENTS.md`\n- `CLAUDE.md`\n- `protocol/agent-task-protocol.md`\n- `protocol/task-routing.md`\n- `protocol/cursor-skill-index.md`\n- `config/agent-project-profile.yaml`\n\nHosts must call AGS preflight before AGS-governed work.\n"),
        mode: None,
    });

    files.push(InitFile {
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
  execution_mode: single-writer
  execution_topology: single
  delegation_planning: no

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
    files.push(InitFile {
        path: canonical.join("config/agent-project-profile.yaml"),
        description: "AGS project profile".to_string(),
        content: profile,
        mode: None,
    });

    files.push(InitFile {
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
    files.push(InitFile {
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
                Ok(content) => files.push(InitFile {
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

pub(crate) fn project_init_plan(target: &Path, slug: Option<String>) -> ProjectInitPlan {
    project_init_plan_with_protocol(target, slug, project_template_protocol_dir())
}

pub(crate) fn append_content_present(path: &Path, existing: &str, append: &str) -> bool {
    if existing.contains(append.trim()) {
        return true;
    }
    if path.file_name().and_then(|name| name.to_str()) != Some(".gitignore") {
        return false;
    }
    let existing_rules = existing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<std::collections::BTreeSet<_>>();
    append
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .all(|rule| existing_rules.contains(rule))
}

pub(crate) fn project_file_status(file: &InitFile, append_candidates: &[InitFile]) -> &'static str {
    if !file.path.exists() {
        return "would-create";
    }
    if append_candidates
        .iter()
        .any(|candidate| candidate.path == file.path)
    {
        if let Ok(existing) = std::fs::read_to_string(&file.path) {
            if append_candidates.iter().any(|candidate| {
                candidate.path == file.path
                    && append_content_present(&file.path, &existing, &candidate.content)
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

#[cfg(test)]
mod tests {
    use super::append_content_present;
    use std::path::Path;

    #[test]
    fn gitignore_managed_rules_are_idempotent_across_heading_changes() {
        let managed = "# AGS managed\n.ags/\ntask-archive/\n";
        let existing = "# Project ignores\ntarget/\n\n# Older AGS heading\ntask-archive/\n.ags/\n";

        assert!(append_content_present(
            Path::new(".gitignore"),
            existing,
            managed
        ));
        assert!(!append_content_present(
            Path::new(".gitignore"),
            "# Project ignores\ntarget/\n.ags/\n",
            managed
        ));
        assert!(!append_content_present(
            Path::new("AGENTS.md"),
            ".ags/\ntask-archive/\n",
            managed
        ));
    }
}
