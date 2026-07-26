use super::*;
#[allow(unused_imports)]
use super::{actions::*, apply_transaction::*, host_probe::*, inventory::*, model::*};
// ── Cross-Agent capability sync ──────────────────────────────────────────────
//
// `sync_plan` is the batch face: it builds the inventory ONCE and produces an
// adopt proposal for every adopted/governed capability, so a single call shows
// (and, with `apply`, performs) the cross-host entry plan for the whole set.
// AGS-owned skill thin-index writes go through the same single mutation guard;
// MCP / CLI-backed capabilities remain advise-only. Reused by
// `ags capability sync`.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySyncSummary {
    /// Capabilities considered for sync (adopted suite skills + governed MCPs).
    pub considered: usize,
    /// Total AGS-owned writes planned across all considered capabilities.
    pub planned_writes: usize,
    /// Capabilities whose AGS-owned writes were applied (apply mode only).
    pub applied: usize,
    /// Capabilities whose only action is an advised host command AGS never runs.
    pub advised_only: usize,
    /// Capabilities with at least one blocked reason.
    pub blocked: usize,
    /// Capabilities whose apply errored.
    pub failed: usize,
    /// Capabilities that need action (planned writes or advised commands).
    pub needs_action: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySyncResult {
    pub schema_version: String,
    pub hosts: Vec<String>,
    pub apply_requested: bool,
    pub items: Vec<ConsoleProposalResult>,
    pub shared_store_hygiene: SharedStoreHygieneResult,
    pub summary: CapabilitySyncSummary,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedStoreHygieneResult {
    pub apply_requested: bool,
    /// "dry-run" | "applied" | "failed" | "blocked" | "nothing-to-do"
    pub apply_status: String,
    pub planned_writes: Vec<PlannedWrite>,
    pub applied_writes: Vec<String>,
    pub apply_errors: Vec<String>,
    pub blocked_reasons: Vec<String>,
    pub note: String,
}

/// A capability is syncable through the console when AGS governs it as an
/// adopted suite skill (distributable thin-index) or a governed MCP
/// (advise-only). AGS self (suite-interface), discovered/ignored/unmanaged
/// capabilities are never auto-synced.
pub(super) fn is_syncable(cap: &ManagedCapability) -> bool {
    // Retired capabilities are never synced into a host, regardless of
    // managed_status — a retired front-stage entry must not be resurrected.
    if cap
        .routing
        .as_ref()
        .is_some_and(|r| r.route_state == RouteState::Retired)
    {
        return false;
    }
    matches!(
        cap.managed_status,
        ManagedStatus::SuiteManaged | ManagedStatus::Governed
    )
}

pub(super) const AGS_SUITE_ROOT_NAMES: &[&str] = &[
    "Agent-General-Staff",
    "agent-governance-suite",
    "agent-governance-suite-runtime",
];

/// Read the top-level skill registry authority for deliberately retired suite
/// skills. External-manager bodies are excluded: AGS does not own their host
/// entries and therefore must never clean them up automatically.
pub(super) fn retired_suite_skill_names(repo_root: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(repo_root.join("manifests/skills-registry.yaml"))
    else {
        return Vec::new();
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        return Vec::new();
    };
    let Some(skills) = doc.get("skills").and_then(serde_yaml::Value::as_sequence) else {
        return Vec::new();
    };

    let mut names = std::collections::BTreeSet::new();
    for skill in skills {
        let Some(name) = skill.get("name").and_then(serde_yaml::Value::as_str) else {
            continue;
        };
        let retired = skill
            .get("routing")
            .and_then(|routing| routing.get("route_state"))
            .and_then(serde_yaml::Value::as_str)
            == Some("retired");
        let external_body = skill
            .get("source")
            .and_then(|source| source.get("type"))
            .and_then(serde_yaml::Value::as_str)
            == Some("external_cli_skill");
        if retired && !external_body && is_safe_path_component(name) {
            names.insert(name.to_string());
        }
    }
    names.into_iter().collect()
}

pub(super) fn normalized_absolute_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::Normal(_) => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
        }
    }
    normalized.is_absolute().then_some(normalized)
}

/// Return the three suite roots only when the injected repository itself is a
/// recognized suite workspace. This binds cleanup to siblings of the current
/// authority instead of trusting an arbitrary path that merely contains a
/// similar-looking directory name.
pub(super) fn sibling_ags_suite_roots(repo_root: &Path) -> Vec<PathBuf> {
    let candidate = normalized_absolute_path(repo_root)
        .or_else(|| std::fs::canonicalize(repo_root).ok())
        .unwrap_or_else(|| repo_root.to_path_buf());
    let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    if !AGS_SUITE_ROOT_NAMES.contains(&name) {
        return Vec::new();
    }
    let Some(parent) = candidate.parent() else {
        return Vec::new();
    };
    AGS_SUITE_ROOT_NAMES
        .iter()
        .map(|name| parent.join(name))
        .filter(|root| root.is_dir())
        .collect()
}

pub(super) fn canonicalize_nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if let Ok(real) = std::fs::canonicalize(candidate) {
            return Some(real);
        }
        current = candidate.parent();
    }
    None
}

pub(super) fn suite_store_target_matches_name(relative: &Path, name: &str) -> bool {
    let components: Vec<&std::ffi::OsStr> = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();
    let expected = std::ffi::OsStr::new(name);
    matches!(
        components.as_slice(),
        [store, skill]
            if *store == std::ffi::OsStr::new("global-skills") && *skill == expected
    ) || matches!(
        components.as_slice(),
        [store, profile, skill]
            if *store == std::ffi::OsStr::new("skill-packs")
                && is_safe_path_component(&profile.to_string_lossy())
                && *skill == expected
    )
}

/// Prove that `entry` is an AGS-owned retired-skill thin index. The entry must
/// be a symlink; its resolved target must have the exact canonical-store shape
/// for `name`; and both resolving and dangling targets must remain beneath one
/// of the current authority's private/stable/runtime sibling roots. Real
/// directories, arbitrary links, and name-mismatched suite links fail closed.
pub(super) fn retired_suite_thin_index_is_safe(
    ctx: &ConsoleContext,
    entry: &Path,
    name: &str,
) -> bool {
    if !is_safe_path_component(name)
        || !retired_suite_skill_names(&ctx.repo_root)
            .iter()
            .any(|retired| retired == name)
        || std::fs::symlink_metadata(entry)
            .map(|meta| !meta.file_type().is_symlink())
            .unwrap_or(true)
    {
        return false;
    }
    let Ok(raw_target) = std::fs::read_link(entry) else {
        return false;
    };
    let target = if raw_target.is_absolute() {
        raw_target
    } else {
        let Some(parent) = entry.parent() else {
            return false;
        };
        parent.join(raw_target)
    };
    let Some(target) = normalized_absolute_path(&target) else {
        return false;
    };

    sibling_ags_suite_roots(&ctx.repo_root)
        .into_iter()
        .any(|root| {
            let Some(root) = normalized_absolute_path(&root) else {
                return false;
            };
            let Ok(relative) = target.strip_prefix(&root) else {
                return false;
            };
            if !suite_store_target_matches_name(relative, name) {
                return false;
            }
            match (
                std::fs::canonicalize(&root),
                canonicalize_nearest_existing(&target),
            ) {
                (Ok(real_root), Some(real_target_or_ancestor)) => {
                    real_target_or_ancestor.starts_with(real_root)
                }
                _ => false,
            }
        })
}

pub(super) fn plan_retired_suite_thin_index_cleanup(
    ctx: &ConsoleContext,
    planned_writes: &mut Vec<PlannedWrite>,
) {
    let mut roots: Vec<PathBuf> = supported_skill_hosts()
        .into_iter()
        .filter_map(host_skills_subdir)
        .map(|subdir| ctx.home.join(subdir))
        .collect();
    roots.push(ctx.home.join(".agents/skills"));
    roots.sort();
    roots.dedup();

    for name in retired_suite_skill_names(&ctx.repo_root) {
        for root in &roots {
            let entry = root.join(&name);
            if retired_suite_thin_index_is_safe(ctx, &entry, &name) {
                planned_writes.push(PlannedWrite {
                    op: "unlink-retired-suite-thin-index".to_string(),
                    path: entry.display().to_string(),
                    from: None,
                    detail: format!(
                        "remove proven retired suite thin index ({name}); canonical body and non-suite entries untouched"
                    ),
                });
            }
        }
    }
}

/// Plan the one-time shared-index migration for skills that expose internal
/// playbook entrypoints through one host-visible parent. The registry metadata
/// is the authority: children with `entrypoint.kind=playbook` must not remain
/// standalone host skills. Cleanup is intentionally narrow — only dangling
/// symlinks are unlinked; resolving links and real directories are preserved.
pub(super) fn shared_store_hygiene_plan(
    ctx: &ConsoleContext,
    inventory: &ManagedInventoryResult,
    apply: bool,
) -> SharedStoreHygieneResult {
    let mut parent_playbooks: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for cap in &inventory.capabilities {
        let Some(routing) = cap.routing.as_ref() else {
            continue;
        };
        let (Some(parent), Some(entrypoint)) = (&routing.parent, &routing.entrypoint) else {
            continue;
        };
        if parent.kind == ManagedKind::Skill && entrypoint.kind == EntrypointKind::Playbook {
            parent_playbooks
                .entry(parent.name.clone())
                .or_default()
                .push(cap.name.clone());
        }
    }

    let shared_root = ctx.home.join(".agents/skills");
    let mut planned_writes = Vec::new();
    let mut blocked_reasons = Vec::new();
    plan_retired_suite_thin_index_cleanup(ctx, &mut planned_writes);
    for (parent_name, mut playbooks) in parent_playbooks {
        if !is_safe_path_component(&parent_name) {
            blocked_reasons.push(format!(
                "Unsafe playbook parent name '{parent_name}' — shared migration refused."
            ));
            continue;
        }
        let Some(parent) = inventory
            .capabilities
            .iter()
            .find(|cap| cap.name == parent_name && is_syncable(cap))
        else {
            continue;
        };
        let Some(source) = parent.source.as_deref() else {
            blocked_reasons.push(format!(
                "No canonical source for playbook parent '{parent_name}' — shared migration refused."
            ));
            continue;
        };
        let canonical = resolve_source(&ctx.repo_root, source);
        playbooks.sort();
        playbooks.dedup();
        if !canonical.join("SKILL.md").is_file()
            || !canonical_source_allowed(ctx, parent, &canonical)
        {
            blocked_reasons.push(format!(
                "Canonical parent body for '{parent_name}' is missing or outside its approved store: {}",
                canonical.display()
            ));
            continue;
        }
        let body_issues = playbook_body_issues(&canonical, &playbooks);
        if !body_issues.is_empty() {
            blocked_reasons.push(format!(
                "Canonical parent body for '{parent_name}' has an unsafe playbook exposure shape: {}",
                body_issues.join("; ")
            ));
            continue;
        }

        let parent_entry = shared_root.join(&parent_name);
        if !thin_index_matches_canonical(&parent_entry, &canonical, &parent_name) {
            match std::fs::symlink_metadata(&parent_entry) {
                Ok(meta) if !meta.file_type().is_symlink() => blocked_reasons.push(format!(
                    "Shared parent entry is user-owned/non-symlink; refusing to replace it: {}",
                    parent_entry.display()
                )),
                _ => planned_writes.push(PlannedWrite {
                    op: "relink".to_string(),
                    path: parent_entry.display().to_string(),
                    from: Some(canonical.display().to_string()),
                    detail: format!(
                        "shared parent thin index for internal playbooks ({parent_name})"
                    ),
                }),
            }
        }

        // Codex also loads the shared root. Once the parent is exposed there,
        // an existing host-specific symlink is a duplicate picker entry.
        let codex_entry = ctx.home.join(".codex/skills").join(&parent_name);
        match std::fs::symlink_metadata(&codex_entry) {
            Ok(meta) if !meta.file_type().is_symlink() => blocked_reasons.push(format!(
                "Codex parent entry is user-owned/non-symlink; refusing to replace or remove it: {}",
                codex_entry.display()
            )),
            Ok(_) => planned_writes.push(PlannedWrite {
                op: "unlink".to_string(),
                path: codex_entry.display().to_string(),
                from: None,
                detail: format!(
                    "remove duplicate Codex parent entry after shared migration ({parent_name})"
                ),
            }),
            Err(_) => {}
        }

        for playbook in playbooks {
            if !is_safe_path_component(&playbook) {
                blocked_reasons.push(format!(
                    "Unsafe playbook name '{playbook}' — retired-link cleanup refused."
                ));
                continue;
            }
            let entry = shared_root.join(&playbook);
            let is_symlink = std::fs::symlink_metadata(&entry)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false);
            if is_symlink && !entry.exists() {
                planned_writes.push(PlannedWrite {
                    op: "unlink".to_string(),
                    path: entry.display().to_string(),
                    from: None,
                    detail: format!(
                        "retire dangling standalone playbook entry; use parent '{parent_name}'"
                    ),
                });
            }
        }
    }

    let outcome = guarded_apply(apply && blocked_reasons.is_empty(), &planned_writes, ctx);
    let apply_status = if !apply {
        "dry-run"
    } else if !blocked_reasons.is_empty() {
        "blocked"
    } else if !outcome.errors.is_empty() {
        "failed"
    } else if planned_writes.is_empty() {
        "nothing-to-do"
    } else {
        "applied"
    }
    .to_string();

    SharedStoreHygieneResult {
        apply_requested: apply,
        apply_status,
        planned_writes,
        applied_writes: outcome.applied_writes,
        apply_errors: outcome.errors,
        blocked_reasons,
        note: "Registry-derived thin-index hygiene: create the host-visible parent for internal playbooks, unlink retired dangling child playbooks, and remove retired suite-skill links only when the symlink target is proven to live in the sibling private/stable/runtime canonical stores. Real directories, external or name-mismatched links, and canonical bodies are never removed. Restart the host or open a new task after apply.".to_string(),
    }
}

/// Build (and, with `apply`, perform) the cross-host entry plan for every
/// adopted/governed capability. Builds the inventory once and reuses it.
pub fn sync_plan(ctx: &ConsoleContext, hosts: &[&str], apply: bool) -> CapabilitySyncResult {
    let inventory = build_inventory(ctx, hosts);
    // Shared migration runs first in apply mode. The refreshed inventory then
    // sees the shared parent and naturally suppresses a duplicate Codex entry.
    let shared_store_hygiene = shared_store_hygiene_plan(ctx, &inventory, apply);
    let hygiene_succeeded = shared_store_hygiene.blocked_reasons.is_empty()
        && shared_store_hygiene.apply_errors.is_empty();
    let refreshed_inventory;
    let inventory_for_items = if apply && hygiene_succeeded {
        refreshed_inventory = build_inventory(ctx, hosts);
        &refreshed_inventory
    } else {
        &inventory
    };
    let mut items: Vec<ConsoleProposalResult> = Vec::new();
    for cap in &inventory_for_items.capabilities {
        if is_syncable(cap) {
            let mut item = propose_action_inner(
                ctx,
                inventory_for_items,
                ConsoleAction::Adopt,
                &cap.name,
                apply && hygiene_succeeded,
            );
            // Dry-run cannot materialize the shared parent before planning the
            // capability phase. Remove the redundant Codex relink that would be
            // suppressed after the planned shared relink actually runs.
            if !apply
                && shared_store_hygiene.planned_writes.iter().any(|write| {
                    write.op == "relink"
                        && Path::new(&write.path).file_name()
                            == Some(std::ffi::OsStr::new(&cap.name))
                        && Path::new(&write.path).starts_with(ctx.home.join(".agents/skills"))
                })
            {
                let codex_entry = ctx.home.join(".codex/skills").join(&cap.name);
                item.planned_writes
                    .retain(|write| Path::new(&write.path) != codex_entry);
                item.note.push_str(
                    " Shared parent migration will expose this capability to Codex; the duplicate host-specific Codex write is suppressed.",
                );
            }
            items.push(item);
        }
    }

    let summary = CapabilitySyncSummary {
        considered: items.len(),
        planned_writes: items.iter().map(|i| i.planned_writes.len()).sum::<usize>()
            + shared_store_hygiene.planned_writes.len(),
        applied: items.iter().filter(|i| i.applied).count(),
        advised_only: items
            .iter()
            .filter(|i| i.apply_status == "advised-only" || !i.advised_commands.is_empty())
            .count(),
        blocked: items
            .iter()
            .filter(|i| !i.blocked_reasons.is_empty())
            .count()
            + usize::from(!shared_store_hygiene.blocked_reasons.is_empty()),
        failed: items.iter().filter(|i| !i.apply_errors.is_empty()).count()
            + usize::from(!shared_store_hygiene.apply_errors.is_empty()),
        needs_action: items
            .iter()
            .filter(|i| !i.planned_writes.is_empty() || !i.advised_commands.is_empty())
            .count()
            + usize::from(!shared_store_hygiene.planned_writes.is_empty()),
    };

    CapabilitySyncResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        apply_requested: apply,
        items,
        shared_store_hygiene,
        summary,
        note: if apply {
            "Cross-Agent sync apply: AGS-owned skill thin-index writes were performed through the single guard; MCP / CLI-backed capabilities are advised-only (AGS ran nothing). Restart each host, then `ags capability verify --host <host>`.".to_string()
        } else {
            "Cross-Agent sync plan (dry-run): nothing written, no external command run. Re-run with `--apply` to write AGS-owned skill thin-index entries; MCP / CLI registration is always advised, never run by AGS.".to_string()
        },
    }
}

/// Render the sync result as JSON.
pub fn render_sync_json(result: &CapabilitySyncResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render the sync result as compact human-readable text (one line per
/// capability + summary). Full per-capability detail is available via
/// `ags capability install --capability <name>`.
pub fn render_sync_text(result: &CapabilitySyncResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Cross-Agent Capability Sync".to_string());
    lines.push("===========================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Hosts:   {}", result.hosts.join(", ")));
    lines.push(format!(
        "Mode:    {}",
        if result.apply_requested {
            "apply"
        } else {
            "dry-run"
        }
    ));
    lines.push(format!(
        "Summary: considered {}, needs-action {}, planned-writes {}, applied {}, advised-only {}, blocked {}, failed {}",
        result.summary.considered,
        result.summary.needs_action,
        result.summary.planned_writes,
        result.summary.applied,
        result.summary.advised_only,
        result.summary.blocked,
        result.summary.failed,
    ));
    lines.push(format!(
        "Shared-store hygiene: {} (planned {}, applied {}, blocked {}, errors {})",
        result.shared_store_hygiene.apply_status,
        result.shared_store_hygiene.planned_writes.len(),
        result.shared_store_hygiene.applied_writes.len(),
        result.shared_store_hygiene.blocked_reasons.len(),
        result.shared_store_hygiene.apply_errors.len()
    ));
    for write in &result.shared_store_hygiene.planned_writes {
        lines.push(format!("  {} {}", write.op, write.path));
    }
    lines.push(String::new());
    lines.push("─ Capabilities ─".to_string());
    if result.items.is_empty() {
        lines.push("  None syncable (no adopted suite skills or governed MCPs).".to_string());
    } else {
        for item in &result.items {
            lines.push(format!(
                "  [{}] {} ({}) — writes: {}, advised: {}{}",
                item.apply_status,
                item.capability,
                item.kind.as_deref().unwrap_or("?"),
                item.planned_writes.len(),
                item.advised_commands.len(),
                if item.blocked_reasons.is_empty() {
                    String::new()
                } else {
                    format!(", blocked: {}", item.blocked_reasons.len())
                },
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}
