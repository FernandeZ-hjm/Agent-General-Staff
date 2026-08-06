//! Transactional projection from private authority A into public checkout B.
//!
//! A owns shared source. B owns only the explicitly hash-pinned overlays and
//! rewrites. Generated capability files are produced by the typed capability
//! projector. One plan hash binds all three classes plus removal of retired
//! tracked files.

use crate::public_capability_projection::{
    apply_public_capability_projection, plan_public_capability_projection,
    PublicCapabilityProjectionPlan, PublicCapabilityProjectionReceipt,
};
use crate::release_manifest::{
    load_public_payload_authority, public_payload_files, public_source_payload_files,
    verify_promotion_manifest,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSourceWrite {
    pub path: String,
    pub source_sha256: String,
    pub target_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSourceDelete {
    pub path: String,
    pub target_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSourceProjectionPlan {
    pub schema_version: String,
    pub source_root: PathBuf,
    pub target_root: PathBuf,
    pub plan_hash: String,
    pub writes: Vec<PublicSourceWrite>,
    pub deletes: Vec<PublicSourceDelete>,
    pub capability_projection: PublicCapabilityProjectionPlan,
    pub blocking_findings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSourceProjectionReceipt {
    pub schema_version: String,
    pub plan_hash: String,
    pub target_root: PathBuf,
    pub written_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub capability_projection: PublicCapabilityProjectionReceipt,
    pub verified: bool,
}

pub fn plan_public_source_projection(
    source_root: &Path,
    target_root: &Path,
) -> PublicSourceProjectionPlan {
    let capability_projection = plan_public_capability_projection(source_root, target_root);
    let mut blocking = capability_projection.blocking_findings.clone();
    let authority = load_public_payload_authority(source_root).map_err(|errors| {
        blocking.extend(errors);
    });
    let source_files = public_source_payload_files(source_root).map_err(|errors| {
        blocking.extend(errors);
    });
    let expected_files = public_payload_files(source_root).map_err(|errors| {
        blocking.extend(errors);
    });
    let tracked_files = tracked_files(target_root).map_err(|error| blocking.push(error));
    let untracked_files = untracked_files(target_root).map_err(|error| blocking.push(error));

    let mut writes = Vec::new();
    let mut deletes = Vec::new();
    if let (Ok(authority), Ok(source_files)) = (&authority, &source_files) {
        let rewrites = authority
            .public_rewrites
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        for relative in source_files {
            if rewrites.contains(relative.as_str()) {
                continue;
            }
            match safe_regular_bytes(source_root, relative) {
                Ok(source_bytes) => match optional_safe_regular_bytes(target_root, relative) {
                    Ok(target_bytes) => {
                        let source_sha256 = ags_platform::sha256(&source_bytes);
                        let target_sha256 = target_bytes.as_ref().map(ags_platform::sha256);
                        if target_sha256.as_deref() != Some(source_sha256.as_str()) {
                            writes.push(PublicSourceWrite {
                                path: relative.clone(),
                                source_sha256,
                                target_sha256,
                            });
                        }
                    }
                    Err(error) => blocking.push(error),
                },
                Err(error) => blocking.push(error),
            }
        }

        for pinned in &authority.public_overlay_files {
            verify_pinned_target(
                target_root,
                &pinned.path,
                &pinned.target_sha256,
                &mut blocking,
            );
        }
        for pinned in &authority.public_rewrites {
            verify_pinned_target(
                target_root,
                &pinned.path,
                &pinned.target_sha256,
                &mut blocking,
            );
        }
    }
    if let (Ok(expected_files), Ok(tracked_files)) = (&expected_files, &tracked_files) {
        for relative in tracked_files.difference(expected_files) {
            match optional_safe_regular_bytes(target_root, relative) {
                Ok(Some(bytes)) => deletes.push(PublicSourceDelete {
                    path: relative.clone(),
                    target_sha256: ags_platform::sha256(&bytes),
                }),
                Ok(None) => {}
                Err(error) => blocking.push(error),
            }
        }
    }
    if let (Ok(expected_files), Ok(untracked_files)) = (&expected_files, &untracked_files) {
        for relative in untracked_files.difference(expected_files) {
            blocking.push(format!(
                "public target has a non-authority untracked file; preserve or remove it explicitly before projection: {relative}"
            ));
        }
    }

    writes.sort_by(|left, right| left.path.cmp(&right.path));
    deletes.sort_by(|left, right| left.path.cmp(&right.path));
    blocking.sort();
    blocking.dedup();
    let mut plan = PublicSourceProjectionPlan {
        schema_version: "1.0-public-source-projection-plan".into(),
        source_root: source_root.to_path_buf(),
        target_root: target_root.to_path_buf(),
        plan_hash: String::new(),
        writes,
        deletes,
        capability_projection,
        blocking_findings: blocking,
    };
    plan.plan_hash = hash_plan(&plan);
    plan
}

pub fn apply_public_source_projection(
    source_root: &Path,
    target_root: &Path,
    approved_plan_hash: &str,
) -> Result<PublicSourceProjectionReceipt, String> {
    let plan = plan_public_source_projection(source_root, target_root);
    if !plan.blocking_findings.is_empty() {
        return Err(format!(
            "public source projection is blocked: {}",
            plan.blocking_findings.join("; ")
        ));
    }
    if approved_plan_hash != plan.plan_hash {
        return Err("public source projection plan_hash changed; re-plan and approve".into());
    }

    let touched = plan
        .writes
        .iter()
        .map(|write| write.path.clone())
        .chain(plan.deletes.iter().map(|delete| delete.path.clone()))
        .chain(
            plan.capability_projection
                .generated_files
                .iter()
                .map(|file| file.path.clone()),
        )
        .collect::<BTreeSet<_>>();
    let previous = touched
        .iter()
        .map(|relative| {
            optional_safe_regular_bytes(target_root, relative)
                .map(|bytes| (relative.clone(), bytes))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let result = (|| {
        for write in &plan.writes {
            let bytes = safe_regular_bytes(source_root, &write.path)?;
            ags_platform::atomic_write(&target_root.join(&write.path), &bytes)
                .map_err(|error| format!("cannot project {}: {error}", write.path))?;
        }
        let capability_receipt = apply_public_capability_projection(
            source_root,
            target_root,
            &plan.capability_projection.plan_hash,
        )?;
        for delete in &plan.deletes {
            fs::remove_file(target_root.join(&delete.path))
                .map_err(|error| format!("cannot remove retired {}: {error}", delete.path))?;
        }
        let verification = verify_promotion_manifest(source_root, target_root);
        if !verification.passed {
            return Err(format!(
                "post-projection promotion verification failed: missing={:?}; forbidden={:?}; extra={:?}; mismatches={:?}; authority={:?}",
                verification.required_missing,
                verification.forbidden_found,
                verification.extra_files,
                verification.content_mismatches,
                verification.authority_errors
            ));
        }
        Ok(capability_receipt)
    })();

    match result {
        Ok(capability_receipt) => Ok(PublicSourceProjectionReceipt {
            schema_version: "1.0-public-source-projection-receipt".into(),
            plan_hash: plan.plan_hash,
            target_root: target_root.to_path_buf(),
            written_files: plan.writes.into_iter().map(|write| write.path).collect(),
            deleted_files: plan.deletes.into_iter().map(|delete| delete.path).collect(),
            capability_projection: capability_receipt,
            verified: true,
        }),
        Err(error) => {
            let recovery = restore(target_root, previous);
            match recovery {
                Ok(()) => Err(format!("{error}; target recovered")),
                Err(recovery_error) => {
                    Err(format!("{error}; TARGET RECOVERY FAILED: {recovery_error}"))
                }
            }
        }
    }
}

fn verify_pinned_target(target: &Path, relative: &str, expected: &str, errors: &mut Vec<String>) {
    match safe_regular_bytes(target, relative) {
        Ok(bytes) if ags_platform::sha256(&bytes) == expected => {}
        Ok(bytes) => errors.push(format!(
            "B-owned file requires review and a refreshed pin: {relative}: expected {expected}, observed {}",
            ags_platform::sha256(&bytes)
        )),
        Err(error) => errors.push(error),
    }
}

fn hash_plan(plan: &PublicSourceProjectionPlan) -> String {
    let mut value = plan.clone();
    value.plan_hash.clear();
    ags_platform::sha256(serde_json::to_vec(&value).expect("public projection plan serializes"))
}

fn restore(target: &Path, previous: Vec<(String, Option<Vec<u8>>)>) -> Result<(), String> {
    let mut errors = Vec::new();
    for (relative, old) in previous.into_iter().rev() {
        let path = target.join(&relative);
        let result = match old {
            Some(bytes) => ags_platform::atomic_write(&path, &bytes),
            None if path.exists() => fs::remove_file(&path).map_err(|error| error.to_string()),
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(format!("{relative}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn safe_regular_bytes(root: &Path, relative: &str) -> Result<Vec<u8>, String> {
    validate_relative(relative)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "projection path is not a regular file: {}",
            path.display()
        ));
    }
    fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn optional_safe_regular_bytes(root: &Path, relative: &str) -> Result<Option<Vec<u8>>, String> {
    validate_relative(relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "projection target is not a regular file: {}",
            path.display()
        )),
        Ok(_) => fs::read(&path)
            .map(Some)
            .map_err(|error| format!("cannot read {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn validate_relative(relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe public projection path: {relative}"));
    }
    Ok(())
}

fn tracked_files(root: &Path) -> Result<BTreeSet<String>, String> {
    git_files(root, &["ls-files", "-z"])
}

fn untracked_files(root: &Path) -> Result<BTreeSet<String>, String> {
    git_files(root, &["ls-files", "--others", "--exclude-standard", "-z"])
}

fn git_files(root: &Path, args: &[&str]) -> Result<BTreeSet<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("cannot inspect public git index: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot inspect public git index: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).replace('\\', "/"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tracked_retirement_is_already_converged() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            optional_safe_regular_bytes(root.path(), "retired.rs").unwrap(),
            None
        );
    }

    #[test]
    fn present_tracked_retirement_is_hash_bound() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("retired.rs"), b"retired").unwrap();
        let bytes = optional_safe_regular_bytes(root.path(), "retired.rs")
            .unwrap()
            .unwrap();
        assert_eq!(
            ags_platform::sha256(&bytes),
            ags_platform::sha256(b"retired")
        );
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_tracked_retirement_never_counts_as_converged() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("outside.rs"), b"outside").unwrap();
        symlink("outside.rs", root.path().join("retired.rs")).unwrap();
        let error = optional_safe_regular_bytes(root.path(), "retired.rs").unwrap_err();
        assert!(error.contains("not a regular file"), "{error}");
    }
}
