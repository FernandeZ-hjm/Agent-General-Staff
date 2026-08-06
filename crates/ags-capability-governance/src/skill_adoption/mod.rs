//! Machine-local third-party Skill adoption.
//!
//! The module owns source audit, local provenance, immutable bodies, host
//! thin indexes, and snapshot publication behind a plan/apply interface.

mod model;
mod projection;
mod remote;
mod source;
mod store;
mod transaction;

pub use model::*;
pub use projection::{inspect_adoption, verify_adoption_routes, verify_adoption_routes_batch};
pub use remote::{
    acquire_remote_candidate, acquire_remote_candidate_with_backend, GitBackend, RemoteCandidate,
    RemoteTreeEntry, RemoteTreeEntryKind, SystemGitBackend,
};
pub use source::{parse_github_source, parse_github_url, parse_github_url_with_ref};
pub use store::{bodies_root, body_path, installed_skill_index_path, load_installed_skills};
pub use transaction::{
    apply_install, apply_install_in_maintenance_transaction,
    apply_reactivation_in_maintenance_transaction, apply_removal, apply_rollback, apply_update,
    plan_install, plan_install_with_backend, plan_legacy_catalog_migration, plan_removal,
    plan_rollback, plan_update, plan_update_with_backend, recover_applied_change,
    recover_applied_change_in_maintenance_transaction, recover_pending_transactions,
    transaction_journal_path, transaction_lock_path,
};

pub(crate) use projection::project_installed_skills;
