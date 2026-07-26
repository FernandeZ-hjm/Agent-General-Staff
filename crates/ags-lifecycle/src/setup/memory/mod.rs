//! AGS context-memory product mechanism wiring for `ags setup`.
//!
//! Restores the project-memory injection/capture chain as a first-class product:
//!   - installs the canonical start/guard/capture scripts to the host script dir,
//!   - installs the OMP native lifecycle extension,
//!   - structurally merges Claude Code SessionStart/Stop and Codex
//!     SessionStart/SessionEnd command hooks without replacing unrelated hooks,
//!   - bootstraps the current workspace's memory capsule via the installed
//!     `context-memory.sh` (create-if-missing; never overwrites the capsule).
//!
//! Command boundary: this lives in `ags setup` (host/workspace bootstrap).
//! `ags init` only creates per-project memory files and never installs a host
//! hook — the installed start/capture bridges are cwd-aware and resolve each
//! project's memory by repository.

use super::InstallFile;
use std::path::{Path, PathBuf};

mod adapter;
mod assets;
mod merge;
mod wire;

pub use adapter::apply_host_memory_adapter;
pub use merge::MergeOutcome;

pub(in crate::setup) use adapter::{add_workspace_memory_capture, render_memory_capture_plan};
pub(in crate::setup) use assets::{
    claude_stop_memory_capture_path, context_memory_script_path, memory_script_install_files,
};
#[cfg(test)]
mod tests;
