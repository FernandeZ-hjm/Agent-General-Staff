use super::*;

// ── Action receipt (write-action evidence) ───────────────────────────────────
//
// Distinct from the task-card-centric `Receipt`. AGS write / half-write actions
// (setup --yes, skill --apply, init, update apply / repair-local)
// emit an `ActionReceipt` so every mutation leaves machine-readable evidence.
// `receipt_id` is prefixed `ar-` and `schema_version`
// is `0.3.6-action-receipt`, so action receipts never collide with task-card
// receipts (`receipt-` / `0.3.6-task-receipt`) and verifiers can dispatch by schema.
//
// Pure advised-only surfaces such as `ags agents govern` without `--apply` do
// NOT emit receipts. `agents govern --apply` emits one because it writes only
// AGS-owned native memory adapters; external host registration remains advice.

/// One file write recorded in an action receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptWrite {
    /// create | overwrite | remove | relink | unlink
    pub op: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub detail: String,
}

/// An external command AGS advised but never executed (e.g. `claude mcp add`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptAdvised {
    pub command: String,
    pub reason: String,
}

/// A write-action receipt: what an AGS mutation planned, applied, advised, and
/// verified.
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
    /// dry-run | applied | failed | advised-only | nothing-to-do | blocked
    pub apply_status: String,
    pub applied: bool,
}
