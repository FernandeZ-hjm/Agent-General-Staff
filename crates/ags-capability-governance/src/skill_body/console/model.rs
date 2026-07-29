use super::*;
pub const CONSOLE_SCHEMA_VERSION: &str = "0.3.6-skill-console";

// ── Host-adapter runner seam ────────────────────────────────────────────────

pub use ags_host_integration::{
    HostProbeExecution as CommandOutcome, HostProbeRunner as CommandRunner,
    SystemHostProbeRunner as SystemCommandRunner,
};

// ── Injectable context (testability seam) ───────────────────────────────────

/// All filesystem roots and the host-protocol runner the console reads through.
/// Production builds it with the real repo root, real `$HOME`, and the system
/// command runner; tests inject temp dirs and a mock runner.
pub struct ConsoleContext {
    pub repo_root: PathBuf,
    pub home: PathBuf,
    pub(super) runner: Box<dyn CommandRunner>,
}

impl ConsoleContext {
    /// Build an explicit context — used by tests with temp dirs + mock runner.
    pub fn new(
        repo_root: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        runner: Box<dyn CommandRunner>,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            home: home.into(),
            runner,
        }
    }

    /// Production context: real repo root, real `$HOME`, system command runner.
    pub fn system(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            home: default_home(),
            runner: Box::new(SystemCommandRunner),
        }
    }
}

/// Resolve the user home directory (Windows-aware), falling back to ".".
pub(super) fn default_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ── Unified capability model ─────────────────────────────────────────────────

/// What kind of managed capability this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedKind {
    /// A skill (suite-managed or discovered on disk).
    Skill,
    /// A governed third-party MCP server.
    Mcp,
    /// AGS self — host initialization adapter, NOT a governed third-party MCP.
    SuiteInterface,
    /// A capability fronted by an external CLI (e.g. `lark-cli`).
    CliBacked,
}

/// Whether and how AGS governs this capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedStatus {
    /// Adopted into the suite manifest (required/optional/personal).
    SuiteManaged,
    /// A governed third-party MCP (in `mcps:`).
    Governed,
    /// AGS self — host initialization adapter (governance authority).
    SuiteInterface,
    /// Present/known but not declared in the reviewed registry — an opt-in candidate. Covers
    /// repo-local skills outside the manifest AND user-installed skills
    /// discovered on disk in a host skills dir (discovered-local).
    Discovered,
    /// A host built-in / system skill (e.g. a Codex `.system` skill such as
    /// `skill-creator`). Recognized READ-ONLY: AGS never holds, copies, or
    /// relinks the body. Fail-closed not-routable until a reviewed registry
    /// release includes it.
    HostSystem,
    /// A skill scoped to a project repo other than the AGS suite (its canonical
    /// body resolves inside another git project). Read-only recognition only.
    ProjectLocal,
    /// Present but outside AGS governance.
    Unmanaged,
    /// An internal entrypoint route target (playbook / MCP tool / CLI
    /// subcommand) of a real parent capability. Routing-only: never a host
    /// body, never installed / refreshed / relinked, never the primary itself.
    RouteTarget,
}

/// Whether the capability is recorded in an AGS registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryStatus {
    /// Present in suite.yaml or mcp-registry.yaml.
    Registered,
    /// Not in any AGS registry.
    NotRegistered,
}

/// Host visibility status — whether a host can actually see/load the capability.
/// Distinct from runtime health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostVisibilityStatus {
    /// Host can load the skill / the MCP is registered.
    Visible,
    /// Checked, not found.
    NotVisible,
    /// Could not fully verify (e.g. host CLI unavailable, dangling symlink).
    Degraded,
    /// This host's check is not implemented in this version.
    Unsupported,
    /// Reserved for a later phase (model fields stable).
    Deferred,
}

/// Runtime health — distinct from host visibility. A skill file existing, a
/// host loading the skill, an MCP being registered, an MCP being connected, and
/// an external endpoint passing a doctor check are all different evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unknown,
    Unhealthy,
}

/// Per-host visibility evidence for a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVisibility {
    pub host: String,
    /// Whether this host's check is implemented in this version.
    pub supported: bool,
    pub status: HostVisibilityStatus,
    pub evidence: Vec<String>,
}

/// What kind of state a capability mutates when invoked. Retained as inventory
/// metadata; execution authority is owned by Policy/Gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MutationSurface {
    /// Read / analyze only (e.g. diagnosing-bugs, codebase-design).
    #[default]
    ReadOnly,
    /// Writes inside the local working tree (e.g. tdd).
    LocalWrite,
    /// Talks to an external account / service (e.g. lark-*).
    ExternalWrite,
}

/// Relative invocation cost, a deterministic routing tie-break (cheaper
/// preferred when route priorities tie).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CostClass {
    /// No meaningful cost (local skill prompt).
    #[default]
    Free,
    /// Local compute only (e.g. a local CLI).
    Local,
    /// Requires a network round-trip.
    Network,
    /// Billed / metered external service.
    Paid,
}

/// Whether a managed capability may enter ActiveSkillTable. Fail-closed by
/// construction: the serde default is `NotRoutable`, so a capability is NEVER
/// silently routed merely by carrying routing fields — only an explicit
/// `route_state: routable` makes it a routing candidate. `Retired` keeps the row
/// for history / dedupe but is excluded from routing exactly like `NotRoutable`.
/// (Capabilities with no `routing:` block at all are absent from the routing map
/// entirely — see `collect_routing` — so this enum only ever applies to members
/// that authored a routing block.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RouteState {
    /// Explicitly eligible for deterministic skill resolution.
    Routable,
    /// Intentionally never routed (e.g. AGS ops commands, personal packs). The
    /// fail-closed default: absence of an explicit state reads as not-routable.
    #[default]
    NotRoutable,
    /// Was routable, now decommissioned; retained for history, excluded from
    /// routing (never `Available`, never `primary`).
    Retired,
}

/// Per-member positive / negative request examples that drive the hermetic route
/// smoke. LABEL-LEVEL test fixtures only — never inherited from a group and never
/// an input to production routing; they cannot change a live route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RouteExamples {
    /// Short user-request samples that SHOULD route to this member.
    #[serde(default)]
    pub positive: Vec<String>,
    /// Short user-request samples that should NOT route to this member.
    #[serde(default)]
    pub negative: Vec<String>,
}

/// Reference to the real, host-visible PARENT capability an internal entrypoint
/// belongs to. When a routing block carries this, the member is a route target
/// (an internal entrypoint such as a superpowers playbook, an MCP tool, or a CLI
/// subcommand), NOT a standalone body: it never produces `expected_hosts`, never
/// enters install / refresh, and is never the `primary` itself — primary
/// derefs to the parent and availability is inherited from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRef {
    /// The parent capability kind (`skill` / `mcp` / `cli-backed`).
    pub kind: ManagedKind,
    /// The parent capability name (must be a real host-visible body).
    pub name: String,
}

/// The kind of an internal entrypoint within a parent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EntrypointKind {
    /// A superpowers (or similar skill) playbook.
    #[default]
    Playbook,
    /// An MCP server tool.
    Tool,
    /// A CLI command.
    Command,
    /// A CLI subcommand.
    Subcommand,
    /// A skill / MCP prompt.
    Prompt,
}

/// The specific internal entrypoint a route target points at (e.g. the
/// `verification-before-completion` playbook of `superpowers`, or the
/// `get-library-docs` tool of `context7`). Display / routing metadata only —
/// the host always invokes the parent body, never the entrypoint standalone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrypointRef {
    pub kind: EntrypointKind,
    pub name: String,
}

/// Stable routing facts declared in a manifest (`skills-registry.yaml` /
/// `mcp-registry.yaml`) and read into the inventory. This is the SINGLE source
/// of truth for deterministic skill eligibility — there is no built-in fallback
/// table. Only *stable facts* live here; the runtime `auth_status` (whether an
/// account is actually configured) is DERIVED at route time and is NEVER stored
/// in a tracked manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingMetadata {
    /// Demand categories this capability serves (e.g. `["debug","root-cause"]`).
    #[serde(default)]
    pub intent_tags: Vec<String>,
    /// Domain scopes (e.g. `["rust"]`; `["*"]` for any).
    #[serde(default)]
    pub scope_tags: Vec<String>,
    /// What the capability mutates when invoked.
    #[serde(default)]
    pub mutation_surface: MutationSurface,
    /// Whether invoking it needs an external account / credential.
    #[serde(default)]
    pub requires_auth: bool,
    /// What kind of auth it needs (e.g. `"feishu-account"`), advisory only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_kind: Option<String>,
    /// Relative invocation cost (routing tie-break).
    #[serde(default)]
    pub cost_class: CostClass,
    /// Explicit wakeup hint the host emits (e.g. `"[skill: diagnosing-bugs]"`).
    /// AGS NEVER auto-invokes — this is a suggestion string only.
    #[serde(default)]
    pub invoke_hint: String,
    /// Routing priority — lower is preferred.
    #[serde(default = "default_route_priority")]
    pub route_priority: i32,
    /// Explicit routing participation state. Fail-closed default `not-routable`:
    /// a member is a routing candidate only when this is `routable`.
    #[serde(default)]
    pub route_state: RouteState,
    /// Capability (demand) groups this member belongs to — LABELS ONLY, no
    /// inherited routing / policy values. A member may serve several demand
    /// pools (e.g. requesting-code-review in {code-review, verification}).
    #[serde(default)]
    pub capability_group: Vec<String>,
    /// Upstream source group (e.g. `"obra/superpowers:requesting-code-review"`) — LABEL ONLY,
    /// for update / dedupe / provenance; never inherits routing values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_group: Option<String>,
    /// Positive / negative request examples driving the hermetic route smoke.
    #[serde(default)]
    pub examples: RouteExamples,
    /// When set, this routing block belongs to an internal ENTRYPOINT of the
    /// named real, host-visible parent capability (i.e. the member is a route
    /// target). The route target never produces `expected_hosts`, never enters
    /// install / refresh, and is never the `primary` itself — primary
    /// derefs to this parent and availability is inherited from it. A real body
    /// leaves this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    /// The specific internal entrypoint this route target points at. Display /
    /// routing metadata only; the host invokes the parent body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<EntrypointRef>,
}

pub(super) fn default_route_priority() -> i32 {
    100
}

impl Default for RoutingMetadata {
    fn default() -> Self {
        Self {
            intent_tags: Vec::new(),
            scope_tags: Vec::new(),
            mutation_surface: MutationSurface::default(),
            requires_auth: false,
            auth_kind: None,
            cost_class: CostClass::default(),
            invoke_hint: String::new(),
            route_priority: default_route_priority(),
            route_state: RouteState::default(),
            capability_group: Vec::new(),
            upstream_group: None,
            examples: RouteExamples::default(),
            parent: None,
            entrypoint: None,
        }
    }
}

/// A single managed capability in the unified inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedCapability {
    pub kind: ManagedKind,
    pub name: String,
    pub source: Option<String>,
    /// Suite profile for skills (`required` / `optional` / `personal` / …).
    pub profile: Option<String>,
    pub managed_status: ManagedStatus,
    pub registry_status: RegistryStatus,
    /// Whether AGS holds the canonical body (the one managed copy: a skill dir
    /// with SKILL.md, an MCP definition, etc.). Distinct from host visibility:
    /// hosts only carry a thin index pointing back at this canonical body.
    pub canonical_present: bool,
    /// Hosts where this capability is *expected* to be visible (drives the
    /// verify failure signal). Empty = opt-in / not-applicable for any host.
    pub expected_hosts: Vec<String>,
    /// Per-host thin-index visibility (the discoverable entry each host owns).
    pub host_visibility: Vec<HostVisibility>,
    pub health_status: HealthStatus,
    pub risk_notes: Vec<String>,
    /// Stable routing facts from the manifest (Capability Resolver input). `None`
    /// when the manifest declares no `routing:` block — production routing does
    /// NOT fall back to a built-in table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingMetadata>,
}

impl ManagedCapability {
    /// Whether this is an internal-entrypoint route target — it carries a
    /// `routing.parent`. Route targets are routing-only: no `expected_hosts`, no
    /// install / refresh / relink, and never the `primary` themselves
    /// (primary derefs to the parent capability).
    pub fn is_route_target(&self) -> bool {
        self.routing.as_ref().is_some_and(|r| r.parent.is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInventorySummary {
    pub total: usize,
    pub skills: usize,
    pub mcps: usize,
    pub suite_interfaces: usize,
    pub cli_backed: usize,
    /// Count whose canonical body is present in the AGS store.
    pub canonical_present: usize,
    /// Count visible to Claude Code (host_visibility status == visible).
    pub claude_visible: usize,
    pub risk_flagged: usize,
    /// Skill-resolution coverage — members by route_state.
    #[serde(default)]
    pub routing_routable: usize,
    #[serde(default)]
    pub routing_not_routable: usize,
    #[serde(default)]
    pub routing_retired: usize,
    /// Adopted (suite-managed / governed) members with NO routing block — the
    /// coverage gap the doctor coverage check flags. 0 = full coverage.
    #[serde(default)]
    pub routing_uncovered: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInventoryResult {
    pub schema_version: String,
    pub hosts: Vec<String>,
    pub capabilities: Vec<ManagedCapability>,
    pub summary: ManagedInventorySummary,
    pub note: String,
    /// Names of capabilities whose `routing:` block was present but failed to
    /// parse. Surfaced (not silently swallowed) so doctor / inventory can flag
    /// routing schema drift. Empty in the healthy case.
    #[serde(default)]
    pub routing_parse_failures: Vec<String>,
}
