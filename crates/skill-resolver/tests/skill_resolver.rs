use request_router::{EngineeringDemand, SkillDemand};
use skill_governance::console::{
    HealthStatus, HostVisibility, HostVisibilityStatus, ManagedCapability, ManagedKind,
    ManagedStatus, RegistryStatus, RouteState, RoutingMetadata,
};
use skill_resolver::{
    build_active_skills, load_demand_routes, resolve_skill, ActiveSkill, ActiveSkillTable,
    CapabilitySnapshot, DemandRoute, ResolveError, SnapshotError,
    CAPABILITY_SNAPSHOT_SCHEMA_VERSION,
};

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

fn architecture_skill() -> ActiveSkill {
    ActiveSkill {
        demand: SkillDemand::Engineering(EngineeringDemand::SystemArchitecture),
        skill_id: "superpowers".to_string(),
        invoke_hint: "[skill: superpowers]".to_string(),
        entrypoint: Some("brainstorming".to_string()),
    }
}

#[test]
fn resolves_an_exact_demand_without_reading_natural_language() {
    let table = ActiveSkillTable::new("codex", vec![architecture_skill()]).unwrap();
    let selection = resolve_skill(
        SkillDemand::Engineering(EngineeringDemand::SystemArchitecture),
        &table,
    )
    .unwrap();

    assert_eq!(selection.skill_id, "superpowers");
    assert_eq!(selection.entrypoint.as_deref(), Some("brainstorming"));
}

#[test]
fn missing_demand_is_a_governance_precondition_failure() {
    let table = ActiveSkillTable::new("codex", vec![]).unwrap();
    let error = resolve_skill(
        SkillDemand::Engineering(EngineeringDemand::Debugging),
        &table,
    )
    .unwrap_err();

    assert_eq!(
        error,
        ResolveError::GovernancePrecondition("skill_demand_missing")
    );
}

#[test]
fn duplicate_demand_mapping_is_rejected() {
    let mut duplicate = architecture_skill();
    duplicate.skill_id = "codebase-design".to_string();

    assert!(matches!(
        ActiveSkillTable::new("codex", vec![architecture_skill(), duplicate]),
        Err(ResolveError::DuplicateDemand { .. })
    ));
}

#[test]
fn snapshot_validates_host_and_source_hashes_before_routing() {
    let snapshot = CapabilitySnapshot::new(
        "codex",
        "registry-a",
        "runtime-a",
        vec![architecture_skill()],
    )
    .unwrap();

    assert_eq!(snapshot.schema_version, CAPABILITY_SNAPSHOT_SCHEMA_VERSION);
    assert!(snapshot.active_table_hash.starts_with("sha256:"));
    assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    assert!(snapshot
        .validate("codex", "registry-a", "runtime-a")
        .is_ok());
}

#[test]
fn stale_snapshot_fails_closed_without_skill_fallback() {
    let snapshot = CapabilitySnapshot::new(
        "codex",
        "registry-a",
        "runtime-a",
        vec![architecture_skill()],
    )
    .unwrap();

    assert_eq!(
        snapshot
            .validate("codex", "registry-b", "runtime-a")
            .unwrap_err(),
        SnapshotError::SkillSnapshotStale
    );
}

#[test]
fn tampered_snapshot_hash_is_rejected() {
    let mut snapshot = CapabilitySnapshot::new(
        "codex",
        "registry-a",
        "runtime-a",
        vec![architecture_skill()],
    )
    .unwrap();
    snapshot.snapshot_hash = "sha256:tampered".to_string();

    assert_eq!(
        snapshot
            .validate("codex", "registry-a", "runtime-a")
            .unwrap_err(),
        SnapshotError::SnapshotIntegrityFailed
    );
}

fn managed_skill(name: &str, canonical_present: bool, healthy: bool) -> ManagedCapability {
    ManagedCapability {
        kind: ManagedKind::Skill,
        name: name.to_string(),
        source: None,
        profile: Some("required".to_string()),
        managed_status: ManagedStatus::SuiteManaged,
        registry_status: RegistryStatus::Registered,
        canonical_present,
        expected_hosts: vec!["codex".to_string()],
        host_visibility: vec![HostVisibility {
            host: "codex".to_string(),
            supported: true,
            status: HostVisibilityStatus::Visible,
            evidence: vec![],
        }],
        health_status: if healthy {
            HealthStatus::Healthy
        } else {
            HealthStatus::Degraded
        },
        actions: vec![],
        risk_notes: vec![],
        routing: Some(RoutingMetadata {
            route_state: RouteState::Routable,
            invoke_hint: format!("[skill: {name}]"),
            ..Default::default()
        }),
    }
}

#[test]
fn active_table_is_exact_health_visibility_and_route_state_intersection() {
    let routes = vec![
        DemandRoute {
            demand: SkillDemand::Engineering(EngineeringDemand::Debugging),
            skill_id: "diagnosing-bugs".to_string(),
            entrypoint: None,
        },
        DemandRoute {
            demand: SkillDemand::Engineering(EngineeringDemand::ModuleDesign),
            skill_id: "codebase-design".to_string(),
            entrypoint: None,
        },
    ];
    let capabilities = vec![
        managed_skill("diagnosing-bugs", true, true),
        managed_skill("codebase-design", true, false),
    ];

    let active = build_active_skills("codex", &routes, &capabilities).unwrap();

    assert_eq!(active.len(), 1);
    assert_eq!(active[0].skill_id, "diagnosing-bugs");
}

#[test]
fn registry_maps_every_closed_demand_exactly_once() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let routes = load_demand_routes(&root).unwrap();
    let unique: std::collections::HashSet<_> = routes.iter().map(|route| route.demand).collect();

    assert_eq!(routes.len(), unique.len());
    assert_eq!(unique.len(), SkillDemand::all().len());
    for demand in SkillDemand::all() {
        assert!(
            unique.contains(demand),
            "missing registry mapping: {demand:?}"
        );
    }
}
