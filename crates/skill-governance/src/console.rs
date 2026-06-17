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
use std::path::{Path, PathBuf};

pub const CONSOLE_SCHEMA_VERSION: &str = "2.6.0-skill-console";

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
    /// Present/known but not yet adopted — an opt-in candidate.
    Discovered,
    /// Explicitly ignored (in the ignore list / rejected in adoption log).
    Ignored,
    /// Present but outside AGS governance.
    Unmanaged,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedInventoryResult {
    pub schema_version: String,
    pub hosts: Vec<String>,
    pub capabilities: Vec<ManagedCapability>,
    pub summary: ManagedInventorySummary,
    pub note: String,
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
/// `name: /path/to/cmd args - ✔ Connected`. Lenient: takes the single token
/// before the first `:` as the server name and detects a connected marker.
fn parse_claude_mcp_list(stdout: &str) -> Vec<(String, bool)> {
    let mut servers = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, rest)) = line.split_once(':') else {
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

const SUPPORTED_HOSTS: &[&str] = &["claude-code", "codex"];
const DEFERRED_HOSTS: &[&str] = &["cursor"];

/// The `~/<subdir>` skills directory a host loads skill entries from, if any.
/// `Some` ⇒ the host is supported and gets a real probe.
fn host_skills_subdir(host: &str) -> Option<&'static str> {
    match host {
        "claude-code" => Some(".claude/skills"),
        "codex" => Some(".codex/skills"),
        _ => None,
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
    probe: Option<&HostMcpProbe>,
) -> HostVisibility {
    if let Some(subdir) = host_skills_subdir(host) {
        return match cap_kind {
            ManagedKind::Skill => {
                skill_path_visibility(host, &ctx.home.join(subdir), cap_name, canonical_source)
            }
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
        if real_entry != real_canonical {
            evidence.push(format!(
                "host thin index points to {}, expected AGS canonical {}",
                real_entry.display(),
                real_canonical.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        }
        evidence.push(format!(
            "thin index resolves to AGS canonical body: {}",
            real_canonical.display()
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
        let expected_hosts = if s.profile == "required" {
            vec!["claude-code".to_string()]
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
        });
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
        let expected_hosts: Vec<String> = e
            .installed_clients
            .iter()
            .filter(|c| SUPPORTED_HOSTS.contains(&c.as_str()))
            .cloned()
            .collect();
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
        });
    }

    // 5. Fill host visibility, health, and actions for every capability.
    for cap in &mut caps {
        let cli_backed_external = matches!(cap.kind, ManagedKind::CliBacked)
            && matches!(cap.managed_status, ManagedStatus::Unmanaged);
        for host in &hosts {
            let probe = probes.iter().find(|(h, _)| h == host).map(|(_, p)| p);
            let canonical_source = if matches!(cap.kind, ManagedKind::Skill) {
                cap.source
                    .as_deref()
                    .map(|source| resolve_source(&ctx.repo_root, source))
            } else {
                None
            };
            cap.host_visibility.push(host_visibility(
                ctx,
                host,
                &cap.kind,
                &cap.name,
                canonical_source.as_deref(),
                probe,
            ));
        }
        cap.health_status = derive_health(
            &cap.kind,
            &cap.name,
            &cap.host_visibility,
            &probes,
            cli_backed_external,
        );
        cap.actions = actions_for(&cap.kind, &cap.managed_status);
    }

    caps.sort_by(|a, b| a.name.cmp(&b.name));

    let summary = summarize(&caps);
    ManagedInventoryResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        hosts,
        capabilities: caps,
        summary,
        note: "Read-only inventory. Third-party capabilities are opt-in; AGS never silently bundles or installs. Use `ags skill propose --action <verb> --skill <name>` for a dry-run, then `--apply` to confirm.".to_string(),
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
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVerifyResult {
    pub schema_version: String,
    pub host: String,
    pub supported: bool,
    /// "ok" | "degraded" | "incomplete" | "unsupported"
    pub status: String,
    pub checks: Vec<HostCheck>,
    pub summary: HostVerifySummary,
    pub note: String,
}

/// Verify host visibility for one host. Read-only. For reserved hosts
/// (codex/cursor) returns `supported: false`, `status: "unsupported"` with
/// stable fields and an empty check list.
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
    } else if degraded > 0 || expected_degraded > 0 {
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
        note: "Read-only host-visibility verify. status=incomplete means an expected capability is not visible. Restart the host after adopt/update so it re-scans entry points; use --strict to gate (exit nonzero unless status=ok).".to_string(),
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
        "applied" => note_lines.push("Applied. Restart the host (Claude Code / Codex / Cursor) so it re-scans thin indexes, then run `ags skill verify --host <host>`.".to_string()),
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
        ManagedStatus::Ignored => "ignored",
        ManagedStatus::Unmanaged => "unmanaged",
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

/// Plan per-host thin-index distribution. AGS keeps ONE canonical skill body;
/// each supported host gets a symlink (thin index) at `<host>/skills/<name>`
/// pointing back at that canonical directory, so reference files travel with it
/// and nothing is copied. `remove`/`uninstall` move only the thin index aside —
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
        // Containment: the symlink target must live inside an approved canonical
        // store. A bad/stale manifest source must not expose an arbitrary local
        // directory as a host-loadable skill body.
        if !canonical_within_store(&ctx.repo_root, &dir) {
            plan.blocked.push(format!(
                "Canonical source {} is outside the approved AGS stores (global-skills/, skill-packs/) — refusing to link a host to it.",
                dir.display()
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
                plan.writes.push(PlannedWrite {
                    op: "relink".to_string(),
                    path: entry_str.clone(),
                    from: Some(canonical.as_ref().unwrap().display().to_string()),
                    detail: format!(
                        "[{host}] thin index → canonical skill dir (transactional; existing entry backed up to .bak; references travel with it)"
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
                if thin_index_needs_repair(&entry, &cap.name) {
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
                plan.advised.push(AdvisedCommand {
                    command: format!("claude mcp add {name} -- <command> [args...]"),
                    reason: "AGS records MCP governance but never registers MCP servers in host config; run this in your host, then restart it.".to_string(),
                });
            }
        }
        ConsoleAction::Remove | ConsoleAction::Uninstall => {
            plan.advised.push(AdvisedCommand {
                command: format!("claude mcp remove {name}"),
                reason: "AGS never unregisters MCP servers from host config; run this in your host yourself.".to_string(),
            });
        }
        ConsoleAction::Verify => {}
    }
    plan.notes.push(
        "MCP / CLI-backed capabilities have no AGS-owned host file; AGS advises the host command but never runs it.".to_string(),
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
        backup: Option<PathBuf>,
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

/// Transactionally install a thin-index symlink at `entry` → `canonical`,
/// backing up any existing entry. On **any** failure the original entry is left
/// exactly as it was (rolled back) — never half-removed.
fn transactional_relink(
    entry: &Path,
    canonical: &Path,
) -> std::io::Result<(String, AppliedChange)> {
    let tmp = staging_path(entry);
    // 1. Stage the new symlink first. If this fails, nothing has moved.
    let _ = remove_host_entry(&tmp);
    make_symlink(canonical, &tmp)?;
    // 2. Back up any existing entry.
    let backup = if std::fs::symlink_metadata(entry).is_ok() {
        let bak = next_backup_path(entry);
        if let Err(e) = std::fs::rename(entry, &bak) {
            let _ = remove_host_entry(&tmp);
            return Err(e);
        }
        Some(bak)
    } else {
        None
    };
    // 3. Swap the staged link into place. On failure, roll the backup back.
    if let Err(e) = std::fs::rename(&tmp, entry) {
        if let Some(bak) = &backup {
            let _ = std::fs::rename(bak, entry);
        }
        let _ = remove_host_entry(&tmp);
        return Err(e);
    }
    let msg = match &backup {
        Some(bak) => format!(
            "relink {} -> {} (old entry → {})",
            entry.display(),
            canonical.display(),
            bak.display()
        ),
        None => format!("relink {} -> {}", entry.display(), canonical.display()),
    };
    Ok((
        msg,
        AppliedChange::Relink {
            entry: entry.to_path_buf(),
            backup,
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
        AppliedChange::Relink { entry, backup } => {
            remove_host_entry(entry)?;
            if let Some(bak) = backup {
                if bak.exists() {
                    std::fs::rename(bak, entry)?;
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

/// The single mutation gate. Returns which writes succeeded and which errored.
///
/// When `confirmed` is false it performs **no** filesystem writes. It first
/// PREFLIGHTS every planned write (containment + host skills dir creatable); if
/// any host fails preflight, NOTHING is mutated — a later host's failure can
/// never leave an earlier host half-changed. Each `relink`/`unlink` then runs
/// transactionally (stage → backup → atomic swap). The batch also keeps a
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock runner: returns canned `claude mcp list` / `codex mcp list` and
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
                 context7: npx -y @upstash/context7-mcp - ✔ Connected\n"
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
        let base = std::env::temp_dir().join(format!("ags-console-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
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
             \x20 - name: \"codegraph\"\n    package:\n      manager: \"external-cli\"\n    install:\n      installed_clients:\n        - \"codex\"\n",
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

    #[cfg(unix)]
    fn link_skill_entry(ctx: &ConsoleContext, host_subdir: &str, name: &str, source: &str) {
        let parent = ctx.home.join(host_subdir);
        std::fs::create_dir_all(&parent).unwrap();
        make_symlink(&ctx.repo_root.join(source), &parent.join(name)).unwrap();
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
        // P1.1 + thin index: BOTH hosts get a symlink (not a copy) into the
        // injected home, and SKILL.md is reachable THROUGH it (canonical body).
        for sub in [".claude/skills", ".codex/skills"] {
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
    fn propose_apply_moves_existing_entry_aside() {
        let (ctx, base) = ctx_with("applybackup", canned_list());
        // A pre-existing REAL dir entry on claude (e.g. a manual copy).
        let entry = ctx.home.join(".claude/skills/lark-shared");
        write_file(&entry.join("SKILL.md"), "OLD CONTENT");
        let res = propose_action(&ctx, ConsoleAction::Update, "lark-shared", true);
        assert!(res.applied);
        // The whole old entry is moved aside to <name>.bak (not copied file).
        let bak = ctx.home.join(".claude/skills/lark-shared.bak/SKILL.md");
        assert!(bak.exists(), "existing entry moved aside before relinking");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "OLD CONTENT");
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
                 \x20   - name: \"sneaky\"\n      version: \"1\"\n      source: \"{}\"\n      hash: h\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: r\n",
                evil.display()
            ),
        );
        let res = propose_action(&ctx, ConsoleAction::Adopt, "sneaky", true);
        assert!(res.found);
        assert!(
            res.blocked_reasons
                .iter()
                .any(|b| b.contains("outside the approved")),
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
}
