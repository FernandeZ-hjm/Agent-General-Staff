//! Read-only skill catalog facade.

use crate::cli::SkillAction;

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
            "{}\n\nCatalog is static. Refresh only during an explicit AGS/skill update, then run `ags capability snapshot --write --host <host>`.",
            console::render_inventory_text(&inventory)
        )
    });
}

pub(crate) fn run(action: Option<SkillAction>, format: &str) {
    match action {
        Some(SkillAction::Verify {
            host,
            strict,
            format,
        }) => verify(&host, strict, &format),
        Some(SkillAction::Inventory { format }) => inventory(&format),
        None => overview(format),
    }
}
