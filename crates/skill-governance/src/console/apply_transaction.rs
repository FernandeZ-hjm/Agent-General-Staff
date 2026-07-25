use super::*;
#[allow(unused_imports)]
use super::{actions::*, host_probe::*, host_verify::*, inventory::*, model::*, rendering::*};
// ── Proposal / guarded apply ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedWrite {
    /// "create" | "overwrite" | "backup" | "remove"
    pub op: String,
    pub path: String,
    pub from: Option<String>,
    pub detail: String,
}

/// An external command a human must run in their host. AGS NEVER executes these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisedCommand {
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleProposalResult {
    pub schema_version: String,
    pub action: String,
    pub capability: String,
    pub found: bool,
    pub kind: Option<String>,
    pub managed_status: Option<String>,
    pub apply_requested: bool,
    /// True ONLY when ≥1 AGS-owned write was planned AND every one succeeded.
    /// Never true for advised-only (MCP/CLI) actions — AGS performed nothing.
    pub applied: bool,
    /// "dry-run" | "applied" | "failed" | "advised-only" | "nothing-to-do" | "blocked"
    pub apply_status: String,
    pub planned_writes: Vec<PlannedWrite>,
    pub applied_writes: Vec<String>,
    /// Per-write failures during apply. Non-empty ⇒ apply did NOT fully succeed
    /// and `applied` is false; the CLI exits nonzero.
    pub apply_errors: Vec<String>,
    /// External installer/registrar commands AGS will NOT run on your behalf.
    pub advised_commands: Vec<AdvisedCommand>,
    pub blocked_reasons: Vec<String>,
    pub risk_notes: Vec<String>,
    pub note: String,
}

#[derive(Default)]
pub(super) struct ActionPlan {
    writes: Vec<PlannedWrite>,
    advised: Vec<AdvisedCommand>,
    blocked: Vec<String>,
    notes: Vec<String>,
}

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
        .and_then(|text| crate::parse_front_matter(&text).0);
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
pub(super) fn propose_action_inner(
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

pub(super) fn dry_run_note() -> String {
    "DRY-RUN — no files written, no external command run. Re-run with `--apply` to confirm. Apply never runs external installers (npx skills add, lark-cli update, claude mcp add/remove).".to_string()
}

pub(super) fn managed_status_str(s: &ManagedStatus) -> &'static str {
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

pub(super) fn plan_action(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
    action: ConsoleAction,
) -> ActionPlan {
    let mut plan = ActionPlan::default();

    // The AGS host initialization adapter is never mutated through the console.
    if matches!(cap.kind, ManagedKind::SuiteInterface) && !matches!(action, ConsoleAction::Verify) {
        plan.blocked.push(
            "AGS host initialization adapter cannot be adopted/updated/removed via the skill console; it is the governance authority, not a governed object.".to_string(),
        );
        return plan;
    }

    if matches!(action, ConsoleAction::Verify) {
        plan.notes.push(format!(
            "Verify is read-only. Run `ags skill verify --host claude-code` for host-visibility evidence for '{}'.",
            cap.name
        ));
        return plan;
    }

    // Retired capabilities (route_state: retired) keep a registry row for
    // history/dedupe and may still have a canonical body on disk, but they must
    // NEVER be (re)adopted, updated, or repaired into a host — that would
    // resurrect a deliberately retired front-stage entry. `remove`/`uninstall`
    // (cleanup) and `verify` stay available.
    if matches!(
        action,
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair
    ) && cap
        .routing
        .as_ref()
        .is_some_and(|r| r.route_state == RouteState::Retired)
    {
        plan.blocked.push(format!(
            "'{}' is retired (route_state: retired) and cannot be adopted/updated/repaired into a host — it is kept only as a history/compat record. Any underlying CLI/successor remains; `remove`/`uninstall` and `verify` stay available.",
            cap.name
        ));
        return plan;
    }

    match cap.kind {
        ManagedKind::Skill => plan_skill_entry(ctx, cap, action, &mut plan),
        ManagedKind::Mcp | ManagedKind::CliBacked => plan_mcp_or_cli(cap, action, &mut plan),
        ManagedKind::SuiteInterface => {}
    }
    plan
}

/// The supported hosts that load skills from a `~/<subdir>/skills` directory.
pub(super) fn supported_skill_hosts() -> Vec<&'static str> {
    SUPPORTED_HOSTS
        .iter()
        .copied()
        .filter(|h| host_skills_subdir(h).is_some())
        .collect()
}

/// Does the host thin index at `entry` need (re)creating? True if absent, a
/// dangling symlink, missing SKILL.md, or a front-matter name mismatch.
pub(super) fn thin_index_needs_repair(entry: &Path, name: &str) -> bool {
    // Dangling symlink → broken.
    if std::fs::symlink_metadata(entry)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
        && std::fs::metadata(entry).is_err()
    {
        return true;
    }
    let skill_md = entry.join("SKILL.md");
    if !skill_md.is_file() {
        return true;
    }
    match std::fs::read_to_string(&skill_md) {
        Ok(text) => crate::parse_front_matter(&text).0.as_deref().map(str::trim) != Some(name),
        Err(_) => true,
    }
}

pub(super) fn thin_index_matches_canonical(entry: &Path, canonical: &Path, name: &str) -> bool {
    if std::fs::symlink_metadata(entry)
        .map(|meta| !meta.file_type().is_symlink())
        .unwrap_or(true)
        || thin_index_needs_repair(entry, name)
    {
        return false;
    }
    match (
        std::fs::canonicalize(entry),
        std::fs::canonicalize(canonical),
    ) {
        (Ok(real_entry), Ok(real_canonical)) => {
            thin_index_target_match(&real_entry, &real_canonical).is_some()
        }
        _ => false,
    }
}

pub(super) fn shared_skill_entry_for_plan(
    ctx: &ConsoleContext,
    host: &str,
    name: &str,
    canonical: &Path,
) -> Result<Option<PathBuf>, PathBuf> {
    let shared_agents_root = ctx.home.join(".agents/skills");
    for dir in shared_skill_dirs_for_host(ctx, host) {
        let entry = dir.join(name);
        if std::fs::symlink_metadata(&entry).is_err() || thin_index_needs_repair(&entry, name) {
            continue;
        }
        // The multi-agent shared root participates in the same canonical
        // identity contract as native thin indexes. An unrelated same-name
        // body must fail closed instead of suppressing the official body.
        if dir == shared_agents_root {
            let exact = std::fs::canonicalize(&entry)
                .ok()
                .zip(std::fs::canonicalize(canonical).ok())
                .is_some_and(|(actual, expected)| {
                    thin_index_target_match(&actual, &expected).is_some()
                });
            return if exact { Ok(Some(entry)) } else { Err(entry) };
        }
        // Codex plugin sources are manager-owned runtime providers rather than
        // AGS thin indexes. Preserve their established de-duplication behavior.
        return Ok(Some(entry));
    }
    Ok(None)
}

/// Plan per-host thin-index distribution. The declared owner keeps ONE
/// canonical skill body; each host that lacks shared discovery gets a symlink
/// at `<host>/skills/<name>`. `remove`/`uninstall` touch only the thin index;
/// the canonical body is never touched here. `verify` plans nothing.
pub(super) fn plan_skill_entry(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
    action: ConsoleAction,
    plan: &mut ActionPlan,
) {
    // Hard boundary: the capability name becomes a path component under each
    // host's skills dir. Reject `/`, `\`, `..`, absolute, and multi-component
    // names BEFORE planning any write so a corrupt/hostile name can never make
    // a write target escape the skills directory.
    if !is_safe_path_component(&cap.name) {
        plan.blocked.push(format!(
            "Unsafe capability name '{}' — refusing to plan a thin-index write (path traversal / separator / absolute path not allowed).",
            cap.name
        ));
        return;
    }

    // Resolve and validate the canonical body for actions that link to it. We
    // refuse to create a dangling thin index.
    let canonical = cap
        .source
        .as_ref()
        .map(|s| resolve_source(&ctx.repo_root, s));
    let canonical = if matches!(
        action,
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair
    ) {
        let Some(dir) = canonical else {
            plan.blocked.push(format!(
                "No canonical source path known for '{}'; cannot create a thin index.",
                cap.name
            ));
            return;
        };
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            plan.blocked.push(format!(
                "Canonical SKILL.md not found at {} — refusing to create a dangling thin index.",
                skill_md.display()
            ));
            return;
        }
        // Containment follows declared ownership: suite bodies stay inside the
        // repository stores; external bodies stay inside the shared skill root.
        if !canonical_source_allowed(ctx, cap, &dir) {
            plan.blocked.push(format!(
                "Canonical source {} is outside the store approved for '{}' — refusing to link a host to it.",
                dir.display(),
                cap.name
            ));
            return;
        }
        // The canonical body must declare the capability we think we're linking.
        match std::fs::read_to_string(&skill_md)
            .ok()
            .and_then(|t| crate::parse_front_matter(&t).0)
            .as_deref()
            .map(str::trim)
        {
            Some(n) if n == cap.name => {}
            other => {
                plan.blocked.push(format!(
                    "Canonical SKILL.md at {} declares name {:?}, not '{}' — refusing to mislabel a host entry.",
                    skill_md.display(),
                    other,
                    cap.name
                ));
                return;
            }
        }
        Some(dir)
    } else {
        canonical
    };

    // Distribute / update / remove the thin index on EVERY supported skill host,
    // so one restart makes the skill discoverable on all platforms. Each host is
    // ONE op (`relink` / `unlink`); guarded_apply executes it transactionally
    // and preflights every host before mutating any.
    for host in supported_skill_hosts() {
        let subdir = host_skills_subdir(host).expect("supported host has a skills subdir");
        let entry = ctx.home.join(subdir).join(&cap.name);
        let entry_str = entry.display().to_string();
        let present = std::fs::symlink_metadata(&entry).is_ok();

        match action {
            ConsoleAction::Adopt | ConsoleAction::Update => {
                match shared_skill_entry_for_plan(ctx, host, &cap.name, canonical.as_ref().unwrap())
                {
                    Ok(Some(shared_entry)) => {
                        if present
                            && !thin_index_matches_canonical(
                                &entry,
                                canonical.as_ref().unwrap(),
                                &cap.name,
                            )
                        {
                            plan.blocked.push(format!(
                                "[{host}] native skill entry {} conflicts with the exact shared canonical at {}; refusing ambiguous same-name bodies.",
                                entry.display(),
                                shared_entry.display()
                            ));
                        } else if present {
                            plan.writes.push(PlannedWrite {
                                op: "unlink".to_string(),
                                path: entry_str.clone(),
                                from: None,
                                detail: format!(
                                    "[{host}] remove redundant native thin index; exact shared canonical remains visible at {}",
                                    shared_entry.display()
                                ),
                            });
                        } else {
                            plan.notes.push(format!(
                                "[{host}] shared skill source already visible at {}; skip {} to avoid duplicate picker entries.",
                                shared_entry.display(),
                                entry_str
                            ));
                        }
                        continue;
                    }
                    Err(shared_entry) => {
                        plan.blocked.push(format!(
                            "[{host}] shared skill entry {} does not resolve to the canonical body for '{}'; refusing to hide the official capability behind an unrelated same-name body.",
                            shared_entry.display(),
                            cap.name
                        ));
                        continue;
                    }
                    Ok(None) => {}
                }
                if thin_index_matches_canonical(&entry, canonical.as_ref().unwrap(), &cap.name) {
                    plan.notes.push(format!(
                        "[{host}] thin index already resolves to the canonical body at {entry_str}; nothing to change."
                    ));
                    continue;
                }
                plan.writes.push(PlannedWrite {
                    op: "relink".to_string(),
                    path: entry_str.clone(),
                    from: Some(canonical.as_ref().unwrap().display().to_string()),
                    detail: format!(
                        "[{host}] thin index → canonical skill dir (transactional; existing entry replaced without .bak clutter; references travel with it)"
                    ),
                });
            }
            ConsoleAction::Remove | ConsoleAction::Uninstall => {
                if present {
                    plan.writes.push(PlannedWrite {
                        op: "unlink".to_string(),
                        path: entry_str.clone(),
                        from: None,
                        detail: format!(
                            "[{host}] remove thin index (moved to .bak); canonical body untouched"
                        ),
                    });
                } else {
                    plan.notes.push(format!(
                        "[{host}] no thin index at {entry_str}; nothing to remove."
                    ));
                }
            }
            ConsoleAction::Repair => {
                match shared_skill_entry_for_plan(ctx, host, &cap.name, canonical.as_ref().unwrap())
                {
                    Ok(Some(shared_entry)) => {
                        if present
                            && !thin_index_matches_canonical(
                                &entry,
                                canonical.as_ref().unwrap(),
                                &cap.name,
                            )
                        {
                            plan.blocked.push(format!(
                                "[{host}] native skill entry {} conflicts with the exact shared canonical at {}; refusing ambiguous repair.",
                                entry.display(),
                                shared_entry.display()
                            ));
                        } else if present {
                            plan.writes.push(PlannedWrite {
                                op: "unlink".to_string(),
                                path: entry_str.clone(),
                                from: None,
                                detail: format!(
                                    "[{host}] remove redundant native thin index; exact shared canonical remains visible at {}",
                                    shared_entry.display()
                                ),
                            });
                        } else {
                            plan.notes.push(format!(
                                "[{host}] shared skill source already visible at {}; no host-specific repair needed.",
                                shared_entry.display()
                            ));
                        }
                        continue;
                    }
                    Err(shared_entry) => {
                        plan.blocked.push(format!(
                            "[{host}] shared skill entry {} does not resolve to the canonical body for '{}'; refusing to repair around an unrelated same-name body.",
                            shared_entry.display(),
                            cap.name
                        ));
                        continue;
                    }
                    Ok(None) => {}
                }
                if !thin_index_matches_canonical(&entry, canonical.as_ref().unwrap(), &cap.name) {
                    plan.writes.push(PlannedWrite {
                        op: "relink".to_string(),
                        path: entry_str.clone(),
                        from: Some(canonical.as_ref().unwrap().display().to_string()),
                        detail: format!(
                            "[{host}] recreate broken/missing thin index (transactional)"
                        ),
                    });
                } else {
                    plan.notes.push(format!(
                        "[{host}] thin index present and loadable; nothing to repair."
                    ));
                }
            }
            ConsoleAction::Verify => {}
        }
    }

    // External-CLI advisories (AGS never runs these).
    match action {
        ConsoleAction::Adopt | ConsoleAction::Update => {
            if let Some(fam) = cli_family_for_skill(&cap.name) {
                plan.advised.push(AdvisedCommand {
                    command: format!("{} update", fam.cli),
                    reason: format!(
                        "'{}' is fronted by {}; refresh the CLI yourself — AGS never runs it.",
                        cap.name, fam.cli
                    ),
                });
            }
        }
        ConsoleAction::Uninstall => {
            plan.advised.push(AdvisedCommand {
                command: format!("npx skills remove {} -g", cap.name),
                reason: "Remove the underlying skill body from the AGS canonical store yourself — AGS never runs external installers.".to_string(),
            });
        }
        _ => {}
    }

    if !matches!(action, ConsoleAction::Verify) {
        plan.notes.push(
            "Restart the host(s) after adopt/update so they re-scan thin indexes.".to_string(),
        );
    }
}

/// Plan MCP / CLI-backed actions. AGS owns no file here, so it only *advises*
/// the external registrar/installer command and never executes it.
pub(super) fn plan_mcp_or_cli(
    cap: &ManagedCapability,
    action: ConsoleAction,
    plan: &mut ActionPlan,
) {
    let name = &cap.name;
    match action {
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair => {
            if matches!(cap.kind, ManagedKind::CliBacked)
                && matches!(cap.managed_status, ManagedStatus::Unmanaged)
            {
                plan.advised.push(AdvisedCommand {
                    command: format!("{name} update"),
                    reason: "External official CLI — update it yourself. AGS never runs it."
                        .to_string(),
                });
            } else {
                // Cross-Agent host command plan: AGS advises the registration
                // command for each directly configurable host (Claude Code,
                // Codex). OMP inherits existing host configs; Cursor MCP
                // registration remains host-dependent. AGS never runs any of
                // these.
                plan.advised.push(AdvisedCommand {
                    command: format!("claude mcp add {name} -- <command> [args...]"),
                    reason: "Claude Code: AGS records MCP governance but never registers MCP servers in host config; run this in Claude Code, then restart it.".to_string(),
                });
                plan.advised.push(AdvisedCommand {
                    command: format!("codex mcp add {name} -- <command> [args...]"),
                    reason: "Codex: AGS records MCP governance but never registers MCP servers in host config; run this in Codex, then restart it.".to_string(),
                });
            }
        }
        ConsoleAction::Remove | ConsoleAction::Uninstall => {
            plan.advised.push(AdvisedCommand {
                command: format!("claude mcp remove {name}"),
                reason: "Claude Code: AGS never unregisters MCP servers from host config; run this yourself.".to_string(),
            });
            plan.advised.push(AdvisedCommand {
                command: format!("codex mcp remove {name}"),
                reason:
                    "Codex: AGS never unregisters MCP servers from host config; run this yourself."
                        .to_string(),
            });
        }
        ConsoleAction::Verify => {}
    }
    plan.notes.push(
        "MCP / CLI-backed capabilities have no AGS-owned host file; AGS advises the directly configurable host commands (Claude Code, Codex) but never runs them. OMP inherits existing host configuration and gets no duplicate registrar command.".to_string(),
    );
}

/// Resolve a suite source path; absolute paths are used as-is.
pub(super) fn resolve_source(repo_root: &Path, source: &str) -> PathBuf {
    let p = Path::new(source);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

/// The approved canonical skill stores under the repo. A symlink target must
/// live inside one of these — never an arbitrary local directory.
pub(super) const CANONICAL_STORES: &[&str] = &["global-skills", "skill-packs"];

/// True iff `canonical_dir` (canonicalized) lives inside an approved store.
/// Defends against bad/stale manifest sources (absolute paths, `..` escapes,
/// targets outside the repo) being symlinked as host-loadable skill bodies.
pub(super) fn canonical_within_store(repo_root: &Path, canonical_dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(canonical_dir) else {
        return false;
    };
    CANONICAL_STORES.iter().any(|store| {
        std::fs::canonicalize(repo_root.join(store))
            .map(|root| real.starts_with(&root))
            .unwrap_or(false)
    })
}

pub(super) fn canonical_within_shared_store(home: &Path, name: &str, canonical_dir: &Path) -> bool {
    if !is_safe_path_component(name) {
        return false;
    }
    let shared_root = home.join(".agents/skills");
    let expected = shared_root.join(name);
    match (
        std::fs::canonicalize(canonical_dir),
        std::fs::canonicalize(expected),
        std::fs::canonicalize(shared_root),
    ) {
        (Ok(actual), Ok(expected), Ok(root)) => actual == expected && actual.starts_with(root),
        _ => false,
    }
}

pub(super) fn is_external_shared_skill(ctx: &ConsoleContext, cap: &ManagedCapability) -> bool {
    let expected = ctx.home.join(".agents/skills").join(&cap.name);
    matches!(cap.kind, ManagedKind::Skill)
        && matches!(cap.managed_status, ManagedStatus::Governed)
        && cap.source.as_deref().map(Path::new) == Some(expected.as_path())
}

/// Accept a skill body only from the store declared by its owner. Suite-owned
/// skills stay confined to the repository stores; registry-governed external
/// skills stay confined to the shared multi-agent store under the injected
/// home. No arbitrary absolute manifest source becomes host-loadable.
pub(super) fn canonical_source_allowed(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
    canonical_dir: &Path,
) -> bool {
    if is_external_shared_skill(ctx, cap) {
        canonical_within_shared_store(&ctx.home, &cap.name, canonical_dir)
    } else {
        canonical_within_store(&ctx.repo_root, canonical_dir)
    }
}

/// Pick a non-clobbering backup path: `<dest>.bak`, then `.bak.1`, `.bak.2`, …
pub(super) fn next_backup_path(dest: &Path) -> PathBuf {
    let base = format!("{}.bak", dest.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 1;
    while std::fs::symlink_metadata(&candidate).is_ok() {
        candidate = PathBuf::from(format!("{base}.{i}"));
        i += 1;
    }
    candidate
}

/// Pick a non-clobbering temporary rollback path used only during thin-index
/// relink apply. Successful applies remove it before returning.
pub(super) fn next_replaced_path(dest: &Path) -> PathBuf {
    let base = format!("{}.ags-replaced", dest.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 1;
    while std::fs::symlink_metadata(&candidate).is_ok() {
        candidate = PathBuf::from(format!("{base}.{i}"));
        i += 1;
    }
    candidate
}

/// Outcome of a guarded apply: writes that succeeded, and per-write errors.
/// Errors are kept separate from `applied_writes` so the caller has a real
/// failure signal (rather than `ERROR ...` buried in the success list).
#[derive(Default)]
pub(super) struct ApplyOutcome {
    pub(super) applied_writes: Vec<String>,
    pub(super) errors: Vec<String>,
}

#[derive(Debug)]
pub(super) enum AppliedChange {
    CreatedDir(PathBuf),
    Relink {
        entry: PathBuf,
        previous: Option<PathBuf>,
    },
    Unlink {
        entry: PathBuf,
        backup: PathBuf,
    },
}

/// True iff `name` is a single, safe path component: not empty, not `.`/`..`,
/// no separators or NUL, and exactly one normal component. Keeps host-entry
/// writes from escaping the skills directory.
pub(super) fn is_safe_path_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(std::path::Component::Normal(c)), None) if c == std::ffi::OsStr::new(name)
    )
}

/// Lexical containment: `path` is under `root` and contains no `..` escapes.
pub(super) fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Create a symlink `link` → `target` (a directory) on the host's behalf.
/// Cross-platform; errors cleanly (→ apply error) where symlinks are
/// unsupported, rather than writing an unusable entry.
#[cfg(unix)]
pub(super) fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
pub(super) fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
#[cfg(not(any(unix, windows)))]
pub(super) fn make_symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "thin-index symlink not supported on this platform",
    ))
}

/// Remove a host entry (symlink or real dir). A missing path is success.
pub(super) fn remove_host_entry(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(path),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// A scratch sibling path for staging a symlink before the atomic swap.
pub(super) fn staging_path(entry: &Path) -> PathBuf {
    PathBuf::from(format!("{}.ags-tmp", entry.display()))
}

/// Read-only parent validation for preflight. This never creates directories.
pub(super) fn validate_parent_path(parent: &Path) -> std::io::Result<()> {
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if path.is_dir() {
                return Ok(());
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists but is not a directory", path.display()),
            ));
        }
        current = path.parent();
    }
    Ok(())
}

/// Create missing parent directories during execution and record each one so a
/// later batch failure can roll them back. Preflight remains read-only.
pub(super) fn ensure_parent_dirs(
    parent: &Path,
    changes: &mut Vec<AppliedChange>,
) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if !path.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} exists but is not a directory", path.display()),
                ));
            }
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    for dir in missing.iter().rev() {
        std::fs::create_dir(dir)?;
        changes.push(AppliedChange::CreatedDir(dir.clone()));
    }
    Ok(())
}

/// Transactionally install a thin-index symlink at `entry` → `canonical`.
/// Existing entries are moved to a temporary rollback sibling during the batch,
/// then removed after the whole apply succeeds. No `.bak` host clutter is left.
/// On **any** failure before success cleanup, the original entry is restored.
pub(super) fn transactional_relink(
    entry: &Path,
    canonical: &Path,
) -> std::io::Result<(String, AppliedChange)> {
    let tmp = staging_path(entry);
    // 1. Stage the new symlink first. If this fails, nothing has moved.
    let _ = remove_host_entry(&tmp);
    make_symlink(canonical, &tmp)?;
    // 2. Move any existing entry to a temporary rollback path.
    let previous = if std::fs::symlink_metadata(entry).is_ok() {
        let old = next_replaced_path(entry);
        if let Err(e) = std::fs::rename(entry, &old) {
            let _ = remove_host_entry(&tmp);
            return Err(e);
        }
        Some(old)
    } else {
        None
    };
    // 3. Swap the staged link into place. On failure, roll the previous entry back.
    if let Err(e) = std::fs::rename(&tmp, entry) {
        if let Some(old) = &previous {
            let _ = std::fs::rename(old, entry);
        }
        let _ = remove_host_entry(&tmp);
        return Err(e);
    }
    let msg = match &previous {
        Some(_) => format!(
            "relink {} -> {} (old entry replaced; no .bak kept)",
            entry.display(),
            canonical.display()
        ),
        None => format!("relink {} -> {}", entry.display(), canonical.display()),
    };
    Ok((
        msg,
        AppliedChange::Relink {
            entry: entry.to_path_buf(),
            previous,
        },
    ))
}

/// Move an existing thin index to a temporary rollback sibling. Missing entry
/// is a no-op; successful batches remove the rollback sibling before returning.
pub(super) fn transactional_unlink(
    entry: &Path,
) -> std::io::Result<Option<(String, AppliedChange)>> {
    if std::fs::symlink_metadata(entry).is_err() {
        return Ok(None);
    }
    let bak = next_backup_path(entry);
    std::fs::rename(entry, &bak)?;
    Ok(Some((
        format!("unlinked {} (no .bak kept)", entry.display()),
        AppliedChange::Unlink {
            entry: entry.to_path_buf(),
            backup: bak,
        },
    )))
}

pub(super) fn rollback_change(change: &AppliedChange) -> std::io::Result<()> {
    match change {
        AppliedChange::CreatedDir(path) => match std::fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        AppliedChange::Relink { entry, previous } => {
            remove_host_entry(entry)?;
            if let Some(old) = previous {
                if std::fs::symlink_metadata(old).is_ok() {
                    std::fs::rename(old, entry)?;
                }
            }
            Ok(())
        }
        AppliedChange::Unlink { entry, backup } => {
            if std::fs::symlink_metadata(backup).is_ok() {
                if std::fs::symlink_metadata(entry).is_ok() {
                    remove_host_entry(entry)?;
                }
                std::fs::rename(backup, entry)?;
            }
            Ok(())
        }
    }
}

pub(super) fn rollback_changes(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes.iter().rev() {
        if let Err(e) = rollback_change(change) {
            errors.push(format!("rollback {:?}: {e}", change));
        }
    }
    errors
}

pub(super) fn cleanup_successful_changes(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes {
        match change {
            AppliedChange::Relink {
                previous: Some(old),
                ..
            } => {
                if let Err(e) = remove_host_entry(old) {
                    errors.push(format!("cleanup replaced entry {}: {e}", old.display()));
                }
            }
            AppliedChange::Unlink { backup, .. } => {
                if let Err(e) = remove_host_entry(backup) {
                    errors.push(format!(
                        "cleanup unlinked-entry backup {}: {e}",
                        backup.display()
                    ));
                }
            }
            _ => {}
        }
    }
    errors
}

/// The single mutation gate. Returns which writes succeeded and which errored.
///
/// When `confirmed` is false it performs **no** filesystem writes. It first
/// PREFLIGHTS every planned write (containment + host skills dir creatable); if
/// any host fails preflight, NOTHING is mutated — a later host's failure can
/// never leave an earlier host half-changed. Each `relink`/`unlink` then runs
/// transactionally (stage → temporary rollback path → atomic swap). The batch also keeps a
/// rollback stack, so a later host failure restores earlier hosts and removes
/// directories created during this apply. Only thin-index ops run; no skill body
/// is copied; no external command is executed.
pub(super) fn guarded_apply(
    confirmed: bool,
    planned: &[PlannedWrite],
    ctx: &ConsoleContext,
) -> ApplyOutcome {
    let mut outcome = ApplyOutcome::default();
    if !confirmed {
        return outcome;
    }
    let mut allowed_roots: Vec<PathBuf> = supported_skill_hosts()
        .iter()
        .filter_map(|h| host_skills_subdir(h).map(|s| ctx.home.join(s)))
        .collect();
    allowed_roots.push(ctx.home.join(".agents/skills"));

    // ── Preflight: validate ALL destinations before mutating ANY ──
    let mut preflight_errors: Vec<String> = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        if !allowed_roots.iter().any(|r| within(path, r)) {
            preflight_errors.push(format!(
                "refused: write target escapes the host skill roots: {}",
                w.path
            ));
            continue;
        }
        match w.op.as_str() {
            "relink" => {
                if w.from.is_none() {
                    preflight_errors.push(format!("relink {}: no canonical target", w.path));
                }
                if let Some(target) = w.from.as_deref() {
                    let target = Path::new(target);
                    let skill_md = target.join("SKILL.md");
                    let expected_name = path.file_name().and_then(|name| name.to_str());
                    let declared_name = std::fs::read_to_string(&skill_md)
                        .ok()
                        .and_then(|text| crate::parse_front_matter(&text).0);
                    if std::fs::canonicalize(target).is_err()
                        || expected_name.is_none()
                        || declared_name.as_deref().map(str::trim) != expected_name
                    {
                        preflight_errors.push(format!(
                            "relink {}: canonical target is missing or declares a different skill name",
                            w.path
                        ));
                    }
                }
                if let Some(parent) = path.parent() {
                    if let Err(e) = validate_parent_path(parent) {
                        preflight_errors.push(format!(
                            "relink {}: host skills dir not creatable: {e}",
                            w.path
                        ));
                    }
                } else {
                    preflight_errors.push(format!("relink {}: no parent directory", w.path));
                }
            }
            "unlink" => {}
            "unlink-retired-suite-thin-index" => {
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    preflight_errors.push(format!(
                        "retired thin-index cleanup {}: missing safe skill name",
                        w.path
                    ));
                    continue;
                };
                if !retired_suite_thin_index_is_safe(ctx, path, name) {
                    preflight_errors.push(format!(
                        "retired thin-index cleanup {}: entry is no longer a proven AGS suite symlink",
                        w.path
                    ));
                }
            }
            other => preflight_errors.push(format!("unknown op '{other}' for {}", w.path)),
        }
    }
    if !preflight_errors.is_empty() {
        // Abort with zero mutation so no host is left half-changed.
        outcome.errors = preflight_errors;
        return outcome;
    }

    // ── Execute: each op is transactional; the batch rolls back on first error ──
    let mut changes = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        match w.op.as_str() {
            "relink" => {
                let target = w.from.as_ref().expect("preflight guaranteed a target");
                if let Some(parent) = path.parent() {
                    if let Err(e) = ensure_parent_dirs(parent, &mut changes) {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
                match transactional_relink(path, Path::new(target)) {
                    Ok((msg, change)) => {
                        outcome.applied_writes.push(msg);
                        changes.push(change);
                    }
                    Err(e) => {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
            }
            "unlink" => match transactional_unlink(path) {
                Ok(Some((msg, change))) => {
                    outcome.applied_writes.push(msg);
                    changes.push(change);
                }
                Ok(None) => {}
                Err(e) => {
                    outcome.errors.push(format!("unlink {}: {e}", w.path));
                    outcome.errors.extend(rollback_changes(&changes));
                    outcome.applied_writes.clear();
                    return outcome;
                }
            },
            "unlink-retired-suite-thin-index" => {
                let safe = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| retired_suite_thin_index_is_safe(ctx, path, name));
                if !safe {
                    outcome.errors.push(format!(
                        "retired thin-index cleanup {}: safety proof changed before unlink",
                        w.path
                    ));
                    outcome.errors.extend(rollback_changes(&changes));
                    outcome.applied_writes.clear();
                    return outcome;
                }
                match transactional_unlink(path) {
                    Ok(Some((msg, change))) => {
                        outcome.applied_writes.push(msg);
                        changes.push(change);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        outcome
                            .errors
                            .push(format!("retired thin-index cleanup {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
            }
            _ => {} // unknown ops already rejected in preflight
        }
    }
    let cleanup_errors = cleanup_successful_changes(&changes);
    if !cleanup_errors.is_empty() {
        outcome.errors = cleanup_errors;
        outcome.applied_writes.clear();
        return outcome;
    }
    outcome
}
