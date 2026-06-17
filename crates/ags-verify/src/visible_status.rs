//! Quiet-by-default visible status.
//!
//! AGS produces a lot of governance evidence — full preflight reports, change
//! lanes, verification items, traces, receipts. Surfacing all of it on the
//! foreground every turn is noise. The `VisibleStatus` is the single decision
//! state a host should show by default; the full evidence stays addressable in
//! traces, receipts, and the task archive.
//!
//! **Quiet only affects the foreground.** It never suppresses trace, receipt, or
//! archive writes — "可审计不等于过程直播". Auditable ≠ livestreamed.

use serde::Serialize;

/// The single foreground decision state for a governance step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VisibleStatus {
    /// No action needed — proceed.
    Ok,
    /// Waiting for a human decision (e.g. confirmation gate, task-card instruction).
    NeedsUserDecision,
    /// A policy gate stopped execution.
    BlockedByPolicy,
    /// Risk escalated beyond what the task card declared.
    RiskEscalated,
    /// Completed; a receipt is available.
    DoneWithReceipt,
    /// Advisory/consultation intent detected — mutation is blocked.
    AdvisoryNoMutation,
}

impl VisibleStatus {
    /// Stable SCREAMING_SNAKE_CASE identifier, also used in JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            VisibleStatus::Ok => "OK",
            VisibleStatus::NeedsUserDecision => "NEEDS_USER_DECISION",
            VisibleStatus::BlockedByPolicy => "BLOCKED_BY_POLICY",
            VisibleStatus::RiskEscalated => "RISK_ESCALATED",
            VisibleStatus::DoneWithReceipt => "DONE_WITH_RECEIPT",
            VisibleStatus::AdvisoryNoMutation => "ADVISORY_NO_MUTATION",
        }
    }
}

/// Raw signals a caller collects from its own state, fed into the deterministic
/// status derivation. Each field is a plain bool so callers don't need to share
/// gate/policy/classifier types across crates.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusSignals {
    /// A policy gate stopped execution (e.g. `stop_before_launch`).
    pub blocked_by_policy: bool,
    /// Risk escalated beyond the declared task level.
    pub risk_escalated: bool,
    /// Advisory intent detected and mutation is not allowed.
    pub advisory_no_mutation: bool,
    /// A human decision is required (e.g. confirmation gate, task-card instruction).
    pub needs_user_decision: bool,
    /// Work completed and a receipt exists.
    pub done_with_receipt: bool,
}

/// Derive the single foreground status from raw signals, by descending severity:
/// BlockedByPolicy → RiskEscalated → AdvisoryNoMutation → NeedsUserDecision →
/// DoneWithReceipt → Ok.
pub fn derive_visible_status(s: &StatusSignals) -> VisibleStatus {
    if s.blocked_by_policy {
        VisibleStatus::BlockedByPolicy
    } else if s.risk_escalated {
        VisibleStatus::RiskEscalated
    } else if s.advisory_no_mutation {
        VisibleStatus::AdvisoryNoMutation
    } else if s.needs_user_decision {
        VisibleStatus::NeedsUserDecision
    } else if s.done_with_receipt {
        VisibleStatus::DoneWithReceipt
    } else {
        VisibleStatus::Ok
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_signals_are_ok() {
        let s = StatusSignals::default();
        assert_eq!(derive_visible_status(&s), VisibleStatus::Ok);
    }

    #[test]
    fn blocked_by_policy_wins_over_all() {
        let s = StatusSignals {
            blocked_by_policy: true,
            risk_escalated: true,
            advisory_no_mutation: true,
            needs_user_decision: true,
            done_with_receipt: true,
        };
        assert_eq!(derive_visible_status(&s), VisibleStatus::BlockedByPolicy);
    }

    #[test]
    fn risk_escalated_beats_lower() {
        let s = StatusSignals {
            risk_escalated: true,
            advisory_no_mutation: true,
            needs_user_decision: true,
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(derive_visible_status(&s), VisibleStatus::RiskEscalated);
    }

    #[test]
    fn advisory_beats_needs_user_and_receipt() {
        let s = StatusSignals {
            advisory_no_mutation: true,
            needs_user_decision: true,
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(derive_visible_status(&s), VisibleStatus::AdvisoryNoMutation);
    }

    #[test]
    fn needs_user_decision_beats_receipt() {
        let s = StatusSignals {
            needs_user_decision: true,
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(derive_visible_status(&s), VisibleStatus::NeedsUserDecision);
    }

    #[test]
    fn done_with_receipt_when_only_signal() {
        let s = StatusSignals {
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(derive_visible_status(&s), VisibleStatus::DoneWithReceipt);
    }

    #[test]
    fn json_is_screaming_snake_case() {
        let v = serde_json::to_value(VisibleStatus::NeedsUserDecision).expect("serialize");
        assert_eq!(v, "NEEDS_USER_DECISION");
        assert_eq!(
            VisibleStatus::AdvisoryNoMutation.as_str(),
            "ADVISORY_NO_MUTATION"
        );
        assert_eq!(VisibleStatus::Ok.as_str(), "OK");
    }
}
