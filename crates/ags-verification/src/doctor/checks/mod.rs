//! Diagnostic checks grouped by the facts they inspect.
//!
//! The module keeps orchestration order and the doctor-facing interface stable
//! while concentrating host-memory, runtime, workspace, proxy, and
//! routing-resolution knowledge in separate implementations.

use super::types::{Finding, HealthReport};
use serde_yaml::Value as YamlValue;
use std::path::{Path, PathBuf};
use std::process::Command;

mod conformance;
mod host_memory;
pub(super) mod orchestration;
pub(super) mod resolution;
mod workspace;

use conformance::*;
use host_memory::*;
use resolution::*;
use workspace::*;
