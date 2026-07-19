use request_governance::{EngineeringDemand, SkillDemand};
use skill_resolver::{
    build_capability_snapshot_with_runtime_home, load_demand_routes, load_validated_snapshot,
    load_validated_snapshot_with_roots, resolve_capability_authority_root, resolve_skill,
    snapshot_path, ActiveSkill, ActiveSkillTable, AuthState, AvailabilityState, CapabilitySnapshot,
    GovernanceState, ResolveError, SkillCard, SkillSourceKind, SnapshotError,
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
        skill_resolver::locate_runtime_home(),
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
    let suite = base.join("example-stable-suite");
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
        legacy_demands: vec![SkillDemand::Engineering(
            EngineeringDemand::SystemArchitecture,
        )],
        source_hash: "sha256:source".to_string(),
    }
}

fn architecture_card() -> SkillCard {
    SkillCard {
        skill_id: "superpowers".to_string(),
        display_name: "Superpowers".to_string(),
        summary: "Engineering workflow playbooks".to_string(),
        intent_tags: vec!["system-architecture".to_string()],
        entrypoints: vec!["brainstorming".to_string()],
        source_kind: SkillSourceKind::Suite,
        governance: GovernanceState::Active,
        availability: AvailabilityState::Ready,
        reason_codes: Vec::new(),
        requires_auth: false,
        auth_state: AuthState::NotRequired,
        activity: skill_resolver::ActivityState::Unobserved,
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
        "sha256:overlay-a",
        "sha256:runtime-a",
        vec![architecture_card()],
        vec![architecture_skill()],
    )
    .unwrap()
}

#[test]
fn snapshot_validates_all_authority_hashes_before_routing() {
    let snapshot = snapshot();
    assert_eq!(
        snapshot.schema_version,
        HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION
    );
    assert!(snapshot.active_table_hash.starts_with("sha256:"));
    assert!(snapshot.catalog_hash.starts_with("sha256:"));
    assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    assert!(snapshot
        .validate(
            "codex",
            "sha256:registry-a",
            "sha256:overlay-a",
            "sha256:runtime-a"
        )
        .is_ok());
}

#[test]
fn stale_or_tampered_snapshot_fails_closed() {
    let mut snapshot = snapshot();
    assert_eq!(
        snapshot
            .validate(
                "codex",
                "sha256:registry-b",
                "sha256:overlay-a",
                "sha256:runtime-a"
            )
            .unwrap_err(),
        SnapshotError::SkillSnapshotStale
    );
    snapshot.snapshot_hash = "sha256:tampered".to_string();
    assert_eq!(
        snapshot
            .validate(
                "codex",
                "sha256:registry-a",
                "sha256:overlay-a",
                "sha256:runtime-a"
            )
            .unwrap_err(),
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
        let (snapshot, _) = load_validated_snapshot(&root, &runtime, host).unwrap();
        assert_eq!(snapshot.host, host);
    }

    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn legacy_single_snapshot_is_not_a_hidden_fallback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = temp_path("legacy-snapshot");
    let legacy = runtime
        .join("capability-snapshot")
        .join("capability-snapshot.json");
    let _ = std::fs::remove_dir_all(&runtime);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let snapshot = build_capability_snapshot_with_runtime_home(&root, "codex", &runtime).unwrap();
    std::fs::write(&legacy, serde_json::to_string(&snapshot).unwrap()).unwrap();

    assert!(load_validated_snapshot(&root, &runtime, "codex").is_err());
    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn current_skill_body_change_invalidates_a_self_consistent_saved_snapshot() {
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

    let snapshot =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
            .unwrap();
    let path = snapshot_path(&runtime, "codex");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert!(load_validated_snapshot_with_roots(&root, &runtime, "codex", &home).is_ok());

    // A referenced implementation file is part of source_hash even when the
    // catalog metadata in SKILL.md is unchanged.
    std::fs::write(body.join("scripts/run.sh"), "printf changed\n").unwrap();
    assert!(matches!(
        load_validated_snapshot_with_roots(&root, &runtime, "codex", &home),
        Err(skill_resolver::SnapshotLoadError::Snapshot(
            SnapshotError::SkillSnapshotStale
        ))
    ));

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

    let snapshot =
        skill_resolver::build_capability_snapshot_with_roots(&root, "cursor", &runtime, &home)
            .unwrap();
    assert!(snapshot
        .catalog
        .iter()
        .any(|card| card.skill_id == "cursor-shared-demo"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn registry_legacy_demands_are_metadata_complete_but_not_route_authority() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let routes = load_demand_routes(&root).unwrap();
    let unique: std::collections::HashSet<_> = routes.iter().map(|route| route.demand).collect();

    assert_eq!(routes.len(), unique.len());
    assert_eq!(unique.len(), SkillDemand::all().len());
    for demand in SkillDemand::all() {
        assert!(
            unique.contains(demand),
            "missing registry metadata: {demand:?}"
        );
    }
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

    let snapshot =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
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

#[test]
fn adopted_authenticated_skill_is_ready_only_after_nonsensitive_auth_state() {
    let base = temp_path("auth-gate");
    let _ = std::fs::remove_dir_all(&base);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = base.join("home");
    let runtime = base.join("runtime");
    let skill_id = "catalog-auth-demo";
    let body = home.join(".agents/skills").join(skill_id);
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: catalog-auth-demo\ndescription: Auth-gated catalog fixture.\nintent_tags: [auth-demo]\nrequires_auth: true\n---\nbody\n",
    )
    .unwrap();
    skill_resolver::mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        skill_resolver::OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();

    let missing =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
            .unwrap();
    let card = missing
        .catalog
        .iter()
        .find(|card| card.skill_id == skill_id)
        .unwrap();
    assert_eq!(card.auth_state, AuthState::Unknown);
    assert!(card
        .reason_codes
        .iter()
        .any(|reason| reason == "auth_required"));
    assert!(!missing
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == skill_id));

    let auth_path = runtime.join("auth-state/codex.json");
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(
        &auth_path,
        format!(r#"{{"skills":{{"{skill_id}":"satisfied"}}}}"#),
    )
    .unwrap();
    let ready =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
            .unwrap();
    let card = ready
        .catalog
        .iter()
        .find(|card| card.skill_id == skill_id)
        .unwrap();
    assert_eq!(card.auth_state, AuthState::Satisfied);
    assert_eq!(card.availability, AvailabilityState::Ready);
    assert!(ready
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == skill_id));
    assert!(!std::fs::read_to_string(&auth_path)
        .unwrap()
        .contains("secret"));

    std::fs::write(
        &auth_path,
        format!(r#"{{"skills":{{"{skill_id}":"satisfied"}},"token":"secret"}}"#),
    )
    .unwrap();
    let rejected_secret_bearing_state =
        skill_resolver::build_capability_snapshot_with_roots(&root, "codex", &runtime, &home)
            .unwrap();
    let card = rejected_secret_bearing_state
        .catalog
        .iter()
        .find(|card| card.skill_id == skill_id)
        .unwrap();
    assert_eq!(card.auth_state, AuthState::Unknown);
    assert!(!rejected_secret_bearing_state
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == skill_id));

    let _ = std::fs::remove_dir_all(base);
}
