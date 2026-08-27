//! `ags` — Thin AGS unified CLI (contract v3).
//!
//! Command surface: init, run, apply, check, test, log, status, doctor,
//! update, govern, schema. Adapters translate argv and own no policy; every
//! decision flows through `ags-kernel`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use ags_kernel::capabilities::CapabilitiesLock;
use ags_kernel::config::Config;
use ags_kernel::error::{Error, Result};
use ags_kernel::evidence::EvidenceLog;
use ags_kernel::seal::SealStore;
use ags_kernel::workspace::{self, WorkspaceBinding};

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(exit) => exit,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    std::process::exit(code);
}

#[derive(Parser)]
#[command(
    name = "ags",
    version = env!("AGS_BUILD_DISPLAY"),
    disable_help_subcommand = true,
    about = "ags — Thin AGS (contract v3). Three layers, lark-cli style: shortcuts, typed commands, sealed escape hatch.",
    after_help = "RISK LEGEND (shown per command as read | write | high-risk-write):
  read            never writes project or AGS state
  write           writes only through a sealed plan; run `ags apply <ACTION_REF>` to commit once
  high-risk-write sealed + crosses a boundary; same plan/apply flow, never auto-applied

QUICKSTART (agent driving this? start here):
  ags run --task card.md                 # +shortcut: prepare a task (validate + matrix + review level)
  ags run --task card.md --verify        # run the structured verification commands
  ags run --task card.md --close --report report.json
  ags skill list                         # installed skill inventory
  ags skill recommend [query]            # discover recommended third-party skills
  ags route <need>                       # fallback matcher when host selected no skill
  ags schema [operation]                 # inspect the sealed registry/payload shape
  ags doctor                             # runtime health; project audit is separate"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// [read] Serve the canonical AGS MCP surface over stdio.
    Mcp {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [write] Adopt a workspace: seals a plan; `ags apply` commits it once. User files are never overwritten.
    Init {
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long, default_value = "A")]
        role: String,
    },
    /// [high-risk-write] Consume one sealed action_ref exactly once (the only mutation surface).
    Apply {
        action_ref: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [read] +shortcut: one-command task flow — prepare (default) / --verify / --close. Execution stays host-side.
    Run {
        #[arg(long)]
        task: PathBuf,
        #[arg(long, default_value = "smoke")]
        profile: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        update_capabilities: bool,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        close: bool,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        instance: Option<String>,
    },
    /// [read] Typed checks: governance | matrix | capabilities | evidence.
    Check {
        #[arg(default_value = "governance")]
        scope: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        format: Option<String>,
    },
    /// [read] Structured project test execution (no shell; the verify command set from ags.toml).
    Test {
        profile: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [read] Derive a readable report from the evidence log (--type/--task/--scope filter).
    Log {
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [read] Current task state from the evidence tail.
    Status {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [read] Health probe: matrix lint, hook wiring, capability routes, evidence chain.
    Doctor {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [write] Machine-level install: record the official content source, install official skills, refresh the machine lock.
    Setup {
        #[arg(long)]
        source_root: Option<PathBuf>,
    },
    /// [write] Sealed capability-lock refresh (the only hash-pin refresh entry).
    Update {
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        sources: Vec<String>,
    },
    /// [write] Sealed skill install/remove and host projection.
    Govern {
        #[command(subcommand)]
        action: GovernAction,
    },
    /// [read] Skill lifecycle views: installed inventory and recommendations.
    Skill {
        #[command(subcommand)]
        action: SkillReadAction,
    },
    /// [write] Evidence-only delegation lifecycle: accept / return / integrate.
    Delegation {
        #[command(subcommand)]
        action: DelegationAction,
    },
    /// [read] Git clean/smudge projection for local AGS entry blocks.
    EntryFilter {
        #[command(subcommand)]
        action: EntryFilterAction,
    },
    /// [read] Deterministic skill match: unique ready hit, ties rejected.
    Route {
        input: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// [read] Print the sealed operation registry; `ags schema <operation>` inspects one op.
    Schema {
        #[arg(value_name = "OPERATION")]
        operation: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
}

#[derive(Subcommand)]
enum GovernAction {
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    DelegationIssue {
        #[arg(long)]
        parent: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        subtask: String,
        #[arg(long, value_delimiter = ',')]
        allowed_resources: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        allowed_capabilities: Vec<String>,
        #[arg(long, default_value_t = 1)]
        depth: u32,
        #[arg(long)]
        return_contract: String,
        #[arg(long)]
        owner_instance: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    HostRegister {
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "cli")]
        surface: String,
        #[arg(long)]
        dispatch: bool,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    HostProjection {
        #[arg(long, default_value = "reconcile")]
        mode: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value = "cli")]
        surface: String,
        #[arg(long, default_value = "full")]
        lifecycle: String,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

/// Evidence-only delegation lifecycle steps (no sealed mutation).
#[derive(Subcommand)]
enum DelegationAction {
    /// Child instance accepts a grant.
    Accept {
        #[arg(long)]
        grant: String,
        #[arg(long)]
        instance: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Child returns its result + evidence.
    Return {
        #[arg(long)]
        grant: String,
        #[arg(long)]
        instance: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Task owner integrates a returned grant.
    Integrate {
        #[arg(long)]
        grant: String,
        #[arg(long)]
        instance: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum EntryFilterAction {
    Clean,
    Smudge,
}

#[derive(Subcommand)]
enum SkillReadAction {
    /// List bodies currently present under ~/.agents/skills.
    List,
    /// List recommended third-party skills, optionally filtered by text.
    Recommend { query: Option<String> },
}

#[derive(Subcommand)]
enum SkillAction {
    Install {
        #[arg(long)]
        skill_id: String,
        #[arg(long)]
        path: String,
        #[arg(long = "ack-risk")]
        acknowledged_risks: Vec<String>,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    Remove {
        #[arg(long)]
        skill_id: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Mcp { workspace } => cmd_mcp(workspace),
        Command::Init {
            workspace,
            slug,
            role,
        } => cmd_init(workspace, slug, role),
        Command::Apply {
            action_ref,
            workspace,
        } => cmd_apply(&action_ref, workspace),
        Command::Run {
            task,
            profile,
            workspace,
            update_capabilities,
            verify,
            close,
            report,
            instance,
        } => cmd_run(
            task,
            profile,
            workspace,
            update_capabilities,
            verify,
            close,
            report,
            instance,
        ),
        Command::Check {
            scope,
            workspace,
            format,
        } => cmd_check(scope, workspace, format),
        Command::Test { profile, workspace } => cmd_test(profile, workspace),
        Command::Log {
            r#type,
            task,
            scope,
            workspace,
        } => cmd_log(r#type, task, scope, workspace),
        Command::Status { workspace } => cmd_status(workspace),
        Command::Doctor { workspace } => cmd_doctor(workspace),
        Command::Update { workspace, sources } => cmd_update(workspace, sources),
        Command::Govern { action } => match action {
            GovernAction::Skill { action } => match action {
                SkillAction::Install {
                    skill_id,
                    path,
                    acknowledged_risks,
                    workspace,
                } => cmd_skill_install(skill_id, path, acknowledged_risks, workspace),
                SkillAction::Remove {
                    skill_id,
                    workspace,
                } => cmd_skill_remove(skill_id, workspace),
            },
            GovernAction::HostProjection {
                mode,
                host,
                surface,
                lifecycle,
                slug,
                workspace,
            } => cmd_host_projection(mode, host, surface, lifecycle, slug, workspace),
            GovernAction::HostRegister {
                id,
                surface,
                dispatch,
                workspace,
            } => cmd_host_register(id, surface, dispatch, workspace),
            GovernAction::DelegationIssue {
                parent,
                target,
                subtask,
                allowed_resources,
                allowed_capabilities,
                depth,
                return_contract,
                owner_instance,
                workspace,
            } => cmd_delegation_issue(
                parent,
                target,
                subtask,
                allowed_resources,
                allowed_capabilities,
                depth,
                return_contract,
                owner_instance,
                workspace,
            ),
        },
        Command::Delegation { action } => match action {
            DelegationAction::Accept {
                grant,
                instance,
                workspace,
            } => cmd_delegation_accept(grant, instance, workspace),
            DelegationAction::Return {
                grant,
                instance,
                summary,
                workspace,
            } => cmd_delegation_return(grant, instance, summary, workspace),
            DelegationAction::Integrate {
                grant,
                instance,
                workspace,
            } => cmd_delegation_integrate(grant, instance, workspace),
        },
        Command::EntryFilter { action } => cmd_entry_filter(action),
        Command::Skill { action } => match action {
            SkillReadAction::List => cmd_skill_list(),
            SkillReadAction::Recommend { query } => cmd_skill_recommend(query),
        },
        Command::Setup { source_root } => cmd_setup(source_root),
        Command::Route { input, workspace } => cmd_route(input, workspace),
        Command::Schema { operation, format } => cmd_schema(operation, format),
    }
}

fn cmd_mcp(workspace: Option<PathBuf>) -> Result<i32> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("ags-mcp")))
        .filter(|path| path.is_file());
    let executable = sibling.unwrap_or_else(|| PathBuf::from("ags-mcp"));
    let mut command = std::process::Command::new(&executable);
    if let Some(root) = workspace {
        command.arg("--workspace").arg(root);
    }
    let status = command.status().map_err(|e| {
        Error::new(
            "ags_mcp_launch_failed",
            format!("{}: {e}", executable.display()),
        )
    })?;
    Ok(status.code().unwrap_or(1))
}

fn cmd_entry_filter(action: EntryFilterAction) -> Result<i32> {
    use std::io::{Read, Write};
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| Error::new("entry_filter_read_failed", e.to_string()))?;
    let output = match action {
        EntryFilterAction::Clean => ags_kernel::sync::strip_entry_text(&input),
        EntryFilterAction::Smudge => ags_kernel::sync::render_entry_text(&input),
    };
    std::io::stdout()
        .write_all(output.as_bytes())
        .map_err(|e| Error::new("entry_filter_write_failed", e.to_string()))?;
    Ok(0)
}

fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    match explicit {
        Some(p) => {
            if p.join(workspace::AGS_TOML).is_file() {
                Ok(p)
            } else {
                workspace::find_workspace(&p)
            }
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| Error::new("cwd_resolve_failed", e.to_string()))?;
            workspace::find_workspace(&cwd)
        }
    }
}

// ── init ─────────────────────────────────────────────────────────────────

/// Host hook templates installed by `ags init` when the target file is
/// absent; existing files are preserved byte-for-byte.
const HOOK_TEMPLATES: &[(&str, &str)] = &[
    (
        ".claude/settings.json",
        include_str!("../hook-templates/claude-code.settings.json"),
    ),
    (
        ".codex/hooks.json",
        include_str!("../hook-templates/codex.hooks.json"),
    ),
    (
        ".cursor/hooks.json",
        include_str!("../hook-templates/cursor.hooks.json"),
    ),
    (
        ".codebuddy/settings.local.json",
        include_str!("../hook-templates/codebuddy.settings.local.json"),
    ),
    (
        ".omp/extensions/ags-policy.js",
        include_str!("../hook-templates/omp-ags-policy.js"),
    ),
];

fn cmd_init(workspace: Option<PathBuf>, slug: Option<String>, role: String) -> Result<i32> {
    let root = match workspace {
        Some(p) => p,
        None => {
            std::env::current_dir().map_err(|e| Error::new("cwd_resolve_failed", e.to_string()))?
        }
    };
    let binding = workspace::provisional(&root);
    let existing = Config::load(&root).ok();
    let slug = slug
        .or_else(|| existing.map(|c| c.workspace.slug))
        .unwrap_or_else(|| {
            root.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".to_string())
        });
    let mut scaffold = ags_kernel::config::scaffold(&slug);
    scaffold.workspace.role = role;
    // Capability sources are type-projected: only directories that actually
    // exist in this project are configured. Source-repo directory names
    // (ags-skills / skill-packs) must never be projected into ordinary
    // projects — a configured-but-missing source yields an empty routing
    // table and a lint finding.
    scaffold.capabilities.sources.retain(|source| {
        let dir = root.join(source);
        dir.is_dir()
    });
    let toml_text = toml::to_string(&scaffold)
        .map_err(|e| Error::new("ags_toml_encode_failed", e.to_string()))?;
    let hooks: Vec<serde_json::Value> = HOOK_TEMPLATES
        .iter()
        .map(|(rel, content)| serde_json::json!({"rel": rel, "content": content}))
        .collect();
    let payload = serde_json::json!({
        "root": root,
        "slug": slug,
        "ags_toml": toml_text,
        "hooks": hooks,
    });
    let store = SealStore::new(&binding);
    let action = store.seal_plan("init", &payload, &binding)?;
    println!(
        "init plan sealed: {}\nrun `ags apply {} --workspace {}` to adopt the workspace",
        action.plan_hash,
        action.token,
        root.display()
    );
    Ok(0)
}

// ── apply ────────────────────────────────────────────────────────────────

fn cmd_apply(action_ref: &str, workspace: Option<PathBuf>) -> Result<i32> {
    let root = match &workspace {
        Some(p) => p.clone(),
        None => {
            std::env::current_dir().map_err(|e| Error::new("cwd_resolve_failed", e.to_string()))?
        }
    };
    // The binding must match the one used at seal time, per operation:
    // non-init plans were sealed against the real workspace identity
    // (slug+role), init plans against the provisional (pre-ags.toml) one.
    let provisional = workspace::provisional(&root);
    let probe_store = SealStore::new(&provisional);
    let plan = probe_store.load_plan(action_ref)?;
    let real_binding = workspace::bind(&root);
    let binding = if plan.operation == "init" {
        provisional
    } else {
        real_binding.as_ref().map_err(|e| e.clone())?.clone()
    };
    let store = SealStore::new(&binding);
    let receipt =
        store.apply_with_result(action_ref, &binding, |plan| match plan.operation.as_str() {
            "init" => ags_kernel::effects::init_effect(&root, &plan.payload).map(Into::into),
            other => {
                let binding = real_binding.as_ref().map_err(|e| e.clone())?;
                ags_kernel::effects::run(other, &plan.payload, binding)
            }
        })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt)
            .map_err(|e| Error::new("receipt_encode_failed", e.to_string()))?
    );
    Ok(0)
}

// ── run ──────────────────────────────────────────────────────────────────

// phase flags map 1:1 to clap args; bundling them would hide the surface.
#[allow(clippy::too_many_arguments)]
fn cmd_run(
    task: PathBuf,
    profile: String,
    workspace: Option<PathBuf>,
    update_capabilities: bool,
    verify: bool,
    close: bool,
    report: Option<PathBuf>,
    instance: Option<String>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    if update_capabilities {
        let config = Config::load(&root)?;
        ags_kernel::capabilities::refresh(&binding, &config.capabilities.sources)?;
        println!("capabilities lock refreshed");
    }
    if close {
        let report_value = match report {
            Some(path) => {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| Error::new("report_read_failed", e.to_string()))?;
                serde_json::from_str(&text)
                    .map_err(|e| Error::new("report_parse_failed", e.to_string()))?
            }
            None => serde_json::json!({"status": "succeeded"}),
        };
        let out = ags_task_contract::run_close(&binding, &task, report_value, instance.as_deref())?;
        print_json(&out);
        return Ok(if out["governance_status"] == "CLOSED" {
            0
        } else {
            1
        });
    }
    if verify {
        let out = ags_task_contract::run_verify(&binding, &task, &profile)?;
        print_json(&out);
        return Ok(if out["governance_status"] == "VERIFIED" {
            0
        } else {
            1
        });
    }
    let out = ags_task_contract::run_prepare(&binding, &task)?;
    print_json(&out);
    Ok(if out["validated"] == true { 0 } else { 1 })
}

fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

// ── check / test ─────────────────────────────────────────────────────────

fn cmd_check(scope: String, workspace: Option<PathBuf>, format: Option<String>) -> Result<i32> {
    if !matches!(
        scope.as_str(),
        "governance" | "matrix" | "capabilities" | "evidence"
    ) {
        return Err(Error::new(
            "check_scope_unknown",
            format!("unknown scope `{scope}` (governance|matrix|capabilities|evidence)"),
        ));
    }
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let config = Config::load(&root)?;
    let mut findings: Vec<serde_json::Value> = Vec::new();
    match scope.as_str() {
        "governance" | "matrix" => {
            for finding in config.lint() {
                findings.push(serde_json::json!({
                    "code": finding.code,
                    "message": finding.message,
                }));
            }
        }
        "capabilities" => {}
        "evidence" => {}
        _ => unreachable!("scope validated above"),
    }
    // Configured-but-missing capability sources are a routing defect: the
    // lock stays empty and the routing table is silently dead. Flag them so
    // `ags update` (which prunes dead sources) or a re-init fixes the config.
    let mut missing_sources: Vec<String> = Vec::new();
    for source in &config.capabilities.sources {
        let dir = if std::path::Path::new(source).is_absolute() {
            std::path::PathBuf::from(source)
        } else {
            root.join(source)
        };
        if !dir.is_dir() {
            missing_sources.push(source.clone());
        }
    }
    for source in &missing_sources {
        findings.push(serde_json::json!({
            "code": "capability_source_missing",
            "message": format!("configured capability source `{source}` does not exist; prune it or run `ags update` to repair"),
        }));
    }
    let routes = CapabilitiesLock::load(&binding)?.check_routes(&root);
    let route_ok = routes.iter().all(|r| r.status == "exact");
    let chain_ok = EvidenceLog::verify_chain(
        &EvidenceLog::new(binding.evidence_dir.clone())
            .read_all()
            .unwrap_or_default(),
    )
    .is_ok();
    // Project capability routes are an audit view. They only gate the
    // explicit capabilities scope; runtime governance health is owned by the
    // machine skill directory and machine lock.
    let passed = findings.is_empty() && chain_ok && (scope != "capabilities" || route_ok);
    let pinned = CapabilitiesLock::load(&binding)?.entries.len();
    let hint = if pinned == 0 && missing_sources.is_empty() {
        "no AGS-pinned capability bodies (sources empty); host-native skills load via the host's own skill system (SKILL.md), not through AGS routing"
    } else {
        ""
    };
    let out = serde_json::json!({
        "scope": scope,
        "passed": passed,
        "project_tests_run": false,
        "lint_findings": findings,
        "capability_routes": routes,
        "capability_audit_clean": route_ok,
        "evidence_chain_ok": chain_ok,
        "hint": hint,
    });
    if format.as_deref() == Some("json") {
        print_json(&out);
    } else if passed {
        println!("check {scope}: ok (project_tests_run=false)");
    } else {
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    }
    Ok(if passed { 0 } else { 1 })
}

fn cmd_test(profile: String, workspace: Option<PathBuf>) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let config = Config::load(&root)?;
    let mut receipts = Vec::new();
    let mut failed = Vec::new();
    for command in &config.verify.commands {
        let spec = ags_task_contract::command::parse_command(command)?;
        let mut spec = spec;
        spec.cwd = root.clone();
        let receipt = ags_task_contract::command::run(&spec);
        let ok = receipt.status == "succeeded";
        receipts.push(receipt);
        if !ok {
            failed.push(command.clone());
        }
    }
    let out = serde_json::json!({
        "profile": profile,
        "project_tests_run": true,
        "receipts": receipts,
        "failed_commands": failed,
        "passed": failed.is_empty(),
    });
    print_json(&out);
    Ok(if failed.is_empty() { 0 } else { 1 })
}

// ── log / status ─────────────────────────────────────────────────────────

fn cmd_log(
    r#type: Option<String>,
    task: Option<String>,
    scope: Option<String>,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let events = EvidenceLog::new(binding.evidence_dir).read_all()?;
    let filtered: Vec<_> = events
        .iter()
        .filter(|e| r#type.as_deref().map(|t| e.event_type == t).unwrap_or(true))
        .filter(|e| {
            task.as_deref()
                .map(|t| e.task_card_hash.as_deref() == Some(t))
                .unwrap_or(true)
        })
        .filter(|e| scope.as_deref().map(|s| e.scope == s).unwrap_or(true))
        .collect();
    for event in filtered {
        println!(
            "{} {} type={} scope={} task={} payload={}",
            event.ts,
            event.event_id,
            event.event_type,
            event.scope,
            event.task_card_hash.as_deref().unwrap_or("-"),
            event.payload
        );
    }
    Ok(0)
}

fn cmd_status(workspace: Option<PathBuf>) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let events = EvidenceLog::new(binding.evidence_dir.clone()).read_all()?;
    let chain = EvidenceLog::verify_chain(&events);
    // Derived task state per task_card_hash (evidence is the single source
    // of truth; no state file is kept).
    let mut by_task: std::collections::BTreeMap<String, Vec<ags_kernel::evidence::Event>> =
        std::collections::BTreeMap::new();
    for event in &events {
        if let Some(hash) = &event.task_card_hash {
            by_task.entry(hash.clone()).or_default().push(event.clone());
        }
    }
    for (hash, task_events) in &by_task {
        println!(
            "task {hash}: {}",
            ags_task_contract::derive_state(task_events)
        );
    }
    for event in events.iter().rev().take(10).rev() {
        println!("{} {} {}", event.ts, event.event_type, event.event_id);
    }
    match chain {
        Ok(()) => println!("evidence chain: ok"),
        Err(e) => println!("evidence chain: BROKEN ({e})"),
    }
    Ok(0)
}

// ── doctor ───────────────────────────────────────────────────────────────

fn cmd_doctor(workspace: Option<PathBuf>) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let config = Config::load(&root)?;
    let lint = config.lint();
    let hosts = ags_kernel::hosts::hook_health(&root, &config.hosts);
    let routes = CapabilitiesLock::load(&binding)?.check_routes(&root);
    // Doctor reports real health: a failed evidence read or drift probe is a
    // finding, never silently treated as "empty/healthy".
    let evidence_read = EvidenceLog::new(binding.evidence_dir.clone()).read_all();
    let chain_ok = evidence_read
        .as_ref()
        .map(|events| EvidenceLog::verify_chain(events).is_ok())
        .unwrap_or(false);
    let evidence_error = evidence_read.err().map(|e| e.message);
    let capability_audit_clean = routes.iter().all(|r| r.status == "exact");
    let install_ok = ags_kernel::sync::install_info().is_ok();
    let (entry_drift, drift_error) = match ags_kernel::sync::drift_report() {
        Ok(d) => (d, None),
        Err(e) => (Vec::new(), Some(e.message)),
    };
    let (bodies_drift, bodies_error) = match ags_kernel::sync::bodies_drift() {
        Ok(d) => (d, None),
        Err(e) => (Vec::new(), Some(e.message)),
    };
    let (git_projection_drift, git_projection_error) =
        match ags_kernel::git_projection::drift(&root) {
            Ok(d) => (d, None),
            Err(e) => (Vec::new(), Some(e.message)),
        };
    let healthy = lint.is_empty()
        && !hosts.is_empty()
        && hosts.iter().all(|h| h.wired)
        && chain_ok
        && install_ok
        && drift_error.is_none()
        && bodies_error.is_none()
        && git_projection_error.is_none()
        && entry_drift.is_empty()
        && bodies_drift.is_empty()
        && git_projection_drift.is_empty();
    let experience = ags_kernel::host_projection::experience_status(&root, &config)?;
    let experience_healthy = experience
        .get("healthy")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let git_projection_repair = (!git_projection_drift.is_empty()
        || git_projection_error.is_some())
    .then(|| format!("ags update --workspace {:?}", root));
    let out = serde_json::json!({
        "version": env!("AGS_PRODUCT_VERSION"),
        "build": env!("AGS_BUILD_ID"),
        "build_identity": env!("AGS_BUILD_DISPLAY"),
        "healthy": healthy,
        "core_healthy": healthy,
        "experience_healthy": experience_healthy,
        "experience": experience,
        "install_ok": install_ok,
        "hosts_configured": !hosts.is_empty(),
        "lint_findings": lint,
        "hosts": hosts,
        "capability_routes": routes,
        "capability_audit_clean": capability_audit_clean,
        "evidence_chain_ok": chain_ok,
        "evidence_error": evidence_error,
        "entry_drift": entry_drift,
        "drift_error": drift_error,
        "third_party_bodies_drift": bodies_drift,
        "bodies_drift_error": bodies_error,
        "git_projection_drift": git_projection_drift,
        "git_projection_error": git_projection_error,
        "git_projection_repair": git_projection_repair,
    });
    print_json(&out);
    Ok(if healthy { 0 } else { 1 })
}

// ── skill lifecycle views ────────────────────────────────────────────────

fn cmd_setup(source_root: Option<PathBuf>) -> Result<i32> {
    let source_root = source_root
        .or_else(|| std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from))
        .ok_or_else(|| {
            ags_kernel::error::Error::new(
                "setup_source_required",
                "pass --source-root <checkout-or-release-runtime>",
            )
        })?;
    let wrote = ags_kernel::sync::setup(&source_root)?;
    print_json(&serde_json::json!({
        "installed": true,
        "source_root": source_root,
        "writes": wrote,
    }));
    Ok(0)
}

fn cmd_skill_list() -> Result<i32> {
    let rows = ags_kernel::skills::list_installed()?;
    print_json(&serde_json::json!({
        "count": rows.len(),
        "skills": rows,
    }));
    Ok(0)
}

fn cmd_skill_recommend(query: Option<String>) -> Result<i32> {
    let rows = ags_kernel::skills::recommendations(query.as_deref())?;
    print_json(&serde_json::json!({
        "query": query,
        "count": rows.len(),
        "skills": rows,
    }));
    Ok(0)
}

// ── update / govern (sealed) ─────────────────────────────────────────────

fn cmd_update(workspace: Option<PathBuf>, sources: Vec<String>) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let payload = serde_json::json!({ "sources": sources });
    seal_and_print("update", &payload, &binding)
}

fn cmd_skill_install(
    skill_id: String,
    path: String,
    acknowledged_risks: Vec<String>,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    // Detection before sealing: report where the skill already lives so the
    // caller decides (AGS-installed = done; host-owned = informational).
    match ags_kernel::sync::detect_skill_anywhere(&skill_id) {
        Ok((true, _)) => println!("skill `{skill_id}` already installed in ~/.agents/skills"),
        Ok((false, hosts)) if !hosts.is_empty() => println!(
            "skill `{skill_id}` found only in host dirs {} — not AGS-installed",
            hosts.join(", ")
        ),
        Ok((false, _)) => println!("skill `{skill_id}` not found on this machine"),
        Err(e) => println!("detection skipped: {}", e.message),
    }
    let payload = ags_kernel::skill_adoption::prepare_install(
        &binding,
        &skill_id,
        &path,
        &acknowledged_risks,
    )?;
    if payload.get("ready").and_then(serde_json::Value::as_bool) != Some(true) {
        print_json(&payload);
        return Ok(2);
    }
    seal_and_print("govern.skill.install", &payload, &binding)
}

fn cmd_skill_remove(skill_id: String, workspace: Option<PathBuf>) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let payload = serde_json::json!({ "skill_id": skill_id });
    seal_and_print("govern.skill.remove", &payload, &binding)
}

fn cmd_host_projection(
    mode: String,
    host: Option<String>,
    surface: String,
    lifecycle: String,
    slug: Option<String>,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let payload = serde_json::json!({
        "mode": mode,
        "host": host,
        "surface": surface,
        "lifecycle": lifecycle,
        "slug": slug,
    });
    seal_and_print("govern.host_projection", &payload, &binding)
}

fn cmd_host_register(
    id: String,
    surface: String,
    dispatch: bool,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let payload = serde_json::json!({
        "id": id,
        "surface": surface,
        "dispatch": dispatch,
    });
    seal_and_print("govern.host.register", &payload, &binding)
}

#[allow(clippy::too_many_arguments)]
fn cmd_delegation_issue(
    parent: String,
    target: String,
    subtask: String,
    allowed_resources: Vec<String>,
    allowed_capabilities: Vec<String>,
    depth: u32,
    return_contract: String,
    owner_instance: String,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let payload = serde_json::json!({
        "parent_contract": parent,
        "target_agent": target,
        "subtask": subtask,
        "allowed_resources": allowed_resources,
        "allowed_capabilities": allowed_capabilities,
        "delegation_depth": depth,
        "return_contract": return_contract,
        "owner_instance": owner_instance,
    });
    seal_and_print("govern.delegation.issue", &payload, &binding)
}

fn cmd_delegation_accept(
    grant: String,
    instance: String,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let event = ags_kernel::delegation::accept(&binding, &grant, &instance)?;
    print_json(&serde_json::json!({
        "grant": grant,
        "instance": instance,
        "state": "ACCEPTED",
        "evidence_event": event.event_id,
    }));
    Ok(0)
}

fn cmd_delegation_return(
    grant: String,
    instance: String,
    summary: String,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let summary_value: serde_json::Value = serde_json::from_str(&summary)
        .map_err(|e| Error::new("delegation_summary_invalid", e.to_string()))?;
    let event = ags_kernel::delegation::return_result(&binding, &grant, &instance, summary_value)?;
    print_json(&serde_json::json!({
        "grant": grant,
        "instance": instance,
        "state": "RETURNED",
        "evidence_event": event.event_id,
    }));
    Ok(0)
}

fn cmd_delegation_integrate(
    grant: String,
    instance: String,
    workspace: Option<PathBuf>,
) -> Result<i32> {
    let root = resolve_root(workspace)?;
    let binding = workspace::bind(&root)?;
    let event = ags_kernel::delegation::integrate(&binding, &grant, &instance)?;
    print_json(&serde_json::json!({
        "grant": grant,
        "instance": instance,
        "state": "INTEGRATED",
        "evidence_event": event.event_id,
    }));
    Ok(0)
}

fn seal_and_print(
    operation: &str,
    payload: &serde_json::Value,
    binding: &WorkspaceBinding,
) -> Result<i32> {
    let store = SealStore::new(binding);
    let action = store.seal_plan(operation, payload, binding)?;
    println!(
        "{} sealed: {}\nrun `ags apply {} --workspace {}` to apply once",
        operation,
        action.plan_hash,
        action.token,
        binding.root.display()
    );
    Ok(0)
}

// ── route ────────────────────────────────────────────────────────────────

fn cmd_route(input: String, workspace: Option<PathBuf>) -> Result<i32> {
    let view = ags_kernel::route::load_route_view()?;
    let result = ags_kernel::route::match_route(&view, &input);
    // Hit accounting: every route query lands in the evidence log so
    // `ags log` shows which skills get used and which rot.
    if let Some(root) = workspace.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| ags_kernel::workspace::find_workspace(&cwd).ok())
    }) {
        if let Ok(binding) = ags_kernel::workspace::bind(&root) {
            let log = ags_kernel::evidence::EvidenceLog::new(binding.evidence_dir.clone());
            let _ = log.append(
                "route",
                &binding.slug,
                None,
                "local",
                serde_json::json!({
                    "input": input,
                    "skill": result.skill,
                    "candidate": result.candidate,
                    "ambiguous": result.ambiguous,
                    "hits": result.hits,
                    "verified": result.verified,
                }),
            );
        }
    }
    let out = serde_json::json!(result);
    print_json(&out);
    Ok(if result.skill.is_some() && !result.ambiguous {
        0
    } else {
        2
    })
}

// ── schema ───────────────────────────────────────────────────────────────

/// Operation shapes: name → (description, payload example). The registry only
/// contains this sealed subset (G-06); read-only commands are plain CLI.
const OPERATION_SHAPES: &[(&str, &str, &str)] = &[
    (
        "init",
        "Adopt a workspace: ags.toml + ownership manifest (no user file overwritten).",
        r#"{"root": "<absolute path>", "slug": "<id>", "ags_toml": "<toml text>"}"#,
    ),
    (
        "update",
        "Refresh project audit leaves, official skills, rules, entries, and the machine lock.",
        r#"{"sources": ["ags-skills"]}"#,
    ),
    (
        "govern.skill.install",
        "Audit and install one local Skill as an immutable machine body.",
        r#"{"skill_id": "demo", "path": "skill-packs/demo", "source_sha256": "<planned hash>", "acknowledged_risks": []}"#,
    ),
    (
        "govern.skill.remove",
        "Uninstall one AGS-managed machine Skill symlink and remove its project audit declaration.",
        r#"{"skill_id": "demo"}"#,
    ),
    (
        "govern.host.register",
        "Register any host through the generic CLI or MCP transport.",
        r#"{"id": "future-host", "surface": "cli|mcp", "dispatch": true}"#,
    ),
    (
        "govern.host_projection",
        "Reconcile host identity, canonical ags CLI/MCP connection and lifecycle hooks.",
        r#"{"mode": "reconcile", "host": "codex", "surface": "mcp", "lifecycle": "full", "slug": "ai-workstation"}"#,
    ),
    (
        "govern.delegation.issue",
        "Issue a narrowed DelegationGrant to a child agent (sealed at issuance).",
        r#"{"parent_contract": "<task hash>", "target_agent": "codebuddy-child", "subtask": "...", "allowed_resources": ["src/db"], "allowed_capabilities": ["skill:database-migration"], "delegation_depth": 1, "return_contract": "...", "owner_instance": "<parent instance>"}"#,
    ),
];

fn cmd_schema(operation: Option<String>, format: Option<String>) -> Result<i32> {
    let out = match operation {
        Some(op) => match OPERATION_SHAPES.iter().find(|(name, _, _)| *name == op) {
            Some((name, description, payload)) => serde_json::json!({
                "contract": "v3",
                "operation": name,
                "kind": "transaction",
                "risk": "write",
                "description": description,
                "payload_example": payload,
                "flow": "decide seals a plan; `ags apply <ACTION_REF>` commits it exactly once",
            }),
            None => {
                return Err(Error::new(
                    "operation_unknown",
                    format!("`{op}` is not in the sealed registry (ags schema for the full list)"),
                ))
            }
        },
        None => serde_json::json!({
            "contract": "v3",
            "commands": ["init", "run", "apply", "check", "test", "log", "status", "doctor", "update", "govern", "schema"],
            "sealed_operations": ags_kernel::config::CANONICAL_SEALED_OPS,
            "seal_states": ags_kernel::seal::SEAL_STATES,
            "hooks": {
                "binary": "ags-policy",
                "events": ["pretooluse", "permissionrequest", "posttooluse", "sessionstart", "sessionend"],
                "decisions": ["allow", "ask", "deny", "sealed"],
            },
        }),
    };
    if format.as_deref() == Some("json") {
        print_json(&out);
    } else {
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_sealed_subset_only() {
        // The registry contains exactly the canonical sealed subset (G-06).
        assert_eq!(
            ags_kernel::config::CANONICAL_SEALED_OPS,
            &[
                "govern.skill.install",
                "govern.skill.remove",
                "govern.host.register",
                "govern.host_projection",
                "govern.delegation.issue",
                "update"
            ]
        );
    }

    #[test]
    fn cli_and_mcp_agree_on_sealed_decisions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"govern.skill.install\", \"govern.skill.remove\", \"govern.host.register\", \"govern.host_projection\", \"govern.delegation.issue\", \"update\"]\n",
        )
        .unwrap();
        let mut config = Config::load(&root).unwrap();
        config.guardrails.protected_resources = vec![".ags".to_string()];
        // Both surfaces decide through the same kernel matrix: a sealed op
        // seals a plan, anything else is unknown.
        assert_eq!(
            ags_kernel::matrix::evaluate_op(&config, "govern.delegation.issue"),
            ags_kernel::matrix::Decision::Sealed
        );
        assert_ne!(
            ags_kernel::matrix::evaluate_op(&config, "unknown.op"),
            ags_kernel::matrix::Decision::Sealed
        );
        // CLI (seal_and_print) and MCP (decide) build the identical
        // action_ref for the same plan.
        let binding = workspace::bind(&root).unwrap();
        let store = SealStore::new(&binding);
        let payload = serde_json::json!({
            "parent_contract": "tc-0123456789abcdef",
            "target_agent": "codebuddy",
            "subtask": "s",
            "allowed_resources": [],
            "allowed_capabilities": [],
            "delegation_depth": 1,
            "return_contract": "r",
            "owner_instance": "owner-1",
        });
        let action = store
            .seal_plan("govern.delegation.issue", &payload, &binding)
            .unwrap();
        assert_eq!(action.operation, "govern.delegation.issue");
        assert_eq!(action.token.len(), 64);
        // Effect-level decisions agree with the policy hook: the guardrails
        // evaluation is the same function on both surfaces.
        use ags_kernel::govern::{ActionIntent, Effect};
        let intent = ActionIntent {
            actor: None,
            task: None,
            effect: Effect::WorkspaceWrite,
            resource: Some(".ags/x".to_string()),
            capability: None,
            externality: None,
        };
        assert_eq!(
            ags_kernel::govern::evaluate_guardrails(&config, &intent),
            ags_kernel::matrix::Decision::Deny
        );
    }

    #[test]
    fn resolve_root_finds_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n",
        )
        .unwrap();
        let found = resolve_root(Some(root.join("sub"))).unwrap();
        assert_eq!(found, root);
    }

    #[test]
    fn unknown_scope_is_rejected() {
        let err = cmd_check("nonsense".to_string(), None, None).unwrap_err();
        assert_eq!(err.code, "check_scope_unknown");
    }

    /// CLI end-to-end sealed loop for a non-init operation: seal with the
    /// real binding, apply with the real binding (regression test for the
    /// binding-identity asymmetry the independent review found).
    #[test]
    fn cli_seal_apply_roundtrip_for_update() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(root.join("ags-skills/demo")).unwrap();
        std::fs::write(root.join("ags-skills/demo/SKILL.md"), "# Demo\n").unwrap();
        std::fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"govern.skill.install\", \"govern.skill.remove\", \"govern.host.register\", \"govern.host_projection\", \"govern.delegation.issue\", \"update\"]\n\n[verify]\nprofile = \"smoke\"\n",
        )
        .unwrap();
        // update now requires a machine install record; mirror `ags setup`.
        let source = tmp.path().join("source");
        std::fs::create_dir_all(source.join("ags-skills/ags-demo")).unwrap();
        std::fs::write(
            source.join("ags-skills/ags-demo/SKILL.md"),
            "---\nname: ags-demo\ndescription: Official.\n---\n",
        )
        .unwrap();
        ags_kernel::sync::setup(&source).unwrap();
        let binding = workspace::bind(&root).unwrap();
        let store = SealStore::new(&binding);
        let action = store
            .seal_plan(
                "update",
                &serde_json::json!({"sources": ["ags-skills"]}),
                &binding,
            )
            .unwrap();
        let exit = cmd_apply(&action.token, Some(root.clone())).unwrap();
        assert_eq!(exit, 0);
        let lock = CapabilitiesLock::load(&binding).unwrap();
        assert_eq!(lock.entries.len(), 1);
        assert_eq!(lock.entries[0].id, "demo");
    }
}
