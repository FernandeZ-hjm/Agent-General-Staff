use super::*;
use super::{assess::skill_item, util::pin_github_source};

fn fixture_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ags-onboarding-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::write(
            root.join("manifests/onboarding-public.yaml"),
            "profile: public\nexcluded_capabilities: [evomap, gep]\nrequired_mcps: []\noptional_mcps: []\ndeveloper_tools: []\n",
        )
        .unwrap();
    std::fs::write(
        root.join("manifests/skill-recommendations.yaml"),
        "skills: []\n",
    )
    .unwrap();
    root
}

#[test]
fn uninitialized_project_gets_closed_init_action() {
    let root = fixture_root("init");
    let home = root.join("home");
    let target = root.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let exe = root.join("ags");
    std::fs::write(&exe, "").unwrap();
    let plan = assess_public(&AssessContext {
        source_root: &root,
        home: &home,
        target: &target,
        host: "codex",
        ags_executable: &exe,
        mcp_connected: true,
        host_registered: Some(true),
        registered_mcp_ids: &[],
        active_skill_ids: &[],
    })
    .unwrap();
    assert!(plan.bootstrap_required);
    assert!(matches!(
        find_action(&plan, "project-init").unwrap(),
        OnboardingAction::ProjectInit { .. }
    ));
    assert!(!plan.excluded_capabilities.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pins_github_tree_to_reviewed_commit() {
    let revision = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(
        pin_github_source(
            "https://github.com/acme/skills",
            revision,
            Some("skills/review")
        ),
        format!("https://github.com/acme/skills/tree/{revision}/skills/review")
    );
}

#[test]
fn onboarding_rollback_is_manual_and_action_specific() {
    let registration = rollback_advice(&OnboardingAction::RegisterAgsMcp {
        registrar: "claude".to_string(),
        executable: "/tmp/ags".to_string(),
    });
    assert_eq!(
        registration[0].inverse_command.as_deref(),
        Some("claude mcp remove -s user ags")
    );

    let npm = rollback_advice(&OnboardingAction::InstallNpmCli {
        package: "@scope/tool@1.2.3".to_string(),
        integrity: "sha512-test".to_string(),
    });
    assert_eq!(
        npm[0].inverse_command.as_deref(),
        Some("npm uninstall --global @scope/tool")
    );
}

#[test]
fn active_snapshot_prevents_duplicate_skill_adoption() {
    let root = fixture_root("active-skill");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let skill = ThirdPartyCapability {
        id: "review".to_string(),
        kind: CapabilityKind::Skill,
        name: "Review".to_string(),
        profiles: vec!["public".to_string()],
        required: false,
        tier: "flow".to_string(),
        purpose: "review changes".to_string(),
        risk: "low".to_string(),
        requires_auth: false,
        source: manifest::CapabilitySource {
            manager: "git".to_string(),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            repository: Some("https://github.com/acme/skills".to_string()),
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        install: manifest::InstallContract {
            strategy: "ags-skill-adopt".to_string(),
            ..Default::default()
        },
        routing: manifest::RoutingContract {
            route_state: "routable".to_string(),
            invoke_hint: Some("[skill: review]".to_string()),
            ..Default::default()
        },
        mcp: None,
        hook: None,
    };
    let active = vec!["review".to_string()];
    let exe = root.join("ags");
    let target = root.join("project");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(&exe, "").unwrap();
    let context = AssessContext {
        source_root: &root,
        home: &home,
        target: &target,
        host: "codex",
        ags_executable: &exe,
        mcp_connected: true,
        host_registered: Some(true),
        registered_mcp_ids: &[],
        active_skill_ids: &active,
    };
    let item = skill_item(&context, &skill);
    assert_eq!(item.state, ComponentState::ActiveReady);
    assert!(item.action.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn visible_but_unadmitted_skill_is_not_reported_ready_or_reinstalled() {
    let root = fixture_root("visible-skill");
    let home = root.join("home");
    let body = home.join(".codex/skills/review");
    std::fs::create_dir_all(&body).unwrap();
    std::fs::write(body.join("SKILL.md"), "# review").unwrap();
    let skill = ThirdPartyCapability {
        id: "review".to_string(),
        kind: CapabilityKind::Skill,
        name: "Review".to_string(),
        profiles: vec!["public".to_string()],
        required: false,
        tier: "flow".to_string(),
        purpose: "review changes".to_string(),
        risk: "low".to_string(),
        requires_auth: false,
        source: Default::default(),
        install: Default::default(),
        routing: Default::default(),
        mcp: None,
        hook: None,
    };
    let exe = root.join("ags");
    let target = root.join("project");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(&exe, "").unwrap();
    let context = AssessContext {
        source_root: &root,
        home: &home,
        target: &target,
        host: "codex",
        ags_executable: &exe,
        mcp_connected: true,
        host_registered: Some(true),
        registered_mcp_ids: &[],
        active_skill_ids: &[],
    };
    let item = skill_item(&context, &skill);
    assert_eq!(item.state, ComponentState::VisibleNotReady);
    assert!(item.action.is_none());
    let _ = std::fs::remove_dir_all(root);
}
