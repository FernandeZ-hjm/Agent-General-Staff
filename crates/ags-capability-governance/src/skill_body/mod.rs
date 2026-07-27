//! AGS skill governance — static scan and inventory.
//!
//! This module is a facade over read-only skill governance:
//! - [`scan_skills`] reads the static suite manifest.
//! - [`scan_skill_inventory`] inspects only on-disk `SKILL.md` front matter.
//! - render functions expose text, JSON, and Markdown views.

/// Third-party skill and MCP management console.
pub mod console;
pub mod recommendations;

mod inventory;
mod model;
mod read_model;
mod render;

pub use inventory::{
    scan_skill_inventory, SkillInventoryEntry, SkillInventoryResult, SkillInventorySummary,
};
pub use model::{SkillEntry, SkillScanResult, SkillScanSummary, SkillStatus, SCHEMA_VERSION};
pub use read_model::scan_skills;
pub use render::{render_inventory_json, render_inventory_markdown, render_inventory_text};

pub(crate) use inventory::parse_front_matter;
