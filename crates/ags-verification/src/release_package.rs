//! Canonical release-package discovery and payload planning.

use std::path::Path;
use std::process::Command;

fn matches_path_boundary(relative: &str, boundary: &str) -> bool {
    let relative = relative.trim_start_matches("./").replace('\\', "/");
    let boundary = boundary.trim_start_matches("./").replace('\\', "/");

    if boundary.ends_with('/') {
        let dir = boundary.trim_end_matches('/');
        relative == dir || relative.starts_with(&boundary)
    } else {
        relative == boundary
    }
}
fn is_public_release_profile(profile: &str) -> bool {
    profile == "public-full" || profile == "public-core"
}
fn public_release_forbidden_patterns() -> Vec<&'static str> {
    crate::sync::manifest::PUBLIC_FORBIDDEN_PAYLOAD
        .iter()
        .copied()
        .chain([
            "proposals/",
            "graphify-out/",
            "governance/backups/",
            ".claude/",
            ".codegraph/",
        ])
        .collect()
}
fn walk_release_files(root: &Path, prefix: &str, files: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(root.join(prefix)) {
        for entry in entries.flatten() {
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(&entry.path())
                .to_string_lossy()
                .to_string();
            let file_type = entry.file_type().ok();
            if file_type
                .as_ref()
                .is_some_and(std::fs::FileType::is_symlink)
            {
                files.push(rel);
            } else if file_type.as_ref().is_some_and(std::fs::FileType::is_dir) {
                if rel == ".git" || rel == "target" || rel.starts_with("target/") {
                    continue;
                }
                walk_release_files(root, &rel, files);
            } else {
                files.push(rel);
            }
        }
    }
}
fn git_tracked_release_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("-z")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                let relative = String::from_utf8_lossy(entry).replace('\\', "/");
                match std::fs::symlink_metadata(root.join(&relative)) {
                    Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                        Some(relative)
                    }
                    _ => None,
                }
            })
            .collect(),
    )
}
fn release_file_list(source_root: &Path) -> Vec<String> {
    if let Some(files) = git_tracked_release_files(source_root) {
        return files;
    }

    let mut files = Vec::new();
    walk_release_files(source_root, "", &mut files);
    files
}
pub fn release_package_plan(
    source_root: &Path,
    profile: &str,
    dry_run: bool,
) -> (serde_json::Value, bool) {
    let public_full_forbidden_patterns = public_release_forbidden_patterns();
    let mut included: Vec<String> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut exclusion_reasons: Vec<(String, String)> = Vec::new();

    let mut all_files = release_file_list(source_root);
    all_files.sort();

    let mut required_missing = Vec::new();
    let mut extra_files = Vec::new();
    let mut content_mismatches = Vec::new();
    let mut authority_errors = Vec::new();
    let mut runtime_asset_files = Vec::new();

    if is_public_release_profile(profile) {
        let verification = crate::sync::manifest::verify_release_manifest(source_root);
        included = verification.required_present;
        required_missing = verification.required_missing;
        extra_files = verification.extra_files;
        content_mismatches = verification.content_mismatches;
        authority_errors = verification.authority_errors;
        match crate::sync::manifest::public_runtime_asset_files(source_root) {
            Ok(files) => runtime_asset_files = files,
            Err(errors) => authority_errors.extend(errors),
        }
        included.sort();

        let included_set: std::collections::BTreeSet<_> = included.iter().cloned().collect();
        for f in &all_files {
            if included_set.contains(f) {
                continue;
            }
            let reason = public_full_forbidden_patterns
                .iter()
                .find(|pat| matches_path_boundary(f, pat))
                .map(|pat| format!("matches forbidden pattern: {pat}"))
                .unwrap_or_else(|| "outside canonical public payload authority".to_string());
            excluded.push(f.clone());
            exclusion_reasons.push((f.clone(), reason));
        }
    } else {
        for f in &all_files {
            included.push(f.clone());
        }
    }

    let forbidden_included: Vec<String> = included
        .iter()
        .filter(|file| {
            public_full_forbidden_patterns
                .iter()
                .any(|pat| matches_path_boundary(file, pat))
        })
        .cloned()
        .collect();

    let plan = serde_json::json!({
        "schema_version": "2.0-release",
        "profile": profile,
        "dry_run": dry_run,
        "source_root": source_root.to_string_lossy(),
        "summary": {
            "total_files": all_files.len(),
            "included": included.len(),
            "excluded": excluded.len(),
        },
        "included_files": included,
        "runtime_asset_files": runtime_asset_files,
        "required_missing": required_missing,
        "extra_files": extra_files,
        "content_mismatches": content_mismatches,
        "authority_errors": authority_errors,
        "forbidden_included": forbidden_included,
        "excluded_files": excluded.iter().map(|f| {
            let empty_reason = String::new();
            let reason = exclusion_reasons
                .iter()
                .find(|(name, _)| name == f)
                .map(|(_, r)| r)
                .unwrap_or(&empty_reason);
            serde_json::json!({"file": f, "reason": reason})
        }).collect::<Vec<_>>(),
    });

    let has_forbidden_included = plan["forbidden_included"]
        .as_array()
        .map(|items| !items.is_empty())
        .unwrap_or(false)
        || plan["required_missing"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || plan["extra_files"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || plan["content_mismatches"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || plan["authority_errors"]
            .as_array()
            .is_some_and(|items| !items.is_empty());

    (plan, has_forbidden_included)
}
#[cfg(test)]
mod tests {
    use super::{
        is_public_release_profile, matches_path_boundary, release_file_list, release_package_plan,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_boundary_requires_exact_match() {
        assert!(matches_path_boundary(
            "scripts/verify.sh",
            "scripts/verify.sh"
        ));
        assert!(!matches_path_boundary(
            "scripts/verify.sh.bak",
            "scripts/verify.sh"
        ));
        assert!(!matches_path_boundary(
            "scripts/verify.sh/extra",
            "scripts/verify.sh"
        ));
    }

    #[test]
    fn directory_boundary_allows_descendants_only_when_marked_as_directory() {
        assert!(matches_path_boundary("crates", "crates/"));
        assert!(matches_path_boundary("crates/runner/src/lib.rs", "crates/"));
        assert!(!matches_path_boundary("crates-private/lib.rs", "crates/"));
        assert!(!matches_path_boundary("crates/runner/src/lib.rs", "crates"));
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    fn unique_temp_repo(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{suffix}", std::process::id()))
    }

    #[test]
    fn public_release_profile_detection_is_explicit() {
        assert!(is_public_release_profile("public-core"));
        assert!(is_public_release_profile("public-full"));
        assert!(!is_public_release_profile("private-full"));
    }

    #[test]
    fn public_release_package_keeps_rust_workspace_and_strips_evomap_runtime() {
        let (plan, failed) = release_package_plan(&workspace_root(), "public-full", true);
        assert!(
            failed,
            "private authority checkout must not masquerade as the closed public payload"
        );

        let included = plan["included_files"]
            .as_array()
            .expect("included_files must be an array");
        let included: Vec<&str> = included.iter().filter_map(|value| value.as_str()).collect();

        assert!(included.contains(&"AGENTS.md"));
        assert!(included.contains(&"Cargo.toml"));
        assert!(included.contains(&"crates/ags-cli/src/main.rs"));
        assert!(included.contains(&"protocol/task-card-template.md"));
        assert!(!included.contains(&"manifests/templates/runtime-profiles.template.yaml"));
        assert!(included.contains(&"manifests/mcp-registry.yaml"));
        assert!(included.contains(&"manifests/skills-registry.yaml"));
        assert!(!included.contains(&"protocol/evolution-memory.md"));
        assert!(!included
            .iter()
            .any(|path| path.starts_with("crates/ags-capability-governance/src/adoption/")));
        assert!(plan["runtime_asset_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "protocol/cursor-skill-index.md"));

        for rel in included {
            let lower = rel.to_ascii_lowercase();
            assert!(
                !lower.contains("evomap")
                    && !lower.contains("evolver")
                    && !lower.contains("/gep/")
                    && !lower.ends_with("/gep")
                    && !lower.starts_with(".evolver/")
                    && !lower.starts_with("assets/gep/"),
                "public package leaked EvoMap/GEP surface: {rel}"
            );
        }
    }

    #[test]
    fn public_release_package_uses_tracked_files_not_untracked_workspace_artifacts() {
        let root = unique_temp_repo("ags-release-package-tracked-files");
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(root.join("README.md"), "# public\n").unwrap();
        fs::write(root.join("notes/untracked.txt"), "local artifact\n").unwrap();

        let status = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("add")
            .arg("Cargo.toml")
            .arg("README.md")
            .status()
            .unwrap();
        assert!(status.success());

        let included = release_file_list(&root);
        assert!(included.iter().any(|path| path == "Cargo.toml"));
        assert!(included.iter().any(|path| path == "README.md"));
        assert!(!included.iter().any(|path| path == "notes/untracked.txt"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_release_package_excludes_all_machine_private_skill_runtime_data() {
        let root = unique_temp_repo("ags-release-package-machine-private");
        fs::create_dir_all(root.join("capability-snapshot")).unwrap();
        fs::create_dir_all(root.join("skill-registry")).unwrap();
        fs::create_dir_all(root.join("skill-usage")).unwrap();
        fs::create_dir_all(root.join("decision-leases")).unwrap();
        fs::create_dir_all(root.join("auth-state")).unwrap();
        fs::create_dir_all(root.join("receipts")).unwrap();
        fs::create_dir_all(root.join("workspace-services")).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        for path in [
            "capability-snapshot/codex.json",
            "skill-registry/user-overlay.yaml",
            "skill-usage/codex.ndjson",
            "decision-leases/lease.json",
            "auth-state/codex.json",
            "receipts/action.json",
            "workspace-services/workspace.capabilities.json",
        ] {
            fs::write(root.join(path), "private runtime data\n").unwrap();
        }

        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("add")
            .arg(".")
            .status()
            .unwrap()
            .success());

        let (plan, failed) = release_package_plan(&root, "public-full", true);
        assert!(failed);
        let included: Vec<&str> = plan["included_files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert_eq!(included, vec!["Cargo.toml"]);
        assert_eq!(plan["excluded_files"].as_array().unwrap().len(), 7);
        assert!(!plan["authority_errors"].as_array().unwrap().is_empty());

        let _ = fs::remove_dir_all(root);
    }
}
