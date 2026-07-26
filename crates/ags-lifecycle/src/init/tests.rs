use super::apply::write_project_init_file;
use super::execute::{execute, should_register_project, InitRequest};
use super::managed_projects::{desired_project_file_content, merge_managed_project_block};
use super::model::{InitCheckStatus, InitFile, PROJECT_INIT_SCHEMA};
use super::plan::{
    project_file_status, project_init_plan, project_init_plan_with_protocol,
    project_protocol_files, ProjectInitPlan,
};

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
    let existing = "# Project rules\n\n## Agent Governance Suite\n\nCustom project-owned prose.\n";
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
    let agents = InitFile {
        path: target.join("AGENTS.md"),
        description: "entry".to_string(),
        content:
            "# AGENTS.md\n\n## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n"
                .to_string(),
        mode: None,
    };
    let append = InitFile {
        path: agents.path.clone(),
        description: "managed block".to_string(),
        content: "## Agent Governance Suite\n\nThis project is governed by AGS 0.3.0.\n"
            .to_string(),
        mode: None,
    };
    let protocol = InitFile {
        path: target.join("protocol/task-routing.md"),
        description: "owned".to_string(),
        content: "current protocol\n".to_string(),
        mode: None,
    };
    let memory = InitFile {
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

    let plan =
        project_init_plan_with_protocol(&target, Some("test".to_string()), Some(protocol_source));
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
fn project_init_entry_files_are_concise_and_reference_canonical_contracts() {
    let target = unique_temp_project("ags-project-init-entry-contract");
    std::fs::create_dir_all(&target).unwrap();
    let plan = project_init_plan(&target, None);

    let entry_files: Vec<&InitFile> = plan
        .files
        .iter()
        .chain(plan.append_files.iter())
        .filter(|file| file.path.ends_with("AGENTS.md") || file.path.ends_with("CLAUDE.md"))
        .collect();
    assert_eq!(entry_files.len(), 4, "create and append entry surfaces");

    for file in &entry_files {
        assert!(
            file.content.lines().count() < 30,
            "entry files stay concise; details belong in canonical docs"
        );
        assert!(file.content.contains("Agent Governance Suite"));
        assert!(file.content.contains("protocol/"));
    }
    for file in entry_files
        .iter()
        .filter(|file| file.path.ends_with("AGENTS.md"))
    {
        assert!(file.content.contains("`ags_preflight`"));
        assert!(file.content.contains("`ags_route_request`"));
        assert!(file.content.contains("typed `HostRouteProposal`"));
        assert!(file.content.contains("`ags_apply_action`"));
        assert!(file.content.to_lowercase().contains("existing `## 任务卡`"));
        assert!(!file.content.contains("@CLAUDE.md"));
    }
    for file in entry_files
        .iter()
        .filter(|file| file.path.ends_with("CLAUDE.md"))
    {
        assert!(file.content.contains("bounded direct"));
        assert!(file.content.contains("protocol/runtime-adapters.md"));
    }
    let generated_claude = plan
        .files
        .iter()
        .find(|file| file.path.ends_with("CLAUDE.md"))
        .expect("generated CLAUDE.md");
    assert!(generated_claude.content.contains("@AGENTS.md"));

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
    assert!(gitignore.content.contains("assets/gep/"));
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

    assert_eq!(first.status, InitCheckStatus::Pass);
    assert_eq!(second.status, InitCheckStatus::Pass);
    let content = std::fs::read_to_string(&gitignore_path).unwrap();
    assert_eq!(content.matches("assets/gep/").count(), 1);
    let _ = std::fs::remove_dir_all(target);
}

#[test]
fn project_init_gitignore_dry_run_status_is_idempotent() {
    let target = unique_temp_project("ags-project-init-ignore-status");
    std::fs::create_dir_all(&target).unwrap();
    let gitignore_path = target.join(".gitignore");
    std::fs::write(
            &gitignore_path,
            "/target/\n\n# AGS/GEP local runtime data\nassets/gep/\n/capability-snapshot/\n/skill-registry/\n/skill-usage/\n/decision-leases/\n/auth-state/\n/receipts/\n/.ags/\n",
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
    let file = InitFile {
        path: path.clone(),
        description: "capsule".to_string(),
        content: "GENERATED — should not overwrite\n".to_string(),
        mode: None,
    };
    let finding = write_project_init_file(&file, &[]);
    assert_eq!(finding.status, InitCheckStatus::Pass);
    assert!(finding.message.contains("kept existing"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), human);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn write_project_init_file_creates_missing_file() {
    let dir = unique_temp_project("ags-init-create-missing");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memory/context-capsule.md");
    let file = InitFile {
        path: path.clone(),
        description: "capsule".to_string(),
        content: "# fresh capsule\n".to_string(),
        mode: None,
    };
    let finding = write_project_init_file(&file, &[]);
    assert_eq!(finding.status, InitCheckStatus::Pass);
    assert!(finding.message.contains("written"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# fresh capsule\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn init_request_preserves_shared_migration_rejection() {
    let request = InitRequest {
        target: PathBuf::from("."),
        slug: None,
        dry_run: true,
        mode: "shared".to_string(),
        migrate_tracked_overlay: true,
    };
    let error = match execute(request, |_, _, _| None) {
        Ok(_) => panic!("shared migration must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
            error,
            "ags init: --migrate-tracked-overlay requires --mode local (shared/tracked overlays stay committed)"
        );
}

#[test]
fn dry_run_output_keeps_text_and_json_contract_without_writes() {
    let target = unique_temp_project("ags-init-dry-run-output");
    std::fs::create_dir_all(&target).unwrap();
    let output = execute(
        InitRequest {
            target: target.clone(),
            slug: Some("dry-run-contract".to_string()),
            dry_run: true,
            mode: "local".to_string(),
            migrate_tracked_overlay: false,
        },
        |_, _, _| panic!("dry-run must not register a project"),
    )
    .expect("dry-run output");

    assert!(output.succeeded());
    assert!(output
        .render_text()
        .starts_with("AGS Project Init Plan 2.4-project-init"));
    let json: serde_json::Value =
        serde_json::from_str(&output.render_json()).expect("valid init JSON");
    assert_eq!(json["schema_version"], PROJECT_INIT_SCHEMA);
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["slug"], "dry-run-contract");
    assert!(json.get("overlay").is_some());
    assert!(!target.join("AGENTS.md").exists());

    let _ = std::fs::remove_dir_all(target);
}
