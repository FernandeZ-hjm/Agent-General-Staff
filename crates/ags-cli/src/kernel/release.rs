use crate::cli::ReleaseAction;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Shared dispatch: `release verify`
fn cmd_release_verify(target: &str, format: &str) {
    let target_root = match target {
        "stable" => PathBuf::from("/Volumes/Projects/example-stable-suite"),
        "public" | "public-core" | "public-full" | "public-full-sanitized" => {
            PathBuf::from("/Volumes/AI Project/ai-dev-env-bootstrap")
        }
        _ => unreachable!("clap guards target values"),
    };

    let target_config = workflow_sync_check::TargetConfig {
        root: target_root.clone(),
        name: target.to_string(),
        kind: match target {
            "stable" => workflow_sync_check::ProjectKind::Stable,
            "public" | "public-core" | "public-full" | "public-full-sanitized" => {
                workflow_sync_check::ProjectKind::PublicCoreOnly
            }
            _ => unreachable!(),
        },
    };

    let options = workflow_sync_check::CheckOptions {
        source_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        source_name: "private".to_string(),
        targets: vec![target_config],
        allowlist_path: None,
    };

    let report_format = match format {
        "json" => workflow_sync_check::ReportFormat::Json,
        _ => workflow_sync_check::ReportFormat::Text,
    };

    let ok = workflow_sync_check::run_cli(options, report_format);
    if !ok {
        std::process::exit(1);
    }
}
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
    workflow_sync_check::manifest::PUBLIC_FORBIDDEN_PAYLOAD
        .iter()
        .copied()
        .chain([
            "proposals/",
            "graphify-out/",
            "governance/skill-adoption-log.yaml",
            "governance/skill-ignore-list.yaml",
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
                .replace('\\', "/");
            if entry.path().is_dir() {
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
                if root.join(&relative).is_file() {
                    Some(relative)
                } else {
                    None
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
fn release_package_plan(
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

    if is_public_release_profile(profile) {
        for f in &all_files {
            let forbidden_reason = public_full_forbidden_patterns
                .iter()
                .find(|pat| matches_path_boundary(f, pat))
                .map(|pat| format!("matches forbidden pattern: {}", pat));

            if let Some(reason) = forbidden_reason {
                excluded.push(f.clone());
                exclusion_reasons.push((f.clone(), reason));
                continue;
            }

            included.push(f.clone());
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
        .unwrap_or(false);

    (plan, has_forbidden_included)
}
fn render_release_package_plan_text(plan: &serde_json::Value) {
    println!("Release Package Plan");
    println!("====================");
    println!("Schema:    {}", plan["schema_version"]);
    println!("Profile:   {}", plan["profile"]);
    println!("Dry run:   {}", plan["dry_run"]);
    println!("Source:    {}", plan["source_root"]);
    println!();
    println!(
        "Files:     {} total, {} included, {} excluded",
        plan["summary"]["total_files"], plan["summary"]["included"], plan["summary"]["excluded"]
    );
    println!();
    println!("Included:");
    if let Some(files) = plan["included_files"].as_array() {
        for file in files.iter().filter_map(|value| value.as_str()) {
            println!("  + {}", file);
        }
    }
    if let Some(files) = plan["forbidden_included"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Forbidden included:");
            for file in files.iter().filter_map(|value| value.as_str()) {
                println!("  ! {}", file);
            }
        }
    }
    if let Some(files) = plan["excluded_files"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Excluded:");
            for entry in files {
                let file = entry["file"].as_str().unwrap_or("");
                let reason = entry["reason"].as_str().unwrap_or("");
                println!("  - {}  ({})", file, reason);
            }
        }
    }
    println!();
    println!("Verdict: DRY-RUN — no files written. Ready for review.");
}
/// Shared dispatch: `release package`
fn cmd_release_package(profile: &str, dry_run: bool, format: &str) {
    if !dry_run {
        eprintln!("release package: --dry-run is required for now. Apply not yet implemented.");
        std::process::exit(2);
    }

    let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (plan, has_forbidden_included) = release_package_plan(&source_root, profile, dry_run);

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        _ => render_release_package_plan_text(&plan),
    }

    if has_forbidden_included {
        std::process::exit(1);
    }
}
#[cfg(test)]
mod release_package_tests {
    use super::{is_public_release_profile, matches_path_boundary, release_package_plan};
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
        assert!(matches_path_boundary(
            ".ags-local/private-public-update.sh",
            ".ags-local/"
        ));
        assert!(!matches_path_boundary(
            ".ags-locality/private-public-update.sh",
            ".ags-local/"
        ));
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
    fn public_release_package_keeps_rust_workspace() {
        let (plan, failed) = release_package_plan(&workspace_root(), "public-full", true);
        assert!(
            !failed,
            "public-full package plan must not include forbidden files"
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

        let (plan, failed) = release_package_plan(&root, "public-full", true);
        assert!(!failed);
        let included = plan["included_files"]
            .as_array()
            .expect("included_files must be an array");
        let included: Vec<&str> = included.iter().filter_map(|value| value.as_str()).collect();
        assert!(included.contains(&"Cargo.toml"));
        assert!(included.contains(&"README.md"));
        assert!(!included.contains(&"notes/untracked.txt"));

        let _ = fs::remove_dir_all(root);
    }
}

pub(crate) fn run(action: ReleaseAction) {
    match action {
        ReleaseAction::Verify { target, format } => cmd_release_verify(&target, &format),
        ReleaseAction::Package {
            profile,
            dry_run,
            format,
        } => cmd_release_package(&profile, dry_run, &format),
    }
}
