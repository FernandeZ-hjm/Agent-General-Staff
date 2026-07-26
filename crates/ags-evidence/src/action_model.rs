use super::*;

// ── Action receipt (write-action evidence) ───────────────────────────────────
//
// Distinct from the task-card-centric `Receipt`. AGS write / half-write actions
// (setup --yes, skill --apply, init, update apply / repair-local)
// emit an `ActionReceipt` so every mutation leaves machine-readable evidence
// plus a plan-only rollback. `receipt_id` is prefixed `ar-` and `schema_version`
// is `2.0-action-receipt`, so action receipts never collide with task-card
// receipts (`receipt-` / `2.0-m6` or `2.1-m6`) and verifiers can dispatch by schema.
//
// Pure advised-only surfaces such as `ags agents govern` without `--apply` do
// NOT emit receipts. `agents govern --apply` emits one because it writes only
// AGS-owned native memory adapters; external host registration remains advice.

/// One file write recorded in an action receipt. `op` reuses the skill-console
/// PlannedWrite vocabulary so receipts and the console speak the same dialect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptWrite {
    /// create | overwrite | backup | remove | relink | unlink
    pub op: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `.bak.<stamp>` backup path when one was written (rollback references it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    pub detail: String,
}

/// An external command AGS advised but never executed (e.g. `claude mcp add`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptAdvised {
    pub command: String,
    pub reason: String,
}

/// One inverse step in a rollback plan. Data only — AGS never executes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackStep {
    pub affected_path: String,
    /// restore-backup | remove-created | relink-previous | manual-confirm | none
    pub inverse_op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// Human-runnable inverse command (advice; AGS does not run it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inverse_command: Option<String>,
    pub detail: String,
}

/// A plan-only rollback description embedded in every action receipt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollbackPlan {
    pub schema_version: String,
    /// backup-restore | thin-index-relink | manual-confirm | none
    pub strategy: String,
    pub steps: Vec<RollbackStep>,
    pub note: String,
}

impl RollbackPlan {
    /// An empty plan-only rollback with a stable note.
    pub fn none() -> Self {
        RollbackPlan {
            schema_version: "2.0-rollback".to_string(),
            strategy: "none".to_string(),
            steps: Vec::new(),
            note: "PLAN-ONLY — nothing to roll back".to_string(),
        }
    }
    /// A backup-restore rollback (setup / runtime / update lanes).
    pub fn backup_restore(steps: Vec<RollbackStep>) -> Self {
        RollbackPlan {
            schema_version: "2.0-rollback".to_string(),
            strategy: "backup-restore".to_string(),
            steps,
            note: "PLAN-ONLY — apply requires explicit task-card authorization".to_string(),
        }
    }
    /// A thin-index relink rollback (skill / agents / capability host entries).
    pub fn thin_index_relink(steps: Vec<RollbackStep>) -> Self {
        RollbackPlan {
            schema_version: "2.0-rollback".to_string(),
            strategy: "thin-index-relink".to_string(),
            steps,
            note: "PLAN-ONLY — apply requires explicit task-card authorization".to_string(),
        }
    }
    /// A human-reviewed inverse for effects whose prior state cannot be
    /// reconstructed safely from the onboarding command alone.
    pub fn manual_confirm(steps: Vec<RollbackStep>) -> Self {
        RollbackPlan {
            schema_version: "2.0-rollback".to_string(),
            strategy: "manual-confirm".to_string(),
            steps,
            note: "PLAN-ONLY — inspect the action receipt and confirm each inverse step manually"
                .to_string(),
        }
    }
}

/// A write-action receipt: what an AGS mutation planned, applied, advised, and
/// how to roll it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub schema_version: String,
    /// `ar-<action>-<stamp>-<hash12>`
    pub receipt_id: String,
    pub action: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub gate: GateResult,
    pub planned_writes: Vec<ReceiptWrite>,
    pub applied_writes: Vec<ReceiptWrite>,
    pub advised_commands: Vec<ReceiptAdvised>,
    pub verification_results: Vec<VerificationResult>,
    pub rollback: RollbackPlan,
    /// dry-run | applied | failed | advised-only | nothing-to-do | blocked
    pub apply_status: String,
    pub applied: bool,
}
