use super::workspace_facts::*;
use super::*;
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
/// Scans for project-specific tooling and returns Rust-kernel verification
/// commands. First-party shell verifier implementations are not supported.
/// and returns the list of verification commands that apply to the target.
/// If no project-specific tooling is found, returns guidance rather than
/// false commands.
pub(super) fn detect_verification_commands(target: &Path) -> Vec<String> {
    let mut commands = Vec::new();

    // Check for Rust/Cargo project
    if target.join("Cargo.toml").exists() {
        commands.push("cargo fmt --check".to_string());
        commands.push("RUSTFLAGS=\"-D warnings\" cargo test".to_string());
        commands.push("cargo build --release".to_string());
    }

    commands.push("ags check governance --workspace .".to_string());

    // If no project tooling found, give guidance
    if commands.is_empty() {
        commands
            .push("Define verification commands in config/agent-project-profile.yaml.".to_string());
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
            content.contains("ags-task-contract")
        } else {
            false
        }
    } else {
        false
    };

    let (validator_entry, alternate_entry) = if has_rust_validator {
        (
            "ags govern task validate --task-card <task-card> --workspace .".to_string(),
            "cargo run -p ags-cli -- govern task validate --task-card <task-card>".to_string(),
        )
    } else {
        (
            "ags govern task validate --task-card <task-card> --workspace .".to_string(),
            "N/A (use the installed AGS Rust kernel)".to_string(),
        )
    };

    let task_card_validator = ValidatorInfo {
        available: true,
        entry: validator_entry,
        description: "The Rust validator owned by ags-task-contract is the canonical task-card format gate. It provides structural format checks, field-value validation, field-combination checks, Execution Authority Gate, protected-path analysis, contradiction detection, and content-quality checks.".to_string(),
        alternate_entry,
    };

    // ── Risk boundaries ────────────────────────────────────────────────
    let risk_boundaries = RiskBoundaries {
        protected_paths: vec![
            "protocol/".to_string(),
            "crates/ags-task-contract/src/validator/".to_string(),
            "Cargo.toml".to_string(),
            "Cargo.lock".to_string(),
            "manifests/public-release-payload.yaml".to_string(),
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
