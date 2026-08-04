use super::model::{
    AdoptionContext, AdoptionPlan, AdoptionReceipt, PrivateSkillRegistry, SnapshotDiscovery,
    ADOPTION_PLAN_SCHEMA, ADOPTION_RECEIPT_SCHEMA,
};
use super::projection::{host_index_path, index_points_to};
use super::source::audit_local_source;
use super::store::{body_path, load_registry, registry_path, write_registry};
use crate::{
    build_capability_snapshot_with_live_roots, build_capability_snapshot_with_roots,
    hash_skill_source, sha256, snapshot_path, write_private_atomic,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

type FileBackup = (PathBuf, Option<Vec<u8>>);

pub fn plan_adoption(
    context: &AdoptionContext,
    source: &Path,
    routing_metadata: Option<&Path>,
    requested_hosts: &[String],
) -> Result<AdoptionPlan, String> {
    let mut target_hosts = normalize_hosts(requested_hosts)?;
    let existing_registry = load_registry(&context.runtime_home)?;
    let audited = audit_local_source(source, target_hosts.clone(), routing_metadata)?;
    reject_official_collision(&context.authority_root, &audited.record.skill_id)?;
    if let Some(existing) = existing_registry.skills.get(&audited.record.skill_id) {
        target_hosts.extend(existing.target_hosts.iter().cloned());
        target_hosts.sort();
        target_hosts.dedup();
    }
    let mut record = audited.record;
    record.target_hosts = target_hosts.clone();
    let body = body_path(&context.runtime_home, &record);
    let indexes = target_hosts
        .iter()
        .map(|host| {
            host_index_path(&context.host_home, host, &record.skill_id)
                .ok_or_else(|| format!("unsupported skill host: {host}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for index in &indexes {
        if let Ok(metadata) = fs::symlink_metadata(index) {
            if !metadata.file_type().is_symlink() {
                return Err(format!(
                    "host index conflict is not a symlink and will not be replaced: {}",
                    index.display()
                ));
            }
        }
    }
    let mut plan = AdoptionPlan {
        schema_version: ADOPTION_PLAN_SCHEMA.to_string(),
        operation: "adopt".to_string(),
        plan_hash: String::new(),
        skill_id: record.skill_id.clone(),
        source: record.source.clone(),
        source_hash: record.source_hash.clone(),
        license_path: record.license_path.clone(),
        license_hash: record.license_hash.clone(),
        routing_metadata_path: record.routing_metadata_path.clone(),
        routing_metadata_hash: record.routing_metadata_hash.clone(),
        body_path: body.to_string_lossy().into_owned(),
        registry_path: registry_path(&context.runtime_home)
            .to_string_lossy()
            .into_owned(),
        target_hosts,
        host_indexes: indexes
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        planned_writes: vec![
            format!("install immutable body: {}", body.display()),
            "replace exact host thin indexes".to_string(),
            format!(
                "update private registry: {}",
                registry_path(&context.runtime_home).display()
            ),
            "refresh selected host capability snapshots".to_string(),
        ],
        warnings: audited.warnings,
    };
    plan.plan_hash = plan_hash(&plan)?;
    Ok(plan)
}

pub fn apply_adoption(
    context: &AdoptionContext,
    source: &Path,
    routing_metadata: Option<&Path>,
    requested_hosts: &[String],
    expected_plan_hash: &str,
) -> Result<AdoptionReceipt, String> {
    let plan = plan_adoption(context, source, routing_metadata, requested_hosts)?;
    if plan.plan_hash != expected_plan_hash {
        return Err(format!(
            "adoption_plan_changed: reviewed {}, current {}",
            expected_plan_hash, plan.plan_hash
        ));
    }
    let audited = audit_local_source(
        Path::new(&plan.source),
        plan.target_hosts.clone(),
        plan.routing_metadata_path.as_deref().map(Path::new),
    )?;
    if audited.record.source_hash != plan.source_hash
        || audited.record.license_hash != plan.license_hash
        || audited.record.routing_metadata_hash != plan.routing_metadata_hash
        || audited.record.skill_id != plan.skill_id
    {
        return Err("adoption_source_changed_after_plan".to_string());
    }
    let mut record = audited.record;
    record.target_hosts = plan.target_hosts.clone();
    let body = body_path(&context.runtime_home, &record);
    let registry_file = registry_path(&context.runtime_home);
    let registry_backup = read_optional(&registry_file)?;
    let link_backups = capture_links(&plan.host_indexes)?;
    let snapshot_backups = capture_snapshots(&context.runtime_home, &plan.target_hosts)?;
    let body_created = install_body(&audited.source_dir, &body, &record.source_hash)?;

    let applied = (|| -> Result<(PrivateSkillRegistry, BTreeMap<String, String>), String> {
        for index in &plan.host_indexes {
            replace_symlink(Path::new(index), &body)?;
        }
        let mut registry = load_registry(&context.runtime_home)?;
        registry.revision = registry.revision.saturating_add(1);
        registry
            .skills
            .insert(record.skill_id.clone(), record.clone());
        write_registry(&context.runtime_home, &registry)?;
        let snapshots = refresh_snapshots(context, &plan.target_hosts)?;
        Ok((registry, snapshots))
    })();

    match applied {
        Ok((registry, snapshots)) => Ok(AdoptionReceipt {
            schema_version: ADOPTION_RECEIPT_SCHEMA.to_string(),
            operation: "adopt".to_string(),
            plan_hash: plan.plan_hash,
            skill_id: record.skill_id,
            registry_revision: registry.revision,
            body_path: body.to_string_lossy().into_owned(),
            host_indexes: plan.host_indexes,
            snapshot_hashes: snapshots,
            requires_repreflight: true,
        }),
        Err(error) => {
            let rollback = rollback(
                &registry_file,
                registry_backup.as_deref(),
                &link_backups,
                &snapshot_backups,
                body_created.then_some(body.as_path()),
            );
            Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
            })
        }
    }
}

pub fn plan_removal(context: &AdoptionContext, skill_id: &str) -> Result<AdoptionPlan, String> {
    let registry = load_registry(&context.runtime_home)?;
    let record = registry
        .skills
        .get(skill_id)
        .ok_or_else(|| format!("private skill is not adopted: {skill_id}"))?;
    let body = body_path(&context.runtime_home, record);
    let indexes = record
        .target_hosts
        .iter()
        .map(|host| {
            host_index_path(&context.host_home, host, skill_id)
                .ok_or_else(|| format!("unsupported skill host: {host}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for index in &indexes {
        if index.exists() && !index_points_to(index, &body) {
            return Err(format!(
                "host index no longer points at the adopted body: {}",
                index.display()
            ));
        }
    }
    let mut plan = AdoptionPlan {
        schema_version: ADOPTION_PLAN_SCHEMA.to_string(),
        operation: "remove".to_string(),
        plan_hash: String::new(),
        skill_id: skill_id.to_string(),
        source: record.source.clone(),
        source_hash: record.source_hash.clone(),
        license_path: record.license_path.clone(),
        license_hash: record.license_hash.clone(),
        routing_metadata_path: record.routing_metadata_path.clone(),
        routing_metadata_hash: record.routing_metadata_hash.clone(),
        body_path: body.to_string_lossy().into_owned(),
        registry_path: registry_path(&context.runtime_home)
            .to_string_lossy()
            .into_owned(),
        target_hosts: record.target_hosts.clone(),
        host_indexes: indexes
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        planned_writes: vec![
            "unlink exact adopted host indexes".to_string(),
            "remove private registry entry".to_string(),
            "refresh selected host capability snapshots".to_string(),
        ],
        warnings: vec!["immutable body is retained for recoverable rollback".to_string()],
    };
    plan.plan_hash = plan_hash(&plan)?;
    Ok(plan)
}

pub fn apply_removal(
    context: &AdoptionContext,
    skill_id: &str,
    expected_plan_hash: &str,
) -> Result<AdoptionReceipt, String> {
    let plan = plan_removal(context, skill_id)?;
    if plan.plan_hash != expected_plan_hash {
        return Err(format!(
            "removal_plan_changed: reviewed {}, current {}",
            expected_plan_hash, plan.plan_hash
        ));
    }
    let registry_file = registry_path(&context.runtime_home);
    let registry_backup = read_optional(&registry_file)?;
    let link_backups = capture_links(&plan.host_indexes)?;
    let snapshot_backups = capture_snapshots(&context.runtime_home, &plan.target_hosts)?;
    let applied = (|| -> Result<(PrivateSkillRegistry, BTreeMap<String, String>), String> {
        for index in &plan.host_indexes {
            let index = Path::new(index);
            if fs::symlink_metadata(index).is_ok() {
                fs::remove_file(index)
                    .map_err(|error| format!("cannot unlink {}: {error}", index.display()))?;
            }
        }
        let mut registry = load_registry(&context.runtime_home)?;
        registry.skills.remove(skill_id);
        registry.revision = registry.revision.saturating_add(1);
        write_registry(&context.runtime_home, &registry)?;
        let snapshots = refresh_snapshots(context, &plan.target_hosts)?;
        Ok((registry, snapshots))
    })();
    match applied {
        Ok((registry, snapshots)) => Ok(AdoptionReceipt {
            schema_version: ADOPTION_RECEIPT_SCHEMA.to_string(),
            operation: "remove".to_string(),
            plan_hash: plan.plan_hash,
            skill_id: skill_id.to_string(),
            registry_revision: registry.revision,
            body_path: plan.body_path,
            host_indexes: plan.host_indexes,
            snapshot_hashes: snapshots,
            requires_repreflight: true,
        }),
        Err(error) => {
            let rollback = rollback(
                &registry_file,
                registry_backup.as_deref(),
                &link_backups,
                &snapshot_backups,
                None,
            );
            Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
            })
        }
    }
}

fn normalize_hosts(requested: &[String]) -> Result<Vec<String>, String> {
    let supported = ags_host_integration::supported_skill_hosts()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut hosts = if requested.is_empty() || requested.iter().any(|host| host == "all") {
        supported.iter().cloned().collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    hosts.sort();
    hosts.dedup();
    if let Some(unsupported) = hosts.iter().find(|host| !supported.contains(*host)) {
        return Err(format!("unsupported skill host: {unsupported}"));
    }
    Ok(hosts)
}

fn reject_official_collision(authority_root: &Path, skill_id: &str) -> Result<(), String> {
    let path = authority_root.join("manifests/skills-registry.yaml");
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "cannot read official skill registry {}: {error}",
            path.display()
        )
    })?;
    let document: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("cannot parse official skill registry: {error}"))?;
    let collision = document["skills"]
        .as_sequence()
        .into_iter()
        .flatten()
        .any(|skill| skill["name"].as_str() == Some(skill_id));
    if collision {
        Err(format!(
            "private adoption cannot shadow official skill id: {skill_id}"
        ))
    } else {
        Ok(())
    }
}

fn plan_hash(plan: &AdoptionPlan) -> Result<String, String> {
    let mut canonical = plan.clone();
    canonical.plan_hash.clear();
    serde_json::to_vec(&canonical)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("cannot serialize adoption plan: {error}"))
}

fn install_body(source: &Path, body: &Path, expected_hash: &str) -> Result<bool, String> {
    if body.exists() {
        let actual = hash_skill_source(body)?;
        if actual == expected_hash {
            return Ok(false);
        }
        return Err(format!(
            "immutable body path contains different content: {}",
            body.display()
        ));
    }
    let parent = body
        .parent()
        .ok_or_else(|| "immutable body path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create body parent {}: {error}", parent.display()))?;
    let stage = parent.join(format!(
        ".stage-{}-{}",
        std::process::id(),
        body.file_name().unwrap().to_string_lossy()
    ));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|error| {
            format!("cannot clear stale body stage {}: {error}", stage.display())
        })?;
    }
    copy_tree(source, &stage)?;
    let staged_hash = hash_skill_source(&stage)?;
    if staged_hash != expected_hash {
        let _ = fs::remove_dir_all(&stage);
        return Err("source_drift_during_copy".to_string());
    }
    fs::rename(&stage, body)
        .map_err(|error| format!("cannot publish immutable body {}: {error}", body.display()))?;
    Ok(true)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read source entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("cannot inspect {}: {error}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink_refused: {}", source_path.display()));
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(format!("special_file_refused: {}", source_path.display()));
        }
    }
    Ok(())
}

fn refresh_snapshots(
    context: &AdoptionContext,
    hosts: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let mut hashes = BTreeMap::new();
    for host in hosts {
        let snapshot = match context.snapshot_discovery {
            SnapshotDiscovery::Live => build_capability_snapshot_with_live_roots(
                &context.authority_root,
                host,
                &context.runtime_home,
                &context.host_home,
            ),
            SnapshotDiscovery::Offline => build_capability_snapshot_with_roots(
                &context.authority_root,
                host,
                &context.runtime_home,
                &context.host_home,
            ),
        }
        .map_err(|error| format!("capability snapshot build failed for {host}: {error:?}"))?;
        let json = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| format!("cannot serialize {host} snapshot: {error}"))?;
        write_private_atomic(
            &snapshot_path(&context.runtime_home, host),
            &[json, b"\n".to_vec()].concat(),
        )?;
        hashes.insert(host.clone(), snapshot.snapshot_hash);
    }
    Ok(hashes)
}

fn capture_links(paths: &[String]) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    paths
        .iter()
        .map(|path| {
            let path = PathBuf::from(path);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => fs::read_link(&path)
                    .map(Some)
                    .map(|target| (path.clone(), target))
                    .map_err(|error| format!("cannot read link {}: {error}", path.display())),
                Ok(_) => Err(format!(
                    "host index conflict is not a symlink: {}",
                    path.display()
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
                Err(error) => Err(format!(
                    "cannot inspect host index {}: {error}",
                    path.display()
                )),
            }
        })
        .collect()
}

fn capture_snapshots(runtime_home: &Path, hosts: &[String]) -> Result<Vec<FileBackup>, String> {
    hosts
        .iter()
        .map(|host| {
            let path = snapshot_path(runtime_home, host);
            read_optional(&path).map(|bytes| (path, bytes))
        })
        .collect()
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn replace_symlink(index: &Path, target: &Path) -> Result<(), String> {
    let parent = index
        .parent()
        .ok_or_else(|| "host index has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create host skill root {}: {error}",
            parent.display()
        )
    })?;
    let stage = parent.join(format!(
        ".ags-adopt-{}-{}.tmp",
        std::process::id(),
        index.file_name().unwrap().to_string_lossy()
    ));
    if fs::symlink_metadata(&stage).is_ok() {
        fs::remove_file(&stage)
            .map_err(|error| format!("cannot clear staged link {}: {error}", stage.display()))?;
    }
    create_dir_symlink(target, &stage)?;
    if fs::symlink_metadata(index).is_ok() {
        fs::remove_file(index)
            .map_err(|error| format!("cannot replace host index {}: {error}", index.display()))?;
    }
    fs::rename(&stage, index)
        .map_err(|error| format!("cannot publish host index {}: {error}", index.display()))
}

fn create_dir_symlink(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .map_err(|error| format!("cannot create link {}: {error}", link.display()))
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
            .map_err(|error| format!("cannot create link {}: {error}", link.display()))
    }
}

fn rollback(
    registry_file: &Path,
    registry_backup: Option<&[u8]>,
    links: &[(PathBuf, Option<PathBuf>)],
    snapshots: &[FileBackup],
    new_body: Option<&Path>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = restore_optional_file(registry_file, registry_backup) {
        errors.push(error);
    }
    for (index, previous) in links {
        if fs::symlink_metadata(index).is_ok() {
            if let Err(error) = fs::remove_file(index) {
                errors.push(format!(
                    "cannot clear link {} during rollback: {error}",
                    index.display()
                ));
                continue;
            }
        }
        if let Some(target) = previous {
            if let Err(error) = create_dir_symlink(target, index) {
                errors.push(error);
            }
        }
    }
    for (path, previous) in snapshots {
        if let Err(error) = restore_optional_file(path, previous.as_deref()) {
            errors.push(error);
        }
    }
    if let Some(body) = new_body {
        if body.exists() {
            if let Err(error) = fs::remove_dir_all(body) {
                errors.push(format!(
                    "cannot remove new body {} during rollback: {error}",
                    body.display()
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    if let Some(bytes) = bytes {
        write_private_atomic(path, bytes)
    } else if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("cannot remove {} during rollback: {error}", path.display()))
    } else {
        Ok(())
    }
}
