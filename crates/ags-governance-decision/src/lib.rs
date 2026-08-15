//! Contract-v2 governance policy primitives.
//!
//! Natural-language interpretation and surface adaptation belong to the host.
//! The operation registry and sealed decision/apply state machine live in the
//! deep `ags-control-plane` Module. This crate owns only execution policy and
//! the shared terminal status vocabulary; it deliberately exposes no legacy
//! route proposal, machine-CLI capability, or lease wire.

use serde::{Deserialize, Serialize};

pub mod policy;

pub const CONTRACT_VERSION: &str = "2";
pub const EXECUTION_POLICY_SCHEMA: &str = "ags://schema/contract/v2/execution-policy";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GovernanceStatus {
    Ok,
    NeedsUserDecision,
    BlockedByPolicy,
    RiskEscalated,
    DoneWithReceipt,
    AdvisoryNoMutation,
    HostExecutionRequired,
}
impl GovernanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::NeedsUserDecision => "NEEDS_USER_DECISION",
            Self::BlockedByPolicy => "BLOCKED_BY_POLICY",
            Self::RiskEscalated => "RISK_ESCALATED",
            Self::DoneWithReceipt => "DONE_WITH_RECEIPT",
            Self::AdvisoryNoMutation => "ADVISORY_NO_MUTATION",
            Self::HostExecutionRequired => "HOST_EXECUTION_REQUIRED",
        }
    }
}
