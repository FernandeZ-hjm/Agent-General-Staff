//! Receipt / Compliance — task run receipt generation, verification, and
//! compliance checking (M6).
//!
//! # Receipt schema
//!
//! A receipt captures the full audit trail of a task run:
//! - `task_card_hash` — SHA-256 of the task card content
//! - `gate_result` — gate check decision (allow / stop) and optional reason
//! - `verification_results` — list of verification commands with exit codes and output hashes
//! - `delivery_report_hash` — SHA-256 of the delivery report (optional)
//!
//! # Compliance check
//!
//! The compliance checker only performs MVP checks:
//! 1. Schema is valid (all required fields present)
//! 2. Task card hash is consistent (if source file still exists)
//! 3. Gate decision is not "stop"

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Delivery-report closure against a canonical task contract.
pub mod delivery_report;
pub mod memory;
mod receipt;
mod receipt_model;

pub use receipt::*;
pub use receipt_model::*;

#[cfg(test)]
mod tests;
