//! Project initialization, overlay, refresh, and verification lifecycle.

mod apply;
mod execute;
mod managed_projects;
mod model;
pub mod overlay;
mod plan;
mod render;

pub use execute::{execute, InitOutput, InitRequest};
pub use managed_projects::{
    refresh_managed_project, ManagedProjectRefresh, ManagedProjectRegistration,
};
pub use model::{InitCheckStatus, InitFile, InitFinding, InitReport, InitSeverity};
pub use plan::ProjectInitPlan;
