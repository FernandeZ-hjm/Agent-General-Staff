use ags_verification::{
    Scope, VerificationBundle, VerificationReport, VerificationSummary, TEST_POLICY_VERSION,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

fn passing_report() -> VerificationReport {
    VerificationReport {
        schema_version: "0.3.6-verification-report".to_string(),
        scope: Scope::Local,
        repo_root: "/fixture".to_string(),
        items: Vec::new(),
        summary: VerificationSummary {
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            warnings: 0,
        },
    }
}

fn fixture_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.lock"), "# verification fixture\n").unwrap();
    fs::write(dir.path().join("tracked.txt"), "clean\n").unwrap();
    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["add", "Cargo.lock", "tracked.txt"]);
    run_git(dir.path(), &["commit", "-qm", "fixture"]);
    dir
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    if args.first() == Some(&"commit") {
        command
            .env("GIT_AUTHOR_NAME", "verification-fixture")
            .env("GIT_AUTHOR_EMAIL", "verification-fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "verification-fixture")
            .env(
                "GIT_COMMITTER_EMAIL",
                "verification-fixture@example.invalid",
            );
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn valid_bundle(repo_root: &Path) -> VerificationBundle {
    VerificationBundle::create(
        repo_root,
        "local",
        vec!["cargo test -p ags-verification".to_string()],
        vec!["ags-verification::tests::scope_and_check_item_contract_matrix".to_string()],
        BTreeMap::from([("report.json".to_string(), ags_platform::sha256(b"report"))]),
        passing_report(),
        true,
    )
    .unwrap()
}

#[test]
fn bundle_hash_is_content_addressed_and_excludes_itself() {
    let repo = fixture_repo();
    let bundle = valid_bundle(repo.path());
    assert_eq!(bundle.compute_bundle_hash().unwrap(), bundle.bundle_hash);

    let mut without_hash = bundle.clone();
    without_hash.bundle_hash.clear();
    assert_eq!(
        without_hash.compute_bundle_hash().unwrap(),
        bundle.compute_bundle_hash().unwrap()
    );
}

#[test]
fn exact_current_inputs_are_required_for_reuse() {
    let repo = fixture_repo();
    let bundle = valid_bundle(repo.path());
    bundle.validate_reuse(repo.path()).unwrap();
    bundle
        .validate_reuse_for(repo.path(), "release", TEST_POLICY_VERSION)
        .unwrap_err();

    let mut wrong_commit = bundle.clone();
    wrong_commit.commit_sha = "main".to_string();
    assert!(wrong_commit.validate_reuse(repo.path()).is_err());
}

#[test]
fn malformed_and_duplicate_evidence_is_rejected() {
    let repo = fixture_repo();
    let mut duplicate = valid_bundle(repo.path());
    duplicate
        .test_inventory
        .push(duplicate.test_inventory[0].clone());
    assert!(duplicate.validate_reuse(repo.path()).is_err());

    let mut malformed_artifact = valid_bundle(repo.path());
    malformed_artifact
        .artifact_hashes
        .insert("bad.bin".to_string(), "not-a-hash".to_string());
    assert!(malformed_artifact.validate_reuse(repo.path()).is_err());

    let mut tampered_hash = valid_bundle(repo.path());
    tampered_hash.bundle_hash = ags_platform::sha256(b"tampered");
    assert!(tampered_hash.validate_reuse(repo.path()).is_err());
}

#[test]
fn failed_result_cannot_be_reused() {
    let repo = fixture_repo();
    let result = VerificationBundle::create(
        repo.path(),
        "local",
        vec!["cargo test -p ags-verification".to_string()],
        vec!["ags-verification::failed".to_string()],
        BTreeMap::new(),
        passing_report(),
        false,
    );
    assert!(result.is_err());
}

#[test]
fn tracked_dirty_worktree_is_rejected_for_creation_and_reuse() {
    let repo = fixture_repo();
    let bundle = valid_bundle(repo.path());
    fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();

    let reuse_error = bundle.validate_reuse(repo.path()).unwrap_err();
    assert!(reuse_error.contains("worktree is dirty"), "{reuse_error}");
    let create_error = VerificationBundle::create(
        repo.path(),
        "local",
        vec!["cargo test -p ags-verification".to_string()],
        vec!["fixture::tracked".to_string()],
        BTreeMap::new(),
        passing_report(),
        true,
    )
    .unwrap_err();
    assert!(create_error.contains("worktree is dirty"), "{create_error}");
}

#[test]
fn untracked_dirty_worktree_is_rejected_for_creation_and_reuse() {
    let repo = fixture_repo();
    let bundle = valid_bundle(repo.path());
    fs::write(repo.path().join("untracked.txt"), "new\n").unwrap();

    let reuse_error = bundle.validate_reuse(repo.path()).unwrap_err();
    assert!(reuse_error.contains("worktree is dirty"), "{reuse_error}");
    let create_error = VerificationBundle::create(
        repo.path(),
        "local",
        vec!["cargo test -p ags-verification".to_string()],
        vec!["fixture::untracked".to_string()],
        BTreeMap::new(),
        passing_report(),
        true,
    )
    .unwrap_err();
    assert!(create_error.contains("worktree is dirty"), "{create_error}");
}

#[test]
fn hidden_index_flags_are_rejected_even_when_git_status_looks_clean() {
    let repo = fixture_repo();
    let bundle = valid_bundle(repo.path());

    run_git(
        repo.path(),
        &["update-index", "--skip-worktree", "tracked.txt"],
    );
    let error = bundle.validate_reuse(repo.path()).unwrap_err();
    assert!(error.contains("skip-worktree"), "{error}");

    run_git(
        repo.path(),
        &["update-index", "--no-skip-worktree", "tracked.txt"],
    );
    run_git(
        repo.path(),
        &["update-index", "--assume-unchanged", "tracked.txt"],
    );
    let error = bundle.validate_reuse(repo.path()).unwrap_err();
    assert!(error.contains("assume-unchanged"), "{error}");
}
