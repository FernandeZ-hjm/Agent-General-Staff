//! Workspace-owned AGS lifecycle wiring shared by setup and host governance.
//!
//! The lifecycle projection installs one three-event adapter in the canonical
//! workspace, preserves unrelated host configuration, and bootstraps the Rust
//! memory store without replacing an existing capsule.
//!
//! `ags setup --register-claude` and `ags agents govern --apply` delegate to the
//! same implementation. `ags init` only creates project state.

mod adapter;

pub use adapter::{apply_host_memory_adapter, lifecycle_migration_preview};

pub(in crate::setup) use adapter::{add_workspace_memory_capture, render_memory_capture_plan};
