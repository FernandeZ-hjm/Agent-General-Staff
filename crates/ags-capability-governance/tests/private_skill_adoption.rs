use ags_capability_governance::skill_adoption::{
    apply_adoption, apply_removal, body_path, inspect_adoption, load_registry, plan_adoption,
    plan_removal, AdoptionContext, SnapshotDiscovery,
};
use ags_capability_governance::{load_static_snapshot, resolve_skill};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn authority_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn context(temp: &TempDir) -> AdoptionContext {
    AdoptionContext {
        authority_root: authority_root(),
        runtime_home: temp.path().join("runtime"),
        host_home: temp.path().join("home"),
        snapshot_discovery: SnapshotDiscovery::Offline,
    }
}

fn source_fixture(temp: &TempDir, name: &str) -> PathBuf {
    let repository = temp.path().join(format!("{name}-source"));
    let skill = repository.join("skill");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
    fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: Upstream description.\nextra_upstream_field: preserved-by-source-but-ignored-by-adoption\n---\n\n# {name}\n"
        ),
    )
    .unwrap();
    fs::write(skill.join("REFERENCE.md"), "bounded worker contract\n").unwrap();
    skill
}

#[cfg(unix)]
#[test]
fn private_adoption_is_plan_bound_recoverable_and_routable() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "third-party-team");
    let metadata = temp.path().join("routing.yaml");
    fs::write(
        &metadata,
        "summary: Delegate bounded software work when parallel exploration or independent testing is useful.\nintent_tags: [delegation, parallel-software-work]\npositive_examples: [Delegate this investigation in parallel]\nnegative_examples: [Answer this one-line question directly]\n",
    )
    .unwrap();
    let hosts = vec!["codex".to_string()];

    let plan = plan_adoption(&context, &source, Some(&metadata), &hosts).unwrap();
    assert!(!context.runtime_home.exists(), "planning must not write");
    let changed =
        apply_adoption(&context, &source, Some(&metadata), &hosts, "sha256:stale").unwrap_err();
    assert!(changed.contains("adoption_plan_changed"));
    assert!(
        !context.runtime_home.exists(),
        "rejected apply must not write"
    );

    let receipt =
        apply_adoption(&context, &source, Some(&metadata), &hosts, &plan.plan_hash).unwrap();
    assert!(receipt.requires_repreflight);
    let registry = load_registry(&context.runtime_home).unwrap();
    let record = registry.skills.get("third-party-team").unwrap();
    let immutable_body = body_path(&context.runtime_home, record);
    assert!(immutable_body.join("SKILL.md").is_file());
    let host_index = context.host_home.join(".codex/skills/third-party-team");
    assert_eq!(
        fs::canonicalize(&host_index).unwrap(),
        fs::canonicalize(&immutable_body).unwrap()
    );

    let status = inspect_adoption(
        &context.runtime_home,
        &context.host_home,
        "third-party-team",
    )
    .unwrap();
    assert!(status.registered && status.body_present && status.body_hash_matches);
    assert_eq!(status.visible_hosts, ["codex"]);
    assert_eq!(status.active_hosts, ["codex"]);

    let (snapshot, _) = load_static_snapshot(&context.runtime_home, "codex").unwrap();
    let card = snapshot
        .catalog
        .iter()
        .find(|card| card.skill_id == "third-party-team")
        .unwrap();
    assert!(card.intent_tags.iter().any(|tag| tag == "delegation"));
    assert!(!card.positive_examples.is_empty());
    assert!(!card.negative_examples.is_empty());
    let tables = snapshot.validate_integrity("codex").unwrap();
    let selection = resolve_skill(
        "third-party-team",
        None,
        &snapshot.snapshot_hash,
        &tables.skills,
    )
    .unwrap();
    assert_eq!(selection.skill_id, "third-party-team");

    let removal = plan_removal(&context, "third-party-team").unwrap();
    apply_removal(&context, "third-party-team", &removal.plan_hash).unwrap();
    assert!(!load_registry(&context.runtime_home)
        .unwrap()
        .skills
        .contains_key("third-party-team"));
    assert!(fs::symlink_metadata(&host_index).is_err());
    assert!(immutable_body.is_dir(), "removal retains recoverable body");
    let (snapshot, _) = load_static_snapshot(&context.runtime_home, "codex").unwrap();
    assert!(!snapshot
        .active_skills
        .iter()
        .any(|skill| skill.skill_id == "third-party-team"));
}

#[test]
fn private_adoption_cannot_shadow_the_official_registry() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "superpowers");
    let error = plan_adoption(&context, &source, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("cannot shadow official skill id"));
}

#[cfg(unix)]
#[test]
fn private_adoption_refuses_symlinked_source_content() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "symlinked-team");
    std::os::unix::fs::symlink(source.join("REFERENCE.md"), source.join("ALIAS.md")).unwrap();
    let error = plan_adoption(&context, &source, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("symlink_refused"));
}
