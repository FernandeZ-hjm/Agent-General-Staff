//! Project discovery — Agent Governance Suite M2 Agent Awareness library.
//!
//! Provides three read-only capabilities for `ags` CLI:
//!
//! - **Project detection** (`detect_project`) — identify whether a repo is an
//!   AGS suite, an AGS-integrated project, or not integrated, including
//!   workspace role, protocol file inventory, and memory paths.
//! - **Protocol status** (`check_protocol_status`) — report which protocol
//!   files are present or missing, task-card validator entry point, risk
//!   boundaries, and review/verify/receipt requirements.
//! - **Agent instructions** (`generate_agent_instructions`) — export
//!   project instructions tailored to a specific agent (Codex, Claude Code,
//!   or Cursor) in text or structured JSON.
//!
//! All functions are read-only; they never mutate files, install hooks, or
//! launch agents.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Shared / common types ──────────────────────────────────────────────────

/// Known AGS protocol files under `protocol/`.
const PROTOCOL_FILES: &[(&str, &str)] = &[
    (
        "agent-task-protocol.md",
        "Task card and review rules (canonical)",
    ),
    ("task-card-template.md", "Fixed task card skeleton"),
    (
        "runtime-adapters.md",
        "Executor, permission, review, resume rules + resolver protocol",
    ),
    ("task-routing.md", "Light/medium/heavy task routing"),
    ("project-profile.md", "Project profile schema"),
    ("context-memory.md", "Context memory protocol"),
    ("2.0-baseline.md", "M0 baseline freeze document"),
    ("2.0-roadmap.md", "M0-M8 milestone roadmap"),
];

/// Known root-level protocol entry-point documents.
const ROOT_ENTRY_FILES: &[(&str, &str)] = &[
    ("AGENTS.md", "Agent entry point"),
    ("CLAUDE.md", "Agent execution protocol"),
    ("WORKSPACE.md", "Repository role map"),
    ("AGENT_SUITE_PROTOCOL.md", "Suite protocol overview"),
];

// ── Workspace detection ────────────────────────────────────────────────────

/// Parsed workspace identity row from WORKSPACE.md.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceIdentity {
    pub code: String,
    pub role: String,
    pub path: String,
}

/// Known workspace paths that imply a role even without WORKSPACE.md.
const KNOWN_WORKSPACE_PATHS: &[(&str, &str, &str)] = &[
    (
        "A",
        "Development private suite",
        "/Volumes/Projects/example-private-suite",
    ),
    (
        "A1",
        "Private bare repo",
        "/Volumes/Projects/remotes/example-private-suite.git",
    ),
    (
        "S",
        "Stable private suite",
        "/Volumes/Projects/example-stable-suite",
    ),
    (
        "B",
        "Public worktree",
        "/Volumes/AI Project/ai-dev-env-bootstrap",
    ),
    (
        "B1",
        "Public bare repo",
        "/Volumes/Projects/remotes/example-public-suite.git",
    ),
];

/// Parse WORKSPACE.md content to extract the workspace identity table.
///
/// Looks for a markdown table with columns `Code | Role | Path` and
/// extracts each row until a blank line or non-table line.
fn parse_workspace_table(content: &str) -> Vec<WorkspaceIdentity> {
    let mut identities = Vec::new();
    let mut in_table = false;
    let mut seen_header_sep = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect table header
        if trimmed.starts_with("| Code | Role | Path |")
            || trimmed.starts_with("| Code | Role | Path |")
        {
            in_table = true;
            continue;
        }

        if !in_table {
            continue;
        }

        // Header separator row
        if trimmed.starts_with("|---") || trimmed.starts_with("| ---") {
            seen_header_sep = true;
            continue;
        }

        // Table data row
        if trimmed.starts_with('|') && seen_header_sep {
            let cells: Vec<&str> = trimmed
                .split('|')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if cells.len() >= 3 {
                // Strip markdown backticks and whitespace from path cell
                let path = cells[2].trim_matches('`').trim().to_string();
                identities.push(WorkspaceIdentity {
                    code: cells[0].to_string(),
                    role: cells[1].to_string(),
                    path,
                });
            }
        } else if seen_header_sep && trimmed.is_empty() {
            // Blank line ends the table
            break;
        } else if seen_header_sep && !trimmed.starts_with('|') {
            // Non-table, non-blank line ends the table
            break;
        }
    }

    identities
}

// ── Project detection ──────────────────────────────────────────────────────

/// Integration status classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    /// This IS the AGS development suite.
    Suite,
    /// AGS-integrated project (has profile, AGENTS.md, memory).
    Integrated,
    /// Not integrated at all.
    NotIntegrated,
    /// Some AGS markers present but significant gaps.
    Partial,
}

/// Full project identity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIdentity {
    pub target: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_role: Option<WorkspaceIdentity>,
    pub integration_status: IntegrationStatus,
    pub is_ags_suite: bool,
    pub is_ags_integrated: bool,
    pub gaps: Vec<String>,
    pub workspace_identities: Vec<WorkspaceIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_profile_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_capsule_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_memory_path: Option<PathBuf>,
    pub protocol_files_found: Vec<String>,
    pub protocol_files_missing: Vec<String>,
    pub root_entry_files_found: Vec<String>,
    pub root_entry_files_missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
}

/// Detect project identity for the given target directory.
///
/// This is a read-only inspection; it does not modify any files.
pub fn detect_project(target: &Path) -> ProjectIdentity {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let mut identity = ProjectIdentity {
        target: canonical.clone(),
        inferred_role: None,
        integration_status: IntegrationStatus::NotIntegrated,
        is_ags_suite: false,
        is_ags_integrated: false,
        gaps: Vec::new(),
        workspace_identities: Vec::new(),
        project_profile_path: None,
        memory_capsule_path: None,
        task_memory_path: None,
        protocol_files_found: Vec::new(),
        protocol_files_missing: Vec::new(),
        root_entry_files_found: Vec::new(),
        root_entry_files_missing: Vec::new(),
        project_slug: None,
    };

    // ── Check root entry files ─────────────────────────────────────────
    for (name, _desc) in ROOT_ENTRY_FILES {
        let path = canonical.join(name);
        if path.exists() {
            identity.root_entry_files_found.push(name.to_string());
        } else {
            identity.root_entry_files_missing.push(name.to_string());
        }
    }

    // ── Check protocol/ files ──────────────────────────────────────────
    let protocol_dir = canonical.join("protocol");
    for (name, _desc) in PROTOCOL_FILES {
        let path = protocol_dir.join(name);
        if path.exists() {
            identity
                .protocol_files_found
                .push(format!("protocol/{}", name));
        } else {
            identity
                .protocol_files_missing
                .push(format!("protocol/{}", name));
        }
    }

    // ── Parse WORKSPACE.md for role identities ─────────────────────────
    let workspace_path = canonical.join("WORKSPACE.md");
    if workspace_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&workspace_path) {
            identity.workspace_identities = parse_workspace_table(&content);
        }
    }

    // ── Check for Cargo.toml with AGS workspace members ────────────────
    let cargo_toml = canonical.join("Cargo.toml");
    let has_ags_workspace = if cargo_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
            content.contains("ags-cli") || content.contains("task-card-validator")
        } else {
            false
        }
    } else {
        false
    };

    // ── Check for crates/ directory ────────────────────────────────────
    let crates_dir = canonical.join("crates");
    let has_crates_dir = crates_dir.is_dir();

    // ── Suite detection ────────────────────────────────────────────────
    identity.is_ags_suite = identity
        .root_entry_files_found
        .contains(&"WORKSPACE.md".to_string())
        && identity
            .root_entry_files_found
            .contains(&"AGENT_SUITE_PROTOCOL.md".to_string())
        && has_ags_workspace
        && has_crates_dir
        && !identity.workspace_identities.is_empty();

    // ── Integration markers ────────────────────────────────────────────
    // Check for project profile
    let profile_path = canonical.join("config").join("agent-project-profile.yaml");
    if profile_path.exists() {
        identity.project_profile_path = Some(profile_path.clone());
        // Extract slug if possible
        if let Ok(content) = std::fs::read_to_string(&profile_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("slug:") {
                    identity.project_slug = Some(
                        trimmed
                            .strip_prefix("slug:")
                            .unwrap()
                            .trim()
                            .trim_matches('"')
                            .to_string(),
                    );
                    break;
                }
            }
        }
    }

    // Check for memory capsule
    let slug = identity
        .project_slug
        .clone()
        .unwrap_or_else(|| slug_from_path(&canonical));
    let home = ags_platform::home_dir_or_temp();
    let capsule_path = home
        .join(".agents/memory/projects")
        .join(&slug)
        .join("context-capsule.md");
    if capsule_path.exists() {
        identity.memory_capsule_path = Some(capsule_path.clone());
    }
    let task_mem_path = home
        .join(".agents/memory/projects")
        .join(&slug)
        .join("task-memory.md");
    if task_mem_path.exists() {
        identity.task_memory_path = Some(task_mem_path);
    }

    // Check AGENTS.md references AGS
    let agents_md = canonical.join("AGENTS.md");
    let agents_refs_ags = if agents_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&agents_md) {
            content.contains("AGENT_SUITE_PROTOCOL.md")
                || content.contains("agent-governance")
                || content.contains("task-card-validator")
        } else {
            false
        }
    } else {
        false
    };

    // Count integration markers
    let mut integration_markers = 0u8;
    if identity.project_profile_path.is_some() {
        integration_markers += 1;
    }
    if identity.memory_capsule_path.is_some() {
        integration_markers += 1;
    }
    if agents_refs_ags {
        integration_markers += 1;
    }

    // ── Inferred role from known paths ─────────────────────────────────
    let canonical_str = canonical.to_string_lossy().to_string();
    for (code, role, path) in KNOWN_WORKSPACE_PATHS {
        if canonical_str == *path {
            identity.inferred_role = Some(WorkspaceIdentity {
                code: code.to_string(),
                role: role.to_string(),
                path: path.to_string(),
            });
            break;
        }
    }

    // If no identity table but we have a known role, add it
    if identity.workspace_identities.is_empty() {
        if let Some(ref role) = identity.inferred_role {
            identity.workspace_identities.push(role.clone());
        }
    }

    // ── Classify integration status ────────────────────────────────────
    if identity.is_ags_suite {
        identity.integration_status = IntegrationStatus::Suite;
    } else if integration_markers >= 3 {
        identity.integration_status = IntegrationStatus::Integrated;
    } else if integration_markers > 0 {
        identity.integration_status = IntegrationStatus::Partial;
    } else {
        identity.integration_status = IntegrationStatus::NotIntegrated;
    }

    // is_ags_integrated must be consistent with integration_status
    identity.is_ags_integrated = matches!(
        identity.integration_status,
        IntegrationStatus::Suite | IntegrationStatus::Integrated
    );

    // ── Build gaps list ────────────────────────────────────────────────
    if identity.integration_status != IntegrationStatus::Suite
        && identity.integration_status != IntegrationStatus::Integrated
    {
        if identity.project_profile_path.is_none() {
            identity
                .gaps
                .push("Missing config/agent-project-profile.yaml".to_string());
        }
        if identity.memory_capsule_path.is_none() {
            identity
                .gaps
                .push("Missing context-capsule.md in local memory".to_string());
        }
        if !agents_refs_ags {
            identity
                .gaps
                .push("AGENTS.md does not reference AGS protocols".to_string());
        }
        if !identity
            .root_entry_files_found
            .contains(&"CLAUDE.md".to_string())
        {
            identity
                .gaps
                .push("Missing CLAUDE.md (agent execution protocol)".to_string());
        }
    }

    // For suite repos, list any missing protocol files
    if identity.is_ags_suite {
        for m in &identity.protocol_files_missing {
            identity.gaps.push(format!("Missing protocol file: {}", m));
        }
        for m in &identity.root_entry_files_missing {
            identity.gaps.push(format!("Missing root entry: {}", m));
        }
    }

    identity
}

/// Derive a project slug from a path (fallback when no profile exists).
fn slug_from_path(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── Protocol status ────────────────────────────────────────────────────────

/// Status of a single protocol file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolFileStatus {
    pub name: String,
    pub path: PathBuf,
    pub present: bool,
    pub description: String,
    pub category: String, // "protocol" or "root_entry"
}

/// Task-card validator entry information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub available: bool,
    pub entry: String,
    pub description: String,
    pub alternate_entry: String,
}

/// Risk boundary information derived from protocol docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskBoundaries {
    pub protected_paths: Vec<String>,
    pub high_risk_indicators: Vec<String>,
    pub destructive_actions_require_confirmation: bool,
    pub public_payload_boundary: String,
}

/// Review requirements per task level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequirements {
    pub light: String,
    pub medium: String,
    pub heavy: String,
}

/// Receipt and delivery requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptRequirements {
    pub delivery_report_required: bool,
    pub format_reference: String,
    pub archive_location: String,
}

/// Full protocol status report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolStatus {
    pub target: PathBuf,
    pub protocol_dir: PathBuf,
    pub protocol_dir_exists: bool,
    pub files: Vec<ProtocolFileStatus>,
    pub present_count: usize,
    pub missing_count: usize,
    pub task_card_validator: ValidatorInfo,
    pub risk_boundaries: RiskBoundaries,
    pub review_requirements: ReviewRequirements,
    pub verify_requirements: Vec<String>,
    pub receipt_requirements: ReceiptRequirements,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
}

/// Detect available verification commands for a target directory.
///
/// Scans for project-specific tooling (Cargo, scripts/verify.sh, etc.)
/// and returns the list of verification commands that apply to the target.
/// If no project-specific tooling is found, returns guidance rather than
/// false commands.
fn detect_verification_commands(target: &Path) -> Vec<String> {
    let mut commands = Vec::new();

    // Check for Rust/Cargo project
    if target.join("Cargo.toml").exists() {
        commands.push("cargo fmt --check".to_string());
        commands.push("RUSTFLAGS=\"-D warnings\" cargo test".to_string());
        commands.push("cargo build --release".to_string());
    }

    // Check for verify.sh
    if target.join("scripts").join("verify.sh").exists() {
        commands.push("bash scripts/verify.sh".to_string());
    }

    // If no project tooling found, give guidance
    if commands.is_empty() {
        commands.push("No project-specific verification commands detected.".to_string());
        commands
            .push("Install AGS verification scripts or define verification commands".to_string());
        commands.push("in config/agent-project-profile.yaml.".to_string());
    }

    commands
}

/// Check protocol file status for a target directory.
///
/// Reports which protocol files exist, which are missing, and provides
/// the task-card validator entry, risk boundaries, and review/verify/receipt
/// requirements extracted from protocol documentation.
pub fn check_protocol_status(target: &Path) -> ProtocolStatus {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let protocol_dir = canonical.join("protocol");
    let protocol_dir_exists = protocol_dir.is_dir();

    let mut files = Vec::new();

    // Check protocol/ files
    for (name, desc) in PROTOCOL_FILES {
        let path = protocol_dir.join(name);
        files.push(ProtocolFileStatus {
            name: format!("protocol/{}", name),
            path: path.clone(),
            present: path.exists(),
            description: desc.to_string(),
            category: "protocol".to_string(),
        });
    }

    // Check root entry files
    for (name, desc) in ROOT_ENTRY_FILES {
        let path = canonical.join(name);
        files.push(ProtocolFileStatus {
            name: name.to_string(),
            path: path.clone(),
            present: path.exists(),
            description: desc.to_string(),
            category: "root_entry".to_string(),
        });
    }

    let present_count = files.iter().filter(|f| f.present).count();
    let missing_count = files.len() - present_count;

    // ── Task-card validator info ───────────────────────────────────────
    let cargo_toml = canonical.join("Cargo.toml");
    let has_rust_validator = if cargo_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
            content.contains("task-card-validator")
        } else {
            false
        }
    } else {
        false
    };

    let validate_script = canonical.join("scripts").join("validate.sh");
    let has_validate_script = validate_script.exists();

    let (validator_entry, alternate_entry, validator_available) = if has_rust_validator {
        (
            "cargo run -p ags-cli -- task validate <task-card>".to_string(),
            "bash scripts/validate.sh <task-card>".to_string(),
            true,
        )
    } else if has_validate_script {
        (
            "bash scripts/validate.sh <task-card>".to_string(),
            "N/A (no Rust validator in this repo)".to_string(),
            true,
        )
    } else {
        (
            "N/A (no validator found in this repo)".to_string(),
            "N/A".to_string(),
            false,
        )
    };

    let task_card_validator = ValidatorInfo {
        available: validator_available,
        entry: validator_entry,
        description: "Rust task-card-validator is the canonical task-card format gate. It provides structural format checks, field-value validation, field-combination checks, Execution Authority Gate, protected-path analysis, contradiction detection, and content-quality checks.".to_string(),
        alternate_entry,
    };

    // ── Risk boundaries ────────────────────────────────────────────────
    let risk_boundaries = RiskBoundaries {
        protected_paths: vec![
            "protocol/".to_string(),
            "crates/task-card-validator/".to_string(),
            "Cargo.toml".to_string(),
            "Cargo.lock".to_string(),
            "scripts/verify.sh".to_string(),
            "$HOME/.agents/memory/projects/".to_string(),
        ],
        high_risk_indicators: vec![
            "Data migration or historical output mutation".to_string(),
            "Baseline deletion or overwrite".to_string(),
            "Hook installation or production wiring".to_string(),
            "Public-full sanitized payload boundary change".to_string(),
            "Canonical task-card skeleton modification".to_string(),
            "Execution-policy M1-M10 rule change".to_string(),
            "Stable (S) direct modification from A".to_string(),
        ],
        destructive_actions_require_confirmation: true,
        public_payload_boundary: "Public-full sanitized payload may include the public Rust ags workspace (Cargo.toml, Cargo.lock, crates/) and public governance runtime, but must not include target/, release/debug ags binaries, build caches, preinstalled private skill packs, local agent config, real memory, real receipts, real task archives, secrets, or machine-specific private state.".to_string(),
    };

    // ── Review requirements ────────────────────────────────────────────
    let review_requirements = ReviewRequirements {
        light: "Complete verification then run requesting-code-review or equivalent light diff review. Upgrade to Medium if cross-file protocol, permission, hook, data writes, path migration, or artifact sync risks are found.".to_string(),
        medium: "Codex final Review gate. Executor marks task as 'partially complete / awaiting Codex review' after verification. Codex reviews and approves before release.".to_string(),
        heavy: "Plan-first then execute. Human Adversarial Review gate. Executor marks task as 'partially complete / awaiting human adversarial review' and reminds operator to run /codex:adversarial-review before release.".to_string(),
    };

    // ── Verify requirements ────────────────────────────────────────────
    let verify_requirements = detect_verification_commands(&canonical);

    // ── Receipt requirements ───────────────────────────────────────────
    let receipt_requirements = ReceiptRequirements {
        delivery_report_required: true,
        format_reference: "See protocol/agent-task-protocol.md delivery report format: one copyable Markdown fenced block containing task status, one-line conclusion, changed files, new outputs, deleted files, verification results, risk notes, next steps.".to_string(),
        archive_location: "$HOME/.agents/memory/projects/<project-slug>/task-archive/".to_string(),
    };

    // ── Warnings and failures ──────────────────────────────────────────
    let mut warnings = Vec::new();
    let mut failures = Vec::new();

    // Critical protocol files that must exist
    let critical_files = [
        "AGENTS.md",
        "CLAUDE.md",
        "protocol/agent-task-protocol.md",
        "protocol/task-card-template.md",
        "protocol/runtime-adapters.md",
    ];

    for critical in &critical_files {
        let found = files.iter().any(|f| f.name == *critical && f.present);
        if !found {
            failures.push(format!(
                "CRITICAL: {} is missing — required for agent governance",
                critical
            ));
        }
    }

    // Check for validator
    if !validator_available {
        failures.push(
            "CRITICAL: No task-card validator found — task cards cannot be validated in this repo"
                .to_string(),
        );
    }

    // Non-critical but recommended
    let recommended = [
        "WORKSPACE.md",
        "AGENT_SUITE_PROTOCOL.md",
        "protocol/task-routing.md",
        "protocol/project-profile.md",
        "protocol/context-memory.md",
    ];

    for rec in &recommended {
        let found = files.iter().any(|f| f.name == *rec && f.present);
        if !found {
            warnings.push(format!(
                "Recommended file {} is missing — some governance features may be unavailable",
                rec
            ));
        }
    }

    // If protocol dir doesn't exist at all
    if !protocol_dir_exists {
        warnings.push(format!(
            "protocol/ directory not found at {} — this repo may not be an AGS-governed project",
            protocol_dir.display()
        ));
    }

    ProtocolStatus {
        target: canonical,
        protocol_dir,
        protocol_dir_exists,
        files,
        present_count,
        missing_count,
        task_card_validator,
        risk_boundaries,
        review_requirements,
        verify_requirements,
        receipt_requirements,
        warnings,
        failures,
    }
}

// ── Agent instructions ─────────────────────────────────────────────────────

/// Agent type for instruction generation.
///
/// Known hosts get tailored instructions. Unknown non-empty host identifiers
/// fall back to a generic governed-host profile so new desktop modes do not
/// fail just because their agent string is new.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentType {
    Codex,
    ClaudeCode,
    Cursor,
    Generic(String),
}

impl AgentType {
    #[allow(clippy::should_implement_trait)] // inherent parser with domain String error; intentionally not std::str::FromStr
    pub fn from_str(s: &str) -> Result<Self, String> {
        let normalized = normalize_agent_id(s)?;
        match normalized.as_str() {
            "codex" => Ok(AgentType::Codex),
            "claude" | "claude-code" | "claudecode" => Ok(AgentType::ClaudeCode),
            "cursor" => Ok(AgentType::Cursor),
            other => Ok(AgentType::Generic(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            AgentType::Codex => "codex",
            AgentType::ClaudeCode => "claude-code",
            AgentType::Cursor => "cursor",
            AgentType::Generic(agent) => agent.as_str(),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            AgentType::Codex => "Codex".to_string(),
            AgentType::ClaudeCode => "Claude Code".to_string(),
            AgentType::Cursor => "Cursor".to_string(),
            AgentType::Generic(agent) => match recognized_host_display(agent) {
                Some(name) => name.to_string(),
                None => format!("Generic Agent ({agent})"),
            },
        }
    }

    pub fn is_generic(&self) -> bool {
        matches!(self, AgentType::Generic(_))
    }
}

fn normalize_agent_id(input: &str) -> Result<String, String> {
    let mut normalized = String::new();
    let mut last_was_sep = false;

    for ch in input.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            normalized.push(lower);
            last_was_sep = false;
        } else if matches!(lower, '-' | '_' | '.' | ' ' | '\t' | '\n') && !last_was_sep {
            normalized.push('-');
            last_was_sep = true;
        }
    }

    while normalized.ends_with('-') {
        normalized.pop();
    }

    if normalized.is_empty() {
        Err("invalid agent type: empty or unsupported identifier".to_string())
    } else {
        Ok(normalized)
    }
}

/// Recognized governed-host display names, keyed by normalized agent id.
///
/// These hosts are still carried as `AgentType::Generic` — they get a branded
/// display name and are recognized as Tencent Agent host clients, but they add
/// NO new canonical runtime adapter and do NOT change the generic fallback for
/// unknown hosts. `normalize_agent_id` folds input casing/spacing into these
/// keys (e.g. "WorkBuddy" → "workbuddy", "CodeBuddy-Code" → "codebuddy-code",
/// "Tencent Agent" → "tencent-agent"). Tencent Agent is the umbrella adapter
/// entry; WorkBuddy and CodeBuddy-Code are its host clients.
const RECOGNIZED_HOST_DISPLAY: &[(&str, &str)] = &[
    ("tencent-agent", "Tencent Agent"),
    ("tencent", "Tencent Agent"),
    ("workbuddy", "Tencent Agent (WorkBuddy)"),
    ("codebuddy-code", "Tencent Agent (CodeBuddy-Code)"),
    ("codebuddy", "Tencent Agent (CodeBuddy-Code)"),
];

/// Branded display name for a recognized governed host, or `None` for an unknown
/// generic host (which keeps the plain `Generic Agent (x)` form). The input must
/// already be normalized via `normalize_agent_id`.
fn recognized_host_display(normalized: &str) -> Option<&'static str> {
    RECOGNIZED_HOST_DISPLAY
        .iter()
        .find(|(key, _)| *key == normalized)
        .map(|(_, name)| *name)
}

impl Serialize for AgentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        AgentType::from_str(&value).map_err(serde::de::Error::custom)
    }
}

/// Agent-specific project instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstructions {
    pub agent_type: String,
    pub agent_display_name: String,
    pub target: PathBuf,
    pub project_name: String,
    pub is_ags_suite: bool,
    pub integration_status: IntegrationStatus,
    pub required_reads: Vec<InstructionFile>,
    pub protocol_entry_points: Vec<String>,
    pub verification_commands: Vec<String>,
    pub role_description: String,
    pub risk_boundaries: RiskBoundaries,
    pub stop_conditions: Vec<String>,
    pub permissions: AgentPermissions,
    pub instructions_text: String,
    /// When true, the agent must stop before executing in this repo.
    pub should_stop: bool,
    /// Reasons the agent must stop (integration gaps, protocol failures).
    pub stop_reasons: Vec<String>,
    /// Integration gaps from project detection.
    pub integration_gaps: Vec<String>,
    /// Critical protocol failures that block agent execution.
    pub protocol_failures: Vec<String>,
    /// Protocol warnings (non-blocking).
    pub protocol_warnings: Vec<String>,
    /// Recommended exit code: 0 for suite/integrated, 1 for partial/not-integrated with failures.
    pub exit_code: i32,
}

/// A file the agent must read before starting work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionFile {
    pub path: String,
    pub description: String,
    pub priority: String, // "required" or "recommended"
}

/// Agent-specific permission defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissions {
    pub default_permission_mode: String,
    pub default_parallelism: String,
    pub may_edit_files: bool,
    pub may_delegate: bool,
    pub may_install: bool,
}

/// Generate agent instructions for the given agent type and target.
pub fn generate_agent_instructions(target: &Path, agent_type: &AgentType) -> AgentInstructions {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let identity = detect_project(&canonical);
    let project_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let (role_description, permissions, stop_conditions, required_reads) =
        match agent_type {
            AgentType::Codex => (
                "Codex owns the full pre-execution lifecycle: ambient preflight (project detection, context reading), solution formation (understanding, diagnosis, approach design), user confirmation, execution contract formalization, and task routing (Light/Medium/Heavy classification). Classification happens only after the solution is confirmed — never from raw user requests. Codex may directly implement changes for light/medium tasks and may delegate bounded execution to Claude Code CLI, but must provide a self-contained task card (from the confirmed execution contract) and review the result before treating the task as complete.\n\nCRITICAL — Task-Card Request Gate: \"方案 OK\" only ends the solution phase. Codex must NOT call `ags task compile --task-card-requested` or generate executable task cards until the user explicitly issues a task-card instruction (\"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.). Without explicit user instruction, the compiler gate will block executable output. The lifecycle is: preflight → solution → user says 方案 OK → user issues task-card instruction → execution contract → routing → task card → gate/execution/receipt."
                    .to_string(),
                AgentPermissions {
                    default_permission_mode: "execute-and-verify".to_string(),
                    default_parallelism: "none".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Do not install hooks, runner adapters, or production wiring without explicit task-card authorization."
                        .to_string(),
                    "Stop before broad refactors unless the task card explicitly authorizes them."
                        .to_string(),
                    "If actual risk is higher than the task card declares, stop and report — do not silently downgrade."
                        .to_string(),
                    "Do not modify S (stable) directly without explicit task-card authorization."
                        .to_string(),
                    "Do not change public-full sanitized payload boundary, canonical task-card skeleton, or execution-policy M1-M10 rules without explicit approval."
                        .to_string(),
                    "Do not generate executable task cards or call `ags task compile --task-card-requested` until the user explicitly issues a task-card instruction. \"方案 OK\" only ends the solution phase — a separate user task-card instruction is required before routing and task card generation."
                        .to_string(),
                ],
                vec![
                    InstructionFile {
                        path: "AGENTS.md".to_string(),
                        description: "Agent entry point".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "CLAUDE.md".to_string(),
                        description: "Agent execution protocol".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "AGENT_SUITE_PROTOCOL.md".to_string(),
                        description: "Suite protocol overview".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/agent-task-protocol.md".to_string(),
                        description: "Task card and review rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-card-template.md".to_string(),
                        description: "Fixed task card skeleton".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/runtime-adapters.md".to_string(),
                        description: "Executor, permission, review, resume rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-routing.md".to_string(),
                        description: "Light/medium/heavy task routing".to_string(),
                        priority: "required".to_string(),
                    },
                ],
            ),
            AgentType::ClaudeCode => (
                "Claude Code executes bounded task cards formed from confirmed execution contracts. It reads the task card, protocol files, and project docs; implements within scope; runs verification; and outputs a delivery report. Claude Code must NOT perform ambient preflight, solution formation, or task-level classification from raw user requests — those phases belong to Codex/Cursor. Claude Code must not frame tasks, change protocol boundaries, or install hooks without explicit task-card authorization.\n\nCRITICAL — Task-Card Request Gate: Claude Code only consumes already-formed task cards. It must NOT generate task cards from raw user requests or from solution-phase outputs. The \"方案 OK\" signal is not a routing trigger — only an explicit user task-card instruction (\"生成任务卡\", \"交给 Claude Code 执行\", etc.) authorizes routing and task card generation. Claude Code must NOT self-classify tasks as Light/Medium/Heavy from raw requests."
                    .to_string(),
                AgentPermissions {
                    default_permission_mode: "execute-and-verify".to_string(),
                    default_parallelism: "none".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Do not perform task-level classification (Light/Medium/Heavy) from raw user requests — task classification belongs to the Codex/Cursor pre-execution lifecycle. Claude Code executes the task level already declared in the task card."
                        .to_string(),
                    "Do not modify files outside the task card scope, even if they appear related."
                        .to_string(),
                    "Do not install hooks, runner adapters, or production wiring without explicit task-card authorization."
                        .to_string(),
                    "Do not install dependencies without first explaining necessity and waiting for confirmation."
                        .to_string(),
                    "Stop before destructive git commands (push --force, reset --hard, etc.) unless the task card explicitly authorizes them."
                        .to_string(),
                    "If the task risk escalates beyond what the task card declares, stop and report."
                        .to_string(),
                    "For Heavy tasks: task level is a risk/review tier, not the execution authority — it never downgrades the permission mode and never by itself adds a mutation gate or forces a plan-first round trip. Two Heavy classes: Heavy plan (Permission mode plan-only, declared or the compiler default when Permission mode is unspecified) returns root cause + design + implementation plan + verification plan and waits for explicit human approval before mutation; Heavy execute (Permission mode execute-and-verify) runs and verifies directly per the task card. Always honor the card's independent Review gate and stop conditions."
                        .to_string(),
                    "On resume/continue: reread the task card, run `git status --short`, reconfirm review_targets, and honor the card's permission mode (plan-only awaits explicit approval; execute-and-verify resumes execution and verification)."
                        .to_string(),
                    "Do not generate task cards or call `ags task compile --task-card-requested` from raw user requests or solution-phase outputs. Only Codex/Cursor may generate task cards after receiving an explicit user task-card instruction (\"生成任务卡\", \"按这个方案出任务卡\", etc.). \"方案 OK\" alone is not a task-card generation trigger."
                        .to_string(),
                ],
                vec![
                    InstructionFile {
                        path: "AGENTS.md".to_string(),
                        description: "Agent entry point".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "CLAUDE.md".to_string(),
                        description: "Agent execution protocol".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "AGENT_SUITE_PROTOCOL.md".to_string(),
                        description: "Suite protocol overview".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/agent-task-protocol.md".to_string(),
                        description: "Task card and review rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-card-template.md".to_string(),
                        description: "Fixed task card skeleton".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/runtime-adapters.md".to_string(),
                        description: "Executor, permission, review, resume rules".to_string(),
                        priority: "required".to_string(),
                    },
                ],
            ),
            AgentType::Cursor => (
                "Cursor owns the full pre-execution lifecycle inside its IDE workflow: ambient preflight (project detection, context reading), solution formation (understanding, diagnosis, approach design), user confirmation, execution contract formalization, and task routing (Light/Medium/Heavy classification). Classification happens only after the solution is confirmed — never from raw user requests. Cursor may directly implement changes or delegate to Claude Code CLI. When delegating, Cursor generates task cards from the confirmed execution contract using the canonical task-card template.\n\nCRITICAL — Task-Card Request Gate: \"方案 OK\" only ends the solution phase. Cursor must NOT generate executable task cards until the user explicitly issues a task-card instruction (\"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.). Without explicit user instruction, `ags task compile` will block executable output with `executable_allowed=false` and `block_reason=task_card_not_requested`. The lifecycle is: preflight → solution → user says 方案 OK → user issues task-card instruction → execution contract → routing → task card → gate/execution/receipt."
                    .to_string(),
                AgentPermissions {
                    default_permission_mode: "execute-and-verify".to_string(),
                    default_parallelism: "none".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Stop before broad refactors unless the task card explicitly authorizes them."
                        .to_string(),
                    "Do not install hooks without explicit task-card authorization."
                        .to_string(),
                    "Keep task-card facts project-local; do not bake global suite internals into project-specific prompts."
                        .to_string(),
                    "Use IDE context only as supporting evidence; final claims still need commands, diffs, screenshots, or other explicit evidence."
                        .to_string(),
                    "If delegating to Claude Code CLI, provide a self-contained prompt and review the result before treating the task as complete."
                        .to_string(),
                ],
                vec![
                    InstructionFile {
                        path: "AGENTS.md".to_string(),
                        description: "Agent entry point".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "CLAUDE.md".to_string(),
                        description: "Agent execution protocol".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "AGENT_SUITE_PROTOCOL.md".to_string(),
                        description: "Suite protocol overview".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/agent-task-protocol.md".to_string(),
                        description: "Task card and review rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-card-template.md".to_string(),
                        description: "Fixed task card skeleton".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/runtime-adapters.md".to_string(),
                        description: "Executor, permission, review, resume rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-routing.md".to_string(),
                        description: "Light/medium/heavy task routing".to_string(),
                        priority: "required".to_string(),
                    },
                ],
            ),
            AgentType::Generic(agent) => {
                let host_label = recognized_host_display(agent)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Generic Agent ({agent})"));
                (
                format!(
                    "{host_label} is an AGS-compatible governed host (Tencent Agent hosts — WorkBuddy and CodeBuddy-Code — resolve to this governed-host profile; other unknown hosts use it too). It must complete AGS initialization preflight before any AGS scenario work, then follow the governed lifecycle surfaced by the preflight report. Governed hosts may read project context and help form solutions, but they must not assume Codex, Claude Code, or Cursor-specific privileges from their agent name alone.\n\nCRITICAL — Task-Card Request Gate: do not generate executable task cards from raw user requests or solution-phase output. \"方案 OK\" only ends solution formation; a separate explicit task-card instruction is required before routing or execution."
                ),
                AgentPermissions {
                    default_permission_mode: "execute-and-verify".to_string(),
                    default_parallelism: "none".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Call `ags_preflight` first for AGS scenarios; do not call other AGS tools before preflight succeeds.".to_string(),
                    "Use the explicit `target` project path supplied by the host; do not assume the desktop workspace folder is the governed project.".to_string(),
                    "Do not perform Light/Medium/Heavy task classification from raw user requests.".to_string(),
                    "Do not install hooks, runner adapters, dependencies, or production wiring without explicit task-card authorization.".to_string(),
                    "Do not generate task cards or call `ags task compile --task-card-requested` until the user explicitly issues a task-card instruction.".to_string(),
                    "If the host cannot identify the target project, stop and ask for the repository path instead of running `ags init` in the current desktop workspace.".to_string(),
                ],
                vec![
                    InstructionFile {
                        path: "AGENTS.md".to_string(),
                        description: "Agent entry point".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "CLAUDE.md".to_string(),
                        description: "Agent execution protocol".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "AGENT_SUITE_PROTOCOL.md".to_string(),
                        description: "Suite protocol overview".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/agent-task-protocol.md".to_string(),
                        description: "Task card and review rules".to_string(),
                        priority: "required".to_string(),
                    },
                    InstructionFile {
                        path: "protocol/task-routing.md".to_string(),
                        description: "Task routing lifecycle".to_string(),
                        priority: "required".to_string(),
                    },
                ],
                )
            }
        };

    let protocol_entry_points = required_reads
        .iter()
        .filter(|f| f.priority == "required")
        .map(|f| f.path.clone())
        .collect();

    // ── Target-aware verification commands ──────────────────────────────
    let verification_commands = detect_verification_commands(&canonical);

    // ── Protocol status for integration context ─────────────────────────
    let protocol_status = check_protocol_status(&canonical);
    let protocol_failures = protocol_status.failures.clone();
    let protocol_warnings = protocol_status.warnings.clone();
    let integration_gaps = identity.gaps.clone();

    // ── Determine stop behavior ─────────────────────────────────────────
    let (should_stop, stop_reasons, exit_code) = match identity.integration_status {
        IntegrationStatus::Suite | IntegrationStatus::Integrated => {
            // Still check for critical protocol failures
            let critical_failures: Vec<String> = protocol_failures
                .iter()
                .filter(|f| f.starts_with("CRITICAL:"))
                .cloned()
                .collect();
            if critical_failures.is_empty() {
                (false, vec![], 0)
            } else {
                (
                    true,
                    critical_failures
                        .iter()
                        .map(|f| format!("Protocol failure: {}", f))
                        .collect(),
                    1,
                )
            }
        }
        IntegrationStatus::Partial => {
            let mut reasons: Vec<String> = integration_gaps
                .iter()
                .map(|g| format!("Integration gap: {}", g))
                .collect();
            for f in &protocol_failures {
                reasons.push(format!("Protocol failure: {}", f));
            }
            (true, reasons, 1)
        }
        IntegrationStatus::NotIntegrated => {
            let mut reasons: Vec<String> = integration_gaps
                .iter()
                .map(|g| format!("Integration gap: {}", g))
                .collect();
            for f in &protocol_failures {
                reasons.push(format!("Protocol failure: {}", f));
            }
            (true, reasons, 1)
        }
    };

    // ── Build risk boundaries ───────────────────────────────────────────
    let risk_boundaries = RiskBoundaries {
        protected_paths: vec![
            "protocol/".to_string(),
            "crates/task-card-validator/".to_string(),
            "Cargo.toml".to_string(),
            "Cargo.lock".to_string(),
        ],
        high_risk_indicators: vec![
            "Protocol boundary changes".to_string(),
            "Hook installation or production wiring".to_string(),
            "Public-full sanitized payload boundary change".to_string(),
            "Canonical task-card skeleton modification".to_string(),
            "Execution-policy M1-M10 rule change".to_string(),
            "Stable (S) direct modification from A".to_string(),
        ],
        destructive_actions_require_confirmation: true,
        public_payload_boundary: "Public-full sanitized payload may include the public Rust ags workspace (Cargo.toml, Cargo.lock, crates/) and public governance runtime, but must not include target/, release/debug ags binaries, build caches, preinstalled private skill packs, local agent config, real memory, real receipts, real task archives, secrets, or machine-specific private state.".to_string(),
    };

    let instructions_text = build_instructions_text(
        agent_type,
        &project_name,
        &identity,
        &role_description,
        &required_reads,
        &stop_conditions,
        &permissions,
        &verification_commands,
        &risk_boundaries,
        should_stop,
        &stop_reasons,
        &integration_gaps,
        &protocol_failures,
        &protocol_warnings,
    );

    AgentInstructions {
        agent_type: agent_type.as_str().to_string(),
        agent_display_name: agent_type.display_name(),
        target: canonical,
        project_name,
        is_ags_suite: identity.is_ags_suite,
        integration_status: identity.integration_status.clone(),
        required_reads,
        protocol_entry_points,
        verification_commands,
        role_description,
        risk_boundaries,
        stop_conditions,
        permissions,
        instructions_text,
        should_stop,
        stop_reasons,
        integration_gaps,
        protocol_failures,
        protocol_warnings,
        exit_code,
    }
}

/// Build the human-readable instructions text block.
#[allow(clippy::too_many_arguments)] // cohesive instruction-rendering inputs; a parameter struct adds indirection without clarity
fn build_instructions_text(
    agent_type: &AgentType,
    project_name: &str,
    identity: &ProjectIdentity,
    role_description: &str,
    required_reads: &[InstructionFile],
    stop_conditions: &[String],
    permissions: &AgentPermissions,
    verification_commands: &[String],
    risk_boundaries: &RiskBoundaries,
    should_stop: bool,
    stop_reasons: &[String],
    integration_gaps: &[String],
    protocol_failures: &[String],
    protocol_warnings: &[String],
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!(
        "# Agent Governance Instructions — {}",
        agent_type.display_name()
    ));
    lines.push(String::new());

    // ── STOP banner for non-integrated/partial repos ────────────────────
    if should_stop {
        lines.push("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        lines.push("⛔ STOP — DO NOT EXECUTE IN THIS REPOSITORY ⛔".to_string());
        lines.push("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        lines.push(String::new());
        lines.push("This repository is not fully integrated with AGS governance.".to_string());
        lines.push(
            "The agent must NOT execute tasks here until all stop reasons are resolved."
                .to_string(),
        );
        lines.push(String::new());
        lines.push("Stop reasons:".to_string());
        for reason in stop_reasons {
            lines.push(format!("  ✗ {}", reason));
        }
        lines.push(String::new());
        if !integration_gaps.is_empty() {
            lines.push("Integration gaps:".to_string());
            for gap in integration_gaps {
                lines.push(format!("  ! {}", gap));
            }
            lines.push(String::new());
        }
        if !protocol_failures.is_empty() {
            lines.push("Protocol failures:".to_string());
            for f in protocol_failures {
                lines.push(format!("  ✗ {}", f));
            }
            lines.push(String::new());
        }
        if !protocol_warnings.is_empty() {
            lines.push("Protocol warnings:".to_string());
            for w in protocol_warnings {
                lines.push(format!("  ! {}", w));
            }
            lines.push(String::new());
        }
        lines.push("Resolution: install AGS governance files (AGENTS.md, CLAUDE.md,".to_string());
        lines.push(
            "protocol/, task-card-validator) before executing tasks in this repo.".to_string(),
        );
        lines.push(
            "Use `ags init --target <dir>` to onboard the project, or manually add".to_string(),
        );
        lines.push("the required protocol files from an AGS suite distribution.".to_string());
        lines.push(String::new());
        lines.push("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        lines.push(String::new());

        // Return early — don't emit executable-looking instructions
        lines.push(
            "Below would be the standard instructions for an integrated project.".to_string(),
        );
        lines.push("They are NOT safe to execute until the issues above are resolved.".to_string());
        lines.push(String::new());
        lines.push("---".to_string());
        lines.push(String::new());
        lines.push("## REFERENCE ONLY — Standard Instructions (unsafe to execute)".to_string());
        lines.push(String::new());
    }

    lines.push(format!("Project: {}", project_name));
    lines.push(format!(
        "Integration status: {:?}",
        identity.integration_status
    ));
    lines.push(String::new());

    // Role
    lines.push("## Role".to_string());
    lines.push(role_description.to_string());
    lines.push(String::new());

    // Required reads
    lines.push("## Required Reads (Before Starting Any Task)".to_string());
    for f in required_reads {
        lines.push(format!(
            "- [{}] `{}` — {}",
            f.priority.to_uppercase(),
            f.path,
            f.description
        ));
    }
    lines.push(String::new());

    // Permissions
    lines.push("## Default Permissions".to_string());
    lines.push(format!(
        "- Permission mode: {}",
        permissions.default_permission_mode
    ));
    lines.push(format!(
        "- Parallelism: {}",
        permissions.default_parallelism
    ));
    lines.push(format!(
        "- May edit files: {}",
        if permissions.may_edit_files {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(format!(
        "- May delegate: {}",
        if permissions.may_delegate {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(format!(
        "- May install: {}",
        if permissions.may_install { "yes" } else { "no" }
    ));
    lines.push(String::new());

    // Verification
    lines.push("## Verification Commands".to_string());
    for cmd in verification_commands {
        lines.push(format!("- `{}`", cmd));
    }
    lines.push(String::new());

    // Risk boundaries
    lines.push("## Risk Boundaries".to_string());
    lines.push("### Protected Paths (do not modify without explicit authorization)".to_string());
    for p in &risk_boundaries.protected_paths {
        lines.push(format!("- `{}`", p));
    }
    lines.push(String::new());
    lines.push("### High-Risk Indicators (escalate to Heavy if any apply)".to_string());
    for r in &risk_boundaries.high_risk_indicators {
        lines.push(format!("- {}", r));
    }
    lines.push(String::new());

    // Stop conditions
    lines.push("## Stop Conditions".to_string());
    for (i, s) in stop_conditions.iter().enumerate() {
        lines.push(format!("{}. {}", i + 1, s));
    }
    lines.push(String::new());

    // Public payload boundary
    lines.push("## Public Payload Boundary".to_string());
    lines.push(risk_boundaries.public_payload_boundary.clone());
    lines.push(String::new());

    // Delivery report
    lines.push("## Delivery Report".to_string());
    lines.push("Every task completion must include one copyable Markdown fenced-block delivery report with:".to_string());
    lines.push("- Task status (complete / partially complete / incomplete)".to_string());
    lines.push("- One-line conclusion".to_string());
    lines.push("- Changed files with change summaries".to_string());
    lines.push("- New outputs or artifacts".to_string());
    lines.push("- Deleted files (if any)".to_string());
    lines.push("- Verification results with exact commands".to_string());
    lines.push("- Risk notes".to_string());
    lines.push("- Next steps".to_string());
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(format!(
        "Generated by `ags agent instructions --for {}`",
        agent_type.as_str()
    ));

    lines.join("\n")
}

// ── Session Preflight ───────────────────────────────────────────────────────

/// Overall preflight status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    /// All clear — project is integrated, no failures.
    Ok,
    /// Warnings present but no blocking failures.
    Warning,
    /// Blocking failures — agent should stop before executing.
    Stop,
}

/// Aggregated session preflight report.
///
/// Combines project identity, protocol status, agent instructions, memory
/// paths, stop conditions, warnings, failures, and recommended next steps
/// into a single preflight output for the specified agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPreflight {
    pub target: PathBuf,
    pub for_agent: String,
    pub agent_display_name: String,

    // Project identity (abridged)
    pub integration_status: IntegrationStatus,
    pub is_ags_suite: bool,
    pub is_ags_integrated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_role: Option<WorkspaceIdentity>,
    pub protocol_files_found: Vec<String>,
    pub protocol_files_missing: Vec<String>,
    pub root_entry_files_found: Vec<String>,
    pub root_entry_files_missing: Vec<String>,

    // Protocol status highlights
    pub validator_available: bool,
    pub validator_entry: String,
    pub present_count: usize,
    pub missing_count: usize,

    // Memory paths
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_capsule_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_capsule_exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_memory_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_memory_exists: Option<bool>,

    // Agent instructions summary
    pub should_stop: bool,
    pub stop_conditions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub default_permission_mode: String,

    // Aggregated diagnostics
    pub overall_status: PreflightStatus,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
}

/// Run a complete session preflight for the given agent and target.
///
/// Aggregates project detection, protocol status, agent instructions, and
/// memory path discovery into a single report. This is the kernel activation
/// entry point — it does NOT depend on skill governance or any third-party
/// configuration.
pub fn run_session_preflight(target: &Path, agent_type: &AgentType) -> SessionPreflight {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());

    let identity = detect_project(&canonical);
    let protocol_status = check_protocol_status(&canonical);
    let instructions = generate_agent_instructions(&canonical, agent_type);

    // ── Memory paths ─────────────────────────────────────────────────────
    let capsule_path = identity.memory_capsule_path.clone();
    let capsule_exists = capsule_path.as_ref().map(|p| p.exists());
    let task_mem_path = identity.task_memory_path.clone();
    let task_mem_exists = task_mem_path.as_ref().map(|p| p.exists());

    // ── Aggregate warnings ────────────────────────────────────────────────
    let mut warnings: Vec<String> = protocol_status.warnings.clone();
    // Add protocol warnings from agent instructions
    for w in &instructions.protocol_warnings {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    // Add integration gaps as warnings (non-blocking for suite)
    if identity.is_ags_suite {
        for g in &identity.gaps {
            let msg = format!("Suite gap: {}", g);
            if !warnings.contains(&msg) {
                warnings.push(msg);
            }
        }
    }

    // ── Aggregate failures ────────────────────────────────────────────────
    let mut failures: Vec<String> = protocol_status.failures.clone();
    // Add instruction-level stop reasons as failures (dedup against bare protocol failures)
    for r in &instructions.stop_reasons {
        if !failures.contains(r) {
            failures.push(format!("Agent stop: {}", r));
        }
    }

    // ── Determine overall status ──────────────────────────────────────────
    // Two distinct conditions (critical failures, agent-requested stop) both
    // map to Stop; kept as separate arms for semantic clarity.
    #[allow(clippy::if_same_then_else)]
    let overall_status =
        if !failures.is_empty() && failures.iter().any(|f| f.starts_with("CRITICAL:")) {
            PreflightStatus::Stop
        } else if instructions.should_stop {
            PreflightStatus::Stop
        } else if !warnings.is_empty() {
            PreflightStatus::Warning
        } else {
            PreflightStatus::Ok
        };

    // ── Build next steps ──────────────────────────────────────────────────
    let mut next_steps: Vec<String> = Vec::new();

    match overall_status {
        PreflightStatus::Stop => {
            next_steps.push(
                "⛔ STOP — resolve failures before executing tasks in this repository.".to_string(),
            );
            for f in &failures {
                next_steps.push(format!("  Fix: {}", f));
            }
            if !identity.is_ags_suite && !identity.is_ags_integrated {
                next_steps.push(
                    "  If this target is the intended project repo, run `ags init --target <dir>` to install governance files.".to_string(),
                );
                next_steps.push(
                    "  If this is only a desktop/Cowork workspace, rerun preflight with `target` pointing at the real project repo instead of initializing the current directory.".to_string(),
                );
            }
        }
        PreflightStatus::Warning => {
            next_steps.push("⚠ Proceed with caution — warnings present.".to_string());
            next_steps.push(
                "  Review warnings above and resolve before Heavy/Medium mutation tasks."
                    .to_string(),
            );
            next_steps.push(format!(
                "  {} will execute with permission mode: {}",
                agent_type.display_name(),
                instructions.permissions.default_permission_mode
            ));
            next_steps.push("  Read required protocol files before starting any task.".to_string());
        }
        PreflightStatus::Ok => {
            next_steps.push("✓ All clear — project is fully integrated.".to_string());
            next_steps.push(format!(
                "  {} may execute tasks per AGS governance lifecycle.",
                agent_type.display_name()
            ));
            next_steps.push("  Read required protocol files before starting any task.".to_string());
        }
    }

    // Always suggest reading memory if available
    if capsule_exists == Some(true) {
        next_steps.push("  Read context-capsule.md for project background.".to_string());
    }

    let exit_code = if overall_status == PreflightStatus::Stop {
        1
    } else {
        0
    };

    SessionPreflight {
        target: canonical,
        for_agent: agent_type.as_str().to_string(),
        agent_display_name: agent_type.display_name(),

        integration_status: identity.integration_status.clone(),
        is_ags_suite: identity.is_ags_suite,
        is_ags_integrated: identity.is_ags_integrated,
        inferred_role: identity.inferred_role.clone(),
        protocol_files_found: identity.protocol_files_found,
        protocol_files_missing: identity.protocol_files_missing,
        root_entry_files_found: identity.root_entry_files_found,
        root_entry_files_missing: identity.root_entry_files_missing,

        validator_available: protocol_status.task_card_validator.available,
        validator_entry: protocol_status.task_card_validator.entry.clone(),
        present_count: protocol_status.present_count,
        missing_count: protocol_status.missing_count,

        memory_capsule_path: capsule_path,
        memory_capsule_exists: capsule_exists,
        task_memory_path: task_mem_path,
        task_memory_exists: task_mem_exists,

        should_stop: instructions.should_stop,
        stop_conditions: instructions.stop_conditions.clone(),
        verification_commands: instructions.verification_commands.clone(),
        default_permission_mode: instructions.permissions.default_permission_mode.clone(),

        overall_status,
        warnings,
        failures,
        next_steps,
        exit_code,
    }
}

/// Compute exit code for session preflight: 0 = ok/warning, 1 = stop.
pub fn session_preflight_exit_code(preflight: &SessionPreflight) -> i32 {
    preflight.exit_code
}

// ── Text renderers ─────────────────────────────────────────────────────────

/// Render a `ProjectIdentity` as human-readable text.
pub fn render_project_identity_text(identity: &ProjectIdentity) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Project Identity".to_string());
    lines.push("================".to_string());
    lines.push(format!("Target:       {}", identity.target.display()));
    lines.push(format!("Status:       {:?}", identity.integration_status));
    lines.push(format!("AGS Suite:    {}", identity.is_ags_suite));
    lines.push(format!("AGS Integrated: {}", identity.is_ags_integrated));
    lines.push(String::new());

    if let Some(ref role) = identity.inferred_role {
        lines.push("Inferred Role:".to_string());
        lines.push(format!(
            "  Code: {}  Role: {}  Path: {}",
            role.code, role.role, role.path
        ));
        lines.push(String::new());
    }

    if !identity.workspace_identities.is_empty() {
        lines.push("Workspace Identities:".to_string());
        for ws in &identity.workspace_identities {
            lines.push(format!("  [{}] {} — {}", ws.code, ws.role, ws.path));
        }
        lines.push(String::new());
    }

    if let Some(ref slug) = identity.project_slug {
        lines.push(format!("Project Slug: {}", slug));
    }
    if let Some(ref pp) = identity.project_profile_path {
        lines.push(format!("Profile:      {}", pp.display()));
    }
    if let Some(ref mc) = identity.memory_capsule_path {
        lines.push(format!("Memory Capsule: {}", mc.display()));
    }
    if let Some(ref tm) = identity.task_memory_path {
        lines.push(format!("Task Memory:  {}", tm.display()));
    }
    lines.push(String::new());

    if !identity.root_entry_files_found.is_empty() {
        lines.push("Root Entry Files Found:".to_string());
        for f in &identity.root_entry_files_found {
            lines.push(format!("  ✓ {}", f));
        }
    }
    if !identity.root_entry_files_missing.is_empty() {
        lines.push("Root Entry Files Missing:".to_string());
        for f in &identity.root_entry_files_missing {
            lines.push(format!("  ✗ {}", f));
        }
    }
    lines.push(String::new());

    if !identity.protocol_files_found.is_empty() {
        lines.push("Protocol Files Found:".to_string());
        for f in &identity.protocol_files_found {
            lines.push(format!("  ✓ {}", f));
        }
    }
    if !identity.protocol_files_missing.is_empty() {
        lines.push("Protocol Files Missing:".to_string());
        for f in &identity.protocol_files_missing {
            lines.push(format!("  ✗ {}", f));
        }
    }
    lines.push(String::new());

    if !identity.gaps.is_empty() {
        lines.push("Gaps:".to_string());
        for g in &identity.gaps {
            lines.push(format!("  ! {}", g));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render a `ProtocolStatus` as human-readable text.
pub fn render_protocol_status_text(status: &ProtocolStatus) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Protocol Status".to_string());
    lines.push("===============".to_string());
    lines.push(format!("Target:        {}", status.target.display()));
    lines.push(format!(
        "Protocol Dir:  {} ({})",
        status.protocol_dir.display(),
        if status.protocol_dir_exists {
            "exists"
        } else {
            "missing"
        }
    ));
    lines.push(format!(
        "Files:         {} present / {} missing / {} total",
        status.present_count,
        status.missing_count,
        status.present_count + status.missing_count
    ));
    lines.push(String::new());

    // Protocol files
    lines.push("Protocol Files:".to_string());
    for f in &status.files {
        let marker = if f.present { "✓" } else { "✗" };
        lines.push(format!("  {} {} — {}", marker, f.name, f.description));
    }
    lines.push(String::new());

    // Task-card validator
    lines.push("Task-Card Validator:".to_string());
    lines.push(format!(
        "  Available: {}",
        if status.task_card_validator.available {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(format!("  Entry:     {}", status.task_card_validator.entry));
    if status.task_card_validator.alternate_entry != "N/A" {
        lines.push(format!(
            "  Alternate: {}",
            status.task_card_validator.alternate_entry
        ));
    }
    lines.push(String::new());

    // Risk boundaries
    lines.push("Risk Boundaries:".to_string());
    lines.push("  Protected Paths:".to_string());
    for p in &status.risk_boundaries.protected_paths {
        lines.push(format!("    - {}", p));
    }
    lines.push("  High-Risk Indicators:".to_string());
    for r in &status.risk_boundaries.high_risk_indicators {
        lines.push(format!("    - {}", r));
    }
    lines.push(format!(
        "  Destructive Actions Require Confirmation: {}",
        status
            .risk_boundaries
            .destructive_actions_require_confirmation
    ));
    lines.push(String::new());

    // Review requirements
    lines.push("Review Requirements:".to_string());
    lines.push(format!("  Light:  {}", status.review_requirements.light));
    lines.push(format!("  Medium: {}", status.review_requirements.medium));
    lines.push(format!("  Heavy:  {}", status.review_requirements.heavy));
    lines.push(String::new());

    // Verify requirements
    lines.push("Verify Requirements:".to_string());
    for v in &status.verify_requirements {
        lines.push(format!("  - {}", v));
    }
    lines.push(String::new());

    // Receipt requirements
    lines.push("Receipt Requirements:".to_string());
    lines.push(format!(
        "  Delivery Report: {}",
        if status.receipt_requirements.delivery_report_required {
            "required"
        } else {
            "not required"
        }
    ));
    lines.push(format!(
        "  Archive: {}",
        status.receipt_requirements.archive_location
    ));
    lines.push(String::new());

    // Failures
    if !status.failures.is_empty() {
        lines.push("FAILURES:".to_string());
        for f in &status.failures {
            lines.push(format!("  ✗ {}", f));
        }
        lines.push(String::new());
    }

    // Warnings
    if !status.warnings.is_empty() {
        lines.push("Warnings:".to_string());
        for w in &status.warnings {
            lines.push(format!("  ! {}", w));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render `AgentInstructions` as human-readable text.
pub fn render_agent_instructions_text(instructions: &AgentInstructions) -> String {
    // instructions_text is already the full text block; return it directly
    instructions.instructions_text.clone()
}

/// Render a `SessionPreflight` as human-readable text.
pub fn render_session_preflight_text(preflight: &SessionPreflight) -> String {
    let mut lines: Vec<String> = Vec::new();

    // ── Header ────────────────────────────────────────────────────────────
    lines.push(format!(
        "Session Preflight — for {}",
        preflight.agent_display_name
    ));
    lines.push("=".repeat(60));
    lines.push(format!("Target:     {}", preflight.target.display()));
    lines.push(format!(
        "Agent:      {} ({})",
        preflight.agent_display_name, preflight.for_agent
    ));
    lines.push(String::new());

    // ── Project Identity ──────────────────────────────────────────────────
    lines.push("── Project Identity ──".to_string());
    lines.push(format!(
        "Status:         {:?}",
        preflight.integration_status
    ));
    lines.push(format!("AGS Suite:      {}", preflight.is_ags_suite));
    lines.push(format!("AGS Integrated: {}", preflight.is_ags_integrated));
    if let Some(ref role) = preflight.inferred_role {
        lines.push(format!(
            "Inferred Role:  [{}] {} — {}",
            role.code, role.role, role.path
        ));
    }
    lines.push(String::new());

    // ── Protocol Status ───────────────────────────────────────────────────
    lines.push("── Protocol Status ──".to_string());
    lines.push(format!(
        "Files:          {} present / {} missing / {} total",
        preflight.present_count,
        preflight.missing_count,
        preflight.present_count + preflight.missing_count
    ));
    lines.push(format!(
        "Validator:      {}",
        if preflight.validator_available {
            "available"
        } else {
            "unavailable"
        }
    ));
    if preflight.validator_available {
        lines.push(format!("  Entry:  {}", preflight.validator_entry));
    }
    if !preflight.protocol_files_missing.is_empty() {
        lines.push("Missing protocol files:".to_string());
        for f in &preflight.protocol_files_missing {
            lines.push(format!("  ✗ {}", f));
        }
    }
    if !preflight.root_entry_files_missing.is_empty() {
        lines.push("Missing root entry files:".to_string());
        for f in &preflight.root_entry_files_missing {
            lines.push(format!("  ✗ {}", f));
        }
    }
    lines.push(String::new());

    // ── Memory Paths ──────────────────────────────────────────────────────
    lines.push("── Memory Paths ──".to_string());
    if let Some(ref path) = preflight.memory_capsule_path {
        let marker = if preflight.memory_capsule_exists == Some(true) {
            "✓"
        } else {
            "✗"
        };
        lines.push(format!("Context Capsule: {} {}", marker, path.display()));
    } else {
        lines.push("Context Capsule: (not detected)".to_string());
    }
    if let Some(ref path) = preflight.task_memory_path {
        let marker = if preflight.task_memory_exists == Some(true) {
            "✓"
        } else {
            "✗"
        };
        lines.push(format!("Task Memory:     {} {}", marker, path.display()));
    } else {
        lines.push("Task Memory:     (not detected)".to_string());
    }
    lines.push(String::new());

    // ── Stop Conditions ──────────────────────────────────────────────────
    lines.push("── Stop Conditions ──".to_string());
    if preflight.stop_conditions.is_empty() {
        lines.push("  (none — project-specific stop conditions not enumerated)".to_string());
    } else {
        for (i, s) in preflight.stop_conditions.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, s));
        }
    }
    lines.push(String::new());

    // ── Warnings ─────────────────────────────────────────────────────────
    if !preflight.warnings.is_empty() {
        lines.push("── Warnings ──".to_string());
        for w in &preflight.warnings {
            lines.push(format!("  ! {}", w));
        }
        lines.push(String::new());
    }

    // ── Failures ─────────────────────────────────────────────────────────
    if !preflight.failures.is_empty() {
        lines.push("── FAILURES ──".to_string());
        for f in &preflight.failures {
            lines.push(format!("  ✗ {}", f));
        }
        lines.push(String::new());
    }

    // ── Verification Commands ─────────────────────────────────────────────
    lines.push("── Verification Commands ──".to_string());
    for cmd in &preflight.verification_commands {
        lines.push(format!("  - `{}`", cmd));
    }
    lines.push(String::new());

    // ── Next Steps ───────────────────────────────────────────────────────
    lines.push("── Next Steps ──".to_string());
    for step in &preflight.next_steps {
        lines.push(step.clone());
    }
    lines.push(String::new());

    // ── Overall Status ───────────────────────────────────────────────────
    lines.push("── Overall ──".to_string());
    match preflight.overall_status {
        PreflightStatus::Ok => lines.push("Status: OK — all clear".to_string()),
        PreflightStatus::Warning => {
            lines.push("Status: WARNING — proceed with caution".to_string())
        }
        PreflightStatus::Stop => lines.push("Status: STOP — resolve failures first".to_string()),
    }
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(format!(
        "Generated by `ags session preflight --for {}`",
        preflight.for_agent
    ));

    lines.join("\n")
}

// ── JSON renderers ─────────────────────────────────────────────────────────

/// Render a value as pretty-printed JSON.
pub fn render_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("JSON error: {}", e))
}

// ── Exit code helpers ──────────────────────────────────────────────────────

/// Compute exit code for protocol status: 0 = clean, 1 = failures present.
pub fn protocol_status_exit_code(status: &ProtocolStatus) -> i32 {
    if status.failures.is_empty() {
        0
    } else {
        1
    }
}

/// Compute exit code for project detect: 0 = suite/integrated, 1 = partial/not-integrated.
pub fn project_detect_exit_code(identity: &ProjectIdentity) -> i32 {
    match identity.integration_status {
        IntegrationStatus::Suite | IntegrationStatus::Integrated => 0,
        IntegrationStatus::Partial | IntegrationStatus::NotIntegrated => 1,
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Return the repo root path (two levels up from the crate directory).
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
    }

    fn expected_known_role_for_root(root: &Path) -> Option<(&'static str, &'static str)> {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let canonical_str = canonical.to_string_lossy();
        KNOWN_WORKSPACE_PATHS
            .iter()
            .find(|(_, _, path)| *path == canonical_str)
            .map(|(code, role, _)| (*code, *role))
    }

    // ── Workspace table parsing ────────────────────────────────────────

    #[test]
    fn test_parse_workspace_table_standard() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Development private suite | /Volumes/Projects/example-private-suite |
| A1 | Private bare repo | /Volumes/Projects/remotes/example-private-suite.git |
| S | Stable private suite | /Volumes/Projects/example-stable-suite |
| B | Public worktree | /Volumes/AI Project/ai-dev-env-bootstrap |
| B1 | Public bare repo | /Volumes/Projects/remotes/example-public-suite.git |
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 5);
        assert_eq!(identities[0].code, "A");
        assert_eq!(identities[0].role, "Development private suite");
        assert_eq!(
            identities[0].path,
            "/Volumes/Projects/example-private-suite"
        );
        assert_eq!(identities[4].code, "B1");
    }

    #[test]
    fn test_parse_workspace_table_blank_line_terminates() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /path/to/a |

Some other text here.
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].code, "A");
    }

    #[test]
    fn test_parse_workspace_table_non_table_line_terminates() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /path/to/a |
## Next Section
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 1);
    }

    #[test]
    fn test_parse_workspace_table_empty() {
        let identities = parse_workspace_table("No table here.");
        assert_eq!(identities.len(), 0);
    }

    #[test]
    fn test_parse_workspace_table_no_header_sep() {
        let content = "\
| Code | Role | Path |
| A | Dev | /path |
";
        let identities = parse_workspace_table(content);
        // No header separator, so no rows parsed
        assert_eq!(identities.len(), 0);
    }

    // ── AgentType parsing ──────────────────────────────────────────────

    #[test]
    fn test_agent_type_from_str_valid() {
        assert_eq!(AgentType::from_str("codex").unwrap(), AgentType::Codex);
        assert_eq!(
            AgentType::from_str("claude-code").unwrap(),
            AgentType::ClaudeCode
        );
        assert_eq!(AgentType::from_str("cursor").unwrap(), AgentType::Cursor);
        assert_eq!(
            AgentType::from_str("workbuddy").unwrap(),
            AgentType::Generic("workbuddy".to_string())
        );
        // Tencent Agent host clients normalize to recognized generic ids
        // (no new canonical AgentType variant; casing/spacing folded by
        // normalize_agent_id).
        assert_eq!(
            AgentType::from_str("WorkBuddy").unwrap(),
            AgentType::Generic("workbuddy".to_string())
        );
        assert_eq!(
            AgentType::from_str("CodeBuddy-Code").unwrap(),
            AgentType::Generic("codebuddy-code".to_string())
        );
        assert_eq!(
            AgentType::from_str("Tencent Agent").unwrap(),
            AgentType::Generic("tencent-agent".to_string())
        );
        assert_eq!(
            AgentType::from_str("Claude Desktop Cowork").unwrap(),
            AgentType::Generic("claude-desktop-cowork".to_string())
        );
    }

    #[test]
    fn test_agent_type_from_str_invalid() {
        assert!(AgentType::from_str("").is_err());
        assert!(AgentType::from_str("   ").is_err());
    }

    #[test]
    fn test_agent_type_display_name() {
        assert_eq!(AgentType::Codex.display_name(), "Codex");
        assert_eq!(AgentType::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(AgentType::Cursor.display_name(), "Cursor");
        // Tencent Agent host family gets branded display names while still
        // carried as Generic (no new AgentType variant).
        assert_eq!(
            AgentType::Generic("workbuddy".to_string()).display_name(),
            "Tencent Agent (WorkBuddy)"
        );
        assert_eq!(
            AgentType::Generic("codebuddy-code".to_string()).display_name(),
            "Tencent Agent (CodeBuddy-Code)"
        );
        assert_eq!(
            AgentType::Generic("tencent-agent".to_string()).display_name(),
            "Tencent Agent"
        );
        // Unknown hosts keep the plain generic fallback — not broken.
        assert_eq!(
            AgentType::Generic("claude-desktop-cowork".to_string()).display_name(),
            "Generic Agent (claude-desktop-cowork)"
        );
    }

    #[test]
    fn test_agent_type_as_str() {
        assert_eq!(AgentType::Codex.as_str(), "codex");
        assert_eq!(AgentType::ClaudeCode.as_str(), "claude-code");
        assert_eq!(AgentType::Cursor.as_str(), "cursor");
        assert_eq!(
            AgentType::Generic("workbuddy".to_string()).as_str(),
            "workbuddy"
        );
    }

    // ── Agent type serde ───────────────────────────────────────────────

    #[test]
    fn test_agent_type_serialize() {
        assert_eq!(
            serde_json::to_string(&AgentType::Codex).unwrap(),
            "\"codex\""
        );
        assert_eq!(
            serde_json::to_string(&AgentType::ClaudeCode).unwrap(),
            "\"claude-code\""
        );
        assert_eq!(
            serde_json::to_string(&AgentType::Cursor).unwrap(),
            "\"cursor\""
        );
        assert_eq!(
            serde_json::to_string(&AgentType::Generic("workbuddy".to_string())).unwrap(),
            "\"workbuddy\""
        );
    }

    #[test]
    fn test_agent_type_deserialize() {
        let a: AgentType = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(a, AgentType::Codex);
        let a: AgentType = serde_json::from_str("\"claude-code\"").unwrap();
        assert_eq!(a, AgentType::ClaudeCode);
        let a: AgentType = serde_json::from_str("\"cursor\"").unwrap();
        assert_eq!(a, AgentType::Cursor);
        let a: AgentType = serde_json::from_str("\"workbuddy\"").unwrap();
        assert_eq!(a, AgentType::Generic("workbuddy".to_string()));
    }

    // ── IntegrationStatus serde ────────────────────────────────────────

    #[test]
    fn test_integration_status_serde() {
        assert_eq!(
            serde_json::to_string(&IntegrationStatus::Suite).unwrap(),
            "\"suite\""
        );
        assert_eq!(
            serde_json::to_string(&IntegrationStatus::Integrated).unwrap(),
            "\"integrated\""
        );
        assert_eq!(
            serde_json::to_string(&IntegrationStatus::NotIntegrated).unwrap(),
            "\"not_integrated\""
        );
        assert_eq!(
            serde_json::to_string(&IntegrationStatus::Partial).unwrap(),
            "\"partial\""
        );
    }

    // ── Project detection against the real AGS repo ────────────────────

    #[test]
    fn test_detect_ags_suite_repo() {
        let root = repo_root();
        let identity = detect_project(&root);
        assert!(
            identity.is_ags_suite,
            "Running from AGS repo — should detect as suite"
        );
        assert_eq!(identity.integration_status, IntegrationStatus::Suite);
        // Public edition WORKSPACE.md may be a public role map rather than the
        // private A/A1/S/B/B1 maintainer topology. Project detection must not
        // require machine-local workspace identities to classify the suite.
        // Should have found root entry files
        assert!(identity
            .root_entry_files_found
            .contains(&"AGENTS.md".to_string()));
        assert!(identity
            .root_entry_files_found
            .contains(&"CLAUDE.md".to_string()));
        // The development private suite has local memory; the stable suite may
        // not, but both are valid AGS suite roots.
        if matches!(
            identity
                .inferred_role
                .as_ref()
                .map(|role| role.code.as_str()),
            Some("A")
        ) {
            assert!(identity.memory_capsule_path.is_some());
        }
    }

    #[test]
    fn test_detect_temp_dir_not_integrated() {
        let tmp = std::env::temp_dir().join("ags-test-not-integrated");
        let _ = std::fs::create_dir_all(&tmp);
        let identity = detect_project(&tmp);
        assert!(!identity.is_ags_suite);
        assert!(!identity.is_ags_integrated);
        assert_eq!(
            identity.integration_status,
            IntegrationStatus::NotIntegrated
        );
        assert!(!identity.gaps.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_detect_nonexistent_target() {
        let identity = detect_project(Path::new("/tmp/ags-nonexistent-XXXXXX"));
        assert!(!identity.is_ags_suite);
        assert!(!identity.is_ags_integrated);
        assert_eq!(
            identity.integration_status,
            IntegrationStatus::NotIntegrated
        );
    }

    #[test]
    fn test_detect_project_json_output() {
        let identity = detect_project(&repo_root());
        let json = render_json(&identity);
        assert!(json.contains("\"target\""));
        assert!(json.contains("\"integration_status\""));
        assert!(json.contains("\"is_ags_suite\""));
        // Verify parseable
        let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    }

    // ── Protocol status tests ──────────────────────────────────────────

    #[test]
    fn test_protocol_status_ags_repo() {
        let root = repo_root();
        let status = check_protocol_status(&root);
        // Running from AGS repo — most protocol files should be present
        assert!(status.present_count > 5);
        assert!(status.protocol_dir_exists);
        assert!(status.task_card_validator.available);
        // Should have no critical failures in our own repo
        let critical_failures: Vec<_> = status
            .failures
            .iter()
            .filter(|f| f.starts_with("CRITICAL:"))
            .collect();
        assert!(
            critical_failures.is_empty(),
            "AGS repo should have no critical failures: {:?}",
            critical_failures
        );
    }

    #[test]
    fn test_protocol_status_temp_dir() {
        let tmp = std::env::temp_dir().join("ags-test-protocol-status");
        let _ = std::fs::create_dir_all(&tmp);
        let status = check_protocol_status(&tmp);
        assert!(!status.protocol_dir_exists);
        assert!(status.present_count == 0);
        assert!(!status.task_card_validator.available);
        assert!(!status.failures.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_protocol_status_json_output() {
        let status = check_protocol_status(&repo_root());
        let json = render_json(&status);
        assert!(json.contains("\"target\""));
        assert!(json.contains("\"files\""));
        assert!(json.contains("\"present_count\""));
        assert!(json.contains("\"task_card_validator\""));
        // Verify parseable
        let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    }

    #[test]
    fn test_protocol_status_exit_code_clean() {
        let status = check_protocol_status(&repo_root());
        let code = protocol_status_exit_code(&status);
        // Running from AGS repo — should be clean (0)
        assert_eq!(code, 0, "AGS repo should have exit code 0");
    }

    #[test]
    fn test_protocol_status_exit_code_dirty() {
        let tmp = std::env::temp_dir().join("ags-test-exit-code");
        let _ = std::fs::create_dir_all(&tmp);
        let status = check_protocol_status(&tmp);
        let code = protocol_status_exit_code(&status);
        assert_eq!(code, 1, "Non-AGS repo should have exit code 1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Project detect exit code ───────────────────────────────────────

    #[test]
    fn test_project_detect_exit_code_suite() {
        let identity = detect_project(&repo_root());
        let code = project_detect_exit_code(&identity);
        assert_eq!(code, 0, "Suite repo should have exit code 0");
    }

    #[test]
    fn test_project_detect_exit_code_not_integrated() {
        let tmp = std::env::temp_dir().join("ags-test-exit-code-2");
        let _ = std::fs::create_dir_all(&tmp);
        let identity = detect_project(&tmp);
        let code = project_detect_exit_code(&identity);
        assert_eq!(code, 1, "Non-integrated repo should have exit code 1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Agent instructions tests ───────────────────────────────────────

    #[test]
    fn test_generate_instructions_codex() {
        let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
        assert_eq!(instructions.agent_type, "codex");
        assert_eq!(instructions.agent_display_name, "Codex");
        assert_eq!(
            instructions.permissions.default_permission_mode,
            "execute-and-verify"
        );
        assert!(instructions.is_ags_suite);
        assert!(!instructions.required_reads.is_empty());
        assert!(instructions.instructions_text.contains("## Required Reads"));
        assert!(instructions
            .instructions_text
            .contains("## Stop Conditions"));
        assert!(instructions
            .instructions_text
            .contains("## Verification Commands"));
    }

    #[test]
    fn test_generate_instructions_claude_code() {
        let root = repo_root();
        let instructions = generate_agent_instructions(&root, &AgentType::ClaudeCode);
        assert_eq!(instructions.agent_type, "claude-code");
        assert_eq!(instructions.agent_display_name, "Claude Code");
        assert_eq!(
            instructions.permissions.default_permission_mode,
            "execute-and-verify"
        );
        assert!(!instructions.required_reads.is_empty());
        assert!(instructions
            .instructions_text
            .contains("Claude Code executes bounded task cards"));
    }

    #[test]
    fn test_generate_instructions_cursor() {
        let instructions = generate_agent_instructions(&repo_root(), &AgentType::Cursor);
        assert_eq!(instructions.agent_type, "cursor");
        assert_eq!(instructions.agent_display_name, "Cursor");
        assert_eq!(
            instructions.permissions.default_permission_mode,
            "execute-and-verify"
        );
        assert!(!instructions.required_reads.is_empty());
        assert!(instructions
            .instructions_text
            .contains("Cursor owns the full pre-execution lifecycle"));
    }

    #[test]
    fn test_generate_instructions_generic_agent() {
        let instructions =
            generate_agent_instructions(&repo_root(), &AgentType::Generic("workbuddy".to_string()));
        assert_eq!(instructions.agent_type, "workbuddy");
        assert_eq!(instructions.agent_display_name, "Tencent Agent (WorkBuddy)");
        assert_eq!(
            instructions.permissions.default_permission_mode,
            "execute-and-verify"
        );
        assert!(instructions
            .instructions_text
            .contains("AGS-compatible governed host"));
    }

    #[test]
    fn test_generate_instructions_codebuddy_code() {
        let instructions = generate_agent_instructions(
            &repo_root(),
            &AgentType::Generic("codebuddy-code".to_string()),
        );
        assert_eq!(instructions.agent_type, "codebuddy-code");
        assert_eq!(
            instructions.agent_display_name,
            "Tencent Agent (CodeBuddy-Code)"
        );
        // Recognized Tencent Agent clients keep the governed-host permission
        // profile (no elevated privileges from the name).
        assert_eq!(
            instructions.permissions.default_permission_mode,
            "execute-and-verify"
        );
    }

    #[test]
    fn test_agent_instructions_json_output() {
        let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
        let json = render_json(&instructions);
        assert!(json.contains("\"agent_type\""));
        assert!(json.contains("\"required_reads\""));
        assert!(json.contains("\"stop_conditions\""));
        // Verify parseable
        let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    }

    #[test]
    fn test_agent_instructions_non_ags_repo() {
        let tmp = std::env::temp_dir().join("ags-test-agent-instructions");
        let _ = std::fs::create_dir_all(&tmp);
        let instructions = generate_agent_instructions(&tmp, &AgentType::ClaudeCode);
        assert!(!instructions.is_ags_suite);
        assert_eq!(
            instructions.integration_status,
            IntegrationStatus::NotIntegrated
        );
        // Should still generate valid instructions
        assert!(!instructions.instructions_text.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Text renderers ─────────────────────────────────────────────────

    #[test]
    fn test_render_project_identity_text() {
        let identity = detect_project(&repo_root());
        let text = render_project_identity_text(&identity);
        assert!(text.contains("Project Identity"));
        assert!(text.contains("AGS Suite:"));
        assert!(text.contains("Workspace Identities:"));
    }

    #[test]
    fn test_render_protocol_status_text() {
        let status = check_protocol_status(&repo_root());
        let text = render_protocol_status_text(&status);
        assert!(text.contains("Protocol Status"));
        assert!(text.contains("Task-Card Validator:"));
        assert!(text.contains("Risk Boundaries:"));
        assert!(text.contains("Review Requirements:"));
    }

    #[test]
    fn test_render_agent_instructions_text() {
        let instructions = generate_agent_instructions(&repo_root(), &AgentType::Codex);
        let text = render_agent_instructions_text(&instructions);
        assert!(text.contains("# Agent Governance Instructions"));
        assert!(text.contains("## Role"));
        assert!(text.contains("## Required Reads"));
    }

    // ── Slug derivation ────────────────────────────────────────────────

    #[test]
    fn test_slug_from_path() {
        assert_eq!(
            slug_from_path(Path::new("/foo/bar/my-project")),
            "my-project"
        );
        assert_eq!(slug_from_path(Path::new("/foo/bar")), "bar");
    }

    // ── Adversarial: backtick stripping in WORKSPACE.md paths ──────────

    #[test]
    fn test_workspace_table_strips_backticks_from_paths() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Dev suite | `/Volumes/Projects/example-private-suite` |
| S | Stable | `/Volumes/Projects/example-stable-suite` |
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 2);
        assert_eq!(
            identities[0].path,
            "/Volumes/Projects/example-private-suite"
        );
        assert_eq!(identities[1].path, "/Volumes/Projects/example-stable-suite");
        // Verify no backticks remain
        assert!(!identities[0].path.contains('`'));
        assert!(!identities[1].path.contains('`'));
    }

    #[test]
    fn test_workspace_table_strips_backticks_with_whitespace() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | `  /path/with spaces  ` |
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].path, "/path/with spaces");
        assert!(!identities[0].path.contains('`'));
    }

    #[test]
    fn test_workspace_table_path_without_backticks_unchanged() {
        let content = "\
| Code | Role | Path |
|---|---|
| A | Dev | /plain/path |
";
        let identities = parse_workspace_table(content);
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].path, "/plain/path");
    }

    // ── Adversarial: is_ags_integrated consistency ─────────────────────

    #[test]
    fn test_is_ags_integrated_consistent_with_status() {
        let tmp = std::env::temp_dir().join("ags-test-integrated-consistency");
        let _ = std::fs::create_dir_all(&tmp);

        // Empty repo: not integrated
        let identity = detect_project(&tmp);
        assert_eq!(
            identity.integration_status,
            IntegrationStatus::NotIntegrated
        );
        assert!(!identity.is_ags_integrated);
        assert!(!identity.is_ags_suite);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_suite_has_is_ags_integrated_true() {
        let root = repo_root();
        let identity = detect_project(&root);
        assert_eq!(identity.integration_status, IntegrationStatus::Suite);
        assert!(identity.is_ags_suite);
        assert!(
            identity.is_ags_integrated,
            "Suite must have is_ags_integrated=true"
        );
    }

    // ── Adversarial: agent instructions for non-integrated repos ───────

    #[test]
    fn test_agent_instructions_non_integrated_should_stop() {
        let tmp = std::env::temp_dir().join("ags-test-agent-stop");
        let _ = std::fs::create_dir_all(&tmp);

        let instructions = generate_agent_instructions(&tmp, &AgentType::ClaudeCode);
        assert!(
            instructions.should_stop,
            "Non-integrated repo must set should_stop=true"
        );
        assert!(!instructions.stop_reasons.is_empty());
        assert!(instructions.exit_code != 0);
        // Must contain STOP banner in text
        assert!(instructions.instructions_text.contains("⛔ STOP"));
        assert!(instructions.instructions_text.contains("DO NOT EXECUTE"));
        // Must contain gaps
        assert!(!instructions.integration_gaps.is_empty());
        // Must contain protocol failures
        assert!(!instructions.protocol_failures.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_agent_instructions_suite_does_not_stop() {
        let root = repo_root();
        let instructions = generate_agent_instructions(&root, &AgentType::ClaudeCode);
        assert!(
            !instructions.should_stop,
            "Suite repo must have should_stop=false"
        );
        assert!(instructions.stop_reasons.is_empty());
        assert_eq!(instructions.exit_code, 0);
        // Must NOT contain STOP banner
        assert!(!instructions.instructions_text.contains("⛔ STOP"));
    }

    #[test]
    fn test_agent_instructions_non_integrated_json_has_stop_fields() {
        let tmp = std::env::temp_dir().join("ags-test-agent-json-stop");
        let _ = std::fs::create_dir_all(&tmp);

        let instructions = generate_agent_instructions(&tmp, &AgentType::Codex);
        let json = render_json(&instructions);
        assert!(json.contains("\"should_stop\": true"));
        assert!(json.contains("\"exit_code\": 1"));
        assert!(json.contains("\"stop_reasons\""));
        assert!(json.contains("\"integration_gaps\""));
        assert!(json.contains("\"protocol_failures\""));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Adversarial: target-aware verification commands ─────────────────

    #[test]
    fn test_verification_commands_rust_project() {
        let tmp = std::env::temp_dir().join("ags-test-verify-rust");
        let _ = std::fs::create_dir_all(&tmp);
        // Create a fake Cargo.toml
        std::fs::write(tmp.join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();

        let commands = detect_verification_commands(&tmp);
        assert!(commands.iter().any(|c| c.contains("cargo fmt")));
        assert!(commands.iter().any(|c| c.contains("cargo test")));
        assert!(commands.iter().any(|c| c.contains("cargo build")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_verification_commands_with_verify_sh() {
        let tmp = std::env::temp_dir().join("ags-test-verify-sh");
        let _ = std::fs::create_dir_all(&tmp);
        let scripts_dir = tmp.join("scripts");
        let _ = std::fs::create_dir_all(&scripts_dir);
        std::fs::write(scripts_dir.join("verify.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let commands = detect_verification_commands(&tmp);
        assert!(commands.iter().any(|c| c.contains("verify.sh")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_verification_commands_empty_project() {
        let tmp = std::env::temp_dir().join("ags-test-verify-empty");
        let _ = std::fs::create_dir_all(&tmp);

        let commands = detect_verification_commands(&tmp);
        // Should return guidance, not false commands
        assert!(!commands.is_empty());
        assert!(!commands.iter().any(|c| c.contains("cargo")));
        assert!(commands.iter().any(|c| c.contains("No project-specific")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_protocol_status_uses_target_aware_verify_commands() {
        let root = repo_root();
        let status = check_protocol_status(&root);
        // AGS repo has Cargo.toml and scripts/verify.sh
        assert!(status
            .verify_requirements
            .iter()
            .any(|c| c.contains("cargo fmt")));
        assert!(status
            .verify_requirements
            .iter()
            .any(|c| c.contains("verify.sh")));
    }

    #[test]
    fn test_protocol_status_empty_repo_verify_commands_guidance() {
        let tmp = std::env::temp_dir().join("ags-test-protocol-verify");
        let _ = std::fs::create_dir_all(&tmp);

        let status = check_protocol_status(&tmp);
        // Empty repo — should give guidance, not false cargo commands
        assert!(!status
            .verify_requirements
            .iter()
            .any(|c| c.contains("cargo")));
        assert!(status
            .verify_requirements
            .iter()
            .any(|c| c.contains("No project-specific")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_agent_instructions_target_aware_verify_commands() {
        let root = repo_root();
        let instructions = generate_agent_instructions(&root, &AgentType::Codex);
        assert!(instructions
            .verification_commands
            .iter()
            .any(|c| c.contains("cargo fmt")));
        assert!(instructions
            .verification_commands
            .iter()
            .any(|c| c.contains("verify.sh")));
    }

    // ── Known workspace path detection ─────────────────────────────────

    #[test]
    fn test_known_workspace_path_detection() {
        let root = repo_root();
        let expected = match expected_known_role_for_root(&root) {
            Some(e) => e,
            None => return, // CI or non-maintainer machine — path not in KNOWN_WORKSPACE_PATHS
        };
        let identity = detect_project(&root);
        assert!(
            identity.inferred_role.is_some(),
            "Running from AGS repo — should infer role"
        );
        if let Some(ref role) = identity.inferred_role {
            assert_eq!(role.code, expected.0);
            assert_eq!(role.role, expected.1);
        }
    }

    // ── Session Preflight tests ───────────────────────────────────────

    #[test]
    fn test_session_preflight_codex() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Codex);
        assert_eq!(preflight.for_agent, "codex");
        assert_eq!(preflight.agent_display_name, "Codex");
        assert!(preflight.is_ags_suite);
        assert_eq!(preflight.integration_status, IntegrationStatus::Suite);
        assert!(preflight.validator_available);
        assert!(!preflight.stop_conditions.is_empty());
        assert!(!preflight.verification_commands.is_empty());
        // Suite repo should be OK or Warning (not Stop)
        assert_ne!(preflight.overall_status, PreflightStatus::Stop);
        assert_eq!(preflight.exit_code, 0);
    }

    #[test]
    fn test_session_preflight_claude_code() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::ClaudeCode);
        assert_eq!(preflight.for_agent, "claude-code");
        assert_eq!(preflight.agent_display_name, "Claude Code");
        assert!(preflight.is_ags_suite);
        assert!(preflight.validator_available);
        assert_eq!(preflight.default_permission_mode, "execute-and-verify");
        assert_ne!(preflight.overall_status, PreflightStatus::Stop);
    }

    #[test]
    fn test_session_preflight_cursor() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Cursor);
        assert_eq!(preflight.for_agent, "cursor");
        assert_eq!(preflight.agent_display_name, "Cursor");
        assert!(preflight.is_ags_suite);
    }

    #[test]
    fn test_session_preflight_generic_workbuddy() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::from_str("workbuddy").unwrap());
        assert_eq!(preflight.for_agent, "workbuddy");
        assert_eq!(preflight.agent_display_name, "Tencent Agent (WorkBuddy)");
        assert!(preflight.is_ags_suite);
        assert_ne!(preflight.overall_status, PreflightStatus::Stop);
    }

    #[test]
    fn test_session_preflight_tencent_agent_clients() {
        let root = repo_root();
        // CodeBuddy-Code resolves and triggers preflight with branded display.
        let cb = run_session_preflight(&root, &AgentType::from_str("CodeBuddy-Code").unwrap());
        assert_eq!(cb.for_agent, "codebuddy-code");
        assert_eq!(cb.agent_display_name, "Tencent Agent (CodeBuddy-Code)");
        assert!(cb.is_ags_suite);
        assert_ne!(cb.overall_status, PreflightStatus::Stop);
        // The Tencent Agent umbrella id itself resolves.
        let ta = run_session_preflight(&root, &AgentType::from_str("Tencent Agent").unwrap());
        assert_eq!(ta.for_agent, "tencent-agent");
        assert_eq!(ta.agent_display_name, "Tencent Agent");
    }

    #[test]
    fn test_session_preflight_non_integrated() {
        let tmp = std::env::temp_dir().join("ags-test-preflight-non-integrated");
        let _ = std::fs::create_dir_all(&tmp);

        let preflight = run_session_preflight(&tmp, &AgentType::ClaudeCode);
        assert!(!preflight.is_ags_suite);
        assert!(!preflight.is_ags_integrated);
        assert_eq!(preflight.overall_status, PreflightStatus::Stop);
        assert!(preflight.should_stop);
        assert!(!preflight.failures.is_empty());
        assert_eq!(preflight.exit_code, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_session_preflight_json_output() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Codex);
        let json = render_json(&preflight);
        assert!(json.contains("\"target\""));
        assert!(json.contains("\"for_agent\""));
        assert!(json.contains("\"integration_status\""));
        assert!(json.contains("\"overall_status\""));
        assert!(json.contains("\"stop_conditions\""));
        assert!(json.contains("\"warnings\""));
        assert!(json.contains("\"failures\""));
        assert!(json.contains("\"next_steps\""));
        assert!(json.contains("\"exit_code\""));
        // Verify parseable
        let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    }

    #[test]
    fn test_session_preflight_text_output() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Codex);
        let text = render_session_preflight_text(&preflight);
        assert!(text.contains("Session Preflight"));
        assert!(text.contains("Project Identity"));
        assert!(text.contains("Protocol Status"));
        assert!(text.contains("Memory Paths"));
        assert!(text.contains("Stop Conditions"));
        assert!(text.contains("Next Steps"));
        assert!(text.contains("Overall"));
    }

    #[test]
    fn test_session_preflight_has_memory_paths() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Codex);
        // AGS suite repo may or may not have memory depending on role
        // At minimum, the fields should be populated for an A repo
        let inferred_is_a = preflight
            .inferred_role
            .as_ref()
            .map(|r| r.code == "A")
            .unwrap_or(false);
        if inferred_is_a {
            assert!(preflight.memory_capsule_path.is_some());
            assert!(preflight.memory_capsule_exists == Some(true));
        }
    }

    #[test]
    fn test_session_preflight_exit_code_ok() {
        let root = repo_root();
        let preflight = run_session_preflight(&root, &AgentType::Codex);
        let code = session_preflight_exit_code(&preflight);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_session_preflight_exit_code_stop() {
        let tmp = std::env::temp_dir().join("ags-test-preflight-exit-stop");
        let _ = std::fs::create_dir_all(&tmp);

        let preflight = run_session_preflight(&tmp, &AgentType::ClaudeCode);
        let code = session_preflight_exit_code(&preflight);
        assert_eq!(code, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_session_preflight_independent_of_skill_governance() {
        // session preflight must work even when governance/ and
        // protocol/skill-governance.md are not present
        let tmp = std::env::temp_dir().join("ags-test-preflight-no-skills");
        let _ = std::fs::create_dir_all(&tmp);
        // Create minimal AGS markers
        std::fs::write(tmp.join("AGENTS.md"), "# AGENTS\n@CLAUDE.md\n").unwrap();
        std::fs::write(tmp.join("CLAUDE.md"), "# CLAUDE\n").unwrap();

        let preflight = run_session_preflight(&tmp, &AgentType::Codex);
        // Must not panic, must produce valid output
        assert!(!preflight.for_agent.is_empty());
        let text = render_session_preflight_text(&preflight);
        assert!(!text.is_empty());
        let json = render_json(&preflight);
        assert!(json.contains("for_agent"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_preflight_status_serde() {
        assert_eq!(
            serde_json::to_string(&PreflightStatus::Ok).unwrap(),
            "\"ok\""
        );
        assert_eq!(
            serde_json::to_string(&PreflightStatus::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(
            serde_json::to_string(&PreflightStatus::Stop).unwrap(),
            "\"stop\""
        );
    }
}
