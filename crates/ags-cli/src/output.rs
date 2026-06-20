//! Shared output / formatting helpers.

pub(crate) fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}
pub(crate) fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

// ── Receipt bridge (AGS-owned receipts) ──────────────────────────────────────
