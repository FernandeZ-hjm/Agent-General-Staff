use super::*;

pub(in super::super) fn plan_action(
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
pub(in super::super) fn supported_skill_hosts() -> Vec<&'static str> {
    SUPPORTED_HOSTS
        .iter()
        .copied()
        .filter(|h| host_skills_subdir(h).is_some())
        .collect()
}

/// Does the host thin index at `entry` need (re)creating? True if absent, a
/// dangling symlink, missing SKILL.md, or a front-matter name mismatch.
pub(in super::super) fn thin_index_needs_repair(entry: &Path, name: &str) -> bool {
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
        Ok(text) => {
            crate::skill_body::parse_front_matter(&text)
                .0
                .as_deref()
                .map(str::trim)
                != Some(name)
        }
        Err(_) => true,
    }
}

pub(in super::super) fn thin_index_matches_canonical(
    entry: &Path,
    canonical: &Path,
    name: &str,
) -> bool {
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

pub(in super::super) fn shared_skill_entry_for_plan(
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
pub(in super::super) fn plan_skill_entry(
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
            .and_then(|t| crate::skill_body::parse_front_matter(&t).0)
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
pub(in super::super) fn plan_mcp_or_cli(
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
pub(in super::super) fn resolve_source(repo_root: &Path, source: &str) -> PathBuf {
    let p = Path::new(source);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

/// The approved canonical skill stores under the repo. A symlink target must
/// live inside one of these — never an arbitrary local directory.
pub(in super::super) const CANONICAL_STORES: &[&str] = &["global-skills", "skill-packs"];

/// True iff `canonical_dir` (canonicalized) lives inside an approved store.
/// Defends against bad/stale manifest sources (absolute paths, `..` escapes,
/// targets outside the repo) being symlinked as host-loadable skill bodies.
pub(in super::super) fn canonical_within_store(repo_root: &Path, canonical_dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(canonical_dir) else {
        return false;
    };
    CANONICAL_STORES.iter().any(|store| {
        std::fs::canonicalize(repo_root.join(store))
            .map(|root| real.starts_with(&root))
            .unwrap_or(false)
    })
}

pub(in super::super) fn canonical_within_shared_store(
    home: &Path,
    name: &str,
    canonical_dir: &Path,
) -> bool {
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

pub(in super::super) fn is_external_shared_skill(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
) -> bool {
    let expected = ctx.home.join(".agents/skills").join(&cap.name);
    matches!(cap.kind, ManagedKind::Skill)
        && matches!(cap.managed_status, ManagedStatus::Governed)
        && cap.source.as_deref().map(Path::new) == Some(expected.as_path())
}

/// Accept a skill body only from the store declared by its owner. Suite-owned
/// skills stay confined to the repository stores; registry-governed external
/// skills stay confined to the shared multi-agent store under the injected
/// home. No arbitrary absolute manifest source becomes host-loadable.
pub(in super::super) fn canonical_source_allowed(
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
pub(in super::super) fn next_backup_path(dest: &Path) -> PathBuf {
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
pub(in super::super) fn next_replaced_path(dest: &Path) -> PathBuf {
    let base = format!("{}.ags-replaced", dest.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 1;
    while std::fs::symlink_metadata(&candidate).is_ok() {
        candidate = PathBuf::from(format!("{base}.{i}"));
        i += 1;
    }
    candidate
}
