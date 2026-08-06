use super::*;

fn fixture_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ags-onboarding-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::write(
            root.join("manifests/onboarding-public.yaml"),
            "profile: public\nexcluded_capabilities: [evomap, gep]\nrequired_mcps: []\noptional_mcps: []\ndeveloper_tools: []\n",
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
