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
//! `full` adds drift checks against stable and public targets.
//! `release` focuses on public-full sanitized boundary checks.

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
    /// Local + drift checks against stable and public targets.
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
    /// Stable identifier for this check (e.g. "cargo-fmt", "fixture-valid-compact").
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
            exit_code: Some(1),
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
    /// Whether all non-skipped checks passed (no failures).
    pub fn passed(&self) -> bool {
        self.summary.failed == 0
    }

    /// Exit code: 0 if all passed, 1 if any failed.
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
    let fixtures = [
        "tests/fixtures/valid-compact.md",
        "tests/fixtures/valid-full.md",
    ];
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
        "manifests/suite.yaml",
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

fn check_private_vs_stable_drift(repo_root: &Path) -> CheckItem {
    if is_public_distributable_workspace(repo_root) {
        return CheckItem::skip(
            "drift-private-vs-stable",
            "full",
            "Current repository is a public-full sanitized workspace; private/stable drift checks must be run from the private or stable source workspace.",
        );
    }

    // In public version, stable target must be configured via env var
    let stable_root = std::env::var("AGS_STABLE_ROOT").unwrap_or_default();
    if stable_root.is_empty() || !Path::new(&stable_root).exists() {
        return CheckItem::skip(
            "drift-private-vs-stable",
            "full",
            "Stable root not configured. Set AGS_STABLE_ROOT env var to enable drift checks against a stable target.",
        );
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
            "sync",
            "check",
            "--source",
            &repo_root.to_string_lossy(),
            "--target",
            &stable_root,
            "--target-name",
            "stable",
            "--format",
            "json",
        ],
        &[],
    );

    let output = format!("{}\n{}", stdout, stderr);
    if code == 0 {
        CheckItem::pass(
            "drift-private-vs-stable",
            "full",
            "No protocol drift detected between source and stable.",
        )
    } else {
        let evidence = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            let drift_count = json
                .get("projects")
                .and_then(|p| p.get(0))
                .and_then(|p| p.get("drift_count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!(
                "Protocol drift detected: {} drift item(s) between source and stable.",
                drift_count
            )
        } else {
            format!(
                "Drift check failed (exit {}): {}",
                code,
                truncate(&output, 400)
            )
        };

        CheckItem::warn(
            "drift-private-vs-stable",
            "full",
            &evidence,
            "Review and sync protocol files.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name stable",
            repo_root.display(),
            stable_root
        ))
        .with_exit_code(code)
    }
}

fn is_public_distributable_workspace(repo_root: &Path) -> bool {
    for rel in ["WORKSPACE.md", "CLAUDE.md", "README.md"] {
        let Ok(content) = std::fs::read_to_string(repo_root.join(rel)) else {
            continue;
        };
        if content.contains("public distributable edition")
            || content.contains("Public Edition")
            || content.contains("public-full sanitized")
        {
            return true;
        }
    }
    false
}

fn check_private_vs_public_boundary(repo_root: &Path) -> CheckItem {
    // In public version, public target must be configured via env var
    let public_root = std::env::var("AGS_PUBLIC_ROOT").unwrap_or_default();
    if public_root.is_empty() || !Path::new(&public_root).exists() {
        return CheckItem::skip(
            "drift-private-vs-public",
            "full",
            "Public root not configured. Set AGS_PUBLIC_ROOT env var to enable boundary checks.",
        );
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
            "sync",
            "check",
            "--source",
            &repo_root.to_string_lossy(),
            "--target",
            &public_root,
            "--target-name",
            "public-full-sanitized",
            "--format",
            "json",
        ],
        &[],
    );

    let output = format!("{}\n{}", stdout, stderr);

    // Check for hard boundary violations first
    let has_violation = output.contains("INVARIANT_MISSING")
        || output.contains("INVARIANT_CONTRADICTED")
        || output.contains("PUBLIC_FORBIDDEN_PAYLOAD");

    if code == 0 {
        CheckItem::pass(
            "drift-private-vs-public",
            "full",
            "No public-full sanitized boundary violations detected.",
        )
    } else if has_violation {
        CheckItem::fail(
            "drift-private-vs-public",
            "full",
            &format!(
                "Public-full sanitized boundary violation detected (exit {}): {}",
                code,
                truncate(&output, 500)
            ),
            "Review public-full sanitized boundary: INVARIANT or PUBLIC_FORBIDDEN_PAYLOAD violation.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name public-full-sanitized",
            repo_root.display(),
            public_root
        ))
        .with_exit_code(code)
    } else {
        // Allowlist gap — warn but don't hard-fail
        CheckItem::warn(
            "drift-private-vs-public",
            "full",
            &format!(
                "Public-full sanitized allowlist gap (exit {}): content drift within PUBLIC_MANIFEST files.",
                code
            ),
            "Review public promotion allowlist and update public manifest.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name public-full-sanitized",
            repo_root.display(),
            public_root
        ))
        .with_exit_code(code)
    }
}

fn check_public_full_sanitized_tracked(repo_root: &Path) -> CheckItem {
    let (code, stdout, stderr) = run_command(repo_root, "git", &["ls-files"], &[]);
    if code != 0 {
        return CheckItem::warn(
            "release-sanitize-tracked",
            "release",
            &format!(
                "git ls-files unavailable (exit {}): {}",
                code,
                truncate(&stderr, 200)
            ),
            "Run release sanitize checks from a git worktree.",
        );
    }

    let forbidden_paths = [
        "target/",
        "global-skills/",
        "skill-packs/",
        ".agents/",
        ".codex/",
        ".claude/local/",
    ];
    let mut private_patterns = vec![
        concat!("PRIVATE_", "DO_NOT_SHIP").to_string(),
        concat!("LOCAL_ONLY_", "DO_NOT_SHIP").to_string(),
        concat!("SECRET_", "DO_NOT_SHIP").to_string(),
        concat!("INTERNAL_ONLY_", "DO_NOT_SHIP").to_string(),
    ];
    if let Ok(extra_patterns) = std::env::var("AGS_RELEASE_PRIVATE_PATTERNS") {
        private_patterns.extend(
            extra_patterns
                .split(';')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
        );
    }

    let mut violations: Vec<String> = Vec::new();
    for rel in stdout.lines().filter(|l| !l.trim().is_empty()) {
        if forbidden_paths
            .iter()
            .any(|prefix| rel == prefix.trim_end_matches('/') || rel.starts_with(prefix))
        {
            violations.push(format!("forbidden tracked path: {}", rel));
            continue;
        }

        let path = repo_root.join(rel);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for pattern in &private_patterns {
            if content.contains(pattern) {
                violations.push(format!("private pattern `{}` in {}", pattern, rel));
            }
        }

        if rel == "governance/skill-adoption-log.yaml" || rel == "governance/skill-ignore-list.yaml"
        {
            if !content.contains("entries: []") {
                violations.push(format!("public audit skeleton is not empty: {}", rel));
            }
        }
    }

    if violations.is_empty() {
        CheckItem::pass(
            "release-sanitize-tracked",
            "release",
            "Tracked public-full files are sanitized: no private identity markers, preinstalled skill packs, or non-empty audit logs found.",
        )
    } else {
        CheckItem::fail(
            "release-sanitize-tracked",
            "release",
            &format!(
                "Sanitize violations: {}",
                truncate(&violations.join("; "), 800)
            ),
            "Remove private paths/data and ship only empty public templates or skeleton logs.",
        )
    }
}

fn check_release_boundary(repo_root: &Path) -> Vec<CheckItem> {
    // In public version, public target must be configured via env var
    let public_root = std::env::var("AGS_PUBLIC_ROOT").unwrap_or_default();
    let mut items = Vec::new();

    items.push(check_public_full_sanitized_tracked(repo_root));

    if public_root.is_empty() || !Path::new(&public_root).exists() {
        items.push(CheckItem::skip(
            "release-public-root",
            "release",
            "Public root not configured. Set AGS_PUBLIC_ROOT env var to enable release boundary checks.",
        ));
    } else {
        // Check 1: Run sync check with public target
        let (code, stdout, stderr) = run_command(
            repo_root,
            "cargo",
            &[
                "run",
                "-q",
                "-p",
                "ags-cli",
                "--",
                "sync",
                "check",
                "--source",
                &repo_root.to_string_lossy(),
                "--target",
                &public_root,
                "--target-name",
                "public-full-sanitized",
                "--format",
                "json",
            ],
            &[],
        );

        let output = format!("{}\n{}", stdout, stderr);
        let has_violation = output.contains("INVARIANT_MISSING")
            || output.contains("INVARIANT_CONTRADICTED")
            || output.contains("PUBLIC_FORBIDDEN_PAYLOAD");

        if code == 0 {
            items.push(CheckItem::pass(
                "release-boundary-sync",
                "release",
                "Public-full sanitized sync check passed — no boundary violations.",
            ));
        } else if has_violation {
            items.push(CheckItem::fail(
                "release-boundary-sync",
                "release",
                &format!(
                    "Public-full sanitized boundary violation: {}",
                    truncate(&output, 500)
                ),
                "Fix boundary violations before release. Check INVARIANT and PUBLIC_FORBIDDEN_PAYLOAD.",
            ));
        } else {
            items.push(CheckItem::warn(
                "release-boundary-sync",
                "release",
                "Public-full sanitized allowlist gap — review before release.",
                "Update public promotion allowlist.",
            ));
        }
    }

    // Check 2: Verify bootstrap --apply produces a sanitized public payload
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

// ── Builder extensions on CheckItem ─────────────────────────────────────────

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

    // Full scope — add drift checks
    if matches!(scope, Scope::Full) || matches!(scope, Scope::Release) {
        items.push(check_private_vs_stable_drift(&repo_root));
        items.push(check_private_vs_public_boundary(&repo_root));
    }

    // Release scope — add release-specific checks
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
        let expected = root
            .canonicalize()
            .unwrap_or_else(|_| root.clone())
            .to_string_lossy()
            .to_string();

        let (code, stdout, stderr) = run_command(&root, "sh", &["-c", "pwd"], &[]);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(code, 0, "stderr={stderr}");
        assert_eq!(stdout.trim(), expected);
    }

    #[test]
    fn test_public_workspace_detection() {
        let root =
            std::env::temp_dir().join(format!("ags-verify-public-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("WORKSPACE.md"),
            "# Agent Governance Suite — Public Edition Workspace\n\nThis is the public distributable edition.\n",
        )
        .unwrap();

        assert!(is_public_distributable_workspace(&root));
        let _ = std::fs::remove_dir_all(&root);
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
}
