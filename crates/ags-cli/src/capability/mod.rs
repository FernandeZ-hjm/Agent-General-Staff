//! Static capability catalog and snapshot facade.

use crate::cli::CapabilityAction;
use std::path::{Path, PathBuf};

const DEFAULT_HOSTS: &[&str] = &["claude-code", "codex", "omp", "codebuddy-code", "cursor"];

fn write_capability_snapshot(
    runtime_home: &Path,
    active_host: &str,
    snapshot: &ags_capability_governance::HostCapabilitySnapshot,
) -> Result<PathBuf, String> {
    let path = ags_capability_governance::snapshot_path(runtime_home, active_host);
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|error| format!("capability snapshot serialization failed: {error}"))?;
    ags_platform::atomic_write(&path, (json + "\n").as_bytes())?;
    Ok(path)
}

fn inventory(hosts: &[String], format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = crate::context::capability_authority_root_or_exit("ags capability inventory");
    let context = console::ConsoleContext::system(root);
    let requested = if hosts.is_empty() {
        DEFAULT_HOSTS.to_vec()
    } else {
        hosts.iter().map(String::as_str).collect()
    };
    let result = console::build_inventory(&context, &requested);
    crate::output::emit_rendered(
        format,
        || console::render_inventory_json(&result),
        || console::render_inventory_text(&result),
    );
}

pub(crate) fn cmd_capability_verify(host: &str, strict: bool, format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = crate::context::capability_authority_root_or_exit("ags capability verify");
    let context = console::ConsoleContext::system(root);
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

fn snapshot(host: &str, target: &Path, write: bool, format: &str) {
    let requested = if target.as_os_str().is_empty() || target == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        target.to_path_buf()
    };
    let root = crate::context::resolve_capability_authority_root(
        &requested,
        &ags_platform::runtime_home(),
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    )
    .unwrap_or_else(|detail| {
        eprintln!("ags capability snapshot: refused — {detail}");
        std::process::exit(1);
    });
    let runtime_home = ags_platform::runtime_home();
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let built = ags_capability_governance::build_capability_snapshot_with_live_roots_at(
        &root,
        host,
        &runtime_home,
        &host_home,
        &requested,
    )
    .unwrap_or_else(|error| {
        eprintln!("ags capability snapshot: build failed — {error:?}");
        std::process::exit(1);
    });
    let written = write
        .then(|| write_capability_snapshot(&runtime_home, host, &built))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("ags capability snapshot: {error}");
            std::process::exit(1);
        });

    crate::output::emit(format, &built, || {
        [
            "Static capability snapshot".to_string(),
            format!("Host: {}", built.host),
            format!("Snapshot hash: {}", built.snapshot_hash),
            format!("Active skills: {}", built.active_skills.len()),
            format!("Active MCPs: {}", built.active_mcps.len()),
            written.as_ref().map_or_else(
                || "Dry-run: pass --write during an explicit update.".to_string(),
                |path| format!("Written: {}", path.display()),
            ),
        ]
        .join("\n")
    });
}

pub(crate) fn run(action: CapabilityAction) {
    match action {
        CapabilityAction::Inventory { host, format } => inventory(&host, &format),
        CapabilityAction::Verify {
            host,
            strict,
            format,
        } => cmd_capability_verify(&host, strict, &format),
        CapabilityAction::Snapshot {
            host,
            target,
            write,
            format,
        } => snapshot(&host, &target, write, &format),
    }
}
