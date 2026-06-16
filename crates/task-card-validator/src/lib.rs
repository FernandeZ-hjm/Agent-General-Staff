//! Task card validator — validates Agent Governance Suite task cards.
//!
//! Rules enforced:
//! - First non-empty line must be `## 任务卡`
//! - Reject `text`-typed code fences (3+ backticks or 4+ tildes then `text`)
//! - Single canonical task-card format: the classic fixed skeleton whose
//!   second non-empty line is `读取并遵守：` (the removed compact format is
//!   rejected at that structural position)
//! - Check required header and body fields
//! - Validate field values against allowed sets (e.g. Executor, Permission mode)
//! - Validate field combinations (e.g. Executor ↔ Runtime adapter)
//! - Detect protected-path violations
//! - Check content quality (non-empty goals, concrete verification, etc.)
//! - Detect contradictory requirements
//!
//! # Example
//!
//! ```rust
//! use task_card_validator::validate;
//!
//! let input = "## 任务卡\n读取并遵守：\n- AGENTS.md\nExecutor: Claude Code\nRuntime adapter: claude-code\nExecution surface: cli\nPermission mode: execute-and-verify\nParallelism: none\n任务级别：Medium\nReview gate:\n- Medium review\n任务：运行测试验证校验器\n背景：验证校验器功能\n项目画像：无\n记忆胶囊：无\n任务存档：无\n目标文件夹路径：\n- .\n相关路径：\n- .\n本次任务相关文件：\n- .\n目标：验证校验器功能\n非目标：不修改文件\n验证：\ncargo test\nVerification gate:\n- commands: cargo test\n交付：\n返回结果\n";
//! let errors = validate(input);
//! assert!(errors.is_empty());
//! ```
//!
//! # Module layout
//!
//! This validator was split from a single ~5300-line `lib.rs` into cohesive
//! modules. Sibling modules reach shared, crate-internal items through
//! `use super::*`. The public API surface is `validate`, `validate_files`,
//! `parse_validated`, `ParsedTaskCard`, and `error_code`.

mod authority;
mod checks;
mod constants;
mod contradictions;
mod parse;
mod types;
mod validate;

#[cfg(test)]
mod tests;

// Shared std imports, re-exported crate-internally so each module's
// `use super::*` resolves them.
pub(crate) use std::collections::HashMap;
pub(crate) use std::fs;
pub(crate) use std::io::{self, Read};
pub(crate) use std::path::Path;

// Crate-internal re-exports: each module's items are visible at the crate root
// so sibling modules can rely on `use super::*`.
pub(crate) use authority::*;
pub(crate) use checks::*;
pub(crate) use constants::*;
pub(crate) use contradictions::*;
pub(crate) use parse::*;

// `types` and `validate` expose their entire surface through the public
// re-exports below, so no crate-internal glob is needed for them.

// `read_input` is private-by-default (used by `validate_files`) but is also
// exercised directly by unit tests, so re-export it crate-internally under test.
#[cfg(test)]
pub(crate) use validate::read_input;

// Public API for downstream crates. NOTE: `CardType` and the
// `ParsedTaskCard.card_type` field were removed as part of deleting the
// compact task-card format (single canonical format = the classic skeleton).
// This is an intentional breaking change to this crate's public API; no
// in-workspace consumer read `card_type`.
pub use parse::parse_validated;
pub use types::{error_code, ParsedTaskCard};
pub use validate::{validate, validate_files};
