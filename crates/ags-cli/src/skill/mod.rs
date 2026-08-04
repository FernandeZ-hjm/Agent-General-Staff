//! Skill catalog and machine-private adoption facade.

use crate::cli::SkillAction;
use ags_capability_governance::skill_adoption::{
    apply_adoption, apply_removal, inspect_adoption, plan_adoption, plan_removal, AdoptionContext,
    AdoptionPlan, AdoptionReceipt,
};
use std::path::Path;

fn authority(command: &str) -> std::path::PathBuf {
    crate::context::capability_authority_root_or_exit(command)
}

fn verify(host: &str, strict: bool, format: &str) {
    use ags_capability_governance::skill_body::console;

    let context = console::ConsoleContext::system(authority("ags skill verify"));
    let result = console::verify_host(&context, host);
    crate::output::emit_rendered(
        format,
        || console::render_verify_json(&result),
        || console::render_verify_text(&result),
    );
    if strict && result.status != "ok" {
        std::process::exit(1);
    }
}

fn inventory(format: &str) {
    let result = ags_capability_governance::skill_body::scan_skill_inventory(&authority(
        "ags skill inventory",
    ));
    crate::output::emit_rendered(
        format,
        || ags_capability_governance::skill_body::render_inventory_json(&result),
        || ags_capability_governance::skill_body::render_inventory_text(&result),
    );
}

fn adoption_context(command: &str) -> AdoptionContext {
    AdoptionContext {
        authority_root: authority(command),
        runtime_home: ags_capability_governance::locate_runtime_home(),
        host_home: ags_platform::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
        snapshot_discovery: ags_capability_governance::skill_adoption::SnapshotDiscovery::Live,
    }
}

fn adopt(
    source: &Path,
    metadata: Option<&Path>,
    hosts: &[String],
    plan_hash: Option<&str>,
    yes: bool,
    format: &str,
) {
    let context = adoption_context("ags skill adopt");
    if yes {
        let reviewed = plan_hash.unwrap_or_else(|| {
            eprintln!("ags skill adopt: refused — --plan-hash is required with --yes");
            std::process::exit(2);
        });
        let receipt =
            apply_adoption(&context, source, metadata, hosts, reviewed).unwrap_or_else(|error| {
                eprintln!("ags skill adopt: refused — {error}");
                std::process::exit(1);
            });
        emit_receipt(&receipt, format);
    } else {
        let plan = plan_adoption(&context, source, metadata, hosts).unwrap_or_else(|error| {
            eprintln!("ags skill adopt: refused — {error}");
            std::process::exit(1);
        });
        emit_plan(&plan, format);
    }
}

fn remove(skill_id: &str, plan_hash: Option<&str>, yes: bool, format: &str) {
    let context = adoption_context("ags skill remove");
    if yes {
        let reviewed = plan_hash.unwrap_or_else(|| {
            eprintln!("ags skill remove: refused — --plan-hash is required with --yes");
            std::process::exit(2);
        });
        let receipt = apply_removal(&context, skill_id, reviewed).unwrap_or_else(|error| {
            eprintln!("ags skill remove: refused — {error}");
            std::process::exit(1);
        });
        emit_receipt(&receipt, format);
    } else {
        let plan = plan_removal(&context, skill_id).unwrap_or_else(|error| {
            eprintln!("ags skill remove: refused — {error}");
            std::process::exit(1);
        });
        emit_plan(&plan, format);
    }
}

fn status(skill_id: &str, format: &str) {
    let context = adoption_context("ags skill status");
    let status = inspect_adoption(&context.runtime_home, &context.host_home, skill_id)
        .unwrap_or_else(|error| {
            eprintln!("ags skill status: {error}");
            std::process::exit(1);
        });
    crate::output::emit(format, &status, || {
        format!(
            "Private Skill adoption\nSkill: {}\nRegistered: {}\nBody: present={} hash_matches={}\nVisible hosts: {}\nActive hosts: {}",
            status.skill_id,
            status.registered,
            status.body_present,
            status.body_hash_matches,
            status.visible_hosts.join(","),
            status.active_hosts.join(",")
        )
    });
}

fn emit_plan(plan: &AdoptionPlan, format: &str) {
    crate::output::emit(format, plan, || {
        format!(
            "Machine-private Skill {} plan\nSkill: {}\nSource: {}\nSource hash: {}\nTargets: {}\nPlan hash: {}\nDry-run only — review, then pass --yes --plan-hash <hash>.",
            plan.operation,
            plan.skill_id,
            plan.source,
            plan.source_hash,
            plan.target_hosts.join(","),
            plan.plan_hash
        )
    });
}

fn emit_receipt(receipt: &AdoptionReceipt, format: &str) {
    crate::output::emit(format, receipt, || {
        format!(
            "Machine-private Skill {} applied\nSkill: {}\nRegistry revision: {}\nBody: {}\nHosts: {}\nRestart AGS MCP and re-run preflight before routing.",
            receipt.operation,
            receipt.skill_id,
            receipt.registry_revision,
            receipt.body_path,
            receipt.snapshot_hashes.keys().cloned().collect::<Vec<_>>().join(",")
        )
    });
}

fn overview(format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = authority("ags skill");
    let context = console::ConsoleContext::system(root.clone());
    let inventory = console::build_inventory(
        &context,
        &["claude-code", "codex", "omp", "codebuddy-code", "cursor"],
    );
    let output = serde_json::json!({
        "schema_version": console::CONSOLE_SCHEMA_VERSION,
        "inventory": inventory,
        "update_policy": "explicit_snapshot_refresh_only"
    });
    crate::output::emit(format, &output, || {
        format!(
            "{}\n\nRequest routing consumes static snapshots. Official catalog refreshes use AGS update; audited third-party bodies use explicit `ags skill adopt` plan/apply and remain machine-private.",
            console::render_inventory_text(&inventory)
        )
    });
}

pub(crate) fn run(action: Option<SkillAction>, format: &str) {
    match action {
        Some(SkillAction::Adopt {
            source,
            metadata,
            host,
            plan_hash,
            yes,
            format,
        }) => adopt(
            &source,
            metadata.as_deref(),
            &host,
            plan_hash.as_deref(),
            yes,
            &format,
        ),
        Some(SkillAction::Remove {
            skill_id,
            plan_hash,
            yes,
            format,
        }) => remove(&skill_id, plan_hash.as_deref(), yes, &format),
        Some(SkillAction::Status { skill_id, format }) => status(&skill_id, &format),
        Some(SkillAction::Verify {
            host,
            strict,
            format,
        }) => verify(&host, strict, &format),
        Some(SkillAction::Inventory { format }) => inventory(&format),
        None => overview(format),
    }
}
