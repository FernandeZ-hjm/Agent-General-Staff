use super::instruction_projection::*;
use super::protocol_audit::*;
use super::session_preflight::*;
use super::workspace_facts::*;
use super::*;
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
    let ml = &preflight.memory_lifecycle;
    lines.push(format!(
        "Lifecycle:       {} host={} adapter={} (read={}, write={}, stop-guard={}, archive={}, kernel-backed={})",
        ml.status,
        ml.host,
        ml.adapter,
        ml.read_wired,
        ml.write_wired,
        ml.stop_guard_wired,
        ml.archive_ready,
        ml.kernel_backed
    ));
    lines.push(format!("  {}", ml.summary));
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
