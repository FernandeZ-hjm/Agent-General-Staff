use super::protocol_audit::*;
use super::workspace_facts::*;
use super::*;
use ags_host_integration::*;
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
    pub default_execution_mode: String,
    pub default_execution_topology: String,
    pub default_delegation_planning: String,
    pub may_edit_files: bool,
    pub may_delegate: bool,
    pub may_install: bool,
}

/// Generate agent instructions for the given agent type and target.
pub fn generate_agent_instructions(target: &Path, agent_type: &AgentType) -> AgentInstructions {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let identity = detect_project(&canonical);
    let protocol_status = check_protocol_status(&canonical);
    generate_agent_instructions_from_facts(&canonical, agent_type, &identity, &protocol_status)
}

pub(super) fn generate_agent_instructions_from_facts(
    canonical: &Path,
    agent_type: &AgentType,
    identity: &ProjectIdentity,
    protocol_status: &ProtocolStatus,
) -> AgentInstructions {
    let project_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let (role_description, permissions, stop_conditions, required_reads) =
        match agent_type {
            AgentType::Codex => (
                "Codex owns ambient preflight, complete conversation context, the only natural-language semantic decision, conditional solution formation, execution decision, and review. It reads `ags://capabilities/current-host` and submits a typed HostRouteProposal to read-only MCP `ags_route_request`. DirectResponse delivers and stops; exact SkillTarget is checked against ActiveSkillTable; MachineCli becomes a connection-held action and only `ags_apply_action` may consume it. An approved contract plus explicit same-session modification instruction enters host-native direct execution without repeating solution formation or compiling a task card. New unresolved solutions require confirmation. Explicit task-card generation requires a handoff instruction and confirmed contract. In host Plan mode, the final decision-complete artifact is the canonical task card; approval switches to execution mode and dispatches that exact card without regeneration. Reopened solution work stays in solution formation."
                    .to_string(),
                AgentPermissions {
                    default_execution_mode: "single-writer".to_string(),
                    default_execution_topology: "single".to_string(),
                    default_delegation_planning: "no".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Do not install hooks, runner adapters, or production wiring without explicit protected-operation authorization."
                        .to_string(),
                    "Stop before broad refactors unless the confirmed solution and current execution authorization explicitly cover them."
                        .to_string(),
                    "If actual risk is higher than the confirmed solution or handoff card declares, stop and report — do not silently downgrade."
                        .to_string(),
                    "Do not modify S (stable) directly without explicit stable-boundary authorization."
                        .to_string(),
                    "Do not change public-full sanitized payload boundary, canonical task-card skeleton, or execution-policy M1-M10 rules without explicit approval."
                        .to_string(),
                    "Do not generate task cards or call `ags task compile --task-card-requested --confirmed-handoff-contract` until the user explicitly issues a task-card/handoff instruction and the handoff contract is confirmed. This compiler gate does not restrict authorized same-session direct execution."
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
                "Claude Code executes bounded handoff task cards. It reads the task card, protocol files, and project docs; implements within scope; runs verification; and outputs a delivery report. Claude Code must NOT perform solution formation or task-level classification from raw user requests — those phases belong to Codex/Cursor. Claude Code only consumes already-formed cards and must not infer same-session direct-edit authority from `方案 OK` or raw prose."
                    .to_string(),
                AgentPermissions {
                    default_execution_mode: "single-writer".to_string(),
                    default_execution_topology: "single".to_string(),
                    default_delegation_planning: "no".to_string(),
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
                    "For Heavy tasks: task level is a risk/review tier, not execution authority. Honor the task card's explicit Execution mode, Execution topology, Delegation planning, independent Review gate, and stop conditions. Heavy never upgrades or downgrades those authority fields."
                        .to_string(),
                    "On resume/continue: reread the task card, run `git status --short`, reconfirm review_targets, and honor its explicit execution authority without inferring permission from the task level or conversation state."
                        .to_string(),
                    "Do not generate task cards or call `ags task compile --task-card-requested --confirmed-handoff-contract` from raw user requests or unresolved solution-phase outputs. Only Codex/Cursor may generate task cards after receiving an explicit task-card instruction and confirming the handoff contract. \"方案 OK\" alone is not a task-card generation trigger."
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
                "Cursor owns ambient preflight, complete conversation context, the only natural-language semantic decision, conditional solution formation, execution decision, and review inside its IDE workflow. It reads `ags://capabilities/current-host` and submits a typed HostRouteProposal to read-only MCP `ags_route_request`. DirectResponse delivers and stops; exact SkillTarget is checked against ActiveSkillTable; MachineCli is only consumed by explicit `ags_apply_action`. Explicit same-session modification authorization enters host-native direct execution. Explicit task compilation requires a confirmed contract and handoff request; in host Plan mode the final decision-complete artifact is the canonical task card, and approval switches mode before dispatching that exact card without regeneration."
                    .to_string(),
                AgentPermissions {
                    default_execution_mode: "single-writer".to_string(),
                    default_execution_topology: "single".to_string(),
                    default_delegation_planning: "no".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Stop before broad refactors unless the confirmed solution and current execution authorization explicitly cover them."
                        .to_string(),
                    "Do not install hooks without explicit protected-operation authorization."
                        .to_string(),
                    "Keep task-card facts project-local; do not bake global suite internals into project-specific prompts."
                        .to_string(),
                    "Use IDE context only as supporting evidence; final claims still need commands, diffs, screenshots, or other explicit evidence."
                        .to_string(),
                    "If delegating to Claude Code CLI, provide a self-contained prompt and review the result before treating the task as complete."
                        .to_string(),
                    "Do not generate a task card until both the explicit handoff/task-card request and confirmed handoff contract gates are closed; stop if solution work is unresolved or reopened."
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
                    "{host_label} is an AGS-compatible governed host (Tencent Agent hosts — WorkBuddy and CodeBuddy-Code — resolve to this governed-host profile; other unknown hosts use it too). It must complete AGS initialization preflight before any AGS scenario work, then follow the governed lifecycle surfaced by the preflight report. Governed hosts may form solutions, but must not infer privileges from the Agent product name. Explicit same-session modification authorization and confirmed-contract task-card handoff are separate paths; `方案 OK` alone authorizes neither."
                ),
                AgentPermissions {
                    default_execution_mode: "single-writer".to_string(),
                    default_execution_topology: "single".to_string(),
                    default_delegation_planning: "no".to_string(),
                    may_edit_files: true,
                    may_delegate: true,
                    may_install: false,
                },
                vec![
                    "Call `ags_preflight` first for AGS scenarios; do not call other AGS tools before preflight succeeds.".to_string(),
                    "Use the explicit `target` project path supplied by the host; do not assume the desktop workspace folder is the governed project.".to_string(),
                    "Do not perform Light/Medium/Heavy task classification from raw user requests.".to_string(),
                    "Do not install hooks, runner adapters, dependencies, or production wiring without explicit protected-operation authorization.".to_string(),
                    "Do not generate task cards or call `ags task compile --task-card-requested --confirmed-handoff-contract` until the user explicitly issues a task-card instruction and the handoff contract is confirmed.".to_string(),
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
    let verification_commands = protocol_status.verify_requirements.clone();

    // ── Protocol status for integration context ─────────────────────────
    let protocol_failures = protocol_status.failures.clone();
    let protocol_warnings = protocol_status.warnings.clone();
    let integration_gaps = identity.gaps.clone();

    // ── Determine stop behavior ─────────────────────────────────────────
    let (should_stop, stop_reasons, exit_code) = match &identity.integration_status {
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
            "crates/ags-task-contract/src/validator/".to_string(),
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
        identity,
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
        target: canonical.to_path_buf(),
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
pub(super) fn build_instructions_text(
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
            "protocol/, and the ags-task-contract validator) before executing tasks in this repo."
                .to_string(),
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
        "- Execution mode: {}",
        permissions.default_execution_mode
    ));
    lines.push(format!(
        "- Execution topology: {}",
        permissions.default_execution_topology
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
