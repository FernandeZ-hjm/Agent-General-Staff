//! Diff-aware Change Lane classification.
//!
//! Maps a set of changed file paths to a deterministic `ChangeLane`, and binds
//! each lane to a minimal-sufficient `VerificationProfile`. This lets hygiene
//! changes (e.g. a `.gitignore` edit) skip the full Rust gate while keeping
//! protocol/core and release-manifest changes on their guarded paths.
//!
//! # Design
//!
//! - **Deterministic** — every path is classified by a fixed precedence of
//!   path prefixes, file names, and extensions. No model judgment.
//! - **Fail-safe escalation** — when a change set spans multiple lanes, the
//!   effective lane is the component with the *highest* verification profile.
//!   Unknown files fall back to `SourceCode` (Standard), never to Minimal.
//! - **Protected-path first** — `protocol/`, root governance entry files, and
//!   `governance/` always escalate to `ProtocolCore`; `manifests/` to
//!   `ReleaseSync`. This mirrors the protected-path concept in
//!   `task-card-validator` but operates on repo-relative paths for diff scope.

use serde::Serialize;

/// A change lane — the category of a change set, by what it touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeLane {
    /// Only ignore-rule files (`.gitignore`, `.dockerignore`, …).
    IgnoreOnly,
    /// Only documentation (`.md`, `.txt`, README/CHANGELOG/LICENSE) outside protocol/.
    DocsOnly,
    /// Only config (`.yaml` / `.yml` / `.toml` / `.json`) outside Cargo + manifests.
    ConfigOnly,
    /// Rust source, scripts, tests, `Cargo.toml` / `Cargo.lock`.
    SourceCode,
    /// `protocol/` files, `governance/`, and root governance entry files
    /// (`AGENTS.md`, `CLAUDE.md`, `AGENT_SUITE_PROTOCOL.md`, `WORKSPACE.md`).
    ProtocolCore,
    /// Release surface: `manifests/` (suite manifest, capability metadata,
    /// runtime profiles, MCP registry).
    ReleaseSync,
}

impl ChangeLane {
    /// Stable snake_case identifier, also used in JSON output and shell scripts.
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeLane::IgnoreOnly => "ignore_only",
            ChangeLane::DocsOnly => "docs_only",
            ChangeLane::ConfigOnly => "config_only",
            ChangeLane::SourceCode => "source_code",
            ChangeLane::ProtocolCore => "protocol_core",
            ChangeLane::ReleaseSync => "release_sync",
        }
    }

    /// The minimal-sufficient verification profile bound to this lane.
    pub fn profile(&self) -> VerificationProfile {
        match self {
            ChangeLane::IgnoreOnly => VerificationProfile::Minimal,
            ChangeLane::DocsOnly => VerificationProfile::Minimal,
            ChangeLane::ConfigOnly => VerificationProfile::YamlParse,
            ChangeLane::SourceCode => VerificationProfile::Standard,
            ChangeLane::ProtocolCore => VerificationProfile::Protocol,
            ChangeLane::ReleaseSync => VerificationProfile::Release,
        }
    }
}

/// Minimal-sufficient verification effort. Ordering is `Minimal < YamlParse <
/// Standard < Protocol < Release` — used to pick the strongest profile when a
/// change set spans multiple lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationProfile {
    /// `cargo fmt --check` only (ignore-only / docs-only hygiene).
    Minimal,
    /// fmt + governance YAML parse (config-only).
    YamlParse,
    /// fmt + test + build + fixtures + yaml + preflight + templates (= local scope).
    Standard,
    /// Standard plus governance-protocol checks.
    Protocol,
    /// Protocol plus the separately composed release scope.
    Release,
}

impl VerificationProfile {
    /// Stable snake_case identifier for JSON output and shell scripts.
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationProfile::Minimal => "minimal",
            VerificationProfile::YamlParse => "yaml_parse",
            VerificationProfile::Standard => "standard",
            VerificationProfile::Protocol => "protocol",
            VerificationProfile::Release => "release",
        }
    }
}

/// The result of classifying a change set.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeClassification {
    /// The effective lane — the component with the highest verification profile.
    pub lane: ChangeLane,
    /// Every distinct lane present in the change set, ordered by ascending profile.
    pub components: Vec<ChangeLane>,
    /// The verification profile bound to `lane`.
    pub profile: VerificationProfile,
    /// The changed file paths that were analyzed (trimmed, non-empty).
    pub changed_files: Vec<String>,
}

/// Classify a single repo-relative path into exactly one lane.
///
/// Precedence (first match wins): ProtocolCore → ReleaseSync → IgnoreOnly →
/// SourceCode → ConfigOnly → DocsOnly → SourceCode (fallback).
fn classify_single_file(path: &str) -> ChangeLane {
    let p = path.trim();
    let lower = p.to_lowercase();
    let basename = p.rsplit('/').next().unwrap_or(p);
    let basename_lower = basename.to_lowercase();

    // 1. ProtocolCore — protocol/, governance/, and root governance entry files.
    if p.starts_with("protocol/")
        || p.starts_with("governance/")
        || matches!(
            basename,
            "AGENTS.md" | "CLAUDE.md" | "AGENT_SUITE_PROTOCOL.md" | "WORKSPACE.md"
        )
    {
        return ChangeLane::ProtocolCore;
    }

    // 2. ReleaseSync — manifests/ (suite manifest, capability metadata, profiles).
    if p.starts_with("manifests/") {
        return ChangeLane::ReleaseSync;
    }

    // 3. IgnoreOnly — dotfiles whose name ends in "ignore" (.gitignore, .dockerignore…).
    if basename.starts_with('.') && basename_lower.ends_with("ignore") {
        return ChangeLane::IgnoreOnly;
    }

    // 4. SourceCode — Rust, build manifests, scripts, tests, shell.
    if lower.ends_with(".rs")
        || basename == "Cargo.toml"
        || basename == "Cargo.lock"
        || lower.ends_with(".sh")
        || p.starts_with("scripts/")
        || p.starts_with("tests/")
    {
        return ChangeLane::SourceCode;
    }

    // 5. ConfigOnly — structured config (Cargo.* already captured above).
    if lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".toml")
        || lower.ends_with(".json")
    {
        return ChangeLane::ConfigOnly;
    }

    // 6. DocsOnly — prose docs.
    if lower.ends_with(".md")
        || lower.ends_with(".txt")
        || basename_lower.starts_with("readme")
        || basename_lower.starts_with("changelog")
        || basename_lower.starts_with("license")
    {
        return ChangeLane::DocsOnly;
    }

    // 7. Fallback — unknown files conservatively trigger the Standard gate.
    ChangeLane::SourceCode
}

/// Classify a change set (repo-relative paths) into a `ChangeClassification`.
///
/// An empty change set is a safe no-op (`IgnoreOnly` / `Minimal`).
pub fn classify_lane(changed_files: &[&str]) -> ChangeClassification {
    let changed: Vec<String> = changed_files
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if changed.is_empty() {
        return ChangeClassification {
            lane: ChangeLane::IgnoreOnly,
            components: vec![ChangeLane::IgnoreOnly],
            profile: VerificationProfile::Minimal,
            changed_files: Vec::new(),
        };
    }

    let mut components: Vec<ChangeLane> = Vec::new();
    for f in &changed {
        let lane = classify_single_file(f);
        if !components.contains(&lane) {
            components.push(lane);
        }
    }

    // Effective lane = component with the highest verification profile (fail-safe).
    let lane = *components
        .iter()
        .max_by_key(|c| c.profile())
        .expect("non-empty change set has at least one component");
    let profile = lane.profile();

    // Deterministic component order: ascending by profile.
    components.sort_by_key(|c| c.profile());

    ChangeClassification {
        lane,
        components,
        profile,
        changed_files: changed,
    }
}

/// Classify the change set produced by a git diff range.
///
/// `range` is supplied by the caller and never defaulted — push callers pass the
/// commit range actually being pushed (e.g. `<a1-head>..HEAD`), verify callers
/// pass `--cached` or a self-chosen range. This avoids the `HEAD~1` pitfall of
/// misjudging a multi-commit push.
pub fn classify_from_git_range(
    repo_root: &std::path::Path,
    range: &str,
) -> Result<ChangeClassification, String> {
    let range = range.trim();
    if range.is_empty() {
        return Err("git range must not be empty".to_string());
    }

    let args: Vec<&str> = if range == "--cached" || range == "--staged" {
        vec!["diff", range, "--name-only"]
    } else {
        vec!["diff", "--name-only", range]
    };

    let (code, stdout, stderr) = crate::run_command(repo_root, "git", &args, &[]);
    if code != 0 {
        return Err(format!(
            "git diff --name-only {} failed (exit {}): {}",
            range,
            code,
            stderr.trim()
        ));
    }

    let files: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    Ok(classify_lane(&refs))
}

// ── Tests ──────────────────────────────────────────────────────────────────────
