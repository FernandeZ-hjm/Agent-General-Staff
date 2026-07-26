//! Diagnostic checks grouped by the facts they inspect.
//!
//! The module keeps orchestration order and the doctor-facing interface stable
//! while concentrating host-memory, runtime, workspace, proxy, template, and
//! routing-resolution knowledge in separate implementations.

use super::types::{Finding, HealthReport};
use serde_yaml::Value as YamlValue;
use std::path::{Path, PathBuf};
use std::process::Command;

mod host_memory;
pub(super) mod orchestration;
pub(super) mod resolution;
mod runtime;
mod workspace;

use host_memory::*;
use resolution::*;
use runtime::*;
use workspace::*;

pub(super) fn is_public_edition(repo_root: &Path) -> bool {
    ["WORKSPACE.md", "CLAUDE.md"].iter().any(|relative| {
        std::fs::read_to_string(repo_root.join(relative))
            .map(|raw| {
                raw.contains("Public Edition") || raw.contains("public distributable edition")
            })
            .unwrap_or(false)
    })
}

include!("tests.rs");
