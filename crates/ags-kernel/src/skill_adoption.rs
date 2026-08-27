//! Minimal immutable third-party Skill adoption for contract v3.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::capabilities::{CapabilitiesLock, LockEntry};
use crate::error::{Error, Result};
use crate::sync::SkillRouting;
use crate::workspace::WorkspaceBinding;

const REGISTRY_SCHEMA: &str = "ags://schema/v0.4.21/installed-skills";
const REGISTRY_PATH: &str = ".ags/v3/installed-skills.json";
const BODY_ROOT: &str = ".ags/v3/skill-bodies";
const LOCK_PATH: &str = ".ags/v3/skill-adoption.lock";
const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(test)]
static FAIL_AFTER_MACHINE_SYNC: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillRecord {
    pub skill_id: String,
    pub source: String,
    pub source_sha256: String,
    pub body_path: String,
    pub routing: String,
    pub observed_license: Option<String>,
    #[serde(default = "default_update_policy")]
    pub update_policy: String,
    #[serde(default)]
    pub previous_revisions: Vec<String>,
}

fn default_update_policy() -> String {
    "manual".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillRegistry {
    pub schema_version: String,
    pub revision: u64,
    pub skills: BTreeMap<String, InstalledSkillRecord>,
}

impl Default for InstalledSkillRegistry {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA.to_string(),
            revision: 0,
            skills: BTreeMap::new(),
        }
    }
}

pub fn prepare_install(
    binding: &WorkspaceBinding,
    id: &str,
    path: &str,
    acknowledged_risks: &[String],
) -> Result<Value> {
    validate_skill_id(id)?;
    let source = resolve_source(binding, path)?;
    let routing = crate::sync::validate_skill_source(id, &source)?;
    let audit = audit_source(&source)?;
    let source_sha256 = crate::capabilities::dir_sha256(&source)?;
    let mut required = Vec::new();
    if audit.observed_license.is_none() {
        required.push("missing-license".to_string());
    }
    required.extend(audit.risk_findings.clone());
    required.sort();
    required.dedup();
    let missing: Vec<String> = required
        .iter()
        .filter(|risk| !acknowledged_risks.iter().any(|ack| ack == *risk))
        .cloned()
        .collect();
    Ok(json!({
        "skill_id": id,
        "path": path,
        "source_sha256": source_sha256,
        "routing": routing.label(),
        "observed_license": audit.observed_license,
        "file_count": audit.file_count,
        "total_bytes": audit.total_bytes,
        "risk_findings": audit.risk_findings,
        "required_acknowledgements": required,
        "acknowledged_risks": acknowledged_risks,
        "ready": missing.is_empty(),
        "missing_acknowledgements": missing,
    }))
}

pub fn apply_install(
    binding: &WorkspaceBinding,
    payload: &Value,
) -> Result<(SkillRouting, String)> {
    let id = string_field(payload, "skill_id")?;
    validate_skill_id(id)?;
    let path = string_field(payload, "path")?;
    let expected = string_field(payload, "source_sha256")?;
    let ready = payload.get("ready").and_then(Value::as_bool) == Some(true);
    if !ready {
        return Err(Error::new(
            "skill_risk_acknowledgement_required",
            format!(
                "missing acknowledgements: {}",
                payload
                    .get("missing_acknowledgements")
                    .cloned()
                    .unwrap_or_else(|| json!([]))
            ),
        ));
    }
    let source = resolve_source(binding, path)?;
    let routing = crate::sync::validate_skill_source(id, &source)?;
    let actual = crate::capabilities::dir_sha256(&source)?;
    if actual != expected {
        return Err(Error::new(
            "skill_source_drift",
            format!("planned {expected}, current {actual}"),
        ));
    }
    let audit = audit_source(&source)?;
    let acknowledgements: Vec<&str> = payload
        .get("acknowledged_risks")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut actual_required = audit.risk_findings.clone();
    if audit.observed_license.is_none() {
        actual_required.push("missing-license".to_string());
    }
    let missing: Vec<String> = actual_required
        .into_iter()
        .filter(|risk| !acknowledgements.iter().any(|ack| *ack == risk))
        .collect();
    if !missing.is_empty() {
        return Err(Error::new(
            "skill_risk_acknowledgement_required",
            format!("missing acknowledgements: {}", missing.join(", ")),
        ));
    }
    let home = home()?;
    let body_rel = PathBuf::from(BODY_ROOT).join(id).join(expected);
    crate::projection::reject_symlink_path(&home, &body_rel)?;
    let link_rel = PathBuf::from(".agents/skills").join(id);
    crate::projection::reject_symlink_components(&home, &link_rel)?;
    crate::projection::reject_symlink_path(
        &binding.root,
        Path::new(".ags")
            .join(crate::capabilities::LOCK_FILE)
            .as_path(),
    )?;
    let _lock = AdoptionLock::acquire()?;
    recover_pending(binding)?;
    let body = home.join(&body_rel);
    let skills = crate::sync::skills_dir()?;
    let link = skills.join(id);
    let registry_path = home.join(REGISTRY_PATH);
    let previous_registry = read_optional(&registry_path, "skill_registry_snapshot_failed")?;
    let machine_lock_path = home.join(crate::sync::MACHINE_LOCK_PATH);
    let previous_machine_lock = read_optional(&machine_lock_path, "machine_lock_snapshot_failed")?;
    let project_lock_path = binding.ags_dir.join(crate::capabilities::LOCK_FILE);
    let previous_project_lock = read_optional(&project_lock_path, "project_lock_snapshot_failed")?;
    let previous_link = read_link_state(&link)?;
    let body_preexisting = body.is_dir();
    let journal = json!({
        "registry": previous_registry.as_ref().map(|bytes| hex_bytes(bytes)),
        "machine_lock": previous_machine_lock.as_ref().map(|bytes| hex_bytes(bytes)),
        "project_lock": previous_project_lock.as_ref().map(|bytes| hex_bytes(bytes)),
        "project_lock_path": project_lock_path,
        "link": previous_link,
        "body": body,
        "body_preexisting": body_preexisting,
    });
    write_atomic(
        &home.join(".ags/v3/skill-adoption-journal.json"),
        &serde_json::to_vec_pretty(&journal).unwrap(),
    )?;

    let result = (|| -> Result<(SkillRouting, String)> {
        if !body_preexisting {
            copy_body(&source, &body)?;
            let copied = crate::capabilities::dir_sha256(&body)?;
            if copied != expected {
                return Err(Error::new(
                    "skill_body_copy_drift",
                    format!("expected {expected}, copied {copied}"),
                ));
            }
        } else {
            let existing = crate::capabilities::dir_sha256(&body)?;
            if existing != expected {
                return Err(Error::new(
                    "skill_immutable_body_drift",
                    format!("expected {expected}, existing {existing}"),
                ));
            }
        }
        fs::create_dir_all(&skills).map_err(|e| crate::error::io("skills_dir_failed", &e))?;
        replace_link(&link, &body)?;

        let mut registry = load_registry()?;
        let previous = registry.skills.get(id).cloned();
        let mut revisions = previous
            .as_ref()
            .map(|record| record.previous_revisions.clone())
            .unwrap_or_default();
        if let Some(previous) = previous {
            if previous.source_sha256 != expected
                && !revisions
                    .iter()
                    .any(|revision| revision == &previous.source_sha256)
            {
                revisions.push(previous.source_sha256);
            }
        }
        registry.revision += 1;
        registry.skills.insert(
            id.to_string(),
            InstalledSkillRecord {
                skill_id: id.to_string(),
                source: source.display().to_string(),
                source_sha256: expected.to_string(),
                body_path: body.display().to_string(),
                routing: routing.label().to_string(),
                observed_license: audit.observed_license,
                update_policy: "manual".to_string(),
                previous_revisions: revisions,
            },
        );
        save_registry(&registry)?;
        crate::sync::sync_bodies()?;
        fail_after_machine_sync()?;
        write_project_audit(binding, id, path, expected)?;
        harden_tree(&body)?;
        Ok((routing, body.display().to_string()))
    })();

    if result.is_err() {
        rollback(RollbackState {
            registry: previous_registry.as_deref(),
            machine_lock: previous_machine_lock.as_deref(),
            machine_lock_path: &machine_lock_path,
            project_lock: previous_project_lock.as_deref(),
            project_lock_path: &project_lock_path,
            link: &link,
            previous_link: previous_link.as_deref(),
            body: &body,
            body_preexisting,
        })?;
    }
    let _ = fs::remove_file(home.join(".ags/v3/skill-adoption-journal.json"));
    result
}

pub fn remove(binding: &WorkspaceBinding, id: &str) -> Result<bool> {
    validate_skill_id(id)?;
    let home = home()?;
    crate::projection::reject_symlink_path(&home, Path::new(".ags/v3/installed-skills.json"))?;
    let link_rel = PathBuf::from(".agents/skills").join(id);
    crate::projection::reject_symlink_components(&home, &link_rel)?;
    crate::projection::reject_symlink_path(
        &binding.root,
        Path::new(".ags")
            .join(crate::capabilities::LOCK_FILE)
            .as_path(),
    )?;
    let _lock = AdoptionLock::acquire()?;
    recover_pending(binding)?;
    let mut registry = load_registry()?;
    let Some(record) = registry.skills.remove(id) else {
        return Ok(false);
    };
    let registry_path = home.join(REGISTRY_PATH);
    let previous_registry = read_optional(&registry_path, "skill_registry_snapshot_failed")?;
    let machine_lock_path = home.join(crate::sync::MACHINE_LOCK_PATH);
    let previous_machine_lock = read_optional(&machine_lock_path, "machine_lock_snapshot_failed")?;
    let project_lock_path = binding.ags_dir.join(crate::capabilities::LOCK_FILE);
    let previous_project_lock = read_optional(&project_lock_path, "project_lock_snapshot_failed")?;
    let link = crate::sync::skills_dir()?.join(id);
    let previous_link = read_link_state(&link)?;
    let body = PathBuf::from(&record.body_path);
    validate_installed_body_path(&home, id, &body)?;
    let journal = json!({
        "registry": previous_registry.as_ref().map(|bytes| hex_bytes(bytes)),
        "machine_lock": previous_machine_lock.as_ref().map(|bytes| hex_bytes(bytes)),
        "project_lock": previous_project_lock.as_ref().map(|bytes| hex_bytes(bytes)),
        "project_lock_path": project_lock_path,
        "link": previous_link,
        "body": body,
        "body_preexisting": true,
    });
    let journal_path = home.join(".ags/v3/skill-adoption-journal.json");
    write_atomic(
        &journal_path,
        &serde_json::to_vec_pretty(&journal)
            .map_err(|error| Error::new("skill_registry_encode_failed", error.to_string()))?,
    )?;
    let result = (|| -> Result<()> {
        if let Ok(target) = fs::canonicalize(&link) {
            let recorded = fs::canonicalize(&record.body_path)
                .unwrap_or_else(|_| PathBuf::from(&record.body_path));
            if target == recorded {
                fs::remove_file(&link)
                    .map_err(|error| crate::error::io("skill_remove_failed", &error))?;
            }
        }
        registry.revision += 1;
        save_registry(&registry)?;
        crate::sync::sync_bodies()?;
        fail_after_machine_sync()?;
        write_project_audit_remove(binding, id)?;
        Ok(())
    })();
    if result.is_err() {
        rollback(RollbackState {
            registry: previous_registry.as_deref(),
            machine_lock: previous_machine_lock.as_deref(),
            machine_lock_path: &machine_lock_path,
            project_lock: previous_project_lock.as_deref(),
            project_lock_path: &project_lock_path,
            link: &link,
            previous_link: previous_link.as_deref(),
            body: &body,
            body_preexisting: true,
        })?;
    }
    let _ = fs::remove_file(journal_path);
    result.map(|_| true)
}

pub fn load_registry() -> Result<InstalledSkillRegistry> {
    let path = home()?.join(REGISTRY_PATH);
    if !path.is_file() {
        return Ok(InstalledSkillRegistry::default());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| crate::error::io("skill_registry_read_failed", &e))?;
    let registry: InstalledSkillRegistry = serde_json::from_str(&text)
        .map_err(|e| Error::new("skill_registry_parse_failed", e.to_string()))?;
    if registry.schema_version != REGISTRY_SCHEMA {
        return Err(Error::new(
            "skill_registry_schema_invalid",
            registry.schema_version,
        ));
    }
    Ok(registry)
}

fn save_registry(registry: &InstalledSkillRegistry) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|e| Error::new("skill_registry_encode_failed", e.to_string()))?;
    write_atomic(&home()?.join(REGISTRY_PATH), &bytes)
}

fn write_project_audit(
    binding: &WorkspaceBinding,
    id: &str,
    source_path: &str,
    sha256: &str,
) -> Result<()> {
    let mut lock = CapabilitiesLock::load(binding)?;
    lock.entries.retain(|entry| entry.id != id);
    lock.entries.push(LockEntry {
        id: id.to_string(),
        kind: "skill".to_string(),
        path: source_path.to_string(),
        sha256: sha256.to_string(),
        hosts: vec![],
    });
    lock.entries.sort_by(|left, right| left.id.cmp(&right.id));
    let bytes = serde_json::to_vec_pretty(&lock)
        .map_err(|error| Error::new("capabilities_lock_encode_failed", error.to_string()))?;
    write_atomic(
        &binding.ags_dir.join(crate::capabilities::LOCK_FILE),
        &bytes,
    )
}

fn write_project_audit_remove(binding: &WorkspaceBinding, id: &str) -> Result<()> {
    let mut lock = CapabilitiesLock::load(binding)?;
    lock.entries.retain(|entry| entry.id != id);
    let bytes = serde_json::to_vec_pretty(&lock)
        .map_err(|error| Error::new("capabilities_lock_encode_failed", error.to_string()))?;
    write_atomic(
        &binding.ags_dir.join(crate::capabilities::LOCK_FILE),
        &bytes,
    )
}

fn resolve_source(binding: &WorkspaceBinding, path: &str) -> Result<PathBuf> {
    let rel = Path::new(path);
    if rel.is_absolute() {
        return Err(Error::new(
            "skill_install_path_outside_workspace",
            "install path must be workspace-relative",
        ));
    }
    if rel
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(Error::new(
            "skill_install_path_outside_workspace",
            "install path must not escape the workspace",
        ));
    }
    crate::projection::reject_symlink_path(&binding.root, rel)?;
    let root = binding
        .root
        .canonicalize()
        .map_err(|e| crate::error::io("workspace_resolve_failed", &e))?;
    let source = binding.root.join(rel).canonicalize().map_err(|e| {
        Error::new(
            "skill_source_missing",
            format!("cannot resolve {}: {e}", binding.root.join(rel).display()),
        )
    })?;
    if !source.starts_with(&root) {
        return Err(Error::new(
            "skill_install_path_outside_workspace",
            format!("{} escapes the workspace", source.display()),
        ));
    }
    Ok(source)
}

fn validate_installed_body_path(home: &Path, id: &str, body: &Path) -> Result<()> {
    let body_root = home.join(BODY_ROOT);
    if !body.starts_with(&body_root)
        || body.parent().and_then(Path::parent) != Some(body_root.as_path())
        || body
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            != Some(id)
    {
        return Err(Error::new(
            "skill_registry_body_invalid",
            format!("{} is outside the immutable body store", body.display()),
        ));
    }
    let relative = body
        .strip_prefix(home)
        .map_err(|_| Error::new("skill_registry_body_invalid", body.display().to_string()))?;
    crate::projection::reject_symlink_path(home, relative)
}

struct Audit {
    observed_license: Option<String>,
    file_count: usize,
    total_bytes: u64,
    risk_findings: Vec<String>,
}

fn audit_source(source: &Path) -> Result<Audit> {
    fn walk(
        root: &Path,
        dir: &Path,
        count: &mut usize,
        bytes: &mut u64,
        risks: &mut Vec<String>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir).map_err(|e| crate::error::io("skill_audit_failed", &e))? {
            let entry = entry.map_err(|e| crate::error::io("skill_audit_failed", &e))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| crate::error::io("skill_audit_failed", &e))?;
            if metadata.file_type().is_symlink() {
                return Err(Error::new(
                    "skill_symlink_refused",
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                ));
            }
            if metadata.is_dir() {
                walk(root, &path, count, bytes, risks)?;
            } else if metadata.is_file() {
                *count += 1;
                *bytes += metadata.len();
                if *count > MAX_FILES || *bytes > MAX_BYTES {
                    return Err(Error::new(
                        "skill_size_limit_exceeded",
                        format!("files={count}, bytes={bytes}"),
                    ));
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_ascii_lowercase();
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if name == ".env"
                    || name == "auth.json"
                    || name.starts_with("id_rsa")
                    || name.starts_with("id_ed25519")
                    || relative.contains("credential")
                    || relative.contains("secret")
                {
                    risks.push(format!("sensitive-file@{relative}"));
                }
                if matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("sh" | "py" | "js" | "ts")
                ) {
                    risks.push("executable-content".to_string());
                }
            } else {
                return Err(Error::new(
                    "skill_special_file_refused",
                    path.display().to_string(),
                ));
            }
        }
        Ok(())
    }
    let mut file_count = 0;
    let mut total_bytes = 0;
    let mut risk_findings = Vec::new();
    walk(
        source,
        source,
        &mut file_count,
        &mut total_bytes,
        &mut risk_findings,
    )?;
    let observed_license = fs::read_dir(source)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .find(|name| name.to_ascii_lowercase().starts_with("license"));
    Ok(Audit {
        observed_license,
        file_count,
        total_bytes,
        risk_findings,
    })
}

fn copy_body(source: &Path, target: &Path) -> Result<()> {
    let staging = target.with_extension(format!("staging-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| crate::error::io("skill_staging_cleanup_failed", &e))?;
    }
    copy_dir(source, &staging)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io("skill_body_dir_failed", &e))?;
    }
    fs::rename(&staging, target).map_err(|e| crate::error::io("skill_body_write_failed", &e))
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).map_err(|e| crate::error::io("skill_body_write_failed", &e))?;
    for entry in fs::read_dir(source).map_err(|e| crate::error::io("skill_body_read_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("skill_body_read_failed", &e))?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from)
            .map_err(|e| crate::error::io("skill_body_read_failed", &e))?;
        if metadata.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| crate::error::io("skill_body_write_failed", &e))?;
        }
    }
    Ok(())
}

fn replace_link(link: &Path, body: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(link) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            return Err(Error::new(
                "skill_body_not_managed_symlink",
                format!("{} is a real directory", link.display()),
            ));
        }
        fs::remove_file(link).map_err(|e| crate::error::io("skill_link_replace_failed", &e))?;
    }
    std::os::unix::fs::symlink(body, link)
        .map_err(|e| crate::error::io("skill_link_write_failed", &e))
}

fn read_link_state(link: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(fs::read_link(link)
            .ok()
            .map(|path| path.display().to_string())),
        Ok(metadata) if metadata.is_dir() => Err(Error::new(
            "skill_body_not_managed_symlink",
            format!("{} is a real directory", link.display()),
        )),
        Ok(_) => Err(Error::new(
            "skill_body_conflict",
            format!("{} is not a symlink", link.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::error::io("skill_link_read_failed", &error)),
    }
}

struct RollbackState<'a> {
    registry: Option<&'a [u8]>,
    machine_lock: Option<&'a [u8]>,
    machine_lock_path: &'a Path,
    project_lock: Option<&'a [u8]>,
    project_lock_path: &'a Path,
    link: &'a Path,
    previous_link: Option<&'a str>,
    body: &'a Path,
    body_preexisting: bool,
}

fn rollback(state: RollbackState<'_>) -> Result<()> {
    let registry_path = home()?.join(REGISTRY_PATH);
    match state.registry {
        Some(bytes) => write_atomic(&registry_path, bytes)?,
        None => {
            let _ = fs::remove_file(&registry_path);
        }
    }
    match state.machine_lock {
        Some(bytes) => write_atomic(state.machine_lock_path, bytes)?,
        None => {
            let _ = fs::remove_file(state.machine_lock_path);
        }
    }
    match state.project_lock {
        Some(bytes) => write_atomic(state.project_lock_path, bytes)?,
        None => {
            let _ = fs::remove_file(state.project_lock_path);
        }
    }
    if fs::symlink_metadata(state.link).is_ok() {
        let _ = fs::remove_file(state.link);
    }
    if let Some(previous) = state.previous_link {
        std::os::unix::fs::symlink(previous, state.link)
            .map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
    }
    if !state.body_preexisting && state.body.is_dir() {
        make_tree_writable(state.body)?;
        fs::remove_dir_all(state.body)
            .map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
    }
    Ok(())
}

fn recover_pending(binding: &WorkspaceBinding) -> Result<()> {
    let path = home()?.join(".ags/v3/skill-adoption-journal.json");
    if !path.is_file() {
        return Ok(());
    }
    let text =
        fs::read_to_string(&path).map_err(|e| crate::error::io("skill_recovery_failed", &e))?;
    let journal: Value = serde_json::from_str(&text)
        .map_err(|e| Error::new("skill_recovery_failed", e.to_string()))?;
    let registry = journal
        .get("registry")
        .and_then(Value::as_str)
        .map(decode_hex)
        .transpose()?;
    let machine_lock = journal
        .get("machine_lock")
        .and_then(Value::as_str)
        .map(decode_hex)
        .transpose()?;
    let machine_lock_path = home()?.join(crate::sync::MACHINE_LOCK_PATH);
    let project_lock = journal
        .get("project_lock")
        .and_then(Value::as_str)
        .map(decode_hex)
        .transpose()?;
    let project_lock_path = journal
        .get("project_lock_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("skill_recovery_failed", "journal missing project lock"))?;
    let expected_project_lock = binding.ags_dir.join(crate::capabilities::LOCK_FILE);
    if project_lock_path != expected_project_lock {
        return Err(Error::new(
            "skill_recovery_failed",
            "journal project lock crosses workspace binding",
        ));
    }
    let previous_link = journal.get("link").and_then(Value::as_str);
    let body = journal
        .get("body")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("skill_recovery_failed", "journal missing body"))?;
    let body_root = home()?.join(BODY_ROOT);
    if !body.starts_with(&body_root)
        || body
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        || body.parent().and_then(Path::parent) != Some(body_root.as_path())
    {
        return Err(Error::new(
            "skill_recovery_failed",
            format!("journal body escapes {}", body_root.display()),
        ));
    }
    let body_preexisting = journal
        .get("body_preexisting")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let id = body
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| Error::new("skill_recovery_failed", "journal body has no skill id"))?;
    let link = crate::sync::skills_dir()?.join(id);
    rollback(RollbackState {
        registry: registry.as_deref(),
        machine_lock: machine_lock.as_deref(),
        machine_lock_path: &machine_lock_path,
        project_lock: project_lock.as_deref(),
        project_lock_path: &project_lock_path,
        link: &link,
        previous_link,
        body: &body,
        body_preexisting,
    })?;
    fs::remove_file(path).map_err(|e| crate::error::io("skill_recovery_failed", &e))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io("skill_store_failed", &e))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| crate::error::io("skill_store_failed", &e))?;
    fs::rename(&tmp, path).map_err(|e| crate::error::io("skill_store_failed", &e))
}

fn read_optional(path: &Path, code: &'static str) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::error::io(code, &error)),
    }
}

fn harden_tree(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| crate::error::io("skill_body_harden_failed", &e))?;
    if metadata.is_dir() {
        for entry in
            fs::read_dir(path).map_err(|e| crate::error::io("skill_body_harden_failed", &e))?
        {
            let entry = entry.map_err(|e| crate::error::io("skill_body_harden_failed", &e))?;
            harden_tree(&entry.path())?;
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o555))
            .map_err(|e| crate::error::io("skill_body_harden_failed", &e))?;
    } else {
        let executable = metadata.permissions().mode() & 0o111 != 0;
        let mode = if executable { 0o555 } else { 0o444 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| crate::error::io("skill_body_harden_failed", &e))?;
    }
    Ok(())
}

fn make_tree_writable(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
    if metadata.is_dir() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
        for entry in
            fs::read_dir(path).map_err(|e| crate::error::io("skill_rollback_failed", &e))?
        {
            let entry = entry.map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
            make_tree_writable(&entry.path())?;
        }
    } else {
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))
            .map_err(|e| crate::error::io("skill_rollback_failed", &e))?;
    }
    Ok(())
}

fn home() -> Result<PathBuf> {
    crate::sync::machine_home()
}

fn fail_after_machine_sync() -> Result<()> {
    #[cfg(test)]
    if FAIL_AFTER_MACHINE_SYNC.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::new(
            "skill_test_failpoint",
            "injected failure after machine lock refresh",
        ));
    }
    Ok(())
}

fn string_field<'a>(payload: &'a Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("skill_install_payload_invalid", format!("missing {key}")))
}

pub fn validate_skill_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(Error::new(
            "skill_id_invalid",
            "skill id must be one safe ASCII path component",
        ));
    }
    Ok(())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(Error::new("skill_recovery_failed", "invalid registry hex"));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|error| Error::new("skill_recovery_failed", error.to_string()))
        })
        .collect()
}

struct AdoptionLock {
    path: PathBuf,
}

impl AdoptionLock {
    fn acquire() -> Result<Self> {
        let path = home()?.join(LOCK_PATH);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| crate::error::io("skill_lock_failed", &e))?;
        }
        if path.is_file() {
            let live = fs::read_to_string(&path)
                .ok()
                .and_then(|value| value.trim().parse::<i32>().ok())
                .map(process_alive)
                .unwrap_or(false);
            if live {
                return Err(Error::new(
                    "skill_lock_busy",
                    format!("active adoption lock at {}", path.display()),
                ));
            }
            fs::remove_file(&path).map_err(|e| crate::error::io("skill_lock_failed", &e))?;
        }
        use std::io::Write;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::new("skill_lock_busy", e.to_string()))?;
        writeln!(file, "{}", std::process::id())
            .map_err(|e| crate::error::io("skill_lock_failed", &e))?;
        Ok(Self { path })
    }
}

fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Signal 0 performs existence/permission probing without delivering a signal.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

impl Drop for AdoptionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(root: &Path) -> WorkspaceBinding {
        let ags_dir = root.join(".ags");
        WorkspaceBinding {
            root: root.to_path_buf(),
            slug: "demo".to_string(),
            role: "A".to_string(),
            evidence_dir: ags_dir.join("evidence"),
            state_dir: ags_dir.join("state"),
            ags_dir,
        }
    }

    #[test]
    fn adoption_rejects_drift_then_installs_immutable_body() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let root = tmp.path().join("workspace");
        let source = root.join("skill-packs/demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo task\n---\n# Demo\n",
        )
        .unwrap();
        fs::write(source.join("LICENSE"), "MIT\n").unwrap();
        let binding = binding(&root);
        let planned = prepare_install(&binding, "demo", "skill-packs/demo", &[]).unwrap();
        assert_eq!(planned["ready"], true);
        fs::write(source.join("extra.txt"), "drift\n").unwrap();
        let error = apply_install(&binding, &planned).unwrap_err();
        assert_eq!(error.code, "skill_source_drift");

        let planned = prepare_install(&binding, "demo", "skill-packs/demo", &[]).unwrap();
        let (_, body) = apply_install(&binding, &planned).unwrap();
        let link = crate::sync::skills_dir().unwrap().join("demo");
        assert_eq!(
            fs::canonicalize(link).unwrap(),
            fs::canonicalize(&body).unwrap()
        );
        assert_ne!(
            fs::canonicalize(&body).unwrap(),
            fs::canonicalize(&source).unwrap()
        );
        assert!(fs::write(PathBuf::from(&body).join("SKILL.md"), "mutate").is_err());
        assert!(load_registry().unwrap().skills.contains_key("demo"));
        let first_hash = load_registry().unwrap().skills["demo"]
            .source_sha256
            .clone();
        fs::write(source.join("revision.txt"), "second\n").unwrap();
        let update = prepare_install(&binding, "demo", "skill-packs/demo", &[]).unwrap();
        apply_install(&binding, &update).unwrap();
        let updated = load_registry().unwrap().skills["demo"].clone();
        assert_ne!(updated.source_sha256, first_hash);
        assert!(updated.previous_revisions.contains(&first_hash));
        let machine_before =
            fs::read(home().unwrap().join(crate::sync::MACHINE_LOCK_PATH)).unwrap();
        FAIL_AFTER_MACHINE_SYNC.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = remove(&binding, "demo").unwrap_err();
        assert_eq!(error.code, "skill_test_failpoint");
        assert!(crate::sync::skills_dir().unwrap().join("demo").exists());
        assert!(load_registry().unwrap().skills.contains_key("demo"));
        assert_eq!(
            fs::read(home().unwrap().join(crate::sync::MACHINE_LOCK_PATH)).unwrap(),
            machine_before
        );
        assert!(remove(&binding, "demo").unwrap());
    }

    #[test]
    fn stale_process_lock_is_recovered() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let path = tmp.path().join(LOCK_PATH);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "2147483647\n").unwrap();
        let lock = AdoptionLock::acquire().unwrap();
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn skill_id_cannot_escape_machine_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding(tmp.path());
        let error = prepare_install(&binding, "../evil", "skill-packs/demo", &[]).unwrap_err();
        assert_eq!(error.code, "skill_id_invalid");
        assert!(remove(&binding, "../evil").is_err());
    }

    #[test]
    fn install_failure_restores_registry_links_and_locks() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let root = tmp.path().join("workspace");
        let source = root.join("skill-packs/demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo\n---\n",
        )
        .unwrap();
        fs::write(source.join("LICENSE"), "MIT\n").unwrap();
        let binding = binding(&root);
        let plan = prepare_install(&binding, "demo", "skill-packs/demo", &[]).unwrap();
        FAIL_AFTER_MACHINE_SYNC.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = apply_install(&binding, &plan).unwrap_err();
        assert_eq!(error.code, "skill_test_failpoint");
        assert!(!crate::sync::skills_dir().unwrap().join("demo").exists());
        assert!(!home().unwrap().join(REGISTRY_PATH).exists());
        assert!(!home()
            .unwrap()
            .join(crate::sync::MACHINE_LOCK_PATH)
            .exists());
        assert!(!binding
            .ags_dir
            .join(crate::capabilities::LOCK_FILE)
            .exists());
    }

    #[test]
    fn machine_store_parent_symlink_is_rejected() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::os::unix::fs::symlink(outside.path(), tmp.path().join(".ags")).unwrap();
        let root = tmp.path().join("workspace");
        let source = root.join("skill-packs/demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo\n---\n",
        )
        .unwrap();
        fs::write(source.join("LICENSE"), "MIT\n").unwrap();
        let binding = binding(&root);
        let plan = prepare_install(&binding, "demo", "skill-packs/demo", &[]).unwrap();
        let error = apply_install(&binding, &plan).unwrap_err();
        assert_eq!(error.code, "projection_symlink_rejected");
        assert!(!outside.path().join("v3/skill-bodies/demo").exists());
    }
}
