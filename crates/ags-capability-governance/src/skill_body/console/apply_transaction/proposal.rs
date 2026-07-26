use super::*;

/// Propose an action on a named capability. `apply == false` → dry-run (no
/// writes). `apply == true` → guarded apply: only AGS-owned file writes within
/// `ctx` are performed; external installer/registrar commands are only advised,
/// never executed.
pub fn propose_action(
    ctx: &ConsoleContext,
    action: ConsoleAction,
    name: &str,
    apply: bool,
) -> ConsoleProposalResult {
    let inventory = build_inventory(ctx, &supported_skill_hosts());
    propose_action_inner(ctx, &inventory, action, name, apply)
}

/// Plan or apply thin-index links for one machine-private imported skill body.
///
/// The body must already exist under `allowed_body_root`; this function never
/// downloads or copies it. Destinations are limited to the explicit supported
/// hosts and all writes still pass through the console's single transactional
/// thin-index guard.
pub fn distribute_external_skill(
    ctx: &ConsoleContext,
    name: &str,
    canonical: &Path,
    allowed_body_root: &Path,
    hosts: &[String],
    apply: bool,
) -> ConsoleProposalResult {
    let mut result = ConsoleProposalResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        action: "adopt".to_string(),
        capability: name.to_string(),
        found: true,
        kind: Some("skill".to_string()),
        managed_status: Some("governed".to_string()),
        apply_requested: apply,
        ..Default::default()
    };
    if !is_safe_path_component(name) {
        result.blocked_reasons.push("invalid_skill_id".to_string());
    }
    let real_body_root = std::fs::canonicalize(allowed_body_root).ok();
    let real_canonical = std::fs::canonicalize(canonical).ok();
    if real_body_root
        .as_ref()
        .zip(real_canonical.as_ref())
        .is_none_or(|(root, body)| !body.starts_with(root))
    {
        result.blocked_reasons.push(format!(
            "canonical body {} is outside the machine-private skill body store",
            canonical.display()
        ));
    }
    let skill_md = canonical.join("SKILL.md");
    let declared_name = std::fs::read_to_string(&skill_md)
        .ok()
        .and_then(|text| crate::skill_body::parse_front_matter(&text).0);
    if declared_name.as_deref().map(str::trim) != Some(name) {
        result.blocked_reasons.push(format!(
            "canonical SKILL.md name mismatch at {}",
            skill_md.display()
        ));
    }

    let mut writes = Vec::new();
    let mut seen = HashSet::new();
    for host in hosts {
        if !seen.insert(host.clone()) {
            continue;
        }
        let Some(subdir) = host_skills_subdir(host) else {
            result
                .blocked_reasons
                .push(format!("unsupported skill host: {host}"));
            continue;
        };
        let shared_entries = shared_skill_dirs_for_host(ctx, host)
            .into_iter()
            .map(|root| root.join(name))
            .filter(|entry| entry.exists() || std::fs::symlink_metadata(entry).is_ok())
            .collect::<Vec<_>>();
        if let Some(entry) = shared_entries
            .iter()
            .find(|entry| !thin_index_matches_canonical(entry, canonical, name))
        {
            result.blocked_reasons.push(format!(
                "canonical name collision at shared skill entry: {}",
                entry.display()
            ));
            continue;
        }
        let entry = ctx.home.join(subdir).join(name);
        let entry_exists = entry.exists() || std::fs::symlink_metadata(&entry).is_ok();
        if entry_exists && !thin_index_matches_canonical(&entry, canonical, name) {
            result.blocked_reasons.push(format!(
                "canonical name collision at host skill entry: {}",
                entry.display()
            ));
            continue;
        }
        if entry_exists && !shared_entries.is_empty() {
            result.blocked_reasons.push(format!(
                "duplicate canonical skill entries for host '{host}': {} and {}",
                entry.display(),
                shared_entries[0].display()
            ));
            continue;
        }
        if entry_exists || !shared_entries.is_empty() {
            continue;
        }
        writes.push(PlannedWrite {
            op: "relink".to_string(),
            path: entry.display().to_string(),
            from: Some(canonical.display().to_string()),
            detail: format!("[{host}] thin index → imported canonical skill body"),
        });
    }
    result.planned_writes = writes.clone();
    let confirmed = apply && result.blocked_reasons.is_empty();
    let outcome = guarded_apply(confirmed, &writes, ctx);
    result.applied_writes = outcome.applied_writes;
    result.apply_errors = outcome.errors;
    result.applied = confirmed && !writes.is_empty() && result.apply_errors.is_empty();
    result.apply_status = if !apply {
        "dry-run"
    } else if !result.blocked_reasons.is_empty() {
        "blocked"
    } else if !result.apply_errors.is_empty() {
        "failed"
    } else if result.applied {
        "applied"
    } else {
        "nothing-to-do"
    }
    .to_string();
    result.note = if apply {
        "Machine-private imported body retained; host thin-index transaction completed.".to_string()
    } else {
        dry_run_note()
    };
    result
}

/// Remove only thin-index links that still resolve to the expected imported
/// canonical body. This is the compensation path for a larger adoption
/// transaction; an unrelated same-name host entry is never removed.
pub fn remove_external_skill_distribution(
    ctx: &ConsoleContext,
    name: &str,
    canonical: &Path,
    hosts: &[String],
    apply: bool,
) -> ConsoleProposalResult {
    let mut result = ConsoleProposalResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        action: "remove".to_string(),
        capability: name.to_string(),
        found: true,
        kind: Some("skill".to_string()),
        managed_status: Some("governed".to_string()),
        apply_requested: apply,
        ..Default::default()
    };
    if !is_safe_path_component(name) {
        result.blocked_reasons.push("invalid_skill_id".to_string());
    }
    let mut writes = Vec::new();
    let mut seen = HashSet::new();
    for host in hosts {
        if !seen.insert(host.clone()) {
            continue;
        }
        let Some(subdir) = host_skills_subdir(host) else {
            result
                .blocked_reasons
                .push(format!("unsupported skill host: {host}"));
            continue;
        };
        let entry = ctx.home.join(subdir).join(name);
        if !entry.exists() && std::fs::symlink_metadata(&entry).is_err() {
            continue;
        }
        if !thin_index_matches_canonical(&entry, canonical, name) {
            result.blocked_reasons.push(format!(
                "refused to remove unrelated host skill entry: {}",
                entry.display()
            ));
            continue;
        }
        writes.push(PlannedWrite {
            op: "unlink".to_string(),
            path: entry.display().to_string(),
            from: None,
            detail: format!("[{host}] remove imported skill thin index"),
        });
    }
    result.planned_writes = writes.clone();
    let confirmed = apply && result.blocked_reasons.is_empty();
    let outcome = guarded_apply(confirmed, &writes, ctx);
    result.applied_writes = outcome.applied_writes;
    result.apply_errors = outcome.errors;
    result.applied = confirmed && !writes.is_empty() && result.apply_errors.is_empty();
    result.apply_status = if !apply {
        "dry-run"
    } else if !result.blocked_reasons.is_empty() {
        "blocked"
    } else if !result.apply_errors.is_empty() {
        "failed"
    } else if result.applied {
        "applied"
    } else {
        "nothing-to-do"
    }
    .to_string();
    result.note = "Only proven imported-skill thin indexes are eligible for removal.".to_string();
    result
}

/// Plan/apply against a pre-built inventory. Lets batch callers (e.g.
/// [`sync_plan`]) reuse a single inventory instead of rebuilding it — and
/// re-invoking host CLIs — once per capability.
pub(in super::super) fn propose_action_inner(
    ctx: &ConsoleContext,
    inventory: &ManagedInventoryResult,
    action: ConsoleAction,
    name: &str,
    apply: bool,
) -> ConsoleProposalResult {
    let cap = inventory.capabilities.iter().find(|c| c.name == name);

    let mut result = ConsoleProposalResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        action: action.as_str().to_string(),
        capability: name.to_string(),
        apply_requested: apply,
        ..Default::default()
    };

    let Some(cap) = cap else {
        result.found = false;
        result.blocked_reasons.push(format!(
            "Capability '{name}' not found in the managed inventory. Run `ags skill` to list, or place its source under the suite before adopting."
        ));
        result.note = dry_run_note();
        return result;
    };

    result.found = true;
    result.kind = Some(kind_str(&cap.kind).to_string());
    result.managed_status = Some(managed_status_str(&cap.managed_status).to_string());
    result.risk_notes = cap.risk_notes.clone();

    let plan = plan_action(ctx, cap, action);
    result.planned_writes = plan.writes.clone();
    result.advised_commands = plan.advised;
    result.blocked_reasons = plan.blocked;

    // The single mutation guard. No confirmation, or any blocked reason → no writes.
    let confirmed = apply && result.blocked_reasons.is_empty();
    let outcome = guarded_apply(confirmed, &plan.writes, ctx);
    result.applied_writes = outcome.applied_writes;
    result.apply_errors = outcome.errors;
    // `applied` is true only when a write was confirmed, at least one AGS-owned
    // write was planned, AND every one succeeded — NEVER from confirmation
    // alone. Advised-only actions (MCP/CLI) plan no writes ⇒ applied stays false.
    result.applied = confirmed
        && !matches!(action, ConsoleAction::Verify)
        && !result.planned_writes.is_empty()
        && result.apply_errors.is_empty();

    // Distinct apply state so callers never mistake "AGS only advised you to run
    // a command" for "AGS performed the action".
    result.apply_status = if !apply {
        "dry-run"
    } else if !result.blocked_reasons.is_empty() {
        "blocked"
    } else if !result.apply_errors.is_empty() {
        "failed"
    } else if result.applied {
        "applied"
    } else if !result.advised_commands.is_empty() {
        // Confirmed, but the only "action" AGS can offer is advice it never runs.
        "advised-only"
    } else {
        "nothing-to-do"
    }
    .to_string();

    let mut note_lines = plan.notes;
    match result.apply_status.as_str() {
        "dry-run" => note_lines.push(dry_run_note()),
        "blocked" => note_lines.push(
            "Apply was requested but is blocked — see blocked_reasons. Nothing written."
                .to_string(),
        ),
        "failed" => note_lines.push(
            "Apply FAILED — one or more writes errored (see apply_errors); no host was left half-changed (per-host transactional, multi-host preflighted). Resolve, re-run, then `ags skill verify`.".to_string(),
        ),
        "advised-only" => note_lines.push(
            "AGS performed NOTHING — this capability has no AGS-owned host file. Run the advised command(s) yourself, then restart the host. `applied` is false by design.".to_string(),
        ),
        "applied" => note_lines.push("Applied. Restart the host (Claude Code / Codex / OMP / CodeBuddy-Code / Cursor) so it re-scans thin indexes, then run `ags skill verify --host <host>`.".to_string()),
        _ => {}
    }
    result.note = note_lines.join(" ");
    result
}

pub(in super::super) fn dry_run_note() -> String {
    "DRY-RUN — no files written, no external command run. Re-run with `--apply` to confirm. Apply never runs external installers (npx skills add, lark-cli update, claude mcp add/remove).".to_string()
}

pub(in super::super) fn managed_status_str(s: &ManagedStatus) -> &'static str {
    match s {
        ManagedStatus::SuiteManaged => "suite-managed",
        ManagedStatus::Governed => "governed",
        ManagedStatus::SuiteInterface => "suite-interface",
        ManagedStatus::Discovered => "discovered",
        ManagedStatus::HostSystem => "host-system",
        ManagedStatus::ProjectLocal => "project-local",
        ManagedStatus::Ignored => "ignored",
        ManagedStatus::Unmanaged => "unmanaged",
        ManagedStatus::RouteTarget => "route-target",
    }
}
