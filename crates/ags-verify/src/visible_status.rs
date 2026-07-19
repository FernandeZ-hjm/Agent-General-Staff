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

pub use request_governance::GovernanceStatus;

/// Compatibility alias. New contracts should use `GovernanceStatus`.
#[deprecated(note = "use GovernanceStatus")]
pub type VisibleStatus = GovernanceStatus;

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
    /// A human decision is required (e.g. task-card instruction or hard-stop approval).
    pub needs_user_decision: bool,
    /// Work completed and a receipt exists.
    pub done_with_receipt: bool,
}

/// Derive the single foreground status from raw signals, by descending severity:
/// BlockedByPolicy → RiskEscalated → AdvisoryNoMutation → NeedsUserDecision →
/// DoneWithReceipt → Ok.
pub fn derive_governance_status(s: &StatusSignals) -> GovernanceStatus {
    if s.blocked_by_policy {
        GovernanceStatus::BlockedByPolicy
    } else if s.risk_escalated {
        GovernanceStatus::RiskEscalated
    } else if s.advisory_no_mutation {
        GovernanceStatus::AdvisoryNoMutation
    } else if s.needs_user_decision {
        GovernanceStatus::NeedsUserDecision
    } else if s.done_with_receipt {
        GovernanceStatus::DoneWithReceipt
    } else {
        GovernanceStatus::Ok
    }
}

#[allow(deprecated)]
pub fn derive_visible_status(s: &StatusSignals) -> VisibleStatus {
    derive_governance_status(s)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_signals_are_ok() {
        let s = StatusSignals::default();
        assert_eq!(derive_governance_status(&s), GovernanceStatus::Ok);
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
        assert_eq!(
            derive_governance_status(&s),
            GovernanceStatus::BlockedByPolicy
        );
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
        assert_eq!(
            derive_governance_status(&s),
            GovernanceStatus::RiskEscalated
        );
    }

    #[test]
    fn advisory_beats_needs_user_and_receipt() {
        let s = StatusSignals {
            advisory_no_mutation: true,
            needs_user_decision: true,
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(
            derive_governance_status(&s),
            GovernanceStatus::AdvisoryNoMutation
        );
    }

    #[test]
    fn needs_user_decision_beats_receipt() {
        let s = StatusSignals {
            needs_user_decision: true,
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(
            derive_governance_status(&s),
            GovernanceStatus::NeedsUserDecision
        );
    }

    #[test]
    fn done_with_receipt_when_only_signal() {
        let s = StatusSignals {
            done_with_receipt: true,
            ..Default::default()
        };
        assert_eq!(
            derive_governance_status(&s),
            GovernanceStatus::DoneWithReceipt
        );
    }

    #[test]
    fn json_is_screaming_snake_case() {
        let v = serde_json::to_value(GovernanceStatus::NeedsUserDecision).expect("serialize");
        assert_eq!(v, "NEEDS_USER_DECISION");
        assert_eq!(
            GovernanceStatus::AdvisoryNoMutation.as_str(),
            "ADVISORY_NO_MUTATION"
        );
        assert_eq!(GovernanceStatus::Ok.as_str(), "OK");
    }
}
