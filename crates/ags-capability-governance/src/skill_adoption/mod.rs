//! Machine-private third-party Skill adoption.
//!
//! The module owns source audit, private provenance, immutable bodies, host
//! thin indexes, and snapshot publication behind a plan/apply interface.

mod model;
mod projection;
mod source;
mod store;
mod transaction;

pub use model::*;
pub use projection::inspect_adoption;
pub use store::{bodies_root, body_path, load_registry, registry_path};
pub use transaction::{apply_adoption, apply_removal, plan_adoption, plan_removal};

pub(crate) use projection::project_private_skills;
