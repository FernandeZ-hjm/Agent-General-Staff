//! Canonical diagnostic report vocabulary.
//!
//! Setup, update, doctor, and verification share one report model. Lifecycle
//! owns the model because it produces setup findings; verification adds only
//! checks and renderers rather than translating the same facts.

pub use ags_lifecycle::setup::{
    SetupCheckStatus as CheckStatus, SetupFinding as Finding, SetupReport as HealthReport,
    SetupSeverity as Severity,
};
