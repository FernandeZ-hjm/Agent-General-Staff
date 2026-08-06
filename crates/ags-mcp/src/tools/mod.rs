//! MCP governance tool adapter.
//!
//! Wire declaration/dispatch is kept separate from preflight admission,
//! read-only decisions, and effectful apply transactions. The exported MCP
//! tool names, schemas, JSON results, and error strings remain unchanged.

use crate::protocol::ToolListResult;
use ags_governance_decision::{
    proposal_hash, validate_machine_input, validate_proposal, CliCapabilityId,
    DecisionLeaseEvidence, ExecutionAuthority, GovernanceStatus, HostRouteProposal, ProposalError,
    ProposalTarget, ResolvedTarget, RouteResolution, ServerHeldActionKind, TaskCardHandoffSource,
    TypedCliInput, ROUTE_RESOLUTION_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod apply;
mod decision;
mod maintenance;
mod preflight;
mod wire;

#[allow(unused_imports)]
pub(crate) use wire::{
    call_tool, is_onboarding_bootstrap_tool_name, is_preflight_bootstrap_tool_name,
    is_preflight_tool_name, list_tools, CapabilityCatalogSource, HeldAction, PreflightBinding,
    RoutingSession, CURRENT_HOST_CAPABILITIES_URI, TOOL_AGENT_INSTRUCTIONS, TOOL_APPLY_ACTION,
    TOOL_MAINTENANCE_APPLY, TOOL_MAINTENANCE_PLAN, TOOL_MAINTENANCE_RECOVER,
    TOOL_MAINTENANCE_STATUS, TOOL_MAINTENANCE_VERIFY, TOOL_ONBOARDING_PLAN, TOOL_POLICY_RESOLVE,
    TOOL_PREFLIGHT, TOOL_PROTOCOL_STATUS, TOOL_ROUTE_REQUEST, TOOL_TASK_VALIDATE,
};
