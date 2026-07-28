use super::instruction_projection::*;
use super::protocol_audit::*;
use super::workspace_facts::*;
use super::*;
use ags_host_integration::*;
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
    #[serde(skip_serializing)]
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

    // Project memory lifecycle closure (read / write / archive / verify state)
    pub memory_lifecycle: MemoryLifecycle,

    // Agent instructions summary
    pub should_stop: bool,
    pub stop_conditions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub default_execution_mode: String,
    pub default_execution_topology: String,
    pub default_delegation_planning: String,

    // Aggregated diagnostics
    pub governance_status: ags_governance_decision::GovernanceStatus,
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
    let instructions =
        generate_agent_instructions_from_facts(&canonical, agent_type, &identity, &protocol_status);

    // ── Memory paths ─────────────────────────────────────────────────────
    let capsule_path = identity.memory_capsule_path.clone();
    let capsule_exists = capsule_path.as_ref().map(|p| p.exists());
    let task_mem_path = identity.task_memory_path.clone();
    let task_mem_exists = task_mem_path.as_ref().map(|p| p.exists());

    // ── Project memory lifecycle closure ──────────────────────────────────
    // Authoritative read/write/archive/verify verdict (shared with `ags doctor`).
    let memory_lifecycle = compute_memory_lifecycle_for_host(&canonical, agent_type);

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
                "  {} will use default execution mode: {}",
                agent_type.display_name(),
                instructions.permissions.default_execution_mode
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

        memory_lifecycle,

        should_stop: instructions.should_stop,
        stop_conditions: instructions.stop_conditions.clone(),
        verification_commands: instructions.verification_commands.clone(),
        default_execution_mode: instructions.permissions.default_execution_mode.clone(),
        default_execution_topology: instructions.permissions.default_execution_topology.clone(),
        default_delegation_planning: instructions.permissions.default_delegation_planning.clone(),

        governance_status: if overall_status == PreflightStatus::Stop {
            ags_governance_decision::GovernanceStatus::BlockedByPolicy
        } else {
            ags_governance_decision::GovernanceStatus::Ok
        },
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
