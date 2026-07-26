use super::*;
#[allow(unused_imports)]
use super::{
    authority::*, catalog::*, hashing::*, overlay_transaction::*, private_store::*,
    snapshot_compiler::*, snapshot_validation::*, usage_ledger::*,
};
use crate::skill_body::console::{HealthStatus, ManagedCapability, ManagedStatus, RegistryStatus};

fn active_skill() -> ActiveSkill {
    ActiveSkill {
        skill_id: "codebase-design".to_string(),
        invoke_hint: "[skill: codebase-design]".to_string(),
        allowed_entrypoints: vec!["module-design".to_string()],
        intent_tags: vec!["module-design".to_string()],
        legacy_demands: Vec::new(),
        source_hash: "sha256:source".to_string(),
    }
}

fn card() -> SkillCard {
    SkillCard {
        skill_id: "codebase-design".to_string(),
        display_name: "Codebase Design".to_string(),
        summary: "Deep module design".to_string(),
        intent_tags: vec!["module-design".to_string()],
        positive_examples: vec!["设计这个模块接口".to_string()],
        negative_examples: vec!["解释这个模块".to_string()],
        entrypoints: vec!["module-design".to_string()],
        source_kind: SkillSourceKind::Suite,
        governance: GovernanceState::Active,
        availability: AvailabilityState::Ready,
        reason_codes: Vec::new(),
        requires_auth: false,
        auth_state: AuthState::NotRequired,
        activity: ActivityState::Unobserved,
        version: "registry".to_string(),
        source_hash: "sha256:source".to_string(),
    }
}

#[test]
fn relative_suite_skill_sources_resolve_from_manifest_root() {
    let base =
        std::env::temp_dir().join(format!("ags-relative-suite-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let body = base.join("global-skills/demo");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: demo\ndescription: Stable suite metadata.\nintent_tags: [demo]\n---\n",
    )
    .unwrap();
    let capability = ManagedCapability {
        kind: ManagedKind::Skill,
        name: "demo".to_string(),
        source: Some("global-skills/demo".to_string()),
        profile: Some("required".to_string()),
        managed_status: ManagedStatus::SuiteManaged,
        registry_status: RegistryStatus::Registered,
        canonical_present: true,
        expected_hosts: Vec::new(),
        host_visibility: Vec::new(),
        health_status: HealthStatus::Healthy,
        actions: Vec::new(),
        risk_notes: Vec::new(),
        routing: None,
    };

    let metadata = load_skill_file_metadata(&base, &capability);
    assert_eq!(metadata.description, "Stable suite metadata.");
    assert_eq!(metadata.intent_tags, vec!["demo"]);
    assert_eq!(
        source_hash(&base, &capability),
        hash_skill_source(&body).unwrap()
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn exact_skill_and_entrypoint_resolution() {
    let table = ActiveSkillTable::new("codex", "sha256:snapshot", vec![active_skill()]).unwrap();
    let selection = resolve_skill(
        "codebase-design",
        Some("module-design"),
        "sha256:snapshot",
        &table,
    )
    .unwrap();
    assert_eq!(selection.skill_id, "codebase-design");
    assert_eq!(selection.snapshot_hash, "sha256:snapshot");
}

#[test]
fn exact_skill_resolution_rejects_a_different_snapshot_hash() {
    let table = ActiveSkillTable::new("codex", "sha256:expected", vec![active_skill()]).unwrap();
    assert!(matches!(
        resolve_skill("codebase-design", None, "sha256:stale", &table),
        Err(ResolveError::SnapshotHashMismatch { .. })
    ));
}

#[test]
fn entrypoint_fails_closed_without_fallback() {
    let table = ActiveSkillTable::new("codex", "sha256:snapshot", vec![active_skill()]).unwrap();
    assert!(matches!(
        resolve_skill(
            "codebase-design",
            Some("brainstorming"),
            "sha256:snapshot",
            &table
        ),
        Err(ResolveError::EntrypointNotAllowed { .. })
    ));
}

#[test]
fn snapshot_hash_is_deterministic_and_binds_catalog() {
    let one = HostCapabilitySnapshot::new(
        "codex",
        "sha256:registry",
        "sha256:overlay",
        "sha256:runtime",
        vec![card()],
        "https://example.invalid/manifest.yaml",
        "sha256:third-party",
        vec![],
        vec![active_skill()],
    )
    .unwrap();
    let two = HostCapabilitySnapshot::new(
        "codex",
        "sha256:registry",
        "sha256:overlay",
        "sha256:runtime",
        vec![card()],
        "https://example.invalid/manifest.yaml",
        "sha256:third-party",
        vec![],
        vec![active_skill()],
    )
    .unwrap();
    assert_eq!(one.snapshot_hash, two.snapshot_hash);
    assert!(one.validate_integrity("codex").is_ok());
}

#[test]
fn snapshot_deserialization_rejects_unknown_top_level_and_nested_fields() {
    let snapshot = HostCapabilitySnapshot::new(
        "codex",
        "sha256:registry",
        "sha256:overlay",
        "sha256:runtime",
        vec![card()],
        "https://example.invalid/manifest.yaml",
        "sha256:third-party",
        vec![],
        vec![active_skill()],
    )
    .unwrap();
    let mut top = serde_json::to_value(&snapshot).unwrap();
    top["raw_prompt"] = serde_json::json!("must not be ignored");
    assert!(serde_json::from_value::<HostCapabilitySnapshot>(top).is_err());

    let mut nested = serde_json::to_value(snapshot).unwrap();
    nested["catalog"][0]["raw_prompt"] = serde_json::json!("must not be ignored");
    assert!(serde_json::from_value::<HostCapabilitySnapshot>(nested).is_err());
}

#[test]
fn activity_thresholds_are_advisory_only() {
    let now = 100 * 86_400;
    assert_eq!(
        activity_for_skill("x", &[], now, Some(60 * 86_400)),
        ActivityState::Cold
    );
    assert_eq!(
        activity_for_skill("x", &[], now, Some(90 * 86_400)),
        ActivityState::Unobserved
    );
}

#[test]
fn activity_does_not_change_catalog_or_snapshot_hash() {
    let mut cold_card = card();
    cold_card.activity = ActivityState::Cold;
    let cold = HostCapabilitySnapshot::new(
        "codex",
        "sha256:registry",
        "sha256:overlay",
        "sha256:runtime",
        vec![cold_card],
        "https://example.invalid/manifest.yaml",
        "sha256:third-party",
        vec![],
        vec![active_skill()],
    )
    .unwrap();
    let warm = HostCapabilitySnapshot::new(
        "codex",
        "sha256:registry",
        "sha256:overlay",
        "sha256:runtime",
        vec![card()],
        "https://example.invalid/manifest.yaml",
        "sha256:third-party",
        vec![],
        vec![active_skill()],
    )
    .unwrap();
    assert_eq!(cold.catalog_hash, warm.catalog_hash);
    assert_eq!(cold.snapshot_hash, warm.snapshot_hash);
}

#[test]
fn reason_code_contract_covers_all_governance_failures() {
    for required in [
        "candidate_requires_adoption",
        "registry_not_routable",
        "retired",
        "canonical_missing",
        "host_not_visible",
        "health_degraded",
        "auth_required",
        "metadata_incomplete",
        "source_hash_changed",
        "snapshot_stale",
    ] {
        assert!(SKILL_REASON_CODES.contains(&required), "missing {required}");
    }
}

#[cfg(unix)]
#[test]
fn private_overlay_adopt_ignore_and_rollback_are_versioned_and_private() {
    use std::os::unix::fs::PermissionsExt;

    let base = std::env::temp_dir().join(format!("ags-overlay-lifecycle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let runtime = base.join("runtime");
    let home = base.join("home");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let skill_id = "machine-private-demo";
    let body = home.join(".agents/skills").join(skill_id);
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
            body.join("SKILL.md"),
            "---\nname: machine-private-demo\ndescription: A private test skill.\nintent_tags: [private-demo]\n---\nbody\n",
        )
        .unwrap();

    let dry_run = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Adopt,
        None,
        false,
    )
    .unwrap();
    assert!(dry_run.dry_run && dry_run.changed && !dry_run.applied);
    assert!(!overlay_path(&runtime).exists());

    let adopted = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    assert_eq!(adopted.overlay_revision, 1);
    assert_eq!(
        std::fs::metadata(overlay_path(&runtime))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(overlay_events_path(&runtime))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    // An adopted entry remains manageable even when its downloaded body
    // later disappears; ignore uses the versioned overlay metadata.
    std::fs::remove_dir_all(&body).unwrap();

    let ignored = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Ignore,
        None,
        true,
    )
    .unwrap();
    assert_eq!(ignored.overlay_revision, 2);
    assert_eq!(
        load_user_overlay(&runtime).unwrap().entries[0].state,
        OverlayEntryState::Ignored
    );

    let rolled_back = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Rollback,
        Some(1),
        true,
    )
    .unwrap();
    assert_eq!(rolled_back.overlay_revision, 3);
    let overlay = load_user_overlay(&runtime).unwrap();
    assert_eq!(overlay.entries[0].state, OverlayEntryState::Active);
    assert_eq!(overlay.entries[0].revision, 3);
    assert_eq!(load_overlay_mutation_receipts(&runtime).unwrap().len(), 3);

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn private_overlay_cannot_shadow_official_registry() {
    let base = std::env::temp_dir().join(format!(
        "ags-overlay-official-precedence-{}",
        std::process::id()
    ));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let error = mutate_user_overlay(
        &root,
        &base.join("runtime"),
        &base.join("home"),
        "codex",
        "diagnosing-bugs",
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap_err();
    assert_eq!(error, "official_registry_precedence");
    assert!(!overlay_path(&base.join("runtime")).exists());
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn imported_source_registry_candidate_becomes_active_when_linked_and_adopted() {
    let base = std::env::temp_dir().join(format!("ags-imported-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let runtime = base.join("runtime");
    let home = base.join("home");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let body = user_skill_body_root(&runtime).join("apple-design");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
            body.join("SKILL.md"),
            "---\nname: apple-design\ndescription: Apple design guidance.\nintent_tags: [apple-design, design]\n---\n",
        )
        .unwrap();
    let source_hash = hash_skill_source(&body).unwrap();
    write_capability_snapshot_with_roots(&root, "claude-code", &runtime, &home).unwrap();
    let mut sources = UserSourceRegistry {
        revision: 1,
        ..Default::default()
    };
    sources.entries.push(UserSourceEntry {
        skill_id: "apple-design".to_string(),
        source_kind: UserSourceKind::Local,
        source: body.display().to_string(),
        resolved_ref: None,
        subdir: None,
        source_hash,
        license: "MIT".to_string(),
        canonical_path: body.display().to_string(),
        audit_version: USER_SOURCE_AUDIT_VERSION.to_string(),
        target_hosts: vec!["codex".to_string(), "omp".to_string()],
        display_name: "apple-design".to_string(),
        summary: "Apple design guidance.".to_string(),
        intent_tags: vec!["apple-design".to_string(), "design".to_string()],
        entrypoints: Vec::new(),
        requires_auth: false,
    });
    write_user_source_registry(&runtime, &sources).unwrap();
    let host_entry = home.join(".codex/skills/apple-design");
    std::fs::create_dir_all(host_entry.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&body, &host_entry).unwrap();
    let omp_entry = home.join(".omp/agent/skills/apple-design");
    std::fs::create_dir_all(omp_entry.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&body, &omp_entry).unwrap();

    mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        "apple-design",
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    let snapshot = build_capability_snapshot_with_roots(&root, "codex", &runtime, &home).unwrap();
    assert!(snapshot
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    assert!(snapshot.catalog.iter().any(|card| {
        card.skill_id == "apple-design"
            && card.governance == GovernanceState::Active
            && card.availability == AvailabilityState::Ready
    }));
    let omp_snapshot = build_capability_snapshot_with_roots(&root, "omp", &runtime, &home).unwrap();
    assert!(omp_snapshot
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    let shared_entry = home.join(".agents/skills/apple-design");
    std::fs::create_dir_all(shared_entry.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&body, &shared_entry).unwrap();
    let omp_duplicate =
        build_capability_snapshot_with_roots(&root, "omp", &runtime, &home).unwrap();
    assert!(!omp_duplicate
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    assert!(omp_duplicate.catalog.iter().any(|card| {
        card.skill_id == "apple-design"
            && card
                .reason_codes
                .iter()
                .any(|reason| reason == "host_not_visible")
    }));
    std::fs::remove_file(&shared_entry).unwrap();
    let shared_shadow = base.join("shared-shadow/apple-design");
    std::fs::create_dir_all(&shared_shadow).unwrap();
    std::fs::write(
        shared_shadow.join("SKILL.md"),
        "---\nname: apple-design\ndescription: unrelated shared shadow.\n---\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&shared_shadow, &shared_entry).unwrap();
    let omp_conflict = build_capability_snapshot_with_roots(&root, "omp", &runtime, &home).unwrap();
    assert!(!omp_conflict
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    std::fs::remove_file(&shared_entry).unwrap();
    assert!(
        load_static_snapshot(&runtime, "claude-code").is_ok(),
        "a codex/omp adoption must not stale the persisted Claude snapshot"
    );
    let original_runtime_hash = snapshot.runtime_hash;
    std::fs::remove_file(&host_entry).unwrap();
    let shadow = base.join("shadow/apple-design");
    std::fs::create_dir_all(&shadow).unwrap();
    std::fs::write(
            shadow.join("SKILL.md"),
            "---\nname: apple-design\ndescription: unrelated shadow.\nintent_tags: [apple-design]\n---\n",
        )
        .unwrap();
    std::os::unix::fs::symlink(&shadow, &host_entry).unwrap();
    let shadowed = build_capability_snapshot_with_roots(&root, "codex", &runtime, &home).unwrap();
    assert!(!shadowed
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    assert!(shadowed.catalog.iter().any(|card| {
        card.skill_id == "apple-design"
            && card
                .reason_codes
                .iter()
                .any(|reason| reason == "host_not_visible")
    }));
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: apple-design\ndescription: tampered.\nintent_tags: [apple-design]\n---\n",
    )
    .unwrap();
    let tampered = build_capability_snapshot_with_roots(&root, "codex", &runtime, &home).unwrap();
    assert_ne!(tampered.runtime_hash, original_runtime_hash);
    assert!(!tampered
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "apple-design"));
    assert!(tampered.catalog.iter().any(|card| {
        card.skill_id == "apple-design"
            && card
                .reason_codes
                .iter()
                .any(|reason| reason == "source_hash_changed")
    }));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn source_registry_rejects_frontmatter_name_alias() {
    let base =
        std::env::temp_dir().join(format!("ags-source-registry-alias-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let runtime = base.join("runtime");
    let body = user_skill_body_root(&runtime).join("apple-design");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: another-skill\ndescription: alias attempt\n---\n",
    )
    .unwrap();
    let registry = UserSourceRegistry {
        schema_version: USER_SOURCE_REGISTRY_SCHEMA_VERSION.to_string(),
        revision: 1,
        entries: vec![UserSourceEntry {
            skill_id: "apple-design".to_string(),
            source_kind: UserSourceKind::Local,
            source: body.display().to_string(),
            resolved_ref: None,
            subdir: None,
            source_hash: hash_skill_source(&body).unwrap(),
            license: "MIT".to_string(),
            canonical_path: body.display().to_string(),
            audit_version: USER_SOURCE_AUDIT_VERSION.to_string(),
            target_hosts: vec!["codex".to_string()],
            display_name: "apple-design".to_string(),
            summary: "alias attempt".to_string(),
            intent_tags: vec!["apple-design".to_string()],
            entrypoints: Vec::new(),
            requires_auth: false,
        }],
    };
    write_user_source_registry(&runtime, &registry).unwrap();
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("different canonical name"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn source_registry_rejects_unknown_license_audit_and_unpinned_provenance() {
    let base = std::env::temp_dir().join(format!(
        "ags-source-registry-provenance-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let runtime = base.join("runtime");
    let body = user_skill_body_root(&runtime).join("apple-design");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
        body.join("SKILL.md"),
        "---\nname: apple-design\ndescription: provenance test\n---\n",
    )
    .unwrap();
    let mut entry = UserSourceEntry {
        skill_id: "apple-design".to_string(),
        source_kind: UserSourceKind::Local,
        source: body.display().to_string(),
        resolved_ref: None,
        subdir: None,
        source_hash: hash_skill_source(&body).unwrap(),
        license: "MIT".to_string(),
        canonical_path: body.display().to_string(),
        audit_version: USER_SOURCE_AUDIT_VERSION.to_string(),
        target_hosts: vec!["codex".to_string()],
        display_name: "apple-design".to_string(),
        summary: "provenance test".to_string(),
        intent_tags: vec!["apple-design".to_string()],
        entrypoints: Vec::new(),
        requires_auth: false,
    };
    let write = |entry: &UserSourceEntry| {
        write_user_source_registry(
            &runtime,
            &UserSourceRegistry {
                schema_version: USER_SOURCE_REGISTRY_SCHEMA_VERSION.to_string(),
                revision: 1,
                entries: vec![entry.clone()],
            },
        )
        .unwrap();
    };
    write(&entry);
    assert!(load_user_source_registry(&runtime).is_ok());

    entry.target_hosts = vec!["omp".to_string()];
    write(&entry);
    assert!(load_user_source_registry(&runtime).is_ok());
    entry.target_hosts = vec!["codex".to_string()];

    entry.license = "unknown".to_string();
    write(&entry);
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("invalid metadata"));

    entry.license = "MIT".to_string();
    entry.audit_version = "future-or-tampered".to_string();
    write(&entry);
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("invalid metadata"));

    entry.audit_version = USER_SOURCE_AUDIT_VERSION.to_string();
    entry.source_kind = UserSourceKind::Github;
    entry.source = "https://github.com/acme/skills/tree/main/skills/apple-design".to_string();
    entry.resolved_ref = Some("a".repeat(40));
    entry.subdir = Some("skills/apple-design".to_string());
    write(&entry);
    assert!(load_user_source_registry(&runtime).is_ok());

    entry.source = "nonsense".to_string();
    entry.resolved_ref = Some("main".to_string());
    entry.subdir = None;
    write(&entry);
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("not safely pinned"));

    entry.source_kind = UserSourceKind::Local;
    entry.source = body.display().to_string();
    write(&entry);
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("invalid provenance"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn source_registry_rejects_body_outside_private_store() {
    let base =
        std::env::temp_dir().join(format!("ags-source-registry-escape-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let runtime = base.join("runtime");
    let body_root = user_skill_body_root(&runtime);
    std::fs::create_dir_all(&body_root).unwrap();
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("SKILL.md"),
        "---\nname: escaped\ndescription: escaped\n---\n",
    )
    .unwrap();
    let registry = UserSourceRegistry {
        schema_version: USER_SOURCE_REGISTRY_SCHEMA_VERSION.to_string(),
        revision: 1,
        entries: vec![UserSourceEntry {
            skill_id: "escaped".to_string(),
            source_kind: UserSourceKind::Local,
            source: outside.display().to_string(),
            resolved_ref: None,
            subdir: None,
            source_hash: hash_skill_source(&outside).unwrap(),
            license: "MIT".to_string(),
            canonical_path: outside.display().to_string(),
            audit_version: USER_SOURCE_AUDIT_VERSION.to_string(),
            target_hosts: vec!["codex".to_string()],
            display_name: "escaped".to_string(),
            summary: "escaped".to_string(),
            intent_tags: vec!["escaped".to_string()],
            entrypoints: Vec::new(),
            requires_auth: false,
        }],
    };
    let path = user_source_registry_path(&runtime);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_yaml::to_string(&registry).unwrap()).unwrap();
    assert!(load_user_source_registry(&runtime)
        .unwrap_err()
        .contains("escapes the private body store"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn unreadable_snapshot_backup_blocks_before_mutating_previous_revision() {
    let base = std::env::temp_dir().join(format!(
        "ags-overlay-snapshot-rollback-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = base.join("runtime");
    let home = base.join("home");
    let skill_id = "snapshot-rollback-demo";
    let body = home.join(".agents/skills").join(skill_id);
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(
            body.join("SKILL.md"),
            "---\nname: snapshot-rollback-demo\ndescription: rollback test\nintent_tags: [rollback]\n---\n",
        )
        .unwrap();
    mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    let overlay_before = std::fs::read(overlay_path(&runtime)).unwrap();
    let receipts_before = std::fs::read(overlay_events_path(&runtime)).unwrap();
    let saved_snapshot = snapshot_path(&runtime, "codex");
    std::fs::remove_file(&saved_snapshot).unwrap();
    std::fs::create_dir(&saved_snapshot).unwrap();

    let error = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        skill_id,
        OverlayMutationOperation::Ignore,
        None,
        true,
    )
    .unwrap_err();
    assert!(error.contains("cannot read existing private file"));
    assert_eq!(
        std::fs::read(overlay_path(&runtime)).unwrap(),
        overlay_before
    );
    assert_eq!(
        std::fs::read(overlay_events_path(&runtime)).unwrap(),
        receipts_before
    );
    assert!(saved_snapshot.is_dir());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn receipt_sync_failure_atomically_restores_overlay_and_snapshot() {
    let base =
        std::env::temp_dir().join(format!("ags-overlay-receipt-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = base.join("runtime");
    let home = base.join("home");
    for skill_id in ["receipt-atomic-one", "receipt-atomic-two"] {
        let body = home.join(".agents/skills").join(skill_id);
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(
                body.join("SKILL.md"),
                format!(
                    "---\nname: {skill_id}\ndescription: receipt atomic test\nintent_tags: [atomic]\n---\n"
                ),
            )
            .unwrap();
    }
    mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        "receipt-atomic-one",
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap();
    let overlay_before = std::fs::read(overlay_path(&runtime)).unwrap();
    let snapshot_before = std::fs::read(snapshot_path(&runtime, "codex")).unwrap();
    let receipts_before = std::fs::read(overlay_events_path(&runtime)).unwrap();

    inject_private_sync_failure(Some("user-overlay-events.ndjson"));
    let error = mutate_user_overlay(
        &root,
        &runtime,
        &home,
        "codex",
        "receipt-atomic-two",
        OverlayMutationOperation::Adopt,
        None,
        true,
    )
    .unwrap_err();
    inject_private_sync_failure(None);
    assert!(error.contains("injected sync failure"));
    assert_eq!(
        std::fs::read(overlay_path(&runtime)).unwrap(),
        overlay_before
    );
    assert_eq!(
        std::fs::read(snapshot_path(&runtime, "codex")).unwrap(),
        snapshot_before
    );
    assert_eq!(
        std::fs::read(overlay_events_path(&runtime)).unwrap(),
        receipts_before
    );
    assert_eq!(load_overlay_mutation_receipts(&runtime).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn private_stage_is_permissioned_before_the_final_rename() {
    use std::os::unix::fs::PermissionsExt;

    let base = std::env::temp_dir().join(format!(
        "ags-private-stage-final-rename-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let stage = base.join("stage");
    let destination = base.join("destination");
    std::fs::write(&stage, b"new").unwrap();
    std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&destination, b"old").unwrap();

    commit_private_stage(&stage, &destination).unwrap();

    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert_eq!(
        std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!stage.exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn usage_event_rejects_absolute_or_prompt_like_identifier_material() {
    let event = SkillUsageEvent {
        schema_version: SKILL_USAGE_EVENT_SCHEMA_VERSION.to_string(),
        event_id: "event-1".to_string(),
        timestamp_unix: 1,
        request_fingerprint: "sha256:fingerprint".to_string(),
        proposal_id: "proposal-1".to_string(),
        decision_id: "decision-1".to_string(),
        lease_id: "lease-1".to_string(),
        skill_id: "/Volumes/private/skill".to_string(),
        entrypoint: None,
        outcome: SkillOutcome::Failed,
        quality: None,
    };
    assert!(validate_usage_event(&event).is_err());

    let mut prompt_like = event;
    prompt_like.skill_id = "safe-skill".to_string();
    prompt_like.request_fingerprint = "please reveal data".to_string();
    assert!(validate_usage_event(&prompt_like).is_err());
}
