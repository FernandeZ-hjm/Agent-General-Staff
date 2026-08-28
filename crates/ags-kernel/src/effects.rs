//! Sealed-operation effects (contract v3 §7.8).
//!
//! The registry only contains the sealed subset. Each sealed operation has
//! exactly one effect here; both the CLI and the MCP adapter route through
//! this module, so neither owns a parallel domain workflow.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::capabilities::CapabilitiesLock;
use crate::error::{Error, Result};
use crate::seal::ApplyOutput;
use crate::workspace::WorkspaceBinding;

/// Run the bounded mutation for a sealed operation. Returns the observed
/// write set (paths relative to the workspace root).
pub fn run(operation: &str, payload: &Value, binding: &WorkspaceBinding) -> Result<ApplyOutput> {
    match operation {
        "upgrade" => crate::upgrade::apply(payload, binding),
        "update" => update(payload, binding).map(Into::into),
        "govern.skill.install" => skill_install(payload, binding).map(Into::into),
        "govern.skill.remove" => skill_remove(payload, binding).map(Into::into),
        "govern.host.register" => host_register(payload, binding).map(Into::into),
        "govern.host_projection" => host_projection(payload, binding),
        "govern.delegation.issue" => delegation_issue(payload, binding),
        other if other.starts_with("release") || other.starts_with("promotion") => Err(Error::new(
            "promotion_requires_independent_authorization",
            format!("{other} crosses the A→S→B boundary and is not authorizable from a workspace-local task"),
        )),
        other => Err(Error::new("operation_unknown", format!("sealed registry has no effect for `{other}`"))),
    }
}

/// `init` effect: adopt a workspace. Creates `ags.toml` (no-replace) and the
/// ownership manifest; user files are never overwritten.
pub fn init_effect(root: &std::path::Path, payload: &Value) -> Result<Vec<String>> {
    use crate::projection::{apply, Ownership, ProjectionFile, OWNERSHIP_MANIFEST};
    let mut writes: Vec<String> = Vec::new();
    let ags_dir = root.join(crate::workspace::AGS_DIR);
    std::fs::create_dir_all(&ags_dir).map_err(|e| crate::error::io("ags_dir_create_failed", &e))?;
    let mut ownership = Ownership::load(&ags_dir)?;
    let mut desired: Vec<ProjectionFile> = Vec::new();

    let toml_text = payload
        .get("ags_toml")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("init_payload_missing", "payload requires ags_toml"))?
        .to_string();
    let rel = std::path::PathBuf::from(crate::workspace::AGS_TOML);
    let disposition = crate::projection::classify(root, &rel, &ownership);
    match disposition {
        crate::projection::Disposition::Create => {
            desired.push(ProjectionFile {
                rel_path: rel,
                content: toml_text.into_bytes(),
                disposition: crate::projection::Disposition::Create,
            });
        }
        // Already adopted (AGS-owned) or user-owned: never overwrite. The
        // project still gets registered and entry-synced below, so `ags init`
        // is idempotent re-run for the sync-on-update mechanism.
        crate::projection::Disposition::ReclaimExactOwned
        | crate::projection::Disposition::PreserveUnowned
        | crate::projection::Disposition::PreserveModified => {
            writes.push(format!("{} (preserved)", crate::workspace::AGS_TOML));
        }
    }

    let outcome = apply(root, &ags_dir, &desired, &mut ownership)?;
    for f in outcome.files {
        writes.push(f.rel_path);
    }
    // Optional hook files: installed only when absent; existing files are
    // preserved byte-for-byte (never overwritten).
    if let Some(hooks) = payload.get("hooks").and_then(|v| v.as_array()) {
        for hook in hooks {
            let rel_text = hook
                .get("rel")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::new("init_hook_rel_missing", "hook entry requires rel"))?;
            let content = hook
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Error::new("init_hook_content_missing", "hook entry requires content")
                })?;
            let rel = std::path::PathBuf::from(rel_text);
            match crate::projection::classify(root, &rel, &ownership) {
                crate::projection::Disposition::Create => {
                    let desired_file = ProjectionFile {
                        rel_path: rel,
                        content: content.to_string().into_bytes(),
                        disposition: crate::projection::Disposition::Create,
                    };
                    let file_outcome = apply(
                        root,
                        &ags_dir,
                        std::slice::from_ref(&desired_file),
                        &mut ownership,
                    )?;
                    for f in file_outcome.files {
                        writes.push(f.rel_path);
                    }
                }
                _ => writes.push(format!("{rel_text} (preserved)")),
            }
        }
    }
    writes.push(OWNERSHIP_MANIFEST.to_string());
    // Adoption registers the project for sync-on-update and installs the
    // current managed entry blocks immediately.
    crate::sync::register_project(root)?;
    for name in crate::sync::sync_project(root)? {
        writes.push(format!("entry:{name}"));
    }
    writes.extend(crate::host_projection::sync_workspace_hooks(root)?);
    writes.extend(crate::git_projection::install(root)?);
    Ok(writes)
}

fn update(payload: &Value, binding: &WorkspaceBinding) -> Result<Vec<String>> {
    // Preflight: every predictable failure condition is checked before any
    // write, so an update that fails never leaves a half-updated machine.
    // Remaining failure modes are IO-level; update is idempotent and
    // re-runnable, so a retry converges (see protocol/runtime-adapters.md).
    let config = crate::config::Config::load(&binding.root)?;
    let info = crate::sync::install_info()?;
    if !info.source_root.join("ags-skills").is_dir() {
        return Err(crate::Error::new(
            "skill_source_missing",
            format!(
                "official skill source {} does not exist; re-run `ags setup --source-root <dir>`",
                info.source_root.display()
            ),
        ));
    }
    let registry = crate::sync::load_registry()?;
    for project in &registry.projects {
        if !project.is_dir() {
            return Err(crate::Error::new(
                "update_project_missing",
                format!(
                    "registered project {} does not exist; remove it from ~/.ags/v3/managed.json or restore it",
                    project.display()
                ),
            ));
        }
        crate::sync::preflight_project(project)?;
        crate::git_projection::preflight(project)?;
        crate::host_projection::preflight_workspace_hooks(project)?;
    }
    // Empty or absent sources mean "refresh from the configured defaults".
    let explicit: Option<Vec<String>> = payload
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<String>>()
        })
        .filter(|list: &Vec<String>| !list.is_empty());
    let sources = match explicit {
        Some(list) => list,
        None => config.capabilities.sources.clone(),
    };
    let mut writes: Vec<String> = vec!["capabilities.lock".to_string()];
    crate::capabilities::refresh(binding, &sources)?;
    // Self-healing policy: a configured-but-missing source makes the routing
    // table silently dead. When ags.toml is AGS-owned, prune dead sources and
    // rewrite the policy in the same sealed transaction; user-owned policy
    // files are left alone (check capabilities still flags them).
    if let Some(changes) = prune_dead_sources(&binding.root)? {
        writes.push(format!("ags.toml ({changes})"));
    }
    // The entry ecosystem moves with the product: registered projects'
    // managed blocks, global rules and installed AGS skills are refreshed in
    // the same sealed transaction, so entry files can never drift behind the
    // binary (see protocol/runtime-adapters.md §sync-on-update). Registered
    // projects also get their dead capability sources pruned.
    for project in &registry.projects {
        for name in crate::sync::sync_project(project)? {
            writes.push(format!("entry:{}/{}", project.display(), name));
        }
        for name in crate::host_projection::sync_workspace_hooks(project)? {
            writes.push(format!("{}:{name}", project.display()));
        }
        for name in crate::git_projection::install(project)? {
            writes.push(format!("{}:{name}", project.display()));
        }
        if let Some(changes) = prune_dead_sources(project)? {
            writes.push(format!("policy:{} ({changes})", project.display()));
        }
    }
    for name in crate::sync::sync_rules()? {
        writes.push(format!("rules:{name}"));
    }
    for name in crate::sync::sync_skills(&info.source_root)? {
        writes.push(format!("skill:{name}"));
    }
    // Machine-level capability bodies: every installed skill body (official
    // ags-*, third-party, legacy symlinks) is pinned into the machine lock in
    // the same transaction, so tamper or drift is visible to doctor.
    let (lock, problems) = crate::sync::sync_bodies()?;
    writes.push(format!(
        "machine-capabilities:{} bodies",
        lock.entries.len()
    ));
    for problem in problems {
        writes.push(format!("machine-capabilities-problem:{problem}"));
    }
    Ok(writes)
}

fn skill_install(payload: &Value, binding: &WorkspaceBinding) -> Result<Vec<String>> {
    let id = payload
        .get("skill_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("skill_install_id_missing", "payload requires skill_id"))?;
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::new(
                "skill_install_path_missing",
                "payload requires a local skill directory path",
            )
        })?;
    let prepared;
    let payload = if payload.get("source_sha256").is_some() {
        payload
    } else {
        let acknowledgements: Vec<String> = payload
            .get("acknowledged_risks")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        prepared = crate::skill_adoption::prepare_install(binding, id, path, &acknowledgements)?;
        &prepared
    };
    let (routing, body_path) = crate::skill_adoption::apply_install(binding, payload)?;
    let mut writes = vec![
        "machine-capabilities".to_string(),
        format!("skill-routing:{}", routing.label()),
        format!("skill-body:{body_path}"),
        "skill-registry:~/.ags/v3/installed-skills.json".to_string(),
    ];
    writes.push(crate::capabilities::LOCK_FILE.to_string());
    Ok(writes)
}

fn skill_remove(payload: &Value, binding: &WorkspaceBinding) -> Result<Vec<String>> {
    let id = payload
        .get("skill_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("skill_remove_id_missing", "payload requires skill_id"))?;
    // `remove` is the inverse of install: uninstall the machine symlink and
    // drop this project's audit declaration when present.
    let adopted_removed = crate::skill_adoption::remove(binding, id)?;
    let legacy_removed = if adopted_removed {
        false
    } else {
        crate::sync::remove_skill_body(id)?
    };
    let machine_removed = adopted_removed || legacy_removed;
    let mut lock = CapabilitiesLock::load(binding)?;
    let before = lock.entries.len();
    lock.entries.retain(|e| e.id != id);
    let audit_removed = lock.entries.len() != before;
    if !machine_removed && !audit_removed {
        return Err(Error::new(
            "skill_remove_not_found",
            format!("`{id}` is not installed or declared in this workspace"),
        ));
    }
    let mut writes = Vec::new();
    if machine_removed {
        writes.push("machine-capabilities".to_string());
        writes.push(format!("skill-uninstalled:{id}"));
    }
    if audit_removed {
        let text = serde_json::to_string_pretty(&lock)
            .map_err(|e| Error::new("capabilities_lock_encode_failed", e.to_string()))?;
        std::fs::create_dir_all(&binding.ags_dir)
            .map_err(|e| crate::error::io("ags_dir_create_failed", &e))?;
        let lock_path = binding.ags_dir.join(crate::capabilities::LOCK_FILE);
        let tmp = lock_path.with_extension("tmp");
        std::fs::write(&tmp, text)
            .map_err(|e| crate::error::io("capabilities_lock_write_failed", &e))?;
        std::fs::rename(&tmp, &lock_path)
            .map_err(|e| crate::error::io("capabilities_lock_write_failed", &e))?;
        writes.push(crate::capabilities::LOCK_FILE.to_string());
    }
    Ok(writes)
}

fn host_projection(payload: &Value, binding: &WorkspaceBinding) -> Result<ApplyOutput> {
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("host_projection_mode_missing", "payload requires mode"))?;
    if mode != "reconcile" {
        return Err(Error::new(
            "host_projection_mode_unsupported",
            "contract v3 supports reconcile only",
        ));
    }
    let host = payload
        .get("host")
        .and_then(Value::as_str)
        .map(crate::hosts::normalize_host_id)
        .transpose()?;
    let surface = payload
        .get("surface")
        .and_then(Value::as_str)
        .unwrap_or("cli");
    let descriptor = crate::host_projection::connection_descriptor(surface)?;
    let lifecycle = payload
        .get("lifecycle")
        .and_then(Value::as_str)
        .unwrap_or("full");
    let requested_slug = payload.get("slug").and_then(Value::as_str);
    if lifecycle != "full" {
        return Err(Error::new(
            "host_lifecycle_invalid",
            "v0.4.21 requires lifecycle=full",
        ));
    }
    if let Some(host) = host.as_deref() {
        crate::host_projection::preflight_client(host, surface)?;
    }

    let mut config = crate::config::Config::load(&binding.root)?;
    let config_path = binding.root.join(crate::workspace::AGS_TOML);
    let before = fs::read_to_string(&config_path)
        .map_err(|e| crate::error::io("ags_toml_read_failed", &e))?;
    crate::projection::reject_symlink_path(
        &binding.root,
        Path::new(".ags")
            .join(crate::projection::OWNERSHIP_MANIFEST)
            .as_path(),
    )?;
    let hook_snapshot = crate::host_projection::snapshot_workspace_hooks(&binding.root)?;
    let ownership_path = binding.ags_dir.join(crate::projection::OWNERSHIP_MANIFEST);
    let ownership_snapshot = vec![(
        ownership_path.clone(),
        crate::host_projection::read_optional_file(&ownership_path, "ownership_snapshot_failed")?,
    )];
    let old_slug = config.workspace.slug.clone();
    let mut memory_move = None;
    if let Some(new_slug) = requested_slug {
        let old_memory = crate::host_projection::legacy_memory_dir(&old_slug)?;
        let new_memory = crate::host_projection::memory_dir(new_slug)?;
        if old_memory != new_memory && old_memory.exists() {
            if let Some(parent) = new_memory.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| crate::error::io("memory_slug_migration_failed", &e))?;
            }
            memory_move = Some((old_memory, new_memory));
        }
        config.workspace.slug = new_slug.to_string();
    }
    let mut canonical_hosts: std::collections::BTreeMap<String, crate::config::HostEntry> =
        std::collections::BTreeMap::new();
    for entry in config.hosts.drain(..) {
        let id = crate::hosts::normalize_host_id(&entry.id)?;
        let entry_surface = if entry.surface == "hybrid" {
            "cli".to_string()
        } else {
            entry.surface.clone()
        };
        canonical_hosts
            .entry(id.clone())
            .and_modify(|current| {
                current.dispatch |= entry.dispatch;
                if current.surface == "cli" && entry_surface != "cli" {
                    current.surface = entry_surface.clone();
                }
            })
            .or_insert(crate::config::HostEntry {
                id,
                surface: entry_surface,
                dispatch: entry.dispatch,
            });
    }
    config.hosts = canonical_hosts.into_values().collect();
    if let Some(host) = host.as_ref() {
        let dispatch = config
            .hosts
            .iter()
            .find(|entry| crate::hosts::normalize_host_id(&entry.id).ok().as_deref() == Some(host))
            .map(|entry| entry.dispatch)
            .unwrap_or(false);
        config.hosts.retain(|entry| {
            crate::hosts::normalize_host_id(&entry.id).ok().as_deref() != Some(host)
        });
        config.hosts.push(crate::config::HostEntry {
            id: host.clone(),
            surface: surface.to_string(),
            dispatch,
        });
    }
    config.hosts.sort_by(|a, b| a.id.cmp(&b.id));
    config.hosts.dedup_by(|a, b| a.id == b.id);
    let text = toml::to_string(&config)
        .map_err(|e| Error::new("ags_toml_encode_failed", e.to_string()))?;
    let mut writes = Vec::new();
    let mut applied_memory = None;
    if before != text {
        let path = binding.root.join(crate::workspace::AGS_TOML);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &text).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
        std::fs::rename(&tmp, &path).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
        writes.push(crate::workspace::AGS_TOML.to_string());
    }
    if let Some((old_memory, new_memory)) = &memory_move {
        match crate::host_projection::migrate_memory(old_memory, new_memory) {
            Ok(migration) => {
                applied_memory = migration;
                writes.push(format!(
                    "memory-slug:{}->{}",
                    old_memory.display(),
                    new_memory.display()
                ));
            }
            Err(error) => {
                let _ = fs::write(&config_path, &before);
                return Err(error);
            }
        }
    }
    if before != text {
        match refresh_policy_ownership(binding, &text) {
            Ok(true) => writes.push(format!(
                "{}/{}",
                crate::workspace::AGS_DIR,
                crate::projection::OWNERSHIP_MANIFEST
            )),
            Ok(false) => {}
            Err(error) => {
                if let Some(migration) = &applied_memory {
                    let _ = crate::host_projection::rollback_memory(migration);
                }
                let _ = fs::write(&config_path, &before);
                let _ = crate::host_projection::restore_files(&ownership_snapshot);
                return Err(error);
            }
        }
    }
    let projection_result = (|| -> Result<Vec<String>> {
        let mut projected = crate::host_projection::sync_workspace_hooks(&binding.root)?;
        if let Some(host) = host.as_deref() {
            projected.extend(crate::host_projection::reconcile_client(
                &binding.root,
                host,
                surface,
            )?);
        }
        Ok(projected)
    })();
    match projection_result {
        Ok(projected) => writes.extend(projected),
        Err(error) => {
            let _ = crate::host_projection::restore_files(&hook_snapshot);
            let _ = crate::host_projection::restore_files(&ownership_snapshot);
            if let Some(migration) = &applied_memory {
                let _ = crate::host_projection::rollback_memory(migration);
            }
            let _ = fs::write(&config_path, &before);
            return Err(error);
        }
    }
    Ok(ApplyOutput::with_result(
        writes,
        serde_json::json!({
            "state": "host_ready",
            "host": host,
            "lifecycle": lifecycle,
            "workspace_slug": config.workspace.slug,
            "connection": descriptor,
        }),
    ))
}

fn host_register(payload: &Value, binding: &WorkspaceBinding) -> Result<Vec<String>> {
    let id = payload
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| Error::new("host_id_missing", "payload requires id"))?;
    let id = crate::hosts::normalize_host_id(id)?;
    let surface = payload
        .get("surface")
        .and_then(|value| value.as_str())
        .ok_or_else(|| Error::new("host_surface_missing", "payload requires surface"))?;
    if !matches!(surface, "cli" | "mcp") {
        return Err(Error::new(
            "host_surface_invalid",
            "generic host registration supports cli or mcp",
        ));
    }
    let dispatch = payload
        .get("dispatch")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let mut config = crate::config::Config::load(&binding.root)?;
    config
        .hosts
        .retain(|host| crate::hosts::normalize_host_id(&host.id).ok().as_deref() != Some(&id));
    config.hosts.push(crate::config::HostEntry {
        id: id.clone(),
        surface: surface.to_string(),
        dispatch,
    });
    config.hosts.sort_by(|left, right| left.id.cmp(&right.id));
    if config.workspace.role == "A" {
        for operation in crate::config::CANONICAL_SEALED_OPS {
            if !config.sealed.ops.iter().any(|current| current == operation) {
                config.sealed.ops.push((*operation).to_string());
            }
        }
    }
    let text = toml::to_string(&config)
        .map_err(|e| Error::new("ags_toml_encode_failed", e.to_string()))?;
    let path = binding.root.join(crate::workspace::AGS_TOML);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &text).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
    std::fs::rename(&tmp, &path).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
    let mut writes = vec![crate::workspace::AGS_TOML.to_string()];
    if refresh_policy_ownership(binding, &text)? {
        writes.push(format!(
            "{}/{}",
            crate::workspace::AGS_DIR,
            crate::projection::OWNERSHIP_MANIFEST
        ));
    }
    Ok(writes)
}

fn refresh_policy_ownership(binding: &WorkspaceBinding, text: &str) -> Result<bool> {
    let mut ownership = crate::projection::Ownership::load(&binding.ags_dir)?;
    if !ownership.paths.contains_key(crate::workspace::AGS_TOML) {
        return Ok(false);
    }
    ownership.paths.insert(
        crate::workspace::AGS_TOML.to_string(),
        crate::workspace::sha256_hex(text.as_bytes()),
    );
    let path = binding.ags_dir.join(crate::projection::OWNERSHIP_MANIFEST);
    let tmp = path.with_extension("tmp");
    let manifest = serde_json::to_string_pretty(&ownership)
        .map_err(|e| Error::new("ownership_encode_failed", e.to_string()))?;
    fs::write(&tmp, manifest).map_err(|e| crate::error::io("ownership_write_failed", &e))?;
    fs::rename(&tmp, &path).map_err(|e| crate::error::io("ownership_write_failed", &e))?;
    Ok(true)
}

fn delegation_issue(payload: &Value, binding: &WorkspaceBinding) -> Result<ApplyOutput> {
    let config = crate::config::Config::load(&binding.root)?;
    let event = crate::delegation::issue(binding, &config, payload)?;
    let result = crate::delegation::dispatch_result(&event)?;
    Ok(ApplyOutput::with_result(
        vec![format!("evidence:{}", event.event_id)],
        result,
    ))
}

/// Prune configured capability sources whose relative directory does not
/// exist, and keep the ownership manifest in lockstep with the policy.
/// Only touches files that are AGS-authored: the file must parse AND be in
/// canonical `toml::to_string` form (user-formatted files are left alone),
/// and the ownership manifest must already record ags.toml. Returns the
/// pruned source list or None when nothing was pruned.
fn prune_dead_sources(root: &std::path::Path) -> Result<Option<String>> {
    let Ok(config) = crate::config::Config::load(root) else {
        return Ok(None);
    };
    let canonical = toml::to_string(&config)
        .map_err(|e| Error::new("ags_toml_encode_failed", e.to_string()))?;
    let path = root.join(crate::workspace::AGS_TOML);
    let current = fs::read_to_string(&path).unwrap_or_default();
    if current != canonical {
        // User-formatted policy: never rewrite, never claim ownership drift.
        return Ok(None);
    }
    let ags_dir = root.join(crate::workspace::AGS_DIR);
    let mut ownership = crate::projection::Ownership::load(&ags_dir)?;
    if !ownership.paths.contains_key(crate::workspace::AGS_TOML) {
        return Ok(None);
    }
    let missing: Vec<String> = config
        .capabilities
        .sources
        .iter()
        .filter(|s| !root.join(s).is_dir() && !std::path::Path::new(s).is_absolute())
        .cloned()
        .collect();
    let missing_ops: Vec<String> = if config.workspace.role == "A" {
        crate::config::CANONICAL_SEALED_OPS
            .iter()
            .filter(|op| !config.sealed.ops.iter().any(|current| current == **op))
            .map(|op| (*op).to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Heal: the recorded hash must equal the canonical bytes. Stale records
    // (e.g. from a prune before the lockstep fix) are corrected even when
    // nothing needs pruning now.
    let new_hash = crate::workspace::sha256_hex(canonical.as_bytes());
    let recorded = ownership.paths.get(crate::workspace::AGS_TOML).cloned();
    let manifest = ags_dir.join(crate::projection::OWNERSHIP_MANIFEST);
    let write_manifest = |ownership: &crate::projection::Ownership| -> Result<()> {
        let manifest_text = serde_json::to_string_pretty(ownership)
            .map_err(|e| Error::new("ownership_encode_failed", e.to_string()))?;
        let manifest_tmp = manifest.with_extension("tmp");
        fs::write(&manifest_tmp, manifest_text)
            .map_err(|e| crate::error::io("ownership_write_failed", &e))?;
        fs::rename(&manifest_tmp, &manifest)
            .map_err(|e| crate::error::io("ownership_write_failed", &e))
    };
    if missing.is_empty() && missing_ops.is_empty() {
        if recorded.as_deref() != Some(new_hash.as_str()) {
            ownership
                .paths
                .insert(crate::workspace::AGS_TOML.to_string(), new_hash);
            write_manifest(&ownership)?;
        }
        return Ok(None);
    }

    let mut reconciled = config;
    reconciled
        .capabilities
        .sources
        .retain(|s| root.join(s).is_dir() || std::path::Path::new(s).is_absolute());
    for operation in &missing_ops {
        reconciled.sealed.ops.push(operation.clone());
    }
    let text = toml::to_string(&reconciled)
        .map_err(|e| Error::new("ags_toml_encode_failed", e.to_string()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &text).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
    fs::rename(&tmp, &path).map_err(|e| crate::error::io("ags_toml_write_failed", &e))?;
    // Ownership lockstep: the rewritten policy bytes become the new recorded
    // hash, so future init/update classify it as AGS-owned (exact match).
    let new_hash = crate::workspace::sha256_hex(text.as_bytes());
    ownership
        .paths
        .insert(crate::workspace::AGS_TOML.to_string(), new_hash);
    write_manifest(&ownership)?;
    let mut changes = Vec::new();
    if !missing.is_empty() {
        changes.push(format!("sources pruned: {}", missing.join(", ")));
    }
    if !missing_ops.is_empty() {
        changes.push(format!("sealed ops added: {}", missing_ops.join(", ")));
    }
    Ok(Some(changes.join("; ")))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::bind;
    use std::fs;

    fn ws(tmp: &tempfile::TempDir) -> WorkspaceBinding {
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n",
        )
        .unwrap();
        bind(&root).unwrap()
    }

    #[test]
    fn update_refreshes_lock_and_syncs_entries() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // Isolate the machine-local registry/rules/skills side of the
        // sync-on-update transaction.
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        let binding = ws(&tmp);
        fs::create_dir_all(binding.root.join("ags-skills/demo")).unwrap();
        fs::write(binding.root.join("ags-skills/demo/SKILL.md"), "# Demo\n").unwrap();
        // Machine install record + official skill source must exist before
        // update converges; this mirrors `ags setup` on a real machine.
        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("ags-skills/ags-demo")).unwrap();
        fs::write(
            source.join("ags-skills/ags-demo/SKILL.md"),
            "---\nname: ags-demo\ndescription: Official.\n---\n",
        )
        .unwrap();
        crate::sync::setup(&source).unwrap();
        let writes = run(
            "update",
            &serde_json::json!({"sources": ["ags-skills"]}),
            &binding,
        )
        .unwrap();
        assert!(writes
            .observed_write_set
            .iter()
            .any(|w| w == "capabilities.lock"));
        // Rules were already converged by setup; update is idempotent and
        // skips identical content.
        assert!(!writes
            .observed_write_set
            .iter()
            .any(|w| w.starts_with("rules:")));
        // No registered projects in the isolated registry → no entry writes.
        assert!(!writes
            .observed_write_set
            .iter()
            .any(|w| w.starts_with("entry:")));
        let lock = CapabilitiesLock::load(&binding).unwrap();
        assert_eq!(lock.entries.len(), 1);
        assert_eq!(lock.entries[0].id, "demo");
    }

    #[test]
    fn update_preflights_every_project_before_writing() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        let binding = ws(&tmp);
        fs::write(binding.root.join("AGENTS.md"), "project instructions\n").unwrap();
        let mut permissions = fs::metadata(binding.root.join("AGENTS.md"))
            .unwrap()
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(binding.root.join("AGENTS.md"), permissions).unwrap();
        crate::sync::register_project(&binding.root).unwrap();

        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("ags-skills/ags-demo")).unwrap();
        fs::write(
            source.join("ags-skills/ags-demo/SKILL.md"),
            "---\nname: ags-demo\ndescription: Official.\n---\n",
        )
        .unwrap();
        crate::sync::setup(&source).unwrap();

        let error = run("update", &serde_json::json!({}), &binding).unwrap_err();
        assert_eq!(error.code, "entry_not_writable");
        assert!(!binding.ags_dir.join("capabilities.lock").exists());
    }

    #[test]
    fn release_requires_independent_authorization() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let err = run("release:project-public", &serde_json::json!({}), &binding).unwrap_err();
        assert_eq!(err.code, "promotion_requires_independent_authorization");
    }

    #[test]
    fn generic_host_registration_upserts_transport_and_dispatch() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let config = crate::config::Config::load(&binding.root).unwrap();
        let text = toml::to_string(&config).unwrap();
        fs::write(binding.root.join("ags.toml"), &text).unwrap();
        fs::create_dir_all(&binding.ags_dir).unwrap();
        let mut ownership = crate::projection::Ownership {
            paths: std::collections::BTreeMap::new(),
        };
        ownership.paths.insert(
            crate::workspace::AGS_TOML.to_string(),
            crate::workspace::sha256_hex(text.as_bytes()),
        );
        fs::write(
            binding.ags_dir.join(crate::projection::OWNERSHIP_MANIFEST),
            serde_json::to_string_pretty(&ownership).unwrap(),
        )
        .unwrap();
        let output = run(
            "govern.host.register",
            &serde_json::json!({
                "id": "Future Host",
                "surface": "cli",
                "dispatch": true,
            }),
            &binding,
        )
        .unwrap();
        assert_eq!(
            output.observed_write_set,
            vec!["ags.toml".to_string(), ".ags/ownership-v2.json".to_string()]
        );
        let config = crate::config::Config::load(&binding.root).unwrap();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].id, "future-host");
        assert_eq!(config.hosts[0].surface, "cli");
        assert!(config.hosts[0].dispatch);
        assert!(crate::config::CANONICAL_SEALED_OPS
            .iter()
            .all(|operation| config.sealed.ops.iter().any(|current| current == operation)));
        let current = fs::read_to_string(binding.root.join("ags.toml")).unwrap();
        let ownership = crate::projection::Ownership::load(&binding.ags_dir).unwrap();
        let expected_hash = crate::workspace::sha256_hex(current.as_bytes());
        assert_eq!(
            ownership.paths.get("ags.toml").map(String::as_str),
            Some(expected_hash.as_str())
        );

        run(
            "govern.host.register",
            &serde_json::json!({
                "id": "future-host",
                "surface": "mcp",
                "dispatch": false,
            }),
            &binding,
        )
        .unwrap();
        let config = crate::config::Config::load(&binding.root).unwrap();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].surface, "mcp");
        assert!(!config.hosts[0].dispatch);
    }

    #[test]
    fn host_projection_migrates_slug_and_memory_without_manual_edits() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let binding = ws(&tmp);
        let old_memory = tmp.path().join(".agents/memory/projects/t");
        fs::create_dir_all(&old_memory).unwrap();
        fs::write(old_memory.join("context-capsule.md"), "# Memory\n").unwrap();
        let output = run(
            "govern.host_projection",
            &serde_json::json!({
                "mode": "reconcile",
                "surface": "cli",
                "lifecycle": "full",
                "slug": "renamed-workspace",
            }),
            &binding,
        )
        .unwrap();
        assert_eq!(
            output.result.unwrap()["workspace_slug"],
            "renamed-workspace"
        );
        assert_eq!(
            crate::config::Config::load(&binding.root)
                .unwrap()
                .workspace
                .slug,
            "renamed-workspace"
        );
        assert!(tmp
            .path()
            .join(".agents/memory/projects/renamed-workspace/context-capsule.md")
            .is_file());
    }

    #[test]
    fn host_projection_collapses_transport_aliases() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let binding = ws(&tmp);
        let mut config = crate::config::Config::load(&binding.root).unwrap();
        config.hosts = vec![
            crate::config::HostEntry {
                id: "codex".to_string(),
                surface: "hybrid".to_string(),
                dispatch: false,
            },
            crate::config::HostEntry {
                id: "codex-cli".to_string(),
                surface: "cli".to_string(),
                dispatch: true,
            },
        ];
        fs::write(
            binding.root.join("ags.toml"),
            toml::to_string(&config).unwrap(),
        )
        .unwrap();
        run(
            "govern.host_projection",
            &serde_json::json!({"mode": "reconcile", "surface": "cli", "lifecycle": "full"}),
            &binding,
        )
        .unwrap();
        let config = crate::config::Config::load(&binding.root).unwrap();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].id, "codex");
        assert_eq!(config.hosts[0].surface, "cli");
        assert!(config.hosts[0].dispatch);
    }

    #[test]
    fn host_projection_rejects_surface_before_any_write() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let before = fs::read(binding.root.join("ags.toml")).unwrap();
        let error = run(
            "govern.host_projection",
            &serde_json::json!({
                "mode": "reconcile",
                "host": "codex",
                "surface": "typo",
                "lifecycle": "full",
            }),
            &binding,
        )
        .unwrap_err();
        assert_eq!(error.code, "host_surface_invalid");
        assert_eq!(fs::read(binding.root.join("ags.toml")).unwrap(), before);
        assert!(!binding.root.join(".codex/hooks.json").exists());
    }

    #[test]
    fn host_projection_rolls_back_when_client_reconcile_fails() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let codex = bin.join("codex");
        fs::write(
            &codex,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nif [ \"$2\" = \"add\" ]; then exit 9; fi\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{old_path}", bin.display()));
        let binding = ws(&tmp);
        let hook = binding.root.join(".codex/hooks.json");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();
        let user_hook = b"{\"hooks\":{\"SessionStart\":[{\"command\":\"user-hook\"}]}}\n";
        fs::write(&hook, user_hook).unwrap();
        let config_before = fs::read(binding.root.join("ags.toml")).unwrap();
        let error = run(
            "govern.host_projection",
            &serde_json::json!({
                "mode": "reconcile",
                "host": "codex",
                "surface": "mcp",
                "lifecycle": "full",
            }),
            &binding,
        )
        .unwrap_err();
        std::env::set_var("PATH", old_path);
        assert_eq!(error.code, "host_client_command_failed");
        assert_eq!(
            fs::read(binding.root.join("ags.toml")).unwrap(),
            config_before
        );
        assert_eq!(fs::read(hook).unwrap(), user_hook);
    }

    #[test]
    fn update_reconciles_new_canonical_ops_in_owned_config() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let mut config = crate::config::scaffold("t");
        config
            .sealed
            .ops
            .retain(|operation| operation != "govern.host.register");
        let text = toml::to_string(&config).unwrap();
        fs::write(binding.root.join("ags.toml"), &text).unwrap();
        fs::create_dir_all(&binding.ags_dir).unwrap();
        let mut ownership = crate::projection::Ownership {
            paths: std::collections::BTreeMap::new(),
        };
        ownership.paths.insert(
            crate::workspace::AGS_TOML.to_string(),
            crate::workspace::sha256_hex(text.as_bytes()),
        );
        fs::write(
            binding.ags_dir.join(crate::projection::OWNERSHIP_MANIFEST),
            serde_json::to_string_pretty(&ownership).unwrap(),
        )
        .unwrap();

        let changes = prune_dead_sources(&binding.root).unwrap().unwrap();
        assert!(changes.contains("govern.host.register"));
        let config = crate::config::Config::load(&binding.root).unwrap();
        assert!(config
            .sealed
            .ops
            .iter()
            .any(|operation| operation == "govern.host.register"));
    }

    #[test]
    fn update_does_not_expand_sealed_ops_outside_role_a() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let mut config = crate::config::scaffold("t");
        config.workspace.role = "S".to_string();
        config
            .sealed
            .ops
            .retain(|operation| operation != "govern.host.register");
        let text = toml::to_string(&config).unwrap();
        fs::write(binding.root.join("ags.toml"), &text).unwrap();
        fs::create_dir_all(&binding.ags_dir).unwrap();
        let mut ownership = crate::projection::Ownership {
            paths: std::collections::BTreeMap::new(),
        };
        ownership.paths.insert(
            crate::workspace::AGS_TOML.to_string(),
            crate::workspace::sha256_hex(text.as_bytes()),
        );
        fs::write(
            binding.ags_dir.join(crate::projection::OWNERSHIP_MANIFEST),
            serde_json::to_string_pretty(&ownership).unwrap(),
        )
        .unwrap();

        let _ = prune_dead_sources(&binding.root).unwrap();
        let config = crate::config::Config::load(&binding.root).unwrap();
        assert!(!config
            .sealed
            .ops
            .iter()
            .any(|operation| operation == "govern.host.register"));
    }

    #[test]
    fn skill_install_rejects_paths_outside_workspace() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let binding = ws(&tmp);
        // An absolute path must be rejected outright.
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: demo\ndescription: Outside.\n---\n",
        )
        .unwrap();
        let err = run(
            "govern.skill.install",
            &serde_json::json!({"skill_id": "demo", "path": outside}),
            &binding,
        )
        .unwrap_err();
        assert_eq!(err.code, "skill_install_path_outside_workspace");
        // A relative path escaping the root via .. must be rejected too.
        let err = run(
            "govern.skill.install",
            &serde_json::json!({"skill_id": "demo", "path": "../outside"}),
            &binding,
        )
        .unwrap_err();
        assert_eq!(err.code, "skill_install_path_outside_workspace");
    }

    #[test]
    fn skill_install_remove_roundtrip() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let binding = ws(&tmp);
        fs::create_dir_all(binding.root.join("skill-packs/demo")).unwrap();
        fs::write(
            binding.root.join("skill-packs/demo/SKILL.md"),
            "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo need\n---\n# D\n",
        )
        .unwrap();
        fs::write(binding.root.join("skill-packs/demo/LICENSE"), "MIT\n").unwrap();
        run(
            "govern.skill.install",
            &serde_json::json!({"skill_id": "demo", "path": "skill-packs/demo"}),
            &binding,
        )
        .unwrap();
        let lock = CapabilitiesLock::load(&binding).unwrap();
        assert_eq!(lock.entries.len(), 1);
        // Machine-level install materialized the body into ~/.agents/skills.
        let skills = tmp.path().join(".agents/skills/demo");
        assert!(
            skills.exists(),
            "body must be materialized into ~/.agents/skills"
        );
        let machine = crate::sync::load_machine_lock().unwrap();
        assert!(
            machine.entries.iter().any(|e| e.id == "demo"),
            "machine lock must pin the installed body"
        );
        run(
            "govern.skill.remove",
            &serde_json::json!({"skill_id": "demo"}),
            &binding,
        )
        .unwrap();
        assert!(CapabilitiesLock::load(&binding).unwrap().entries.is_empty());
        assert!(!skills.exists(), "machine symlink must be removed");
        let machine = crate::sync::load_machine_lock().unwrap();
        assert!(!machine.entries.iter().any(|e| e.id == "demo"));
    }
}
