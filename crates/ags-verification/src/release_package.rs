//! Canonical release-package discovery and payload planning.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const RELEASE_PLAN_SCHEMA_VERSION: &str = "0.4.0-release-plan";

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStageResult {
    pub schema_version: &'static str,
    pub source_root: String,
    pub target_root: String,
    pub staged_files: Vec<String>,
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
    crate::release_manifest::PUBLIC_FORBIDDEN_PAYLOAD
        .iter()
        .copied()
        .chain(["proposals/", "graphify-out/", ".claude/", ".codegraph/"])
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
        let verification = crate::release_manifest::verify_release_manifest(source_root);
        included = verification.required_present;
        required_missing = verification.required_missing;
        extra_files = verification.extra_files;
        content_mismatches = verification.content_mismatches;
        authority_errors = verification.authority_errors;
        match crate::release_manifest::public_runtime_asset_files(source_root) {
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

    let forbidden_included: Vec<String> = if is_public_release_profile(profile) {
        included
            .iter()
            .filter(|file| {
                public_full_forbidden_patterns
                    .iter()
                    .any(|pat| matches_path_boundary(file, pat))
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let plan = serde_json::json!({
        "schema_version": RELEASE_PLAN_SCHEMA_VERSION,
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

pub fn stage_release_runtime(
    plan_path: &Path,
    source_root: &Path,
    target_root: &Path,
) -> Result<RuntimeStageResult, String> {
    reject_root_symlink(plan_path, "release plan")?;
    let plan_bytes = std::fs::read(plan_path)
        .map_err(|error| format!("cannot read release plan {}: {error}", plan_path.display()))?;
    let plan: serde_json::Value = serde_json::from_slice(&plan_bytes)
        .map_err(|error| format!("invalid release plan JSON: {error}"))?;
    if plan["schema_version"].as_str() != Some(RELEASE_PLAN_SCHEMA_VERSION) {
        return Err(format!(
            "release plan schema_version must be {RELEASE_PLAN_SCHEMA_VERSION}"
        ));
    }
    if plan["profile"].as_str() != Some("public-full") {
        return Err("release plan profile must be public-full".to_string());
    }
    for (field, message) in [
        ("authority_errors", "release payload authority errors"),
        ("required_missing", "release payload missing required files"),
        (
            "extra_files",
            "release payload contains non-authority files",
        ),
        (
            "content_mismatches",
            "release payload contains unapproved content drift",
        ),
        (
            "forbidden_included",
            "release payload contains forbidden files",
        ),
    ] {
        let values = string_array(&plan, field)?;
        if !values.is_empty() {
            return Err(format!("{message}: {}", values.join(", ")));
        }
    }
    let runtime_assets = string_array(&plan, "runtime_asset_files")?;
    let included_files = string_array(&plan, "included_files")?;
    ensure_unique(&runtime_assets, "runtime_asset_files")?;
    ensure_unique(&included_files, "included_files")?;
    let included: BTreeSet<_> = included_files.iter().cloned().collect();
    let outside: Vec<_> = runtime_assets
        .iter()
        .filter(|path| !included.contains(*path))
        .cloned()
        .collect();
    if !outside.is_empty() {
        return Err(format!(
            "runtime assets are outside the canonical included payload: {}",
            outside.join(", ")
        ));
    }

    let source_input = absolute(source_root)?;
    reject_root_symlink(&source_input, "runtime source root")?;
    let source = source_input
        .canonicalize()
        .map_err(|error| format!("cannot resolve runtime source root: {error}"))?;
    let target_input = absolute(target_root)?;
    reject_root_symlink(&target_input, "runtime target root")?;
    std::fs::create_dir_all(&target_input)
        .map_err(|error| format!("cannot create runtime target root: {error}"))?;
    let target = target_input
        .canonicalize()
        .map_err(|error| format!("cannot resolve runtime target root: {error}"))?;
    let entries = std::fs::read_dir(&target)
        .map_err(|error| format!("cannot inspect runtime target root: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot inspect runtime target entry: {error}"))?;
    if entries.iter().any(|entry| {
        std::fs::symlink_metadata(entry.path())
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    }) {
        return Err("runtime target path must not contain a symlink".to_string());
    }
    if !entries.is_empty() {
        return Err("runtime target root must be empty before staging".to_string());
    }

    let mut validated = Vec::new();
    for relative in &runtime_assets {
        let relative_path = safe_relative_path(relative)?;
        reject_symlink_components(&source, &relative_path, "runtime source")?;
        let source_file = source.join(&relative_path);
        let metadata = std::fs::symlink_metadata(&source_file)
            .map_err(|_| format!("runtime asset missing: {relative}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!("runtime asset is not a regular file: {relative}"));
        }
        let resolved_source = source_file
            .canonicalize()
            .map_err(|error| format!("cannot resolve runtime asset {relative}: {error}"))?;
        if !resolved_source.starts_with(&source) {
            return Err(format!("runtime asset escapes source root: {relative}"));
        }
        validated.push((relative.clone(), relative_path, resolved_source));
    }

    for (relative, relative_path, resolved_source) in &validated {
        let parent = ensure_target_parent(&target, relative_path)?;
        let target_file = target.join(relative_path);
        if std::fs::symlink_metadata(&target_file)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(format!(
                "runtime target path must not contain a symlink: {relative}"
            ));
        }
        if target_file.exists() && !target_file.is_file() {
            return Err(format!("runtime target is not a regular file: {relative}"));
        }
        let resolved_parent = parent
            .canonicalize()
            .map_err(|error| format!("cannot resolve runtime target parent: {error}"))?;
        if !resolved_parent.starts_with(&target) {
            return Err(format!("runtime target escapes target root: {relative}"));
        }
        std::fs::copy(resolved_source, &target_file)
            .map_err(|error| format!("cannot stage runtime asset {relative}: {error}"))?;
    }

    Ok(RuntimeStageResult {
        schema_version: "0.4.0-runtime-stage",
        source_root: source.display().to_string(),
        target_root: target.display().to_string(),
        staged_files: runtime_assets,
    })
}

fn string_array(plan: &serde_json::Value, field: &str) -> Result<Vec<String>, String> {
    let values = plan
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("release plan {field} must be a string array"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("release plan {field} must be a string array"))
        })
        .collect()
}

fn ensure_unique(values: &[String], field: &str) -> Result<(), String> {
    let unique: BTreeSet<_> = values.iter().collect();
    if unique.len() == values.len() {
        Ok(())
    } else {
        Err(format!("release plan {field} must not contain duplicates"))
    }
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("cannot resolve current directory: {error}"))
    }
}

fn reject_root_symlink(path: &Path, label: &str) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        Err(format!("{label} must not be a symlink: {}", path.display()))
    } else {
        Ok(())
    }
}

fn safe_relative_path(relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || path == Path::new(".")
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe runtime asset path: {relative}"));
    }
    Ok(path.to_path_buf())
}

fn reject_symlink_components(root: &Path, relative: &Path, label: &str) -> Result<(), String> {
    reject_root_symlink(root, &format!("{label} root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(format!(
                "{label} path must not contain a symlink: {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn ensure_target_parent(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            current.push(component.as_os_str());
            if std::fs::symlink_metadata(&current)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(format!(
                    "runtime target path must not contain a symlink: {}",
                    relative.display()
                ));
            }
            std::fs::create_dir(&current)
                .or_else(|error| if current.is_dir() { Ok(()) } else { Err(error) })
                .map_err(|error| {
                    format!(
                        "runtime target parent is not a directory for {}: {error}",
                        relative.display()
                    )
                })?;
        }
    }
    Ok(current)
}
#[cfg(test)]
mod tests {
    use super::{
        is_public_release_profile, matches_path_boundary, release_file_list, release_package_plan,
        stage_release_runtime, RELEASE_PLAN_SCHEMA_VERSION,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_boundary_requires_exact_match() {
        assert!(matches_path_boundary(
            "scripts/ags-memory-lifecycle-omp.js",
            "scripts/ags-memory-lifecycle-omp.js"
        ));
        assert!(!matches_path_boundary(
            "scripts/ags-memory-lifecycle-omp.js.tmp",
            "scripts/ags-memory-lifecycle-omp.js"
        ));
        assert!(!matches_path_boundary(
            "scripts/ags-memory-lifecycle-omp.js/extra",
            "scripts/ags-memory-lifecycle-omp.js"
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

    fn write_stage_plan(root: &Path, assets: &[&str]) -> PathBuf {
        let plan = root.join("plan.json");
        fs::write(
            &plan,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": RELEASE_PLAN_SCHEMA_VERSION,
                "profile": "public-full",
                "authority_errors": [],
                "required_missing": [],
                "extra_files": [],
                "content_mismatches": [],
                "forbidden_included": [],
                "runtime_asset_files": assets,
                "included_files": assets,
            }))
            .unwrap(),
        )
        .unwrap();
        plan
    }

    #[test]
    fn runtime_stage_copies_only_authorized_assets() {
        let root = unique_temp_repo("ags-runtime-stage-success");
        let source = root.join("source");
        let target = root.join("target");
        for relative in ["manifests/a.yaml", "protocol/b.md"] {
            let path = source.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, relative).unwrap();
        }
        let plan = write_stage_plan(&root, &["manifests/a.yaml", "protocol/b.md"]);

        let result = stage_release_runtime(&plan, &source, &target).unwrap();
        assert_eq!(result.staged_files.len(), 2);
        assert_eq!(
            fs::read_to_string(target.join("manifests/a.yaml")).unwrap(),
            "manifests/a.yaml"
        );
        assert_eq!(
            fs::read_to_string(target.join("protocol/b.md")).unwrap(),
            "protocol/b.md"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_stage_rejects_non_authority_plans_and_nonempty_targets() {
        for field in [
            "authority_errors",
            "required_missing",
            "extra_files",
            "content_mismatches",
            "forbidden_included",
        ] {
            let root = unique_temp_repo("ags-runtime-stage-authority");
            fs::create_dir_all(root.join("source")).unwrap();
            let plan_path = write_stage_plan(&root, &[]);
            let mut plan: serde_json::Value =
                serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
            plan[field] = serde_json::json!(["rejected.txt"]);
            fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
            assert!(
                stage_release_runtime(&plan_path, &root.join("source"), &root.join("target"))
                    .is_err(),
                "{field}"
            );
            let _ = fs::remove_dir_all(root);
        }

        let root = unique_temp_repo("ags-runtime-stage-nonempty");
        fs::create_dir_all(root.join("source")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/stale"), "stale").unwrap();
        let plan = write_stage_plan(&root, &[]);
        let error =
            stage_release_runtime(&plan, &root.join("source"), &root.join("target")).unwrap_err();
        assert!(error.contains("must be empty"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_stage_rejects_path_traversal_and_assets_outside_payload() {
        let root = unique_temp_repo("ags-runtime-stage-paths");
        fs::create_dir_all(root.join("source")).unwrap();
        let plan = write_stage_plan(&root, &["../outside"]);
        let error =
            stage_release_runtime(&plan, &root.join("source"), &root.join("target")).unwrap_err();
        assert!(error.contains("unsafe runtime asset path"), "{error}");
        let _ = fs::remove_dir_all(&root);

        let root = unique_temp_repo("ags-runtime-stage-outside");
        fs::create_dir_all(root.join("source")).unwrap();
        let plan_path = write_stage_plan(&root, &["private.txt"]);
        let mut plan: serde_json::Value =
            serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["included_files"] = serde_json::json!([]);
        fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
        let error = stage_release_runtime(&plan_path, &root.join("source"), &root.join("target"))
            .unwrap_err();
        assert!(error.contains("outside the canonical included payload"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_stage_rejects_symlinked_plan_source_and_target_paths() {
        let root = unique_temp_repo("ags-runtime-stage-symlinks");
        let source = root.join("source");
        let outside = root.join("outside");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("a.yaml"), "outside").unwrap();
        std::os::unix::fs::symlink(&outside, source.join("manifests")).unwrap();
        let plan = write_stage_plan(&root, &["manifests/a.yaml"]);
        let error =
            stage_release_runtime(&plan, &source, &root.join("target-source-link")).unwrap_err();
        assert!(error.contains("source path must not contain a symlink"));

        let linked_plan = root.join("linked-plan.json");
        std::os::unix::fs::symlink(&plan, &linked_plan).unwrap();
        let error = stage_release_runtime(&linked_plan, &source, &root.join("target-plan-link"))
            .unwrap_err();
        assert!(error.contains("plan must not be a symlink"));

        let target = root.join("target");
        std::os::unix::fs::symlink(&outside, &target).unwrap();
        let error = stage_release_runtime(&plan, &source, &target).unwrap_err();
        assert!(error.contains("target root must not be a symlink"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_release_profile_detection_is_explicit() {
        assert!(is_public_release_profile("public-core"));
        assert!(is_public_release_profile("public-full"));
        assert!(!is_public_release_profile("private-full"));
    }

    #[test]
    fn private_full_candidate_is_not_rejected_by_public_only_forbidden_patterns() {
        let root = workspace_root();
        let (plan, failed) = release_package_plan(&root, "private-full", true);
        assert!(
            !failed,
            "private-full intentionally carries private authority assets: {plan}"
        );
        for field in [
            "forbidden_included",
            "required_missing",
            "extra_files",
            "content_mismatches",
            "authority_errors",
        ] {
            assert_eq!(plan[field], serde_json::json!([]), "{field}");
        }
        assert!(plan["included_files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()));
    }

    #[test]
    fn public_release_package_keeps_rust_workspace_and_strips_evomap_runtime() {
        let root = workspace_root();
        let (plan, failed) = release_package_plan(&root, "public-full", true);
        if crate::edition::is_public_edition(&root) {
            assert!(
                !failed,
                "canonical public checkout must close its own release payload: {}",
                plan
            );
        } else {
            assert!(
                failed,
                "private authority checkout must not masquerade as the closed public payload"
            );
        }

        let included = plan["included_files"]
            .as_array()
            .expect("included_files must be an array");
        let included: Vec<&str> = included.iter().filter_map(|value| value.as_str()).collect();

        assert!(included.contains(&"AGENTS.md"));
        assert!(included.contains(&"Cargo.toml"));
        assert!(included.contains(&"crates/ags-cli/src/main.rs"));
        assert!(included.contains(&"protocol/task-card-template.md"));
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
