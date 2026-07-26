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
        let stable_root = std::env::var_os("AGS_SYNC_STABLE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                source
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(ags_verification::sync::DEFAULT_STABLE_ROOT)
            });
        all_targets.push(("stable".to_string(), stable_root));
    }

    let target_configs: Vec<ags_verification::sync::TargetConfig> = all_targets
        .into_iter()
        .map(|(name, root)| {
            let kind = match name.as_str() {
                "stable" => ags_verification::sync::ProjectKind::Stable,
                "public"
                | "public-core"
                | "public-core-only"
                | "public-full"
                | "public-full-sanitized" => ags_verification::sync::ProjectKind::PublicCoreOnly,
                _ => ags_verification::sync::ProjectKind::Custom(name.clone()),
            };
            ags_verification::sync::TargetConfig { root, name, kind }
        })
        .collect();

    let report_format = match format {
        "json" => ags_verification::sync::ReportFormat::Json,
        _ => ags_verification::sync::ReportFormat::Text,
    };

    let options = ags_verification::sync::CheckOptions {
        source_root: source,
        source_name: "private".to_string(),
        targets: target_configs,
        allowlist_path: allowlist,
    };

    let ok = ags_verification::sync::run_cli(options, report_format);
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
