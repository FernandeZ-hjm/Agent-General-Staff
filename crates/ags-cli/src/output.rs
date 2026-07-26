//! Shared output / formatting helpers.

pub(crate) fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

// ── Receipt bridge (AGS-owned receipts) ──────────────────────────────────────
