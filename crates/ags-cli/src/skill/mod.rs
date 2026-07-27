//! Read-only skill catalog facade.

use crate::cli::SkillAction;

fn authority(command: &str) -> std::path::PathBuf {
    crate::context::capability_authority_root_or_exit(command)
}

fn verify(host: &str, strict: bool, format: &str) {
    use ags_capability_governance::skill_body::console;

    let context = console::ConsoleContext::system(authority("ags skill verify"));
    let result = console::verify_host(&context, host);
    match format {
        "json" => println!("{}", console::render_verify_json(&result)),
        _ => println!("{}", console::render_verify_text(&result)),
    }
    if strict && result.status != "ok" {
        std::process::exit(1);
    }
}

fn inventory(format: &str) {
    let result = ags_capability_governance::skill_body::scan_skill_inventory(&authority(
        "ags skill inventory",
    ));
    match format {
        "json" => println!(
            "{}",
            ags_capability_governance::skill_body::render_inventory_json(&result)
        ),
        _ => println!(
            "{}",
            ags_capability_governance::skill_body::render_inventory_text(&result)
        ),
    }
}

fn overview(format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = authority("ags skill");
    let context = console::ConsoleContext::system(root.clone());
    let inventory = console::build_inventory(
        &context,
        &["claude-code", "codex", "omp", "codebuddy-code", "cursor"],
    );
    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": console::CONSOLE_SCHEMA_VERSION,
                "inventory": inventory,
                "update_policy": "explicit_snapshot_refresh_only"
            }))
            .unwrap_or_default()
        ),
        _ => {
            println!("{}", console::render_inventory_text(&inventory));
            println!();
            println!(
                "Catalog is static. Refresh only during an explicit AGS/skill update, then run `ags capability snapshot --write --host <host>`."
            );
        }
    }
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
