//! Confirmation-protected capability mutations.
//!
//! The facade preserves the console API while separating proposal/model
//! construction, mutation planning, filesystem apply, and rollback.

use super::*;
#[allow(unused_imports)]
use super::{actions::*, host_probe::*, host_verify::*, inventory::*, model::*, rendering::*};

mod apply;
mod model;
mod mutation_plan;
mod proposal;
mod rollback;

pub use model::{AdvisedCommand, ConsoleProposalResult, PlannedWrite};
pub use proposal::{distribute_external_skill, propose_action, remove_external_skill_distribution};

#[allow(unused_imports)]
pub(super) use apply::*;
pub(super) use model::{ActionPlan, AppliedChange, ApplyOutcome};
#[allow(unused_imports)]
pub(super) use mutation_plan::*;
#[allow(unused_imports)]
pub(super) use proposal::{dry_run_note, managed_status_str, propose_action_inner};
#[allow(unused_imports)]
pub(super) use rollback::*;
