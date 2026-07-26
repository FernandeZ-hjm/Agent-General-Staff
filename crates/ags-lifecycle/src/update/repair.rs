//! Repair-local lifecycle.

/// Reapply the AGS-owned runtime projection without updating source or projects.
pub fn execute(request: crate::setup::PrivateApplyRequest<'_>) -> crate::setup::PrivateApplyResult {
    crate::setup::apply_private(request)
}
