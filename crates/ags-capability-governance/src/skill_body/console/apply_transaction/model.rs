use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedWrite {
    /// "create" | "overwrite" | "backup" | "remove"
    pub op: String,
    pub path: String,
    pub from: Option<String>,
    pub detail: String,
}

/// An external command a human must run in their host. AGS NEVER executes these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisedCommand {
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleProposalResult {
    pub schema_version: String,
    pub action: String,
    pub capability: String,
    pub found: bool,
    pub kind: Option<String>,
    pub managed_status: Option<String>,
    pub apply_requested: bool,
    /// True ONLY when ≥1 AGS-owned write was planned AND every one succeeded.
    /// Never true for advised-only (MCP/CLI) actions — AGS performed nothing.
    pub applied: bool,
    /// "dry-run" | "applied" | "failed" | "advised-only" | "nothing-to-do" | "blocked"
    pub apply_status: String,
    pub planned_writes: Vec<PlannedWrite>,
    pub applied_writes: Vec<String>,
    /// Per-write failures during apply. Non-empty ⇒ apply did NOT fully succeed
    /// and `applied` is false; the CLI exits nonzero.
    pub apply_errors: Vec<String>,
    /// External installer/registrar commands AGS will NOT run on your behalf.
    pub advised_commands: Vec<AdvisedCommand>,
    pub blocked_reasons: Vec<String>,
    pub risk_notes: Vec<String>,
    pub note: String,
}

#[derive(Default)]
pub(in super::super) struct ActionPlan {
    pub(super) writes: Vec<PlannedWrite>,
    pub(super) advised: Vec<AdvisedCommand>,
    pub(super) blocked: Vec<String>,
    pub(super) notes: Vec<String>,
}
/// Outcome of a guarded apply: writes that succeeded, and per-write errors.
/// Errors are kept separate from `applied_writes` so the caller has a real
/// failure signal (rather than `ERROR ...` buried in the success list).
#[derive(Default)]
pub(in super::super) struct ApplyOutcome {
    pub(in super::super) applied_writes: Vec<String>,
    pub(in super::super) errors: Vec<String>,
}

#[derive(Debug)]
pub(in super::super) enum AppliedChange {
    CreatedDir(PathBuf),
    Relink {
        entry: PathBuf,
        previous: Option<PathBuf>,
    },
    Unlink {
        entry: PathBuf,
        backup: PathBuf,
    },
}
