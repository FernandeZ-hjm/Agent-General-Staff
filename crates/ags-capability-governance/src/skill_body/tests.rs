use super::read_model::extract_schema_version;
use super::*;

fn repo_root() -> std::path::PathBuf {
    // Tests run from crates/ags-capability-governance/, so ../.. reaches the repo root
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("../..")
}

#[test]
fn test_schema_version() {
    assert_eq!(SCHEMA_VERSION, "2.0-skill");
}

#[test]
fn test_scan_migrated_manifest() {
    // Scan the edition's migrated suite manifest. Private/stable ship the
    // governed bodies; the public edition deliberately ships no skill bodies.
    let root = repo_root();
    let result = scan_skills(&root);
    assert_eq!(result.schema_version, SCHEMA_VERSION);
    if !root.join("global-skills").is_dir() {
        assert_eq!(result.summary.available, 0);
        assert_eq!(result.summary.personal, 0);
        return;
    }
    // 25 active required skills: retired duplicate routes stay out of the
    // active suite, and the 14 Superpowers playbooks are internal resources
    // behind one host-visible parent.
    assert_eq!(result.summary.available, 25);
    assert!(result
        .skills
        .iter()
        .all(|skill| !skill.name.starts_with("lark-")));
    for retained in ["claude-mem", "guizang-ppt-skill"] {
        assert!(result.skills.iter().any(|skill| {
            skill.name == retained && matches!(skill.status, SkillStatus::Optional)
        }));
    }
    assert_eq!(result.summary.personal, 7);
    assert_eq!(result.summary.disabled, 1);
}

#[test]
fn test_scan_personal_manifest_metadata() {
    let root = repo_root();
    let result = scan_skills(&root);
    if !root.join("skill-packs/personal").is_dir() {
        assert_eq!(result.summary.personal, 0);
        assert!(result
            .skills
            .iter()
            .all(|skill| !matches!(skill.status, SkillStatus::Personal)));
        return;
    }
    let skill = result
        .skills
        .iter()
        .find(|s| s.name == "辐射塔罗牌")
        .expect("personal skill should be present");
    assert_eq!(skill.status, SkillStatus::Personal);
    assert_eq!(
        skill.source.as_deref(),
        Some("skill-packs/personal/辐射塔罗牌")
    );
    assert!(skill.hash.is_some());
    assert!(skill.adopted.is_some());
}

#[test]
fn test_scan_finds_suite_name() {
    let root = repo_root();
    let result = scan_skills(&root);
    assert!(!result.suite_name.is_empty());
}

#[test]
fn test_check_governance_files() {
    let root = repo_root();
    let result = check_skills(&root);
    assert_eq!(result.schema_version, SCHEMA_VERSION);
    assert!(result.governance_files.suite_manifest.present);
    assert!(result.governance_files.suite_manifest.parseable);
}

#[test]
fn optional_manifest_skills_do_not_require_adoption_log_entries() {
    let base =
        std::env::temp_dir().join(format!("ags-skill-check-optional-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("governance")).unwrap();
    std::fs::create_dir_all(base.join("manifests")).unwrap();
    std::fs::write(
        base.join("governance/skill-adoption-log.yaml"),
        "schema_version: \"2.0-skill\"\nentries: []\n",
    )
    .unwrap();
    std::fs::write(
        base.join("governance/skill-ignore-list.yaml"),
        "schema_version: \"2.0-skill\"\nentries: []\n",
    )
    .unwrap();
    std::fs::write(
        base.join("manifests/suite.yaml"),
        "schema_version: \"2.0-skill\"\nsuite:\n  name: public\n  version: \"0.3.2\"\n  required: []\n  optional:\n    - name: diagnosing-bugs\n",
    )
    .unwrap();

    let result = check_skills(&base);
    let _ = std::fs::remove_dir_all(&base);

    assert!(
        result.passed,
        "optional recommendations are not adopted requirements: {result:#?}"
    );
}

#[test]
fn adopted_optional_manifest_skill_requires_adoption_log_entry() {
    let base = std::env::temp_dir().join(format!("ags-skill-check-adopted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("governance")).unwrap();
    std::fs::create_dir_all(base.join("manifests")).unwrap();
    std::fs::write(
        base.join("governance/skill-adoption-log.yaml"),
        "schema_version: \"2.0-skill\"\nentries: []\n",
    )
    .unwrap();
    std::fs::write(
        base.join("governance/skill-ignore-list.yaml"),
        "schema_version: \"2.0-skill\"\nentries: []\n",
    )
    .unwrap();
    std::fs::write(
        base.join("manifests/suite.yaml"),
        "schema_version: \"2.0-skill\"\nsuite:\n  name: private\n  version: \"0.3.2\"\n  required: []\n  optional:\n    - name: claude-mem\n      adopted: \"2026-01-01\"\n      entry_ref: governance/skill-adoption-log.yaml#claude-mem\n",
    )
    .unwrap();

    let result = check_skills(&base);
    let _ = std::fs::remove_dir_all(&base);
    let adoption_check = result
        .consistency_checks
        .iter()
        .find(|check| check.name == "manifest-to-adoption-log")
        .expect("manifest-to-adoption-log check");

    assert!(!result.passed);
    assert!(!adoption_check.passed);
    assert!(adoption_check.detail.contains("claude-mem"));
}

#[test]
fn test_propose_unknown_skill() {
    let root = repo_root();
    let result = propose_skills(&root, "adopt", "nonexistent-skill");
    assert_eq!(result.proposal_type, "adopt");
    assert!(result.dry_run);
    assert!(!result.proposed_changes.is_empty());
}

#[test]
fn test_propose_unknown_action_is_blocked() {
    let root = repo_root();
    let result = propose_skills(&root, "unknown-action", "test-skill");
    assert!(result.target_skills.is_empty());
    assert!(!result.blocked_reasons.is_empty());
}

#[test]
fn test_inventory_on_fixture() {
    // Temporary skill tree so the test is independent of the repo's actual
    // skill directories (the public edition ships none).
    let base = std::env::temp_dir().join(format!("ags-skill-inv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let gs = base.join("global-skills/demo-skill");
    std::fs::create_dir_all(&gs).unwrap();
    std::fs::write(
        gs.join("SKILL.md"),
        "---\nname: demo-skill\ndescription: A demo skill.\n---\nbody\n",
    )
    .unwrap();
    let personal = base.join("skill-packs/personal/secret-skill");
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::write(
        personal.join("SKILL.md"),
        "---\nname: secret-skill\ndescription: manages an API token secret.\n---\n",
    )
    .unwrap();

    let result = scan_skill_inventory(&base);
    let _ = std::fs::remove_dir_all(&base);

    assert_eq!(result.summary.total, 2);
    assert_eq!(result.summary.global, 1);
    assert_eq!(result.summary.personal, 1);

    let demo = result
        .entries
        .iter()
        .find(|e| e.name == "demo-skill")
        .expect("demo-skill discovered via front-matter name");
    assert!(demo.has_skill_md && demo.description_present);
    assert!(demo.public_allowed_guess); // global, no risk hints

    let secret = result
        .entries
        .iter()
        .find(|e| e.name == "secret-skill")
        .expect("secret-skill discovered");
    assert!(!secret.public_allowed_guess); // personal + risk hints
    assert!(!secret.risk_hints.is_empty());
}

#[test]
fn test_inventory_empty_tree_renders() {
    // No skill dirs (mirrors the public edition) → total 0, still renders.
    let base = std::env::temp_dir().join(format!("ags-skill-inv-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let result = scan_skill_inventory(&base);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(result.summary.total, 0);
    assert!(render_inventory_text(&result).contains("Skill Asset Inventory"));
    assert!(render_inventory_markdown(&result).contains("# Skill Asset Inventory"));
    assert!(render_inventory_json(&result).contains("\"total\": 0"));
}

#[test]
fn test_render_scan_text() {
    let root = repo_root();
    let result = scan_skills(&root);
    let text = render_scan_text(&result);
    assert!(text.contains("Skill Governance"));
    assert!(text.contains("Suite:"));
}

#[test]
fn test_render_scan_json() {
    let root = repo_root();
    let result = scan_skills(&root);
    let json = render_scan_json(&result);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
    assert!(parsed.is_ok());
}

#[test]
fn test_render_check_text() {
    let root = repo_root();
    let result = check_skills(&root);
    let text = render_check_text(&result);
    assert!(text.contains("Check Report"));
}

#[test]
fn test_render_check_json() {
    let root = repo_root();
    let result = check_skills(&root);
    let json = render_check_json(&result);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
    assert!(parsed.is_ok());
}

#[test]
fn test_render_proposal_text() {
    let root = repo_root();
    let result = propose_skills(&root, "adopt", "test-skill");
    let text = render_proposal_text(&result);
    assert!(text.contains("Proposal"));
    assert!(text.contains("DRY-RUN"));
}

#[test]
fn test_render_proposal_json() {
    let root = repo_root();
    let result = propose_skills(&root, "adopt", "test-skill");
    let json = render_proposal_json(&result);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
    assert!(parsed.is_ok());
    assert_eq!(parsed.unwrap()["dry_run"], true);
}

#[test]
fn test_extract_schema_version() {
    assert_eq!(
        extract_schema_version("schema_version: \"1.0\"\nentries: []"),
        Some("1.0".to_string())
    );
    assert_eq!(
        extract_schema_version("# comment\nschema_version: \"2.0\"\n"),
        Some("2.0".to_string())
    );
    assert_eq!(extract_schema_version("entries: []"), None);
}

#[test]
fn test_upstream_proposal_on_repo_registry() {
    let root = repo_root();
    let result = upstream_proposal(&root);
    assert!(result.registry_present, "repo ships skills-registry.yaml");
    assert!(result.registry_parseable);
    assert_eq!(
        result.update_policy.as_deref(),
        Some("read_only_crawl_then_diff_proposal")
    );
    // Declared upstream comparison sources.
    assert!(result
        .upstreams
        .iter()
        .any(|u| u.name == "mattpocock_skills" && u.crawl));
    assert!(result.upstreams.iter().any(|u| u.name == "graphify"));
    // Skills that track an upstream are surfaced; purely-local ones are not.
    assert!(result
        .watched_skills
        .iter()
        .any(|s| s.name == "diagnosing-bugs"));
    // Candidates are declared but not adopted.
    assert!(result
        .candidates
        .iter()
        .any(|c| c.name == "git-guardrails-claude-code"));
    // This is a stub — no crawl is ever performed.
    assert!(!result.summary.crawl_performed);
    assert_eq!(result.summary.watched_skills, result.watched_skills.len());
}

#[test]
fn test_upstream_proposal_missing_registry() {
    let base = std::env::temp_dir().join(format!("ags-upstream-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let result = upstream_proposal(&base);
    let _ = std::fs::remove_dir_all(&base);
    assert!(!result.registry_present);
    assert!(!result.registry_parseable);
    assert!(result.upstreams.is_empty());
    assert!(result.watched_skills.is_empty());
    assert!(!result.summary.crawl_performed);
}

#[test]
fn test_render_upstream_text_and_json() {
    let root = repo_root();
    let result = upstream_proposal(&root);
    let text = render_upstream_text(&result);
    assert!(text.contains("Upstream Update Proposal"));
    assert!(text.contains("STUB"));
    let json = render_upstream_json(&result);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["summary"]["crawl_performed"], false);
}

#[test]
fn superpowers_parent_does_not_reintroduce_duplicate_routes_or_heavy_chaining() {
    let root = repo_root();
    if !root.join("global-skills").is_dir() {
        assert!(
            !root.join("global-skills/superpowers/SKILL.md").exists(),
            "public-safe edition must not ship the private Superpowers body"
        );
        return;
    }
    let parent = std::fs::read_to_string(root.join("global-skills/superpowers/SKILL.md"))
        .expect("read Superpowers parent");
    assert!(parent.contains("Use only when AGS Skill Resolver"));
    assert!(!parent.contains("| Bug, test failure, or unexpected behavior |"));
    assert!(
        parent.contains("Difficult diagnosis\n   routes through the independent `diagnosing-bugs`")
    );
    assert!(parent.contains("routes through host `skill-creator` when available"));

    let brainstorming = std::fs::read_to_string(
        root.join("global-skills/superpowers/playbooks/brainstorming/PLAYBOOK.md"),
    )
    .expect("read brainstorming resource");
    assert!(brainstorming.contains("Do not auto-chain to `writing-plans`"));
    assert!(brainstorming.contains("does not authorize repository writes"));
    for forbidden in [
        "This applies to EVERY project",
        "You MUST create a task for each",
        "commit the design document",
        "The terminal state is invoking writing-plans",
    ] {
        assert!(
            !brainstorming.contains(forbidden),
            "brainstorming reintroduced heavy ceremony: {forbidden}"
        );
    }
}
