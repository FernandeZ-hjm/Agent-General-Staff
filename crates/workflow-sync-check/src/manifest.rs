//! Sync manifests for different project types.
//!
//! Each manifest defines which files must be checked for a given project kind.
//! Public-core-only targets may have a reduced manifest compared to private/stable.

use crate::types::ProjectKind;
use std::collections::BTreeSet;

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
    ],
    protocol_dir: "protocol",
};

/// Reduced manifest for public/core-only targets.
///
/// Excludes files that contain internal collaboration details, machine-specific
/// paths, or private workflow instructions not suitable for public distribution.
pub const PUBLIC_MANIFEST: SyncManifest = SyncManifest {
    required_files: &[
        // Root-level: only AGENTS.md and CLAUDE.md may be published
        // (AGENT_SUITE_PROTOCOL.md and WORKSPACE.md are internal)
        "protocol/agent-task-protocol.md",
        "protocol/runtime-adapters.md",
        "protocol/task-card-template.md",
        "protocol/task-routing.md",
    ],
    protocol_dir: "protocol",
};

/// Private-only payload that must never be present in a public/core-only
/// release target.
///
/// Stable -> public promotion publishes the user-facing kit, not the private
/// Rust governance toolchain that produces and checks that kit.
pub const PUBLIC_FORBIDDEN_PAYLOAD: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "crates/",
    "target/",
    "ags",
    "ags.exe",
    // Private governance audit logs and suite manifest must never appear in
    // public/core-only releases. Public targets may have their own governance/
    // and manifests/ directories with public-safe content.
    "governance/skill-adoption-log.yaml",
    "governance/skill-ignore-list.yaml",
    "manifests/suite.yaml",
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

/// Whether a relative path is forbidden in public/core-only release payloads.
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

    // Check for forbidden payload
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
    list_files_recursive(&root_canonical, &root_canonical, &mut files);
    files
}

fn list_files_recursive(root: &std::path::Path, dir: &std::path::Path, files: &mut Vec<String>) {
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
    fn full_manifest_has_16_files() {
        assert_eq!(FULL_MANIFEST.required_files.len(), 16);
    }

    #[test]
    fn public_manifest_is_subset_of_full() {
        let full: BTreeSet<_> = FULL_MANIFEST.required_files.iter().copied().collect();
        for path in PUBLIC_MANIFEST.required_files {
            assert!(
                full.contains(path),
                "public manifest file {path} not in full manifest"
            );
        }
    }

    #[test]
    fn public_manifest_excludes_internal_files() {
        let public: BTreeSet<_> = PUBLIC_MANIFEST.required_files.iter().copied().collect();
        assert!(!public.contains("AGENTS.md"));
        assert!(!public.contains("CLAUDE.md"));
        assert!(!public.contains("WORKSPACE.md"));
        assert!(!public.contains("AGENT_SUITE_PROTOCOL.md"));
    }

    #[test]
    fn stable_uses_full_manifest() {
        let m = manifest_for(&ProjectKind::Stable);
        assert_eq!(m.required_files.len(), 16);
    }

    #[test]
    fn public_core_only_uses_reduced_manifest() {
        let m = manifest_for(&ProjectKind::PublicCoreOnly);
        assert_eq!(m.required_files.len(), 4);
    }

    #[test]
    fn public_forbidden_payload_covers_rust_toolchain_artifacts() {
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "crates/ags-cli/src/main.rs",
            "crates/workflow-sync-check/src/lib.rs",
            "target/release/ags",
            "ags",
            // Private governance audit files must be forbidden
            "governance/skill-adoption-log.yaml",
            "governance/skill-ignore-list.yaml",
            "manifests/suite.yaml",
        ] {
            assert!(
                is_public_forbidden_payload(path),
                "expected public forbidden payload: {path}"
            );
        }
    }

    #[test]
    fn public_forbidden_payload_uses_exact_file_and_directory_boundaries() {
        assert!(is_public_forbidden_payload("crates/runner/src/lib.rs"));
        assert!(is_public_forbidden_payload("crates"));
        assert!(!is_public_forbidden_payload(
            "crates-private/runner/src/lib.rs"
        ));
        assert!(is_public_forbidden_payload("Cargo.toml"));
        assert!(!is_public_forbidden_payload("Cargo.toml.bak"));
    }

    #[test]
    fn public_forbidden_payload_allows_public_protocol_and_scripts() {
        for path in [
            "protocol/task-card-template.md",
            "templates/fallback-task-cards/light.md",
            "project-integration/AGENTS.md.template",
            "scripts/verify.sh",
            "scripts/validate-task-card.sh",
            "README.md",
            // Public targets may have their own governance/ and manifests/
            // directories with public-safe content. Only specific private
            // files are forbidden.
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
        assert_eq!(m.required_files.len(), 16);
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
        // Create a forbidden file
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("crates").join("test-crate")).unwrap();
        std::fs::write(
            dir.path()
                .join("crates")
                .join("test-crate")
                .join("Cargo.toml"),
            "[package]\n",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(!result.forbidden_found.is_empty());
        assert!(
            result
                .forbidden_found
                .iter()
                .any(|f| f.contains("Cargo.toml")),
            "should detect Cargo.toml as forbidden"
        );
    }

    #[test]
    fn verify_release_manifest_detects_governance_and_manifest_payload() {
        let dir = tempfile::tempdir().unwrap();
        // Create private governance files
        std::fs::create_dir_all(dir.path().join("governance")).unwrap();
        std::fs::write(
            dir.path()
                .join("governance")
                .join("skill-adoption-log.yaml"),
            "schema_version: \"1.0\"\nentries: []\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("manifests")).unwrap();
        std::fs::write(
            dir.path().join("manifests").join("suite.yaml"),
            "schema_version: \"1.0\"\nsuite:\n  name: test\n",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(!result.forbidden_found.is_empty());
        assert!(
            result
                .forbidden_found
                .iter()
                .any(|f| f.contains("skill-adoption-log.yaml")),
            "should detect governance/skill-adoption-log.yaml as forbidden"
        );
        assert!(
            result
                .forbidden_found
                .iter()
                .any(|f| f.contains("suite.yaml")),
            "should detect manifests/suite.yaml as forbidden"
        );

        // Also create a public-safe governance file — must NOT be flagged
        std::fs::write(
            dir.path().join("governance").join("sync-protocol.md"),
            "# Public sync protocol\n",
        )
        .unwrap();

        let result2 = verify_release_manifest(dir.path());
        assert!(!result2.passed);
        assert!(
            !result2
                .forbidden_found
                .iter()
                .any(|f| f.contains("sync-protocol.md")),
            "public-safe governance file should not be forbidden"
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
