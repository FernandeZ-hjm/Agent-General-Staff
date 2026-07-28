use super::*;

fn unix_secs() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Per-call entropy for collision-resistant receipt ids: (nanoseconds, pid).
fn unique_token() -> (u128, u32) {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    (nanos, std::process::id())
}

/// Build an action receipt from the facts of one write action. `receipt_id` is
/// derived deterministically from the action plus planned/applied content.
#[allow(clippy::too_many_arguments)]
pub fn build_action_receipt(
    action: &str,
    target: Option<&str>,
    gate: GateResult,
    planned: Vec<ReceiptWrite>,
    applied: Vec<ReceiptWrite>,
    advised: Vec<ReceiptAdvised>,
    verification: Vec<VerificationResult>,
    apply_status: &str,
    applied_flag: bool,
) -> ActionReceipt {
    let stamp = unix_secs();
    let (nanos, pid) = unique_token();
    // Hash includes per-call entropy (nanos, pid) and the full action surface
    // (target, advised, verification) so distinct actions in the same second
    // never collide on the same receipt id.
    let mut basis = format!("{action}:{stamp}:{nanos}:{pid}:{apply_status}");
    if let Some(t) = target {
        basis.push_str(&format!("|target:{t}"));
    }
    for w in planned.iter().chain(applied.iter()) {
        basis.push_str(&format!("|{}:{}", w.op, w.path));
    }
    for a in &advised {
        basis.push_str(&format!("|advised:{}", a.command));
    }
    for v in &verification {
        basis.push_str(&format!("|verify:{}:{}", v.command, v.exit_code));
    }
    let hash = sha256_hex(basis.as_bytes());
    ActionReceipt {
        schema_version: "0.3.6-action-receipt".to_string(),
        receipt_id: format!("ar-{action}-{stamp}-{}", &hash[..16.min(hash.len())]),
        action: action.to_string(),
        timestamp: format!("unix-{stamp}"),
        target: target.map(|s| s.to_string()),
        gate,
        planned_writes: planned,
        applied_writes: applied,
        advised_commands: advised,
        verification_results: verification,
        apply_status: apply_status.to_string(),
        applied: applied_flag,
    }
}

/// Persist an action receipt to `<receipts_root>/<receipt_id>.json`, returning
/// the absolute path. Refuses to write if any serialized field carries a
/// token-like secret. On Unix the file is chmod 0o600.
pub fn emit_action_receipt(
    receipts_root: &Path,
    receipt: &ActionReceipt,
) -> Result<std::path::PathBuf, String> {
    let json = render_action_receipt_json(receipt);
    if receipt_contains_secret(&json) {
        return Err("refusing to write receipt: token-like secret detected".to_string());
    }
    std::fs::create_dir_all(receipts_root)
        .map_err(|e| format!("cannot create {}: {}", receipts_root.display(), e))?;
    // Create-new semantics: never overwrite an existing receipt (mutation
    // evidence must not be lost). On the rare id collision, append a counter.
    use std::io::Write;
    for attempt in 0..1000u32 {
        let name = if attempt == 0 {
            format!("{}.json", receipt.receipt_id)
        } else {
            format!("{}-{attempt}.json", receipt.receipt_id)
        };
        let path = receipts_root.join(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(json.as_bytes())
                    .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {}", path.display(), e)),
        }
    }
    Err("receipt id collision: too many receipts with the same id".to_string())
}

/// Render an action receipt as pretty JSON.
pub fn render_action_receipt_json(r: &ActionReceipt) -> String {
    serde_json::to_string_pretty(r)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// A single-line `receipt: <path>` summary for quiet-by-default output.
pub fn render_action_receipt_summary_line(path: &Path) -> String {
    format!("receipt: {}", path.display())
}

/// Minimal token-like secret detector (Bearer / sk- tails) so receipts never
/// leak credentials. Self-contained to avoid a cross-crate dependency.
fn receipt_contains_secret(text: &str) -> bool {
    token_like(text, "Bearer ", 20) || token_like(text, "sk-", 20)
}

fn token_like(text: &str, prefix: &str, min_tail: usize) -> bool {
    let mut start = 0;
    while let Some(off) = text[start..].find(prefix) {
        let tail_start = start + off + prefix.len();
        let tail = &text[tail_start..];
        let len = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if len >= min_tail {
            return true;
        }
        start = tail_start;
    }
    false
}
