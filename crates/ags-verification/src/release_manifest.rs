//! Exact public release payload authority and verification.
//!
//! This module is the single release seam shared by edition detection,
//! packaging, self-verification, and private-to-public promotion.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

// ── Manifest definition ────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseManifest {
    /// Relative file paths that must exist and match the source.
    pub required_files: &'static [&'static str],
    /// Protocol directory files to scan for extras beyond the manifest.
    pub protocol_dir: &'static str,
}

/// Manifest for public-full sanitized targets.
///
/// Public-full includes the Rust AGS runtime and governance framework, while
/// keeping private EvoMap/GEP runtime surfaces outside the public sync surface.
pub const PUBLIC_MANIFEST: ReleaseManifest = ReleaseManifest {
    required_files: &[
        "AGENTS.md",
        "CLAUDE.md",
        "WORKSPACE.md",
        "AGENT_SUITE_PROTOCOL.md",
        "README.md",
        "RELEASE_NOTES.md",
        "SECURITY.md",
        "LICENSE",
        "NOTICE.md",
        "THIRD_PARTY_NOTICES.md",
        "COMMERCIAL.md",
        "Cargo.toml",
        "Cargo.lock",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/workflows/npm-publish.yml",
        "packages/ags-mcp/package.json",
        "packages/ags-mcp/LICENSE",
        "packages/ags-mcp/bin/ags-mcp.js",
        "packages/ags-mcp/src/launcher.js",
        "packages/ags-mcp/test/launcher.test.js",
        "packages/ags-mcp/README.md",
        "protocol/agent-task-protocol.md",
        "protocol/entrypoint-guidelines.md",
        "protocol/mcp-server.md",
        "protocol/runtime-adapters.md",
        "protocol/task-card-template.md",
        "protocol/task-routing.md",
        "protocol/skill-governance.md",
        "protocol/context-memory.md",
        "protocol/cursor-skill-index.md",
        "protocol/project-profile.md",
        "templates/task-card-template.md",
        "templates/memory/context-capsule.md",
        "templates/memory/task-memory.md",
        "templates/memory/archive-index.md",
        "templates/memory/task-archive/README.md",
        "templates/global-entry/ags-core.md",
        "templates/global-entry/ags-task-handoff.md",
        "templates/global-entry/host-operations.md",
        "scripts/ags-memory-lifecycle-omp.js",
        "manifests/suite.yaml",
        "manifests/onboarding-public.yaml",
        "manifests/public-release-payload.yaml",
        "manifests/templates/README.md",
        "manifests/templates/hooks/claude-code-executor-stop.template.js",
        "manifests/templates/hooks/codex-planner-recall.template.json",
        "manifests/templates/runtime-profiles.template.yaml",
        "manifests/third-party-capabilities.yaml",
        "governance/skill-sync.md",
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
    ".codex/",
    ".claude/local/",
    ".evolver/",
    "assets/gep/",
    "crates/ags-mcp/assets/gep/",
    "crates/ags-capability-governance/assets/gep/",
    "crates/ags-capability-governance/src/adoption/",
    "crates/ags-cli/assets/gep/",
    "evomap/",
    "mcp/gep.mcp.json",
    "hosts/claude-code.evomap-mcp.snippet.json",
    "bin/evolver-proxy-mcp",
    "manifests/runtime-profiles.yaml",
    "crates/ags-mcp/src/resources/evolver_boundary.md",
    "protocol/evolution-memory.md",
    "memory/",
    "task-archive/",
    "capability-snapshot/",
    "skill-registry/",
    "skill-usage/",
    "decision-leases/",
    "auth-state/",
    "receipts/",
    "workspace-services/",
    ".ags/",
];

pub const PUBLIC_PAYLOAD_AUTHORITY_PATH: &str = "manifests/public-release-payload.yaml";

const PUBLIC_CRATE_ROOTS: &[&str] = &[
    "crates/ags-platform",
    "crates/ags-workspace-facts",
    "crates/ags-host-integration",
    "crates/ags-capability-governance",
    "crates/ags-task-contract",
    "crates/ags-governance-decision",
    "crates/ags-session",
    "crates/ags-evidence",
    "crates/ags-verification",
    "crates/ags-lifecycle",
    "crates/ags-cli",
    "crates/ags-mcp",
];

/// Public-safe projection files that are allowed to differ byte-for-byte from
/// private A. The set is deliberately compiled into the verifier: editing the
/// YAML authority alone cannot create a new whole-file redaction escape hatch.
///
/// Every path in this set must also carry the exact SHA-256 of the projected B
/// file in [`PublicPayloadAuthority::public_rewrites`].
const APPROVED_PUBLIC_REWRITE_PATHS: &[&str] = &[
    "AGENTS.md",
    "AGENT_SUITE_PROTOCOL.md",
    "CLAUDE.md",
    "README.md",
    "README_EN.md",
    "WORKSPACE.md",
    "crates/ags-capability-governance/src/lib.rs",
    "crates/ags-cli/src/cli/kernel_actions.rs",
    "crates/ags-cli/src/cli/mod.rs",
    "crates/ags-cli/src/init/mod.rs",
    "crates/ags-cli/src/kernel/mcp.rs",
    "crates/ags-cli/src/setup/mod.rs",
    "crates/ags-cli/src/skill/mod.rs",
    "crates/ags-cli/src/update/apply.rs",
    "crates/ags-cli/src/update/repair.rs",
    "crates/ags-cli/tests/public_api_facade_contract.rs",
    "crates/ags-lifecycle/src/init/managed_projects.rs",
    "crates/ags-lifecycle/src/init/plan.rs",
    "crates/ags-lifecycle/src/update/apply.rs",
    "crates/ags-lifecycle/src/setup/apply.rs",
    "crates/ags-lifecycle/src/setup/memory/adapter.rs",
    "crates/ags-lifecycle/src/setup/memory/mod.rs",
    "crates/ags-lifecycle/src/setup/mod.rs",
    "crates/ags-lifecycle/src/setup/plan.rs",
    "crates/ags-lifecycle/src/setup/plan/tests.rs",
    "crates/ags-lifecycle/src/setup/templates.rs",
    "crates/ags-lifecycle/src/setup/verify.rs",
    "crates/ags-mcp/src/lib.rs",
    "crates/ags-mcp/src/prompts.rs",
    "crates/ags-mcp/src/prompts/delivery_report.txt",
    "crates/ags-mcp/src/prompts/global_kernel.txt",
    "crates/ags-mcp/src/resources.rs",
    "crates/ags-mcp/src/resources/global_kernel.md",
    "crates/ags-mcp/src/tools/apply.rs",
    "crates/ags-verification/src/bootstrap.rs",
    "crates/ags-verification/src/doctor/checks/mod.rs",
    "crates/ags-verification/src/doctor/checks/orchestration.rs",
    "crates/ags-verification/src/doctor/checks/runtime.rs",
    "docs/adr/0001-workspace-service-and-deep-modules.md",
    "docs/architecture.md",
    "governance/skill-sync.md",
    "manifests/mcp-registry.yaml",
    "manifests/skills-registry.yaml",
    "manifests/suite.yaml",
    "protocol/agent-task-protocol.md",
    "protocol/context-memory.md",
    "protocol/mcp-server.md",
    "protocol/runtime-adapters.md",
    "protocol/skill-governance.md",
];

/// B-owned release overlays are also a closed, compiled set. Each entry is
/// hash-pinned in the YAML authority; a manifest-only edit cannot add another
/// workflow, script, Rust source, or arbitrary tracked file to the release.
const APPROVED_PUBLIC_OVERLAY_PATHS: &[&str] = &[
    ".gitattributes",
    ".gitignore",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
    ".github/ISSUE_TEMPLATE/feature_request.yml",
    ".github/PULL_REQUEST_TEMPLATE.md",
    ".github/workflows/ci.yml",
    ".github/workflows/npm-publish.yml",
    ".github/workflows/release.yml",
    "COMMERCIAL.md",
    "CONTRIBUTING.md",
    "LICENSE",
    "NOTICE.md",
    "THIRD_PARTY_NOTICES.md",
    "docs/comparison.md",
    "docs/philosophy.en.md",
    "docs/philosophy.md",
    "evals/case-01-authority-escalation.md",
    "evals/case-02-unverified-delivery.md",
    "evals/case-03-solution-as-execution.md",
    "examples/README.md",
    "examples/demo-project/AGENTS.md",
    "examples/demo-project/CLAUDE.md",
    "examples/demo-project/Cargo.toml",
    "examples/demo-project/README.md",
    "examples/demo-project/src/main.rs",
    "examples/demo-project/tests/demo_test.rs",
    "examples/outputs/sample-preflight-output.txt",
    "examples/outputs/sample-verify-output.txt",
    "examples/receipts/sample-receipt.json",
    "examples/task-cards/light-demo-task.md",
    "examples/task-cards/medium-demo-task.md",
    "governance/skills-inventory.md",
    "templates/command-skills/ags-agents/SKILL.md",
    "templates/command-skills/ags-doctor/SKILL.md",
    "templates/command-skills/ags-init/SKILL.md",
    "templates/command-skills/ags-setup/SKILL.md",
    "templates/command-skills/ags-skill/SKILL.md",
    "templates/memory/archive-index.md",
    "templates/memory/context-capsule.md",
    "templates/memory/task-archive/README.md",
    "templates/memory/task-memory.md",
];

pub fn is_approved_public_rewrite_path(path: &str) -> bool {
    APPROVED_PUBLIC_REWRITE_PATHS.contains(&path)
}

pub fn public_pinned_target_files(root: &Path) -> Result<BTreeSet<String>, Vec<String>> {
    let authority = load_public_payload_authority(root)?;
    Ok(authority
        .public_overlay_files
        .iter()
        .map(|overlay| overlay.path.clone())
        .chain(
            authority
                .public_rewrites
                .iter()
                .map(|rewrite| rewrite.path.clone()),
        )
        .collect())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPayloadAuthority {
    pub schema_version: String,
    pub projection_state: String,
    #[serde(default)]
    pub shared_files: Vec<String>,
    #[serde(default)]
    pub public_overlay_files: Vec<PublicPinnedFile>,
    #[serde(default)]
    pub crate_trees: Vec<PublicCrateTree>,
    #[serde(default)]
    pub public_rewrites: Vec<PublicRewrite>,
    #[serde(default)]
    pub runtime_asset_files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCrateTree {
    pub root: String,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicRewrite {
    pub path: String,
    pub target_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPinnedFile {
    pub path: String,
    pub target_sha256: String,
}

/// Load and validate the single public payload authority.
pub fn load_public_payload_authority(root: &Path) -> Result<PublicPayloadAuthority, Vec<String>> {
    let path = root.join(PUBLIC_PAYLOAD_AUTHORITY_PATH);
    let content = fs::read_to_string(&path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    let authority: PublicPayloadAuthority = serde_yaml::from_str(&content)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])?;
    let errors = validate_public_payload_authority(&authority);
    if errors.is_empty() {
        Ok(authority)
    } else {
        Err(errors)
    }
}

fn validate_public_payload_authority(authority: &PublicPayloadAuthority) -> Vec<String> {
    let mut errors = Vec::new();
    if authority.schema_version != "1.1" {
        errors.push(format!(
            "unsupported public payload schema_version: {}",
            authority.schema_version
        ));
    }
    if !matches!(
        authority.projection_state.as_str(),
        "provisional" | "frozen"
    ) {
        errors.push(format!(
            "public payload projection_state must be provisional or frozen: {}",
            authority.projection_state
        ));
    }

    let expected_roots: BTreeSet<_> = PUBLIC_CRATE_ROOTS.iter().copied().collect();
    let actual_roots: BTreeSet<_> = authority
        .crate_trees
        .iter()
        .map(|tree| tree.root.as_str())
        .collect();
    if actual_roots != expected_roots || authority.crate_trees.len() != expected_roots.len() {
        errors.push(format!(
            "crate_trees must contain exactly the twelve AGS crate roots; expected={expected_roots:?}, actual={actual_roots:?}"
        ));
    }

    let mut declared = BTreeSet::new();
    for path in &authority.shared_files {
        validate_authority_path(path, "file", &mut errors);
        if !declared.insert(path.as_str()) {
            errors.push(format!("duplicate public payload file: {path}"));
        }
        if is_public_forbidden_payload(path) {
            errors.push(format!(
                "public payload authority includes forbidden path: {path}"
            ));
        }
    }
    for overlay in &authority.public_overlay_files {
        validate_authority_path(&overlay.path, "public overlay", &mut errors);
        if !declared.insert(overlay.path.as_str()) {
            errors.push(format!("duplicate public payload file: {}", overlay.path));
        }
        if is_public_forbidden_payload(&overlay.path) {
            errors.push(format!(
                "public payload authority includes forbidden path: {}",
                overlay.path
            ));
        }
        if !valid_sha256(&overlay.target_sha256) {
            errors.push(format!(
                "public overlay must use a lowercase sha256:<64-hex> digest: {}",
                overlay.path
            ));
        }
    }
    if !authority
        .shared_files
        .iter()
        .any(|path| path == PUBLIC_PAYLOAD_AUTHORITY_PATH)
    {
        errors.push(format!(
            "{PUBLIC_PAYLOAD_AUTHORITY_PATH} must be a shared public payload file"
        ));
    }

    for tree in &authority.crate_trees {
        validate_authority_path(&tree.root, "crate root", &mut errors);
        for excluded in &tree.exclude {
            validate_authority_path(excluded, "crate exclusion", &mut errors);
            let full = format!("{}/{}", tree.root, excluded);
            if !is_public_forbidden_payload(&full)
                && full != "crates/ags-capability-governance/src/adoption/"
                && full != "crates/ags-cli/src/skill/adoption.rs"
                && full != "crates/ags-verification/src/doctor/checks/evolver_hooks.rs"
                && full != "crates/ags-verification/src/doctor/checks/proxy.rs"
                && full != "crates/ags-verification/src/doctor/checks/templates.rs"
            {
                errors.push(format!(
                    "crate exclusion is not an approved public-safe boundary: {full}"
                ));
            }
        }
    }

    let expected_rewrites: BTreeSet<_> = APPROVED_PUBLIC_REWRITE_PATHS.iter().copied().collect();
    let actual_rewrites: BTreeSet<_> = authority
        .public_rewrites
        .iter()
        .map(|rewrite| rewrite.path.as_str())
        .collect();
    if actual_rewrites != expected_rewrites
        || authority.public_rewrites.len() != expected_rewrites.len()
    {
        errors.push(format!(
            "public_rewrites must contain exactly the compiled approved rewrite paths; expected={expected_rewrites:?}, actual={actual_rewrites:?}"
        ));
    }

    let expected_overlays: BTreeSet<_> = APPROVED_PUBLIC_OVERLAY_PATHS.iter().copied().collect();
    let actual_overlays: BTreeSet<_> = authority
        .public_overlay_files
        .iter()
        .map(|overlay| overlay.path.as_str())
        .collect();
    if actual_overlays != expected_overlays
        || authority.public_overlay_files.len() != expected_overlays.len()
    {
        errors.push(format!(
            "public_overlay_files must contain exactly the compiled approved overlay paths; expected={expected_overlays:?}, actual={actual_overlays:?}"
        ));
    }

    for rewrite in &authority.public_rewrites {
        validate_authority_path(&rewrite.path, "public rewrite", &mut errors);
        if authority
            .public_overlay_files
            .iter()
            .any(|overlay| overlay.path == rewrite.path)
        {
            errors.push(format!(
                "public overlay cannot also be a source rewrite: {}",
                rewrite.path
            ));
        }
        if !valid_sha256(&rewrite.target_sha256) {
            errors.push(format!(
                "public rewrite must use a lowercase sha256:<64-hex> digest: {}",
                rewrite.path
            ));
        }
    }

    let mut runtime_assets = BTreeSet::new();
    for path in &authority.runtime_asset_files {
        validate_authority_path(path, "authority reference", &mut errors);
        if !runtime_assets.insert(path.as_str()) {
            errors.push(format!("duplicate runtime asset file: {path}"));
        }
    }

    errors
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_authority_path(path: &str, label: &str, errors: &mut Vec<String>) {
    let normalized = path.replace('\\', "/");
    if path.is_empty()
        || normalized.starts_with('/')
        || normalized.starts_with("./")
        || normalized.split('/').any(|part| part == "..")
        || normalized != path
    {
        errors.push(format!(
            "unsafe {label} path in public payload authority: {path}"
        ));
    }
}

fn crate_tree_files(root: &Path, tree: &PublicCrateTree) -> BTreeSet<String> {
    let mut files = BTreeSet::from([format!("{}/Cargo.toml", tree.root)]);
    for directory in ["src", "tests"] {
        let base = root.join(&tree.root).join(directory);
        collect_tree_files(root, &base, &mut files);
    }
    for excluded in &tree.exclude {
        let excluded = format!("{}/{}", tree.root, excluded);
        files.retain(|path| {
            path != &excluded && !path.starts_with(&format!("{}/", excluded.trim_end_matches('/')))
        });
    }
    files
}

fn collect_tree_files(root: &Path, directory: &Path, files: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || metadata.is_file() {
            if let Ok(relative) = path.strip_prefix(root) {
                files.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        } else if metadata.is_dir() {
            collect_tree_files(root, &path, files);
        }
    }
}

/// Expand the exact public source payload from the authority at `root`.
pub fn public_payload_files(root: &Path) -> Result<BTreeSet<String>, Vec<String>> {
    let authority = load_public_payload_authority(root)?;
    Ok(expand_public_payload_files(root, &authority))
}

fn expand_public_payload_files(
    root: &Path,
    authority: &PublicPayloadAuthority,
) -> BTreeSet<String> {
    let mut files: BTreeSet<String> = authority.shared_files.iter().cloned().collect();
    files.extend(
        authority
            .public_overlay_files
            .iter()
            .map(|overlay| overlay.path.clone()),
    );
    for tree in &authority.crate_trees {
        files.extend(crate_tree_files(root, tree));
    }
    files
}

fn source_projected_files(root: &Path, authority: &PublicPayloadAuthority) -> BTreeSet<String> {
    let mut files: BTreeSet<String> = authority.shared_files.iter().cloned().collect();
    for tree in &authority.crate_trees {
        files.extend(crate_tree_files(root, tree));
    }
    files
}

/// Expand the A-owned source side of the canonical public payload.
///
/// Public-only overlays and rewrites are deliberately excluded because they
/// are owned by the downstream public projection, not by workspace A.
pub fn public_source_payload_files(root: &Path) -> Result<BTreeSet<String>, Vec<String>> {
    let authority = load_public_payload_authority(root)?;
    Ok(source_projected_files(root, &authority))
}

pub fn public_runtime_asset_files(root: &Path) -> Result<Vec<String>, Vec<String>> {
    let authority = load_public_payload_authority(root)?;
    let expected = expand_public_payload_files(root, &authority);
    let mut errors = Vec::new();
    for path in &authority.runtime_asset_files {
        if !expected.contains(path) {
            errors.push(format!(
                "runtime asset is outside canonical public payload: {path}"
            ));
        }
        if let Err(error) = validate_regular_payload_file(root, path) {
            errors.push(format!("runtime asset {error}"));
        }
    }
    if errors.is_empty() {
        Ok(authority.runtime_asset_files)
    } else {
        Err(errors)
    }
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
    pub content_mismatches: Vec<String>,
    pub authority_errors: Vec<String>,
}

/// Verify a target directory against the public release manifest.
///
/// Checks:
/// 1. All PUBLIC_MANIFEST required files are present in the target.
/// 2. No `PUBLIC_FORBIDDEN_PAYLOAD` files are present in the target.
/// 3. Reports any extra files not in either list.
pub fn verify_release_manifest(target: &std::path::Path) -> ManifestVerifyResult {
    verify_public_payload_against(target, target, false)
}

/// Verify an explicit public promotion target against the source authority.
///
/// Unlike release self-verification, this expands crate trees from the source
/// checkout and therefore detects target-only source files and source/target
/// content drift outside the explicit rewritten-file list.
pub fn verify_promotion_manifest(source: &Path, target: &Path) -> ManifestVerifyResult {
    verify_public_payload_against(source, target, true)
}

fn verify_public_payload_against(
    authority_root: &Path,
    target: &Path,
    compare_content: bool,
) -> ManifestVerifyResult {
    let mut required_present: Vec<String> = Vec::new();
    let mut required_missing: Vec<String> = Vec::new();
    let mut forbidden_found: Vec<String> = Vec::new();
    let mut extra_files: Vec<String> = Vec::new();
    let mut content_mismatches: Vec<String> = Vec::new();
    let mut authority_errors: Vec<String> = Vec::new();

    let target_files = list_files(target);
    let target_set: BTreeSet<String> = target_files.iter().cloned().collect();

    let authority = match load_public_payload_authority(authority_root) {
        Ok(authority) => Some(authority),
        Err(errors) => {
            authority_errors.extend(errors);
            None
        }
    };
    if authority
        .as_ref()
        .is_some_and(|authority| authority.projection_state != "frozen")
    {
        authority_errors.push(
            "public payload projection is provisional; freeze reviewed B hashes before promotion or release"
                .to_string(),
        );
    }
    let expected = authority
        .as_ref()
        .map(|authority| expand_public_payload_files(authority_root, authority))
        .unwrap_or_else(|| {
            PUBLIC_MANIFEST
                .required_files
                .iter()
                .map(|path| (*path).to_string())
                .collect()
        });

    for required in &expected {
        if target_set.contains(required) {
            required_present.push(required.clone());
        } else {
            required_missing.push(required.clone());
        }
    }
    for relative in &target_set {
        if let Err(error) = validate_regular_payload_file(target, relative) {
            authority_errors.push(format!("public target {error}"));
        }
        if is_public_forbidden_payload(relative) {
            forbidden_found.push(relative.clone());
        }
    }
    for relative in target_set.difference(&expected) {
        if !is_public_forbidden_payload(relative) {
            extra_files.push(relative.clone());
        }
    }

    if let Some(authority) = &authority {
        for overlay in &authority.public_overlay_files {
            verify_pinned_target_file(
                target,
                &overlay.path,
                &overlay.target_sha256,
                &mut content_mismatches,
                &mut authority_errors,
                "public overlay",
            );
        }
        for rewrite in &authority.public_rewrites {
            verify_pinned_target_file(
                target,
                &rewrite.path,
                &rewrite.target_sha256,
                &mut content_mismatches,
                &mut authority_errors,
                "public rewrite",
            );
        }

        if compare_content {
            let target_authority = load_public_payload_authority(target);
            if target_authority.as_ref().ok() != Some(authority) {
                authority_errors
                    .push("public target authority differs from source authority".to_string());
            }
            let rewritten: BTreeSet<_> = authority
                .public_rewrites
                .iter()
                .map(|rewrite| rewrite.path.as_str())
                .collect();
            for relative in source_projected_files(authority_root, authority) {
                if let Err(error) = validate_regular_payload_file(authority_root, &relative) {
                    authority_errors.push(format!("public source {error}"));
                    continue;
                }
                if rewritten.contains(relative.as_str()) || !target.join(&relative).is_file() {
                    continue;
                }
                let source_bytes = fs::read(authority_root.join(&relative));
                let target_bytes = fs::read(target.join(&relative));
                if source_bytes.ok() != target_bytes.ok() {
                    content_mismatches.push(relative);
                }
            }
        }
    }

    required_present.sort();
    required_missing.sort();
    forbidden_found.sort();
    extra_files.sort();
    content_mismatches.sort();
    authority_errors.sort();
    authority_errors.dedup();

    let passed = required_missing.is_empty()
        && forbidden_found.is_empty()
        && extra_files.is_empty()
        && content_mismatches.is_empty()
        && authority_errors.is_empty();

    ManifestVerifyResult {
        target: target.display().to_string(),
        passed,
        required_present,
        required_missing,
        forbidden_found,
        extra_files,
        content_mismatches,
        authority_errors,
    }
}

fn verify_pinned_target_file(
    target: &Path,
    relative: &str,
    expected_sha256: &str,
    content_mismatches: &mut Vec<String>,
    authority_errors: &mut Vec<String>,
    label: &str,
) {
    let target_path = target.join(relative);
    if !target_path.is_file() {
        return;
    }
    match fs::read(&target_path) {
        Ok(bytes) if ags_platform::sha256(&bytes) == expected_sha256 => {}
        Ok(_) => content_mismatches.push(relative.to_string()),
        Err(error) => authority_errors.push(format!("cannot hash {label} {relative}: {error}")),
    }
}

fn validate_regular_payload_file(root: &Path, relative: &str) -> Result<(), String> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| format!("root cannot be read: {error}"))?;
    if root_metadata.file_type().is_symlink() {
        return Err("root must not be a symlink".to_string());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("root cannot be resolved: {error}"))?;
    let mut candidate = root.to_path_buf();
    for component in Path::new(relative).components() {
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|error| format!("payload path cannot be read: {relative}: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "payload path must not contain a symlink: {relative}"
            ));
        }
    }
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("payload path cannot be read: {relative}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("payload path is not a regular file: {relative}"));
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("payload path cannot be resolved: {relative}: {error}"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!("payload path escapes root: {relative}"));
    }
    Ok(())
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
            if fs::symlink_metadata(root.join(&relative)).is_ok() {
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
            .replace('\\', "/");
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || metadata.is_file() {
            files.push(relative);
        } else if metadata.is_dir() {
            list_files_recursive(root, &path, files);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
    }

    fn install_fixture_authority(root: &Path, rewrite_content: &[u8]) -> PublicPayloadAuthority {
        let mut authority = load_public_payload_authority(workspace_root()).unwrap();
        authority.projection_state = "frozen".to_string();
        let digest = ags_platform::sha256(rewrite_content);
        for overlay in &mut authority.public_overlay_files {
            overlay.target_sha256.clone_from(&digest);
        }
        for rewrite in &mut authority.public_rewrites {
            rewrite.target_sha256.clone_from(&digest);
        }
        let authority_path = root.join(PUBLIC_PAYLOAD_AUTHORITY_PATH);
        std::fs::create_dir_all(authority_path.parent().unwrap()).unwrap();
        std::fs::write(authority_path, serde_yaml::to_string(&authority).unwrap()).unwrap();
        authority
    }

    fn populate_fixture_payload(root: &Path, authority: &PublicPayloadAuthority, content: &[u8]) {
        for file in expand_public_payload_files(root, authority) {
            if file == PUBLIC_PAYLOAD_AUTHORITY_PATH {
                continue;
            }
            let path = root.join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn recursive_listing_uses_manifest_separators() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested").join("file.txt");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(nested, b"fixture").unwrap();

        assert_eq!(list_files(dir.path()), vec!["nested/file.txt".to_string()]);
    }

    #[test]
    fn public_manifest_includes_public_core_protocol_and_scripts() {
        let public: BTreeSet<_> = PUBLIC_MANIFEST.required_files.iter().copied().collect();
        for path in [
            "AGENTS.md",
            "CLAUDE.md",
            "WORKSPACE.md",
            "AGENT_SUITE_PROTOCOL.md",
            "protocol/entrypoint-guidelines.md",
            "RELEASE_NOTES.md",
            "SECURITY.md",
            "LICENSE",
            "NOTICE.md",
            "THIRD_PARTY_NOTICES.md",
            "COMMERCIAL.md",
            "templates/memory/context-capsule.md",
            "templates/memory/task-memory.md",
            "templates/global-entry/ags-core.md",
            "templates/global-entry/ags-task-handoff.md",
            "templates/global-entry/host-operations.md",
            "scripts/ags-memory-lifecycle-omp.js",
            "manifests/onboarding-public.yaml",
            "manifests/third-party-capabilities.yaml",
            "packages/ags-mcp/package.json",
            "packages/ags-mcp/LICENSE",
            "packages/ags-mcp/bin/ags-mcp.js",
            ".github/workflows/ci.yml",
            ".github/workflows/release.yml",
            ".github/workflows/npm-publish.yml",
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
    fn public_full_sanitized_uses_expanded_manifest() {
        assert!(PUBLIC_MANIFEST.required_files.len() > 20);
    }

    #[test]
    fn public_forbidden_payload_covers_build_artifacts_and_runtime_state() {
        for path in [
            "target/release/ags",
            "ags",
            "global-skills/custom/SKILL.md",
            "skill-packs/personal/example/SKILL.md",
            ".agents/memory/projects/demo/context-capsule.md",
            ".codex/skills/example/SKILL.md",
            "capability-snapshot/codex.json",
            "skill-registry/user-overlay.yaml",
            "skill-usage/codex.ndjson",
            "decision-leases/current.json",
            "auth-state/codex.json",
            "receipts/ar-runtime.json",
            "workspace-services/workspace.capabilities.json",
            ".ags/private-runtime/skill-registry/user-overlay.yaml",
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
        assert!(is_public_forbidden_payload(".evolver/gep/genes.json"));
        assert!(is_public_forbidden_payload("assets/gep/capsules.json"));
        assert!(is_public_forbidden_payload(
            "crates/ags-mcp/assets/gep/capsules.json"
        ));
        assert!(is_public_forbidden_payload(
            "crates/ags-capability-governance/assets/gep/genes.json"
        ));
        assert!(is_public_forbidden_payload("evomap/README.md"));
        assert!(is_public_forbidden_payload("mcp/gep.mcp.json"));
        assert!(is_public_forbidden_payload(
            "hosts/claude-code.evomap-mcp.snippet.json"
        ));
        assert!(is_public_forbidden_payload("bin/evolver-proxy-mcp"));
        assert!(!is_public_forbidden_payload(
            "manifests/templates/runtime-profiles.template.yaml"
        ));
        assert!(is_public_forbidden_payload(
            "manifests/runtime-profiles.yaml"
        ));
        assert!(is_public_forbidden_payload("protocol/evolution-memory.md"));
        assert!(is_public_forbidden_payload(
            "capability-snapshot/codex.json"
        ));
        assert!(is_public_forbidden_payload(
            "skill-registry/user-overlay.yaml"
        ));
        assert!(is_public_forbidden_payload("skill-usage/codex.ndjson"));
        assert!(is_public_forbidden_payload("decision-leases/lease.json"));
        assert!(is_public_forbidden_payload("auth-state/codex.json"));
        assert!(is_public_forbidden_payload("receipts/action.json"));
        assert!(is_public_forbidden_payload(
            ".ags/private-runtime/capability-snapshot/codex.json"
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
            "scripts/ags-memory-lifecycle-omp.js",
            "README.md",
            // Public targets may have their own governance/manifests and empty
            // audit skeletons. Non-empty private audit content is checked by
            // release sanitize, not path-level manifest filtering.
            "governance/sync-protocol.md",
            "governance/inventory-schema.md",
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
    fn verify_release_manifest_accepts_clean_target() {
        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"placeholder");
        populate_fixture_payload(dir.path(), &authority, b"placeholder");

        let result = verify_release_manifest(dir.path());
        assert!(
            result.passed,
            "clean target should pass. missing={:?}, forbidden={:?}, extra={:?}, authority={:?}",
            result.required_missing,
            result.forbidden_found,
            result.extra_files,
            result.authority_errors,
        );
        assert!(result.required_missing.is_empty());
        assert!(result.forbidden_found.is_empty());
        assert!(result.extra_files.is_empty());
    }

    #[test]
    fn public_payload_authority_covers_exactly_twelve_crate_source_trees() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let authority = load_public_payload_authority(workspace).unwrap();
        assert_eq!(authority.crate_trees.len(), 12);
        let files = expand_public_payload_files(workspace, &authority);
        for root in PUBLIC_CRATE_ROOTS {
            assert!(files.contains(&format!("{root}/Cargo.toml")));
            assert!(
                files
                    .iter()
                    .any(|path| path.starts_with(&format!("{root}/src/")) && path.ends_with(".rs")),
                "public authority must include Rust sources for {root}"
            );
        }
        for path in [
            "crates/ags-host-integration/src/lifecycle_codec.rs",
            "crates/ags-lifecycle/src/conformance.rs",
            "crates/ags-lifecycle/src/lifecycle_projection.rs",
            "crates/ags-lifecycle/src/workspace_lifecycle.rs",
            "crates/ags-verification/src/doctor/checks/conformance.rs",
        ] {
            assert!(
                files.contains(path),
                "v0.4.1 public authority must include {path}"
            );
        }
        for retired in [
            "crates/ags-lifecycle/src/setup/memory/assets.rs",
            "crates/ags-lifecycle/src/setup/memory/wire.rs",
        ] {
            assert!(
                !files.contains(retired),
                "retired lifecycle source must not remain required: {retired}"
            );
        }
        assert!(!files
            .iter()
            .any(|path| path.starts_with("crates/ags-capability-governance/src/adoption/")));
        assert!(!files.contains("crates/ags-cli/src/skill/adoption.rs"));
        assert!(!files.contains("crates/ags-mcp/src/resources/evolver_boundary.md"));
    }

    #[test]
    fn extra_public_file_is_a_release_failure() {
        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"placeholder");
        populate_fixture_payload(dir.path(), &authority, b"placeholder");
        std::fs::create_dir_all(dir.path().join("crates/retired/src")).unwrap();
        std::fs::write(dir.path().join("crates/retired/src/lib.rs"), "private").unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert_eq!(
            result.extra_files,
            vec!["crates/retired/src/lib.rs".to_string()]
        );
    }

    #[test]
    fn runtime_assets_are_a_subset_of_the_canonical_payload() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let assets = public_runtime_asset_files(workspace).unwrap();
        assert!(assets.contains(&"manifests/skills-registry.yaml".to_string()));
        assert!(assets.contains(&"manifests/mcp-registry.yaml".to_string()));
        for path in [
            "protocol/project-profile.md",
            "protocol/context-memory.md",
            "protocol/cursor-skill-index.md",
        ] {
            assert!(assets.contains(&path.to_string()));
        }
    }

    #[test]
    fn public_payload_authority_shared_files_exist_in_the_authority_tree() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let authority = load_public_payload_authority(workspace).unwrap();
        let missing = authority
            .shared_files
            .iter()
            .filter(|relative| !workspace.join(relative).is_file())
            .cloned()
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "public payload authority references missing shared files: {}",
            missing.join(", ")
        );
    }

    #[test]
    fn public_payload_authority_pins_target_only_memory_templates() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let authority = load_public_payload_authority(workspace).unwrap();
        let overlays = authority
            .public_overlay_files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<BTreeSet<_>>();

        for relative in [
            "templates/memory/archive-index.md",
            "templates/memory/context-capsule.md",
            "templates/memory/task-archive/README.md",
            "templates/memory/task-memory.md",
        ] {
            assert!(
                overlays.contains(relative),
                "public-only memory template must be a pinned overlay: {relative}"
            );
            assert!(
                !authority.shared_files.iter().any(|path| path == relative),
                "public-only memory template cannot be declared shared: {relative}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_assets_reject_symlinked_parent_directory() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"approved");
        populate_fixture_payload(dir.path(), &authority, b"approved");
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("project-profile.md"), b"approved").unwrap();
        std::fs::remove_dir_all(dir.path().join("protocol")).unwrap();
        symlink(outside.path(), dir.path().join("protocol")).unwrap();

        let errors = public_runtime_asset_files(dir.path()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("must not contain a symlink")));
    }

    #[test]
    fn promotion_rejects_source_drift_and_target_only_payload() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(source.path(), b"same");
        std::fs::create_dir_all(
            target
                .path()
                .join(PUBLIC_PAYLOAD_AUTHORITY_PATH)
                .parent()
                .unwrap(),
        )
        .unwrap();
        std::fs::copy(
            source.path().join(PUBLIC_PAYLOAD_AUTHORITY_PATH),
            target.path().join(PUBLIC_PAYLOAD_AUTHORITY_PATH),
        )
        .unwrap();
        for root in [source.path(), target.path()] {
            populate_fixture_payload(root, &authority, b"same");
        }
        let source_code = source.path().join("crates/ags-platform/src/lib.rs");
        let target_code = target.path().join("crates/ags-platform/src/lib.rs");
        std::fs::create_dir_all(source_code.parent().unwrap()).unwrap();
        std::fs::create_dir_all(target_code.parent().unwrap()).unwrap();
        std::fs::write(source_code, "source").unwrap();
        std::fs::write(target_code, "drifted").unwrap();
        std::fs::create_dir_all(target.path().join("crates/retired/src")).unwrap();
        std::fs::write(target.path().join("crates/retired/src/lib.rs"), "extra").unwrap();

        let result = verify_promotion_manifest(source.path(), target.path());
        assert!(!result.passed);
        assert_eq!(
            result.content_mismatches,
            vec!["crates/ags-platform/src/lib.rs".to_string()]
        );
        assert_eq!(
            result.extra_files,
            vec!["crates/retired/src/lib.rs".to_string()]
        );
    }

    #[test]
    fn release_rejects_rewritten_file_with_wrong_digest() {
        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"approved");
        populate_fixture_payload(dir.path(), &authority, b"approved");
        std::fs::write(dir.path().join("README.md"), b"unreviewed rewrite").unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert_eq!(result.content_mismatches, vec!["README.md".to_string()]);
    }

    #[test]
    fn release_rejects_public_overlay_with_wrong_digest() {
        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"approved");
        populate_fixture_payload(dir.path(), &authority, b"approved");
        std::fs::write(
            dir.path().join(".github/workflows/release.yml"),
            b"unreviewed workflow",
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert_eq!(
            result.content_mismatches,
            vec![".github/workflows/release.yml".to_string()]
        );
    }

    #[test]
    fn release_rejects_provisional_projection_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let mut authority = install_fixture_authority(dir.path(), b"approved");
        populate_fixture_payload(dir.path(), &authority, b"approved");
        authority.projection_state = "provisional".to_string();
        std::fs::write(
            dir.path().join(PUBLIC_PAYLOAD_AUTHORITY_PATH),
            serde_yaml::to_string(&authority).unwrap(),
        )
        .unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(result
            .authority_errors
            .iter()
            .any(|error| error.contains("projection is provisional")));
    }

    #[test]
    fn authority_rejects_uncompiled_rewrite_and_invalid_digest() {
        let mut authority = load_public_payload_authority(workspace_root()).unwrap();
        authority.public_rewrites.push(PublicRewrite {
            path: ".github/workflows/release.yml".to_string(),
            target_sha256: "sha256:not-a-digest".to_string(),
        });
        authority.public_overlay_files.push(PublicPinnedFile {
            path: "crates/extra/src/lib.rs".to_string(),
            target_sha256: ags_platform::sha256(b"extra"),
        });
        authority.projection_state = "unknown".to_string();
        let errors = validate_public_payload_authority(&authority);
        assert!(errors
            .iter()
            .any(|error| error.contains("compiled approved rewrite paths")));
        assert!(errors
            .iter()
            .any(|error| error.contains("lowercase sha256")));
        assert!(errors
            .iter()
            .any(|error| error.contains("compiled approved overlay paths")));
        assert!(errors
            .iter()
            .any(|error| error.contains("projection_state")));
    }

    #[cfg(unix)]
    #[test]
    fn release_rejects_symlink_in_canonical_payload() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let authority = install_fixture_authority(dir.path(), b"approved");
        populate_fixture_payload(dir.path(), &authority, b"approved");
        let outside = dir.path().join("outside");
        std::fs::write(&outside, b"approved").unwrap();
        let target = dir.path().join("README.md");
        std::fs::remove_file(&target).unwrap();
        symlink(&outside, &target).unwrap();

        let result = verify_release_manifest(dir.path());
        assert!(!result.passed);
        assert!(result
            .authority_errors
            .iter()
            .any(|error| error.contains("must not contain a symlink")));
    }

    #[test]
    fn release_workflow_consumes_the_canonical_plan_without_copy_lists() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let workflow =
            std::fs::read_to_string(workspace.join(".github/workflows/release.yml")).unwrap();
        assert!(workflow.contains("release package"));
        assert!(workflow.contains("release stage-runtime"));
        assert!(!workflow.contains("python"));
        assert!(!workflow.contains("cp manifests/{"));
        assert!(!workflow.contains("Copy-Item manifests/skills-registry.yaml"));
    }
}
