//! Front-stage / facade command action sub-enums.

use clap::Subcommand;
use std::path::PathBuf;

/// Unified public onboarding lifecycle.
#[derive(Subcommand)]
pub(crate) enum OnboardingAction {
    /// Assess the active machine/project and emit a deterministic action plan.
    Plan {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Show the current readiness states without applying anything.
    Status {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Apply exactly one reviewed item from the current deterministic plan.
    Apply {
        #[arg(long)]
        item: String,
        /// Exact plan hash printed by `ags onboarding plan`. Required so the
        /// reviewed packaged manifest and current machine facts cannot drift
        /// between review and apply.
        #[arg(long)]
        plan_hash: String,
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        /// Required confirmation. There is no batch or implicit apply mode.
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Re-assess readiness and fail when a required component is not ready.
    Verify {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Cross-Agent host capability layer.
#[derive(Subcommand)]
pub(crate) enum CapabilityAction {
    /// Cross-Agent capability inventory with per-host thin-index visibility.
    ///
    /// Unified view of skills + governed MCPs + CLI-backed capabilities and
    /// whether each is visible to each host. Read-only.
    Inventory {
        /// Host to scope visibility to (repeatable). Default: claude-code + codex + codebuddy-code.
        #[arg(long = "host")]
        host: Vec<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify cross-Agent host visibility and required capability coverage (read-only).
    ///
    /// Resolves the installed AGS capability authority independently of the
    /// current project directory. Missing required registry parents remain in
    /// the expected set and fail closed. `ags skill verify` is the public
    /// human-facing facade for the same host verification. Claude Code, Codex,
    /// Cursor, OMP, and CodeBuddy-Code are supported capability identities.
    Verify {
        /// Host to verify: claude-code | codex | cursor | omp | codebuddy-code
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (post-apply gate).
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Derive the machine-local Skill/MCP capability snapshot + attestation hash.
    ///
    /// Captures the strict intersection of governed routable skills that are
    /// healthy and visible to one active host, plus a deterministic
    /// `snapshot_hash`. The registry stays authoritative for what MAY route.
    /// With `--write` the snapshot is written to the machine-local runtime home
    /// (never tracked, never published).
    Snapshot {
        /// Active host whose routable skill table is captured.
        #[arg(long = "host", default_value = "codex")]
        host: String,
        /// Project root path (default: current directory).
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Write the snapshot JSON to the machine-local runtime home.
        #[arg(long)]
        write: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Skill discovery and machine-local lifecycle commands.
#[derive(Subcommand)]
pub(crate) enum SkillAction {
    /// Browse the AGS recommendation catalog. Recommendations are discovery
    /// facts, never an installation allowlist.
    Recommend {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Inspect a catalog id, GitHub URL, or local path and
    /// produce a hash-bound plan without applying it.
    Inspect {
        source: String,
        #[arg(long)]
        requested_ref: Option<String>,
        /// Existing YAML routing metadata file; inline YAML is not accepted.
        #[arg(long, value_name = "FILE")]
        metadata: Option<PathBuf>,
        #[arg(long = "host")]
        host: Vec<String>,
        #[arg(long, default_value = "notify", value_parser = ["notify", "manual", "pinned"])]
        update_policy: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or apply installation from a catalog id or arbitrary GitHub URL.
    Install {
        source: String,
        #[arg(long)]
        requested_ref: Option<String>,
        /// Existing YAML routing metadata file; inline YAML is not accepted.
        #[arg(long, value_name = "FILE")]
        metadata: Option<PathBuf>,
        #[arg(long = "host")]
        host: Vec<String>,
        #[arg(long, default_value = "notify", value_parser = ["notify", "manual", "pinned"])]
        update_policy: String,
        /// Exact risk acknowledgement id from the reviewed plan; repeatable.
        #[arg(long = "ack-risk")]
        acknowledged_risks: Vec<String>,
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Resolve a candidate for one installed Skill without applying it.
    Check {
        skill_id: Option<String>,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or apply one installed Skill update.
    Update {
        skill_id: String,
        #[arg(long = "ack-risk")]
        acknowledged_risks: Vec<String>,
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or apply rollback to a retained immutable body revision. Without
    /// --revision, selects the most recent revision before the current body.
    Rollback {
        skill_id: String,
        #[arg(long)]
        revision: Option<String>,
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or explicitly adopt one audited local Skill directory into the
    /// machine-local installed index. The default is plan-only.
    Adopt {
        /// Local Skill directory or its SKILL.md file.
        source: PathBuf,
        /// Optional machine-local YAML routing metadata. It is audited and
        /// hash-bound to the reviewed plan, but never copied into AGS Git.
        #[arg(long, value_name = "FILE")]
        metadata: Option<PathBuf>,
        /// Target host id; repeatable. Empty selects the Hosts approved by setup;
        /// `all` explicitly selects every supported Host.
        #[arg(long = "host")]
        host: Vec<String>,
        #[arg(long, default_value = "notify", value_parser = ["notify", "manual", "pinned"])]
        update_policy: String,
        /// Exact hash from the reviewed plan. Required with --yes.
        #[arg(long)]
        plan_hash: Option<String>,
        /// Acknowledge one deterministic risk finding from the reviewed plan;
        /// repeat once per accepted finding.
        #[arg(long = "ack-risk")]
        acknowledged_risks: Vec<String>,
        /// Confirm the reviewed machine-local writes.
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or explicitly remove one machine-local adopted Skill. Immutable
    /// body revisions remain available for recoverable rollback.
    Remove {
        skill_id: String,
        /// Exact hash from the reviewed removal plan. Required with --yes.
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Inspect catalog, installed index, immutable body, host indexes, and active snapshots.
    Status {
        skill_id: Option<String>,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify one installed Skill's exact routes, or reverify a closed
    /// maintenance plan.
    Verify {
        skill_id: Option<String>,
        /// Verify the full apply/activate/preflight/route loop for this Plan.
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        strict: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Agent host governance — five-segment stage 2 (纳管本机 Agent 宿主).
#[derive(Subcommand)]
pub(crate) enum AgentsAction {
    /// Scan local Agent hosts and AGS MCP registration (read-only).
    /// 盘点本机 Agent 宿主与 AGS MCP 注册状态。只读，不写任何配置。
    Scan {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan host onboarding; --apply installs AGS-owned memory lifecycle wiring.
    ///
    /// Default dry-run. `--apply` writes only AGS-owned Claude Code, Codex,
    /// Cursor, CodeBuddy, or OMP workspace lifecycle adapters. External MCP
    /// registrars remain advice-only.
    Govern {
        /// Limit to one host id (claude-code|codex|omp|cursor|workbuddy|codebuddy-code).
        #[arg(long)]
        agent: Option<String>,
        /// Canonical workspace to inspect or receive the adapter.
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Install the selected supported host's AGS memory lifecycle adapter.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify capability visibility plus the host-native memory lifecycle.
    /// 校验宿主能力可见性及原生记忆闭环。
    Verify {
        /// Host to verify: claude-code | codex | omp | codebuddy-code | cursor
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (post-apply gate).
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Signed AGS core update. The shared npm launcher owns network, artifact and
/// pointer mutations before the Rust kernel starts; the Rust command remains a
/// fail-closed fallback for direct, unlaunched binaries.
#[derive(Subcommand)]
pub(crate) enum UpdateAction {
    /// Check the signed release index without applying anything.
    Check {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Configure lazy signed update notices without applying an update.
    Config {
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        ignore_version: Option<String>,
        #[arg(long)]
        snooze_until_unix: Option<u64>,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Download and verify the candidate, then seal an immutable update plan.
    Plan {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Read one exact persisted plan and its current receipt state.
    Status {
        #[arg(long)]
        plan_hash: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Apply one exact signed update plan.
    Apply {
        #[arg(long)]
        plan_hash: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify the active pointer and executable against an applied receipt.
    Verify {
        #[arg(long)]
        plan_hash: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Recover the previous verified public launcher version. The npm CLI
    /// intercepts this before entering Rust and atomically switches the shared
    /// current/previous pointer.
    Recover {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

// ── Top-level Commands ────────────────────────────────────────────────────
