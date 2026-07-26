//! AGS skill governance — skill-body governance face (scan / check /
//! inventory / upstream / propose).
//!
//! This module is a facade over read-only skill governance:
//! - [`scan_skills`], [`check_skills`], and [`propose_skills`] read governance
//!   manifests and produce stable typed results.
//! - [`scan_skill_inventory`] inspects only on-disk `SKILL.md` front matter.
//! - [`upstream_proposal`] builds the read-only upstream comparison skeleton.
//! - render functions preserve the existing text, JSON, and Markdown surfaces.
//!
//! The management console ([`console`]) remains a separate,
//! confirmation-protected mutation boundary.

/// Third-party skill and MCP management console.
pub mod console;
pub mod recommendations;

mod inventory;
mod model;
mod read_model;
mod render;
mod upstream;

pub use inventory::{
    scan_skill_inventory, SkillInventoryEntry, SkillInventoryResult, SkillInventorySummary,
};
pub use model::{
    ConsistencyCheck, FileStatus, GovernanceFileStatus, SkillCheckResult, SkillEntry, SkillIssue,
    SkillProposalResult, SkillScanResult, SkillScanSummary, SkillStatus, SCHEMA_VERSION,
};
pub use read_model::{check_skills, propose_skills, scan_skills};
pub use render::{
    render_check_json, render_check_text, render_inventory_json, render_inventory_markdown,
    render_inventory_text, render_proposal_json, render_proposal_text, render_scan_json,
    render_scan_text, render_upstream_json, render_upstream_text,
};
pub use upstream::{
    upstream_proposal, CandidateSkillInfo, UpstreamProposalResult, UpstreamProposalSummary,
    UpstreamSourceInfo, WatchedSkill,
};

pub(crate) use inventory::parse_front_matter;

#[cfg(test)]
mod tests;
