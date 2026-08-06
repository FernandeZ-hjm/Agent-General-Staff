use ags_capability_governance::{
    build_capability_snapshot_with_runtime_home, load_static_snapshot,
    resolve_capability_authority_root, resolve_mcp, resolve_skill, snapshot_path, ActiveMcp,
    ActiveMcpTable, ActiveSkill, ActiveSkillTable, AuthState, AvailabilityState,
    CapabilitySnapshot, GovernanceState, ResolveError, SkillCard, SkillSourceKind, SnapshotError,
    HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION,
};

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ags-skill-resolver-{name}-{}", std::process::id()))
}

#[test]
fn runtime_home_preserves_existing_environment_precedence() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();
    let old_runtime = std::env::var_os("AGS_RUNTIME_HOME");
    let old_home = std::env::var_os("AGS_HOME");

    std::env::set_var("AGS_RUNTIME_HOME", "/tmp/ags-runtime-priority");
    std::env::set_var("AGS_HOME", "/tmp/ags-home-fallback");
    assert_eq!(
        ags_platform::runtime_home(),
        std::path::PathBuf::from("/tmp/ags-runtime-priority")
    );

    match old_runtime {
        Some(value) => std::env::set_var("AGS_RUNTIME_HOME", value),
        None => std::env::remove_var("AGS_RUNTIME_HOME"),
    }
    match old_home {
        Some(value) => std::env::set_var("AGS_HOME", value),
        None => std::env::remove_var("AGS_HOME"),
    }
}

#[test]
fn integrated_sibling_project_uses_installed_suite_capability_authority() {
    let base = temp_path("sibling-authority");
    let _ = std::fs::remove_dir_all(&base);
    let suite = base.join("agent-governance-suite-stable");
    let project = base.join("integrated-project");
    let runtime = base.join("runtime");
    std::fs::create_dir_all(suite.join("manifests")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(suite.join("manifests/skills-registry.yaml"), "skills: []\n").unwrap();
    std::fs::write(suite.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
    std::fs::write(
        runtime.join("install-manifest.json"),
        serde_json::json!({"source_root": suite.display().to_string()}).to_string(),
    )
    .unwrap();

    assert_eq!(
        resolve_capability_authority_root(&project, &runtime, None).unwrap(),
        std::fs::canonicalize(&suite).unwrap()
    );

    let _ = std::fs::remove_dir_all(&base);
}

fn architecture_skill() -> ActiveSkill {
    ActiveSkill {
        skill_id: "superpowers".to_string(),
        invoke_hint: "[skill: superpowers]".to_string(),
        allowed_entrypoints: vec!["brainstorming".to_string()],
        intent_tags: vec!["system-architecture".to_string()],
        source_hash: "sha256:source".to_string(),
    }
}

fn architecture_card() -> SkillCard {
    SkillCard {
        skill_id: "superpowers".to_string(),
        display_name: "Superpowers".to_string(),
        summary: "Engineering workflow playbooks".to_string(),
        intent_tags: vec!["system-architecture".to_string()],
        positive_examples: vec!["设计一个跨模块架构".to_string()],
        negative_examples: vec!["解释现有模块".to_string()],
        entrypoints: vec!["brainstorming".to_string()],
        routing_surface: ags_capability_governance::SkillRoutingSurface::SkillTarget,
        routing_hint: None,
        source_kind: SkillSourceKind::Suite,
        governance: GovernanceState::Active,
        availability: AvailabilityState::Ready,
        reason_codes: Vec::new(),
        requires_auth: false,
        auth_state: AuthState::NotRequired,
        version: "registry".to_string(),
        source_hash: "sha256:source".to_string(),
    }
}

#[test]
fn resolves_an_exact_skill_and_entrypoint_without_reading_natural_language() {
    let table =
        ActiveSkillTable::new("codex", "sha256:snapshot", vec![architecture_skill()]).unwrap();
    let selection = resolve_skill(
        "superpowers",
        Some("brainstorming"),
        "sha256:snapshot",
        &table,
    )
    .unwrap();

    assert_eq!(selection.skill_id, "superpowers");
    assert_eq!(selection.entrypoint.as_deref(), Some("brainstorming"));
}

#[test]
fn resolves_an_exact_mcp_and_tool_without_reading_natural_language() {
    let table = ActiveMcpTable::new(
        "codex",
        "sha256:snapshot",
        vec![ActiveMcp {
            mcp_id: "context7".to_string(),
            invoke_hint: "context7 MCP".to_string(),
            allowed_tools: vec![
                "get-library-docs".to_string(),
                "resolve-library-id".to_string(),
            ],
            intent_tags: vec!["docs-lookup".to_string()],
            mutation_surface: "read_only".to_string(),
        }],
    )
    .unwrap();
    let selection = resolve_mcp(
        "context7",
        Some("get-library-docs"),
        "sha256:snapshot",
        &table,
    )
    .unwrap();

    assert_eq!(selection.mcp_id, "context7");
    assert_eq!(selection.tool.as_deref(), Some("get-library-docs"));
    assert_eq!(selection.mutation_surface, "read_only");
}

#[cfg(unix)]
#[test]
fn snapshot_registers_optional_parent_entrypoints_without_activating_an_uninstalled_adapter() {
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let base = temp_path("registered-parent-entrypoints");
    let root = base.join("authority");
    let runtime = base.join("runtime");
    let home = base.join("home");
    let visible_parent = home.join(".agents/skills/superpowers");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    for manifest in [
        "mcp-registry.yaml",
        "skills-registry.yaml",
        "suite.yaml",
        "third-party-capabilities.yaml",
    ] {
        std::fs::copy(
            source_root.join("manifests").join(manifest),
            root.join("manifests").join(manifest),
        )
        .unwrap();
    }
    let registry: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("manifests/skills-registry.yaml")).unwrap(),
    )
    .unwrap();
    let external_parent = registry["skills"]
        .as_sequence()
        .into_iter()
        .flatten()
        .find(|skill| skill["name"].as_str() == Some("superpowers"))
        .is_some_and(|skill| skill["source"]["type"].as_str() == Some("external_shared_skill"));
    let canonical_parent = if external_parent {
        visible_parent.clone()
    } else {
        root.join("global-skills/superpowers")
    };
    std::fs::create_dir_all(&canonical_parent).unwrap();
    std::fs::create_dir_all(&visible_parent).unwrap();
    let fixture = "---\nname: superpowers\ndescription: Hermetic parent skill fixture.\nintent_tags: [completion-verification]\n---\n";
    std::fs::write(canonical_parent.join("SKILL.md"), fixture).unwrap();
    for target in registry
        .get("route_targets")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
    {
        let routing = &target["routing"];
        if routing["parent"]["kind"].as_str() != Some("skill")
            || routing["parent"]["name"].as_str() != Some("superpowers")
            || routing["entrypoint"]["kind"].as_str() != Some("playbook")
        {
            continue;
        }
        let entrypoint = routing["entrypoint"]["name"].as_str().unwrap();
        let playbook = canonical_parent.join("playbooks").join(entrypoint);
        std::fs::create_dir_all(&playbook).unwrap();
        std::fs::write(
            playbook.join("PLAYBOOK.md"),
            format!("# Hermetic {entrypoint} fixture\n"),
        )
        .unwrap();
    }
    if !external_parent {
        std::fs::remove_dir(&visible_parent).unwrap();
        std::os::unix::fs::symlink(&canonical_parent, &visible_parent).unwrap();
    }

    let snapshot = ags_capability_governance::build_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    let card = snapshot
        .catalog
        .iter()
        .find(|card| card.skill_id == "superpowers")
        .expect("superpowers card");
    assert!(
        matches!(card.availability, AvailabilityState::Unavailable { .. }),
        "an unmanaged optional adapter must not become routable: {card:?}"
    );
    assert!(
        snapshot
            .active_skills
            .iter()
            .all(|skill| skill.skill_id != "superpowers"),
        "an unmanaged optional adapter leaked into the active route table"
    );
    let active = snapshot.validate_integrity("codex").unwrap().skills;

    for entrypoint in [
        "verification-before-completion",
        "test-driven-development",
        "executing-plans",
        "writing-plans",
    ] {
        assert!(
            card.entrypoints
                .iter()
                .any(|candidate| candidate == entrypoint),
            "catalog omitted {entrypoint}"
        );
        assert_eq!(
            resolve_skill(
                "superpowers",
                Some(entrypoint),
                &snapshot.snapshot_hash,
                &active
            ),
            Err(ResolveError::GovernancePrecondition("skill_not_active"))
        );
    }
    assert!(card
        .intent_tags
        .iter()
        .any(|tag| tag == "completion-verification"));
    assert!(card
        .positive_examples
        .iter()
        .any(|example| example == "做完了验证一下"));
    assert!(!card
        .entrypoints
        .iter()
        .any(|entrypoint| entrypoint == "systematic-debugging"));
    assert_eq!(
        resolve_skill(
            "superpowers",
            Some("systematic-debugging"),
            &snapshot.snapshot_hash,
            &active,
        ),
        Err(ResolveError::GovernancePrecondition("skill_not_active"))
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn missing_skill_and_wrong_entrypoint_fail_closed_without_fallback() {
    let table =
        ActiveSkillTable::new("codex", "sha256:snapshot", vec![architecture_skill()]).unwrap();
    assert_eq!(
        resolve_skill("diagnosing-bugs", None, "sha256:snapshot", &table).unwrap_err(),
        ResolveError::GovernancePrecondition("skill_not_active")
    );
    assert!(matches!(
        resolve_skill(
            "superpowers",
            Some("executing-plans"),
            "sha256:snapshot",
            &table
        ),
        Err(ResolveError::EntrypointNotAllowed { .. })
    ));
}

#[test]
fn duplicate_skill_identifier_is_rejected() {
    assert!(matches!(
        ActiveSkillTable::new(
            "codex",
            "sha256:snapshot",
            vec![architecture_skill(), architecture_skill()],
        ),
        Err(ResolveError::DuplicateSkill { .. })
    ));
}

fn snapshot() -> CapabilitySnapshot {
    CapabilitySnapshot::new(
        "codex",
        "sha256:registry-a",
        "sha256:runtime-a",
        vec![architecture_card()],
        Vec::new(),
        "https://example.com/third-party-capabilities.yaml",
        "sha256:third-party",
        Vec::new(),
        vec![architecture_skill()],
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn static_snapshot_validates_sealed_integrity_before_routing() {
    let snapshot = snapshot();
    assert_eq!(
        snapshot.schema_version,
        HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION
    );
    assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    assert!(snapshot.validate_integrity("codex").is_ok());
}

#[test]
fn wrong_host_or_tampered_static_snapshot_fails_closed() {
    let mut snapshot = snapshot();
    let untampered = snapshot.clone();
    assert_eq!(
        snapshot.validate_integrity("omp").unwrap_err(),
        SnapshotError::SkillSnapshotStale
    );
    snapshot.snapshot_hash = "sha256:tampered".to_string();
    assert_eq!(
        snapshot.validate_integrity("codex").unwrap_err(),
        SnapshotError::SnapshotIntegrityFailed
    );

    let mut snapshot = untampered;
    snapshot.active_skills.clear();
    assert_eq!(
        snapshot.validate_integrity("codex").unwrap_err(),
        SnapshotError::SnapshotIntegrityFailed
    );
}

#[test]
fn host_scoped_snapshots_coexist_and_validate_independently() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = temp_path("host-scoped");
    let _ = std::fs::remove_dir_all(&runtime);

    for host in ["codex", "claude-code"] {
        let snapshot = build_capability_snapshot_with_runtime_home(&root, host, &runtime).unwrap();
        let path = snapshot_path(&runtime, host);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string(&snapshot).unwrap()).unwrap();
    }

    assert_ne!(
        snapshot_path(&runtime, "codex"),
        snapshot_path(&runtime, "claude-code")
    );
    for host in ["codex", "claude-code"] {
        let (snapshot, _) = load_static_snapshot(&runtime, host).unwrap();
        assert_eq!(snapshot.host, host);
    }

    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn current_skill_body_change_waits_for_explicit_snapshot_refresh() {
    let base = temp_path("current-catalog-drift");
    let _ = std::fs::remove_dir_all(&base);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = base.join("runtime");
    let home = base.join("home");
    let body = home.join(".agents/skills/catalog-drift-demo");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: catalog-drift-demo\ndescription: First metadata.\nintent_tags: [catalog-drift]\n---\nfirst body\n",
    )
    .unwrap();
    std::fs::create_dir_all(body.join("scripts")).unwrap();
    std::fs::write(body.join("scripts/run.sh"), "printf first\n").unwrap();

    let snapshot = ags_capability_governance::build_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    let path = snapshot_path(&runtime, "codex");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert!(load_static_snapshot(&runtime, "codex").is_ok());

    // A referenced implementation file is part of source_hash even when the
    // catalog metadata in SKILL.md is unchanged.
    std::fs::write(body.join("scripts/run.sh"), "printf changed\n").unwrap();
    let (loaded, _) = load_static_snapshot(&runtime, "codex").unwrap();
    assert_eq!(loaded.snapshot_hash, snapshot.snapshot_hash);

    let refreshed = ags_capability_governance::write_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    assert_ne!(refreshed.snapshot_hash, snapshot.snapshot_hash);
    let (loaded, _) = load_static_snapshot(&runtime, "codex").unwrap();
    assert_eq!(loaded.snapshot_hash, refreshed.snapshot_hash);

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn cursor_catalog_discovers_shared_user_skills() {
    let base = temp_path("cursor-shared-catalog");
    let _ = std::fs::remove_dir_all(&base);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = base.join("runtime");
    let home = base.join("home");
    let body = home.join(".agents/skills/cursor-shared-demo");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: cursor-shared-demo\ndescription: shared cursor skill\nintent_tags: [cursor]\n---\n",
    )
    .unwrap();

    let snapshot = ags_capability_governance::build_capability_snapshot_with_roots(
        &root, "cursor", &runtime, &home,
    )
    .unwrap();
    assert!(snapshot
        .catalog
        .iter()
        .any(|card| card.skill_id == "cursor-shared-demo"));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn catalog_unifies_all_enabled_sources_and_excludes_disabled_plugin_cache() {
    use std::os::unix::fs::symlink;

    let base = temp_path("all-sources");
    let _ = std::fs::remove_dir_all(&base);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = base.join("home");
    let runtime = base.join("runtime");
    let write_skill = |path: &std::path::Path, name: &str| {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {name} catalog fixture.\nintent_tags: [{name}]\n---\nbody\n"
            ),
        )
        .unwrap();
    };

    write_skill(
        &home.join(".codex/skills/.system/catalog-system-demo"),
        "catalog-system-demo",
    );
    write_skill(
        &home.join(".agents/skills/catalog-user-demo"),
        "catalog-user-demo",
    );

    let project = base.join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let project_body = project.join(".agents/skills/catalog-project-demo");
    write_skill(&project_body, "catalog-project-demo");
    std::fs::create_dir_all(home.join(".codex/skills")).unwrap();
    symlink(
        &project_body,
        home.join(".codex/skills/catalog-project-demo"),
    )
    .unwrap();

    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/config.toml"),
        "[plugins.\"enabled-demo@market\"]\nenabled = true\n\
         [plugins.\"disabled-demo@market\"]\nenabled = false\n",
    )
    .unwrap();
    write_skill(
        &home.join(".codex/plugins/cache/market/enabled-demo/1.0/skills/catalog-plugin-enabled"),
        "catalog-plugin-enabled",
    );
    write_skill(
        &home.join(".codex/plugins/cache/market/disabled-demo/1.0/skills/catalog-plugin-disabled"),
        "catalog-plugin-disabled",
    );

    let snapshot = ags_capability_governance::build_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    let source = |skill_id: &str| {
        snapshot
            .catalog
            .iter()
            .find(|card| card.skill_id == skill_id)
            .map(|card| card.source_kind)
    };
    assert_eq!(
        source("catalog-system-demo"),
        Some(SkillSourceKind::HostSystem)
    );
    assert_eq!(
        source("catalog-user-demo"),
        Some(SkillSourceKind::UserInstalled)
    );
    assert_eq!(
        source("catalog-project-demo"),
        Some(SkillSourceKind::ProjectLocal)
    );
    assert_eq!(
        source("catalog-plugin-enabled"),
        Some(SkillSourceKind::EnabledPlugin)
    );
    assert_eq!(source("catalog-plugin-disabled"), None);
    assert!(snapshot
        .catalog
        .iter()
        .any(|card| card.source_kind == SkillSourceKind::Suite));
    for skill_id in [
        "catalog-user-demo",
        "catalog-project-demo",
        "catalog-plugin-enabled",
    ] {
        let card = snapshot
            .catalog
            .iter()
            .find(|card| card.skill_id == skill_id)
            .unwrap();
        assert_eq!(card.governance, GovernanceState::Candidate);
        assert!(!snapshot
            .active_skills
            .iter()
            .any(|active| active.skill_id == skill_id));
    }

    let _ = std::fs::remove_dir_all(base);
}
