//! Static capability catalog and snapshot facade.

use crate::cli::CapabilityAction;
use std::path::{Path, PathBuf};

const DEFAULT_HOSTS: &[&str] = &["claude-code", "codex", "omp", "codebuddy-code", "cursor"];

pub(crate) fn refresh_skill_snapshot(
    authority_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<PathBuf, String> {
    let snapshot = ags_capability_governance::build_capability_snapshot_with_runtime_home(
        authority_root,
        active_host,
        runtime_home,
    )
    .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
    let path = ags_capability_governance::snapshot_path(runtime_home, active_host);
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
    ags_capability_governance::write_private_atomic(&path, (json + "\n").as_bytes())?;
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
    match format {
        "json" => println!("{}", console::render_inventory_json(&result)),
        _ => println!("{}", console::render_inventory_text(&result)),
    }
}

pub(crate) fn cmd_capability_verify(host: &str, strict: bool, format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = crate::context::capability_authority_root_or_exit("ags capability verify");
    let context = console::ConsoleContext::system(root);
    let result = console::verify_host(&context, host);
    match format {
        "json" => println!("{}", console::render_verify_json(&result)),
        _ => println!("{}", console::render_verify_text(&result)),
    }
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
        &ags_capability_governance::locate_runtime_home(),
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    )
    .unwrap_or_else(|detail| {
        eprintln!("ags capability snapshot: refused — {detail}");
        std::process::exit(1);
    });
    let runtime_home = ags_capability_governance::locate_runtime_home();
    let built = ags_capability_governance::build_capability_snapshot_with_runtime_home(
        &root,
        host,
        &runtime_home,
    )
    .unwrap_or_else(|error| {
        eprintln!("ags capability snapshot: build failed — {error:?}");
        std::process::exit(1);
    });
    let written = write
        .then(|| refresh_skill_snapshot(&root, &runtime_home, host))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("ags capability snapshot: {error}");
            std::process::exit(1);
        });

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&built).unwrap_or_default()
        ),
        _ => {
            println!("Static capability snapshot");
            println!("Host: {}", built.host);
            println!("Snapshot hash: {}", built.snapshot_hash);
            println!("Active skills: {}", built.active_skills.len());
            match written {
                Some(path) => println!("Written: {}", path.display()),
                None => println!("Dry-run: pass --write during an explicit update."),
            }
        }
    }
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
