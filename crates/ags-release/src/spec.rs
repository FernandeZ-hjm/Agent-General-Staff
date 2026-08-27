//! Public projection spec (contract v3, v0.4.21).
//!
//! Exactly one typed definition of what A projects into B. B is the
//! deterministic public-safe product, not a mirror of A. Everything not
//! listed here is private or B-owned and never leaves A.

/// Directories projected byte-exact from A into B (recursive).
pub const PUBLIC_DIRS: &[&str] = &[
    "ags-skills",
    "crates/ags-kernel",
    "crates/ags-task-contract",
    "crates/ags-cli",
    "crates/ags-mcp",
    "crates/ags-release",
    "packages",
    "templates/hooks",
];

/// Single files projected byte-exact from A into B.
pub const PUBLIC_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "LICENSE",
    "deny.toml",
    "THIRD_PARTY_NOTICES.md",
    "SECURITY.md",
    "README.md",
    "README_EN.md",
    "RELEASE_NOTES.md",
    "docs/architecture.md",
];

/// A files that are always private: never projected, and any appearance of
/// their paths in projected content is a blocking sensitivity finding.
pub const PRIVATE_ROOTS: &[&str] = &[
    "protocol/",
    "proposals/",
    "manifests/",
    "config/",
    "skill-packs/",
    "assets/",
    "scripts/",
    "memory/",
    "governance/",
    "graphify-out/",
    ".ags",
    ".ags-close-artifacts",
    ".claude",
    ".codex",
    ".cursor",
    ".codebuddy",
    ".omp",
    ".evolver",
    ".superpowers",
    ".codegraph",
    ".worktrees",
    ".github",
    "target/",
    "node_modules",
];

/// Root files that stay private in A (documented as private by name).
pub const PRIVATE_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "CLAUDE-FABLE-5.md",
    ".gitignore",
    ".graphifyignore",
    "docs/v2-vs-v3-comparison.md",
];

/// B-owned overlays: never synced from A, never retired by the projection.
pub const B_OWNED: &[&str] = &[".github", ".gitignore", ".ags-local", "ags.toml"];

/// Strings that must never appear in projected content. Any hit is a
/// blocking finding for apply (reported by --review). Patterns are built
/// with `concat!` so the scanner never self-flags this file.
pub const SENSITIVE_PATTERNS: &[&str] = &[
    concat!("/Volumes/", "My ", "Passport"),
    concat!("/Users/", "hu", "jiaming"),
    concat!("agent-governance-suite-", "private"),
    concat!("agent-governance-suite-", "stable"),
    concat!("My ", "Passport"),
    concat!("hu", "jiaming"),
    concat!("Evo", "Map"),
    concat!("@evo", "map"),
    concat!("GEP ", "runtime"),
    concat!("dsh-", "stable"),
];

/// Old manifests generated for B by the retired capability projector. These
/// are the "retire on projection" files the contract requires the plan to
/// list before apply.
pub const RETIRED_MANIFESTS: &[&str] = &[
    "manifests/suite.yaml",
    "manifests/skills-registry.yaml",
    "manifests/mcp-registry.yaml",
    "manifests/onboarding-public.yaml",
    "manifests/public-capability-projection.yaml",
    "manifests/public-release-payload.yaml",
    "manifests/third-party-capabilities.yaml",
];
