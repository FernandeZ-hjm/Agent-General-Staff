use crate::{verify_receipt, Receipt};
use serde::Serialize;

const MAX_PORTABLE_RECEIPT_BYTES: usize = 4 * 1024 * 1024;

/// Pure, non-authoritative receipt inspection for portable tooling.
///
/// Paths embedded in the receipt remain opaque strings. Mutation and archive
/// writes belong to the sealed control-plane lifecycle.
#[derive(Debug, Clone, Serialize)]
pub struct PortableMemoryInspection {
    pub schema_version: String,
    pub authoritative: bool,
    pub structurally_valid: bool,
    pub receipt_id: Option<String>,
    pub detail: String,
}

pub fn inspect(receipt_bytes: &[u8]) -> PortableMemoryInspection {
    if receipt_bytes.len() > MAX_PORTABLE_RECEIPT_BYTES {
        return invalid("receipt exceeds portable inspection byte budget");
    }
    let receipt: Receipt = match serde_json::from_slice(receipt_bytes) {
        Ok(receipt) => receipt,
        Err(error) => return invalid(format!("invalid receipt JSON: {error}")),
    };
    let verification = verify_receipt(&receipt);
    PortableMemoryInspection {
        schema_version: "ags://schema/contract/v2/portable-memory-inspection".to_string(),
        authoritative: false,
        structurally_valid: verification.valid,
        receipt_id: Some(receipt.receipt_id),
        detail: "non-authoritative structural inspection; no paths were opened".to_string(),
    }
}

fn invalid(detail: impl Into<String>) -> PortableMemoryInspection {
    PortableMemoryInspection {
        schema_version: "ags://schema/contract/v2/portable-memory-inspection".to_string(),
        authoritative: false,
        structurally_valid: false,
        receipt_id: None,
        detail: detail.into(),
    }
}

pub fn render<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
}
