//! Read-only host configuration, CLI, and visibility probes.
//!
//! The facade retains the console's internal probe API while separating
//! manifest/config reads, external CLI parsing, and host visibility
//! classification.

use super::*;
#[allow(unused_imports)]
use super::{actions::*, model::*};

mod cli_probe;
mod config;
mod visibility;

pub(super) use cli_probe::*;
pub(super) use config::*;
pub(super) use visibility::*;
