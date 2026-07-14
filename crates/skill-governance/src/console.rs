//! AGS third-party skill & MCP management console.
//!
//! This module turns `ags skill` from a static ledger check into a management
//! console over the machine's third-party skills, MCP servers, and CLI-backed
//! capabilities. It provides:
//!
//! - A unified [`ManagedCapability`] model covering suite-managed skills, local
//!   skill directories, governed MCP servers, the AGS suite interface, and
//!   CLI-backed capabilities (e.g. `lark-cli`).
//! - Host-visibility checks ([`verify_host`], [`build_inventory`]) that probe
//!   whether a capability is actually visible to a host (Claude Code skill path
//!   + `claude mcp list`). Codex/Cursor are reserved with stable fields.
//! - A confirmation-protected proposal/apply path ([`propose_action`]) for
//!   adopt / update / remove / uninstall / repair / verify. Without an explicit
//!   `apply` confirmation nothing is written and no external installer runs.
//!
//! ## Safety model
//!
//! Every mutation flows through one guard ([`guarded_apply`]). All write and
//! command targets come from an injectable [`ConsoleContext`] (repo root, home,
//! command runner) so tests use temp dirs and mock commands — never the real
//! `$HOME`. AGS distributes host entry points and *advises* external commands;
//! it never runs `npx skills add/remove`, `lark-cli update`, or `claude mcp
//! add/remove` itself.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONSOLE_SCHEMA_VERSION: &str = "0.2.7-skill-console";

// ── Command runner seam ─────────────────────────────────────────────────────

/// Outcome of attempting to run a host CLI (e.g. `claude mcp list`).
#[derive(Debug, Clone)]
pub enum CommandOutcome {
    /// The command ran to completion (regardless of exit status).
    Ran { success: bool, stdout: String },
    /// The command binary could not be spawned (not installed / not on PATH).
    Unavailable,
}

/// Seam for running host CLIs. Production uses [`SystemCommandRunner`]; tests
/// inject canned responses so no real host CLI is ever invoked.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> CommandOutcome;
}

/// Production runner — spawns the real binary via `std::process::Command`.
/// Read-only host discovery commands only; never used for installers.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> CommandOutcome {
        match std::process::Command::new(program).args(args).output() {
            Ok(out) => CommandOutcome::Ran {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            },
            Err(_) => CommandOutcome::Unavailable,
        }
    }
}

// ── Injectable context (testability seam) ───────────────────────────────────

/// All filesystem roots and the command runner the console reads through.
/// Production builds it with the real repo root, real `$HOME`, and the system
/// command runner; tests inject temp dirs and a mock runner.
pub struct ConsoleContext {
    pub repo_root: PathBuf,
    pub home: PathBuf,
    runner: Box<dyn CommandRunner>,
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
fn default_home() -> PathBuf {
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
    /// Present/known but not yet adopted — an opt-in candidate. Covers
    /// repo-local skills outside the manifest AND user-installed skills
    /// discovered on disk in a host skills dir (discovered-local).
    Discovered,
    /// A host built-in / system skill (e.g. a Codex `.system` skill such as
    /// `skill-creator`). Recognized READ-ONLY: AGS never holds, copies, or
    /// relinks the body. Fail-closed not-routable until explicitly adopted.
    HostSystem,
    /// A skill scoped to a project repo other than the AGS suite (its canonical
    /// body resolves inside another git project). Read-only recognition only.
    ProjectLocal,
    /// Explicitly ignored (in the ignore list / rejected in adoption log).
    Ignored,
    /// Present but outside AGS governance.
    Unmanaged,
    /// An internal entrypoint route target (playbook / MCP tool / CLI
    /// subcommand) of a real parent capability. Routing-only: never a host
    /// body, never adopted / synced / relinked, never the primary itself.
    RouteTarget,
}

/// Whether the capability is recorded in an AGS registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryStatus {
    /// Present in suite.yaml / mcp-registry.yaml / adoption log.
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

/// What kind of state a capability mutates when invoked. Used by Capability
/// Route to cross-check a routed capability against the Value Route posture.
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

/// Whether a managed capability participates in Capability Route. Fail-closed by
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
    /// Explicitly participates in Capability Route.
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
/// enters sync / apply / propose, and is never the `primary` itself — primary
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
/// of truth for production Capability Route — there is no built-in fallback
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
    /// `true` for compatibility aliases (the `auto-*` skills) that should win
    /// their demand ahead of their canonical successors.
    #[serde(default)]
    pub is_compatibility_alias: bool,
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
    /// sync / apply / propose, and is never the `primary` itself — primary
    /// derefs to this parent and availability is inherited from it. A real body
    /// leaves this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    /// The specific internal entrypoint this route target points at. Display /
    /// routing metadata only; the host invokes the parent body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<EntrypointRef>,
}

fn default_route_priority() -> i32 {
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
            is_compatibility_alias: false,
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
    /// Management actions the console offers for this capability.
    pub actions: Vec<String>,
    pub risk_notes: Vec<String>,
    /// Stable routing facts from the manifest (Capability Route input). `None`
    /// when the manifest declares no `routing:` block — production routing does
    /// NOT fall back to a built-in table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingMetadata>,
}

impl ManagedCapability {
    /// Whether this is an internal-entrypoint route target — it carries a
    /// `routing.parent`. Route targets are routing-only: no `expected_hosts`, no
    /// sync / apply / propose / relink, and never the `primary` themselves
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
    /// Routing coverage (Capability Route) — members by route_state.
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

// ── CLI-backed families ──────────────────────────────────────────────────────

/// A family of skills fronted by an external CLI. The console recognises these
/// so it can distinguish the CLI binary, the `*-cli` family skills, and the
/// external endpoint they ultimately talk to.
struct CliFamily {
    prefix: &'static str,
    cli: &'static str,
    endpoint: &'static str,
}

const CLI_FAMILIES: &[CliFamily] = &[CliFamily {
    prefix: "lark-",
    cli: "lark-cli",
    endpoint: "Feishu / Lark Open Platform",
}];

/// Match a *skill* name to a CLI family. The synthetic CLI capability itself
/// (e.g. `lark-cli`) is excluded so it is not double-classified as a family
/// member.
fn cli_family_for_skill(skill_name: &str) -> Option<&'static CliFamily> {
    CLI_FAMILIES
        .iter()
        .find(|f| skill_name != f.cli && skill_name.starts_with(f.prefix))
}

// ── MCP registry reader ──────────────────────────────────────────────────────

struct RegistryEntry {
    name: String,
    manager: Option<String>,
    suite_interface: bool,
    /// Host clients the registry declares this server installed in
    /// (`install.installed_clients`). Used to decide expected host visibility.
    installed_clients: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrySkillBody {
    name: String,
    profile: Option<String>,
    manager: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequiredRegistrySkill {
    name: String,
    profile: String,
    local_path: Option<String>,
    source_type: Option<String>,
}

/// Read `manifests/mcp-registry.yaml` and return entries from both the
/// `suite_interfaces:` (AGS self) and `mcps:` (governed) sections. Lenient:
/// returns an empty list when the file is missing or unparseable.
fn read_mcp_registry(repo_root: &Path) -> Vec<RegistryEntry> {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (section, is_iface) in [("suite_interfaces", true), ("mcps", false)] {
        if let Some(seq) = doc.get(section).and_then(|v| v.as_sequence()) {
            for item in seq {
                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    let manager = item
                        .get("package")
                        .and_then(|p| p.get("manager"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    let installed_clients = item
                        .get("install")
                        .and_then(|i| i.get("installed_clients"))
                        .and_then(|v| v.as_sequence())
                        .map(|seq| {
                            seq.iter()
                                .filter_map(|v| v.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    out.push(RegistryEntry {
                        name: name.to_string(),
                        manager,
                        suite_interface: is_iface,
                        installed_clients,
                    });
                }
            }
        }
    }
    out
}

// ── Routing metadata reader (Capability Route, manifest = single authority) ───

/// Result of reading routing metadata from the manifests: the parsed per-member
/// `map` plus the `parse_failures` — names of members whose `routing:` block was
/// present but failed to parse. Failures are SURFACED (not silently swallowed),
/// so doctor / inventory can flag routing schema drift while routing itself
/// stays fail-closed (a failed member is absent from `map` → never routed).
#[derive(Debug, Clone, Default)]
pub struct RoutingRead {
    pub map: HashMap<String, RoutingMetadata>,
    external_skill_bodies: HashMap<String, RegistrySkillBody>,
    required_skill_parents: Vec<RequiredRegistrySkill>,
    /// Internal-entrypoint route targets declared under a `route_targets:`
    /// section — (name, routing) pairs synthesized into route-target inventory
    /// rows. Each routing carries a `parent`; these are NEVER standalone bodies.
    pub route_targets: Vec<(String, RoutingMetadata)>,
    pub parse_failures: Vec<String>,
}

/// Read stable routing metadata declared in `manifests/skills-registry.yaml`
/// (per skill) and `manifests/mcp-registry.yaml` (per MCP / suite interface),
/// keyed by capability name. This is the ONLY source of production routing
/// metadata — there is no built-in fallback table. Lenient: missing or
/// unparseable files yield an empty map, and entries without a `routing:` block
/// are simply absent (never synthesized). A present-but-malformed block is
/// absent from the map AND recorded in `parse_failures`.
fn read_routing_metadata(repo_root: &Path) -> RoutingRead {
    let mut read = RoutingRead::default();

    let manifests = [
        (
            repo_root.join("manifests/skills-registry.yaml"),
            &["skills"][..],
        ),
        (
            repo_root.join("manifests/mcp-registry.yaml"),
            &["suite_interfaces", "mcps"][..],
        ),
    ];
    for (path, member_sections) in manifests {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
            continue;
        };
        for section in member_sections {
            if let Some(seq) = doc.get(*section).and_then(|v| v.as_sequence()) {
                for item in seq {
                    collect_routing(item, &mut read);
                }
            }
        }
        if let Some(seq) = doc.get("route_targets").and_then(|v| v.as_sequence()) {
            for item in seq {
                collect_route_target(item, &mut read);
            }
        }
    }

    read
}

/// Parse one registry entry's `name` + optional `routing:` block. An entry
/// without `routing:` is skipped (no synthesis). A malformed `routing:` block is
/// kept OUT of the map (fail-closed: never routed) but its name is recorded in
/// `parse_failures` rather than silently swallowed, so schema drift is visible.
fn collect_routing(item: &serde_yaml::Value, read: &mut RoutingRead) {
    let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    if let Some(source) = item.get("source") {
        let external = source.get("type").and_then(|v| v.as_str()) == Some("external_cli_skill");
        let manager = source
            .get("manager")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|manager| !manager.is_empty());
        if external && is_safe_path_component(name) {
            if let Some(manager) = manager {
                read.external_skill_bodies.insert(
                    name.to_string(),
                    RegistrySkillBody {
                        name: name.to_string(),
                        profile: item
                            .get("profile")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string),
                        manager: manager.to_string(),
                    },
                );
            }
        }
    }
    let Some(routing_val) = item.get("routing") else {
        return;
    };
    match serde_yaml::from_value::<RoutingMetadata>(routing_val.clone()) {
        Ok(meta) => {
            if item.get("profile").and_then(|v| v.as_str()) == Some("required")
                && meta.route_state == RouteState::Routable
                && meta.parent.is_none()
                && is_safe_path_component(name)
            {
                read.required_skill_parents.push(RequiredRegistrySkill {
                    name: name.to_string(),
                    profile: "required".to_string(),
                    local_path: item
                        .get("local_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    source_type: item
                        .get("source")
                        .and_then(|source| source.get("type"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                });
            }
            read.map.insert(name.to_string(), meta);
        }
        Err(_) => read.parse_failures.push(name.to_string()),
    }
}

/// Parse one `route_targets:` entry into a (name, routing) pair. The routing
/// block MUST carry a `parent` (that is what makes it a route target); a missing
/// `routing:` block, a malformed one, or one without `parent` is recorded in
/// `parse_failures` (fail-closed: never routed, never synthesized).
fn collect_route_target(item: &serde_yaml::Value, read: &mut RoutingRead) {
    let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(routing_val) = item.get("routing") else {
        read.parse_failures.push(name.to_string());
        return;
    };
    match serde_yaml::from_value::<RoutingMetadata>(routing_val.clone()) {
        Ok(meta) if meta.parent.is_some() => {
            read.route_targets.push((name.to_string(), meta));
        }
        _ => read.parse_failures.push(name.to_string()),
    }
}

// ── Host MCP probe ──────────────────────────────────────────────────────────

/// Cached result of probing one host's MCP registry once per inventory.
struct HostMcpProbe {
    /// Whether the host CLI was runnable. False → MCP checks are degraded.
    available: bool,
    /// (server name, connected/enabled) pairs parsed from `<host> mcp list`.
    servers: Vec<(String, bool)>,
}

impl HostMcpProbe {
    fn unavailable() -> Self {
        Self {
            available: false,
            servers: Vec::new(),
        }
    }

    fn find(&self, name: &str) -> Option<bool> {
        self.servers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, connected)| *connected)
    }
}

/// Probe a host's registered MCP servers via its CLI. Read-only. Unknown hosts
/// or a missing CLI yield an unavailable probe (→ degraded, never a panic).
fn probe_host_mcp(ctx: &ConsoleContext, host: &str) -> HostMcpProbe {
    let (program, args): (&str, &[&str]) = match host {
        "claude-code" => ("claude", &["mcp", "list"]),
        "codex" => ("codex", &["mcp", "list"]),
        _ => return HostMcpProbe::unavailable(),
    };
    match ctx.runner.run(program, args) {
        CommandOutcome::Unavailable => HostMcpProbe::unavailable(),
        // A non-zero exit means we could NOT enumerate the registry — treat it
        // as unavailable (→ degraded), not as an authoritative empty list. A
        // parsed empty/partial stdout on failure would wrongly report MCPs as
        // missing/incomplete.
        CommandOutcome::Ran { success: false, .. } => HostMcpProbe::unavailable(),
        CommandOutcome::Ran {
            success: true,
            stdout,
        } => HostMcpProbe {
            available: true,
            servers: if host == "codex" {
                parse_codex_mcp_list(&stdout)
            } else {
                parse_claude_mcp_list(&stdout)
            },
        },
    }
}

/// Parse `claude mcp list` output. Lines look like
/// `name: /path/to/cmd args - ✔ Connected`. Plugin-owned MCP names may contain
/// colons themselves, e.g. `plugin:claude-mem:mcp-search: node ...`, so split
/// on the first `: ` delimiter instead of the first raw colon.
fn parse_claude_mcp_list(stdout: &str) -> Vec<(String, bool)> {
    let mut servers = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, rest)) = line.split_once(": ") else {
            continue;
        };
        let name = name.trim();
        // Server names are single tokens; skip prose/header lines.
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            continue;
        }
        let connected = rest.contains("Connected") || rest.contains('✔') || rest.contains('✓');
        servers.push((name.to_string(), connected));
    }
    servers
}

/// Parse `codex mcp list` output — a whitespace-padded table with columns
/// `Name Command Args Env Cwd Status Auth`. Lenient: the first token of each
/// non-header row is the server name; the `Status` column (`enabled`/`disabled`)
/// is the best available connection signal codex exposes.
fn parse_codex_mcp_list(stdout: &str) -> Vec<(String, bool)> {
    let mut servers = Vec::new();
    for line in stdout.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        // Skip the header row and any rule/separator lines.
        if name == "Name" || name.chars().all(|c| c == '-' || c == '=') {
            continue;
        }
        // `disabled` contains `enabled` as a substring — check it first.
        let enabled = line.contains("enabled") && !line.contains("disabled");
        servers.push((name.to_string(), enabled));
    }
    servers
}

// ── Host visibility computation ───────────────────────────────────────────────

const SUPPORTED_HOSTS: &[&str] = &["claude-code", "codex", "codebuddy-code"];
const DEFERRED_HOSTS: &[&str] = &["cursor"];

/// The `~/<subdir>` skills directory a host loads skill entries from, if any.
/// `Some` ⇒ the host is supported and gets a real probe.
fn host_skills_subdir(host: &str) -> Option<&'static str> {
    match host {
        "claude-code" => Some(".claude/skills"),
        "codex" => Some(".codex/skills"),
        "codebuddy-code" => Some(".codebuddy/skills"),
        _ => None,
    }
}

/// Additional shared skill source loaded by a host. Codex Desktop currently
/// indexes both `~/.codex/skills` and the multi-agent `~/.agents/skills` store;
/// writing the same suite skill into both roots creates duplicate slash-picker
/// entries.
fn shared_skill_dirs_for_host(ctx: &ConsoleContext, host: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if host == "codex" {
        dirs.push(ctx.home.join(".agents/skills"));
        dirs.extend(codex_plugin_skill_dirs(&ctx.home));
    }
    dirs
}

fn codex_plugin_skill_dirs(home: &Path) -> Vec<PathBuf> {
    let cache = home.join(".codex/plugins/cache");
    let mut dirs = Vec::new();
    collect_plugin_skill_dirs(&cache, 0, &mut dirs);
    dirs.sort();
    dirs.dedup();
    dirs
}

fn collect_plugin_skill_dirs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 5 {
        return;
    }
    if dir.file_name().and_then(|n| n.to_str()) == Some("skills") {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            collect_plugin_skill_dirs(&path, depth + 1, out);
        }
    }
}

/// Compute one capability's visibility for one host. `probe` is that host's
/// MCP probe (`None` for reserved hosts).
fn host_visibility(
    ctx: &ConsoleContext,
    host: &str,
    cap_kind: &ManagedKind,
    cap_name: &str,
    canonical_source: Option<&Path>,
    external_shared: bool,
    probe: Option<&HostMcpProbe>,
) -> HostVisibility {
    if let Some(subdir) = host_skills_subdir(host) {
        return match cap_kind {
            ManagedKind::Skill => skill_path_visibility_for_host(
                ctx,
                host,
                &ctx.home.join(subdir),
                cap_name,
                canonical_source,
                external_shared,
            ),
            ManagedKind::Mcp | ManagedKind::CliBacked | ManagedKind::SuiteInterface => {
                host_mcp_visibility(host, cap_name, probe)
            }
        };
    }

    // Reserved hosts: stable fields, deferred status, no probing.
    let deferred = DEFERRED_HOSTS.contains(&host);
    HostVisibility {
        host: host.to_string(),
        supported: false,
        status: if deferred {
            HostVisibilityStatus::Deferred
        } else {
            HostVisibilityStatus::Unsupported
        },
        evidence: vec![format!(
            "Host '{host}' visibility check is not implemented in this version (model fields are stable)."
        )],
    }
}

fn skill_path_visibility_for_host(
    ctx: &ConsoleContext,
    host: &str,
    primary_skills_dir: &Path,
    name: &str,
    canonical_source: Option<&Path>,
    external_shared: bool,
) -> HostVisibility {
    if external_shared && canonical_source.is_none_or(|source| !source.join("SKILL.md").is_file()) {
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::NotVisible,
            evidence: vec![format!(
                "required shared skill body is missing: {}",
                ctx.home
                    .join(".agents/skills")
                    .join(name)
                    .join("SKILL.md")
                    .display()
            )],
        };
    }
    if external_shared
        && canonical_source
            .map(|source| !canonical_within_shared_store(&ctx.home, name, source))
            .unwrap_or(true)
    {
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Degraded,
            evidence: vec![format!(
                "external canonical body is missing or escapes the shared skill store: {}",
                ctx.home.join(".agents/skills").join(name).display()
            )],
        };
    }
    let primary = skill_path_visibility(host, primary_skills_dir, name, canonical_source);
    let shared_visible =
        shared_skill_dirs_for_host(ctx, host)
            .into_iter()
            .find_map(|shared_skills_dir| {
                let direct_external_body = external_shared
                    && canonical_source
                        .is_some_and(|canonical| shared_skills_dir.join(name) == canonical);
                let shared = skill_path_visibility(
                    host,
                    &shared_skills_dir,
                    name,
                    if external_shared && !direct_external_body {
                        canonical_source
                    } else {
                        None
                    },
                );
                if shared.status == HostVisibilityStatus::Visible {
                    Some((shared_skills_dir, shared))
                } else {
                    None
                }
            });
    let Some((shared_skills_dir, shared)) = shared_visible else {
        return primary;
    };

    let mut evidence = Vec::new();
    evidence.push(format!(
        "shared skill source visible under {}",
        shared_skills_dir.display()
    ));
    if primary.status == HostVisibilityStatus::Visible {
        evidence.push(format!(
            "duplicate host entry also exists under {}",
            primary_skills_dir.display()
        ));
    }
    evidence.extend(shared.evidence);
    if primary.status != HostVisibilityStatus::NotVisible {
        evidence.extend(primary.evidence);
    }

    HostVisibility {
        host: host.to_string(),
        supported: true,
        status: HostVisibilityStatus::Visible,
        evidence,
    }
}

fn host_skill_body_dirs(ctx: &ConsoleContext, host: &str, name: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(subdir) = host_skills_subdir(host) {
        roots.push(ctx.home.join(subdir));
    }
    roots.extend(shared_skill_dirs_for_host(ctx, host));
    roots
        .into_iter()
        .map(|root| root.join(name))
        .filter(|body| body.join("SKILL.md").is_file())
        .collect()
}

fn apply_playbook_entrypoint_integrity(
    ctx: &ConsoleContext,
    caps: &mut [ManagedCapability],
    route_targets: &[(String, RoutingMetadata)],
) {
    let mut by_parent: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (_, routing) in route_targets {
        let (Some(parent), Some(entrypoint)) = (&routing.parent, &routing.entrypoint) else {
            continue;
        };
        if parent.kind == ManagedKind::Skill && entrypoint.kind == EntrypointKind::Playbook {
            by_parent
                .entry(parent.name.clone())
                .or_default()
                .push(entrypoint.name.clone());
        }
    }

    for cap in caps.iter_mut() {
        let Some(playbooks) = by_parent.get(&cap.name) else {
            continue;
        };
        let mut degraded = false;
        for visibility in &mut cap.host_visibility {
            if visibility.status != HostVisibilityStatus::Visible {
                continue;
            }
            let bodies = host_skill_body_dirs(ctx, &visibility.host, &cap.name);
            let loadable = bodies.iter().any(|body| {
                playbooks.iter().all(|playbook| {
                    body.join("playbooks")
                        .join(playbook)
                        .join("SKILL.md")
                        .is_file()
                })
            });
            if !loadable {
                visibility.status = HostVisibilityStatus::Degraded;
                visibility.evidence.push(format!(
                    "parent skill is visible but required playbook entrypoint(s) are missing: {}",
                    playbooks.join(", ")
                ));
                degraded = true;
            }
        }
        if degraded {
            cap.health_status = HealthStatus::Degraded;
            cap.risk_notes.push(
                "One or more declared playbook entrypoints are not loadable from the host-visible parent body."
                    .to_string(),
            );
        }
    }
}

fn apply_route_target_exposure_shape(
    caps: &mut [ManagedCapability],
    route_targets: &[(String, RoutingMetadata)],
) {
    let conflicts: Vec<(String, String, Vec<String>)> = route_targets
        .iter()
        .filter_map(|(entrypoint_name, routing)| {
            let parent = routing.parent.as_ref()?;
            let standalone = caps.iter().find(|cap| {
                cap.name == *entrypoint_name
                    && !cap.is_route_target()
                    && cap
                        .host_visibility
                        .iter()
                        .any(|visibility| visibility.status == HostVisibilityStatus::Visible)
            })?;
            let hosts = standalone
                .host_visibility
                .iter()
                .filter(|visibility| visibility.status == HostVisibilityStatus::Visible)
                .map(|visibility| visibility.host.clone())
                .collect();
            Some((entrypoint_name.clone(), parent.name.clone(), hosts))
        })
        .collect();

    for (entrypoint_name, parent_name, hosts) in conflicts {
        if let Some(parent) = caps.iter_mut().find(|cap| cap.name == parent_name) {
            for visibility in &mut parent.host_visibility {
                if hosts.iter().any(|host| host == &visibility.host) {
                    visibility.status = HostVisibilityStatus::Degraded;
                    visibility.evidence.push(format!(
                        "unexpected standalone entrypoint '{entrypoint_name}' is also visible; invoke it only through parent '{parent_name}'"
                    ));
                }
            }
            parent.health_status = HealthStatus::Degraded;
            parent.risk_notes.push(format!(
                "Internal entrypoint '{entrypoint_name}' is exposed as a standalone host skill."
            ));
        }
        if let Some(standalone) = caps.iter_mut().find(|cap| cap.name == entrypoint_name) {
            standalone.risk_notes.push(format!(
                "Unexpected standalone exposure: this entrypoint belongs to parent '{parent_name}'."
            ));
        }
    }
}

/// Skill-path visibility for a host: `<skills_dir>/<name>/SKILL.md`,
/// symlink-aware. Distinguishes loadable, present-but-not-loadable, dangling
/// symlink, and absent. Works for any host's skills dir (Claude / Codex).
fn skill_path_visibility(
    host: &str,
    skills_dir: &Path,
    name: &str,
    canonical_source: Option<&Path>,
) -> HostVisibility {
    let mut evidence = Vec::new();
    let v = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };

    // Refuse to resolve a host path from an unsafe name — a name with `/`, `..`,
    // or an absolute prefix could otherwise stat outside the skills directory.
    if !is_safe_path_component(name) {
        evidence.push(format!(
            "unsafe capability name '{name}' — refusing to resolve a host skill path"
        ));
        return v(HostVisibilityStatus::Degraded, evidence);
    }

    let skill_dir = skills_dir.join(name);
    let skill_md = skill_dir.join("SKILL.md");
    let link_meta = std::fs::symlink_metadata(&skill_dir);

    // Detect a dangling symlink before following it.
    if let Ok(meta) = &link_meta {
        if meta.file_type().is_symlink() {
            if std::fs::metadata(&skill_dir).is_err() {
                evidence.push(format!(
                    "dangling symlink (target missing): {}",
                    skill_dir.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
            evidence.push(format!(
                "skill dir is a symlink with a resolving target: {}",
                skill_dir.display()
            ));
        }
    }

    if !skill_dir.exists() {
        evidence.push(format!("not found under {}", skill_dir.display()));
        return v(HostVisibilityStatus::NotVisible, evidence);
    }
    if let Some(canonical) = canonical_source {
        let Some(meta) = link_meta.ok() else {
            evidence.push(format!(
                "host entry metadata unreadable: {}",
                skill_dir.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        };
        if !meta.file_type().is_symlink() {
            evidence.push(format!(
                "host entry is not a thin-index symlink to the AGS canonical body: {}",
                skill_dir.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        }
        let real_entry = match std::fs::canonicalize(&skill_dir) {
            Ok(p) => p,
            Err(e) => {
                evidence.push(format!(
                    "host thin index target is not canonicalizable: {} ({e})",
                    skill_dir.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
        };
        let real_canonical = match std::fs::canonicalize(canonical) {
            Ok(p) => p,
            Err(e) => {
                evidence.push(format!(
                    "AGS canonical source is not canonicalizable: {} ({e})",
                    canonical.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
        };
        let Some(match_kind) = thin_index_target_match(&real_entry, &real_canonical) else {
            evidence.push(format!(
                "host thin index points to {}, expected AGS canonical {}",
                real_entry.display(),
                real_canonical.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        };
        evidence.push(format!(
            "thin index resolves to {match_kind}: {}",
            real_entry.display()
        ));
    }
    if !skill_md.is_file() {
        evidence.push(format!(
            "dir present but SKILL.md missing: {}",
            skill_md.display()
        ));
        return v(HostVisibilityStatus::NotVisible, evidence);
    }
    match std::fs::read_to_string(&skill_md) {
        Ok(text) => {
            let (parsed_name, _desc) = crate::parse_front_matter(&text);
            match parsed_name.as_deref().map(str::trim) {
                None => {
                    evidence.push(format!(
                        "SKILL.md present but front-matter not parseable: {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Degraded, evidence)
                }
                // The host loads skills by their front-matter `name`. A file at
                // the expected path whose declared name differs is NOT the
                // capability the operator thinks is installed — do not pass it.
                Some(found) if found != name => {
                    evidence.push(format!(
                        "SKILL.md name mismatch: declares '{found}' but expected '{name}' at {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Degraded, evidence)
                }
                Some(_) => {
                    evidence.push(format!(
                        "SKILL.md present and front-matter name matches: {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Visible, evidence)
                }
            }
        }
        Err(e) => {
            evidence.push(format!("SKILL.md unreadable: {} ({e})", skill_md.display()));
            v(HostVisibilityStatus::Degraded, evidence)
        }
    }
}

fn thin_index_target_match(real_entry: &Path, real_canonical: &Path) -> Option<&'static str> {
    if real_entry == real_canonical {
        return Some("AGS canonical body");
    }
    if same_private_stable_suite_path(real_entry, real_canonical) {
        return Some("AGS stable/private runtime twin");
    }
    None
}

fn same_private_stable_suite_path(real_entry: &Path, real_canonical: &Path) -> bool {
    let Some((entry_suite, entry_rel)) = split_suite_runtime_path(real_entry) else {
        return false;
    };
    let Some((canonical_suite, canonical_rel)) = split_suite_runtime_path(real_canonical) else {
        return false;
    };

    entry_suite != canonical_suite && entry_rel == canonical_rel
}

fn split_suite_runtime_path(path: &Path) -> Option<(&'static str, PathBuf)> {
    const SUITE_PREFIX: &str = "agent-governance-suite-";
    const SOURCE_SUFFIX: &str = "private";
    const RUNTIME_SUFFIX: &str = "stable";

    let mut suite = None;
    let mut rel = PathBuf::new();

    for component in path.components() {
        if let Some(found) = suite {
            rel.push(component.as_os_str());
            suite = Some(found);
            continue;
        }
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if let Some(suffix) = name.strip_prefix(SUITE_PREFIX) {
            if suffix == SOURCE_SUFFIX {
                suite = Some("source");
            } else if suffix == RUNTIME_SUFFIX {
                suite = Some("runtime");
            }
        }
    }

    suite.and_then(|found| {
        if rel.components().next().is_some() {
            Some((found, rel))
        } else {
            None
        }
    })
}

/// MCP-registration visibility for a host via its cached `<host> mcp list`.
fn host_mcp_visibility(host: &str, name: &str, probe: Option<&HostMcpProbe>) -> HostVisibility {
    let v = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };
    let Some(probe) = probe else {
        return v(
            HostVisibilityStatus::Degraded,
            vec![format!(
                "no MCP probe available for host '{host}' (degraded)."
            )],
        );
    };
    if !probe.available {
        return v(
            HostVisibilityStatus::Degraded,
            vec![format!(
                "`{host}` MCP CLI unavailable — cannot verify MCP registration (degraded, not a failure)."
            )],
        );
    }
    match probe.find(name) {
        Some(connected) => v(
            HostVisibilityStatus::Visible,
            vec![format!(
                "registered in `{host} mcp list` (enabled/connected: {connected})"
            )],
        ),
        None => v(
            HostVisibilityStatus::NotVisible,
            vec![format!("'{name}' not found in `{host} mcp list`")],
        ),
    }
}

/// Derive runtime health across the probed hosts; kept distinct from host
/// visibility and conservative — never Healthy without positive evidence, and
/// live external endpoints (e.g. Feishu) are only ever a degraded observation.
fn derive_health(
    kind: &ManagedKind,
    name: &str,
    host_vis: &[HostVisibility],
    probes: &[(String, HostMcpProbe)],
    cli_backed_external: bool,
) -> HealthStatus {
    if cli_backed_external {
        return HealthStatus::Degraded;
    }
    match kind {
        ManagedKind::Skill => {
            if host_vis
                .iter()
                .any(|v| v.status == HostVisibilityStatus::Visible)
            {
                HealthStatus::Healthy
            } else if host_vis
                .iter()
                .any(|v| v.status == HostVisibilityStatus::Degraded)
            {
                HealthStatus::Degraded
            } else {
                HealthStatus::Unknown
            }
        }
        ManagedKind::Mcp | ManagedKind::SuiteInterface => {
            let mut any_connected = false;
            let mut any_present = false;
            for (_, p) in probes {
                if let Some(connected) = p.find(name) {
                    any_present = true;
                    any_connected |= connected;
                }
            }
            if any_connected {
                HealthStatus::Healthy
            } else if any_present {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Unknown
            }
        }
        ManagedKind::CliBacked => HealthStatus::Unknown,
    }
}

// ── Action vocabulary ──────────────────────────────────────────────────────────

/// Management verbs the console understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleAction {
    Adopt,
    Update,
    Remove,
    Uninstall,
    Repair,
    Verify,
}

impl ConsoleAction {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "adopt" => Some(Self::Adopt),
            "update" => Some(Self::Update),
            "remove" => Some(Self::Remove),
            "uninstall" => Some(Self::Uninstall),
            "repair" => Some(Self::Repair),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Adopt => "adopt",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Uninstall => "uninstall",
            Self::Repair => "repair",
            Self::Verify => "verify",
        }
    }
}

/// All console action keywords (for CLI value parsing).
pub const CONSOLE_ACTIONS: &[&str] =
    &["adopt", "update", "remove", "uninstall", "repair", "verify"];

/// Default management actions offered for a capability in the inventory.
fn actions_for(kind: &ManagedKind, status: &ManagedStatus) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    match status {
        ManagedStatus::Discovered | ManagedStatus::Unmanaged | ManagedStatus::Ignored => {
            a.push("adopt".to_string());
        }
        ManagedStatus::SuiteManaged | ManagedStatus::Governed => {
            a.push("update".to_string());
            a.push("repair".to_string());
            a.push("remove".to_string());
        }
        // The AGS host initialization adapter cannot be adopted/removed here.
        ManagedStatus::SuiteInterface => {}
        // Host-system / project-local skills are recognized READ-ONLY: AGS never
        // holds the body, so the console offers no adopt/relink — making them
        // routable is a deliberate manifest adoption edit, not a console action.
        ManagedStatus::HostSystem | ManagedStatus::ProjectLocal => {}
        // Route targets are routing-only — no adopt / update / relink / verify.
        ManagedStatus::RouteTarget => return Vec::new(),
    }
    if matches!(kind, ManagedKind::Skill)
        && matches!(
            status,
            ManagedStatus::SuiteManaged | ManagedStatus::Discovered
        )
    {
        a.push("uninstall".to_string());
    }
    a.push("verify".to_string());
    a
}

// ── Inventory ──────────────────────────────────────────────────────────────────

/// Build the unified managed-capability inventory. Read-only. Includes
/// host-visibility evidence for each requested host (default: claude-code).
/// Walk up from `start` (inclusive) looking for a `.git` entry; the nearest
/// ancestor that has one is the project root. `None` when none is found.
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Host thin-index visibility for a capability discovered on disk in a host's
/// skills dir. Checks BOTH the normal entry (`<subdir>/<name>`) and the host
/// system area (`<subdir>/.system/<name>`), so a `.system` skill reads visible
/// where it actually lives. Read-only.
fn host_dir_entry_visibility(home: &Path, host: &str, name: &str) -> HostVisibility {
    let Some(subdir) = host_skills_subdir(host) else {
        let deferred = DEFERRED_HOSTS.contains(&host);
        return HostVisibility {
            host: host.to_string(),
            supported: false,
            status: if deferred {
                HostVisibilityStatus::Deferred
            } else {
                HostVisibilityStatus::Unsupported
            },
            evidence: vec![if deferred {
                "host check reserved for a later phase".to_string()
            } else {
                "host has no skills directory".to_string()
            }],
        };
    };
    let base = home.join(subdir);
    let mk = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };
    // Track the first degraded reason so a valid match at the other location can
    // still win, but a present-but-invalid SKILL.md is never silently passed as
    // Visible.
    let mut degraded: Option<HostVisibility> = None;
    for (loc, dir) in [
        ("entry", base.join(name)),
        ("system", base.join(".system").join(name)),
    ] {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if meta.file_type().is_symlink() && !dir.exists() {
            if degraded.is_none() {
                degraded = Some(mk(
                    HostVisibilityStatus::Degraded,
                    vec![format!(
                        "dangling symlink (target missing): {}",
                        dir.display()
                    )],
                ));
            }
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        // Validate SKILL.md front-matter identity exactly like
        // `skill_path_visibility`: a present SKILL.md whose declared `name`
        // differs (or is unparseable / unreadable) is NOT the capability being
        // gated and must read Degraded, never Visible — so a mismatched or
        // replaced `.system/<name>` (or host-dir `<name>`) body cannot pass the
        // runtime skill-tag gate as the adopted capability.
        match std::fs::read_to_string(&skill_md) {
            Ok(text) => match crate::parse_front_matter(&text).0.as_deref().map(str::trim) {
                Some(found) if found == name => {
                    return mk(
                        HostVisibilityStatus::Visible,
                        vec![format!(
                            "{loc} present; SKILL.md front-matter name matches: {}",
                            skill_md.display()
                        )],
                    );
                }
                Some(found) => {
                    if degraded.is_none() {
                        degraded = Some(mk(
                            HostVisibilityStatus::Degraded,
                            vec![format!(
                                "SKILL.md name mismatch: declares '{found}' but expected '{name}' at {}",
                                skill_md.display()
                            )],
                        ));
                    }
                }
                None => {
                    if degraded.is_none() {
                        degraded = Some(mk(
                            HostVisibilityStatus::Degraded,
                            vec![format!(
                                "SKILL.md present but front-matter not parseable: {}",
                                skill_md.display()
                            )],
                        ));
                    }
                }
            },
            Err(e) => {
                if degraded.is_none() {
                    degraded = Some(mk(
                        HostVisibilityStatus::Degraded,
                        vec![format!("SKILL.md unreadable: {} ({e})", skill_md.display())],
                    ));
                }
            }
        }
    }
    degraded.unwrap_or_else(|| {
        mk(
            HostVisibilityStatus::NotVisible,
            vec![format!("not found under {}", base.display())],
        )
    })
}

/// One discovered host-dir skill candidate before per-host visibility is filled.
struct HostDirCandidate {
    source: PathBuf,
    managed_status: ManagedStatus,
    canonical_present: bool,
    risk_notes: Vec<String>,
}

/// Classify one host skills-dir entry that is NOT already a known suite/repo
/// skill. Returns `None` when the entry should be ignored (housekeeping names).
/// READ-ONLY: never writes, never copies, never relinks. System (`.system`)
/// skills, sibling-suite bodies, other-project bodies, real user dirs, and
/// arbitrary external symlink targets are each classified distinctly, and all
/// land fail-closed `routing: None` (not-routable) until explicitly adopted.
fn classify_host_dir_entry(
    repo_root: &Path,
    entry: &Path,
    name: &str,
    is_system: bool,
) -> Option<HostDirCandidate> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(".bak")
        || name.starts_with(".ags-")
    {
        return None;
    }
    let link_meta = std::fs::symlink_metadata(entry).ok()?;
    let is_symlink = link_meta.file_type().is_symlink();
    // Dangling symlink → recognized but broken / unmanaged.
    if is_symlink && !entry.exists() {
        return Some(HostDirCandidate {
            source: entry.to_path_buf(),
            managed_status: ManagedStatus::Unmanaged,
            canonical_present: false,
            risk_notes: vec![format!(
                "Dangling host thin index (symlink target missing): {}. Recognized read-only; not routable.",
                entry.display()
            )],
        });
    }
    let real = std::fs::canonicalize(entry).ok()?;
    let has_skill_md = real.join("SKILL.md").is_file();

    // System skills (host built-ins under `.system`) — read-only recognition.
    if is_system {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::HostSystem,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "Host system skill — recognized read-only. AGS never holds/copies/relinks the body. Adopt via the registry to make it routable.".to_string(),
            ],
        });
    }
    // A thin index into THIS repo's AGS store is the same body already covered
    // by the suite/repo passes — skip to avoid a duplicate row.
    if canonical_within_store(repo_root, entry) {
        return None;
    }
    // A body inside a sibling AGS suite mirror (private<->stable) — recognized.
    if split_suite_runtime_path(&real).is_some() {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::Discovered,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "Discovered from a sibling AGS suite mirror. Opt-in candidate; not routable until registered.".to_string(),
            ],
        });
    }
    // A body inside another git project (not the AGS suite) — project-local.
    if let Some(proj) = find_git_root(&real) {
        if proj != *repo_root {
            return Some(HostDirCandidate {
                source: real,
                managed_status: ManagedStatus::ProjectLocal,
                canonical_present: has_skill_md,
                risk_notes: vec![format!(
                    "Project-local skill (body under {}). Read-only recognition; not routable until registered.",
                    proj.display()
                )],
            });
        }
    }
    // A real directory the user dropped directly into the host skills dir.
    if !is_symlink && real.is_dir() {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::Discovered,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "User-installed local skill (real dir in host skills dir). Opt-in candidate; not routable until registered.".to_string(),
            ],
        });
    }
    // Anything else: a symlink to an arbitrary external location AGS does not
    // govern (e.g. an app bundle, a non-suite tool root) — unmanaged.
    Some(HostDirCandidate {
        source: real,
        managed_status: ManagedStatus::Unmanaged,
        canonical_present: has_skill_md,
        risk_notes: vec![format!(
            "External user-installed skill outside AGS governance ({}). Recognized read-only; not routable.",
            entry.display()
        )],
    })
}

/// Full-machine discovery: scan every supported host's skills dir (and its
/// `.system` area) for skills not already known from the suite/repo passes, and
/// model each as a `ManagedCapability` with per-host thin-index visibility.
/// READ-ONLY. Bodies are never copied; classification is fail-closed
/// (`routing: None` ⇒ not-routable) — adoption into the registry is the only
/// way one of these becomes routable.
fn discover_host_dir_capabilities(
    ctx: &ConsoleContext,
    hosts: &[String],
    known: &mut Vec<String>,
) -> Vec<ManagedCapability> {
    let mut by_name: std::collections::BTreeMap<String, HostDirCandidate> =
        std::collections::BTreeMap::new();
    for host in hosts {
        let Some(subdir) = host_skills_subdir(host) else {
            continue;
        };
        let base = ctx.home.join(subdir);
        // Normal entries.
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if known.iter().any(|n| n == &name) || by_name.contains_key(&name) {
                    continue;
                }
                if let Some(c) = classify_host_dir_entry(&ctx.repo_root, &e.path(), &name, false) {
                    by_name.insert(name, c);
                }
            }
        }
        // Host system area.
        let sys = base.join(".system");
        if let Ok(rd) = std::fs::read_dir(&sys) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if known.iter().any(|n| n == &name) || by_name.contains_key(&name) {
                    continue;
                }
                if let Some(c) = classify_host_dir_entry(&ctx.repo_root, &e.path(), &name, true) {
                    by_name.insert(name, c);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (name, cand) in by_name {
        known.push(name.clone());
        let host_visibility: Vec<HostVisibility> = hosts
            .iter()
            .map(|h| host_dir_entry_visibility(&ctx.home, h, &name))
            .collect();
        out.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name,
            source: Some(cand.source.to_string_lossy().to_string()),
            profile: None,
            managed_status: cand.managed_status,
            registry_status: RegistryStatus::NotRegistered,
            canonical_present: cand.canonical_present,
            expected_hosts: Vec::new(),
            host_visibility,
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: cand.risk_notes,
            routing: None,
        });
    }
    out
}

pub fn build_inventory(ctx: &ConsoleContext, hosts: &[&str]) -> ManagedInventoryResult {
    let hosts: Vec<String> = if hosts.is_empty() {
        SUPPORTED_HOSTS.iter().map(|s| s.to_string()).collect()
    } else {
        hosts.iter().map(|s| s.to_string()).collect()
    };

    // One MCP probe per requested supported host (reserved hosts get none).
    let probes: Vec<(String, HostMcpProbe)> = hosts
        .iter()
        .filter(|h| host_skills_subdir(h).is_some())
        .map(|h| (h.clone(), probe_host_mcp(ctx, h)))
        .collect();
    let mut caps: Vec<ManagedCapability> = Vec::new();

    // Routing metadata (Capability Route) — manifest is the single authority.
    // Read up-front so expected-host gating can exclude internal-entrypoint
    // route targets (routing.parent set) before they are ever flagged.
    let routing_meta = read_routing_metadata(&ctx.repo_root);

    // 1. Suite-managed skills (from the suite manifest + ignore/adoption).
    let scan = crate::scan_skills(&ctx.repo_root);
    let mut known_skill_names: Vec<String> = Vec::new();
    for s in &scan.skills {
        known_skill_names.push(s.name.clone());
        let managed_status = match s.profile.as_str() {
            "required" | "optional" | "personal" => ManagedStatus::SuiteManaged,
            "ignored" | "rejected" => ManagedStatus::Ignored,
            _ => ManagedStatus::Discovered,
        };
        let registry_status = match managed_status {
            ManagedStatus::SuiteManaged => RegistryStatus::Registered,
            _ => RegistryStatus::NotRegistered,
        };
        let mut risk_notes: Vec<String> = s.warnings.clone();
        if let Some(fam) = cli_family_for_skill(&s.name) {
            risk_notes.push(format!(
                "Fronted by external CLI `{}` ({}). AGS distributes the skill entry but does not run `{} update`.",
                fam.cli, fam.endpoint, fam.cli
            ));
        }
        // Required skills are what the suite installs → expected visible in the
        // host. Optional/personal are opt-in, so not flagged as a verify gap.
        // An internal-entrypoint route target (routing.parent set) is NEVER a
        // standalone host body, so it never produces an expected-host gap.
        let s_is_route_target = routing_meta
            .map
            .get(&s.name)
            .is_some_and(|r| r.parent.is_some());
        let expected_hosts = if s.profile == "required" && !s_is_route_target {
            supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        };
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: s.name.clone(),
            source: s.source.clone(),
            profile: Some(s.profile.clone()),
            managed_status,
            registry_status,
            canonical_present: canonical_skill_present(&ctx.repo_root, s.source.as_deref()),
            expected_hosts,
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 1.5. Registry-governed external skill bodies. The external manager owns
    // the body under the shared multi-agent store; AGS owns only metadata and
    // per-host thin indexes.
    for body in routing_meta.external_skill_bodies.values() {
        if known_skill_names.iter().any(|name| name == &body.name) {
            continue;
        }
        known_skill_names.push(body.name.clone());
        let source = ctx.home.join(".agents/skills").join(&body.name);
        let canonical_present = canonical_within_shared_store(&ctx.home, &body.name, &source)
            && source.join("SKILL.md").is_file();
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: body.name.clone(),
            source: Some(source.to_string_lossy().to_string()),
            profile: body.profile.clone(),
            managed_status: ManagedStatus::Governed,
            registry_status: RegistryStatus::Registered,
            canonical_present,
            expected_hosts: supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "External skill body managed by `{}`; AGS owns only governance metadata and host thin indexes.",
                body.manager
            )],
            routing: None,
        });
    }

    // Required routable parent skills declared only in the registry must still
    // materialize in the expected universe. A fresh machine with no host body
    // needs a real NotVisible row so strict verify cannot silently shrink its
    // denominator and report a false-green result.
    for required in &routing_meta.required_skill_parents {
        if known_skill_names.iter().any(|name| name == &required.name) {
            continue;
        }
        known_skill_names.push(required.name.clone());
        let host_system = required.source_type.as_deref() == Some("host-system");
        let source = required
            .local_path
            .as_deref()
            .map(|path| ctx.repo_root.join(path))
            .or_else(|| {
                hosts.iter().find_map(|host| {
                    host_skill_body_dirs(ctx, host, &required.name)
                        .into_iter()
                        .next()
                })
            })
            .or_else(|| {
                (!host_system).then(|| ctx.home.join(".agents/skills").join(&required.name))
            });
        let canonical_present = source
            .as_ref()
            .is_some_and(|body| body.join("SKILL.md").is_file());
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: required.name.clone(),
            source: source.map(|path| path.to_string_lossy().to_string()),
            profile: Some(required.profile.clone()),
            managed_status: if host_system {
                ManagedStatus::HostSystem
            } else {
                ManagedStatus::Governed
            },
            registry_status: RegistryStatus::Registered,
            canonical_present,
            expected_hosts: supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "Required registry parent (source.type={}); absence remains an expected-host failure.",
                required.source_type.as_deref().unwrap_or("unspecified")
            )],
            routing: None,
        });
    }

    // 2. Local skill directories not in the manifest → Discovered (opt-in).
    let inv = crate::scan_skill_inventory(&ctx.repo_root);
    for e in &inv.entries {
        if known_skill_names.iter().any(|n| n == &e.name) {
            continue;
        }
        known_skill_names.push(e.name.clone());
        let mut risk_notes = Vec::new();
        if !e.risk_hints.is_empty() {
            risk_notes.push(format!("SKILL.md risk hints: {}", e.risk_hints.join(", ")));
        }
        if let Some(fam) = cli_family_for_skill(&e.name) {
            risk_notes.push(format!(
                "Fronted by external CLI `{}` ({}).",
                fam.cli, fam.endpoint
            ));
        }
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: e.name.clone(),
            source: Some(e.path.clone()),
            profile: None,
            managed_status: ManagedStatus::Discovered,
            registry_status: RegistryStatus::NotRegistered,
            canonical_present: e.has_skill_md,
            // Discovered skills are opt-in candidates → not a verify gap.
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 2.5. Full-machine discovery: host skills dirs (incl. `.system`) — system,
    //      external, sibling-suite, project-local, and user-installed skills not
    //      already known. READ-ONLY, fail-closed not-routable until adopted.
    for cap in discover_host_dir_capabilities(ctx, &hosts, &mut known_skill_names) {
        caps.push(cap);
    }

    // 3. Governed MCPs + AGS suite interface + CLI-backed MCPs from the registry.
    for e in read_mcp_registry(&ctx.repo_root) {
        let is_cli = e.manager.as_deref() == Some("external-cli");
        let (kind, managed_status, mut risk_notes) = if e.suite_interface {
            (
                ManagedKind::SuiteInterface,
                ManagedStatus::SuiteInterface,
                vec![
                    "AGS host initialization adapter — governance authority, not a governed third-party MCP.".to_string(),
                ],
            )
        } else if is_cli {
            (
                ManagedKind::CliBacked,
                ManagedStatus::Governed,
                vec!["Governed CLI-backed MCP.".to_string()],
            )
        } else {
            (
                ManagedKind::Mcp,
                ManagedStatus::Governed,
                vec!["Governed third-party MCP.".to_string()],
            )
        };
        if !e.suite_interface {
            risk_notes.push(
                "AGS advises host MCP registration commands; it never runs `claude mcp add/remove` itself.".to_string(),
            );
        }
        // Expected visible where the registry declares the server installed for
        // a supported host. Flags "registry says installed but host can't see it"
        // drift; an MCP the registry says is NOT installed here is not a gap.
        // An internal-entrypoint route target (routing.parent set) is never a
        // standalone host body → no expected-host gap.
        let e_is_route_target = routing_meta
            .map
            .get(&e.name)
            .is_some_and(|r| r.parent.is_some());
        let expected_hosts: Vec<String> = if e_is_route_target {
            Vec::new()
        } else {
            e.installed_clients
                .iter()
                .filter(|c| SUPPORTED_HOSTS.contains(&c.as_str()))
                .cloned()
                .collect()
        };
        caps.push(ManagedCapability {
            kind,
            name: e.name.clone(),
            source: Some("manifests/mcp-registry.yaml".to_string()),
            profile: None,
            managed_status,
            registry_status: RegistryStatus::Registered,
            // The MCP definition in the registry IS the canonical body.
            canonical_present: true,
            expected_hosts,
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 4. Synthetic CLI-backed binaries for any present family (e.g. lark-cli).
    let mut family_clis: Vec<&'static CliFamily> = Vec::new();
    for fam in CLI_FAMILIES {
        let present = caps.iter().any(|c| {
            matches!(c.kind, ManagedKind::Skill)
                && cli_family_for_skill(&c.name).map(|f| f.cli) == Some(fam.cli)
        });
        let already = caps.iter().any(|c| c.name == fam.cli);
        if present && !already {
            family_clis.push(fam);
        }
    }
    for fam in family_clis {
        caps.push(ManagedCapability {
            kind: ManagedKind::CliBacked,
            name: fam.cli.to_string(),
            source: Some(format!("external CLI binary `{}`", fam.cli)),
            profile: None,
            managed_status: ManagedStatus::Unmanaged,
            registry_status: RegistryStatus::NotRegistered,
            // The CLI binary is external — AGS does not hold its canonical body.
            canonical_present: false,
            // A CLI binary is not a host entry → never a host-visibility gap.
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "External official CLI talking to {}. Referenced, not adopted; AGS never runs `{} update`. Live endpoint health is a degraded observation only.",
                fam.endpoint, fam.cli
            )],
            routing: None,
        });
    }

    // 5. Fill host visibility, health, actions, and routing for every capability.
    for cap in &mut caps {
        let cli_backed_external = matches!(cap.kind, ManagedKind::CliBacked)
            && matches!(cap.managed_status, ManagedStatus::Unmanaged);
        // Host-dir-discovered capabilities pre-fill their own visibility (they
        // may live under `.system`), so only fill when not already populated.
        if cap.host_visibility.is_empty() {
            for host in &hosts {
                let probe = probes.iter().find(|(h, _)| h == host).map(|(_, p)| p);
                let canonical_source = if matches!(cap.kind, ManagedKind::Skill)
                    && !matches!(cap.managed_status, ManagedStatus::HostSystem)
                {
                    cap.source
                        .as_deref()
                        .map(|source| resolve_source(&ctx.repo_root, source))
                } else {
                    None
                };
                let external_shared = is_external_shared_skill(ctx, cap);
                cap.host_visibility.push(host_visibility(
                    ctx,
                    host,
                    &cap.kind,
                    &cap.name,
                    canonical_source.as_deref(),
                    external_shared,
                    probe,
                ));
            }
        }
        cap.health_status = derive_health(
            &cap.kind,
            &cap.name,
            &cap.host_visibility,
            &probes,
            cli_backed_external,
        );
        cap.actions = actions_for(&cap.kind, &cap.managed_status);
        // Stable routing facts (or None when the manifest declares none).
        //
        // A host-dir discovered capability can be explicitly adopted by adding a
        // skills-registry member with routing metadata (for example a
        // host-system `skill-creator` entry). In that case the manifest is the
        // registry authority even though AGS still does not hold or relink the
        // external body, so inventory must not report the row as
        // `not-registered`.
        cap.routing = routing_meta.map.get(&cap.name).cloned();
        if cap.routing.is_some() {
            cap.registry_status = RegistryStatus::Registered;
            if matches!(cap.managed_status, ManagedStatus::HostSystem) {
                cap.risk_notes
                    .retain(|note| !note.contains("Adopt via the registry to make it routable"));
                cap.risk_notes.push(
                    "Host system skill is registry-adopted for routing; AGS still recognizes it read-only and never holds/copies/relinks the body.".to_string(),
                );
            }
        }
    }

    apply_route_target_exposure_shape(&mut caps, &routing_meta.route_targets);
    apply_playbook_entrypoint_integrity(ctx, &mut caps, &routing_meta.route_targets);

    // 6. Synthesize route-target rows for internal entrypoints (playbook / MCP
    //    tool / CLI subcommand) declared under `route_targets:`. Routing-only:
    //    kind inherited from the parent, NO expected_hosts, NO host probe, NO
    //    actions, never adopted/synced. capability-route derefs their
    //    availability + `primary` to the parent capability.
    for (name, routing) in &routing_meta.route_targets {
        if known_skill_names.iter().any(|n| n == name) {
            continue;
        }
        known_skill_names.push(name.clone());
        let kind = routing
            .parent
            .as_ref()
            .map(|p| p.kind.clone())
            .unwrap_or(ManagedKind::Skill);
        caps.push(ManagedCapability {
            kind,
            name: name.clone(),
            source: Some("manifests (route_targets)".to_string()),
            profile: None,
            managed_status: ManagedStatus::RouteTarget,
            registry_status: RegistryStatus::Registered,
            // Metadata-only: the canonical body is the parent capability.
            canonical_present: true,
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![
                "Internal entrypoint route target of a parent capability; routing-only, never a host body, never adopted/synced.".to_string(),
            ],
            routing: Some(routing.clone()),
        });
    }

    caps.sort_by(|a, b| a.name.cmp(&b.name));

    let summary = summarize(&caps);
    ManagedInventoryResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        hosts,
        capabilities: caps,
        summary,
        note: "Read-only inventory. Third-party capabilities are opt-in; AGS never silently bundles or installs. Use `ags skill propose --action <verb> --skill <name>` for a dry-run, then `--apply` to confirm.".to_string(),
        routing_parse_failures: routing_meta.parse_failures,
    }
}

fn summarize(caps: &[ManagedCapability]) -> ManagedInventorySummary {
    let claude_visible = caps
        .iter()
        .filter(|c| {
            c.host_visibility
                .iter()
                .any(|v| v.host == "claude-code" && v.status == HostVisibilityStatus::Visible)
        })
        .count();
    ManagedInventorySummary {
        total: caps.len(),
        skills: caps.iter().filter(|c| c.kind == ManagedKind::Skill).count(),
        mcps: caps.iter().filter(|c| c.kind == ManagedKind::Mcp).count(),
        suite_interfaces: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::SuiteInterface)
            .count(),
        cli_backed: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::CliBacked)
            .count(),
        canonical_present: caps.iter().filter(|c| c.canonical_present).count(),
        claude_visible,
        risk_flagged: caps.iter().filter(|c| !c.risk_notes.is_empty()).count(),
        routing_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Routable)
            })
            .count(),
        routing_not_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::NotRoutable)
            })
            .count(),
        routing_retired: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Retired)
            })
            .count(),
        routing_uncovered: caps
            .iter()
            .filter(|c| {
                matches!(
                    c.managed_status,
                    ManagedStatus::SuiteManaged | ManagedStatus::Governed
                ) && c.routing.is_none()
            })
            .count(),
    }
}

/// Deterministic content hash of the machine-local capability snapshot. Hashes a
/// CANONICAL projection (sorted `name|kind|managed_status|registry|route_state|
/// canonical|host=visibility…|health` lines) with FNV-1a — dependency-free and
/// stable across runs for identical machine state. Used as the task-card snapshot
/// attestation token. Contains capability NAMES + statuses only — NO absolute
/// paths — so it is safe to record in a (machine-local) snapshot or a task card.
pub fn inventory_snapshot_hash(inv: &ManagedInventoryResult) -> String {
    fn vis_str(s: &HostVisibilityStatus) -> &'static str {
        match s {
            HostVisibilityStatus::Visible => "visible",
            HostVisibilityStatus::NotVisible => "not-visible",
            HostVisibilityStatus::Degraded => "degraded",
            HostVisibilityStatus::Unsupported => "unsupported",
            HostVisibilityStatus::Deferred => "deferred",
        }
    }
    fn health_str(h: &HealthStatus) -> &'static str {
        match h {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unknown => "unknown",
            HealthStatus::Unhealthy => "unhealthy",
        }
    }
    fn kind_str(k: &ManagedKind) -> &'static str {
        match k {
            ManagedKind::Skill => "skill",
            ManagedKind::Mcp => "mcp",
            ManagedKind::SuiteInterface => "suite-interface",
            ManagedKind::CliBacked => "cli-backed",
        }
    }
    let route_str = |c: &ManagedCapability| -> &'static str {
        match c.routing.as_ref().map(|r| r.route_state) {
            Some(RouteState::Routable) => "routable",
            Some(RouteState::NotRoutable) => "not-routable",
            Some(RouteState::Retired) => "retired",
            None => "none",
        }
    };
    let mut lines: Vec<String> = inv
        .capabilities
        .iter()
        .map(|c| {
            let mut vis: Vec<String> = c
                .host_visibility
                .iter()
                .map(|v| format!("{}={}", v.host, vis_str(&v.status)))
                .collect();
            vis.sort();
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                c.name,
                kind_str(&c.kind),
                managed_status_str(&c.managed_status),
                if matches!(c.registry_status, RegistryStatus::Registered) {
                    "registered"
                } else {
                    "not-registered"
                },
                route_str(c),
                c.canonical_present,
                vis.join(","),
                health_str(&c.health_status),
            )
        })
        .collect();
    lines.sort();
    let joined = lines.join("\n");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// Whether AGS holds the canonical skill body: the resolved source dir contains
/// a `SKILL.md`. Read-only.
fn canonical_skill_present(repo_root: &Path, source: Option<&str>) -> bool {
    source
        .map(|s| resolve_source(repo_root, s).join("SKILL.md").is_file())
        .unwrap_or(false)
}

// ── Host verify ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCheck {
    pub name: String,
    pub kind: String,
    pub visibility: HostVisibilityStatus,
    /// Whether this capability is expected to be visible on this host (drives
    /// the failure signal). Opt-in / not-applicable capabilities are false.
    pub expected: bool,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVerifySummary {
    pub total: usize,
    pub visible: usize,
    pub not_visible: usize,
    pub degraded: usize,
    /// Capabilities expected to be visible on this host.
    pub expected: usize,
    /// Expected capabilities that are NOT visible (the failure count).
    pub failed: usize,
    /// True when every expected capability is visible.
    pub all_visible: bool,
}

/// Read-only host thin-index DRIFT report: legacy `.bak*` entries, dangling
/// symlinks, and real-directory copies in a host's skills dir. AGS only REPORTS
/// this — cleanup is a separate explicit hygiene action, NEVER performed by
/// this read-only scan. A clean host has exactly one thin-index symlink per
/// capability pointing at the canonical store / `~/.agents/skills`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinIndexDrift {
    pub host: String,
    pub skills_dir: String,
    pub total_entries: usize,
    /// Single thin-index symlinks pointing at a valid target (clean).
    pub clean_symlinks: usize,
    /// `.bak` / `.bak.N` leftover entries from older host-entry relinks.
    pub bak_leftovers: usize,
    /// Dangling symlinks (target missing) — e.g. retired-skill fallout.
    pub broken_symlinks: usize,
    /// Real-directory copies (non-symlink, not `.system`) — informational: may be
    /// a legitimate local/external skill, not necessarily drift.
    pub real_dir_copies: usize,
    /// True when removable drift (`.bak*` leftovers or dangling symlinks) exists.
    pub has_drift: bool,
    /// Capped sample of drift entry names for operator triage.
    pub drift_samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVerifyResult {
    pub schema_version: String,
    pub host: String,
    pub supported: bool,
    /// "ok" | "degraded" | "incomplete" | "unsupported"
    pub status: String,
    pub checks: Vec<HostCheck>,
    pub summary: HostVerifySummary,
    /// Read-only thin-index drift report (None for unsupported hosts / absent
    /// skills dir). Reported, never auto-cleaned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thin_index_drift: Option<ThinIndexDrift>,
    pub note: String,
}

/// Read-only scan of a host's thin-index dir for drift. NEVER mutates. Returns
/// `None` for hosts without a skills subdir or when the dir is absent. Counts
/// `.bak*` leftovers and dangling symlinks as removable drift; real non-`.bak`
/// directories are reported as informational (could be legitimate local skills).
fn scan_thin_index_drift(home: &Path, host: &str) -> Option<ThinIndexDrift> {
    let sub = host_skills_subdir(host)?;
    let dir = home.join(sub);
    let read = std::fs::read_dir(&dir).ok()?;
    let mut total = 0usize;
    let (mut clean, mut bak, mut broken, mut realdir) = (0usize, 0usize, 0usize, 0usize);
    let mut samples: Vec<String> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".system" || name.starts_with(".ags-drift-quarantine") {
            continue;
        }
        total += 1;
        let path = entry.path();
        let is_link = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let target_exists = path.exists(); // follows the symlink
        if name.contains(".bak") {
            bak += 1;
            if samples.len() < 12 {
                samples.push(format!("{name} (.bak leftover)"));
            }
        } else if is_link && !target_exists {
            broken += 1;
            if samples.len() < 12 {
                samples.push(format!("{name} (dangling symlink)"));
            }
        } else if is_link {
            clean += 1;
        } else if path.is_dir() {
            realdir += 1;
        }
    }
    Some(ThinIndexDrift {
        host: host.to_string(),
        skills_dir: dir.to_string_lossy().to_string(),
        total_entries: total,
        clean_symlinks: clean,
        bak_leftovers: bak,
        broken_symlinks: broken,
        real_dir_copies: realdir,
        has_drift: bak > 0 || broken > 0,
        drift_samples: samples,
    })
}

/// Verify host visibility for one host. Read-only. For reserved hosts (cursor)
/// returns `supported: false`, `status: "unsupported"` with stable fields and
/// an empty check list.
pub fn verify_host(ctx: &ConsoleContext, host: &str) -> HostVerifyResult {
    let supported = SUPPORTED_HOSTS.contains(&host);
    if !supported {
        let deferred = DEFERRED_HOSTS.contains(&host);
        return HostVerifyResult {
            schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
            host: host.to_string(),
            supported: false,
            status: "unsupported".to_string(),
            checks: Vec::new(),
            summary: HostVerifySummary {
                total: 0,
                visible: 0,
                not_visible: 0,
                degraded: 0,
                expected: 0,
                failed: 0,
                all_visible: true,
            },
            thin_index_drift: None,
            note: if deferred {
                format!("Host '{host}' visibility check is reserved for a later phase. Model fields are stable; no probing performed.")
            } else {
                format!("Host '{host}' is not a recognized AGS host.")
            },
        };
    }

    let inventory = build_inventory(ctx, &[host]);
    let mut checks = Vec::new();
    for cap in &inventory.capabilities {
        if let Some(vis) = cap.host_visibility.iter().find(|v| v.host == host) {
            checks.push(HostCheck {
                name: cap.name.clone(),
                kind: kind_str(&cap.kind).to_string(),
                visibility: vis.status.clone(),
                expected: cap.expected_hosts.iter().any(|h| h == host),
                evidence: vis.evidence.clone(),
            });
        }
    }

    let visible = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::Visible)
        .count();
    let degraded = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::Degraded)
        .count();
    let not_visible = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::NotVisible)
        .count();
    let expected = checks.iter().filter(|c| c.expected).count();
    // `failed` = an expected capability the host definitively cannot see
    // (NotVisible) → status incomplete. An expected capability we merely
    // couldn't confirm (Degraded) does not count as failed but does prevent an
    // "ok" verdict (→ degraded).
    let failed = checks
        .iter()
        .filter(|c| c.expected && c.visibility == HostVisibilityStatus::NotVisible)
        .count();
    let expected_degraded = checks
        .iter()
        .filter(|c| c.expected && c.visibility == HostVisibilityStatus::Degraded)
        .count();
    let all_visible = failed == 0 && expected_degraded == 0;
    let status = if failed > 0 {
        "incomplete"
    } else if expected_degraded > 0 {
        "degraded"
    } else {
        "ok"
    }
    .to_string();

    HostVerifyResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        host: host.to_string(),
        supported: true,
        status,
        summary: HostVerifySummary {
            total: checks.len(),
            visible,
            not_visible,
            degraded,
            expected,
            failed,
            all_visible,
        },
        checks,
        thin_index_drift: scan_thin_index_drift(&ctx.home, host),
        note: "Read-only host-visibility verify. status=incomplete means an expected capability is not visible. Restart the host after adopt/update so it re-scans entry points; use --strict to gate (exit nonzero unless status=ok). thin_index_drift reports legacy `.bak`/dangling-symlink drift only; capability sync no longer creates `.bak` backups.".to_string(),
    }
}

fn kind_str(k: &ManagedKind) -> &'static str {
    match k {
        ManagedKind::Skill => "skill",
        ManagedKind::Mcp => "mcp",
        ManagedKind::SuiteInterface => "suite-interface",
        ManagedKind::CliBacked => "cli-backed",
    }
}

// ── Proposal / guarded apply ─────────────────────────────────────────────────

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
struct ActionPlan {
    writes: Vec<PlannedWrite>,
    advised: Vec<AdvisedCommand>,
    blocked: Vec<String>,
    notes: Vec<String>,
}

/// Propose an action on a named capability. `apply == false` → dry-run (no
/// writes). `apply == true` → guarded apply: only AGS-owned file writes within
/// `ctx` are performed; external installer/registrar commands are only advised,
/// never executed.
pub fn propose_action(
    ctx: &ConsoleContext,
    action: ConsoleAction,
    name: &str,
    apply: bool,
) -> ConsoleProposalResult {
    let inventory = build_inventory(ctx, &supported_skill_hosts());
    propose_action_inner(ctx, &inventory, action, name, apply)
}

/// Plan/apply against a pre-built inventory. Lets batch callers (e.g.
/// [`sync_plan`]) reuse a single inventory instead of rebuilding it — and
/// re-invoking host CLIs — once per capability.
fn propose_action_inner(
    ctx: &ConsoleContext,
    inventory: &ManagedInventoryResult,
    action: ConsoleAction,
    name: &str,
    apply: bool,
) -> ConsoleProposalResult {
    let cap = inventory.capabilities.iter().find(|c| c.name == name);

    let mut result = ConsoleProposalResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        action: action.as_str().to_string(),
        capability: name.to_string(),
        apply_requested: apply,
        ..Default::default()
    };

    let Some(cap) = cap else {
        result.found = false;
        result.blocked_reasons.push(format!(
            "Capability '{name}' not found in the managed inventory. Run `ags skill` to list, or place its source under the suite before adopting."
        ));
        result.note = dry_run_note();
        return result;
    };

    result.found = true;
    result.kind = Some(kind_str(&cap.kind).to_string());
    result.managed_status = Some(managed_status_str(&cap.managed_status).to_string());
    result.risk_notes = cap.risk_notes.clone();

    let plan = plan_action(ctx, cap, action);
    result.planned_writes = plan.writes.clone();
    result.advised_commands = plan.advised;
    result.blocked_reasons = plan.blocked;

    // The single mutation guard. No confirmation, or any blocked reason → no writes.
    let confirmed = apply && result.blocked_reasons.is_empty();
    let outcome = guarded_apply(confirmed, &plan.writes, ctx);
    result.applied_writes = outcome.applied_writes;
    result.apply_errors = outcome.errors;
    // `applied` is true only when a write was confirmed, at least one AGS-owned
    // write was planned, AND every one succeeded — NEVER from confirmation
    // alone. Advised-only actions (MCP/CLI) plan no writes ⇒ applied stays false.
    result.applied = confirmed
        && !matches!(action, ConsoleAction::Verify)
        && !result.planned_writes.is_empty()
        && result.apply_errors.is_empty();

    // Distinct apply state so callers never mistake "AGS only advised you to run
    // a command" for "AGS performed the action".
    result.apply_status = if !apply {
        "dry-run"
    } else if !result.blocked_reasons.is_empty() {
        "blocked"
    } else if !result.apply_errors.is_empty() {
        "failed"
    } else if result.applied {
        "applied"
    } else if !result.advised_commands.is_empty() {
        // Confirmed, but the only "action" AGS can offer is advice it never runs.
        "advised-only"
    } else {
        "nothing-to-do"
    }
    .to_string();

    let mut note_lines = plan.notes;
    match result.apply_status.as_str() {
        "dry-run" => note_lines.push(dry_run_note()),
        "blocked" => note_lines.push(
            "Apply was requested but is blocked — see blocked_reasons. Nothing written."
                .to_string(),
        ),
        "failed" => note_lines.push(
            "Apply FAILED — one or more writes errored (see apply_errors); no host was left half-changed (per-host transactional, multi-host preflighted). Resolve, re-run, then `ags skill verify`.".to_string(),
        ),
        "advised-only" => note_lines.push(
            "AGS performed NOTHING — this capability has no AGS-owned host file. Run the advised command(s) yourself, then restart the host. `applied` is false by design.".to_string(),
        ),
        "applied" => note_lines.push("Applied. Restart the host (Claude Code / Codex / CodeBuddy-Code / Cursor) so it re-scans thin indexes, then run `ags skill verify --host <host>`.".to_string()),
        _ => {}
    }
    result.note = note_lines.join(" ");
    result
}

fn dry_run_note() -> String {
    "DRY-RUN — no files written, no external command run. Re-run with `--apply` to confirm. Apply never runs external installers (npx skills add, lark-cli update, claude mcp add/remove).".to_string()
}

fn managed_status_str(s: &ManagedStatus) -> &'static str {
    match s {
        ManagedStatus::SuiteManaged => "suite-managed",
        ManagedStatus::Governed => "governed",
        ManagedStatus::SuiteInterface => "suite-interface",
        ManagedStatus::Discovered => "discovered",
        ManagedStatus::HostSystem => "host-system",
        ManagedStatus::ProjectLocal => "project-local",
        ManagedStatus::Ignored => "ignored",
        ManagedStatus::Unmanaged => "unmanaged",
        ManagedStatus::RouteTarget => "route-target",
    }
}

fn plan_action(ctx: &ConsoleContext, cap: &ManagedCapability, action: ConsoleAction) -> ActionPlan {
    let mut plan = ActionPlan::default();

    // The AGS host initialization adapter is never mutated through the console.
    if matches!(cap.kind, ManagedKind::SuiteInterface) && !matches!(action, ConsoleAction::Verify) {
        plan.blocked.push(
            "AGS host initialization adapter cannot be adopted/updated/removed via the skill console; it is the governance authority, not a governed object.".to_string(),
        );
        return plan;
    }

    if matches!(action, ConsoleAction::Verify) {
        plan.notes.push(format!(
            "Verify is read-only. Run `ags skill verify --host claude-code` for host-visibility evidence for '{}'.",
            cap.name
        ));
        return plan;
    }

    // Retired capabilities (route_state: retired) keep a registry row for
    // history/dedupe and may still have a canonical body on disk, but they must
    // NEVER be (re)adopted, updated, or repaired into a host — that would
    // resurrect a deliberately retired front-stage entry. `remove`/`uninstall`
    // (cleanup) and `verify` stay available.
    if matches!(
        action,
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair
    ) && cap
        .routing
        .as_ref()
        .is_some_and(|r| r.route_state == RouteState::Retired)
    {
        plan.blocked.push(format!(
            "'{}' is retired (route_state: retired) and cannot be adopted/updated/repaired into a host — it is kept only as a history/compat record. Any underlying CLI/successor remains; `remove`/`uninstall` and `verify` stay available.",
            cap.name
        ));
        return plan;
    }

    match cap.kind {
        ManagedKind::Skill => plan_skill_entry(ctx, cap, action, &mut plan),
        ManagedKind::Mcp | ManagedKind::CliBacked => plan_mcp_or_cli(cap, action, &mut plan),
        ManagedKind::SuiteInterface => {}
    }
    plan
}

/// The supported hosts that load skills from a `~/<subdir>/skills` directory.
fn supported_skill_hosts() -> Vec<&'static str> {
    SUPPORTED_HOSTS
        .iter()
        .copied()
        .filter(|h| host_skills_subdir(h).is_some())
        .collect()
}

/// Does the host thin index at `entry` need (re)creating? True if absent, a
/// dangling symlink, missing SKILL.md, or a front-matter name mismatch.
fn thin_index_needs_repair(entry: &Path, name: &str) -> bool {
    // Dangling symlink → broken.
    if std::fs::symlink_metadata(entry)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
        && std::fs::metadata(entry).is_err()
    {
        return true;
    }
    let skill_md = entry.join("SKILL.md");
    if !skill_md.is_file() {
        return true;
    }
    match std::fs::read_to_string(&skill_md) {
        Ok(text) => crate::parse_front_matter(&text).0.as_deref().map(str::trim) != Some(name),
        Err(_) => true,
    }
}

fn thin_index_matches_canonical(entry: &Path, canonical: &Path, name: &str) -> bool {
    if std::fs::symlink_metadata(entry)
        .map(|meta| !meta.file_type().is_symlink())
        .unwrap_or(true)
        || thin_index_needs_repair(entry, name)
    {
        return false;
    }
    match (
        std::fs::canonicalize(entry),
        std::fs::canonicalize(canonical),
    ) {
        (Ok(real_entry), Ok(real_canonical)) => {
            thin_index_target_match(&real_entry, &real_canonical).is_some()
        }
        _ => false,
    }
}

fn shared_skill_entry_is_loadable(ctx: &ConsoleContext, host: &str, name: &str) -> Option<PathBuf> {
    shared_skill_dirs_for_host(ctx, host)
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|entry| {
            std::fs::symlink_metadata(entry).is_ok() && !thin_index_needs_repair(entry, name)
        })
}

/// Plan per-host thin-index distribution. The declared owner keeps ONE
/// canonical skill body; each host that lacks shared discovery gets a symlink
/// at `<host>/skills/<name>`. `remove`/`uninstall` touch only the thin index;
/// the canonical body is never touched here. `verify` plans nothing.
fn plan_skill_entry(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
    action: ConsoleAction,
    plan: &mut ActionPlan,
) {
    // Hard boundary: the capability name becomes a path component under each
    // host's skills dir. Reject `/`, `\`, `..`, absolute, and multi-component
    // names BEFORE planning any write so a corrupt/hostile name can never make
    // a write target escape the skills directory.
    if !is_safe_path_component(&cap.name) {
        plan.blocked.push(format!(
            "Unsafe capability name '{}' — refusing to plan a thin-index write (path traversal / separator / absolute path not allowed).",
            cap.name
        ));
        return;
    }

    // Resolve and validate the canonical body for actions that link to it. We
    // refuse to create a dangling thin index.
    let canonical = cap
        .source
        .as_ref()
        .map(|s| resolve_source(&ctx.repo_root, s));
    let canonical = if matches!(
        action,
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair
    ) {
        let Some(dir) = canonical else {
            plan.blocked.push(format!(
                "No canonical source path known for '{}'; cannot create a thin index.",
                cap.name
            ));
            return;
        };
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            plan.blocked.push(format!(
                "Canonical SKILL.md not found at {} — refusing to create a dangling thin index.",
                skill_md.display()
            ));
            return;
        }
        // Containment follows declared ownership: suite bodies stay inside the
        // repository stores; external bodies stay inside the shared skill root.
        if !canonical_source_allowed(ctx, cap, &dir) {
            plan.blocked.push(format!(
                "Canonical source {} is outside the store approved for '{}' — refusing to link a host to it.",
                dir.display(),
                cap.name
            ));
            return;
        }
        // The canonical body must declare the capability we think we're linking.
        match std::fs::read_to_string(&skill_md)
            .ok()
            .and_then(|t| crate::parse_front_matter(&t).0)
            .as_deref()
            .map(str::trim)
        {
            Some(n) if n == cap.name => {}
            other => {
                plan.blocked.push(format!(
                    "Canonical SKILL.md at {} declares name {:?}, not '{}' — refusing to mislabel a host entry.",
                    skill_md.display(),
                    other,
                    cap.name
                ));
                return;
            }
        }
        Some(dir)
    } else {
        canonical
    };

    // Distribute / update / remove the thin index on EVERY supported skill host,
    // so one restart makes the skill discoverable on all platforms. Each host is
    // ONE op (`relink` / `unlink`); guarded_apply executes it transactionally
    // and preflights every host before mutating any.
    for host in supported_skill_hosts() {
        let subdir = host_skills_subdir(host).expect("supported host has a skills subdir");
        let entry = ctx.home.join(subdir).join(&cap.name);
        let entry_str = entry.display().to_string();
        let present = std::fs::symlink_metadata(&entry).is_ok();

        match action {
            ConsoleAction::Adopt | ConsoleAction::Update => {
                if let Some(shared_entry) = shared_skill_entry_is_loadable(ctx, host, &cap.name) {
                    plan.notes.push(format!(
                        "[{host}] shared skill source already visible at {}; skip {} to avoid duplicate picker entries.",
                        shared_entry.display(),
                        entry_str
                    ));
                    continue;
                }
                if thin_index_matches_canonical(&entry, canonical.as_ref().unwrap(), &cap.name) {
                    plan.notes.push(format!(
                        "[{host}] thin index already resolves to the canonical body at {entry_str}; nothing to change."
                    ));
                    continue;
                }
                plan.writes.push(PlannedWrite {
                    op: "relink".to_string(),
                    path: entry_str.clone(),
                    from: Some(canonical.as_ref().unwrap().display().to_string()),
                    detail: format!(
                        "[{host}] thin index → canonical skill dir (transactional; existing entry replaced without .bak clutter; references travel with it)"
                    ),
                });
            }
            ConsoleAction::Remove | ConsoleAction::Uninstall => {
                if present {
                    plan.writes.push(PlannedWrite {
                        op: "unlink".to_string(),
                        path: entry_str.clone(),
                        from: None,
                        detail: format!(
                            "[{host}] remove thin index (moved to .bak); canonical body untouched"
                        ),
                    });
                } else {
                    plan.notes.push(format!(
                        "[{host}] no thin index at {entry_str}; nothing to remove."
                    ));
                }
            }
            ConsoleAction::Repair => {
                if let Some(shared_entry) = shared_skill_entry_is_loadable(ctx, host, &cap.name) {
                    plan.notes.push(format!(
                        "[{host}] shared skill source already visible at {}; no host-specific repair needed.",
                        shared_entry.display()
                    ));
                    continue;
                }
                if !thin_index_matches_canonical(&entry, canonical.as_ref().unwrap(), &cap.name) {
                    plan.writes.push(PlannedWrite {
                        op: "relink".to_string(),
                        path: entry_str.clone(),
                        from: Some(canonical.as_ref().unwrap().display().to_string()),
                        detail: format!(
                            "[{host}] recreate broken/missing thin index (transactional)"
                        ),
                    });
                } else {
                    plan.notes.push(format!(
                        "[{host}] thin index present and loadable; nothing to repair."
                    ));
                }
            }
            ConsoleAction::Verify => {}
        }
    }

    // External-CLI advisories (AGS never runs these).
    match action {
        ConsoleAction::Adopt | ConsoleAction::Update => {
            if let Some(fam) = cli_family_for_skill(&cap.name) {
                plan.advised.push(AdvisedCommand {
                    command: format!("{} update", fam.cli),
                    reason: format!(
                        "'{}' is fronted by {}; refresh the CLI yourself — AGS never runs it.",
                        cap.name, fam.cli
                    ),
                });
            }
        }
        ConsoleAction::Uninstall => {
            plan.advised.push(AdvisedCommand {
                command: format!("npx skills remove {} -g", cap.name),
                reason: "Remove the underlying skill body from the AGS canonical store yourself — AGS never runs external installers.".to_string(),
            });
        }
        _ => {}
    }

    if !matches!(action, ConsoleAction::Verify) {
        plan.notes.push(
            "Restart the host(s) after adopt/update so they re-scan thin indexes.".to_string(),
        );
    }
}

/// Plan MCP / CLI-backed actions. AGS owns no file here, so it only *advises*
/// the external registrar/installer command and never executes it.
fn plan_mcp_or_cli(cap: &ManagedCapability, action: ConsoleAction, plan: &mut ActionPlan) {
    let name = &cap.name;
    match action {
        ConsoleAction::Adopt | ConsoleAction::Update | ConsoleAction::Repair => {
            if matches!(cap.kind, ManagedKind::CliBacked)
                && matches!(cap.managed_status, ManagedStatus::Unmanaged)
            {
                plan.advised.push(AdvisedCommand {
                    command: format!("{name} update"),
                    reason: "External official CLI — update it yourself. AGS never runs it."
                        .to_string(),
                });
            } else {
                // Cross-Agent host command plan: AGS advises the registration
                // command for each supported host (Claude Code, Codex). Cursor
                // MCP registration is reserved. AGS never runs any of these.
                plan.advised.push(AdvisedCommand {
                    command: format!("claude mcp add {name} -- <command> [args...]"),
                    reason: "Claude Code: AGS records MCP governance but never registers MCP servers in host config; run this in Claude Code, then restart it.".to_string(),
                });
                plan.advised.push(AdvisedCommand {
                    command: format!("codex mcp add {name} -- <command> [args...]"),
                    reason: "Codex: AGS records MCP governance but never registers MCP servers in host config; run this in Codex, then restart it.".to_string(),
                });
            }
        }
        ConsoleAction::Remove | ConsoleAction::Uninstall => {
            plan.advised.push(AdvisedCommand {
                command: format!("claude mcp remove {name}"),
                reason: "Claude Code: AGS never unregisters MCP servers from host config; run this yourself.".to_string(),
            });
            plan.advised.push(AdvisedCommand {
                command: format!("codex mcp remove {name}"),
                reason:
                    "Codex: AGS never unregisters MCP servers from host config; run this yourself."
                        .to_string(),
            });
        }
        ConsoleAction::Verify => {}
    }
    plan.notes.push(
        "MCP / CLI-backed capabilities have no AGS-owned host file; AGS advises the per-host command (Claude Code, Codex) but never runs it.".to_string(),
    );
}

/// Resolve a suite source path; absolute paths are used as-is.
fn resolve_source(repo_root: &Path, source: &str) -> PathBuf {
    let p = Path::new(source);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

/// The approved canonical skill stores under the repo. A symlink target must
/// live inside one of these — never an arbitrary local directory.
const CANONICAL_STORES: &[&str] = &["global-skills", "skill-packs"];

/// True iff `canonical_dir` (canonicalized) lives inside an approved store.
/// Defends against bad/stale manifest sources (absolute paths, `..` escapes,
/// targets outside the repo) being symlinked as host-loadable skill bodies.
fn canonical_within_store(repo_root: &Path, canonical_dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(canonical_dir) else {
        return false;
    };
    CANONICAL_STORES.iter().any(|store| {
        std::fs::canonicalize(repo_root.join(store))
            .map(|root| real.starts_with(&root))
            .unwrap_or(false)
    })
}

fn canonical_within_shared_store(home: &Path, name: &str, canonical_dir: &Path) -> bool {
    if !is_safe_path_component(name) {
        return false;
    }
    let shared_root = home.join(".agents/skills");
    let expected = shared_root.join(name);
    match (
        std::fs::canonicalize(canonical_dir),
        std::fs::canonicalize(expected),
        std::fs::canonicalize(shared_root),
    ) {
        (Ok(actual), Ok(expected), Ok(root)) => actual == expected && actual.starts_with(root),
        _ => false,
    }
}

fn is_external_shared_skill(ctx: &ConsoleContext, cap: &ManagedCapability) -> bool {
    let expected = ctx.home.join(".agents/skills").join(&cap.name);
    matches!(cap.kind, ManagedKind::Skill)
        && matches!(cap.managed_status, ManagedStatus::Governed)
        && cap.source.as_deref().map(Path::new) == Some(expected.as_path())
}

/// Accept a skill body only from the store declared by its owner.
fn canonical_source_allowed(
    ctx: &ConsoleContext,
    cap: &ManagedCapability,
    canonical_dir: &Path,
) -> bool {
    if is_external_shared_skill(ctx, cap) {
        canonical_within_shared_store(&ctx.home, &cap.name, canonical_dir)
    } else {
        canonical_within_store(&ctx.repo_root, canonical_dir)
    }
}

/// Pick a non-clobbering backup path: `<dest>.bak`, then `.bak.1`, `.bak.2`, …
fn next_backup_path(dest: &Path) -> PathBuf {
    let base = format!("{}.bak", dest.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 1;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}.{i}"));
        i += 1;
    }
    candidate
}

/// Pick a non-clobbering temporary rollback path used only during thin-index
/// relink apply. Successful applies remove it before returning.
fn next_replaced_path(dest: &Path) -> PathBuf {
    let base = format!("{}.ags-replaced", dest.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 1;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}.{i}"));
        i += 1;
    }
    candidate
}

/// Outcome of a guarded apply: writes that succeeded, and per-write errors.
/// Errors are kept separate from `applied_writes` so the caller has a real
/// failure signal (rather than `ERROR ...` buried in the success list).
#[derive(Default)]
struct ApplyOutcome {
    applied_writes: Vec<String>,
    errors: Vec<String>,
}

#[derive(Debug)]
enum AppliedChange {
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

/// True iff `name` is a single, safe path component: not empty, not `.`/`..`,
/// no separators or NUL, and exactly one normal component. Keeps host-entry
/// writes from escaping the skills directory.
fn is_safe_path_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(std::path::Component::Normal(c)), None) if c == std::ffi::OsStr::new(name)
    )
}

/// Lexical containment: `path` is under `root` and contains no `..` escapes.
fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Create a symlink `link` → `target` (a directory) on the host's behalf.
/// Cross-platform; errors cleanly (→ apply error) where symlinks are
/// unsupported, rather than writing an unusable entry.
#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
#[cfg(not(any(unix, windows)))]
fn make_symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "thin-index symlink not supported on this platform",
    ))
}

/// Remove a host entry (symlink or real dir). A missing path is success.
fn remove_host_entry(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(path),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// A scratch sibling path for staging a symlink before the atomic swap.
fn staging_path(entry: &Path) -> PathBuf {
    PathBuf::from(format!("{}.ags-tmp", entry.display()))
}

/// Read-only parent validation for preflight. This never creates directories.
fn validate_parent_path(parent: &Path) -> std::io::Result<()> {
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if path.is_dir() {
                return Ok(());
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists but is not a directory", path.display()),
            ));
        }
        current = path.parent();
    }
    Ok(())
}

/// Create missing parent directories during execution and record each one so a
/// later batch failure can roll them back. Preflight remains read-only.
fn ensure_parent_dirs(parent: &Path, changes: &mut Vec<AppliedChange>) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if !path.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} exists but is not a directory", path.display()),
                ));
            }
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    for dir in missing.iter().rev() {
        std::fs::create_dir(dir)?;
        changes.push(AppliedChange::CreatedDir(dir.clone()));
    }
    Ok(())
}

/// Transactionally install a thin-index symlink at `entry` → `canonical`.
/// Existing entries are moved to a temporary rollback sibling during the batch,
/// then removed after the whole apply succeeds. No `.bak` host clutter is left.
/// On **any** failure before success cleanup, the original entry is restored.
fn transactional_relink(
    entry: &Path,
    canonical: &Path,
) -> std::io::Result<(String, AppliedChange)> {
    let tmp = staging_path(entry);
    // 1. Stage the new symlink first. If this fails, nothing has moved.
    let _ = remove_host_entry(&tmp);
    make_symlink(canonical, &tmp)?;
    // 2. Move any existing entry to a temporary rollback path.
    let previous = if std::fs::symlink_metadata(entry).is_ok() {
        let old = next_replaced_path(entry);
        if let Err(e) = std::fs::rename(entry, &old) {
            let _ = remove_host_entry(&tmp);
            return Err(e);
        }
        Some(old)
    } else {
        None
    };
    // 3. Swap the staged link into place. On failure, roll the previous entry back.
    if let Err(e) = std::fs::rename(&tmp, entry) {
        if let Some(old) = &previous {
            let _ = std::fs::rename(old, entry);
        }
        let _ = remove_host_entry(&tmp);
        return Err(e);
    }
    let msg = match &previous {
        Some(_) => format!(
            "relink {} -> {} (old entry replaced; no .bak kept)",
            entry.display(),
            canonical.display()
        ),
        None => format!("relink {} -> {}", entry.display(), canonical.display()),
    };
    Ok((
        msg,
        AppliedChange::Relink {
            entry: entry.to_path_buf(),
            previous,
        },
    ))
}

/// Move an existing thin index aside to `.bak`. Missing entry → no-op.
fn transactional_unlink(entry: &Path) -> std::io::Result<Option<(String, AppliedChange)>> {
    if std::fs::symlink_metadata(entry).is_err() {
        return Ok(None);
    }
    let bak = next_backup_path(entry);
    std::fs::rename(entry, &bak)?;
    Ok(Some((
        format!("unlinked {} (moved to {})", entry.display(), bak.display()),
        AppliedChange::Unlink {
            entry: entry.to_path_buf(),
            backup: bak,
        },
    )))
}

fn rollback_change(change: &AppliedChange) -> std::io::Result<()> {
    match change {
        AppliedChange::CreatedDir(path) => match std::fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        AppliedChange::Relink { entry, previous } => {
            remove_host_entry(entry)?;
            if let Some(old) = previous {
                if old.exists() {
                    std::fs::rename(old, entry)?;
                }
            }
            Ok(())
        }
        AppliedChange::Unlink { entry, backup } => {
            if backup.exists() {
                if std::fs::symlink_metadata(entry).is_ok() {
                    remove_host_entry(entry)?;
                }
                std::fs::rename(backup, entry)?;
            }
            Ok(())
        }
    }
}

fn rollback_changes(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes.iter().rev() {
        if let Err(e) = rollback_change(change) {
            errors.push(format!("rollback {:?}: {e}", change));
        }
    }
    errors
}

fn cleanup_successful_relinks(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes {
        if let AppliedChange::Relink {
            previous: Some(old),
            ..
        } = change
        {
            if let Err(e) = remove_host_entry(old) {
                errors.push(format!("cleanup replaced entry {}: {e}", old.display()));
            }
        }
    }
    errors
}

/// The single mutation gate. Returns which writes succeeded and which errored.
///
/// When `confirmed` is false it performs **no** filesystem writes. It first
/// PREFLIGHTS every planned write (containment + host skills dir creatable); if
/// any host fails preflight, NOTHING is mutated — a later host's failure can
/// never leave an earlier host half-changed. Each `relink`/`unlink` then runs
/// transactionally (stage → temporary rollback path → atomic swap). The batch also keeps a
/// rollback stack, so a later host failure restores earlier hosts and removes
/// directories created during this apply. Only thin-index ops run; no skill body
/// is copied; no external command is executed.
fn guarded_apply(confirmed: bool, planned: &[PlannedWrite], ctx: &ConsoleContext) -> ApplyOutcome {
    let mut outcome = ApplyOutcome::default();
    if !confirmed {
        return outcome;
    }
    let allowed_roots: Vec<PathBuf> = supported_skill_hosts()
        .iter()
        .filter_map(|h| host_skills_subdir(h).map(|s| ctx.home.join(s)))
        .collect();

    // ── Preflight: validate ALL destinations before mutating ANY ──
    let mut preflight_errors: Vec<String> = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        if !allowed_roots.iter().any(|r| within(path, r)) {
            preflight_errors.push(format!(
                "refused: write target escapes the host skill roots: {}",
                w.path
            ));
            continue;
        }
        match w.op.as_str() {
            "relink" => {
                if w.from.is_none() {
                    preflight_errors.push(format!("relink {}: no canonical target", w.path));
                } else if let Some(parent) = path.parent() {
                    if let Err(e) = validate_parent_path(parent) {
                        preflight_errors.push(format!(
                            "relink {}: host skills dir not creatable: {e}",
                            w.path
                        ));
                    }
                } else {
                    preflight_errors.push(format!("relink {}: no parent directory", w.path));
                }
            }
            "unlink" => {}
            other => preflight_errors.push(format!("unknown op '{other}' for {}", w.path)),
        }
    }
    if !preflight_errors.is_empty() {
        // Abort with zero mutation so no host is left half-changed.
        outcome.errors = preflight_errors;
        return outcome;
    }

    // ── Execute: each op is transactional; the batch rolls back on first error ──
    let mut changes = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        match w.op.as_str() {
            "relink" => {
                let target = w.from.as_ref().expect("preflight guaranteed a target");
                if let Some(parent) = path.parent() {
                    if let Err(e) = ensure_parent_dirs(parent, &mut changes) {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
                match transactional_relink(path, Path::new(target)) {
                    Ok((msg, change)) => {
                        outcome.applied_writes.push(msg);
                        changes.push(change);
                    }
                    Err(e) => {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
            }
            "unlink" => match transactional_unlink(path) {
                Ok(Some((msg, change))) => {
                    outcome.applied_writes.push(msg);
                    changes.push(change);
                }
                Ok(None) => {}
                Err(e) => {
                    outcome.errors.push(format!("unlink {}: {e}", w.path));
                    outcome.errors.extend(rollback_changes(&changes));
                    outcome.applied_writes.clear();
                    return outcome;
                }
            },
            _ => {} // unknown ops already rejected in preflight
        }
    }
    let cleanup_errors = cleanup_successful_relinks(&changes);
    if !cleanup_errors.is_empty() {
        outcome.errors = cleanup_errors;
        outcome.applied_writes.clear();
        return outcome;
    }
    outcome
}

// ── Rendering ────────────────────────────────────────────────────────────────

pub fn render_inventory_json(result: &ManagedInventoryResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

pub fn render_inventory_text(result: &ManagedInventoryResult) -> String {
    let mut lines = Vec::new();
    lines.push("Skill & MCP Management Console — Inventory".to_string());
    lines.push("==========================================".to_string());
    lines.push(format!("Schema: {}", result.schema_version));
    lines.push(format!("Hosts:  {}", result.hosts.join(", ")));
    lines.push(String::new());
    let s = &result.summary;
    lines.push(format!(
        "Summary: total {} (skills {}, mcps {}, suite-interfaces {}, cli-backed {}); canonical {}, claude-visible {}, risk-flagged {}",
        s.total, s.skills, s.mcps, s.suite_interfaces, s.cli_backed, s.canonical_present, s.claude_visible, s.risk_flagged
    ));
    lines
        .push("(canonical = AGS holds the one body; per-host = thin-index visibility)".to_string());
    lines.push(String::new());
    for c in &result.capabilities {
        // Per-host thin-index visibility, e.g. "claude-code:Visible codex:NotVisible".
        let hosts: String = c
            .host_visibility
            .iter()
            .map(|v| format!("{}:{:?}", v.host, v.status))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!(
            "  [{}] {} — managed:{} canonical:{} health:{:?} | {}",
            kind_str(&c.kind),
            c.name,
            managed_status_str(&c.managed_status),
            if c.canonical_present {
                "present"
            } else {
                "absent"
            },
            c.health_status,
            hosts,
        ));
        if !c.actions.is_empty() {
            lines.push(format!("      actions: {}", c.actions.join(", ")));
        }
        for r in &c.risk_notes {
            lines.push(format!("      ⚠ {r}"));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

pub fn render_verify_json(result: &HostVerifyResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

pub fn render_verify_text(result: &HostVerifyResult) -> String {
    let mut lines = Vec::new();
    lines.push("Host Visibility Verify".to_string());
    lines.push("======================".to_string());
    lines.push(format!("Host:      {}", result.host));
    lines.push(format!("Supported: {}", result.supported));
    lines.push(format!("Status:    {}", result.status));
    if result.supported {
        let s = &result.summary;
        lines.push(format!(
            "Summary:   total {} (visible {}, not-visible {}, degraded {}); expected {}, failed {}, all_visible {}",
            s.total, s.visible, s.not_visible, s.degraded, s.expected, s.failed, s.all_visible
        ));
        lines.push(String::new());
        for c in &result.checks {
            let exp = if c.expected { " (expected)" } else { "" };
            lines.push(format!(
                "  [{}] {} — {:?}{}",
                c.kind, c.name, c.visibility, exp
            ));
            for e in &c.evidence {
                lines.push(format!("      {e}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

pub fn render_proposal_json(result: &ConsoleProposalResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

pub fn render_proposal_text(result: &ConsoleProposalResult) -> String {
    let mut lines = Vec::new();
    lines.push("Skill & MCP Console — Proposal".to_string());
    lines.push("==============================".to_string());
    lines.push(format!("Action:         {}", result.action));
    lines.push(format!("Capability:     {}", result.capability));
    lines.push(format!("Found:          {}", result.found));
    lines.push(format!("Apply requested:{}", result.apply_requested));
    lines.push(format!("Applied:        {}", result.applied));
    lines.push(format!("Apply status:   {}", result.apply_status));
    lines.push(String::new());

    lines.push("─ Planned writes ─".to_string());
    if result.planned_writes.is_empty() {
        lines.push("  (none — AGS owns no file for this action)".to_string());
    } else {
        for w in &result.planned_writes {
            lines.push(format!(
                "  {} {}{}",
                w.op,
                w.path,
                w.from
                    .as_ref()
                    .map(|f| format!(" (from {f})"))
                    .unwrap_or_default()
            ));
        }
    }
    lines.push(String::new());

    if !result.applied_writes.is_empty() {
        lines.push("─ Applied writes ─".to_string());
        for a in &result.applied_writes {
            lines.push(format!("  ✓ {a}"));
        }
        lines.push(String::new());
    }

    if !result.apply_errors.is_empty() {
        lines.push("─ Apply errors (apply did NOT fully succeed) ─".to_string());
        for e in &result.apply_errors {
            lines.push(format!("  ✗ {e}"));
        }
        lines.push(String::new());
    }

    if !result.advised_commands.is_empty() {
        lines.push("─ Advised commands (AGS will NOT run these) ─".to_string());
        for c in &result.advised_commands {
            lines.push(format!("  $ {}", c.command));
            lines.push(format!("    reason: {}", c.reason));
        }
        lines.push(String::new());
    }

    if !result.blocked_reasons.is_empty() {
        lines.push("─ Blocked ─".to_string());
        for b in &result.blocked_reasons {
            lines.push(format!("  ✗ {b}"));
        }
        lines.push(String::new());
    }

    if !result.risk_notes.is_empty() {
        lines.push("─ Risk notes ─".to_string());
        for r in &result.risk_notes {
            lines.push(format!("  ⚠ {r}"));
        }
        lines.push(String::new());
    }

    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

// ── Cross-Agent capability sync ──────────────────────────────────────────────
//
// `sync_plan` is the batch face: it builds the inventory ONCE and produces an
// adopt proposal for every adopted/governed capability, so a single call shows
// (and, with `apply`, performs) the cross-host entry plan for the whole set.
// AGS-owned skill thin-index writes go through the same single mutation guard;
// MCP / CLI-backed capabilities remain advise-only. Reused by
// `ags capability sync`.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySyncSummary {
    /// Capabilities considered for sync (adopted suite skills + governed MCPs).
    pub considered: usize,
    /// Total AGS-owned writes planned across all considered capabilities.
    pub planned_writes: usize,
    /// Capabilities whose AGS-owned writes were applied (apply mode only).
    pub applied: usize,
    /// Capabilities whose only action is an advised host command AGS never runs.
    pub advised_only: usize,
    /// Capabilities with at least one blocked reason.
    pub blocked: usize,
    /// Capabilities whose apply errored.
    pub failed: usize,
    /// Capabilities that need action (planned writes or advised commands).
    pub needs_action: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySyncResult {
    pub schema_version: String,
    pub hosts: Vec<String>,
    pub apply_requested: bool,
    pub items: Vec<ConsoleProposalResult>,
    pub summary: CapabilitySyncSummary,
    pub note: String,
}

/// A capability is syncable through the console when AGS governs it as an
/// adopted suite skill (distributable thin-index) or a governed MCP
/// (advise-only). AGS self (suite-interface), discovered/ignored/unmanaged
/// capabilities are never auto-synced.
fn is_syncable(cap: &ManagedCapability) -> bool {
    // Retired capabilities are never synced into a host, regardless of
    // managed_status — a retired front-stage entry must not be resurrected.
    if cap
        .routing
        .as_ref()
        .is_some_and(|r| r.route_state == RouteState::Retired)
    {
        return false;
    }
    matches!(
        cap.managed_status,
        ManagedStatus::SuiteManaged | ManagedStatus::Governed
    )
}

/// Build (and, with `apply`, perform) the cross-host entry plan for every
/// adopted/governed capability. Builds the inventory once and reuses it.
pub fn sync_plan(ctx: &ConsoleContext, hosts: &[&str], apply: bool) -> CapabilitySyncResult {
    let inventory = build_inventory(ctx, hosts);
    let mut items: Vec<ConsoleProposalResult> = Vec::new();
    for cap in &inventory.capabilities {
        if is_syncable(cap) {
            items.push(propose_action_inner(
                ctx,
                &inventory,
                ConsoleAction::Adopt,
                &cap.name,
                apply,
            ));
        }
    }

    let summary = CapabilitySyncSummary {
        considered: items.len(),
        planned_writes: items.iter().map(|i| i.planned_writes.len()).sum(),
        applied: items.iter().filter(|i| i.applied).count(),
        advised_only: items
            .iter()
            .filter(|i| i.apply_status == "advised-only" || !i.advised_commands.is_empty())
            .count(),
        blocked: items
            .iter()
            .filter(|i| !i.blocked_reasons.is_empty())
            .count(),
        failed: items.iter().filter(|i| !i.apply_errors.is_empty()).count(),
        needs_action: items
            .iter()
            .filter(|i| !i.planned_writes.is_empty() || !i.advised_commands.is_empty())
            .count(),
    };

    CapabilitySyncResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        apply_requested: apply,
        items,
        summary,
        note: if apply {
            "Cross-Agent sync apply: AGS-owned skill thin-index writes were performed through the single guard; MCP / CLI-backed capabilities are advised-only (AGS ran nothing). Restart each host, then `ags capability verify --host <host>`.".to_string()
        } else {
            "Cross-Agent sync plan (dry-run): nothing written, no external command run. Re-run with `--apply` to write AGS-owned skill thin-index entries; MCP / CLI registration is always advised, never run by AGS.".to_string()
        },
    }
}

/// Render the sync result as JSON.
pub fn render_sync_json(result: &CapabilitySyncResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render the sync result as compact human-readable text (one line per
/// capability + summary). Full per-capability detail is available via
/// `ags capability install --capability <name>`.
pub fn render_sync_text(result: &CapabilitySyncResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Cross-Agent Capability Sync".to_string());
    lines.push("===========================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Hosts:   {}", result.hosts.join(", ")));
    lines.push(format!(
        "Mode:    {}",
        if result.apply_requested {
            "apply"
        } else {
            "dry-run"
        }
    ));
    lines.push(format!(
        "Summary: considered {}, needs-action {}, planned-writes {}, applied {}, advised-only {}, blocked {}, failed {}",
        result.summary.considered,
        result.summary.needs_action,
        result.summary.planned_writes,
        result.summary.applied,
        result.summary.advised_only,
        result.summary.blocked,
        result.summary.failed,
    ));
    lines.push(String::new());
    lines.push("─ Capabilities ─".to_string());
    if result.items.is_empty() {
        lines.push("  None syncable (no adopted suite skills or governed MCPs).".to_string());
    } else {
        for item in &result.items {
            lines.push(format!(
                "  [{}] {} ({}) — writes: {}, advised: {}{}",
                item.apply_status,
                item.capability,
                item.kind.as_deref().unwrap_or("?"),
                item.planned_writes.len(),
                item.advised_commands.len(),
                if item.blocked_reasons.is_empty() {
                    String::new()
                } else {
                    format!(", blocked: {}", item.blocked_reasons.len())
                },
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

// ── Skill deduplication ──────────────────────────────────────────────────────
//
// Detects skills that appear under more than one canonical store (a name
// collision) or whose SKILL.md front-matter `name` disagrees with the directory
// name. Default dry-run: a proposal only. With `apply`, the non-keeper copies of
// a name collision are *quarantined* (moved, never deleted) into
// `governance/backups/dedupe-<stamp>/` — an AGS-owned, reversible location
// inside the repo. Canonical bodies are never deleted and host directories are
// never touched. Front-matter mismatches are always advise-only.

/// One copy of a (potentially) duplicated capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateEntry {
    pub path: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_name: Option<String>,
}

/// A planned reversible quarantine move (never a delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineMove {
    pub from: String,
    pub to: String,
}

/// A group of copies sharing one capability name (or a single mismatch entry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub name: String,
    /// name-collision | front-matter-name-mismatch
    pub reason: String,
    pub entries: Vec<DuplicateEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    pub quarantine: Vec<QuarantineMove>,
    pub advice: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeSummary {
    pub groups: usize,
    pub duplicate_entries: usize,
    pub planned_quarantines: usize,
    pub applied: usize,
    pub blocked: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeResult {
    pub schema_version: String,
    pub apply_requested: bool,
    /// dry-run | applied | failed | nothing-to-do | blocked
    pub apply_status: String,
    pub groups: Vec<DuplicateGroup>,
    pub applied_writes: Vec<String>,
    /// Successful quarantine moves (from → to) for rollback-plan construction.
    /// Cleared to empty when a partial failure is rolled back (nothing applied).
    pub applied_moves: Vec<QuarantineMove>,
    pub apply_errors: Vec<String>,
    pub summary: DedupeSummary,
    pub note: String,
}

fn category_rank(category: &str) -> u8 {
    match category {
        "global" => 0,
        "optional" => 1,
        "personal" => 2,
        _ => 3,
    }
}

fn dedupe_stamp() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Detect duplicate skills across the canonical stores. Read-only unless
/// `apply`. Never deletes canonical bodies; never touches host directories.
pub fn analyze_duplicates(repo_root: &Path, apply: bool) -> DedupeResult {
    use std::collections::BTreeMap;

    let scan = crate::scan_skill_inventory(repo_root);
    let stamp = dedupe_stamp();

    let mut by_name: BTreeMap<String, Vec<&crate::SkillInventoryEntry>> = BTreeMap::new();
    for e in &scan.entries {
        by_name.entry(e.name.clone()).or_default().push(e);
    }

    let mut groups: Vec<DuplicateGroup> = Vec::new();

    // 1) name collisions across stores.
    for (name, entries) in &by_name {
        if entries.len() < 2 {
            continue;
        }
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| {
            category_rank(&a.source_category)
                .cmp(&category_rank(&b.source_category))
                .then(a.path.cmp(&b.path))
        });
        let keeper = sorted.first().map(|e| e.path.clone());
        let mut quarantine = Vec::new();
        for e in sorted.iter().skip(1) {
            let base = Path::new(&e.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
            let to = repo_root
                .join("governance/backups")
                .join(format!("dedupe-{stamp}"))
                .join(format!("{}__{}", e.source_category, base))
                .display()
                .to_string();
            quarantine.push(QuarantineMove {
                from: e.path.clone(),
                to,
            });
        }
        groups.push(DuplicateGroup {
            name: name.clone(),
            reason: "name-collision".to_string(),
            entries: sorted
                .iter()
                .map(|e| DuplicateEntry {
                    path: e.path.clone(),
                    category: e.source_category.clone(),
                    declared_name: None,
                })
                .collect(),
            keeper,
            quarantine,
            advice: vec![
                "Keeper is the highest-priority store copy; review before quarantining."
                    .to_string(),
            ],
            blocked_reasons: Vec::new(),
        });
    }

    // 2) front-matter name mismatch (advise-only; a rename is a human decision).
    for e in &scan.entries {
        let base = Path::new(&e.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !base.is_empty() && base != e.name {
            groups.push(DuplicateGroup {
                name: e.name.clone(),
                reason: "front-matter-name-mismatch".to_string(),
                entries: vec![DuplicateEntry {
                    path: e.path.clone(),
                    category: e.source_category.clone(),
                    declared_name: Some(e.name.clone()),
                }],
                keeper: None,
                quarantine: Vec::new(),
                advice: vec![format!(
                    "Directory `{base}` declares front-matter name `{}`; rename is manual.",
                    e.name
                )],
                blocked_reasons: vec!["rename-is-manual".to_string()],
            });
        }
    }

    // apply: stage + validate the ENTIRE move set first, then execute; roll back
    // every successful move if any later move fails. Canonical bodies are never
    // deleted; a failure never leaves a half-quarantined set on disk.
    let mut applied_writes: Vec<String> = Vec::new();
    let mut applied_moves: Vec<QuarantineMove> = Vec::new();
    let mut apply_errors: Vec<String> = Vec::new();
    let backups_root = repo_root.join("governance/backups");
    if apply {
        let all_moves: Vec<&QuarantineMove> =
            groups.iter().flat_map(|g| g.quarantine.iter()).collect();
        // 1) pre-validate containment + destination availability for ALL moves.
        let mut staging_errors: Vec<String> = Vec::new();
        for mv in &all_moves {
            let from = Path::new(&mv.from);
            let to = Path::new(&mv.to);
            if !canonical_within_store(repo_root, from) {
                staging_errors.push(format!(
                    "blocked (source outside canonical store): {}",
                    mv.from
                ));
            } else if !to.starts_with(&backups_root) {
                staging_errors.push(format!(
                    "blocked (dest outside governance/backups): {}",
                    mv.to
                ));
            } else if to.exists() {
                staging_errors.push(format!(
                    "blocked (quarantine dest already exists): {}",
                    mv.to
                ));
            }
        }
        if !staging_errors.is_empty() {
            // zero-change abort: nothing is moved.
            apply_errors = staging_errors;
        } else {
            // 2) execute; track successful moves for rollback.
            let mut failed = false;
            for mv in &all_moves {
                let from = Path::new(&mv.from);
                let to = Path::new(&mv.to);
                if let Some(parent) = to.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        apply_errors.push(format!("mkdir {}: {e}", parent.display()));
                        failed = true;
                        break;
                    }
                }
                match std::fs::rename(from, to) {
                    Ok(()) => {
                        applied_writes.push(mv.to.clone());
                        applied_moves.push((*mv).clone());
                    }
                    Err(e) => {
                        apply_errors.push(format!("rename {} -> {}: {e}", mv.from, mv.to));
                        failed = true;
                        break;
                    }
                }
            }
            // 3) on failure, roll back successful moves (reverse order).
            if failed {
                for mv in applied_moves.iter().rev() {
                    if let Err(e) = std::fs::rename(&mv.to, &mv.from) {
                        apply_errors.push(format!("rollback failed {} -> {}: {e}", mv.to, mv.from));
                    }
                }
                applied_writes.clear();
                applied_moves.clear();
            }
        }
    }

    let groups_len = groups.len();
    let planned: usize = groups.iter().map(|g| g.quarantine.len()).sum();
    let blocked: usize = groups
        .iter()
        .filter(|g| !g.blocked_reasons.is_empty())
        .count();
    let duplicate_entries: usize = groups
        .iter()
        .filter(|g| g.reason == "name-collision")
        .map(|g| g.entries.len())
        .sum();
    let applied_len = applied_writes.len();
    let failed_len = apply_errors.len();

    let apply_status = if !apply {
        "dry-run"
    } else if !apply_errors.is_empty() {
        "failed"
    } else if planned == 0 {
        "nothing-to-do"
    } else if applied_writes.is_empty() {
        "blocked"
    } else {
        "applied"
    }
    .to_string();

    DedupeResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        apply_requested: apply,
        apply_status,
        groups,
        applied_writes,
        applied_moves,
        apply_errors,
        summary: DedupeSummary {
            groups: groups_len,
            duplicate_entries,
            planned_quarantines: planned,
            applied: applied_len,
            blocked,
            failed: failed_len,
        },
        note: "Canonical bodies are never deleted; non-keeper copies are quarantined into governance/backups (reversible). Host directories are never touched."
            .to_string(),
    }
}

/// Render a dedupe result as pretty JSON.
pub fn render_dedupe_json(result: &DedupeResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {e}"}}"#))
}

/// Render a dedupe result as human-readable text (quiet-by-default).
pub fn render_dedupe_text(result: &DedupeResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Deduplication".to_string());
    lines.push("===================".to_string());
    lines.push(format!(
        "Mode: {} | groups {} | duplicate entries {} | planned quarantines {} | applied {} | blocked {} | failed {}",
        result.apply_status,
        result.summary.groups,
        result.summary.duplicate_entries,
        result.summary.planned_quarantines,
        result.summary.applied,
        result.summary.blocked,
        result.summary.failed,
    ));
    if result.groups.is_empty() {
        lines.push("  No duplicates detected.".to_string());
    }
    for g in &result.groups {
        lines.push(format!("  [{}] {}", g.reason, g.name));
        if let Some(keeper) = &g.keeper {
            lines.push(format!("    keeper: {keeper}"));
        }
        for mv in &g.quarantine {
            lines.push(format!("    quarantine: {} -> {}", mv.from, mv.to));
        }
        for b in &g.blocked_reasons {
            lines.push(format!("    blocked: {b}"));
        }
    }
    if !result.apply_errors.is_empty() {
        lines.push("Errors:".to_string());
        for e in &result.apply_errors {
            lines.push(format!("  - {e}"));
        }
    }
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_duplicates_detects_name_collision_dry_run() {
        let root = std::env::temp_dir().join(format!("ags-dedupe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for store in ["global-skills", "skill-packs/optional"] {
            let d = root.join(store).join("dup");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("SKILL.md"), "---\nname: dup\ndescription: x\n---\n").unwrap();
        }
        let r = analyze_duplicates(&root, false);
        assert_eq!(r.apply_status, "dry-run");
        assert!(r.applied_writes.is_empty(), "dry-run writes nothing");
        let group = r
            .groups
            .iter()
            .find(|g| g.name == "dup" && g.reason == "name-collision")
            .expect("name-collision group");
        assert_eq!(group.entries.len(), 2);
        assert!(group.keeper.as_deref().unwrap().contains("global-skills"));
        assert_eq!(group.quarantine.len(), 1);
        // dry-run leaves both copies on disk.
        assert!(root.join("global-skills/dup/SKILL.md").is_file());
        assert!(root.join("skill-packs/optional/dup/SKILL.md").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn seed_dup_repo(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ags-dedupe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for store in ["global-skills", "skill-packs/optional"] {
            let d = root.join(store).join("dup");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("SKILL.md"), "---\nname: dup\ndescription: x\n---\n").unwrap();
        }
        root
    }

    #[test]
    fn analyze_duplicates_apply_populates_reversible_moves() {
        let root = seed_dup_repo("apply");
        let r = analyze_duplicates(&root, true);
        assert_eq!(r.apply_status, "applied");
        assert_eq!(r.applied_moves.len(), 1, "one non-keeper quarantined");
        // keeper (global) stays; non-keeper moved out of the optional store.
        assert!(root.join("global-skills/dup/SKILL.md").is_file());
        assert!(!root.join("skill-packs/optional/dup/SKILL.md").is_file());
        // the quarantine target lives under governance/backups and is restorable.
        let mv = &r.applied_moves[0];
        assert!(mv.to.contains("governance/backups"));
        assert!(Path::new(&mv.to).join("SKILL.md").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn analyze_duplicates_apply_failure_leaves_no_partial_quarantine() {
        let root = seed_dup_repo("fail");
        // Make governance/backups a FILE so the quarantine mkdir fails mid-apply.
        std::fs::create_dir_all(root.join("governance")).unwrap();
        std::fs::write(root.join("governance/backups"), "").unwrap();
        let r = analyze_duplicates(&root, true);
        assert_eq!(r.apply_status, "failed");
        assert!(
            r.applied_moves.is_empty(),
            "no partial quarantine on failure"
        );
        assert!(!r.apply_errors.is_empty());
        // both source copies remain in place (nothing half-moved).
        assert!(root.join("global-skills/dup/SKILL.md").is_file());
        assert!(root.join("skill-packs/optional/dup/SKILL.md").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Mock runner: returns canned `claude mcp list` / `codex mcp list`.
    /// CodeBuddy-Code has no supported CLI MCP probe, so it is not invoked here.
    /// PANICS on anything else — so any attempt to run an external installer or
    /// registrar during a test fails loudly. Proves apply never shells out.
    struct StrictMcpRunner {
        claude: CommandOutcome,
        codex: CommandOutcome,
    }
    impl CommandRunner for StrictMcpRunner {
        fn run(&self, program: &str, args: &[&str]) -> CommandOutcome {
            match (program, args) {
                ("claude", ["mcp", "list"]) => self.claude.clone(),
                ("codex", ["mcp", "list"]) => self.codex.clone(),
                _ => panic!(
                    "console must only ever run a read-only `<host> mcp list`, got: {program} {args:?}"
                ),
            }
        }
    }

    // Claude `mcp list` format: `name: cmd ... - ✔ Connected`. ags + context7.
    fn canned_list() -> CommandOutcome {
        CommandOutcome::Ran {
            success: true,
            stdout: "Checking MCP server health…\n\n\
                 ags: /home/.cargo/bin/ags mcp serve --transport stdio - ✔ Connected\n\
                 context7: npx -y @upstash/context7-mcp - ✔ Connected\n\
                 plugin:claude-mem:mcp-search: node -e launcher - ✔ Connected\n"
                .to_string(),
        }
    }

    // Codex `mcp list` format: a padded table. ags + context7 enabled;
    // codegraph deliberately ABSENT (it is codex-expected → drives incomplete).
    fn canned_codex_list() -> CommandOutcome {
        CommandOutcome::Ran {
            success: true,
            stdout: "Name       Command                Args   Env   Cwd   Status   Auth\n\
                 ags        /home/.cargo/bin/ags   mcp    -     -     enabled  Unsupported\n\
                 context7   npx                    args   -     -     enabled  Unsupported\n"
                .to_string(),
        }
    }

    fn ctx_with(tag: &str, list: CommandOutcome) -> (ConsoleContext, PathBuf) {
        ctx_with_repo_dir(tag, list, "repo")
    }

    fn ctx_with_repo_dir(
        tag: &str,
        list: CommandOutcome,
        repo_dir_name: &str,
    ) -> (ConsoleContext, PathBuf) {
        let base = std::env::temp_dir().join(format!("ags-console-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join(repo_dir_name);
        let home = base.join("home");

        write_file(
            &repo.join("manifests/suite.yaml"),
            "schema_version: \"1.0\"\n\
             suite:\n  name: \"test-suite\"\n  version: \"9.9.9\"\n  required:\n\
             \x20   - name: \"demo-skill\"\n      version: \"1.0\"\n      source: \"global-skills/demo-skill\"\n      hash: \"h1\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"demo-skill-ref\"\n\
             \x20 optional:\n\
             \x20   - name: \"lark-shared\"\n      version: \"1.0\"\n      source: \"skill-packs/optional/lark-shared\"\n      hash: \"h2\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"lark-shared-ref\"\n",
        );
        write_file(
            &repo.join("global-skills/demo-skill/SKILL.md"),
            "---\nname: demo-skill\ndescription: demo.\n---\nbody\n",
        );
        write_file(
            &repo.join("skill-packs/optional/lark-shared/SKILL.md"),
            "---\nname: lark-shared\ndescription: lark shared helper.\n---\nbody\n",
        );
        // An on-disk skill NOT in the manifest → should surface as Discovered.
        write_file(
            &repo.join("global-skills/orphan-skill/SKILL.md"),
            "---\nname: orphan-skill\ndescription: not in the manifest.\n---\nbody\n",
        );
        // installed_clients drives expected host visibility:
        //   ags, context7 → claude-code + codex;  codegraph → codex only.
        write_file(
            &repo.join("manifests/mcp-registry.yaml"),
            "schema_version: \"1.0\"\n\
             suite_interfaces:\n  - name: \"ags\"\n    role: \"host_initialization_adapter\"\n    governed: false\n    install:\n      installed_clients:\n        - \"claude-code\"\n        - \"codex\"\n\
             mcps:\n\
             \x20 - name: \"context7\"\n    package:\n      manager: \"npm\"\n    install:\n      installed_clients:\n        - \"claude-code\"\n        - \"codex\"\n\
             \x20 - name: \"codegraph\"\n    package:\n      manager: \"external-cli\"\n    install:\n      installed_clients:\n        - \"codex\"\n\
             \x20 - name: \"plugin:claude-mem:mcp-search\"\n    package:\n      manager: \"claude-plugin\"\n    install:\n      installed_clients:\n        - \"claude-code\"\n",
        );

        let ctx = ConsoleContext::new(
            repo,
            home,
            Box::new(StrictMcpRunner {
                claude: list,
                codex: canned_codex_list(),
            }),
        );
        (ctx, base)
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn seed_external_skill(ctx: &ConsoleContext) -> PathBuf {
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\nskills:\n  - name: lark-calendar\n    profile: optional\n    source: { type: external_cli_skill, manager: lark-cli }\n",
        );
        let shared = ctx.home.join(".agents/skills/lark-calendar");
        write_file(
            &shared.join("SKILL.md"),
            "---\nname: lark-calendar\ndescription: official external body.\n---\n",
        );
        shared
    }

    #[test]
    fn required_registry_parent_missing_body_is_expected() {
        let (ctx, base) = ctx_with("required-registry-parent-missing", canned_list());
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             \x20     upstream: superpowers\n",
        );

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "superpowers");
        assert_eq!(cap.profile.as_deref(), Some("required"));
        assert_eq!(cap.registry_status, RegistryStatus::Registered);
        assert!(!cap.canonical_present);
        assert!(cap.expected_hosts.iter().any(|host| host == "codex"));
        assert_eq!(
            cap.host_visibility
                .iter()
                .find(|visibility| visibility.host == "codex")
                .map(|visibility| visibility.status.clone()),
            Some(HostVisibilityStatus::NotVisible)
        );

        let verify = verify_host(&ctx, "codex");
        assert_eq!(verify.status, "incomplete");
        assert!(verify.checks.iter().any(|check| {
            check.name == "superpowers"
                && check.expected
                && check.visibility == HostVisibilityStatus::NotVisible
        }));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn required_host_system_parent_accepts_direct_codex_host_body() {
        let (ctx, base) = ctx_with("required-host-system-direct", canned_list());
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n",
        );
        write_file(
            &ctx.home.join(".codex/skills/superpowers/SKILL.md"),
            "---\nname: superpowers\ndescription: host body.\n---\n",
        );

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "superpowers");
        assert_eq!(cap.managed_status, ManagedStatus::HostSystem);
        assert!(cap.canonical_present);
        assert_eq!(
            cap.host_visibility
                .iter()
                .find(|visibility| visibility.host == "codex")
                .map(|visibility| visibility.status.clone()),
            Some(HostVisibilityStatus::Visible)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn entrypoint_integrity_degrades_visible_parent_without_expectation_leak() {
        let (ctx, base) = ctx_with("entrypoint-integrity", canned_list());
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
        );
        write_file(
            &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
            "---\nname: superpowers\ndescription: parent router.\n---\n",
        );

        let inv = build_inventory(&ctx, &["codex"]);
        let parent = find(&inv, "superpowers");
        assert_eq!(parent.health_status, HealthStatus::Degraded);
        assert_eq!(
            parent
                .host_visibility
                .iter()
                .find(|visibility| visibility.host == "codex")
                .map(|visibility| visibility.status.clone()),
            Some(HostVisibilityStatus::Degraded)
        );
        let entrypoint = find(&inv, "verification-before-completion");
        assert!(entrypoint.is_route_target());
        assert!(entrypoint.expected_hosts.is_empty());

        let verify = verify_host(&ctx, "codex");
        assert!(verify.checks.iter().any(|check| {
            check.name == "superpowers"
                && check.expected
                && check.visibility == HostVisibilityStatus::Degraded
        }));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn entrypoint_exposure_shape_degrades_parent_when_playbook_is_standalone() {
        let (ctx, base) = ctx_with("entrypoint-exposure-shape", canned_list());
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
        );
        write_file(
            &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
            "---\nname: superpowers\ndescription: parent router.\n---\n",
        );
        write_file(
            &ctx.home.join(
                ".agents/skills/superpowers/playbooks/verification-before-completion/SKILL.md",
            ),
            "---\nname: verification-before-completion\ndescription: nested playbook.\n---\n",
        );
        write_file(
            &ctx.home
                .join(".codex/skills/verification-before-completion/SKILL.md"),
            "---\nname: verification-before-completion\ndescription: stale standalone entry.\n---\n",
        );

        let inv = build_inventory(&ctx, &["codex"]);
        let parent = find(&inv, "superpowers");
        assert_eq!(parent.health_status, HealthStatus::Degraded);
        assert!(parent.host_visibility.iter().any(|visibility| {
            visibility.host == "codex"
                && visibility.status == HostVisibilityStatus::Degraded
                && visibility
                    .evidence
                    .iter()
                    .any(|item| item.contains("unexpected standalone entrypoint"))
        }));
        let standalone = find(&inv, "verification-before-completion");
        assert!(standalone.expected_hosts.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn external_registry_skill_is_governed_from_shared_store() {
        let (ctx, base) = ctx_with("external-registry", canned_list());
        let shared = seed_external_skill(&ctx);

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "lark-calendar");
        assert_eq!(cap.managed_status, ManagedStatus::Governed);
        assert_eq!(cap.registry_status, RegistryStatus::Registered);
        assert!(cap.canonical_present);
        assert_eq!(
            std::fs::canonicalize(cap.source.as_deref().unwrap()).unwrap(),
            std::fs::canonicalize(&shared).unwrap()
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn external_registry_skill_missing_body_fails_closed() {
        let (ctx, base) = ctx_with("external-missing", canned_list());
        let shared = seed_external_skill(&ctx);
        std::fs::remove_dir_all(shared).unwrap();

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "lark-calendar");
        assert!(!cap.canonical_present);
        assert_eq!(
            cap.host_visibility[0].status,
            HostVisibilityStatus::NotVisible
        );

        let proposal = propose_action(&ctx, ConsoleAction::Adopt, "lark-calendar", true);
        assert!(!proposal.applied);
        assert!(proposal.planned_writes.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn external_registry_skill_escape_is_rejected() {
        let (ctx, base) = ctx_with("external-escape", canned_list());
        let shared = seed_external_skill(&ctx);
        std::fs::remove_dir_all(&shared).unwrap();
        let outside = base.join("outside/lark-calendar");
        write_file(
            &outside.join("SKILL.md"),
            "---\nname: lark-calendar\ndescription: outside body.\n---\n",
        );
        make_symlink(&outside, &shared).unwrap();

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "lark-calendar");
        assert!(!cap.canonical_present);
        assert_eq!(
            cap.host_visibility[0].status,
            HostVisibilityStatus::Degraded
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn external_registry_skill_sync_never_mutates_body_or_duplicates_codex() {
        let (ctx, base) = ctx_with("external-sync", canned_list());
        let shared = seed_external_skill(&ctx);

        let result = propose_action(&ctx, ConsoleAction::Adopt, "lark-calendar", false);
        assert!(result.blocked_reasons.is_empty());
        assert!(result.planned_writes.iter().any(|write| {
            write.path.contains(".codebuddy/skills/lark-calendar")
                && write.from.as_deref() == Some(shared.to_str().unwrap())
        }));
        assert!(result
            .planned_writes
            .iter()
            .all(|write| !write.path.contains(".codex/skills/lark-calendar")));
        assert!(result
            .planned_writes
            .iter()
            .all(|write| write.path != shared.to_string_lossy()));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Manifest is the single routing authority, end-to-end: a `routing:` block
    /// in skills-registry.yaml / mcp-registry.yaml is parsed; an entry with no
    /// block is absent (never synthesized); a malformed block fails closed to
    /// absent rather than panicking.
    #[test]
    fn read_routing_metadata_parses_manifests_and_fails_closed() {
        let base = std::env::temp_dir().join(format!("ags-routing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");

        write_file(
            &repo.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: legacy-compat-skill\n    routing:\n      intent_tags: [debug, diagnosing-bugs]\n      mutation_surface: read-only\n      requires_auth: false\n      cost_class: free\n      invoke_hint: \"[skill: legacy-compat-skill]\"\n      route_priority: 10\n      is_compatibility_alias: true\n\
             \x20 - name: no-routing-skill\n    description: has no routing block\n\
             \x20 - name: broken-skill\n    routing: \"not-a-mapping\"\n",
        );
        write_file(
            &repo.join("manifests/mcp-registry.yaml"),
            "mcps:\n\
             \x20 - name: context7\n    routing:\n      intent_tags: [docs-lookup]\n      cost_class: network\n      route_priority: 30\n",
        );

        let read = read_routing_metadata(&repo);
        let map = &read.map;

        // Well-formed skill block: stable facts + alias flag parsed.
        let ad = map
            .get("legacy-compat-skill")
            .expect("legacy-compat-skill routing present");
        assert_eq!(
            ad.intent_tags,
            vec!["debug".to_string(), "diagnosing-bugs".to_string()]
        );
        assert!(ad.is_compatibility_alias);
        assert_eq!(ad.route_priority, 10);
        assert_eq!(ad.mutation_surface, MutationSurface::ReadOnly);

        // MCP routing block parsed from the other manifest.
        let c7 = map.get("context7").expect("context7 routing present");
        assert_eq!(c7.intent_tags, vec!["docs-lookup".to_string()]);
        assert_eq!(c7.cost_class, CostClass::Network);

        // No routing block → absent (single authority, no synthesis).
        assert!(map.get("no-routing-skill").is_none());
        // Malformed block → fail-closed absent from the map, never a panic...
        assert!(map.get("broken-skill").is_none());
        // ...but the failure is SURFACED (not silently swallowed) for doctor.
        assert!(read.parse_failures.contains(&"broken-skill".to_string()));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Backward-compat: existing routing blocks carry NONE of the new fields
    /// (route_state / capability_group / upstream_group / examples). They must
    /// still deserialize, and the new fields must take their fail-closed / empty
    /// defaults — guarding the silent-drop regression where a non-`default` new
    /// field would make every existing block fail to parse.
    #[test]
    fn routing_metadata_legacy_block_round_trips_with_defaults() {
        let legacy = "intent_tags: [debug, diagnosing-bugs]\nscope_tags: [\"*\"]\nmutation_surface: read-only\nrequires_auth: false\ncost_class: free\ninvoke_hint: \"[skill: legacy-compat-skill]\"\nroute_priority: 10\nis_compatibility_alias: true\n";
        let meta: RoutingMetadata =
            serde_yaml::from_str(legacy).expect("legacy routing block must still parse");
        assert_eq!(
            meta.intent_tags,
            vec!["debug".to_string(), "diagnosing-bugs".to_string()]
        );
        assert!(meta.is_compatibility_alias);
        assert_eq!(meta.route_priority, 10);
        // New fields fail-closed / empty by default.
        assert_eq!(meta.route_state, RouteState::NotRoutable);
        assert!(meta.capability_group.is_empty());
        assert!(meta.upstream_group.is_none());
        assert!(meta.examples.positive.is_empty());
        assert!(meta.examples.negative.is_empty());
    }

    /// `route_state` parses all three explicit values, and absence defaults to
    /// the most restrictive `not-routable` (fail-closed).
    #[test]
    fn route_state_parses_and_defaults_fail_closed() {
        let routable: RoutingMetadata =
            serde_yaml::from_str("route_state: routable\nintent_tags: [verify]\n").unwrap();
        assert_eq!(routable.route_state, RouteState::Routable);
        let retired: RoutingMetadata = serde_yaml::from_str("route_state: retired\n").unwrap();
        assert_eq!(retired.route_state, RouteState::Retired);
        let not_routable: RoutingMetadata =
            serde_yaml::from_str("route_state: not-routable\n").unwrap();
        assert_eq!(not_routable.route_state, RouteState::NotRoutable);
        // Absent → fail-closed not-routable.
        let absent: RoutingMetadata = serde_yaml::from_str("intent_tags: [verify]\n").unwrap();
        assert_eq!(absent.route_state, RouteState::NotRoutable);
    }

    /// capability_group (multi-membership), upstream_group, and examples parse as
    /// plain labels / fixtures.
    #[test]
    fn capability_group_upstream_and_examples_parse_as_labels() {
        let yaml = "route_state: routable\ncapability_group: [code-review, verification]\nupstream_group: \"obra/superpowers:requesting-code-review\"\nexamples:\n\x20 positive: [\"帮我做一次代码审查\"]\n\x20 negative: [\"帮我查飞书日历\"]\n";
        let meta: RoutingMetadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            meta.capability_group,
            vec!["code-review".to_string(), "verification".to_string()]
        );
        assert_eq!(
            meta.upstream_group.as_deref(),
            Some("obra/superpowers:requesting-code-review")
        );
        assert_eq!(
            meta.examples.positive,
            vec!["帮我做一次代码审查".to_string()]
        );
        assert_eq!(meta.examples.negative, vec!["帮我查飞书日历".to_string()]);
    }

    #[cfg(unix)]
    fn link_skill_entry(ctx: &ConsoleContext, host_subdir: &str, name: &str, source: &str) {
        let parent = ctx.home.join(host_subdir);
        std::fs::create_dir_all(&parent).unwrap();
        make_symlink(&ctx.repo_root.join(source), &parent.join(name)).unwrap();
    }

    #[cfg(unix)]
    fn write_codex_plugin_skill(ctx: &ConsoleContext, name: &str) {
        write_file(
            &ctx.home
                .join(".codex/plugins/cache/openai-curated/superpowers/test/skills")
                .join(name)
                .join("SKILL.md"),
            &format!("---\nname: {name}\ndescription: plugin skill.\n---\nbody\n"),
        );
    }

    fn find<'a>(inv: &'a ManagedInventoryResult, name: &str) -> &'a ManagedCapability {
        inv.capabilities
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("capability '{name}' not found"))
    }

    #[test]
    fn inventory_distinguishes_all_four_kinds() {
        let (ctx, base) = ctx_with("kinds", canned_list());
        let inv = build_inventory(&ctx, &["claude-code"]);

        assert_eq!(find(&inv, "lark-shared").kind, ManagedKind::Skill);
        assert_eq!(find(&inv, "ags").kind, ManagedKind::SuiteInterface);
        assert_eq!(find(&inv, "context7").kind, ManagedKind::Mcp);
        // external-cli MCP → CLI-backed
        assert_eq!(find(&inv, "codegraph").kind, ManagedKind::CliBacked);
        // synthetic CLI binary for the lark family
        assert_eq!(find(&inv, "lark-cli").kind, ManagedKind::CliBacked);
        // on-disk skill not in the manifest
        assert_eq!(
            find(&inv, "orphan-skill").managed_status,
            ManagedStatus::Discovered
        );
        assert_eq!(
            find(&inv, "orphan-skill").registry_status,
            RegistryStatus::NotRegistered
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn suite_managed_and_registry_status_are_set() {
        let (ctx, base) = ctx_with("status", canned_list());
        let inv = build_inventory(&ctx, &["claude-code"]);
        let lark = find(&inv, "lark-shared");
        assert_eq!(lark.managed_status, ManagedStatus::SuiteManaged);
        assert_eq!(lark.registry_status, RegistryStatus::Registered);
        let ags = find(&inv, "ags");
        assert_eq!(ags.managed_status, ManagedStatus::SuiteInterface);
        // ags offers only verify — it can't be removed via the console
        assert_eq!(ags.actions, vec!["verify".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn claude_skill_path_visible_only_when_entry_present() {
        let (ctx, base) = ctx_with("skillpath", canned_list());
        // Distribute only lark-shared's host entry.
        link_skill_entry(
            &ctx,
            ".claude/skills",
            "lark-shared",
            "skill-packs/optional/lark-shared",
        );
        let inv = build_inventory(&ctx, &["claude-code"]);

        let lark_vis = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(lark_vis.host, "claude-code");
        assert_eq!(lark_vis.status, HostVisibilityStatus::Visible);

        let demo_vis = &find(&inv, "demo-skill").host_visibility[0];
        assert_eq!(demo_vis.status, HostVisibilityStatus::NotVisible);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_skill_is_degraded() {
        let (ctx, base) = ctx_with("dangling", canned_list());
        let skills = ctx.home.join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        std::os::unix::fs::symlink(base.join("nonexistent-target"), skills.join("demo-skill"))
            .unwrap();
        let inv = build_inventory(&ctx, &["claude-code"]);
        assert_eq!(
            find(&inv, "demo-skill").host_visibility[0].status,
            HostVisibilityStatus::Degraded
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn mcp_visibility_from_claude_list() {
        let (ctx, base) = ctx_with("mcpvis", canned_list());
        let inv = build_inventory(&ctx, &["claude-code"]);
        // context7 + ags are in the canned list → visible
        assert_eq!(
            find(&inv, "context7").host_visibility[0].status,
            HostVisibilityStatus::Visible
        );
        assert_eq!(
            find(&inv, "ags").host_visibility[0].status,
            HostVisibilityStatus::Visible
        );
        assert_eq!(
            find(&inv, "plugin:claude-mem:mcp-search").host_visibility[0].status,
            HostVisibilityStatus::Visible
        );
        // codegraph is NOT in the canned list → not visible
        assert_eq!(
            find(&inv, "codegraph").host_visibility[0].status,
            HostVisibilityStatus::NotVisible
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn mcp_visibility_degraded_when_claude_unavailable() {
        let (ctx, base) = ctx_with("mcpunavail", CommandOutcome::Unavailable);
        let inv = build_inventory(&ctx, &["claude-code"]);
        // No panic; MCP checks degrade gracefully.
        assert_eq!(
            find(&inv, "context7").host_visibility[0].status,
            HostVisibilityStatus::Degraded
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn codex_skill_path_and_mcp_visibility_are_real() {
        let (ctx, base) = ctx_with("codexreal", canned_list());
        // Distribute a skill entry into the CODEX skills dir (~/.codex/skills).
        link_skill_entry(
            &ctx,
            ".codex/skills",
            "lark-shared",
            "skill-packs/optional/lark-shared",
        );
        let inv = build_inventory(&ctx, &["codex"]);

        // Codex is now a real (supported) host — not deferred.
        let lark = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(lark.host, "codex");
        assert!(lark.supported);
        assert_eq!(lark.status, HostVisibilityStatus::Visible);

        // MCP visibility from `codex mcp list`: context7 present, codegraph absent.
        assert_eq!(
            find(&inv, "context7").host_visibility[0].status,
            HostVisibilityStatus::Visible
        );
        assert_eq!(
            find(&inv, "codegraph").host_visibility[0].status,
            HostVisibilityStatus::NotVisible
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn codex_skill_path_can_use_shared_agents_source() {
        let (ctx, base) = ctx_with("codexshared", canned_list());
        link_skill_entry(
            &ctx,
            ".agents/skills",
            "lark-shared",
            "skill-packs/optional/lark-shared",
        );
        let inv = build_inventory(&ctx, &["codex"]);

        let lark = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(lark.host, "codex");
        assert!(lark.supported);
        assert_eq!(lark.status, HostVisibilityStatus::Visible);
        assert!(
            lark.evidence
                .iter()
                .any(|e| e.contains("shared skill source visible")),
            "Codex visibility should cite the shared .agents source: {:?}",
            lark.evidence
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn codex_skill_path_can_use_plugin_source() {
        let (ctx, base) = ctx_with("codexplugin", canned_list());
        write_codex_plugin_skill(&ctx, "lark-shared");
        let inv = build_inventory(&ctx, &["codex"]);

        let lark = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(lark.host, "codex");
        assert_eq!(lark.status, HostVisibilityStatus::Visible);
        assert!(
            lark.evidence
                .iter()
                .any(|e| e.contains(".codex/plugins/cache")),
            "Codex visibility should cite the plugin source: {:?}",
            lark.evidence
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn codebuddy_skill_path_visibility_is_real() {
        let (ctx, base) = ctx_with("codebuddyreal", canned_list());
        link_skill_entry(
            &ctx,
            ".codebuddy/skills",
            "demo-skill",
            "global-skills/demo-skill",
        );
        link_skill_entry(
            &ctx,
            ".codebuddy/skills",
            "lark-shared",
            "skill-packs/optional/lark-shared",
        );

        let inv = build_inventory(&ctx, &["codebuddy-code"]);
        let demo = &find(&inv, "demo-skill").host_visibility[0];
        assert_eq!(demo.host, "codebuddy-code");
        assert!(demo.supported);
        assert_eq!(demo.status, HostVisibilityStatus::Visible);

        let verify = verify_host(&ctx, "codebuddy-code");
        assert!(verify.supported);
        assert_eq!(verify.summary.failed, 0);
        assert!(verify.summary.all_visible);
        let expected_demo = verify
            .checks
            .iter()
            .find(|c| c.name == "demo-skill")
            .unwrap();
        assert!(expected_demo.expected);
        assert_eq!(expected_demo.visibility, HostVisibilityStatus::Visible);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn codex_verify_incomplete_when_expected_mcp_missing() {
        let (ctx, base) = ctx_with("codexverify", canned_list());
        // codegraph is codex-expected (installed_clients=[codex]) but absent
        // from the canned `codex mcp list` → verify must NOT report ok.
        let v = verify_host(&ctx, "codex");
        assert!(v.supported);
        assert_eq!(v.status, "incomplete");
        assert!(!v.summary.all_visible);
        let cg = v.checks.iter().find(|c| c.name == "codegraph").unwrap();
        assert!(cg.expected);
        assert_eq!(cg.visibility, HostVisibilityStatus::NotVisible);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cursor_host_is_deferred_with_stable_fields() {
        let (ctx, base) = ctx_with("cursor", canned_list());
        let inv = build_inventory(&ctx, &["cursor"]);
        let v = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(v.host, "cursor");
        assert!(!v.supported);
        assert_eq!(v.status, HostVisibilityStatus::Deferred);

        let verify = verify_host(&ctx, "cursor");
        assert!(!verify.supported);
        assert_eq!(verify.status, "unsupported");
        assert!(verify.checks.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_host_claude_reports_per_capability_checks() {
        let (ctx, base) = ctx_with("verifyclaude", canned_list());
        let v = verify_host(&ctx, "claude-code");
        assert!(v.supported);
        assert!(v.summary.total > 0);
        assert!(v.checks.iter().any(|c| c.name == "context7"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn propose_dry_run_writes_nothing() {
        let (ctx, base) = ctx_with("dryrun", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", false);
        assert!(res.found);
        assert!(!res.applied);
        assert!(res.applied_writes.is_empty());
        assert!(
            !res.planned_writes.is_empty(),
            "dry-run still shows the plan"
        );
        // Crucially: nothing was written to the (injected) home.
        assert!(!ctx
            .home
            .join(".claude/skills/lark-shared/SKILL.md")
            .exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn propose_apply_writes_thin_index_symlink_on_all_hosts() {
        let (ctx, base) = ctx_with("applywrite", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
        assert!(res.applied);
        assert!(res.apply_errors.is_empty());
        // P1.1 + thin index: all supported skill hosts get a symlink (not a copy) into the
        // injected home, and SKILL.md is reachable THROUGH it (canonical body).
        for sub in [".claude/skills", ".codex/skills", ".codebuddy/skills"] {
            let entry = ctx.home.join(sub).join("lark-shared");
            let meta = std::fs::symlink_metadata(&entry).unwrap();
            assert!(
                meta.file_type().is_symlink(),
                "{sub} entry must be a symlink"
            );
            let md = std::fs::read_to_string(entry.join("SKILL.md")).unwrap();
            assert!(md.contains("name: lark-shared"));
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn propose_apply_replaces_existing_entry_without_backup() {
        let (ctx, base) = ctx_with("applyreplace", canned_list());
        // A pre-existing REAL dir entry on claude (e.g. a manual copy).
        let entry = ctx.home.join(".claude/skills/lark-shared");
        write_file(&entry.join("SKILL.md"), "OLD CONTENT");
        let res = propose_action(&ctx, ConsoleAction::Update, "lark-shared", true);
        assert!(res.applied);
        // Capability/skill thin-index relink replaces the host entry in place
        // and must not leave backup clutter in the host skills directory.
        assert!(
            !ctx.home.join(".claude/skills/lark-shared.bak").exists(),
            "thin-index relink must not leave .bak entries"
        );
        assert!(
            !ctx.home.join(".claude/skills/lark-shared.bak.1").exists(),
            "thin-index relink must not leave numbered .bak entries"
        );
        // The active entry is now a symlink to the canonical body.
        assert!(std::fs::symlink_metadata(&entry)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(entry.join("SKILL.md"))
            .unwrap()
            .contains("name: lark-shared"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_capability_apply_writes_nothing() {
        let (ctx, base) = ctx_with("missing", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "does-not-exist", true);
        assert!(!res.found);
        assert!(!res.applied);
        assert!(res.applied_writes.is_empty());
        assert!(!res.blocked_reasons.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn suite_interface_cannot_be_mutated() {
        let (ctx, base) = ctx_with("ifacelock", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Remove, "ags", true);
        assert!(res.found);
        assert!(!res.blocked_reasons.is_empty());
        assert!(!res.applied);
        assert!(res.applied_writes.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-1: an MCP `--apply` must NOT report applied — AGS only advised.
    #[test]
    fn mcp_action_advises_but_never_writes_or_runs() {
        // StrictMcpRunner would panic if apply tried to run anything other than
        // `<host> mcp list`, so a clean run proves no installer ran.
        let (ctx, base) = ctx_with("mcpadvise", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "context7", true);
        assert!(res.found);
        assert!(res.planned_writes.is_empty(), "AGS owns no file for an MCP");
        assert!(res.applied_writes.is_empty());
        // The high-severity finding: applied must be FALSE (AGS did nothing).
        assert!(!res.applied, "MCP apply must not report applied=true");
        assert_eq!(res.apply_status, "advised-only");
        assert!(res
            .advised_commands
            .iter()
            .any(|c| c.command.contains("claude mcp add")));
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-1: a successful skill apply reports applied=true / status "applied".
    #[cfg(unix)]
    #[test]
    fn skill_apply_status_is_applied() {
        let (ctx, base) = ctx_with("applystatus", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
        assert!(res.applied);
        assert_eq!(res.apply_status, "applied");
        assert!(!res.applied_writes.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn lark_distinction_is_explicit() {
        let (ctx, base) = ctx_with("lark", canned_list());
        // Host skill path present; lark is NOT an MCP in the list.
        link_skill_entry(
            &ctx,
            ".claude/skills",
            "lark-shared",
            "skill-packs/optional/lark-shared",
        );
        let inv = build_inventory(&ctx, &["claude-code"]);

        // 1. lark-* skill — fronted by lark-cli (risk note), claude skill path visible.
        let lark = find(&inv, "lark-shared");
        assert_eq!(lark.kind, ManagedKind::Skill);
        assert!(lark.risk_notes.iter().any(|r| r.contains("lark-cli")));
        assert_eq!(
            lark.host_visibility[0].status,
            HostVisibilityStatus::Visible
        );

        // 2. lark-cli binary — distinct CLI-backed capability, Feishu endpoint, degraded health.
        let cli = find(&inv, "lark-cli");
        assert_eq!(cli.kind, ManagedKind::CliBacked);
        assert_eq!(cli.managed_status, ManagedStatus::Unmanaged);
        assert_eq!(cli.health_status, HealthStatus::Degraded);
        assert!(cli.risk_notes.iter().any(|r| r.contains("Feishu")));

        // 3. There is no MCP named "lark" — lark is CLI-backed, not MCP-registered.
        assert!(!inv
            .capabilities
            .iter()
            .any(|c| c.name == "lark" && c.kind == ManagedKind::Mcp));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Read-only thin-index drift scan classifies `.bak` leftovers and dangling
    /// symlinks as drift and a valid symlink as clean — never mutating. (point 2)
    #[cfg(unix)]
    #[test]
    fn scan_thin_index_drift_classifies_bak_and_dangling() {
        let base = std::env::temp_dir().join(format!("ags-drift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let skills = base.join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let target = base.join("canon-target");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, skills.join("clean-skill")).unwrap();
        std::os::unix::fs::symlink(&target, skills.join("clean-skill.bak")).unwrap();
        std::os::unix::fs::symlink(base.join("missing-target"), skills.join("auto-gone")).unwrap();

        let drift = scan_thin_index_drift(&base, "claude-code").expect("scan present");
        assert!(drift.has_drift);
        assert_eq!(drift.bak_leftovers, 1, "one .bak leftover");
        assert_eq!(drift.broken_symlinks, 1, "one dangling symlink");
        assert!(drift.clean_symlinks >= 1, "clean symlink counted");
        // unsupported host has no skills subdir → None.
        assert!(scan_thin_index_drift(&base, "cursor").is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn all_renders_produce_parseable_json() {
        let (ctx, base) = ctx_with("json", canned_list());
        let inv = build_inventory(&ctx, &["claude-code"]);
        let verify = verify_host(&ctx, "claude-code");
        let proposal = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", false);

        for json in [
            render_inventory_json(&inv),
            render_verify_json(&verify),
            render_proposal_json(&proposal),
        ] {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
            assert!(parsed.is_ok(), "render must be valid JSON: {json}");
        }
        // Round-trip the inventory through the public type.
        let reparsed: ManagedInventoryResult =
            serde_json::from_str(&render_inventory_json(&inv)).unwrap();
        assert_eq!(reparsed.schema_version, CONSOLE_SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn console_action_parsing_roundtrips() {
        for a in CONSOLE_ACTIONS {
            let parsed = ConsoleAction::from_str(a).expect("known action");
            assert_eq!(parsed.as_str(), *a);
        }
        assert!(ConsoleAction::from_str("bogus").is_none());
    }

    // ── Adversarial-review regression tests ──────────────────────────────────

    // Finding 1 + R3-2: apply must not report success when a write fails, AND
    // the multi-host preflight must abort with ZERO cross-host drift.
    #[test]
    fn apply_failure_propagates_with_no_cross_host_drift() {
        let (ctx, base) = ctx_with("applyfail", canned_list());
        // Occupy ~/.codex/skills with a FILE so the codex destination fails
        // preflight — claude must NOT be mutated as a result.
        let codex_skills = ctx.home.join(".codex/skills");
        std::fs::create_dir_all(codex_skills.parent().unwrap()).unwrap();
        std::fs::write(&codex_skills, "not a dir").unwrap();

        let res = propose_action(&ctx, ConsoleAction::Adopt, "demo-skill", true);
        assert!(res.found);
        assert!(
            !res.apply_errors.is_empty(),
            "a host write failure must be recorded"
        );
        assert!(!res.applied, "applied must be false when any write errors");
        assert_eq!(res.apply_status, "failed");
        assert!(
            res.applied_writes.is_empty(),
            "preflight abort → zero writes"
        );
        // No cross-host drift: claude's entry was never created.
        assert!(
            std::fs::symlink_metadata(ctx.home.join(".claude/skills/demo-skill")).is_err(),
            "claude must be untouched when codex preflight fails"
        );
        assert!(
            std::fs::symlink_metadata(ctx.home.join(".claude")).is_err(),
            "read-only preflight must not create host directories"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-2: a per-host relink failure leaves the existing entry intact (rollback).
    #[cfg(unix)]
    #[test]
    fn relink_failure_leaves_existing_entry_intact() {
        use std::os::unix::fs::PermissionsExt;
        let (ctx, base) = ctx_with("relinkfail", canned_list());
        // A working pre-existing entry on claude.
        let skills = ctx.home.join(".claude/skills");
        let entry = skills.join("lark-shared");
        write_file(&entry.join("SKILL.md"), "ORIGINAL WORKING ENTRY");
        // Make the claude skills dir read-only so staging the new symlink fails
        // AFTER preflight (which only needs the dir to exist).
        std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o555)).unwrap();

        let res = propose_action(&ctx, ConsoleAction::Update, "lark-shared", true);

        // Restore perms so the entry stays readable for assertions + cleanup.
        std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!res.applied);
        assert!(!res.apply_errors.is_empty(), "staging failure recorded");
        // The original entry is intact — NOT half-removed into a bare .bak.
        assert_eq!(
            std::fs::read_to_string(entry.join("SKILL.md")).unwrap(),
            "ORIGINAL WORKING ENTRY"
        );
        assert!(
            std::fs::symlink_metadata(skills.join("lark-shared.ags-tmp")).is_err(),
            "no staging leftover"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // R4-1: if a later host fails during execution, earlier host changes roll back.
    #[cfg(unix)]
    #[test]
    fn later_host_execution_failure_rolls_back_earlier_host() {
        use std::os::unix::fs::PermissionsExt;
        let (ctx, base) = ctx_with("batchrollback", canned_list());
        std::fs::create_dir_all(&ctx.home).unwrap();
        let codex_skills = ctx.home.join(".codex/skills");
        std::fs::create_dir_all(&codex_skills).unwrap();
        std::fs::set_permissions(&codex_skills, std::fs::Permissions::from_mode(0o555)).unwrap();

        let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);

        std::fs::set_permissions(&codex_skills, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!res.applied);
        assert_eq!(res.apply_status, "failed");
        assert!(!res.apply_errors.is_empty());
        assert!(
            res.applied_writes.is_empty(),
            "failed batch must not report retained writes"
        );
        assert!(
            std::fs::symlink_metadata(ctx.home.join(".claude/skills/lark-shared")).is_err(),
            "claude relink must be rolled back when codex fails later"
        );
        assert!(
            std::fs::symlink_metadata(ctx.home.join(".claude")).is_err(),
            "directories created only for the rolled-back host must be removed"
        );
        assert!(
            std::fs::symlink_metadata(codex_skills.join("lark-shared.ags-tmp")).is_err(),
            "failed codex staging path must be cleaned"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // Finding 2: verify must not report ok when an expected capability is missing.
    #[test]
    fn verify_incomplete_when_expected_skill_not_visible() {
        let (ctx, base) = ctx_with("verifymissing", canned_list());
        // demo-skill is a required suite skill (expected) but no host entry exists.
        let v = verify_host(&ctx, "claude-code");
        assert!(v.supported);
        assert!(!v.summary.all_visible);
        assert!(v.summary.failed >= 1);
        assert_eq!(v.status, "incomplete");
        let demo = v.checks.iter().find(|c| c.name == "demo-skill").unwrap();
        assert!(demo.expected, "required skill is expected-visible");
        assert_eq!(demo.visibility, HostVisibilityStatus::NotVisible);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn verify_ok_when_expected_skill_visible() {
        let (ctx, base) = ctx_with("verifyok", canned_list());
        // Distribute the required skill's host entry → expected set satisfied.
        link_skill_entry(
            &ctx,
            ".claude/skills",
            "demo-skill",
            "global-skills/demo-skill",
        );
        let v = verify_host(&ctx, "claude-code");
        assert!(v.summary.all_visible, "no expected capability is missing");
        assert_eq!(v.summary.failed, 0);
        assert_eq!(v.status, "ok");
        let _ = std::fs::remove_dir_all(&base);
    }

    // Finding 3: unsafe capability names must never escape the skills dir.
    #[test]
    fn is_safe_path_component_rejects_traversal_and_separators() {
        assert!(is_safe_path_component("lark-shared"));
        assert!(is_safe_path_component("demo_skill"));
        for bad in [
            "",
            ".",
            "..",
            "../evil",
            "../../etc/passwd",
            "/etc/passwd",
            "a/b",
            "a\\b",
            "foo/..",
        ] {
            assert!(!is_safe_path_component(bad), "must reject {bad:?}");
        }
    }

    #[test]
    fn within_rejects_escaping_paths() {
        let root = Path::new("/home/.claude/skills");
        assert!(within(Path::new("/home/.claude/skills/foo/SKILL.md"), root));
        assert!(!within(Path::new("/home/.claude/evil/SKILL.md"), root));
        assert!(!within(Path::new("/etc/passwd"), root));
    }

    #[test]
    fn unsafe_discovered_name_blocks_apply_and_writes_nothing() {
        let (ctx, base) = ctx_with("traversal", canned_list());
        // A discovered on-disk skill whose front-matter NAME is a traversal.
        write_file(
            &ctx.repo_root.join("global-skills/evil-dir/SKILL.md"),
            "---\nname: ../../evil\ndescription: hostile name.\n---\n",
        );
        let res = propose_action(&ctx, ConsoleAction::Adopt, "../../evil", true);
        assert!(res.found, "the hostile-named capability is discovered");
        assert!(
            !res.blocked_reasons.is_empty(),
            "unsafe name must be blocked"
        );
        assert!(!res.applied);
        assert!(res.applied_writes.is_empty());
        assert!(res.apply_errors.is_empty(), "blocked before any write");
        // Nothing was created outside the skills dir.
        assert!(!base.join("home/.claude/evil/SKILL.md").exists());
        assert!(!ctx.home.join(".claude/evil/SKILL.md").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── Canonical-store / thin-index regression tests ────────────────────────

    // Goal 4: a thin index keeps reference files reachable (no SKILL.md-only copy).
    #[cfg(unix)]
    #[test]
    fn thin_index_preserves_reference_files() {
        let (ctx, base) = ctx_with("refs", canned_list());
        // A canonical skill body with a dependency file under references/.
        write_file(
            &ctx.repo_root.join("global-skills/refskill/SKILL.md"),
            "---\nname: refskill\ndescription: needs references.\n---\n",
        );
        write_file(
            &ctx.repo_root
                .join("global-skills/refskill/references/workflow.md"),
            "the workflow lives here",
        );
        let res = propose_action(&ctx, ConsoleAction::Adopt, "refskill", true);
        assert!(res.applied, "{:?}", res.apply_errors);
        // The reference file is reachable THROUGH the host thin index.
        let via_host = ctx
            .home
            .join(".claude/skills/refskill/references/workflow.md");
        assert_eq!(
            std::fs::read_to_string(&via_host).unwrap(),
            "the workflow lives here"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // Goal 3: remove unlinks only the thin index; the canonical body survives.
    #[cfg(unix)]
    #[test]
    fn remove_unlinks_thin_index_keeps_canonical() {
        let (ctx, base) = ctx_with("removeindex", canned_list());
        let canonical = ctx
            .repo_root
            .join("skill-packs/optional/lark-shared/SKILL.md");
        assert!(canonical.is_file());

        assert!(propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true).applied);
        let entry = ctx.home.join(".claude/skills/lark-shared");
        assert!(std::fs::symlink_metadata(&entry)
            .unwrap()
            .file_type()
            .is_symlink());

        let res = propose_action(&ctx, ConsoleAction::Remove, "lark-shared", true);
        assert!(res.applied);
        // Active thin index is gone …
        assert!(std::fs::symlink_metadata(&entry).is_err());
        // … but the canonical body is untouched.
        assert!(canonical.is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    // P2.3: a non-zero `mcp list` exit is degraded, not an authoritative empty list.
    #[test]
    fn probe_failure_is_degraded_not_missing() {
        let failing = CommandOutcome::Ran {
            success: false,
            stdout: String::new(),
        };
        let (ctx, base) = ctx_with("probefail", failing);
        let inv = build_inventory(&ctx, &["claude-code"]);
        // context7 must be degraded (couldn't enumerate), NOT not-visible.
        assert_eq!(
            find(&inv, "context7").host_visibility[0].status,
            HostVisibilityStatus::Degraded
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // P2.4: a host entry whose front-matter name differs is not "visible".
    #[test]
    fn skill_name_mismatch_is_degraded() {
        let (ctx, base) = ctx_with("namemismatch", canned_list());
        // Entry path says lark-shared but the SKILL.md declares a different name.
        write_file(
            &ctx.home.join(".claude/skills/lark-shared/SKILL.md"),
            "---\nname: something-else\ndescription: wrong name.\n---\n",
        );
        let inv = build_inventory(&ctx, &["claude-code"]);
        assert_eq!(
            find(&inv, "lark-shared").host_visibility[0].status,
            HostVisibilityStatus::Degraded
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // R4-2: matching front matter is not enough; the host entry must point to the
    // AGS canonical body, not a random external directory.
    #[cfg(unix)]
    #[test]
    fn non_canonical_symlink_is_degraded() {
        let (ctx, base) = ctx_with("external-target", canned_list());
        let outside = base.join("outside/lark-shared");
        write_file(
            &outside.join("SKILL.md"),
            "---\nname: lark-shared\ndescription: external copy.\n---\n",
        );
        let skills = ctx.home.join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        make_symlink(&outside, &skills.join("lark-shared")).unwrap();

        let inv = build_inventory(&ctx, &["claude-code"]);
        let vis = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(vis.status, HostVisibilityStatus::Degraded);
        assert!(
            vis.evidence
                .iter()
                .any(|e| e.contains("expected AGS canonical")),
            "{:?}",
            vis.evidence
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // The development private suite may verify hosts whose thin indexes point at
    // the stable suite runtime. Treat the same relative skill body under the
    // private/stable suite pair as visible, while unrelated external copies still
    // fail the `non_canonical_symlink_is_degraded` guard above.
    #[cfg(unix)]
    #[test]
    fn stable_runtime_twin_symlink_is_visible() {
        let (ctx, base) = ctx_with_repo_dir(
            "stable-runtime",
            canned_list(),
            &format!("agent-governance-suite-{}", "private"),
        );
        let stable_source = base.join(format!(
            "agent-governance-suite-{}/skill-packs/optional/lark-shared",
            "stable"
        ));
        write_file(
            &stable_source.join("SKILL.md"),
            "---\nname: lark-shared\ndescription: stable runtime body.\n---\n",
        );
        let skills = ctx.home.join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        make_symlink(&stable_source, &skills.join("lark-shared")).unwrap();

        let inv = build_inventory(&ctx, &["claude-code"]);
        let vis = &find(&inv, "lark-shared").host_visibility[0];
        assert_eq!(vis.status, HostVisibilityStatus::Visible);
        assert!(
            vis.evidence
                .iter()
                .any(|e| e.contains("AGS stable/private runtime twin")),
            "{:?}",
            vis.evidence
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // Goal 2: canonical body status is modeled distinctly from host visibility.
    #[test]
    fn canonical_present_reflects_the_body() {
        let (ctx, base) = ctx_with("canonical", canned_list());
        let inv = build_inventory(&ctx, &["claude-code"]);
        // Suite skill has a canonical SKILL.md in the store.
        assert!(find(&inv, "lark-shared").canonical_present);
        // The synthetic lark-cli binary is external — AGS holds no canonical body.
        assert!(!find(&inv, "lark-cli").canonical_present);
        // Summary counts the canonical bodies present.
        assert!(inv.summary.canonical_present >= 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-3: canonical containment helper accepts in-store, rejects out-of-store.
    #[test]
    fn canonical_within_store_helper() {
        let (ctx, base) = ctx_with("withinstore", canned_list());
        let inside = ctx.repo_root.join("skill-packs/optional/lark-shared");
        assert!(canonical_within_store(&ctx.repo_root, &inside));
        let outside = base.join("outside-store");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(!canonical_within_store(&ctx.repo_root, &outside));
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-3: a manifest source pointing outside the approved stores is blocked,
    // even with a valid SKILL.md — AGS must not link a host to an arbitrary dir.
    #[test]
    fn canonical_source_outside_store_is_blocked() {
        let (ctx, base) = ctx_with("outsidesrc", canned_list());
        let evil = base.join("evil-store/sneaky");
        write_file(
            &evil.join("SKILL.md"),
            "---\nname: sneaky\ndescription: x.\n---\n",
        );
        // Register it with an ABSOLUTE outside source.
        write_file(
            &ctx.repo_root.join("manifests/suite.yaml"),
            &format!(
                "schema_version: \"1.0\"\nsuite:\n  name: t\n  version: \"9\"\n  optional:\n\
                 \x20   - name: \"sneaky\"\n      version: \"1\"\n      source: {:?}\n      hash: h\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: r\n",
                evil.to_string_lossy()
            ),
        );
        let res = propose_action(&ctx, ConsoleAction::Adopt, "sneaky", true);
        assert!(res.found);
        assert!(
            res.blocked_reasons
                .iter()
                .any(|b| b.contains("outside the store approved")),
            "{:?}",
            res.blocked_reasons
        );
        assert!(!res.applied);
        assert!(res.applied_writes.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    // R3-3: a canonical body whose SKILL.md declares a different name is blocked.
    #[test]
    fn canonical_name_mismatch_is_blocked() {
        let (ctx, base) = ctx_with("canonmismatch", canned_list());
        // Corrupt the canonical body so its declared name no longer matches.
        write_file(
            &ctx.repo_root
                .join("skill-packs/optional/lark-shared/SKILL.md"),
            "---\nname: not-lark-shared\ndescription: mislabeled.\n---\n",
        );
        let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
        assert!(res.found);
        assert!(
            res.blocked_reasons
                .iter()
                .any(|b| b.contains("declares name")),
            "{:?}",
            res.blocked_reasons
        );
        assert!(!res.applied);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn inventory_host_scoping_limits_visibility() {
        let (ctx, base) = ctx_with("hostscope", canned_list());

        // Scope to codex only → every capability's host_visibility is codex.
        let codex_only = build_inventory(&ctx, &["codex"]);
        assert_eq!(codex_only.hosts, vec!["codex".to_string()]);
        for cap in &codex_only.capabilities {
            for v in &cap.host_visibility {
                assert_eq!(v.host, "codex", "host visibility scoped to requested host");
            }
        }

        // Both hosts requested → claude-code visibility is present again.
        let both = build_inventory(&ctx, &["claude-code", "codex"]);
        assert!(
            both.capabilities
                .iter()
                .any(|c| c.host_visibility.iter().any(|v| v.host == "claude-code")),
            "claude-code visibility present when requested"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── Cross-Agent capability sync ──────────────────────────────────────

    #[test]
    fn sync_plan_covers_adopted_and_governed_only() {
        let (ctx, base) = ctx_with("syncdry", canned_list());
        let result = sync_plan(&ctx, &["claude-code", "codex"], false);

        // Syncable = suite-managed skills (demo-skill, lark-shared) + governed
        // MCPs (context7, codegraph). orphan-skill (discovered) and ags
        // (suite-interface) are excluded.
        let names: Vec<&str> = result.items.iter().map(|i| i.capability.as_str()).collect();
        assert!(names.contains(&"demo-skill"));
        assert!(names.contains(&"lark-shared"));
        assert!(names.contains(&"context7"));
        assert!(names.contains(&"codegraph"));
        assert!(!names.contains(&"orphan-skill"), "discovered is not synced");
        assert!(!names.contains(&"ags"), "AGS self is never synced");
        assert_eq!(result.summary.considered, result.items.len());

        // Dry-run: nothing applied; skills plan thin-index writes; MCPs advise.
        assert!(!result.apply_requested);
        assert!(result.items.iter().all(|i| i.apply_status == "dry-run"));
        assert!(result.summary.planned_writes > 0, "skills need thin-index");
        assert!(result.summary.advised_only >= 2, "two governed MCPs advise");
        assert_eq!(result.summary.applied, 0);

        // Renders without panic and reflects dry-run.
        assert!(render_sync_text(&result).contains("Cross-Agent Capability Sync"));
        let json: serde_json::Value = serde_json::from_str(&render_sync_json(&result)).unwrap();
        assert_eq!(json["apply_requested"], false);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn sync_plan_apply_writes_skill_thin_index_only() {
        let (ctx, base) = ctx_with("syncapply", canned_list());
        let result = sync_plan(&ctx, &["claude-code", "codex", "codebuddy-code"], true);

        // At least one suite skill's thin-index was applied; MCPs stayed advised.
        assert!(result.summary.applied >= 1, "skill thin-index applied");
        assert!(result.apply_requested);
        // A governed MCP item must remain advised-only (AGS ran nothing for it).
        let context7 = result
            .items
            .iter()
            .find(|i| i.capability == "context7")
            .expect("context7 considered");
        assert_eq!(context7.apply_status, "advised-only");
        assert!(!context7.applied);
        // demo-skill thin index now exists under a host skills dir.
        let claude_entry = ctx.home.join(".claude/skills/demo-skill");
        assert!(
            claude_entry.exists(),
            "claude thin-index created for demo-skill"
        );
        let codebuddy_entry = ctx.home.join(".codebuddy/skills/demo-skill");
        assert!(
            codebuddy_entry.exists(),
            "codebuddy thin-index created for demo-skill"
        );
        // Safety invariant: every planned write for a synced skill stays within
        // the injected temp home — AGS never escapes to the real $HOME.
        let home_str = ctx.home.to_string_lossy().to_string();
        let demo = result
            .items
            .iter()
            .find(|i| i.capability == "demo-skill")
            .expect("demo-skill considered");
        assert!(!demo.planned_writes.is_empty());
        for w in &demo.planned_writes {
            assert!(
                w.path.contains(&home_str),
                "planned write escaped temp home: {}",
                w.path
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn sync_plan_skips_codex_thin_index_when_shared_agents_source_exists() {
        let (ctx, base) = ctx_with("syncsharedcodex", canned_list());
        link_skill_entry(
            &ctx,
            ".agents/skills",
            "demo-skill",
            "global-skills/demo-skill",
        );

        let result = sync_plan(&ctx, &["codex"], false);
        let demo = result
            .items
            .iter()
            .find(|i| i.capability == "demo-skill")
            .expect("demo-skill considered");

        assert!(
            demo.planned_writes
                .iter()
                .all(|w| !w.path.contains(".codex/skills/demo-skill")),
            "sync must not create a duplicate Codex thin-index when .agents already exposes the skill: {:?}",
            demo.planned_writes
        );
        assert!(
            demo.note.contains("shared skill source already visible"),
            "operator note should explain why Codex was skipped: {}",
            demo.note
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn sync_plan_skips_codex_thin_index_when_plugin_source_exists() {
        let (ctx, base) = ctx_with("syncplugincodex", canned_list());
        write_codex_plugin_skill(&ctx, "demo-skill");

        let result = sync_plan(&ctx, &["codex"], false);
        let demo = result
            .items
            .iter()
            .find(|i| i.capability == "demo-skill")
            .expect("demo-skill considered");

        assert!(
            demo.planned_writes
                .iter()
                .all(|w| !w.path.contains(".codex/skills/demo-skill")),
            "sync must not create a duplicate Codex thin-index when a plugin already exposes the skill: {:?}",
            demo.planned_writes
        );
        assert!(
            demo.note.contains(".codex/plugins/cache"),
            "operator note should explain the plugin source: {}",
            demo.note
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn sync_plan_apply_replaces_existing_thin_index_without_backup() {
        let (ctx, base) = ctx_with("syncnobak", canned_list());
        let entry = ctx.home.join(".claude/skills/demo-skill");
        write_file(&entry.join("SKILL.md"), "OLD CONTENT");

        let result = sync_plan(&ctx, &["claude-code"], true);

        assert!(result.apply_requested);
        assert_eq!(result.summary.failed, 0);
        assert!(
            std::fs::symlink_metadata(&entry)
                .unwrap()
                .file_type()
                .is_symlink(),
            "sync apply should replace the existing host entry with a thin-index symlink"
        );
        assert!(
            std::fs::read_to_string(entry.join("SKILL.md"))
                .unwrap()
                .contains("name: demo-skill"),
            "active entry should resolve to the canonical skill body"
        );
        assert!(
            !ctx.home.join(".claude/skills/demo-skill.bak").exists(),
            "capability sync must not leave .bak backups"
        );
        assert!(
            !ctx.home.join(".claude/skills/demo-skill.bak.1").exists(),
            "capability sync must not leave numbered .bak backups"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Regression (adversarial review): a capability marked `route_state: retired`
    /// must never be (re)adopted/synced into a host, even though its canonical
    /// body is still on disk — this closes the resurrection path
    /// (`ags capability install --capability <retired>`).
    #[test]
    fn retired_capability_is_blocked_from_adoption_and_sync() {
        let base = std::env::temp_dir().join(format!("ags-retired-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let home = base.join("home");

        // A suite-required skill whose routing is `retired` (a deliberately
        // constructed edge case: even a still-required retired skill must be
        // gated) plus a normal required skill as a control.
        write_file(
            &repo.join("manifests/suite.yaml"),
            "schema_version: \"1.0\"\n\
             suite:\n  name: \"t\"\n  version: \"9.9.9\"\n  required:\n\
             \x20   - name: \"retired-demo\"\n      version: \"1.0\"\n      source: \"global-skills/retired-demo\"\n      hash: \"h\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"retired-demo-ref\"\n\
             \x20   - name: \"demo-skill\"\n      version: \"1.0\"\n      source: \"global-skills/demo-skill\"\n      hash: \"h\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"demo-skill-ref\"\n",
        );
        write_file(
            &repo.join("global-skills/retired-demo/SKILL.md"),
            "---\nname: retired-demo\ndescription: retired body still on disk.\n---\nbody\n",
        );
        write_file(
            &repo.join("global-skills/demo-skill/SKILL.md"),
            "---\nname: demo-skill\ndescription: control.\n---\nbody\n",
        );
        write_file(
            &repo.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: retired-demo\n    routing:\n      route_state: retired\n      capability_group: [ags-governance-ops]\n\
             \x20 - name: demo-skill\n    routing:\n      route_state: not-routable\n",
        );
        write_file(&repo.join("manifests/mcp-registry.yaml"), "mcps: []\n");

        let ctx = ConsoleContext::new(
            repo,
            home,
            Box::new(StrictMcpRunner {
                claude: canned_list(),
                codex: canned_codex_list(),
            }),
        );

        // Adopt / Update / Repair of the retired skill are blocked with NO writes.
        for action in [
            ConsoleAction::Adopt,
            ConsoleAction::Update,
            ConsoleAction::Repair,
        ] {
            let res = propose_action(&ctx, action, "retired-demo", false);
            assert!(res.found, "retired body is still discovered: {action:?}");
            assert!(
                !res.blocked_reasons.is_empty(),
                "retired skill must be blocked for {action:?}"
            );
            assert!(
                res.planned_writes.is_empty(),
                "retired skill must plan no host writes for {action:?}"
            );
            assert!(
                res.blocked_reasons.iter().any(|r| r.contains("retired")),
                "block reason must name retirement: {:?}",
                res.blocked_reasons
            );
        }

        // Sync never considers the retired skill; the control IS synced.
        let sync = sync_plan(&ctx, &["claude-code", "codex"], false);
        let names: Vec<&str> = sync.items.iter().map(|i| i.capability.as_str()).collect();
        assert!(
            !names.contains(&"retired-demo"),
            "retired skill must be excluded from sync: {names:?}"
        );
        assert!(
            names.contains(&"demo-skill"),
            "non-retired required skill still syncs: {names:?}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn mcp_adopt_advises_both_claude_and_codex_hosts() {
        let (ctx, base) = ctx_with("mcphosts", canned_list());
        let res = propose_action(&ctx, ConsoleAction::Adopt, "context7", false);
        assert!(res.found);
        let cmds: Vec<&str> = res
            .advised_commands
            .iter()
            .map(|c| c.command.as_str())
            .collect();
        assert!(
            cmds.iter()
                .any(|c| c.starts_with("claude mcp add context7")),
            "{cmds:?}"
        );
        assert!(
            cmds.iter().any(|c| c.starts_with("codex mcp add context7")),
            "{cmds:?}"
        );
        // Still advise-only — AGS never registers MCP servers itself.
        assert!(res.planned_writes.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Full-machine discovery classifies host-dir skills into the taxonomy and is
    /// fail-closed: every discovered host-dir skill is `routing: None`
    /// (not-routable) until adopted. Also exercises symlink safety — dangling
    /// links, external targets, and symlink loops are recognized without panic
    /// and never linked into the AGS store.
    #[cfg(unix)]
    #[test]
    fn discovers_host_dir_system_user_and_unmanaged_skills_fail_closed() {
        let (ctx, base) = ctx_with("hostdir-discovery", canned_list());
        let skills = ctx.home.join(".codex/skills");
        // Real-dir user skill → discovered-local.
        write_file(
            &skills.join("myuserskill/SKILL.md"),
            "---\nname: myuserskill\ndescription: x.\n---\nbody\n",
        );
        // System skill under `.system` → host-system.
        write_file(
            &skills.join(".system/sys-creator/SKILL.md"),
            "---\nname: sys-creator\ndescription: x.\n---\nbody\n",
        );
        // Dangling symlink → unmanaged (no panic).
        std::os::unix::fs::symlink(base.join("does-not-exist-xyz"), skills.join("broken")).unwrap();
        // Symlink to an external location outside any store → unmanaged.
        let external = base.join("external/extskill");
        write_file(
            &external.join("SKILL.md"),
            "---\nname: extskill\ndescription: x.\n---\nbody\n",
        );
        std::os::unix::fs::symlink(&external, skills.join("extskill")).unwrap();
        // Symlink loop → must not panic.
        std::os::unix::fs::symlink(skills.join("loopb"), skills.join("loopa")).unwrap();
        std::os::unix::fs::symlink(skills.join("loopa"), skills.join("loopb")).unwrap();

        let inv = build_inventory(&ctx, &["codex"]);
        let by = |n: &str| {
            inv.capabilities
                .iter()
                .find(|c| c.name == n)
                .cloned()
                .unwrap_or_else(|| panic!("capability {n} not discovered"))
        };

        assert_eq!(by("myuserskill").managed_status, ManagedStatus::Discovered);
        assert_eq!(by("sys-creator").managed_status, ManagedStatus::HostSystem);
        assert_eq!(by("extskill").managed_status, ManagedStatus::Unmanaged);
        assert_eq!(by("broken").managed_status, ManagedStatus::Unmanaged);
        // Fail-closed: NONE of the discovered host-dir skills are routable.
        for n in ["myuserskill", "sys-creator", "extskill", "broken"] {
            assert!(
                by(n).routing.is_none(),
                "{n} must be fail-closed not-routable until adopted"
            );
            assert_eq!(by(n).registry_status, RegistryStatus::NotRegistered);
        }
        // The system skill is canonical-present (its body exists) but never
        // copied — its source is the external `.system` path, not the AGS store.
        assert!(by("sys-creator").canonical_present);
        assert!(by("sys-creator").source.unwrap().contains(".system"));
        // Public boundary: the snapshot hash (a recordable attestation token)
        // embeds capability NAMES + statuses only — never an absolute machine
        // path or a system-skill body — so it is safe to publish / record.
        let hash = inventory_snapshot_hash(&inv);
        assert!(hash.starts_with("fnv1a64:"));
        assert!(!hash.contains('/') && !hash.contains("Users") && !hash.contains(".system"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A discovered host-system `.system/<name>` whose SKILL.md front-matter
    /// declares a DIFFERENT name must read Degraded (not Visible): a mismatched
    /// or replaced body cannot masquerade as the adopted capability for the
    /// runtime skill-tag gate. A matching body reads Visible. (adversarial-review
    /// hardening — host-dir visibility now validates SKILL.md identity like
    /// `skill_path_visibility`.)
    #[test]
    fn host_dir_skill_visibility_validates_front_matter_identity() {
        let (ctx, base) = ctx_with("hostdir-frontmatter", canned_list());
        let skills = ctx.home.join(".codex/skills");
        let codex_vis = |inv: &ManagedInventoryResult| -> HostVisibilityStatus {
            inv.capabilities
                .iter()
                .find(|c| c.name == "skill-creator")
                .and_then(|c| c.host_visibility.iter().find(|v| v.host == "codex"))
                .map(|v| v.status.clone())
                .expect("skill-creator codex visibility")
        };

        // Directory named `skill-creator` but the body declares another name.
        write_file(
            &skills.join(".system/skill-creator/SKILL.md"),
            "---\nname: not-skill-creator\ndescription: impostor.\n---\nbody\n",
        );
        assert_eq!(
            codex_vis(&build_inventory(&ctx, &["codex"])),
            HostVisibilityStatus::Degraded,
            "mismatched SKILL.md front-matter name must be Degraded, not Visible"
        );

        // A body whose front-matter name matches the directory reads Visible.
        write_file(
            &skills.join(".system/skill-creator/SKILL.md"),
            "---\nname: skill-creator\ndescription: real.\n---\nbody\n",
        );
        assert_eq!(
            codex_vis(&build_inventory(&ctx, &["codex"])),
            HostVisibilityStatus::Visible,
            "matching SKILL.md front-matter name must be Visible"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// If a host-system skill is explicitly adopted in skills-registry.yaml, the
    /// inventory row must reflect that registry authority consistently: routable
    /// AND registered. It remains host-system/read-only; only the routing
    /// authority changes.
    #[test]
    fn adopted_host_system_skill_is_registered_in_inventory() {
        let (ctx, base) = ctx_with("hostdir-adopted-registered", canned_list());
        let skills = ctx.home.join(".codex/skills");
        write_file(
            &skills.join(".system/skill-creator/SKILL.md"),
            "---\nname: skill-creator\ndescription: real.\n---\nbody\n",
        );
        write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: skill-creator\n    routing:\n      route_state: routable\n      intent_tags: [skill-authoring]\n      invoke_hint: \"[skill: skill-creator]\"\n",
        );

        let inv = build_inventory(&ctx, &["codex"]);
        let cap = find(&inv, "skill-creator");
        assert_eq!(cap.managed_status, ManagedStatus::HostSystem);
        assert_eq!(cap.registry_status, RegistryStatus::Registered);
        assert_eq!(
            cap.routing.as_ref().map(|r| r.route_state),
            Some(RouteState::Routable)
        );
        assert!(cap
            .risk_notes
            .iter()
            .any(|note| note.contains("registry-adopted for routing")));
        assert!(cap
            .risk_notes
            .iter()
            .all(|note| !note.contains("Adopt via the registry to make it routable")));

        let _ = std::fs::remove_dir_all(&base);
    }
}
