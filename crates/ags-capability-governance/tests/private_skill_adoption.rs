use ags_capability_governance::skill_adoption::{
    acquire_remote_candidate, acquire_remote_candidate_with_backend, apply_install,
    apply_reactivation_in_maintenance_transaction, apply_removal, apply_rollback, apply_update,
    bodies_root, body_path, inspect_adoption, installed_skill_index_path, load_installed_skills,
    parse_github_url, plan_install, plan_install_with_backend, plan_legacy_catalog_migration,
    plan_removal, plan_rollback, plan_update_with_backend, recover_applied_change,
    recover_pending_transactions, transaction_journal_path, verify_adoption_routes,
    AdoptionContext, CatalogReviewStatus, GitBackend, JournalFileState, JournalLinkState,
    PreparedSkillChange, RemoteTreeEntry, RemoteTreeEntryKind, RiskAcknowledgements,
    SnapshotDiscovery, SourceSpec, TransactionJournal, TransactionPhase, UpdatePolicy,
    TRANSACTION_JOURNAL_SCHEMA,
};
use ags_capability_governance::{
    hash_skill_source, load_static_snapshot, resolve_skill, snapshot_path,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

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

fn source_fixture_without_license(temp: &TempDir, name: &str) -> PathBuf {
    let repository = temp.path().join(format!("{name}-source"));
    let skill = repository.join("skill");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Unlicensed fixture.\n---\n\n# {name}\n"),
    )
    .unwrap();
    skill
}

#[derive(Clone)]
struct FixtureGitBackend {
    source_root: PathBuf,
    commit: String,
    reject_submodule: bool,
    tree_metadata: Option<Vec<RemoteTreeEntry>>,
    expected_requested_ref: Option<String>,
}

impl FixtureGitBackend {
    fn new(source_root: PathBuf, commit: &str) -> Self {
        Self {
            source_root,
            commit: commit.to_string(),
            reject_submodule: false,
            tree_metadata: None,
            expected_requested_ref: None,
        }
    }

    fn with_tree_metadata(mut self, entries: Vec<RemoteTreeEntry>) -> Self {
        self.tree_metadata = Some(entries);
        self
    }

    fn expect_requested_ref(mut self, requested_ref: &str) -> Self {
        self.expected_requested_ref = Some(requested_ref.to_string());
        self
    }
}

impl GitBackend for FixtureGitBackend {
    fn resolve_commit(
        &self,
        _repository_url: &str,
        requested_ref: Option<&str>,
    ) -> Result<String, String> {
        if let Some(expected) = self.expected_requested_ref.as_deref() {
            assert_eq!(requested_ref, Some(expected));
        }
        Ok(self.commit.clone())
    }

    fn prepare_checkout(
        &self,
        _repository_url: &str,
        _resolved_commit: &str,
        _destination: &Path,
    ) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
        Ok(self.tree_metadata.clone())
    }

    fn materialize_selected(
        &self,
        _repository_url: &str,
        _resolved_commit: &str,
        destination: &Path,
        _subdir: &str,
        _license_paths: &[String],
    ) -> Result<(), String> {
        copy_fixture_tree(&self.source_root, destination)
    }

    fn validate_checkout(&self, _destination: &Path) -> Result<(), String> {
        if self.reject_submodule {
            Err("submodule_refused: injected gitlink".to_string())
        } else {
            Ok(())
        }
    }
}

fn copy_fixture_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(
                fs::read_link(&source_path).map_err(|error| error.to_string())?,
                &destination_path,
            )
            .map_err(|error| error.to_string())?;
            #[cfg(windows)]
            return Err("fixture symlink test requires unix".to_string());
        } else if metadata.is_dir() {
            copy_fixture_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| error.to_string())?;
        } else {
            return Err(format!("fixture special file: {}", source_path.display()));
        }
    }
    Ok(())
}

fn remote_source(repository: &Path, subdir: &str) -> SourceSpec {
    SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: None,
        tracking_ref: None,
        subdir: Some(subdir.to_string()),
    }
}

fn authority_fixture(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("authority");
    fs::create_dir_all(root.join("manifests")).unwrap();
    fs::write(root.join("manifests/skills-registry.yaml"), "skills: []\n").unwrap();
    root
}

fn catalog_authority_fixture(
    temp: &TempDir,
    skill_id: &str,
    commit: &str,
    integrity: &str,
) -> PathBuf {
    let root = authority_fixture(temp);
    fs::write(
        root.join("manifests/third-party-capabilities.yaml"),
        format!(
            r#"schema_version: "1.0"
principle: fixture
capabilities:
  - id: {skill_id}
    kind: skill
    name: Fixture
    profiles: [public, private]
    required: false
    tier: fixture
    purpose: Fixture catalog review.
    risk: low
    source:
      manager: git
      revision: "{commit}"
      tracking_ref: main
      integrity: "{integrity}"
      repository: "https://github.com/acme/skills"
      license: MIT
      subdir: skill
    install:
      strategy: external-manager
    routing:
      route_state: routable
      invoke_hint: "[skill: {skill_id}]"
      intent_tags: [fixture]
      mutation_surface: read-only
      cost_class: free
      positive_examples: ["use fixture"]
      negative_examples: ["do not use fixture"]
"#
        ),
    )
    .unwrap();
    root
}

fn context_with_authority(temp: &TempDir, authority_root: PathBuf) -> AdoptionContext {
    AdoptionContext {
        authority_root,
        runtime_home: temp.path().join("runtime"),
        host_home: temp.path().join("home"),
        snapshot_discovery: SnapshotDiscovery::Offline,
    }
}

fn acknowledge_all(plan: &PreparedSkillChange) -> RiskAcknowledgements {
    plan.risk_findings
        .iter()
        .map(|finding| finding.acknowledgement_id())
        .collect()
}

fn plan_local(
    context: &AdoptionContext,
    source: &Path,
    metadata: Option<&Path>,
    hosts: &[String],
) -> Result<PreparedSkillChange, String> {
    plan_install(
        context,
        &SourceSpec::local(source.to_string_lossy()),
        metadata,
        hosts,
        UpdatePolicy::Notify,
    )
}

fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap_or_else(|error| panic!("cannot start git {:?}: {error}", args));
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn actual_git_source_fixture(temp: &TempDir, name: &str) -> (PathBuf, PathBuf, String) {
    let repository = temp.path().join(format!("{name}-git-source"));
    let skill = repository.join("skill");
    fs::create_dir_all(&skill).unwrap();
    fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
    fs::write(
        skill.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Commit-pinned fixture.\n---\n\n# {name}\n"),
    )
    .unwrap();
    fs::write(skill.join("REFERENCE.md"), "commit-pinned body\n").unwrap();
    run_git(&repository, &["init", "--quiet"]);
    run_git(&repository, &["config", "user.name", "fixture"]);
    run_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    run_git(&repository, &["add", "--all"]);
    run_git(&repository, &["commit", "--quiet", "-m", "fixture"]);
    let commit = run_git(&repository, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    (repository, skill, commit)
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

    let plan = plan_local(&context, &source, Some(&metadata), &hosts).unwrap();
    assert!(!context.runtime_home.join("skill-plans").exists());

    let receipt = apply_install(&context, &plan, "test-plan", &acknowledge_all(&plan)).unwrap();
    assert_eq!(receipt.transaction_id, "test-plan");
    assert!(receipt.requires_repreflight);
    let registry = load_installed_skills(&context.runtime_home).unwrap();
    let record = registry.skills.get("third-party-team").unwrap();
    let immutable_body = body_path(&context.runtime_home, record);
    assert!(immutable_body.join("SKILL.md").is_file());
    let host_index = context.host_home.join(".agents/skills/third-party-team");
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
    let route_status = verify_adoption_routes(
        &context.runtime_home,
        &context.host_home,
        "third-party-team",
    )
    .unwrap();
    assert!(route_status.verified_on_all_targets());
    assert_eq!(route_status.activations.len(), 1);
    assert!(route_status.activations[0].route_verified);

    let removal = plan_removal(&context, "third-party-team").unwrap();
    apply_removal(&context, &removal, "remove-plan").unwrap();
    assert!(!load_installed_skills(&context.runtime_home)
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

    recover_applied_change(&context, &removal, "recover-removal").unwrap();
    assert!(load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .contains_key("third-party-team"));
    recover_applied_change(&context, &plan, "recover-install").unwrap();
    assert!(!load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .contains_key("third-party-team"));
}

#[cfg(unix)]
#[test]
fn shared_loading_hosts_use_one_index_and_retire_legacy_native_duplicates() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "shared-index-team");
    let hosts = vec!["codex".to_string(), "cursor".to_string()];

    let first = plan_local(&context, &source, None, &hosts).unwrap();
    assert_eq!(
        first.host_indexes,
        vec![context
            .host_home
            .join(".agents/skills/shared-index-team")
            .to_string_lossy()
            .into_owned()]
    );
    apply_install(&context, &first, "first-plan", &acknowledge_all(&first)).unwrap();

    let index = load_installed_skills(&context.runtime_home).unwrap();
    let record = index.skills.get("shared-index-team").unwrap().clone();
    let body = body_path(&context.runtime_home, &record);
    let shared = context.host_home.join(".agents/skills/shared-index-team");
    fs::remove_file(&shared).unwrap();
    for native in [
        context.host_home.join(".codex/skills/shared-index-team"),
        context.host_home.join(".cursor/skills/shared-index-team"),
    ] {
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&body, native).unwrap();
    }

    let migration = plan_local(&context, &source, None, &hosts).unwrap();
    assert_eq!(migration.host_indexes, first.host_indexes);
    assert_eq!(migration.retired_host_indexes.len(), 2);
    apply_install(
        &context,
        &migration,
        "migration-plan",
        &acknowledge_all(&migration),
    )
    .unwrap();

    assert_eq!(
        fs::canonicalize(&shared).unwrap(),
        fs::canonicalize(body).unwrap()
    );
    assert!(!context
        .host_home
        .join(".codex/skills/shared-index-team")
        .exists());
    assert!(!context
        .host_home
        .join(".cursor/skills/shared-index-team")
        .exists());
    let status = inspect_adoption(
        &context.runtime_home,
        &context.host_home,
        "shared-index-team",
    )
    .unwrap();
    assert_eq!(status.active_hosts, ["codex", "cursor"]);
}

#[test]
fn private_adoption_cannot_shadow_the_official_registry() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "superpowers");
    let error = plan_local(&context, &source, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("cannot shadow suite Skill id"));
}

#[cfg(unix)]
#[test]
fn private_adoption_refuses_symlinked_source_content() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "symlinked-team");
    std::os::unix::fs::symlink(source.join("REFERENCE.md"), source.join("ALIAS.md")).unwrap();
    let error = plan_local(&context, &source, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("symlink_refused"));
}

#[test]
fn normal_reader_rejects_legacy_registry_instead_of_silently_migrating() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let path = installed_skill_index_path(&context.runtime_home);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let legacy = serde_json::json!({
        "schema_version": "0.4.0-private-skill-registry",
        "revision": 7,
        "skills": {
            "legacy-team": {
                "skill_id": "legacy-team",
                "source": "/machine/local/legacy-team",
                "source_hash": "sha256:legacy",
                "license_path": "/machine/local/LICENSE",
                "license_hash": "sha256:license",
                "routing_metadata_path": null,
                "routing_metadata_hash": null,
                "body_revision": "legacy",
                "summary": "legacy",
                "intent_tags": [],
                "positive_examples": [],
                "negative_examples": [],
                "entrypoints": [],
                "invoke_hint": "[skill: legacy-team]",
                "requires_auth": false,
                "version": "",
                "target_hosts": ["codex"]
            }
        }
    });
    fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

    let error = load_installed_skills(&context.runtime_home).unwrap_err();
    assert!(error.contains("unsupported installed Skill index schema"));
}

#[test]
fn system_git_accepts_an_exact_commit_requested_ref_hermetically() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let (repository, _skill, commit) = actual_git_source_fixture(&temp, "commit-pinned-team");
    let source = SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: Some(commit.clone()),
        tracking_ref: Some(commit.clone()),
        subdir: Some("skill".to_string()),
    };

    let candidate = acquire_remote_candidate(&context, &source).unwrap();
    assert_eq!(candidate.resolved_source.resolved_commit, commit);
    assert!(candidate.skill_dir.join("SKILL.md").is_file());
}

#[test]
fn github_source_parser_is_canonical_and_rejects_ambiguous_or_unsafe_urls() {
    assert_eq!(
        parse_github_url("https://github.com/acme/skills.git/", None).unwrap(),
        SourceSpec::GitHub {
            url: "https://github.com/acme/skills".to_string(),
            requested_ref: None,
            tracking_ref: None,
            subdir: None,
        }
    );
    assert_eq!(
        parse_github_url("https://github.com/acme/skills/tree/main/worker", None).unwrap(),
        SourceSpec::GitHub {
            url: "https://github.com/acme/skills".to_string(),
            requested_ref: Some("main".to_string()),
            tracking_ref: Some("main".to_string()),
            subdir: Some("worker".to_string()),
        }
    );
    assert_eq!(
        parse_github_url(
            "https://github.com/acme/skills/blob/main/worker/SKILL.md",
            None
        )
        .unwrap(),
        SourceSpec::GitHub {
            url: "https://github.com/acme/skills".to_string(),
            requested_ref: Some("main".to_string()),
            tracking_ref: Some("main".to_string()),
            subdir: Some("worker".to_string()),
        }
    );
    assert_eq!(
        parse_github_url(
            "https://github.com/acme/skills/tree/feature/x/worker",
            Some("feature/x"),
        )
        .unwrap(),
        SourceSpec::GitHub {
            url: "https://github.com/acme/skills".to_string(),
            requested_ref: Some("feature/x".to_string()),
            tracking_ref: Some("feature/x".to_string()),
            subdir: Some("worker".to_string()),
        }
    );
    for (url, requested_ref) in [
        ("https://evil.example/acme/skills", None),
        ("https://user:pass@github.com/acme/skills", None),
        ("https://github.com/acme/skills?ref=main", None),
        ("https://github.com/acme/skills/tree/feature/x/worker", None),
        ("https://github.com/acme/skills/tree/main/../escape", None),
        ("https://github.com//skills", None),
    ] {
        assert!(
            parse_github_url(url, requested_ref).is_err(),
            "accepted unsafe URL: {url}"
        );
    }
}

#[test]
fn canonical_github_source_keeps_its_separate_subdirectory_binding() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let skill = source_fixture(&temp, "catalog-team");
    let repository = skill.parent().unwrap().to_path_buf();
    let commit = "7".repeat(40);
    let source = SourceSpec::GitHub {
        url: "https://github.com/acme/skills".to_string(),
        requested_ref: Some(commit.clone()),
        tracking_ref: Some("main".to_string()),
        subdir: Some("skill".to_string()),
    };

    let candidate = acquire_remote_candidate_with_backend(
        &context,
        &source,
        &FixtureGitBackend::new(repository, &commit),
    )
    .unwrap();

    assert_eq!(candidate.resolved_source.source_spec, source);
    assert_eq!(candidate.resolved_source.resolved_commit, commit);
    assert_eq!(candidate.resolved_source.subdir, "skill");
    assert!(candidate.skill_dir.join("SKILL.md").is_file());
}

#[test]
fn catalog_review_is_bound_to_exact_source_commit_subdir_and_body_hash() {
    let temp = TempDir::new().unwrap();
    let skill = source_fixture(&temp, "reviewed-team");
    let repository = skill.parent().unwrap().to_path_buf();
    let body_hash = hash_skill_source(&skill).unwrap();
    let commit = "8".repeat(40);
    let authority = catalog_authority_fixture(&temp, "reviewed-team", &commit, &body_hash);
    let context = context_with_authority(&temp, authority.clone());
    let source = SourceSpec::GitHub {
        url: "https://github.com/acme/skills".to_string(),
        requested_ref: Some(commit.clone()),
        tracking_ref: Some("main".to_string()),
        subdir: Some("skill".to_string()),
    };

    let plan = plan_install_with_backend(
        &context,
        &source,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &FixtureGitBackend::new(repository.clone(), &commit),
    )
    .unwrap();
    assert_eq!(plan.catalog_review, CatalogReviewStatus::Reviewed);
    assert_eq!(plan.source_spec.tracking_ref(), Some("main"));
    assert!(!plan
        .risk_findings
        .iter()
        .any(|finding| finding.code == "catalog_unreviewed"));

    apply_install(&context, &plan, "catalog-install", &acknowledge_all(&plan)).unwrap();
    fs::write(skill.join("REFERENCE.md"), "new upstream revision\n").unwrap();
    let new_commit = "9".repeat(40);
    let update = plan_update_with_backend(
        &context,
        "reviewed-team",
        &FixtureGitBackend::new(repository.clone(), &new_commit).expect_requested_ref("main"),
    )
    .unwrap();
    assert_eq!(
        update
            .resolved_source
            .as_ref()
            .and_then(|source| source.source_spec.requested_ref()),
        Some("main")
    );
    assert_eq!(update.catalog_review, CatalogReviewStatus::Unreviewed);
    assert!(update
        .risk_findings
        .iter()
        .any(|finding| finding.code == "catalog_unreviewed"));

    fs::write(
        authority.join("manifests/third-party-capabilities.yaml"),
        fs::read_to_string(authority.join("manifests/third-party-capabilities.yaml"))
            .unwrap()
            .replace(&body_hash, &format!("sha256:{}", "0".repeat(64))),
    )
    .unwrap();
    let error = plan_install_with_backend(
        &context,
        &source,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &FixtureGitBackend::new(repository, &commit),
    )
    .unwrap_err();
    assert!(error.contains("catalog_integrity_mismatch"));
}

#[test]
fn legacy_catalog_migration_preserves_a_diverged_installed_body_and_repairs_activation() {
    let temp = TempDir::new().unwrap();
    let reviewed = source_fixture(&temp, "reviewed-team");
    let reviewed_hash = hash_skill_source(&reviewed).unwrap();
    let authority =
        catalog_authority_fixture(&temp, "reviewed-team", &"8".repeat(40), &reviewed_hash);
    let context = context_with_authority(&temp, authority);
    let diverged_repository = temp.path().join("diverged-source");
    copy_fixture_tree(reviewed.parent().unwrap(), &diverged_repository).unwrap();
    let diverged = diverged_repository.join("skill");
    fs::write(
        diverged.join("REFERENCE.md"),
        "user-maintained content that must not be overwritten\n",
    )
    .unwrap();
    let install = plan_local(&context, &diverged, None, &["codex".to_string()]).unwrap();
    apply_install(
        &context,
        &install,
        "install-diverged",
        &acknowledge_all(&install),
    )
    .unwrap();
    let installed_hash = install.source_hash.clone();

    let reactivation = plan_legacy_catalog_migration(
        &context,
        &reviewed,
        "reviewed-team",
        &["codex".to_string(), "claude-code".to_string()],
    )
    .unwrap();
    assert_eq!(reactivation.operation, "reactivate");
    assert_eq!(reactivation.source_hash, installed_hash);
    assert!(matches!(reactivation.source_spec, SourceSpec::Local { .. }));

    let _lock = ags_platform::MaintenanceLock::acquire(&context.runtime_home).unwrap();
    apply_reactivation_in_maintenance_transaction(&context, &reactivation, "reactivate-diverged")
        .unwrap();
    let record = load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .remove("reviewed-team")
        .unwrap();
    assert_eq!(record.source_hash, installed_hash);
    assert_eq!(
        record.target_hosts,
        vec!["claude-code".to_string(), "codex".to_string()]
    );
    assert_eq!(
        hash_skill_source(&body_path(&context.runtime_home, &record)).unwrap(),
        installed_hash
    );
}

#[test]
fn non_recommended_remote_can_plan_and_policy_is_plan_bound() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "arbitrary-community-skill");
    let repository = source.parent().unwrap().to_path_buf();
    let remote = remote_source(&repository, "skill");
    let backend = FixtureGitBackend::new(repository.clone(), &"1".repeat(40));
    let plan = plan_install_with_backend(
        &context,
        &remote,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &backend,
    )
    .unwrap();
    assert_eq!(plan.skill_id, "arbitrary-community-skill");
    assert_eq!(plan.update_policy, UpdatePolicy::Notify);
    assert!(plan.resolved_source.is_some());
    assert!(!context.runtime_home.join("skill-plans").exists());
    assert!(context.runtime_home.join("candidates").is_dir());

    let manual_source = source_fixture(&temp, "manual-community-skill");
    let manual = plan_install(
        &context,
        &SourceSpec::local(manual_source.to_string_lossy()),
        None,
        &["codex".to_string()],
        UpdatePolicy::Manual,
    )
    .unwrap();
    assert_eq!(manual.update_policy, UpdatePolicy::Manual);

    let pinned_source = source_fixture(&temp, "pinned-community-skill");
    let pinned = plan_install(
        &context,
        &SourceSpec::local(pinned_source.to_string_lossy()),
        None,
        &["codex".to_string()],
        UpdatePolicy::Pinned,
    )
    .unwrap();
    assert_eq!(pinned.update_policy, UpdatePolicy::Pinned);
    let pinned_acknowledgements = acknowledge_all(&pinned);
    apply_install(&context, &pinned, "test-plan", &pinned_acknowledgements).unwrap();
    let error = plan_update_with_backend(&context, "pinned-community-skill", &backend).unwrap_err();
    assert!(error.contains("pinned_update_has_no_candidate"));
}

#[test]
fn candidate_hash_drift_fails_closed_before_install() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "drifted-community-skill");
    let repository = source.parent().unwrap().to_path_buf();
    let plan = plan_install_with_backend(
        &context,
        &remote_source(&repository, "skill"),
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &FixtureGitBackend::new(repository, &"2".repeat(40)),
    )
    .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(Path::new(plan.candidate_path.as_deref().unwrap()).join("SKILL.md"))
        .unwrap()
        .write_all(b"\nchanged after plan\n")
        .unwrap();
    let plan_acknowledgements = acknowledge_all(&plan);
    let error = apply_install(&context, &plan, "test-plan", &plan_acknowledgements).unwrap_err();
    assert!(error.contains("candidate_hash_or_source_drift"));
    assert!(!installed_skill_index_path(&context.runtime_home).exists());
}

#[test]
fn unknown_license_and_scripts_are_acknowledgement_risks_and_never_execute() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let licenseless = source_fixture_without_license(&temp, "unlicensed-community-skill");
    let plan = plan_install(
        &context,
        &SourceSpec::local(licenseless.to_string_lossy()),
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
    )
    .unwrap();
    assert!(plan
        .risk_findings
        .iter()
        .any(|finding| finding.code == "missing_license"));
    let missing_license_id = plan
        .risk_findings
        .iter()
        .find(|finding| finding.code == "missing_license")
        .unwrap()
        .acknowledgement_id();
    let mut partial_acknowledgements = RiskAcknowledgements::new();
    partial_acknowledgements.insert(missing_license_id);
    let error = apply_install(&context, &plan, "test-plan", &partial_acknowledgements).unwrap_err();
    assert!(error.contains("catalog_unreviewed"));
    let mut unknown_acknowledgements = RiskAcknowledgements::new();
    unknown_acknowledgements.insert("not-in-plan".to_string());
    let error = apply_install(&context, &plan, "test-plan", &unknown_acknowledgements).unwrap_err();
    assert!(error.contains("acknowledgement_unknown"));
    let no_acknowledgements = RiskAcknowledgements::new();
    assert!(
        apply_install(&context, &plan, "test-plan", &no_acknowledgements)
            .unwrap_err()
            .contains("acknowledgement_required")
    );
    let acknowledgements = acknowledge_all(&plan);
    apply_install(&context, &plan, "test-plan", &acknowledgements).unwrap();
    assert_eq!(
        load_installed_skills(&context.runtime_home)
            .unwrap()
            .skills
            .get("unlicensed-community-skill")
            .unwrap()
            .catalog_review,
        CatalogReviewStatus::Unreviewed
    );
    let route_status = verify_adoption_routes(
        &context.runtime_home,
        &context.host_home,
        "unlicensed-community-skill",
    )
    .unwrap();
    assert!(
        route_status.verified_on_all_targets(),
        "an explicitly acknowledged missing license must not become a permanent routing block: {route_status:#?}"
    );

    let script_source = source_fixture(&temp, "script-community-skill");
    let marker = temp.path().join("script-ran");
    fs::write(
        script_source.join("install.sh"),
        format!("#!/bin/sh\ntouch {}\n", marker.display()),
    )
    .unwrap();
    let script_plan = plan_install(
        &context,
        &SourceSpec::local(script_source.to_string_lossy()),
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
    )
    .unwrap();
    assert!(script_plan
        .risk_findings
        .iter()
        .any(|finding| finding.code == "script_or_executable_content"));
    let script_acknowledgements = acknowledge_all(&script_plan);
    apply_install(
        &context,
        &script_plan,
        "test-plan",
        &script_acknowledgements,
    )
    .unwrap();
    assert!(!marker.exists(), "install.sh must never be executed");
}

#[cfg(unix)]
#[test]
fn remote_symlink_special_and_traversal_boundaries_are_hard() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let symlinked = source_fixture(&temp, "remote-symlink-team");
    std::os::unix::fs::symlink(symlinked.join("SKILL.md"), symlinked.join("ALIAS.md")).unwrap();
    let repo = symlinked.parent().unwrap().to_path_buf();
    let error = acquire_remote_candidate_with_backend(
        &context,
        &remote_source(&repo, "skill"),
        &FixtureGitBackend::new(repo.clone(), &"3".repeat(40)),
    )
    .unwrap_err();
    assert!(error.contains("symlink_refused"));

    let traversal = SourceSpec::Git {
        url: "file:///fixture".to_string(),
        requested_ref: None,
        tracking_ref: None,
        subdir: Some("../outside".to_string()),
    };
    let error = acquire_remote_candidate_with_backend(
        &context,
        &traversal,
        &FixtureGitBackend::new(repo.clone(), &"4".repeat(40)),
    )
    .unwrap_err();
    assert!(error.contains("invalid") || error.contains("traversing"));

    let special = source_fixture(&temp, "special-file-team");
    let socket_path = special.join("special.sock");
    let _listener = UnixListener::bind(&socket_path).unwrap();
    let error = plan_local(&context, &special, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("special_file_refused"));

    let mut submodule_backend = FixtureGitBackend::new(repo, &"5".repeat(40));
    submodule_backend.reject_submodule = true;
    let error = acquire_remote_candidate_with_backend(
        &context,
        &remote_source(symlinked.parent().unwrap(), "skill"),
        &submodule_backend,
    )
    .unwrap_err();
    assert!(error.contains("submodule_refused"));
}

#[test]
fn update_is_commit_and_body_bound_and_rollback_uses_existing_revision() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source_a = source_fixture(&temp, "updatable-team");
    fs::write(
        source_a.join("SKILL.md"),
        "---\nname: updatable-team\ndescription: First upstream description.\nsummary: First summary.\nintent_tags: [first]\npositive_examples: [first example]\nnegative_examples: [first negative]\nentrypoints: [first-entry]\ninvoke_hint: first invoke\nrequires_auth: false\nversion: 1.0.0\n---\n\n# First body\n",
    )
    .unwrap();
    let repo_a = source_a.parent().unwrap().to_path_buf();
    let source_b = source_fixture(&temp, "updatable-team-v2");
    fs::write(
        source_b.join("SKILL.md"),
        fs::read_to_string(source_b.join("SKILL.md"))
            .unwrap()
            .replace("name: updatable-team-v2", "name: updatable-team")
            .replace("Upstream description.", "Second upstream description.")
            .replace(
                "description: Second upstream description.",
                "description: Second upstream description.\nsummary: Second summary.\nintent_tags: [second]\npositive_examples: [second example]\nnegative_examples: [second negative]\nentrypoints: [second-entry]\ninvoke_hint: second invoke\nrequires_auth: true\nversion: 2.0.0",
            ),
    )
    .unwrap();
    let repo_b = source_b.parent().unwrap().to_path_buf();
    let source = remote_source(&repo_a, "skill");
    let plan = plan_install_with_backend(
        &context,
        &source,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &FixtureGitBackend::new(repo_a, &"a".repeat(40)),
    )
    .unwrap();
    let install_acknowledgements = acknowledge_all(&plan);
    apply_install(&context, &plan, "test-plan", &install_acknowledgements).unwrap();
    let first = load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .get("updatable-team")
        .unwrap()
        .clone();
    let first_revision = first.body_revision.clone();
    assert_eq!(
        first.resolved_source.as_ref().unwrap().resolved_commit,
        "a".repeat(40)
    );
    assert_eq!(first.summary, "First summary.");
    assert_eq!(first.entrypoints, ["first-entry"]);
    assert!(!first.requires_auth);
    assert_eq!(first.version, "1.0.0");

    let update_plan = plan_update_with_backend(
        &context,
        "updatable-team",
        &FixtureGitBackend::new(repo_b, &"b".repeat(40)),
    )
    .unwrap();
    assert_eq!(update_plan.operation, "update");
    assert_ne!(update_plan.body_hash, first.source_hash);
    let update_acknowledgements = acknowledge_all(&update_plan);
    apply_update(
        &context,
        &update_plan,
        "test-plan",
        &update_acknowledgements,
    )
    .unwrap();
    let updated = load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .get("updatable-team")
        .unwrap()
        .clone();
    assert_ne!(updated.body_revision, first_revision);
    assert_eq!(updated.body_revisions.len(), 2);
    assert!(body_path(&context.runtime_home, &first).is_dir());
    assert_eq!(
        updated.resolved_source.as_ref().unwrap().resolved_commit,
        "b".repeat(40)
    );
    assert_eq!(updated.summary, "Second summary.");
    assert_eq!(updated.entrypoints, ["second-entry"]);
    assert!(updated.requires_auth);
    assert_eq!(updated.version, "2.0.0");

    let rollback_plan = plan_rollback(&context, "updatable-team", &first_revision).unwrap();
    assert_eq!(
        rollback_plan.rollback_revision.as_deref(),
        Some(first_revision.as_str())
    );
    apply_rollback(&context, &rollback_plan, "test-plan").unwrap();
    let final_registry = load_installed_skills(&context.runtime_home).unwrap();
    let rolled_back = final_registry.skills.get("updatable-team").unwrap();
    assert_eq!(rolled_back.body_revision, first_revision);
    assert_eq!(
        rolled_back
            .resolved_source
            .as_ref()
            .unwrap()
            .resolved_commit,
        "a".repeat(40)
    );
    assert_eq!(rolled_back.source, first.source);
    assert_eq!(rolled_back.license_path, first.license_path);
    assert_eq!(rolled_back.summary, first.summary);
    assert_eq!(rolled_back.intent_tags, first.intent_tags);
    assert_eq!(rolled_back.positive_examples, first.positive_examples);
    assert_eq!(rolled_back.negative_examples, first.negative_examples);
    assert_eq!(rolled_back.entrypoints, first.entrypoints);
    assert_eq!(rolled_back.invoke_hint, first.invoke_hint);
    assert_eq!(rolled_back.requires_auth, first.requires_auth);
    assert_eq!(rolled_back.version, first.version);
    assert_eq!(rolled_back.catalog_review, first.catalog_review);
    assert_eq!(rolled_back.risk_findings, first.risk_findings);
}

#[test]
fn failed_snapshot_restores_registry_links_and_new_body() {
    let temp = TempDir::new().unwrap();
    let authority = authority_fixture(&temp);
    let context = context_with_authority(&temp, authority.clone());
    let source = source_fixture(&temp, "rollback-on-failure-team");
    let repository = source.parent().unwrap().to_path_buf();
    let plan = plan_install_with_backend(
        &context,
        &remote_source(&repository, "skill"),
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &FixtureGitBackend::new(repository, &"c".repeat(40)),
    )
    .unwrap();
    fs::remove_file(authority.join("manifests/skills-registry.yaml")).unwrap();
    let failure_acknowledgements = acknowledge_all(&plan);
    let error = apply_install(&context, &plan, "test-plan", &failure_acknowledgements).unwrap_err();
    assert!(error.contains("capability snapshot build failed"));
    assert!(!load_installed_skills(&context.runtime_home)
        .unwrap()
        .skills
        .contains_key("rollback-on-failure-team"));
    assert!(!Path::new(&plan.body_path).exists());
    assert!(!context
        .host_home
        .join(".agents/skills/rollback-on-failure-team")
        .exists());
}

#[test]
fn stale_concurrent_plan_is_rejected_by_registry_and_body_cas() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "stale-concurrent-team");
    let repository = source.parent().unwrap().to_path_buf();
    let backend = FixtureGitBackend::new(repository.clone(), &"d".repeat(40));
    let remote = remote_source(&repository, "skill");
    let first = plan_install_with_backend(
        &context,
        &remote,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &backend,
    )
    .unwrap();
    let second = plan_install_with_backend(
        &context,
        &remote,
        None,
        &["codex".to_string()],
        UpdatePolicy::Notify,
        &backend,
    )
    .unwrap();
    apply_install(&context, &first, "test-plan", &acknowledge_all(&first)).unwrap();
    let error =
        apply_install(&context, &second, "test-plan", &acknowledge_all(&second)).unwrap_err();
    assert!(
        error.contains("stale_plan_registry_revision")
            || error.contains("stale_plan_previous_record")
            || error.contains("stale_plan_previous_body")
    );
}

#[test]
fn pending_journal_recovery_restores_every_uncommitted_phase_and_completes_commit() {
    let phases = [
        TransactionPhase::Prepared,
        TransactionPhase::BodyInstalled,
        TransactionPhase::LinksApplied,
        TransactionPhase::RegistryApplied,
        TransactionPhase::SnapshotsApplied,
        TransactionPhase::Committed,
    ];
    for phase in phases {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let registry = installed_skill_index_path(&context.runtime_home);
        let snapshot = snapshot_path(&context.runtime_home, "codex");
        let body = bodies_root(&context.runtime_home).join("recovery-team/new-revision");
        let link = context.host_home.join(".codex/skills/recovery-team");
        let old_target = context.host_home.join("old-body");
        fs::create_dir_all(&old_target).unwrap();
        fs::create_dir_all(body.parent().unwrap()).unwrap();
        fs::create_dir_all(&body).unwrap();
        fs::write(body.join("SKILL.md"), "pending body\n").unwrap();
        fs::create_dir_all(registry.parent().unwrap()).unwrap();
        fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let old_registry = b"old registry bytes\n".to_vec();
        let old_snapshot = b"old snapshot bytes\n".to_vec();
        fs::write(&registry, &old_registry).unwrap();
        fs::write(&snapshot, &old_snapshot).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&old_target, &link).unwrap();
        #[cfg(windows)]
        panic!("journal link recovery test requires unix");

        let expected_body_hash = hash_skill_source(&body).unwrap();
        fs::write(&registry, b"new registry bytes\n").unwrap();
        fs::write(&snapshot, b"new snapshot bytes\n").unwrap();
        fs::remove_file(&link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&body, &link).unwrap();
        let journal = TransactionJournal {
            schema_version: TRANSACTION_JOURNAL_SCHEMA.to_string(),
            transaction_id: "recovery-test".to_string(),
            operation: "install".to_string(),
            phase,
            body_path: body.to_string_lossy().into_owned(),
            expected_body_hash: Some(expected_body_hash),
            previous_body_hash: None,
            body_preexisting: false,
            registry: JournalFileState {
                path: registry.to_string_lossy().into_owned(),
                bytes: Some(old_registry.clone()),
            },
            links: vec![JournalLinkState {
                path: link.to_string_lossy().into_owned(),
                previous_target: Some(old_target.to_string_lossy().into_owned()),
            }],
            snapshots: vec![JournalFileState {
                path: snapshot.to_string_lossy().into_owned(),
                bytes: Some(old_snapshot.clone()),
            }],
        };
        fs::create_dir_all(
            transaction_journal_path(&context.runtime_home)
                .parent()
                .unwrap(),
        )
        .unwrap();
        fs::write(
            transaction_journal_path(&context.runtime_home),
            serde_json::to_vec_pretty(&journal).unwrap(),
        )
        .unwrap();

        recover_pending_transactions(&context.runtime_home, &context.host_home).unwrap();
        recover_pending_transactions(&context.runtime_home, &context.host_home).unwrap();
        if phase == TransactionPhase::Committed {
            assert_eq!(fs::read(&registry).unwrap(), b"new registry bytes\n");
            assert_eq!(fs::read(&snapshot).unwrap(), b"new snapshot bytes\n");
            assert!(body.is_dir());
        } else {
            assert_eq!(fs::read(&registry).unwrap(), old_registry);
            assert_eq!(fs::read(&snapshot).unwrap(), old_snapshot);
            assert_eq!(fs::read_link(&link).unwrap(), old_target);
            assert!(!body.exists());
        }
        assert!(!transaction_journal_path(&context.runtime_home).exists());
    }
}

#[test]
fn remote_tree_metadata_bounds_selected_subtree_not_unrelated_monorepo_content() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "monorepo-selected-team");
    let repository = source.parent().unwrap().to_path_buf();
    let backend =
        FixtureGitBackend::new(repository.clone(), &"f".repeat(40)).with_tree_metadata(vec![
            RemoteTreeEntry {
                path: "skill/SKILL.md".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 128,
            },
            RemoteTreeEntry {
                path: "skill/REFERENCE.md".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 64,
            },
            RemoteTreeEntry {
                path: "LICENSE".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 32,
            },
            RemoteTreeEntry {
                path: "vendor/unrelated-large.bin".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 32 * 1024 * 1024,
            },
            RemoteTreeEntry {
                path: "AGENTS.md".to_string(),
                kind: RemoteTreeEntryKind::Symlink,
                size: 9,
            },
        ]);
    let candidate = acquire_remote_candidate_with_backend(
        &context,
        &remote_source(&repository, "skill"),
        &backend,
    )
    .unwrap();
    assert!(candidate.skill_dir.join("SKILL.md").is_file());
}

#[test]
fn remote_tree_metadata_rejects_selected_subtree_before_checkout() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "monorepo-too-large-team");
    let repository = source.parent().unwrap().to_path_buf();
    let backend =
        FixtureGitBackend::new(repository.clone(), &"1".repeat(40)).with_tree_metadata(vec![
            RemoteTreeEntry {
                path: "skill/SKILL.md".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 2 * 1024 * 1024 + 1,
            },
            RemoteTreeEntry {
                path: "LICENSE".to_string(),
                kind: RemoteTreeEntryKind::Regular,
                size: 32,
            },
        ]);
    let error = acquire_remote_candidate_with_backend(
        &context,
        &remote_source(&repository, "skill"),
        &backend,
    )
    .unwrap_err();
    assert!(error.contains("exceeds"));
    assert!(!context.runtime_home.join("candidates").exists());
}
