//! Workspace facts and host-aware project preflight.
//!
//! The crate keeps its established public interface while concentrating each
//! read-only knowledge area in one internal module. Callers do not coordinate
//! filesystem evidence, host lifecycle probes, protocol audits, projections,
//! or rendering themselves.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod instruction_projection;
pub mod managed_projects;
mod protocol_audit;
mod rendering;
mod session_preflight;
mod workspace_facts;

pub use ags_host_integration::{
    compute_memory_lifecycle, compute_memory_lifecycle_at, compute_memory_lifecycle_at_for_host,
    compute_memory_lifecycle_for_host, AgentType, MemoryLifecycle,
};
pub use instruction_projection::{
    generate_agent_instructions, AgentInstructions, AgentPermissions, InstructionFile,
};
pub use protocol_audit::{
    check_protocol_status, ProtocolFileStatus, ProtocolStatus, ReceiptRequirements,
    ReviewRequirements, RiskBoundaries, ValidatorInfo,
};
pub use rendering::{
    project_detect_exit_code, protocol_status_exit_code, render_agent_instructions_text,
    render_json, render_project_identity_text, render_protocol_status_text,
    render_session_preflight_text,
};
pub use session_preflight::{
    run_session_preflight, session_preflight_exit_code, PreflightStatus, SessionPreflight,
};
pub use workspace_facts::{detect_project, IntegrationStatus, ProjectIdentity, WorkspaceIdentity};

#[cfg(test)]
mod tests;
