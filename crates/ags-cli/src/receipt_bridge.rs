//! AGS-owned receipt emission bridge.

use crate::context::default_private_runtime_home;
use std::path::PathBuf;

/// AGS-owned receipts directory: `<runtime home>/receipts`.
fn ags_receipts_root() -> PathBuf {
    default_private_runtime_home().join("receipts")
}
/// Emit an action receipt into the AGS-owned receipts directory.
pub(crate) fn emit_ags_action_receipt(
    action_receipt: &ags_evidence::ActionReceipt,
) -> Result<PathBuf, String> {
    ags_evidence::emit_action_receipt(&ags_receipts_root(), action_receipt)
}
