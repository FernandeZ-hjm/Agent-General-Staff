use crate::cli::SyncAction;
use std::path::PathBuf;

/// Shared dispatch: `sync check` / `workflow-sync-check`
pub(crate) fn cmd_sync_check(
    source: PathBuf,
    targets: Vec<(String, PathBuf)>,
    target: Option<PathBuf>,
    target_name: String,
    allowlist: Option<PathBuf>,
    format: &str,
) {
    let mut all_targets = targets;

    // Backward compat: --target adds a single target
    if let Some(target_root) = target {
        all_targets.push((target_name, target_root));
    }

    // Default: if no targets specified, use stable as default
    if all_targets.is_empty() {
        all_targets.push((
            "stable".to_string(),
            PathBuf::from(workflow_sync_check::DEFAULT_STABLE_ROOT),
        ));
    }

    let target_configs: Vec<workflow_sync_check::TargetConfig> = all_targets
        .into_iter()
        .map(|(name, root)| {
            let kind = match name.as_str() {
                "stable" => workflow_sync_check::ProjectKind::Stable,
                "public"
                | "public-core"
                | "public-core-only"
                | "public-full"
                | "public-full-sanitized" => workflow_sync_check::ProjectKind::PublicCoreOnly,
                _ => workflow_sync_check::ProjectKind::Custom(name.clone()),
            };
            workflow_sync_check::TargetConfig { root, name, kind }
        })
        .collect();

    let report_format = match format {
        "json" => workflow_sync_check::ReportFormat::Json,
        _ => workflow_sync_check::ReportFormat::Text,
    };

    let options = workflow_sync_check::CheckOptions {
        source_root: source,
        source_name: "private".to_string(),
        targets: target_configs,
        allowlist_path: allowlist,
    };

    let ok = workflow_sync_check::run_cli(options, report_format);
    if !ok {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: SyncAction) {
    match action {
        SyncAction::Check {
            source,
            targets,
            target,
            target_name,
            allowlist,
            format,
        } => cmd_sync_check(source, targets, target, target_name, allowlist, &format),
    }
}
