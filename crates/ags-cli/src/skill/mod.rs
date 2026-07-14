//! `ags skill` thin facade (五段链路第 3 段).
use crate::capability::cmd_capability_sync;
use crate::cli::SkillAction;
use crate::receipt_bridge::emit_ags_action_receipt;

/// Shared dispatch: `skill scan`
fn cmd_skill_scan(format: &str) {
    let root = crate::context::capability_authority_root_or_exit("ags skill scan");
    let result = skill_governance::scan_skills(&root);

    match format {
        "json" => println!("{}", skill_governance::render_scan_json(&result)),
        _ => println!("{}", skill_governance::render_scan_text(&result)),
    }
}
/// Shared dispatch: `skill check`
fn cmd_skill_check(format: &str) {
    let root = crate::context::capability_authority_root_or_exit("ags skill check");
    let result = skill_governance::check_skills(&root);

    match format {
        "json" => println!("{}", skill_governance::render_check_json(&result)),
        _ => println!("{}", skill_governance::render_check_text(&result)),
    }

    if !result.passed {
        std::process::exit(1);
    }
}
/// Shared dispatch: `skill propose` — management console proposal.
///
/// Dry-run by default. `--apply` performs only AGS-owned host-entry writes
/// through the console's transactional mutation guard; external
/// installers/registrars are advised, never executed.
fn cmd_skill_propose(action: &str, skill_name: &str, apply: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags skill propose");
    let Some(parsed) = console::ConsoleAction::from_str(action) else {
        eprintln!("skill propose: unknown action '{action}'");
        std::process::exit(2);
    };
    let ctx = console::ConsoleContext::system(root);
    let result = console::propose_action(&ctx, parsed, skill_name, apply);

    match format {
        "json" => println!("{}", console::render_proposal_json(&result)),
        _ => println!("{}", console::render_proposal_text(&result)),
    }

    // Exit nonzero when an `--apply` could not actually be carried out by AGS:
    // blocked, a write failed, or the action is advised-only (AGS performed
    // nothing and the user must run the advised command). A clean dry-run, a
    // successful apply, and a genuine no-op all exit 0.
    let apply_unfulfilled = apply && matches!(result.apply_status.as_str(), "advised-only");
    if !result.blocked_reasons.is_empty() || !result.apply_errors.is_empty() || apply_unfulfilled {
        std::process::exit(1);
    }
}
/// Shared dispatch: `skill verify --host <host>` — read-only host visibility.
///
/// Informational by default (exit 0). With `--strict` it acts as a post-apply
/// gate: exit nonzero unless status is "ok" (i.e. every expected capability is
/// visible).
fn cmd_skill_verify(host: &str, strict: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags skill verify");
    let ctx = console::ConsoleContext::system(root);
    let result = console::verify_host(&ctx, host);
    let status = result.status.clone();

    match format {
        "json" => println!("{}", console::render_verify_json(&result)),
        _ => println!("{}", console::render_verify_text(&result)),
    }

    if strict && status != "ok" {
        std::process::exit(1);
    }
}
/// Shared dispatch: `skill inventory`
fn cmd_skill_inventory(format: &str, write: bool) {
    let root = crate::context::capability_authority_root_or_exit("ags skill inventory");
    let result = skill_governance::scan_skill_inventory(&root);

    match format {
        "json" => println!("{}", skill_governance::render_inventory_json(&result)),
        _ => println!("{}", skill_governance::render_inventory_text(&result)),
    }

    if write {
        let report_dir = root.join("governance");
        let report_path = report_dir.join("skills-inventory.md");
        let markdown = skill_governance::render_inventory_markdown(&result);
        match std::fs::create_dir_all(&report_dir)
            .and_then(|_| std::fs::write(&report_path, markdown))
        {
            Ok(_) => println!("\nWrote {}", report_path.display()),
            Err(e) => {
                eprintln!("Failed to write {}: {e}", report_path.display());
                std::process::exit(1);
            }
        }
    }
}
/// Shared dispatch: `skill upstream` — read-only upstream proposal stub.
///
/// Reads manifests/skills-registry.yaml and reports the upstream comparison
/// sources and the suite skills that watch them. Performs NO network crawl.
fn cmd_skill_upstream(format: &str) {
    let root = crate::context::capability_authority_root_or_exit("ags skill upstream");
    let result = skill_governance::upstream_proposal(&root);

    match format {
        "json" => println!("{}", skill_governance::render_upstream_json(&result)),
        _ => println!("{}", skill_governance::render_upstream_text(&result)),
    }
}
/// `ags skill update` — incremental, auditable upstream update proposal
/// (check/plan only; never pulls or overwrites). Canonical front-stage name for
/// the upstream proposal; `ags skill upstream` remains as a hidden alias.
fn cmd_skill_update(format: &str) {
    cmd_skill_upstream(format);
}
/// `ags skill sync` — batch cross-host thin-index distribution. Same engine as
/// `ags capability sync` (skill governance is the front-stage; capability is the
/// underlying layer). Dry-run unless `--apply`.
fn cmd_skill_sync(apply: bool, format: &str) {
    cmd_capability_sync(apply, format);
}
/// `ags skill dedupe` — detect duplicate skills across the canonical store and
/// plan a reversible quarantine. Dry-run unless `--apply`; canonical bodies are
/// never deleted. Emits a receipt when writes occur.
fn cmd_skill_dedupe(apply: bool, format: &str) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags skill dedupe");
    let result = console::analyze_duplicates(&root, apply);
    match format {
        "json" => println!("{}", console::render_dedupe_json(&result)),
        _ => println!("{}", console::render_dedupe_text(&result)),
    }
    if apply && !result.applied_moves.is_empty() {
        // Each move (from → to) is recorded as a reversible write, and the
        // rollback plan carries source/dest pairs so a quarantine can be undone.
        let writes: Vec<receipt::ReceiptWrite> = result
            .applied_moves
            .iter()
            .map(|mv| receipt::ReceiptWrite {
                op: "backup".to_string(),
                path: mv.to.clone(),
                from: Some(mv.from.clone()),
                backup: Some(mv.to.clone()),
                detail: "quarantined non-keeper copy".to_string(),
            })
            .collect();
        let rollback_steps: Vec<receipt::RollbackStep> = result
            .applied_moves
            .iter()
            .map(|mv| receipt::RollbackStep {
                affected_path: mv.from.clone(),
                inverse_op: "restore-backup".to_string(),
                backup_path: Some(mv.to.clone()),
                inverse_command: Some(format!("mv \"{}\" \"{}\"", mv.to, mv.from)),
                detail: "restore quarantined copy to its canonical store path".to_string(),
            })
            .collect();
        let ar = receipt::build_action_receipt(
            "skill-dedupe",
            Some(&root.display().to_string()),
            receipt::GateResult {
                decision: "allow".to_string(),
                reason: None,
            },
            vec![],
            writes,
            vec![],
            vec![],
            receipt::RollbackPlan::backup_restore(rollback_steps),
            &result.apply_status,
            true,
        );
        if let Ok(p) = emit_ags_action_receipt(&ar) {
            println!("\n{}", receipt::render_action_receipt_summary_line(&p));
        }
    }
    if apply && !result.apply_errors.is_empty() {
        std::process::exit(1);
    }
}
fn cmd_skill_overview(format: &str, fix: bool) {
    use skill_governance::console;
    let root = crate::context::capability_authority_root_or_exit("ags skill overview");
    let scan = skill_governance::scan_skills(&root);
    let check = skill_governance::check_skills(&root);
    // Unified management-console inventory: skills + MCPs + suite interface +
    // CLI-backed, with canonical body status + per-host thin-index visibility
    // across Claude Code and Codex. Read-only.
    let ctx = console::ConsoleContext::system(root);
    let inventory = console::build_inventory(&ctx, &["claude-code", "codex"]);

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": "2.6.0-skill-console-overview",
                "inventory": inventory,
                "scan": scan,
                "check": check,
                "fix_requested": fix,
                "update_policy": "no_silent_writes_user_confirmation_required",
                "next_steps": if fix {
                    serde_json::json!([
                        "Review the inventory: managed_status, host_visibility, health_status, risk_notes.",
                        "Dry-run a change: `ags skill propose --action <adopt|update|remove|uninstall|repair|verify> --skill <name>`.",
                        "Confirm with `--apply` (writes only AGS-owned host entries; never runs external installers).",
                        "After apply, restart the host and run `ags skill verify --host claude-code`.",
                        "Review upstream comparison sources with `ags skill upstream` (read-only stub; no crawl)."
                    ])
                } else {
                    serde_json::json!([
                        "Run `ags skill verify --host claude-code` to check host visibility.",
                        "Run `ags skill --fix` for update guidance. No files are modified by overview."
                    ])
                }
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", console::render_inventory_text(&inventory));
            println!();
            println!("{}", skill_governance::render_scan_text(&scan));
            println!();
            println!("{}", skill_governance::render_check_text(&check));
            println!();
            if fix {
                println!("Skill Update Guidance");
                println!("=====================");
                println!("No skill files were modified.");
                println!("Review the inventory above, then use:");
                println!("  ags skill propose --action adopt --skill <name>          # dry-run");
                println!("  ags skill propose --action adopt --skill <name> --apply  # confirm");
                println!(
                    "  ags skill verify  --host claude-code                     # host visibility"
                );
                println!("  ags skill upstream                                       # upstream comparison (stub)");
                println!(
                    "Apply writes only AGS-owned host entries via transactional replace and never runs external installers."
                );
            } else {
                println!(
                    "Next: `ags skill verify --host claude-code` for host visibility, or `ags skill --fix` for update guidance. No files were modified."
                );
            }
        }
    }

    if !check.passed && !fix {
        std::process::exit(1);
    }
}

// ── Run dispatch ───────────────────────────────────────────────────────────

pub(crate) fn run(action: Option<SkillAction>, format: &str, fix: bool) {
    match action {
        Some(SkillAction::Scan { format }) => cmd_skill_scan(&format),
        Some(SkillAction::Check { format }) => cmd_skill_check(&format),
        Some(SkillAction::Propose {
            action,
            skill,
            apply,
            format,
        }) => cmd_skill_propose(&action, &skill, apply, &format),
        Some(SkillAction::Verify {
            host,
            strict,
            format,
        }) => cmd_skill_verify(&host, strict, &format),
        Some(SkillAction::Inventory { format, write }) => cmd_skill_inventory(&format, write),
        Some(SkillAction::Upstream { format }) => cmd_skill_upstream(&format),
        Some(SkillAction::Dedupe { apply, format }) => cmd_skill_dedupe(apply, &format),
        Some(SkillAction::Update { format }) => cmd_skill_update(&format),
        Some(SkillAction::Sync { apply, format }) => cmd_skill_sync(apply, &format),
        None => cmd_skill_overview(format, fix),
    }
}
