use ags_capability_governance::build_capability_snapshot_with_roots;
use ags_capability_governance::skill_adoption::{
    acquire_remote_candidate, acquire_remote_candidate_with_backend, installed_skill_index_path,
    load_installed_skills, parse_github_url, plan_install, AdoptionContext, GitBackend,
    PreparedSkillChange, RemoteTreeEntry, RemoteTreeEntryKind, SnapshotDiscovery, SourceSpec,
    UpdatePolicy,
};
#[cfg(unix)]
use ags_capability_governance::skill_adoption::{
    materialize_skill_change, MaterializedBodyDisposition, ReadInputKind, RiskAcknowledgements,
};
#[cfg(unix)]
use ags_capability_governance::{hash_skill_source, load_static_snapshot, snapshot_path};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
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
    let context = AdoptionContext {
        authority_root: authority_root(),
        runtime_home: temp.path().join("runtime"),
        candidate_home: temp.path().join("runtime"),
        host_home: temp.path().join("home"),
        snapshot_discovery: SnapshotDiscovery::Offline,
    };
    seed_registered_base_snapshot(&context, "codex");
    context
}

fn seed_registered_base_snapshot(context: &AdoptionContext, host: &str) {
    let registration = ags_host_integration::HostRegistration::new(
        ags_host_integration::HostId::new(host).unwrap(),
        ags_host_integration::AgentSurface::Hybrid,
        ags_host_integration::platform_spec(host).map(|spec| spec.id.to_string()),
    );
    let registration_path = context
        .runtime_home
        .join("hosts")
        .join(host)
        .join("registration.json");
    fs::create_dir_all(registration_path.parent().unwrap()).unwrap();
    fs::write(
        registration_path,
        serde_json::to_vec_pretty(&registration).unwrap(),
    )
    .unwrap();
    let snapshot = build_capability_snapshot_with_roots(
        &context.authority_root,
        host,
        &context.runtime_home,
        &context.host_home,
    )
    .unwrap();
    ags_capability_governance::publish_capability_snapshots(
        &context.runtime_home,
        vec![(host.to_string(), snapshot)],
    )
    .unwrap();
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
        _destination: &ags_capability_governance::skill_adoption::HeldCheckout,
    ) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
        Ok(self.tree_metadata.clone())
    }

    fn materialize_selected(
        &self,
        _repository_url: &str,
        _resolved_commit: &str,
        destination: &ags_capability_governance::skill_adoption::HeldCheckout,
        _subdir: &str,
        _license_paths: &[String],
    ) -> Result<(), String> {
        destination.copy_from(&self.source_root)
    }

    fn validate_checkout(
        &self,
        _destination: &ags_capability_governance::skill_adoption::HeldCheckout,
    ) -> Result<(), String> {
        if self.reject_submodule {
            Err("submodule_refused: injected gitlink".to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
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

#[cfg(unix)]
fn acknowledge_all(plan: &PreparedSkillChange) -> RiskAcknowledgements {
    plan.risk_findings
        .iter()
        .map(|finding| finding.acknowledgement_id())
        .collect()
}

#[cfg(unix)]
#[test]
fn materialized_skill_refuses_already_exact_body_with_wrong_execute_mode() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "wrong-mode-team");
    let script = source.join("scripts/run.sh");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir_all(
        installed_skill_index_path(&context.runtime_home)
            .parent()
            .unwrap(),
    )
    .unwrap();
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    copy_fixture_tree(&source, Path::new(&plan.body_path)).unwrap();
    fs::set_permissions(
        Path::new(&plan.body_path).join("scripts/run.sh"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(
        error.contains("immutable") && error.contains("mode"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn materialized_skill_refuses_host_link_target_drift_after_plan() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "link-drift-team");
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let index = Path::new(&plan.host_indexes[0]);
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/tmp/third-party-skill", index).unwrap();
    let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(error.contains("link") && error.contains("drift"), "{error}");
}

#[cfg(unix)]
#[test]
fn materialized_skill_refuses_planning_window_h1_record_with_h2_candidate_seals() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "planning-window-team");
    let mut h1 = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    fs::write(
        source.join("REFERENCE.md"),
        "changed after audit before seal\n",
    )
    .unwrap();
    let h2 = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    h1.candidate_read_inputs = h2.candidate_read_inputs;
    let error = materialize_skill_change(&context, &h1, &acknowledge_all(&h1)).unwrap_err();
    assert!(
        error.contains("source_hash") || error.contains("candidate_hash"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn materialized_skill_refuses_candidate_hash_authority_not_matching_held_bytes() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "held-hash-team");
    let mut plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let forged = format!("sha256:{}", "0".repeat(64));
    plan.source_hash = forged.clone();
    plan.body_hash = forged.clone();
    let target = plan.target_record.as_mut().unwrap();
    target.source_hash = forged.clone();
    for revision in &mut target.body_revisions {
        revision.source_hash = forged.clone();
        revision.metadata.source_hash = forged.clone();
    }
    let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(
        error.contains("source_hash") || error.contains("candidate_hash"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn materialized_skill_body_rejects_directory_single_total_and_special_budgets() {
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "directory-budget-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        for index in 0..513 {
            fs::create_dir(source.join(format!("dir-{index:04}"))).unwrap();
        }
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(
            error.contains("directory") && error.contains("512"),
            "{error}"
        );
    }
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "single-budget-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        fs::write(
            source.join("oversized.bin"),
            vec![0_u8; 2 * 1024 * 1024 + 1],
        )
        .unwrap();
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(
            error.contains("file") && error.contains("2097152"),
            "{error}"
        );
    }
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "total-budget-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        for index in 0..9 {
            fs::write(
                source.join(format!("bounded-{index}.bin")),
                vec![index as u8; 2 * 1024 * 1024],
            )
            .unwrap();
        }
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(
            error.contains("total") && error.contains("16777216"),
            "{error}"
        );
    }
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "special-budget-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        let fifo = source.join("special.fifo");
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(error.contains("special_file_refused"), "{error}");
    }
    for (name, target) in [
        ("internal-symlink-team", "SKILL.md"),
        ("external-symlink-team", "../../outside.txt"),
    ] {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, name);
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        std::os::unix::fs::symlink(target, source.join("candidate-link")).unwrap();
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(error.contains("symlink_refused"), "{error}");
    }
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "same-size-drift-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        let reference = source.join("REFERENCE.md");
        let before = fs::read(&reference).unwrap();
        fs::write(&reference, vec![b'x'; before.len()]).unwrap();
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(
            error.contains("candidate") && error.contains("drift"),
            "{error}"
        );
    }
    {
        let temp = TempDir::new().unwrap();
        let context = context(&temp);
        let source = source_fixture(&temp, "inode-drift-team");
        let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
        let reference = source.join("REFERENCE.md");
        let replacement = source.join("REFERENCE.replacement");
        fs::write(&replacement, fs::read(&reference).unwrap()).unwrap();
        fs::rename(&replacement, &reference).unwrap();
        let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
        assert!(
            error.contains("candidate") && error.contains("drift"),
            "{error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn materialized_prefix_sibling_source_hash_matches_legacy_canonical_order() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "prefix-order-team");
    fs::create_dir_all(source.join("a")).unwrap();
    fs::write(source.join("a/child.txt"), b"child\n").unwrap();
    fs::write(source.join("a-thing.txt"), b"sibling\n").unwrap();

    let legacy_hash = hash_skill_source(&source).unwrap();
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    assert_eq!(plan.source_hash, legacy_hash);
    let materialized = materialize_skill_change(&context, &plan, &acknowledge_all(&plan))
        .expect("descriptor scan must preserve legacy pre-order canonical hashing");
    assert_eq!(materialized.skill_id, "prefix-order-team");
}

#[cfg(unix)]
#[test]
fn materialized_body_footprint_types_root_and_absent_parent_directories() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "footprint-team");
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let materialized = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap();
    let MaterializedBodyDisposition::CreateExact(body) = &materialized.body else {
        panic!("expected a new body");
    };
    let body_root = Path::new(&body.root);
    let skill_parent = body_root.parent().unwrap().to_string_lossy().into_owned();
    let bodies_parent = body_root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let write_paths = materialized.write_paths();
    assert!(write_paths.contains(&skill_parent));
    assert!(write_paths.contains(&bodies_parent));
    let encoded = serde_json::to_value(&materialized).unwrap();
    assert_eq!(encoded["body"]["root_mode"], 0o755);
    assert!(encoded["body"]["parent_directories"].is_array());
}

#[cfg(unix)]
#[test]
fn materialized_footprint_includes_all_absent_registry_snapshot_link_and_body_parents() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "complete-parent-footprint-team");
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let materialized = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap();
    let write_paths = materialized.write_paths();
    let mut required_parents = vec![Path::new(&materialized.registry.path)
        .parent()
        .unwrap()
        .to_path_buf()];
    required_parents.extend(materialized.snapshots.iter().map(|snapshot| {
        Path::new(&snapshot.file.path)
            .parent()
            .unwrap()
            .to_path_buf()
    }));
    required_parents.extend(
        materialized
            .links
            .iter()
            .map(|link| Path::new(&link.path).parent().unwrap().to_path_buf()),
    );
    let MaterializedBodyDisposition::CreateExact(body) = &materialized.body else {
        panic!("expected a new body");
    };
    required_parents.push(Path::new(&body.root).parent().unwrap().to_path_buf());
    let missing = required_parents
        .into_iter()
        .filter(|parent| !parent.exists())
        .filter(|parent| !write_paths.contains(&parent.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "absent parent directories are not typed write effects: {missing:?}"
    );
    let existing_anchor = temp.path().to_string_lossy().into_owned();
    assert!(
        !write_paths.contains(&existing_anchor),
        "an existing ancestor was incorrectly promoted to a write effect"
    );
    assert!(
        materialized.read_inputs.iter().any(|seal| {
            seal.root == existing_anchor
                && seal.relative_path.is_empty()
                && seal.kind == ReadInputKind::Directory
        }),
        "the held existing ancestor was not retained as a read seal"
    );
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

#[cfg(unix)]
#[test]
fn pure_overlay_requires_a_canonical_base_snapshot() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    fs::remove_file(snapshot_path(&context.runtime_home, "codex")).unwrap();
    let source = source_fixture(&temp, "snapshot-required-team");
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let error = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(error.contains("snapshot_required"), "{error}");
}

#[cfg(unix)]
#[test]
fn pure_overlay_preserves_mcp_runtime_and_unrelated_skill_rows() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let base = load_static_snapshot(&context.runtime_home, "codex")
        .unwrap()
        .0;
    let source = source_fixture(&temp, "pure-overlay-team");
    let plan = plan_local(&context, &source, None, &["codex".to_string()]).unwrap();
    let materialized = materialize_skill_change(&context, &plan, &acknowledge_all(&plan)).unwrap();
    let candidate: ags_capability_governance::HostCapabilitySnapshot =
        serde_json::from_slice(&materialized.snapshots[0].file.post_bytes).unwrap();

    assert_eq!(
        candidate.runtime_observation_hash,
        base.runtime_observation_hash
    );
    assert_eq!(candidate.mcp_catalog, base.mcp_catalog);
    assert_eq!(candidate.active_mcps, base.active_mcps);
    assert_eq!(candidate.third_party_catalog, base.third_party_catalog);
    let unrelated = candidate
        .catalog
        .iter()
        .filter(|card| card.skill_id != "pure-overlay-team")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(unrelated, base.catalog);
}
#[cfg(unix)]
#[test]
fn materialization_revalidates_license_and_routing_side_input_seals() {
    let temp = TempDir::new().unwrap();
    let adoption_context = context(&temp);
    let source = source_fixture(&temp, "side-revalidation-team");
    let license = source.parent().unwrap().join("LICENSE");
    let routing = temp.path().join("routing.yaml");
    fs::write(&routing, b"summary: Sealed routing metadata.\n").unwrap();
    let plan = plan_local(
        &adoption_context,
        &source,
        Some(&routing),
        &["codex".to_string()],
    )
    .unwrap();

    let materialized =
        materialize_skill_change(&adoption_context, &plan, &acknowledge_all(&plan)).unwrap();
    for expected in &plan.candidate_read_inputs {
        assert!(
            materialized.read_inputs.contains(expected),
            "planned candidate/side seal was not retained in materialization read_inputs: {expected:?}"
        );
    }

    fs::write(&license, b"MIT fixture license changed after plan\n").unwrap();
    let error =
        materialize_skill_change(&adoption_context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(
        error.contains("candidate_side_input_drift_after_plan"),
        "{error}"
    );

    let temp = TempDir::new().unwrap();
    let adoption_context = context(&temp);
    let source = source_fixture(&temp, "routing-revalidation-team");
    let routing = temp.path().join("routing.yaml");
    fs::write(&routing, b"summary: Initial routing.\n").unwrap();
    let plan = plan_local(
        &adoption_context,
        &source,
        Some(&routing),
        &["codex".to_string()],
    )
    .unwrap();
    fs::write(&routing, b"summary: Changed routing.\n").unwrap();
    let error =
        materialize_skill_change(&adoption_context, &plan, &acknowledge_all(&plan)).unwrap_err();
    assert!(
        error.contains("candidate_side_input_drift_after_plan"),
        "{error}"
    );
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

fn actual_git_monorepo_fixture(temp: &TempDir) -> (PathBuf, String) {
    let repository = temp.path().join("multi-skill-git-source");
    fs::create_dir_all(&repository).unwrap();
    fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
    for name in ["alpha", "beta", "gamma"] {
        let skill = repository.join("skills").join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Multi-skill Git fixture.\n---\n\n# {name}\n"),
        )
        .unwrap();
        fs::write(skill.join("REFERENCE.md"), format!("{name} body\n")).unwrap();
    }
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
    (repository, commit)
}

fn contains_git_metadata(root: &Path) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name() == ".git" {
                return true;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    false
}

#[test]
fn private_adoption_cannot_shadow_the_reserved_compatibility_parent() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let source = source_fixture(&temp, "superpowers");
    let error = plan_local(&context, &source, None, &["codex".to_string()]).unwrap_err();
    assert!(error.contains("cannot shadow catalog compatibility parent"));
    assert!(error.contains("install its reviewed distribution id"));
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
        "schema_version": "retired-schema",
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
fn system_git_materializes_a_complete_multi_file_root_skill() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let repository = temp.path().join("root-skill-git-source");
    fs::create_dir_all(repository.join("scripts")).unwrap();
    fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
    fs::write(
        repository.join("SKILL.md"),
        "---\nname: root-team\ndescription: Root Skill fixture.\n---\n",
    )
    .unwrap();
    fs::write(repository.join("REFERENCE.md"), "root reference\n").unwrap();
    fs::write(repository.join("scripts/helper.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    run_git(&repository, &["init", "--quiet"]);
    run_git(&repository, &["config", "user.name", "fixture"]);
    run_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    run_git(&repository, &["add", "--all"]);
    run_git(&repository, &["commit", "--quiet", "-m", "root fixture"]);
    let commit = run_git(&repository, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let source = SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: Some(commit),
        tracking_ref: Some("main".to_string()),
        subdir: None,
    };

    let candidate = acquire_remote_candidate(&context, &source).unwrap();
    assert!(candidate.skill_dir.join("SKILL.md").is_file());
    assert!(candidate.skill_dir.join("REFERENCE.md").is_file());
    assert!(candidate.skill_dir.join("scripts/helper.sh").is_file());
    assert!(!contains_git_metadata(&candidate.checkout_root));
}

#[test]
fn system_git_reuses_sealed_candidates_for_same_commit_and_subdir() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let (repository, commit) = actual_git_monorepo_fixture(&temp);
    let mut identities = Vec::new();

    for subdir in ["skills/alpha", "skills/beta", "skills/gamma"] {
        let source = SourceSpec::Git {
            url: format!("file://{}", repository.display()),
            requested_ref: Some(commit.clone()),
            tracking_ref: Some("main".to_string()),
            subdir: Some(subdir.to_string()),
        };
        for _ in 0..2 {
            let candidate = acquire_remote_candidate(&context, &source).unwrap();
            assert!(candidate.skill_dir.join("SKILL.md").is_file());
            assert!(!contains_git_metadata(&candidate.checkout_root));
            assert!(candidate
                .checkout_root
                .parent()
                .unwrap()
                .join("candidate-manifest.json")
                .is_file());
            identities.push(candidate.resolved_source.candidate_identity);
        }
    }

    assert_eq!(identities[0], identities[1]);
    assert_eq!(identities[2], identities[3]);
    assert_eq!(identities[4], identities[5]);
    assert_ne!(identities[0], identities[2]);
    assert_ne!(identities[2], identities[4]);

    fs::remove_dir_all(context.runtime_home.join("candidates")).unwrap();
    for (index, subdir) in ["skills/alpha", "skills/beta", "skills/gamma"]
        .into_iter()
        .enumerate()
    {
        let source = SourceSpec::Git {
            url: format!("file://{}", repository.display()),
            requested_ref: Some(commit.clone()),
            tracking_ref: Some("main".to_string()),
            subdir: Some(subdir.to_string()),
        };
        let rebuilt = acquire_remote_candidate(&context, &source).unwrap();
        assert_eq!(
            rebuilt.resolved_source.candidate_identity,
            identities[index * 2]
        );
        assert!(!contains_git_metadata(&rebuilt.checkout_root));
    }
}

#[test]
fn candidate_identity_ignores_mutable_ref_spelling_after_commit_resolution() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let (repository, _skill, commit) = actual_git_source_fixture(&temp, "ref-alias-team");
    let pinned = SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: Some(commit.clone()),
        tracking_ref: Some("main".to_string()),
        subdir: Some("skill".to_string()),
    };
    let tracked = SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: Some("master".to_string()),
        tracking_ref: Some("master".to_string()),
        subdir: Some("skill".to_string()),
    };

    let pinned = acquire_remote_candidate(&context, &pinned).unwrap();
    let tracked = acquire_remote_candidate(&context, &tracked).unwrap();
    assert_eq!(
        pinned.resolved_source.candidate_identity,
        tracked.resolved_source.candidate_identity
    );
}

#[test]
fn cached_candidate_body_drift_is_quarantined_and_rebuilt() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let (repository, _skill, commit) = actual_git_source_fixture(&temp, "cache-drift-team");
    let source = SourceSpec::Git {
        url: format!("file://{}", repository.display()),
        requested_ref: Some(commit),
        tracking_ref: Some("main".to_string()),
        subdir: Some("skill".to_string()),
    };

    let first = acquire_remote_candidate(&context, &source).unwrap();
    let expected_hash = first.record.source_hash;
    fs::write(
        first.skill_dir.join("SKILL.md"),
        "---\nname: cache-drift-team\ndescription: Tampered cache.\n---\n",
    )
    .unwrap();
    fs::create_dir_all(first.skill_dir.join("nested/.git/objects")).unwrap();
    fs::write(
        first.skill_dir.join("nested/.git/objects/injected"),
        "untrusted cache metadata\n",
    )
    .unwrap();

    let rebuilt = acquire_remote_candidate(&context, &source).unwrap();
    assert_eq!(rebuilt.record.source_hash, expected_hash);
    let quarantine = context.runtime_home.join("candidates/quarantine");
    assert!(quarantine.read_dir().unwrap().next().is_some());
    assert!(!contains_git_metadata(&quarantine));
}

#[test]
fn new_remote_candidate_git_cleanup_is_bounded_before_unrelated_tree_traversal() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let skill = source_fixture(&temp, "git-cleanup-budget-team");
    let repository = skill.parent().unwrap().to_path_buf();
    for index in 0..=1024 {
        fs::create_dir(repository.join(format!("unrelated-{index:04}"))).unwrap();
    }
    let source = remote_source(&repository, "skill");
    let error = acquire_remote_candidate_with_backend(
        &context,
        &source,
        &FixtureGitBackend::new(repository, &"6".repeat(40)),
    )
    .expect_err("unbounded cleanup accepted an adversarial materialized checkout");
    assert!(
        error.contains("git_metadata_directory_budget_exceeded"),
        "unexpected cleanup budget error: {error}"
    );
}

#[test]
fn new_remote_candidate_git_cleanup_bounds_total_members_before_stat() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let skill = source_fixture(&temp, "git-cleanup-member-budget-team");
    let repository = skill.parent().unwrap().to_path_buf();
    for index in 0..=1024 {
        fs::write(repository.join(format!("unrelated-{index:04}")), b"x").unwrap();
    }
    let source = remote_source(&repository, "skill");
    let error = acquire_remote_candidate_with_backend(
        &context,
        &source,
        &FixtureGitBackend::new(repository, &"8".repeat(40)),
    )
    .expect_err("cleanup accepted more entries than its descriptor walker budget");
    assert!(
        error.contains("git_metadata_entry_budget_exceeded"),
        "unexpected cleanup member-budget error: {error}"
    );
}

#[test]
fn malicious_cached_candidate_over_budget_is_quarantined_before_reuse() {
    let temp = TempDir::new().unwrap();
    let context = context(&temp);
    let skill = source_fixture(&temp, "cached-git-cleanup-budget-team");
    let repository = skill.parent().unwrap().to_path_buf();
    let source = remote_source(&repository, "skill");
    let backend = FixtureGitBackend::new(repository, &"7".repeat(40));
    let cached = acquire_remote_candidate_with_backend(&context, &source, &backend).unwrap();
    for index in 0..=1024 {
        fs::create_dir(cached.checkout_root.join(format!("injected-{index:04}"))).unwrap();
    }

    let error = acquire_remote_candidate_with_backend(&context, &source, &backend)
        .expect_err("over-budget cached checkout must fail closed after quarantine");
    assert!(
        error.contains("git_metadata_directory_budget_exceeded"),
        "unexpected cached cleanup error: {error}"
    );
    let quarantine = context.runtime_home.join("candidates/quarantine");
    assert!(
        quarantine.is_dir() && quarantine.read_dir().unwrap().next().is_some(),
        "over-budget cached checkout was reused instead of quarantined"
    );
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
