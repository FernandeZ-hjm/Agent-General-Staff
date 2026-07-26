//! Shared environment / path / guard / version helpers.

use std::path::{Path, PathBuf};

/// User-facing AGS product version.
pub(crate) const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn guard_writable_target(command: &str, target: &Path) {
    let target_path = guard_path(target);
    let protected_roots = std::env::var_os("AGS_PROTECTED_ROOTS")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();

    for protected in protected_roots {
        let protected_path = guard_path(&protected);
        if target_path == protected_path || target_path.starts_with(&protected_path) {
            eprintln!(
                "{command}: refused — target is a protected suite path: {}",
                target.display()
            );
            eprintln!("Write-mode operations must target a tempdir or non-A/S/B directory.");
            std::process::exit(1);
        }
    }

    if target_path.join("WORKSPACE.md").exists()
        || target_path.join("AGENT_SUITE_PROTOCOL.md").exists()
    {
        eprintln!(
            "{command}: refused — target appears to be a suite root: {}",
            target.display()
        );
        eprintln!("Write-mode operations must target a tempdir or non-A/S/B directory.");
        std::process::exit(1);
    }
}
pub(crate) fn guard_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }

    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        if let Some(name) = existing.file_name() {
            missing.push(name.to_os_string());
        }
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return absolute,
        }
    }

    let mut normalized = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for component in missing.iter().rev() {
        normalized.push(component);
    }
    normalized
}
pub(crate) fn default_private_runtime_home() -> PathBuf {
    if let Some(path) = std::env::var_os("AGS_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = ags_platform::home_dir() {
        return home.join(".ags").join("private-runtime");
    }
    PathBuf::from(".ags").join("private-runtime")
}
pub(crate) fn private_install_target(target: Option<PathBuf>) -> PathBuf {
    target.unwrap_or_else(default_private_runtime_home)
}
/// Guard: refuse to treat `source_repo` as a private-suite source root unless
/// the canonical bootstrap payload files are all present. Shared by the
/// source-root helper and the bootstrap apply path.
pub(crate) fn ensure_bootstrap_source_repo(source_repo: &Path) {
    let required = [
        "protocol/agent-task-protocol.md",
        "protocol/task-card-template.md",
        "protocol/runtime-adapters.md",
        "protocol/task-routing.md",
        "scripts/validate.sh",
        "scripts/run-task-card.sh",
    ];

    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|rel| !source_repo.join(rel).exists())
        .collect();

    if missing.is_empty() {
        return;
    }

    eprintln!(
        "ags bootstrap --apply: refused — source is not a complete private suite root: {}",
        source_repo.display()
    );
    eprintln!("Missing bootstrap payload source file(s):");
    for rel in missing {
        eprintln!("  - {rel}");
    }
    std::process::exit(1);
}

fn is_complete_source_root(root: &Path) -> bool {
    let required = [
        "protocol/agent-task-protocol.md",
        "protocol/task-card-template.md",
        "protocol/runtime-adapters.md",
        "protocol/task-routing.md",
        "scripts/validate.sh",
        "scripts/run-task-card.sh",
        "crates/ags-cli/Cargo.toml",
    ];
    required.iter().all(|rel| root.join(rel).is_file())
}

fn installed_source_root(runtime_home: &Path) -> Option<PathBuf> {
    let manifest = std::fs::read_to_string(runtime_home.join("install-manifest.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    value
        .get("source_root")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
}

fn resolve_source_root(
    cwd: &Path,
    runtime_home: &Path,
    explicit: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit {
        candidates.push(("AGS_SOURCE_ROOT", path));
    }
    candidates.push(("current directory", cwd.to_path_buf()));
    if let Some(path) = installed_source_root(runtime_home) {
        candidates.push(("runtime install manifest", path));
    }

    let mut tried = Vec::new();
    for (origin, candidate) in candidates {
        let candidate = guard_path(&candidate);
        if is_complete_source_root(&candidate) {
            return Ok(candidate);
        }
        tried.push(format!("{origin}: {}", candidate.display()));
    }
    Err(format!(
        "no complete AGS source root found; checked {}",
        tried.join(", ")
    ))
}

pub(crate) fn resolve_capability_authority_root(
    cwd: &Path,
    runtime_home: &Path,
    explicit: Option<PathBuf>,
) -> Result<PathBuf, String> {
    ags_capability_governance::resolve_capability_authority_root(cwd, runtime_home, explicit)
        .map_err(|error| error.to_string())
}

pub(crate) fn capability_authority_root_or_exit(command: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from);
    match resolve_capability_authority_root(
        &cwd,
        &ags_capability_governance::locate_runtime_home(),
        explicit,
    ) {
        Ok(root) => root,
        Err(detail) => {
            eprintln!("{command}: refused — {detail}");
            eprintln!("Run `ags setup --yes` so capability authority can resolve the installed source root.");
            std::process::exit(1);
        }
    }
}

pub(crate) fn source_root_or_exit(command: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from);
    match resolve_source_root(&cwd, &default_private_runtime_home(), explicit) {
        Ok(root) => root,
        Err(detail) => {
            eprintln!("{command}: refused — {detail}");
            eprintln!(
                "Run `ags setup --yes` from a complete suite checkout or set AGS_SOURCE_ROOT."
            );
            std::process::exit(1);
        }
    }
}
pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn home_dir() -> PathBuf {
    ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod source_root_tests {
    use super::*;

    fn seed_suite(root: &Path) {
        for rel in [
            "manifests/skills-registry.yaml",
            "manifests/mcp-registry.yaml",
            "protocol/agent-task-protocol.md",
            "protocol/task-card-template.md",
            "protocol/runtime-adapters.md",
            "protocol/task-routing.md",
            "scripts/validate.sh",
            "scripts/run-task-card.sh",
            "crates/ags-cli/Cargo.toml",
        ] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fixture\n").unwrap();
        }
    }

    #[test]
    fn source_root_resolves_from_install_manifest_outside_suite_cwd() {
        let base =
            std::env::temp_dir().join(format!("ags-source-root-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cwd = base.join("managed-project");
        let suite = base.join("stable-suite");
        let runtime = base.join("runtime");
        std::fs::create_dir_all(&cwd).unwrap();
        seed_suite(&suite);
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(
            runtime.join("install-manifest.json"),
            serde_json::json!({"source_root": suite.display().to_string()}).to_string(),
        )
        .unwrap();

        assert_eq!(
            resolve_source_root(&cwd, &runtime, None).unwrap(),
            std::fs::canonicalize(&suite).unwrap()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_authority_resolves_from_install_manifest_outside_suite_cwd() {
        let base =
            std::env::temp_dir().join(format!("ags-capability-authority-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let suite = base.join("suite");
        let project = base.join("managed-project");
        let runtime = base.join("runtime");
        seed_suite(&suite);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(
            runtime.join("install-manifest.json"),
            serde_json::json!({"source_root": suite.display().to_string()}).to_string(),
        )
        .unwrap();

        assert_eq!(
            resolve_capability_authority_root(&project, &runtime, None).unwrap(),
            guard_path(&suite)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_authority_prefers_install_manifest_over_complete_suite_cwd() {
        let base = std::env::temp_dir().join(format!(
            "ags-capability-installed-authority-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let installed = base.join("installed-suite");
        let other_checkout = base.join("other-suite-checkout");
        let runtime = base.join("runtime");
        seed_suite(&installed);
        seed_suite(&other_checkout);
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(
            runtime.join("install-manifest.json"),
            serde_json::json!({"source_root": installed.display().to_string()}).to_string(),
        )
        .unwrap();

        assert_eq!(
            resolve_capability_authority_root(&other_checkout, &runtime, None).unwrap(),
            guard_path(&installed)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn source_root_prefers_explicit_environment_candidate() {
        let base = std::env::temp_dir().join(format!("ags-source-root-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cwd = base.join("managed-project");
        let suite = base.join("explicit-suite");
        let runtime = base.join("runtime");
        std::fs::create_dir_all(&cwd).unwrap();
        seed_suite(&suite);

        assert_eq!(
            resolve_source_root(&cwd, &runtime, Some(suite.clone())).unwrap(),
            std::fs::canonicalize(&suite).unwrap()
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
