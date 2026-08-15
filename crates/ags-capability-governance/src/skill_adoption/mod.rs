//! Machine-local third-party Skill adoption.
//!
//! The module owns source audit, local provenance, immutable bodies, host
//! thin indexes, and snapshot publication behind a plan/apply interface.

mod materialize;
mod model;
mod projection;
mod remote;
mod source;
mod store;
mod transaction;

pub use materialize::materialize_skill_change;
pub use model::*;
pub use projection::{inspect_adoption, verify_adoption_routes, verify_adoption_routes_batch};
pub use remote::{
    acquire_remote_candidate, acquire_remote_candidate_with_backend, GitBackend, HeldCheckout,
    RemoteCandidate, RemoteTreeEntry, RemoteTreeEntryKind, SystemGitBackend,
};
pub use source::{parse_github_source, parse_github_url, parse_github_url_with_ref};
pub use store::{
    bodies_root, body_path, installed_skill_index_hash, installed_skill_index_path,
    load_installed_skills,
};
pub use transaction::{
    plan_install, plan_install_with_backend, plan_legacy_catalog_migration, plan_removal,
    plan_rollback, plan_update, plan_update_with_backend,
};

pub(crate) use projection::project_installed_skills_with_overlay;
