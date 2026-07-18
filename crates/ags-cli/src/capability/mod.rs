//! `ags capability` thin facade.
use crate::cli::CapabilityAction;
use std::path::{Path, PathBuf};

/// Shared dispatch: `capability list`
fn cmd_capability_list(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "capability list: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let registry = capability_registry::discover_all(target);
    match format {
        "json" => println!("{}", capability_registry::render_json(&registry)),
        _ => println!("{}", capability_registry::render_text(&registry)),
    }
}
/// Shared dispatch: `capability show`
fn cmd_capability_show(name: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "capability show: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let registry = capability_registry::discover_all(target);
    match capability_registry::find_by_id(&registry, name) {
        Some(cap) => match format {
            "json" => println!("{}", capability_registry::render_one_json(cap)),
            _ => println!("{}", capability_registry::render_one_text(cap)),
        },
        None => {
            eprintln!("capability show: capability not found — {}", name);
            std::process::exit(1);
        }
    }
}

// ── Cross-Agent capability layer dispatch ──────────────────────────────────
//
// These reuse the shared skill-governance console (the same model behind
// `ags skill`): inventory + per-host visibility + confirmation-protected
// adopt/sync. AGS-owned skill thin-index writes go through the single guard;
// MCP / CLI-backed registration is advised per host, never run by AGS.
/// Default hosts the cross-Agent capability layer reports on.
fn capability_default_hosts() -> Vec<&'static str> {
    vec!["claude-code", "codex", "codebuddy-code"]
}

/// Rebuild and persist one host-scoped ActiveSkillTable snapshot after a
/// successful capability-affecting write. This is lifecycle maintenance, not a
/// routing layer: the request router never calls it.
pub(crate) fn refresh_skill_snapshot(
    authority_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<PathBuf, String> {
    let snapshot = skill_resolver::build_capability_snapshot(authority_root, active_host)
        .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
    let path = skill_resolver::snapshot_path(runtime_home, active_host);
    let parent = path
        .parent()
        .ok_or_else(|| "skill snapshot path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
    std::fs::write(&path, json + "\n")
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

// Pure exit-code policy for the capability commands, factored out of the I/O
// dispatch so it is unit-testable without touching the real environment.
/// `ags capability verify --strict` gate: nonzero unless status is "ok".
fn capability_verify_exit_code(strict: bool, status: &str) -> i32 {
    if strict && status != "ok" {
        1
    } else {
        0
    }
}
/// `ags capability install` exit: nonzero when AGS could not carry out an
/// `--apply` — blocked, a write failed, or the action is advised-only (the user
/// must run the advised host command). Mirrors `ags skill propose`.
fn capability_install_exit_code(
    apply: bool,
    result: &skill_governance::console::ConsoleProposalResult,
) -> i32 {
    let apply_unfulfilled = apply && result.apply_status == "advised-only";
    if !result.blocked_reasons.is_empty() || !result.apply_errors.is_empty() || apply_unfulfilled {
        1
    } else {
        0
    }
}
/// `ags capability sync` exit: dry-run is informational (always 0); `--apply`
/// is nonzero if any item's write failed or was blocked. Advised-only MCPs do
/// not fail the batch.
fn capability_sync_exit_code(
    apply: bool,
    summary: &skill_governance::console::CapabilitySyncSummary,
) -> i32 {
    if apply && (summary.failed > 0 || summary.blocked > 0) {
        1
    } else {
        0
    }
}
/// `ags capability inventory` — unified cross-Agent inventory + host visibility.
fn cmd_capability_inventory(hosts: &[String], format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags capability inventory");
    let ctx = console::ConsoleContext::system(root);
    let default_hosts = capability_default_hosts();
    let host_refs: Vec<&str> = if hosts.is_empty() {
        default_hosts
    } else {
        hosts.iter().map(String::as_str).collect()
    };
    let inv = console::build_inventory(&ctx, &host_refs);
    match format {
        "json" => println!("{}", console::render_inventory_json(&inv)),
        _ => println!("{}", console::render_inventory_text(&inv)),
    }
}
/// `ags capability verify --host <host>` — read-only host visibility (canonical
/// home for the check `ags skill verify` also exposes for compatibility).
pub(crate) fn cmd_capability_verify(host: &str, strict: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags capability verify");
    let ctx = console::ConsoleContext::system(root);
    let result = console::verify_host(&ctx, host);
    let status = result.status.clone();
    match format {
        "json" => println!("{}", console::render_verify_json(&result)),
        _ => println!("{}", console::render_verify_text(&result)),
    }
    let code = capability_verify_exit_code(strict, &status);
    if code != 0 {
        std::process::exit(code);
    }
}
/// `ags capability install --capability <name>` — single-capability cross-host
/// entry. Dry-run unless `--apply`. AGS-owned thin-index writes go through the
/// guard; MCP / CLI registration is advised, never executed.
fn cmd_capability_install(capability: &str, apply: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags capability install");
    let ctx = console::ConsoleContext::system(root);
    let result = console::propose_action(&ctx, console::ConsoleAction::Adopt, capability, apply);
    let code = capability_install_exit_code(apply, &result);
    if apply && code == 0 {
        refresh_skill_snapshot(
            &ctx.repo_root,
            &skill_resolver::locate_runtime_home(),
            "codex",
        )
        .unwrap_or_else(|error| {
            eprintln!("ags capability install: {error}");
            std::process::exit(1);
        });
    }
    match format {
        "json" => println!("{}", console::render_proposal_json(&result)),
        _ => println!("{}", console::render_proposal_text(&result)),
    }
    // Exit nonzero when an `--apply` could not actually be carried out by AGS:
    // blocked, a write failed, or the action is advised-only (the user must run
    // the advised host command). Mirrors `ags skill propose` semantics.
    if code != 0 {
        std::process::exit(code);
    }
}
/// `ags capability sync` — batch cross-host entry plan for all adopted/governed
/// capabilities. Dry-run unless `--apply`.
///
/// Dry-run is informational and always exits 0 — per-item blocked/needs-action
/// state is surfaced in the report and summary, not as a command failure (a
/// batch plan should not fail because one pre-existing capability is mislabeled).
/// `--apply` exits nonzero if any item's write failed or was blocked, since the
/// user asked AGS to perform the sync. Advised-only MCPs never fail the batch.
pub(crate) fn cmd_capability_sync(apply: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags capability sync");
    let ctx = console::ConsoleContext::system(root);
    let hosts = capability_default_hosts();
    let result = console::sync_plan(&ctx, &hosts, apply);
    let code = capability_sync_exit_code(apply, &result.summary);
    if apply && code == 0 {
        refresh_skill_snapshot(
            &ctx.repo_root,
            &skill_resolver::locate_runtime_home(),
            "codex",
        )
        .unwrap_or_else(|error| {
            eprintln!("ags capability sync: {error}");
            std::process::exit(1);
        });
    }
    match format {
        "json" => println!("{}", console::render_sync_json(&result)),
        _ => println!("{}", console::render_sync_text(&result)),
    }
    if code != 0 {
        std::process::exit(code);
    }
}

// ── M6 dispatch functions ─────────────────────────────────────────────────
#[cfg(test)]
mod capability_exit_code_tests {
    use super::{
        capability_install_exit_code, capability_sync_exit_code, capability_verify_exit_code,
    };
    use skill_governance::console::{CapabilitySyncSummary, ConsoleProposalResult};

    #[test]
    fn verify_strict_gate_only_fails_when_not_ok() {
        assert_eq!(capability_verify_exit_code(true, "ok"), 0);
        assert_eq!(capability_verify_exit_code(true, "degraded"), 1);
        assert_eq!(capability_verify_exit_code(true, "incomplete"), 1);
        // Without --strict, verify is always informational.
        assert_eq!(capability_verify_exit_code(false, "degraded"), 0);
    }

    fn proposal(apply_status: &str) -> ConsoleProposalResult {
        ConsoleProposalResult {
            apply_status: apply_status.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn install_exit_code_covers_blocked_failed_and_advised_only() {
        // Clean dry-run → 0.
        assert_eq!(capability_install_exit_code(false, &proposal("dry-run")), 0);
        // Successful apply → 0.
        assert_eq!(capability_install_exit_code(true, &proposal("applied")), 0);
        // Apply of an advised-only (MCP) capability → 1 (AGS performed nothing).
        assert_eq!(
            capability_install_exit_code(true, &proposal("advised-only")),
            1
        );
        // Advised-only WITHOUT apply → 0 (nothing was requested to apply).
        assert_eq!(
            capability_install_exit_code(false, &proposal("advised-only")),
            0
        );
        // Blocked → 1 regardless of apply.
        let mut blocked = proposal("blocked");
        blocked.blocked_reasons.push("bad source".to_string());
        assert_eq!(capability_install_exit_code(true, &blocked), 1);
        assert_eq!(capability_install_exit_code(false, &blocked), 1);
        // Write failure → 1.
        let mut failed = proposal("failed");
        failed.apply_errors.push("write failed".to_string());
        assert_eq!(capability_install_exit_code(true, &failed), 1);
    }

    fn summary(blocked: usize, failed: usize) -> CapabilitySyncSummary {
        CapabilitySyncSummary {
            considered: 5,
            planned_writes: 3,
            applied: 0,
            advised_only: 2,
            blocked,
            failed,
            needs_action: 4,
        }
    }

    #[test]
    fn sync_exit_code_dryrun_informational_apply_fails_on_blocked_or_failed() {
        // Dry-run is always informational, even with blocked/failed items.
        assert_eq!(capability_sync_exit_code(false, &summary(1, 0)), 0);
        assert_eq!(capability_sync_exit_code(false, &summary(0, 1)), 0);
        // Apply fails hard on blocked or failed.
        assert_eq!(capability_sync_exit_code(true, &summary(1, 0)), 1);
        assert_eq!(capability_sync_exit_code(true, &summary(0, 1)), 1);
        // Clean apply → 0.
        assert_eq!(capability_sync_exit_code(true, &summary(0, 0)), 0);
    }
}

/// `ags capability snapshot` — derive the machine-local capability snapshot +
/// attestation hash. It contains the strict ActiveSkillTable intersection for
/// one host and a deterministic `snapshot_hash`. With `--write`, persists to
/// the machine-local runtime home (never tracked or published).
fn cmd_capability_snapshot(host: &str, target: &Path, write: bool, format: &str) {
    let requested = if target.as_os_str().is_empty() || target == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        target.to_path_buf()
    };
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from);
    let root = crate::context::resolve_capability_authority_root(
        &requested,
        &skill_resolver::locate_runtime_home(),
        explicit,
    )
    .unwrap_or_else(|detail| {
        eprintln!("ags capability snapshot: refused — {detail}");
        std::process::exit(1);
    });
    let snapshot = skill_resolver::build_capability_snapshot(&root, host).unwrap_or_else(|error| {
        eprintln!("ags capability snapshot: build failed — {error:?}");
        std::process::exit(1);
    });

    let mut written: Option<String> = None;
    if write {
        let path = refresh_skill_snapshot(&root, &skill_resolver::locate_runtime_home(), host)
            .unwrap_or_else(|error| {
                eprintln!("snapshot: {error}");
                std::process::exit(1);
            });
        written = Some(path.to_string_lossy().to_string());
    }

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).unwrap_or_default()
        ),
        _ => {
            println!("ActiveSkillTable snapshot (machine-local)");
            println!("Active host: {}", snapshot.active_host);
            println!("Snapshot hash: {}", snapshot.snapshot_hash);
            println!("Active skills: {}", snapshot.active_skills.len());
            match &written {
                Some(p) => println!("Written: {}", p),
                None => println!(
                    "(dry-run — pass --write to persist to the machine-local runtime home; never tracked)"
                ),
            }
        }
    }
}

pub(crate) fn run(action: CapabilityAction) {
    match action {
        CapabilityAction::List { target, format } => cmd_capability_list(&target, &format),
        CapabilityAction::Show {
            name,
            target,
            format,
        } => cmd_capability_show(&name, &target, &format),
        CapabilityAction::Inventory { host, format } => cmd_capability_inventory(&host, &format),
        CapabilityAction::Verify {
            host,
            strict,
            format,
        } => cmd_capability_verify(&host, strict, &format),
        CapabilityAction::Install {
            capability,
            apply,
            format,
        } => cmd_capability_install(&capability, apply, &format),
        CapabilityAction::Sync { apply, format } => cmd_capability_sync(apply, &format),
        CapabilityAction::Snapshot {
            host,
            target,
            write,
            format,
        } => cmd_capability_snapshot(&host, &target, write, &format),
    }
}
