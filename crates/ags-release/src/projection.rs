//! Transactional A→B public projection (v0.4.21).
//!
//! Pattern borrowed from the retired `ags-verification` projector: one plan
//! hash binds writes, deletes and blocking findings; apply is transactional
//! with byte-exact backup/restore; a final verify re-hashes every written
//! file. B-owned overlays and private roots are never touched.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::spec;

pub const PLAN_SCHEMA: &str = "ags://schema/contract/v3/public-projection-plan";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Write {
    pub path: String,
    pub source_sha256: String,
    pub target_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delete {
    pub path: String,
    pub target_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: String,
    pub source_root: PathBuf,
    pub target_root: PathBuf,
    pub plan_hash: String,
    pub writes: Vec<Write>,
    pub deletes: Vec<Delete>,
    pub retired_manifests: Vec<String>,
    pub blocking_findings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: String,
    pub plan_hash: String,
    pub written_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub verified: bool,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn plan_hash_of(plan: &Plan) -> String {
    let mut body = serde_json::to_value(plan).expect("plan serializes");
    body.as_object_mut()
        .expect("plan object")
        .remove("plan_hash");
    sha256_hex(
        serde_json::to_string(&body)
            .expect("canonical plan")
            .as_bytes(),
    )
}

/// Walk a directory recursively; symlinks are rejected (public payload must
/// be plain files/dirs).
fn walk_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| format!("read_dir {}: {e}", root.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        if ft.is_symlink() {
            return Err(format!("symlink in public payload: {}", path.display()));
        }
        if ft.is_dir() {
            walk_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn tracked_files(git_root: &Path) -> Result<BTreeSet<String>, String> {
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(git_root)
        .args(["ls-files"])
        .output()
        .map_err(|e| format!("git ls-files {}: {e}", git_root.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

/// Build the projection plan (read-only).
pub fn build_plan(source_root: &Path, target_root: &Path) -> Plan {
    let mut writes = Vec::new();
    let mut blocking: Vec<String> = Vec::new();

    for dir in spec::PUBLIC_DIRS {
        let src = source_root.join(dir);
        if !src.is_dir() {
            blocking.push(format!("public dir missing in A: {dir}"));
            continue;
        }
        let mut files = Vec::new();
        if let Err(e) = walk_files(&src, &mut files) {
            blocking.push(e);
            continue;
        }
        files.sort();
        for file in files {
            let rel = file
                .strip_prefix(source_root)
                .expect("under source root")
                .to_string_lossy()
                .to_string();
            let bytes = match fs::read(&file) {
                Ok(b) => b,
                Err(e) => {
                    blocking.push(format!("read {}: {e}", rel));
                    continue;
                }
            };
            if let Some(finding) = crate::sensitive::scan(&rel, &bytes) {
                blocking.push(finding);
            }
            let target = target_root.join(&rel);
            let target_sha256 = fs::read(&target).ok().map(|b| sha256_hex(&b));
            writes.push(Write {
                path: rel,
                source_sha256: sha256_hex(&bytes),
                target_sha256,
            });
        }
    }
    for file in spec::PUBLIC_FILES {
        let src = source_root.join(file);
        let Ok(bytes) = fs::read(&src) else {
            blocking.push(format!("public file missing in A: {file}"));
            continue;
        };
        if let Some(finding) = crate::sensitive::scan(file, &bytes) {
            blocking.push(finding);
        }
        let target = target_root.join(file);
        let target_sha256 = fs::read(&target).ok().map(|b| sha256_hex(&b));
        writes.push(Write {
            path: file.to_string(),
            source_sha256: sha256_hex(&bytes),
            target_sha256,
        });
    }
    writes.sort_by(|a, b| a.path.cmp(&b.path));

    // Structural guard: no projected path may fall under a private root or
    // match a private file (belt-and-braces against spec drift).
    for write in &writes {
        if spec::PRIVATE_ROOTS
            .iter()
            .any(|root| write.path.starts_with(root))
            || spec::PRIVATE_FILES.contains(&write.path.as_str())
        {
            blocking.push(format!("private path would be projected: {}", write.path));
        }
    }

    // Retired tracked deletes: everything tracked in B that is neither
    // projected nor B-owned.
    let projected: BTreeSet<String> = writes.iter().map(|w| w.path.clone()).collect();
    let mut deletes = Vec::new();
    match tracked_files(target_root) {
        Ok(tracked) => {
            for path in tracked {
                let is_b_owned = spec::B_OWNED.contains(&path.as_str())
                    || spec::B_OWNED
                        .iter()
                        .any(|b| path.starts_with(&format!("{b}/")));
                if is_b_owned || projected.contains(&path) {
                    continue;
                }
                let target = target_root.join(&path);
                match fs::read(&target) {
                    Ok(bytes) => deletes.push(Delete {
                        path: path.clone(),
                        target_sha256: sha256_hex(&bytes),
                    }),
                    Err(e) => blocking.push(format!("retired {} unreadable: {e}", path)),
                }
            }
        }
        Err(e) => blocking.push(e),
    }
    deletes.sort_by(|a, b| a.path.cmp(&b.path));

    // Contract §5C: the plan must name the retired manifests explicitly.
    let retired_manifests: Vec<String> = deletes
        .iter()
        .map(|d| d.path.as_str())
        .filter(|p| spec::RETIRED_MANIFESTS.contains(p))
        .map(|p| p.to_string())
        .collect();

    let mut plan = Plan {
        schema_version: PLAN_SCHEMA.to_string(),
        source_root: source_root.to_path_buf(),
        target_root: target_root.to_path_buf(),
        plan_hash: String::new(),
        writes,
        deletes,
        retired_manifests,
        blocking_findings: blocking,
    };
    plan.plan_hash = plan_hash_of(&plan);
    plan
}

/// Apply a previously built plan transactionally. On any failure every
/// touched B file is restored from the backup directory.
pub fn apply_plan(plan: &Plan, expected_hash: &str) -> Result<Receipt, String> {
    if plan.plan_hash != expected_hash {
        return Err(format!(
            "plan hash mismatch: plan={} requested={}",
            plan.plan_hash, expected_hash
        ));
    }
    if plan_hash_of(plan) != plan.plan_hash {
        return Err("plan failed integrity recompute".to_string());
    }
    if !plan.blocking_findings.is_empty() {
        return Err(format!(
            "blocking findings prevent apply: {}",
            plan.blocking_findings.join("; ")
        ));
    }
    let backup = plan.target_root.join(".ags-projection-backup");
    let _ = fs::remove_dir_all(&backup);
    fs::create_dir_all(&backup).map_err(|e| e.to_string())?;

    let mut written = Vec::new();
    let mut deleted = Vec::new();
    let result = (|| -> Result<(), String> {
        for write in &plan.writes {
            let target = plan.target_root.join(&write.path);
            let backed = backup.join(&write.path);
            if target.exists() {
                if let Some(parent) = backed.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::rename(&target, &backed).map_err(|e| e.to_string())?;
            }
            let source = plan.source_root.join(&write.path);
            let bytes = fs::read(&source).map_err(|e| format!("read {}: {e}", write.path))?;
            if sha256_hex(&bytes) != write.source_sha256 {
                return Err(format!("source drifted since plan: {}", write.path));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&target, &bytes).map_err(|e| format!("write {}: {e}", write.path))?;
            written.push(write.path.clone());
        }
        for delete in &plan.deletes {
            let target = plan.target_root.join(&delete.path);
            let bytes =
                fs::read(&target).map_err(|e| format!("retired read {}: {e}", delete.path))?;
            if sha256_hex(&bytes) != delete.target_sha256 {
                return Err(format!("retired drifted since plan: {}", delete.path));
            }
            let backed = backup.join(&delete.path);
            if let Some(parent) = backed.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(&target, &backed).map_err(|e| e.to_string())?;
            deleted.push(delete.path.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        // Restore everything touched.
        for path in written.iter().rev() {
            let target = plan.target_root.join(path);
            let backed = backup.join(path);
            let _ = fs::remove_file(&target);
            if backed.exists() {
                let _ = fs::rename(&backed, &target);
            }
        }
        for path in deleted.iter().rev() {
            let backed = backup.join(path);
            if backed.exists() {
                let target = plan.target_root.join(path);
                let _ = fs::rename(&backed, &target);
            }
        }
        return Err(e);
    }
    let _ = fs::remove_dir_all(&backup);

    // Post-apply verify: every written file matches its planned hash.
    let mut verified = true;
    for write in &plan.writes {
        let target = plan.target_root.join(&write.path);
        let Ok(bytes) = fs::read(&target) else {
            verified = false;
            break;
        };
        if sha256_hex(&bytes) != write.source_sha256 {
            verified = false;
            break;
        }
    }
    Ok(Receipt {
        schema_version: PLAN_SCHEMA.to_string(),
        plan_hash: plan.plan_hash.clone(),
        written_files: written,
        deleted_files: deleted,
        verified,
    })
}

/// Promotion verify: S must equal A (exact commit and tree), B written files
/// must match the plan hashes.
pub fn verify_promotion(
    plan: &Plan,
    stable_root: &Path,
    source_root: &Path,
) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    let a_head = git_rev(source_root, "HEAD")?;
    let s_head = git_rev(stable_root, "HEAD")?;
    let a_tree = git_rev(source_root, "HEAD^{tree}")?;
    let s_tree = git_rev(stable_root, "HEAD^{tree}")?;
    if a_head != s_head || a_tree != s_tree {
        errors.push(format!(
            "S does not equal A: A={a_head}/{a_tree} S={s_head}/{s_tree}"
        ));
    }
    for write in &plan.writes {
        let target = plan.target_root.join(&write.path);
        match fs::read(&target) {
            Ok(bytes) if sha256_hex(&bytes) == write.source_sha256 => {}
            Ok(_) => errors.push(format!("B drifted after apply: {}", write.path)),
            Err(e) => errors.push(format!("B missing after apply: {} ({e})", write.path)),
        }
    }
    Ok(errors)
}

fn git_rev(root: &Path, rev: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", rev])
        .output()
        .map_err(|e| format!("git rev-parse {rev} @ {}: {e}", root.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse {rev} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn seed_source(root: &Path) {
        for dir in spec::PUBLIC_DIRS {
            fs::create_dir_all(root.join(dir)).unwrap();
            // one representative file per public dir so the plan is non-empty
            fs::write(root.join(dir).join("placeholder.rs"), "// public\n").unwrap();
        }
        fs::create_dir_all(root.join("docs")).unwrap();
        for file in spec::PUBLIC_FILES {
            if let Some(parent) = root.join(file).parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(root.join(file), format!("content of {file}\n")).unwrap();
        }
        // private material must never appear in the plan
        fs::create_dir_all(root.join("protocol")).unwrap();
        fs::write(root.join("protocol/private.md"), "secret\n").unwrap();
        fs::write(root.join("CLAUDE.md"), "private\n").unwrap();
        // make the source a git repo so promotion verify can rev-parse
        let _ = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["init", "-q"])
            .status();
        let _ = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["add", "-A"])
            .status();
        let _ = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "seed",
            ])
            .status();
    }

    fn seed_target_as_git(root: &Path) {
        fs::create_dir_all(root.join(".github/workflows")).unwrap();
        fs::write(root.join(".github/workflows/ci.yml"), "b-owned\n").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("manifests")).unwrap();
        for m in spec::RETIRED_MANIFESTS {
            fs::write(root.join(m), "legacy\n").unwrap();
        }
        fs::create_dir_all(root.join("crates/old-crate")).unwrap();
        fs::write(root.join("crates/old-crate/lib.rs"), "old\n").unwrap();
        // register them as tracked
        let status = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        let _ = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["add", "-A"])
            .status();
        let _ = std::process::Command::new("git")
            .args(["-C"])
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "seed",
            ])
            .status();
    }

    #[test]
    fn plan_excludes_private_and_includes_public() {
        let (a, b) = fixture();
        seed_source(a.path());
        seed_target_as_git(b.path());
        let plan = build_plan(a.path(), b.path());
        assert!(
            plan.blocking_findings.is_empty(),
            "{:?}",
            plan.blocking_findings
        );
        let paths: Vec<&str> = plan.writes.iter().map(|w| w.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.iter().any(|p| p.starts_with("crates/ags-kernel")));
        assert!(paths.iter().any(|p| p.starts_with("packages")));
        assert!(!paths.iter().any(|p| p.starts_with("protocol/")));
        assert!(!paths.contains(&"CLAUDE.md"));
        assert!(!paths.iter().any(|p| p.starts_with(".github")));
        // retired tracked files: old crate + manifests
        let deleted: Vec<&str> = plan.deletes.iter().map(|d| d.path.as_str()).collect();
        assert!(deleted.iter().any(|p| p.starts_with("crates/old-crate")));
        assert!(deleted.iter().any(|p| p.starts_with("manifests/")));
        assert!(!deleted.iter().any(|p| p.starts_with(".github")));
        assert!(!deleted.contains(&".gitignore"));
    }

    #[test]
    fn apply_is_transactional_and_verified() {
        let (a, b) = fixture();
        seed_source(a.path());
        seed_target_as_git(b.path());
        let plan = build_plan(a.path(), b.path());
        let receipt = apply_plan(&plan, &plan.plan_hash).unwrap();
        assert!(receipt.verified);
        assert!(receipt.written_files.iter().any(|p| p == "Cargo.toml"));
        assert!(receipt
            .deleted_files
            .iter()
            .any(|p| p.starts_with("crates/old-crate")));
        assert!(fs::read_to_string(b.path().join("Cargo.toml"))
            .unwrap()
            .contains("content of Cargo.toml"));
        assert!(!b.path().join("manifests/suite.yaml").exists());
        assert!(
            fs::read_to_string(b.path().join(".github/workflows/ci.yml"))
                .unwrap()
                .contains("b-owned")
        );
    }

    #[test]
    fn wrong_plan_hash_is_rejected() {
        let (a, b) = fixture();
        seed_source(a.path());
        seed_target_as_git(b.path());
        let plan = build_plan(a.path(), b.path());
        let err = apply_plan(&plan, "deadbeef").unwrap_err();
        assert!(err.contains("plan hash mismatch"));
    }

    #[test]
    fn source_drift_rolls_back() {
        let (a, b) = fixture();
        seed_source(a.path());
        seed_target_as_git(b.path());
        let plan = build_plan(a.path(), b.path());
        // drift the source after planning
        fs::write(a.path().join("Cargo.toml"), "drifted\n").unwrap();
        let err = apply_plan(&plan, &plan.plan_hash).unwrap_err();
        assert!(err.contains("source drifted"), "{err}");
        // target untouched
        assert!(!b.path().join("Cargo.toml").exists());
    }

    #[test]
    fn sensitivity_scan_blocks_private_leak() {
        let (a, b) = fixture();
        seed_source(a.path());
        // plant a private absolute path inside a projected file
        let leak = concat!(
            "see /Volumes/",
            "My ",
            "Passport",
            "/AI Project/agent-governance-suite-",
            "private for more\n"
        );
        fs::write(a.path().join("README.md"), leak).unwrap();
        let plan = build_plan(a.path(), b.path());
        assert!(
            plan.blocking_findings
                .iter()
                .any(|f| f.contains("sensitive") && f.contains("README.md")),
            "{:?}",
            plan.blocking_findings
        );
    }

    #[test]
    fn promotion_verify_detects_stable_drift() {
        let (a, b) = fixture();
        seed_source(a.path());
        seed_target_as_git(b.path());
        let plan = build_plan(a.path(), b.path());
        apply_plan(&plan, &plan.plan_hash).unwrap();
        let errors = verify_promotion(&plan, b.path(), a.path()).unwrap();
        // S (here b) is not at A's commit → must report inequality
        assert!(
            errors.iter().any(|e| e.contains("does not equal A")),
            "{errors:?}"
        );
    }
}
