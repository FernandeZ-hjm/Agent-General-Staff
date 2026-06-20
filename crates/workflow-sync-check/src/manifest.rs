//! Sync manifests for different project types.
//!
//! Each manifest defines which files must be checked for a given project kind.
//! Public-full sanitized targets may have a different manifest compared to private/stable.

use crate::types::ProjectKind;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

// ── Manifest definition ────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncManifest {
    /// Relative file paths that must exist and match the source.
    pub required_files: &'static [&'static str],
    /// Protocol directory files to scan for extras beyond the manifest.
    pub protocol_dir: &'static str,
}

/// The full sync manifest for private ↔ stable comparison.
///
/// Covers root-level governance files and all protocol files.
pub const FULL_MANIFEST: SyncManifest = SyncManifest {
    required_files: &[
        "AGENTS.md",
        "CLAUDE.md",
        "WORKSPACE.md",
        "AGENT_SUITE_PROTOCOL.md",
        "protocol/2.0-baseline.md",
        "protocol/2.0-roadmap.md",
        "protocol/README.md",
        "protocol/agent-task-protocol.md",
        "protocol/context-memory.md",
        "protocol/cursor-skill-index.md",
        "protocol/project-profile.md",
        "protocol/runtime-adapters.md",
        "protocol/task-card-template.md",
        "protocol/task-routing.md",
        "protocol/skill-governance.md",
        "governance/skill-sync.md",
        "governance/mcp-adoption-log.yaml",
        "manifests/mcp-registry.yaml",
    ],
    protocol_dir: "protocol",
};

/// Manifest for public-full sanitized targets.
///
/// Public-full includes the Rust AGS runtime and governance framework, while
/// keeping local runtime state outside the public sync surface.
pub const PUBLIC_MANIFEST: SyncManifest = SyncManifest {
    required_files: &[
        "AGENTS.md",
        "CLAUDE.md",
        "WORKSPACE.md",
        "AGENT_SUITE_PROTOCOL.md",
        "README.md",
        "LICENSE",
        "Cargo.toml",
        "Cargo.lock",
        "protocol/agent-task-protocol.md",
        "protocol/mcp-server.md",
        "protocol/runtime-adapters.md",
        "protocol/task-card-template.md",
        "protocol/task-routing.md",
        "protocol/skill-governance.md",
        "protocol/2.0-baseline.md",
        "protocol/2.0-roadmap.md",
        "templates/task-card-template.md",
        "templates/memory/context-capsule.md",
        "templates/memory/task-memory.md",
        "templates/memory/archive-index.md",
        "templates/memory/task-archive/README.md",
        "scripts/install.sh",
        "scripts/validate.sh",
        "scripts/run-task-card.sh",
        "scripts/verify.sh",
        "scripts/context-memory.sh",
        "scripts/stop-archive-hook.sh",
        "manifests/suite.yaml",
        "manifests/skill-recommendations.yaml",
        "governance/skill-sync.md",
        "governance/skill-adoption-log.yaml",
        "governance/skill-ignore-list.yaml",
        "docs/skill-recommendations.md",
    ],
    protocol_dir: "protocol",
};

/// Payload that must never be present in a public-full sanitized release target.
///
/// Public-full ships the AGS Rust workspace and governance framework. It must
/// not ship generated binaries, build caches, preinstalled skill packs, local
/// agent config, or private runtime memory.
pub const PUBLIC_FORBIDDEN_PAYLOAD: &[&str] = &[
    "target/",
    "ags",
    "ags.exe",
    "global-skills/",
    "skill-packs/",
    ".agents/",
    ".ags/",
    ".ags-local/",
    ".codex/",
    ".claude/local/",
    "assets/local-runtime/",
    "manifests/runtime-profiles.yaml",
    "manifests/templates/",
    "memory/",
    "task-archive/",
];

/// Select the appropriate manifest for a project kind.
pub fn manifest_for(kind: &ProjectKind) -> &'static SyncManifest {
    match kind {
        ProjectKind::Stable | ProjectKind::Private | ProjectKind::Custom(_) => &FULL_MANIFEST,
        ProjectKind::PublicCoreOnly => &PUBLIC_MANIFEST,
    }
}

/// Return the union of all files across manifests for extra-file scanning.
pub fn all_manifest_files() -> BTreeSet<&'static str> {
    let mut set = BTreeSet::new();
    for &path in FULL_MANIFEST.required_files {
        set.insert(path);
    }
    for &path in PUBLIC_MANIFEST.required_files {
        set.insert(path);
    }
    set
}

/// Whether a relative path is forbidden in public-full sanitized release payloads.
pub fn is_public_forbidden_payload(relative: &str) -> bool {
    let relative = relative.trim_start_matches("./").replace('\\', "/");
    PUBLIC_FORBIDDEN_PAYLOAD.iter().any(|forbidden| {
        if forbidden.ends_with('/') {
            relative == forbidden.trim_end_matches('/') || relative.starts_with(forbidden)
        } else {
            relative == *forbidden
        }
    })
}

/// Release manifest verification result.
#[derive(Debug, Clone)]
pub struct ManifestVerifyResult {
    pub target: String,
    pub passed: bool,
    pub required_present: Vec<String>,
    pub required_missing: Vec<String>,
    pub forbidden_found: Vec<String>,
    pub extra_files: Vec<String>,
}

/// Verify a target directory against the public release manifest.
///
/// Checks:
/// 1. All PUBLIC_MANIFEST required files are present in the target.
/// 2. No `PUBLIC_FORBIDDEN_PAYLOAD` files are present in the target.
/// 3. Reports any extra files not in either list.
pub fn verify_release_manifest(target: &std::path::Path) -> ManifestVerifyResult {
    let mut required_present: Vec<String> = Vec::new();
    let mut required_missing: Vec<String> = Vec::new();
    let mut forbidden_found: Vec<String> = Vec::new();
    let mut extra_files: Vec<String> = Vec::new();

    // Collect all files in target (recursive, relative paths)
    let target_files = list_files(target);

    // Check required files
    for &required in PUBLIC_MANIFEST.required_files {
        let full = target.join(required);
        if full.exists() {
            required_present.push(required.to_string());
        } else {
            required_missing.push(required.to_string());
        }
    }

    // Check for forbidden payload. In a git worktree, the release payload is the
    // tracked source set, so ignored build output is not scanned as releasable
    // content.
    for relative in &target_files {
        if is_public_forbidden_payload(relative) {
            forbidden_found.push(relative.clone());
        }
    }

    // Find extra files (not in manifest, not forbidden)
    let manifest_files: std::collections::BTreeSet<&str> =
        PUBLIC_MANIFEST.required_files.iter().copied().collect();
    for relative in &target_files {
        if !manifest_files.contains(relative.as_str()) && !is_public_forbidden_payload(relative) {
            extra_files.push(relative.clone());
        }
    }

    let passed = required_missing.is_empty() && forbidden_found.is_empty();

    ManifestVerifyResult {
        target: target.display().to_string(),
        passed,
        required_present,
        required_missing,
        forbidden_found,
        extra_files,
    }
}

/// Recursively list all files in a directory as relative paths.
fn list_files(root: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    let root_canonical = if let Ok(c) = root.canonicalize() {
        c
    } else {
        return files;
    };
    if let Some(tracked) = list_git_tracked_files(&root_canonical) {
        return tracked;
    }
    list_files_recursive(&root_canonical, &root_canonical, &mut files);
    files
}

fn list_git_tracked_files(root: &Path) -> Option<Vec<String>> {
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

    let files = output
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
        .collect();
    Some(files)
}

fn list_files_recursive(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            list_files_recursive(root, &path, files);
        } else {
            files.push(relative);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_manifest_has_18_files() {
        assert_eq!(FULL_MANIFEST.required_files.len(), 18);
    }

    #[test]
    fn public_manifest_includes_public_core_protocol_and_scripts() {
        let public: BTreeSet<_> = PUBLIC_MANIFEST.required_files.iter().copied().collect();
        for path in [
            "AGENTS.md",
            "CLAUDE.md",
            "WORKSPACE.md",
            "AGENT_SUITE_PROTOCOL.md",
            "templates/memory/context-capsule.md",
            "templates/memory/task-memory.md",
            "scripts/context-memory.sh",
            "scripts/stop-archive-hook.sh",
            "governance/skill-adoption-log.yaml",
            "governance/skill-ignore-list.yaml",
        ] {
            assert!(public.contains(path), "public manifest missing {path}");
        }
    }

    #[test]
    fn public_manifest_requires_root_entry_files() {
        let public: BTreeSet<_> = PUBLIC_MANIFEST.required_files.iter().copied().collect();
        assert!(public.contains("AGENTS.md"));
        assert!(public.contains("CLAUDE.md"));
        assert!(public.contains("WORKSPACE.md"));
        assert!(public.contains("AGENT_SUITE_PROTOCOL.md"));
    }

    #[test]
    fn stable_uses_full_manifest() {
        let m = manifest_for(&ProjectKind::Stable);
        assert_eq!(m.required_files.len(), 18);
    }

    #[test]
    fn public_full_sanitized_uses_expanded_manifest() {
        let m = manifest_for(&ProjectKind::PublicCoreOnly);
        assert!(m.required_files.len() > 20);
    }

    #[test]
    fn public_forbidden_payload_covers_build_artifacts_and_runtime_state() {
        for path in [
            "target/release/ags",
            "ags",
            "global-skills/custom/SKILL.md",
            "skill-packs/personal/example/SKILL.md",
            ".agents/memory/projects/demo/context-capsule.md",
            ".ags-local/private-public-update.sh",
            ".codex/skills/example/SKILL.md",
        ] {
            assert!(
                is_public_forbidden_payload(path),
                "expected public forbidden payload: {path}"
            );
        }
    }

    #[test]
    fn public_forbidden_payload_uses_exact_file_and_directory_boundaries() {
        assert!(is_public_forbidden_payload("target/release/ags"));
        assert!(is_public_forbidden_payload("target"));
        assert!(!is_public_forbidden_payload("targets/custom.txt"));
        assert!(is_public_forbidden_payload(
            "global-skills/example/SKILL.md"
        ));
        assert!(is_public_forbidden_payload(".ags/runtime-state.json"));
        assert!(is_public_forbidden_payload(
            ".ags-local/private-public-update.sh"
        ));
        assert!(!is_public_forbidden_payload(".ags-locality/file.txt"));
        assert!(is_public_forbidden_payload(
            "assets/local-runtime/capsules.json"
        ));
        assert!(is_public_forbidden_payload(
            "manifests/templates/runtime-profiles.template.yaml"
        ));
        assert!(!is_public_forbidden_payload("global-skills.md"));
        assert!(!is_public_forbidden_payload("governance/skill-sync.md"));
    }

    #[test]
    fn public_forbidden_payload_allows_public_protocol_and_scripts() {
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "crates/ags-cli/src/main.rs",
            "protocol/task-card-template.md",
            "templates/task-card-template.md",
            "project-integration/AGENTS.md.template",
            "scripts/verify.sh",
            "scripts/validate-task-card.sh",
            "README.md",
            // Public targets may have their own governance/manifests and empty
            // audit skeletons. Non-empty private audit content is checked by
            // release sanitize, not path-level manifest filtering.
            "governance/skill-adoption-log.yaml",
            "governance/skill-ignore-list.yaml",
            "governance/sync-protocol.md",
            "governance/inventory-schema.md",
            "governance/rollback.md",
            "manifests/capabilities.yaml",
            "manifests/skills-registry.yaml",
            "manifests/suite.core.yaml",
        ] {
            assert!(
                !is_public_forbidden_payload(path),
                "expected public-allowed payload: {path}"
            );
        }
    }

    #[test]
    fn custom_target_uses_full_manifest() {
        let m = manifest_for(&ProjectKind::Custom("test".into()));
        assert_eq!(m.required_files.len(), 18);
    }

    // ── verify_release_manifest tests ─────────────────────────────────

    #[test]
    fn verify_release_manifest_empty_dir_fails() {
        let dir = tempfile::tempdir().unwrap();
        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(!result.required_missing.is_empty());
        assert!(result.forbidden_found.is_empty());
    }

    #[test]
    fn verify_release_manifest_detects_forbidden_payload() {
        let dir = tempfile::tempdir().unwrap();
        // Create forbidden build artifact (target/)
        std::fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        std::fs::write(
            dir.path().join("target").join("release").join("ags"),
            "binary\n",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(!result.forbidden_found.is_empty());
        assert!(
            result.forbidden_found.iter().any(|f| f.contains("target")),
            "should detect target/ as forbidden"
        );
    }

    #[test]
    fn verify_release_manifest_ignores_gitignored_build_output() {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("init")
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(dir.path().join(".gitignore"), "/target/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target").join("release")).unwrap();
        std::fs::write(
            dir.path().join("target").join("release").join("ags"),
            "binary\n",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(
            result.forbidden_found.is_empty(),
            "gitignored build output should not count as release payload: {:?}",
            result.forbidden_found
        );
    }

    #[test]
    fn verify_release_manifest_uses_tracked_payload_in_git_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("init")
            .status()
            .unwrap();
        assert!(status.success());

        std::fs::write(dir.path().join(".gitignore"), "/target/\n").unwrap();
        for idx in 0..200 {
            let path = dir
                .path()
                .join("target")
                .join("debug")
                .join(format!("artifact-{idx}.o"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "build artifact\n").unwrap();
        }

        let result = verify_release_manifest(dir.path());
        assert!(
            result.forbidden_found.is_empty(),
            "ignored target files should not be part of tracked release payload: {:?}",
            result.forbidden_found
        );

        let tracked_forbidden = dir.path().join("target").join("release").join("ags");
        std::fs::create_dir_all(tracked_forbidden.parent().unwrap()).unwrap();
        std::fs::write(&tracked_forbidden, "tracked binary\n").unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("add")
            .arg("-f")
            .arg("target/release/ags")
            .status()
            .unwrap();
        assert!(status.success());

        let result = verify_release_manifest(dir.path());
        assert_eq!(
            result.forbidden_found,
            vec!["target/release/ags".to_string()],
            "tracked forbidden payload must still fail release manifest"
        );
    }

    #[test]
    fn verify_release_manifest_allows_empty_governance_audit_skeletons() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("governance")).unwrap();
        std::fs::write(
            dir.path()
                .join("governance")
                .join("skill-adoption-log.yaml"),
            "schema_version: \"1.0\"\nentries: []\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("governance").join("skill-ignore-list.yaml"),
            "schema_version: \"1.0\"\nentries: []\n",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(
            !result
                .forbidden_found
                .iter()
                .any(|f| f.contains("skill-adoption-log.yaml")),
            "empty public audit skeleton should be allowed"
        );
    }

    #[test]
    fn verify_release_manifest_accepts_clean_target() {
        let dir = tempfile::tempdir().unwrap();
        // Create all required public manifest files
        for &file in PUBLIC_MANIFEST.required_files {
            let path = dir.path().join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, "placeholder").unwrap();
        }

        let result = verify_release_manifest(dir.path());
        assert!(
            result.passed,
            "clean target should pass. missing={:?}, forbidden={:?}",
            result.required_missing, result.forbidden_found
        );
        assert!(result.required_missing.is_empty());
        assert!(result.forbidden_found.is_empty());
    }
}
