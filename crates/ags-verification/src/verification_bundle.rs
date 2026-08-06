//! Content-addressed verification evidence.
//!
//! A [`VerificationBundle`] is reusable only when the inputs that produced it
//! are the same inputs available to the current repository.  In particular,
//! a branch name or a filesystem path is never used as a substitute for the
//! immutable Git commit and tree identities.

use crate::VerificationReport;
use ags_platform::sha256;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Schema version for serialized verification bundles.
pub const VERIFICATION_BUNDLE_SCHEMA_VERSION: &str = "0.4.13-verification-bundle";

/// Version of the verification policy whose tests are represented by a
/// bundle.  A policy change must invalidate old evidence even when the source
/// commit itself has not changed.
pub const TEST_POLICY_VERSION: &str = "ags-verification-test-policy/2";

/// Immutable repository and toolchain inputs that identify a verification
/// run.  This is also useful to callers that need to show why a bundle cannot
/// be reused.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationInputIdentity {
    pub commit_sha: String,
    pub tree_hash: String,
    pub rustc_version: String,
    pub cargo_version: String,
    pub cargo_lock_hash: String,
}

/// Content-addressed verification evidence.
///
/// `bundle_hash` is the SHA-256 digest of the canonical serialization of all
/// other fields.  The artifact map is a `BTreeMap` deliberately: serialized
/// key order is part of the stable canonical representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationBundle {
    pub schema_version: String,
    pub bundle_hash: String,
    pub commit_sha: String,
    pub tree_hash: String,
    pub source_scope: String,
    pub rustc_version: String,
    pub cargo_version: String,
    pub cargo_lock_hash: String,
    pub test_policy_version: String,
    pub commands: Vec<String>,
    pub test_inventory: Vec<String>,
    pub artifact_hashes: BTreeMap<String, String>,
    pub verification_report: VerificationReport,
    pub result_identity: String,
    pub final_result: bool,
    pub created_at_unix: u64,
}

#[derive(Debug, Serialize)]
struct CanonicalBundle<'a> {
    schema_version: &'a str,
    commit_sha: &'a str,
    tree_hash: &'a str,
    source_scope: &'a str,
    rustc_version: &'a str,
    cargo_version: &'a str,
    cargo_lock_hash: &'a str,
    test_policy_version: &'a str,
    commands: &'a [String],
    test_inventory: &'a [String],
    artifact_hashes: &'a BTreeMap<String, String>,
    verification_report: &'a VerificationReport,
    result_identity: &'a str,
    final_result: bool,
    created_at_unix: u64,
}

#[derive(Debug, Serialize)]
struct CanonicalResult<'a> {
    verification_report: &'a VerificationReport,
    final_result: bool,
}

impl VerificationBundle {
    /// Create a bundle using the current repository inputs and the canonical
    /// verification policy version.
    pub fn create(
        repo_root: &Path,
        source_scope: impl Into<String>,
        commands: Vec<String>,
        test_inventory: Vec<String>,
        artifact_hashes: BTreeMap<String, String>,
        verification_report: VerificationReport,
        final_result: bool,
    ) -> Result<Self, String> {
        Self::from_current(
            repo_root,
            source_scope,
            TEST_POLICY_VERSION,
            commands,
            test_inventory,
            artifact_hashes,
            verification_report,
            final_result,
            unix_now(),
        )
    }

    /// Create a bundle using explicitly supplied policy and timestamp values.
    /// The explicit form is useful for deterministic producers and fixtures;
    /// reuse through [`Self::validate_reuse`] still requires the current
    /// `TEST_POLICY_VERSION`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_current(
        repo_root: &Path,
        source_scope: impl Into<String>,
        test_policy_version: impl Into<String>,
        commands: Vec<String>,
        test_inventory: Vec<String>,
        artifact_hashes: BTreeMap<String, String>,
        verification_report: VerificationReport,
        final_result: bool,
        created_at_unix: u64,
    ) -> Result<Self, String> {
        let identity = current_input_identity(repo_root)?;
        let mut bundle = Self {
            schema_version: VERIFICATION_BUNDLE_SCHEMA_VERSION.to_string(),
            bundle_hash: String::new(),
            commit_sha: identity.commit_sha,
            tree_hash: identity.tree_hash,
            source_scope: source_scope.into(),
            rustc_version: identity.rustc_version,
            cargo_version: identity.cargo_version,
            cargo_lock_hash: identity.cargo_lock_hash,
            test_policy_version: test_policy_version.into(),
            commands,
            test_inventory,
            artifact_hashes,
            verification_report,
            result_identity: String::new(),
            final_result,
            created_at_unix,
        };

        bundle.validate_fields(false)?;
        bundle.result_identity = bundle.compute_result_identity()?;
        bundle.bundle_hash = bundle.compute_bundle_hash()?;
        bundle.validate_fields(true)?;
        Ok(bundle)
    }

    /// Return the current timestamp in Unix seconds.
    pub fn now_unix() -> u64 {
        unix_now()
    }

    /// Compute the canonical result identity for a report/result pair.
    pub fn result_identity_for(
        verification_report: &VerificationReport,
        final_result: bool,
    ) -> Result<String, String> {
        let canonical = CanonicalResult {
            verification_report,
            final_result,
        };
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| format!("cannot serialize verification result: {error}"))?;
        Ok(sha256(bytes))
    }

    /// Compute the bundle's canonical content hash.  `bundle_hash` is
    /// intentionally absent from the input.
    pub fn compute_bundle_hash(&self) -> Result<String, String> {
        let canonical = self.canonical_payload();
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| format!("cannot serialize verification bundle: {error}"))?;
        Ok(sha256(bytes))
    }

    /// Validate that this bundle can be reused for the current repository and
    /// current verification policy.
    pub fn validate_reuse(&self, repo_root: &Path) -> Result<(), String> {
        self.validate_reuse_for(repo_root, &self.source_scope, TEST_POLICY_VERSION)
    }

    /// Validate reuse while binding the caller's exact source scope and policy
    /// inputs.  The Git and toolchain inputs are always recomputed here.
    pub fn validate_reuse_for(
        &self,
        repo_root: &Path,
        source_scope: &str,
        test_policy_version: &str,
    ) -> Result<(), String> {
        self.validate_fields(true)?;
        if self.source_scope != source_scope {
            return Err(format!(
                "verification bundle source_scope mismatch: bundle `{}`, current `{source_scope}`",
                self.source_scope
            ));
        }
        if self.test_policy_version != test_policy_version {
            return Err(format!(
                "verification bundle test_policy_version mismatch: bundle `{}`, current `{test_policy_version}`",
                self.test_policy_version
            ));
        }

        let current = current_input_identity(repo_root)?;
        compare_identity(self, &current)?;
        Ok(())
    }

    /// Alias with a verb that reads naturally at call sites that gate a cache
    /// lookup.
    pub fn validate_current_reuse(&self, repo_root: &Path) -> Result<(), String> {
        self.validate_reuse(repo_root)
    }

    fn canonical_payload(&self) -> CanonicalBundle<'_> {
        CanonicalBundle {
            schema_version: &self.schema_version,
            commit_sha: &self.commit_sha,
            tree_hash: &self.tree_hash,
            source_scope: &self.source_scope,
            rustc_version: &self.rustc_version,
            cargo_version: &self.cargo_version,
            cargo_lock_hash: &self.cargo_lock_hash,
            test_policy_version: &self.test_policy_version,
            commands: &self.commands,
            test_inventory: &self.test_inventory,
            artifact_hashes: &self.artifact_hashes,
            verification_report: &self.verification_report,
            result_identity: &self.result_identity,
            final_result: self.final_result,
            created_at_unix: self.created_at_unix,
        }
    }

    fn compute_result_identity(&self) -> Result<String, String> {
        Self::result_identity_for(&self.verification_report, self.final_result)
    }

    fn validate_fields(&self, require_success: bool) -> Result<(), String> {
        if self.schema_version != VERIFICATION_BUNDLE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported verification bundle schema_version `{}`",
                self.schema_version
            ));
        }
        validate_git_object("commit_sha", &self.commit_sha)?;
        validate_git_object("tree_hash", &self.tree_hash)?;
        if self.source_scope.trim().is_empty() {
            return Err("verification bundle source_scope is empty".to_string());
        }
        if self.rustc_version.trim().is_empty() {
            return Err("verification bundle rustc_version is empty".to_string());
        }
        if self.cargo_version.trim().is_empty() {
            return Err("verification bundle cargo_version is empty".to_string());
        }
        validate_sha256("cargo_lock_hash", &self.cargo_lock_hash)?;
        if self.test_policy_version.trim().is_empty() {
            return Err("verification bundle test_policy_version is empty".to_string());
        }
        if self.commands.is_empty()
            || self
                .commands
                .iter()
                .any(|command| command.trim().is_empty())
        {
            return Err("verification bundle commands must contain non-empty commands".to_string());
        }
        validate_test_inventory(&self.test_inventory)?;
        validate_artifact_hashes(&self.artifact_hashes)?;
        if self.verification_report.schema_version.trim().is_empty() {
            return Err(
                "verification bundle verification_report schema_version is empty".to_string(),
            );
        }
        if self.result_identity.is_empty() && !require_success {
            // Construction fills this field immediately after structural
            // validation and before the final integrity pass.
        } else {
            validate_sha256("result_identity", &self.result_identity)?;
            let expected_result_identity = self.compute_result_identity()?;
            if self.result_identity != expected_result_identity {
                return Err(format!(
                    "verification bundle result_identity mismatch: expected `{expected_result_identity}`, actual `{}`",
                    self.result_identity
                ));
            }
        }
        if require_success && (!self.final_result || !self.verification_report.passed()) {
            return Err("verification bundle contains a failed final result".to_string());
        }
        if self.bundle_hash.is_empty() {
            if require_success {
                return Err("verification bundle bundle_hash is empty".to_string());
            }
        } else {
            validate_sha256("bundle_hash", &self.bundle_hash)?;
            let expected_bundle_hash = self.compute_bundle_hash()?;
            if self.bundle_hash != expected_bundle_hash {
                return Err(format!(
                    "verification bundle bundle_hash mismatch: expected `{expected_bundle_hash}`, actual `{}`",
                    self.bundle_hash
                ));
            }
        }
        Ok(())
    }
}

/// Recompute the current repository inputs used by a verification bundle.
pub fn current_input_identity(repo_root: &Path) -> Result<VerificationInputIdentity, String> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize verification repository: {error}"))?;
    ensure_clean_worktree(&repo_root)?;
    let commit_sha = git_identity(&repo_root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let tree_hash = git_identity(&repo_root, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    let rustc_version = tool_version("rustc", &repo_root)?;
    let cargo_version = tool_version("cargo", &repo_root)?;
    let lock_path = repo_root.join("Cargo.lock");
    let lock_bytes = std::fs::read(&lock_path).map_err(|error| {
        format!(
            "cannot read Cargo.lock for verification bundle ({}): {error}",
            lock_path.display()
        )
    })?;

    Ok(VerificationInputIdentity {
        commit_sha,
        tree_hash,
        rustc_version,
        cargo_version,
        cargo_lock_hash: sha256(lock_bytes),
    })
}

fn ensure_clean_worktree(repo_root: &Path) -> Result<(), String> {
    ensure_no_hidden_index_entries(repo_root)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .map_err(|error| format!("cannot inspect verification worktree status: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git status failed while checking verification worktree: {}",
            output_summary(&output.stdout, &output.stderr)
        ));
    }
    if !output.stdout.is_empty() {
        return Err(format!(
            "verification worktree is dirty; commit or remove tracked/untracked changes before creating or reusing a bundle: {}",
            String::from_utf8_lossy(&output.stdout).trim_end()
        ));
    }
    Ok(())
}

fn ensure_no_hidden_index_entries(repo_root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", "-v", "-z"])
        .output()
        .map_err(|error| format!("cannot inspect verification index flags: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed while checking verification index flags: {}",
            output_summary(&output.stdout, &output.stderr)
        ));
    }
    let hidden = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| {
            record
                .first()
                .is_some_and(|tag| *tag == b'S' || tag.is_ascii_lowercase())
        })
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect::<Vec<_>>();
    if !hidden.is_empty() {
        return Err(format!(
            "verification index contains skip-worktree or assume-unchanged entries that can hide tested bytes from the commit tree: {}",
            hidden.join(", ")
        ));
    }
    Ok(())
}

/// Validate a bundle against an exact caller-provided source scope and policy.
pub fn validate_bundle_for_reuse(
    bundle: &VerificationBundle,
    repo_root: &Path,
    source_scope: &str,
    test_policy_version: &str,
) -> Result<(), String> {
    bundle.validate_reuse_for(repo_root, source_scope, test_policy_version)
}

fn compare_identity(
    bundle: &VerificationBundle,
    current: &VerificationInputIdentity,
) -> Result<(), String> {
    for (field, bundled, actual) in [
        ("commit_sha", &bundle.commit_sha, &current.commit_sha),
        ("tree_hash", &bundle.tree_hash, &current.tree_hash),
        (
            "rustc_version",
            &bundle.rustc_version,
            &current.rustc_version,
        ),
        (
            "cargo_version",
            &bundle.cargo_version,
            &current.cargo_version,
        ),
        (
            "cargo_lock_hash",
            &bundle.cargo_lock_hash,
            &current.cargo_lock_hash,
        ),
    ] {
        if bundled != actual {
            return Err(format!(
                "verification bundle {field} mismatch: bundle `{bundled}`, current `{actual}`"
            ));
        }
    }
    Ok(())
}

fn validate_test_inventory(inventory: &[String]) -> Result<(), String> {
    if inventory.is_empty() {
        return Err("verification bundle test_inventory is empty".to_string());
    }
    let mut seen = BTreeSet::new();
    for test_id in inventory {
        if test_id.trim() != test_id || test_id.is_empty() || test_id.chars().any(char::is_control)
        {
            return Err(format!(
                "verification bundle test_inventory contains malformed test ID `{test_id}`"
            ));
        }
        if !seen.insert(test_id) {
            return Err(format!(
                "verification bundle test_inventory contains duplicate test identity `{test_id}`"
            ));
        }
    }
    Ok(())
}

fn validate_artifact_hashes(artifacts: &BTreeMap<String, String>) -> Result<(), String> {
    for (artifact, hash) in artifacts {
        if artifact.trim() != artifact
            || artifact.is_empty()
            || artifact.chars().any(char::is_control)
        {
            return Err(format!(
                "verification bundle artifact name is malformed `{artifact}`"
            ));
        }
        validate_sha256(&format!("artifact_hashes[{artifact}]"), hash)?;
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(format!(
            "verification bundle {field} must be lowercase sha256:<64-hex>"
        ));
    };
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        return Err(format!(
            "verification bundle {field} must be lowercase sha256:<64-hex>"
        ));
    }
    Ok(())
}

fn validate_git_object(field: &str, value: &str) -> Result<(), String> {
    let valid_length = value.len() == 40 || value.len() == 64;
    if !valid_length || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(format!(
            "verification bundle {field} must be a Git object ID, not a branch or path"
        ));
    }
    Ok(())
}

fn git_identity(repo_root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|error| format!("cannot execute git for verification identity: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git identity command failed: {}",
            output_summary(&output.stdout, &output.stderr)
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("git identity output is not UTF-8: {error}"))?
        .trim()
        .to_string();
    validate_git_object("git identity", &value)?;
    Ok(value)
}

fn tool_version(program: &str, repo_root: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .current_dir(repo_root)
        .arg("--version")
        .output()
        .map_err(|error| format!("cannot execute {program} --version: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} --version failed: {}",
            output_summary(&output.stdout, &output.stderr)
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("{program} --version output is not UTF-8: {error}"))?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(format!("{program} --version returned empty output"));
    }
    Ok(value)
}

fn output_summary(stdout: &[u8], stderr: &[u8]) -> String {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    combined.chars().take(1200).collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
