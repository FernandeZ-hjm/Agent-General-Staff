//! AGS verification core — structured verification with `CheckItem` model.
//!
//! Provides:
//! - `CheckItem` — stable check model with id, scope, status, severity, evidence
//! - `VerificationReport` — aggregated report with summary and machine-readable JSON
//! - `Scope` — `Local`, `Full`, `Release` verification scopes
//! - `run_verify()` — execute all checks for a given scope
//!
//! # Design
//!
//! Each check is a function that returns a `CheckItem`. The `run_verify()`
//! orchestrator collects items for the requested scope and produces a
//! `VerificationReport` with summary statistics.
//!
//! Checks in `local` scope run entirely within the current repository.
//! `full` is retained as a compatibility alias for `local` in the public
//! edition. `release` adds public manifest, tracked-source leak, and bootstrap
//! payload boundary checks.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Core types ──────────────────────────────────────────────────────────────

/// Verification scope — determines which checks are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Local-only checks: fmt, test, build, fixtures, YAML, preflight.
    Local,
    /// Compatibility alias for local checks in the public edition.
    Full,
    /// Release-focused: public-full sanitized boundary checks.
    Release,
}

impl Scope {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "local" => Ok(Scope::Local),
            "full" => Ok(Scope::Full),
            "release" => Ok(Scope::Release),
            other => Err(format!(
                "invalid scope: '{}'. Expected one of: local, full, release",
                other
            )),
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Local => write!(f, "local"),
            Scope::Full => write!(f, "full"),
            Scope::Release => write!(f, "release"),
        }
    }
}

/// Check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Pass => write!(f, "pass"),
            CheckStatus::Fail => write!(f, "fail"),
            CheckStatus::Skip => write!(f, "skip"),
        }
    }
}

/// Check severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warn => write!(f, "warn"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// A single verification check item — the stable unit of verification evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    /// Stable identifier for this check (e.g. "cargo-fmt", "fixture-valid-full").
    pub id: String,
    /// Which scope(s) this check belongs to.
    pub scope: String,
    /// Pass / fail / skip.
    pub status: CheckStatus,
    /// Info / warn / error.
    pub severity: Severity,
    /// Human-readable evidence summary (command output, parsed result).
    pub evidence: String,
    /// Suggested remediation if the check failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    /// The command that was executed (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Exit code of the executed command (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl CheckItem {
    pub fn pass(id: &str, scope: &str, evidence: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Pass,
            severity: Severity::Info,
            evidence: evidence.to_string(),
            remediation: None,
            command: None,
            exit_code: Some(0),
        }
    }

    pub fn fail(id: &str, scope: &str, evidence: &str, remediation: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Fail,
            severity: Severity::Error,
            evidence: evidence.to_string(),
            remediation: Some(remediation.to_string()),
            command: None,
            exit_code: Some(1),
        }
    }

    pub fn skip(id: &str, scope: &str, reason: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Skip,
            severity: Severity::Info,
            evidence: reason.to_string(),
            remediation: None,
            command: None,
            exit_code: None,
        }
    }

    pub fn warn(id: &str, scope: &str, evidence: &str, remediation: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Fail,
            severity: Severity::Warn,
            evidence: evidence.to_string(),
            remediation: Some(remediation.to_string()),
            command: None,
            exit_code: Some(0),
        }
    }
}

/// Aggregated verification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: String,
    pub scope: Scope,
    pub repo_root: String,
    pub items: Vec<CheckItem>,
    pub summary: VerificationSummary,
}

/// Summary statistics for a verification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl VerificationReport {
    /// Whether all blocking checks passed. Advisory WARN items do not fail the report.
    pub fn passed(&self) -> bool {
        self.summary.errors == 0
    }

    /// Exit code: 0 if all blocking checks passed, 1 if any ERROR failed.
    pub fn exit_code(&self) -> i32 {
        if self.passed() {
            0
        } else {
            1
        }
    }
}

// ── Check execution helpers ─────────────────────────────────────────────────

/// Run a shell command and return (exit_code, stdout, stderr).
fn run_command(
    repo_root: &Path,
    program: &str,
    args: &[&str],
    env_vars: &[(&str, &str)],
) -> (i32, String, String) {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.current_dir(repo_root);
    for (key, value) in env_vars {
        cmd.env(key, value);
    }
    // Suppress cargo's progress output for cleaner evidence
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    match cmd.output() {
        Ok(output) => {
            let code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (code, stdout, stderr)
        }
        Err(e) => (-1, String::new(), format!("Failed to execute: {}", e)),
    }
}

/// Count the longest consecutive run of hex characters in a string.
#[cfg(test)]
fn longest_hex_run(s: &str) -> usize {
    let mut max_run = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch.is_ascii_hexdigit() {
            current += 1;
            if current > max_run {
                max_run = current;
            }
        } else {
            current = 0;
        }
    }
    max_run
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
/// Uses char boundaries to avoid splitting multi-byte UTF-8 characters.
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

// ── Individual checks ───────────────────────────────────────────────────────

fn check_cargo_fmt(repo_root: &Path) -> CheckItem {
    let (code, _stdout, stderr) = run_command(repo_root, "cargo", &["fmt", "--check"], &[]);
    if code == 0 {
        CheckItem::pass("cargo-fmt", "local", "cargo fmt --check passed")
    } else {
        let evidence = if stderr.is_empty() {
            format!("cargo fmt --check failed (exit {})", code)
        } else {
            truncate(&stderr, 500)
        };
        CheckItem::fail(
            "cargo-fmt",
            "local",
            &evidence,
            "Run `cargo fmt` to fix formatting.",
        )
        .with_command("cargo fmt --check")
        .with_exit_code(code)
    }
}

fn check_cargo_test(repo_root: &Path) -> CheckItem {
    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &["test"],
        &[("RUSTFLAGS", "-D warnings")],
    );
    if code == 0 {
        // Extract test summary from stdout for evidence
        let summary = stdout
            .lines()
            .filter(|l| l.contains("test result:"))
            .collect::<Vec<_>>()
            .join("\n");
        let evidence = if summary.is_empty() {
            "cargo test passed (warnings as errors)".to_string()
        } else {
            format!(
                "cargo test passed (warnings as errors)\n{}",
                truncate(&summary, 400)
            )
        };
        CheckItem::pass("cargo-test", "local", &evidence)
    } else {
        let combined = format!(
            "stdout:\n{}\nstderr:\n{}",
            truncate(&stdout, 300),
            truncate(&stderr, 300)
        );
        CheckItem::fail(
            "cargo-test",
            "local",
            &combined,
            "Run `RUSTFLAGS=\"-D warnings\" cargo test` to see full output.",
        )
        .with_command("RUSTFLAGS=\"-D warnings\" cargo test")
        .with_exit_code(code)
    }
}

fn check_cargo_build(repo_root: &Path) -> CheckItem {
    let (code, _stdout, stderr) = run_command(repo_root, "cargo", &["build", "--release"], &[]);
    if code == 0 {
        CheckItem::pass(
            "cargo-build-release",
            "local",
            "cargo build --release passed",
        )
    } else {
        CheckItem::fail(
            "cargo-build-release",
            "local",
            &truncate(&stderr, 500),
            "Run `cargo build --release` to see full compiler errors.",
        )
        .with_command("cargo build --release")
        .with_exit_code(code)
    }
}

fn check_valid_fixtures(repo_root: &Path) -> Vec<CheckItem> {
    let fixtures = ["tests/fixtures/valid-full.md"];
    let mut items = Vec::new();

    for fixture in &fixtures {
        let fixture_path = repo_root.join(fixture);
        if !fixture_path.exists() {
            items.push(CheckItem::skip(
                &format!("fixture-{}", fixture.replace('/', "-").replace('.', "-")),
                "local",
                &format!("Fixture not found: {}", fixture),
            ));
            continue;
        }

        let (code, stdout, stderr) = run_command(
            repo_root,
            "cargo",
            &[
                "run",
                "-q",
                "-p",
                "ags-cli",
                "--",
                "task",
                "validate",
                &fixture_path.to_string_lossy(),
            ],
            &[],
        );

        let id = format!(
            "fixture-{}",
            fixture
                .replace("tests/fixtures/", "")
                .replace('/', "-")
                .replace('.', "-")
                .replace("_", "-")
        );
        if code == 0 {
            items.push(CheckItem::pass(
                &id,
                "local",
                &format!("Fixture {} is valid", fixture),
            ));
        } else {
            let evidence = format!(
                "Fixture {} validation failed (exit {}): {}",
                fixture,
                code,
                truncate(&format!("{}\n{}", stdout, stderr), 400)
            );
            items.push(
                CheckItem::fail(
                    &id,
                    "local",
                    &evidence,
                    &format!("Review fixture {} for schema compliance.", fixture),
                )
                .with_command(&format!(
                    "cargo run -p ags-cli -- task validate {}",
                    fixture
                ))
                .with_exit_code(code),
            );
        }
    }

    items
}

fn check_governance_yaml(repo_root: &Path) -> Vec<CheckItem> {
    let yaml_files = [
        "governance/skill-adoption-log.yaml",
        "governance/skill-ignore-list.yaml",
        "governance/mcp-adoption-log.yaml",
        "manifests/suite.yaml",
        "manifests/mcp-registry.yaml",
    ];
    let mut items = Vec::new();

    for yaml_file in &yaml_files {
        let path = repo_root.join(yaml_file);
        if !path.exists() {
            items.push(CheckItem::skip(
                &format!("yaml-{}", yaml_file.replace('/', "-").replace('.', "-")),
                "local",
                &format!("YAML file not found: {}", yaml_file),
            ));
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                items.push(CheckItem::fail(
                    &format!("yaml-{}", yaml_file.replace('/', "-").replace('.', "-")),
                    "local",
                    &format!("Cannot read {}: {}", yaml_file, e),
                    "Check file permissions.",
                ));
                continue;
            }
        };

        let id = format!("yaml-{}", yaml_file.replace('/', "-").replace('.', "-"));

        match serde_yaml::from_str::<serde_yaml::Value>(&content) {
            Ok(_) => {
                items.push(CheckItem::pass(
                    &id,
                    "local",
                    &format!("{} is valid YAML", yaml_file),
                ));
            }
            Err(e) => {
                items.push(CheckItem::fail(
                    &id,
                    "local",
                    &format!("{} YAML parse error: {}", yaml_file, e),
                    &format!("Fix YAML syntax in {}.", yaml_file),
                ));
            }
        }
    }

    items
}

fn check_session_preflight(repo_root: &Path) -> CheckItem {
    // Run `ags session preflight` for smoke verification.
    // Use cargo run since ags may not be on PATH during development.
    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "session",
            "preflight",
            "--for",
            "claude-code",
            "--format",
            "json",
            "--target",
            &repo_root.to_string_lossy(),
        ],
        &[],
    );

    if code == 0 {
        // Verify the JSON output is parseable
        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(json) => {
                let status = json
                    .get("overall_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                CheckItem::pass(
                    "session-preflight",
                    "local",
                    &format!("session preflight OK (status={})", status),
                )
            }
            Err(e) => CheckItem::fail(
                "session-preflight",
                "local",
                &format!("session preflight produced invalid JSON: {}", e),
                "Check ags session preflight output for errors.",
            )
            .with_command(&format!(
                "ags session preflight --for claude-code --format json --target {}",
                repo_root.display()
            ))
            .with_exit_code(1),
        }
    } else {
        let combined = format!("{}\n{}", truncate(&stdout, 300), truncate(&stderr, 300));
        let remediation = format!(
            "Run `ags session preflight --for claude-code --format json --target {}` to diagnose.",
            repo_root.display()
        );
        CheckItem::fail(
            "session-preflight",
            "local",
            &format!("session preflight failed (exit {}): {}", code, combined),
            &remediation,
        )
        .with_command(&format!(
            "ags session preflight --for claude-code --format json --target {}",
            repo_root.display()
        ))
        .with_exit_code(code)
    }
}

fn check_release_boundary(repo_root: &Path) -> Vec<CheckItem> {
    let mut items = Vec::new();

    // Check 1: Public release manifest — verify the current repo against the
    // public manifest itself (required files present, no forbidden payload). This
    // is a self-contained check on the public tree, not a source↔target sync
    // comparison.
    let manifest = workflow_sync_check::manifest::verify_release_manifest(repo_root);
    if manifest.passed {
        items.push(CheckItem::pass(
            "release-manifest",
            "release",
            "Public release manifest satisfied — required files present, no forbidden payload.",
        ));
    } else {
        let mut parts = Vec::new();
        if !manifest.required_missing.is_empty() {
            parts.push(format!(
                "missing required: {}",
                manifest.required_missing.join(", ")
            ));
        }
        if !manifest.forbidden_found.is_empty() {
            parts.push(format!(
                "forbidden payload present: {}",
                manifest.forbidden_found.join(", ")
            ));
        }
        items.push(CheckItem::fail(
            "release-manifest",
            "release",
            &format!("Public release manifest violation: {}", parts.join("; ")),
            "Add missing required files or remove forbidden payload before release.",
        ));
    }

    // Check 2: Tracked-source leak scan — no maintainer-private paths or runtime
    // markers in git-tracked files.
    items.push(check_tracked_source_leaks(repo_root));

    // Check 3: Verify bootstrap --apply produces a sanitized public payload
    let tmpdir = std::env::temp_dir().join(format!("ags-verify-release-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmpdir);

    let (bootstrap_code, _bs_stdout, bs_stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "bootstrap",
            "--apply",
            "--target",
            &tmpdir.to_string_lossy(),
        ],
        &[],
    );

    if bootstrap_code == 0 {
        // Check that generated build output and private runtime state are NOT in the payload.
        let forbidden = [
            "target",
            "ags",
            "ags.exe",
            "global-skills",
            "skill-packs",
            ".agents",
            ".codex",
            "task-archive",
        ];
        let mut leaked = Vec::new();
        for item in &forbidden {
            if tmpdir.join(item).exists() {
                leaked.push(*item);
            }
        }
        if leaked.is_empty() {
            items.push(CheckItem::pass(
                "release-forbidden-payload",
                "release",
                "No build output, preinstalled skill packs, or private runtime state leaked into bootstrap payload.",
            ));
        } else {
            items.push(CheckItem::fail(
                "release-forbidden-payload",
                "release",
                &format!(
                    "Forbidden public-full sanitized payload leaked into bootstrap: {}",
                    leaked.join(", ")
                ),
                "Check bootstrap --apply payload allowlist.",
            ));
        }
    } else {
        items.push(CheckItem::fail(
            "release-bootstrap-apply",
            "release",
            &format!(
                "bootstrap --apply failed (exit {}): {}",
                bootstrap_code,
                truncate(&bs_stderr, 300)
            ),
            "Fix bootstrap --apply before release.",
        ));
    }

    // Cleanup tempdir
    let _ = std::fs::remove_dir_all(&tmpdir);

    items
}

/// Scan every git-tracked file for maintainer-private paths or runtime markers
/// that must not appear in the public edition. The private markers are assembled
/// at runtime with `concat!` so this scanner's own source never contains the full
/// literals it searches for (same convention as the `receipt` crate), which
/// prevents the scan from flagging itself.
fn check_tracked_source_leaks(repo_root: &Path) -> CheckItem {
    let markers: [String; 8] = [
        concat!("agent-governance-suite", "-stable").to_string(),
        concat!("agent-governance-suite", "-private").to_string(),
        concat!("/Users/", "hujiaming").to_string(),
        concat!("EVOLVER", "_PROXY_MCP").to_string(),
        concat!("evolver", "-token").to_string(),
        concat!("with", "-evomap").to_string(),
        concat!("gep", "-mcp-server").to_string(),
        concat!("@", "evomap").to_string(),
    ];

    let (code, stdout, _stderr) = run_command(repo_root, "git", &["ls-files"], &[]);
    if code != 0 {
        return CheckItem::skip(
            "release-tracked-leak",
            "release",
            "git ls-files unavailable (not a git worktree?) — tracked-source leak scan skipped.",
        );
    }

    let mut leaks: Vec<String> = Vec::new();
    for file in stdout.lines() {
        let file = file.trim();
        if file.is_empty() {
            continue;
        }
        // A maintainer-private sync tool tracked in the public edition is itself a leak.
        if file == "scripts/sync-public.sh" || file.ends_with("/sync-public.sh") {
            leaks.push(format!(
                "{file}: maintainer-private sync tool must not be tracked in the public edition"
            ));
        }
        let content = match std::fs::read_to_string(repo_root.join(file)) {
            Ok(c) => c,
            Err(_) => continue, // binary or unreadable — skip
        };
        for (idx, line) in content.lines().enumerate() {
            for marker in &markers {
                if line.contains(marker.as_str()) {
                    leaks.push(format!("{file}:{}: private marker `{marker}`", idx + 1));
                }
            }
        }
    }

    if leaks.is_empty() {
        CheckItem::pass(
            "release-tracked-leak",
            "release",
            "No maintainer-private paths or runtime markers found in git-tracked files.",
        )
    } else {
        let shown = leaks.len().min(20);
        CheckItem::fail(
            "release-tracked-leak",
            "release",
            &format!(
                "Maintainer-private leak in tracked source ({} hit(s)): {}",
                leaks.len(),
                leaks[..shown].join("; ")
            ),
            "Remove the private path/marker, or move the file out of the public edition (git rm --cached + .gitignore).",
        )
    }
}

/// Detect patterns in template content that indicate REAL leaked secrets or
/// absolute private paths. Documentation words like "token" are fine; actual
/// 64+ char hex tokens, `/Users/` paths, and real memory/archive paths are NOT.
#[cfg(test)]
fn detect_template_leaks(content: &str, rel_path: &str) -> Vec<String> {
    let mut leaks = Vec::new();

    // Check for absolute /Users/<username> paths that look like real home
    // directory paths (not just documentation examples). We specifically
    // check for /Users/ followed by a plausible username (alphanumeric,
    // at least 2 chars) to avoid flagging grep/find example commands.
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Check for /Users/<name>/ paths — indicates real machine-specific leakage.
        // Comments are NOT skipped: a real home path pasted into a comment is still
        // a leak. Shell-command examples are scanned too; grep patterns like
        // `/Users/|sk-...` are not flagged because they do not contain a plausible
        // username segment.
        if let Some(rest) = trimmed.find("/Users/") {
            let after_users = &trimmed[rest + 7..]; // skip "/Users/"
                                                    // Only flag if followed by a plausible username (not just a regex pattern)
            let maybe_user = after_users.split('/').next().unwrap_or("");
            // Real usernames: at least 2 chars, alphanumeric or dash/underscore
            if maybe_user.len() >= 2
                && maybe_user
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                leaks.push(format!(
                    "{rel_path}:{line_num}: potential absolute /Users/ path leak: {trimmed}",
                    line_num = i + 1
                ));
            }
        }
    }

    // Check for long hex strings that look like real tokens (64+ hex chars).
    // Comments are NOT skipped — a real token pasted into a comment is still a leak.
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Skip REPLACE lines and sha256 documentation examples
        if trimmed.starts_with('"') && (trimmed.contains("REPLACE") || trimmed.contains("sha256")) {
            continue;
        }
        let hex_run = longest_hex_run(trimmed);
        if hex_run >= 64 {
            leaks.push(format!(
                "{rel_path}:{line_num}: potential hex token leak ({hex_run} hex chars)",
                line_num = i + 1
            ));
        }
    }

    // Check for real task archive or memory capsule paths.
    // Comments are NOT skipped — these paths shouldn't appear anywhere.
    for pat in &[".agents/memory/projects/", "task-archive/"] {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(pat) {
                leaks.push(format!(
                    "{rel_path}:{line_num}: potential memory/archive path leak: {pat}",
                    line_num = i + 1
                ));
            }
        }
    }

    leaks
}

impl CheckItem {
    fn with_command(mut self, cmd: &str) -> Self {
        self.command = Some(cmd.to_string());
        self
    }

    fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }
}

// ── Orchestrator ────────────────────────────────────────────────────────────

/// Run all verification checks for the given scope and return a report.
pub fn run_verify(scope: Scope, repo_root: &Path) -> VerificationReport {
    let repo_root = canonical_repo_root(repo_root);
    let mut items: Vec<CheckItem> = Vec::new();

    // Local checks — always run
    items.push(check_cargo_fmt(&repo_root));
    items.push(check_cargo_test(&repo_root));
    items.push(check_cargo_build(&repo_root));
    items.extend(check_valid_fixtures(&repo_root));
    items.extend(check_governance_yaml(&repo_root));
    items.push(check_session_preflight(&repo_root));

    // `full` is retained as a compatibility alias for `local`. The private↔stable
    // and private↔public drift gates are maintainer-only and are not part of the
    // public edition.

    // Release scope — current-repo self-checks (public manifest + tracked leak scan)
    if matches!(scope, Scope::Release) {
        items.extend(check_release_boundary(&repo_root));
    }

    // Build summary
    let total = items.len();
    let passed = items
        .iter()
        .filter(|i| i.status == CheckStatus::Pass)
        .count();
    let failed = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail)
        .count();
    let skipped = items
        .iter()
        .filter(|i| i.status == CheckStatus::Skip)
        .count();
    let errors = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail && i.severity == Severity::Error)
        .count();
    let warnings = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail && i.severity == Severity::Warn)
        .count();

    VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope,
        repo_root: repo_root.to_string_lossy().to_string(),
        items,
        summary: VerificationSummary {
            total,
            passed,
            failed,
            skipped,
            errors,
            warnings,
        },
    }
}

fn canonical_repo_root(repo_root: &Path) -> PathBuf {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
}

// ── Renderers ───────────────────────────────────────────────────────────────

/// Render a verification report as human-readable text.
pub fn render_text(report: &VerificationReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("AGS Verification Report — scope: {}", report.scope));
    lines.push(format!("Repo: {}", report.repo_root));
    lines.push(String::new());

    // Sort items: failures first, then passes, then skips
    let mut sorted = report.items.clone();
    sorted.sort_by_key(|i| {
        (
            match i.status {
                CheckStatus::Fail => 0u8,
                CheckStatus::Pass => 1,
                CheckStatus::Skip => 2,
            },
            match i.severity {
                Severity::Error => 0u8,
                Severity::Warn => 1,
                Severity::Info => 2,
            },
        )
    });

    for item in &sorted {
        let status_icon = match item.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Fail => match item.severity {
                Severity::Error => "FAIL",
                Severity::Warn => "WARN",
                Severity::Info => "FAIL",
            },
            CheckStatus::Skip => "SKIP",
        };

        lines.push(format!(
            "[{}] {} — {}",
            status_icon,
            item.id,
            item.evidence.lines().next().unwrap_or("")
        ));

        if item.status == CheckStatus::Fail {
            if let Some(ref rem) = item.remediation {
                lines.push(format!("      remediation: {}", rem));
            }
            if let Some(ref cmd) = item.command {
                lines.push(format!("      command: {}", cmd));
            }
        }

        // For multi-line evidence, show remaining lines indented
        let evidence_lines: Vec<&str> = item.evidence.lines().collect();
        if evidence_lines.len() > 1 {
            for line in &evidence_lines[1..] {
                if !line.is_empty() {
                    lines.push(format!("      {}", line));
                }
            }
        }
    }

    lines.push(String::new());
    lines.push("─".repeat(50));
    lines.push(format!(
        "Summary: {} total, {} passed, {} failed ({} errors, {} warnings), {} skipped",
        report.summary.total,
        report.summary.passed,
        report.summary.failed,
        report.summary.errors,
        report.summary.warnings,
        report.summary.skipped,
    ));

    if report.passed() {
        lines.push("Verdict: PASS".to_string());
    } else {
        lines.push("Verdict: FAIL".to_string());
    }

    lines.join("\n")
}

/// Render a verification report as JSON.
pub fn render_json(report: &VerificationReport) -> String {
    serde_json::to_string_pretty(report)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_from_str() {
        assert_eq!(Scope::from_str("local").unwrap(), Scope::Local);
        assert_eq!(Scope::from_str("full").unwrap(), Scope::Full);
        assert_eq!(Scope::from_str("release").unwrap(), Scope::Release);
        assert!(Scope::from_str("invalid").is_err());
    }

    #[test]
    fn test_scope_display() {
        assert_eq!(Scope::Local.to_string(), "local");
        assert_eq!(Scope::Full.to_string(), "full");
        assert_eq!(Scope::Release.to_string(), "release");
    }

    #[test]
    fn test_check_item_pass() {
        let item = CheckItem::pass("test-check", "local", "all good");
        assert_eq!(item.status, CheckStatus::Pass);
        assert_eq!(item.severity, Severity::Info);
        assert_eq!(item.exit_code, Some(0));
    }

    #[test]
    fn test_check_item_fail() {
        let item = CheckItem::fail("test-check", "local", "broken", "fix it");
        assert_eq!(item.status, CheckStatus::Fail);
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.remediation, Some("fix it".to_string()));
    }

    #[test]
    fn test_check_item_skip() {
        let item = CheckItem::skip("test-check", "local", "not available");
        assert_eq!(item.status, CheckStatus::Skip);
        assert_eq!(item.exit_code, None);
    }

    #[test]
    fn test_check_item_warn() {
        let item = CheckItem::warn("test-check", "local", "advisory", "review");
        assert_eq!(item.status, CheckStatus::Fail);
        assert_eq!(item.severity, Severity::Warn);
    }

    #[test]
    fn test_check_item_builder() {
        let item = CheckItem::pass("test", "local", "ok")
            .with_command("echo hi")
            .with_exit_code(0);
        assert_eq!(item.command, Some("echo hi".to_string()));
        assert_eq!(item.exit_code, Some(0));
    }

    #[test]
    fn test_empty_report_passes() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Local,
            repo_root: "/tmp".to_string(),
            items: vec![],
            summary: VerificationSummary {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                errors: 0,
                warnings: 0,
            },
        };
        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn test_report_with_failures() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Local,
            repo_root: "/tmp".to_string(),
            items: vec![
                CheckItem::pass("a", "local", "ok"),
                CheckItem::fail("b", "local", "broken", "fix"),
            ],
            summary: VerificationSummary {
                total: 2,
                passed: 1,
                failed: 1,
                skipped: 0,
                errors: 1,
                warnings: 0,
            },
        };
        assert!(!report.passed());
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn test_report_with_only_warnings_passes() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Full,
            repo_root: "/tmp".to_string(),
            items: vec![
                CheckItem::pass("a", "local", "ok"),
                CheckItem::warn("b", "full", "advisory", "review"),
            ],
            summary: VerificationSummary {
                total: 2,
                passed: 1,
                failed: 1,
                skipped: 0,
                errors: 0,
                warnings: 1,
            },
        };
        assert!(report.passed());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn test_render_json_produces_valid_json() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Local,
            repo_root: "/tmp/test".to_string(),
            items: vec![CheckItem::pass("t1", "local", "ok")],
            summary: VerificationSummary {
                total: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
                errors: 0,
                warnings: 0,
            },
        };
        let json = render_json(&report);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], "2.0-verify");
        assert_eq!(parsed["scope"], "local");
        assert_eq!(parsed["summary"]["total"], 1);
        assert_eq!(parsed["summary"]["passed"], 1);
    }

    #[test]
    fn test_render_text_contains_summary() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Local,
            repo_root: "/tmp/test".to_string(),
            items: vec![
                CheckItem::pass("t1", "local", "check passed"),
                CheckItem::fail("t2", "local", "check failed", "run fix"),
                CheckItem::skip("t3", "local", "not available"),
            ],
            summary: VerificationSummary {
                total: 3,
                passed: 1,
                failed: 1,
                skipped: 1,
                errors: 1,
                warnings: 0,
            },
        };
        let text = render_text(&report);
        assert!(text.contains("PASS"));
        assert!(text.contains("FAIL"));
        assert!(text.contains("SKIP"));
        assert!(text.contains("Summary:"));
        assert!(text.contains("Verdict: FAIL"));
    }

    #[test]
    fn test_governance_yaml_parse_valid() {
        // Test with inline valid YAML
        let valid_yaml = "key: value\nitems:\n  - a\n  - b\n";
        let result = serde_yaml::from_str::<serde_yaml::Value>(valid_yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_governance_yaml_parse_invalid() {
        let invalid_yaml = "key: value\n\t- tab indent\n";
        let result = serde_yaml::from_str::<serde_yaml::Value>(invalid_yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_check_status_display() {
        assert_eq!(CheckStatus::Pass.to_string(), "pass");
        assert_eq!(CheckStatus::Fail.to_string(), "fail");
        assert_eq!(CheckStatus::Skip.to_string(), "skip");
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn test_json_roundtrip_checkitem() {
        let item = CheckItem {
            id: "test".to_string(),
            scope: "local".to_string(),
            status: CheckStatus::Pass,
            severity: Severity::Info,
            evidence: "ok".to_string(),
            remediation: None,
            command: Some("cmd".to_string()),
            exit_code: Some(0),
        };
        let json = serde_json::to_string(&item).unwrap();
        let parsed: CheckItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test");
        assert_eq!(parsed.status, CheckStatus::Pass);
    }

    #[test]
    fn test_json_roundtrip_report() {
        let report = VerificationReport {
            schema_version: "2.0-verify".to_string(),
            scope: Scope::Full,
            repo_root: "/test".to_string(),
            items: vec![
                CheckItem::pass("a", "local", "ok"),
                CheckItem::fail("b", "full", "bad", "fix"),
                CheckItem::warn("c", "full", "advisory", "review"),
            ],
            summary: VerificationSummary {
                total: 3,
                passed: 1,
                failed: 2,
                skipped: 0,
                errors: 1,
                warnings: 1,
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: VerificationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.schema_version, "2.0-verify");
        assert_eq!(parsed.scope, Scope::Full);
        assert_eq!(parsed.items.len(), 3);
        assert!(!parsed.passed());
    }

    #[test]
    fn test_run_command_executes_in_repo_root() {
        let root = std::env::temp_dir().join(format!("ags-verify-cwd-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("cwd-marker.txt");

        #[cfg(windows)]
        let (program, args): (&str, &[&str]) = ("cmd", &["/C", "echo ok>cwd-marker.txt"]);
        #[cfg(not(windows))]
        let (program, args): (&str, &[&str]) = ("sh", &["-c", "printf ok > cwd-marker.txt"]);

        let (code, _stdout, stderr) = run_command(&root, program, args, &[]);
        let marker_content = std::fs::read_to_string(&marker).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(code, 0, "stderr={stderr}");
        assert_eq!(marker_content.trim(), "ok");
    }

    #[test]
    fn test_session_preflight_failure_records_explicit_target() {
        let root = std::env::temp_dir().join(format!(
            "ags-verify-preflight-target-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();

        let item = check_session_preflight(&root);
        let _ = std::fs::remove_dir_all(&root);

        let command = item.command.unwrap_or_default();
        assert!(
            command.contains("--target"),
            "preflight command must carry explicit --target: {command}"
        );
        assert!(
            command.contains(&root.to_string_lossy().to_string()),
            "preflight command must use the repo root target: {command}"
        );
        assert!(
            item.remediation.unwrap_or_default().contains("--target"),
            "preflight remediation must preserve target authority"
        );
    }

    // ── Template leak detection tests ──────────────────────────────────

    #[test]
    fn test_longest_hex_run() {
        assert_eq!(longest_hex_run(""), 0);
        assert_eq!(longest_hex_run("xyzzy"), 0);
        assert_eq!(longest_hex_run("abc123"), 6);
        assert_eq!(longest_hex_run("abc xyz 123"), 3);
        assert_eq!(longest_hex_run(&"a".repeat(65)), 65);
    }

    #[test]
    fn template_leak_detection_flags_real_user_path() {
        let content = "proxy_url: \"/Users/example/.evolver/settings.json\"";
        let leaks = detect_template_leaks(content, "test.yaml");
        assert!(!leaks.is_empty(), "should detect /Users/example path leak");
    }

    #[test]
    fn template_leak_detection_ignores_grep_command() {
        let content = "grep -E '/Users/' templates/ -r  # check for leaks";
        let leaks = detect_template_leaks(content, "test.md");
        assert!(leaks.is_empty(), "should ignore grep commands with /Users/");
    }

    #[test]
    fn template_leak_detection_flags_node_command_with_real_user_path() {
        let content = "node /Users/example/.evolver/run-hook.js";
        let leaks = detect_template_leaks(content, "test.md");
        assert!(
            !leaks.is_empty(),
            "should detect /Users/ path in node command"
        );
    }

    #[test]
    fn template_leak_detection_flags_python_command_with_real_user_path() {
        let content = "python3 /Users/example/scripts/evolver.py";
        let leaks = detect_template_leaks(content, "test.md");
        assert!(
            !leaks.is_empty(),
            "should detect /Users/ path in python command"
        );
    }

    #[test]
    fn template_leak_detection_flags_comments_with_paths() {
        // Real /Users/<name> paths in comments ARE now detected.
        let content = "# /Users/example/.evolver/settings.json";
        let leaks = detect_template_leaks(content, "test.yaml");
        assert!(
            !leaks.is_empty(),
            "should detect /Users/ path even in comments"
        );
    }

    #[test]
    fn template_leak_detection_accepts_safe_comment() {
        // Comments without real paths or tokens are fine.
        let content = "# This template uses a token file for authentication.";
        let leaks = detect_template_leaks(content, "test.yaml");
        assert!(leaks.is_empty(), "safe comments should pass");
    }

    #[test]
    fn template_leak_detection_ignores_replace_slots() {
        let content = "\"REPLACE: path/to/advisory-recall-script\"";
        let leaks = detect_template_leaks(content, "test.json");
        assert!(leaks.is_empty(), "should ignore REPLACE slot lines");
    }

    #[test]
    fn template_leak_detection_flags_long_hex_token() {
        let hex64 = "a".repeat(64);
        let content = format!("token: \"{hex64}\"");
        let leaks = detect_template_leaks(&content, "test.yaml");
        assert!(!leaks.is_empty(), "should detect 64-char hex token");
    }

    #[test]
    fn template_leak_detection_ignores_short_hex() {
        let content = "hash: \"abc123def456\""; // short hex, not a token
        let leaks = detect_template_leaks(content, "test.yaml");
        assert!(leaks.is_empty(), "should not flag short hex strings");
    }
}
