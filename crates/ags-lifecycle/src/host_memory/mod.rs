//! Workspace-owned AGS lifecycle wiring for host governance.
//!
//! The lifecycle projection installs one three-event adapter in the canonical
//! workspace, preserves unrelated host configuration, and bootstraps the Rust
//! memory store without replacing an existing capsule.
//!
//! `ags agents govern --apply` is the sole mutation owner. `ags init` consumes
//! the approved host set through the lower-level lifecycle projection.

mod adapter;

pub use adapter::{apply_host_memory_adapter, lifecycle_migration_preview};
