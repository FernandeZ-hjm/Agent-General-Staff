//! AGS Capability Route — deterministic, advisory demand → capability routing.
//!
//! Where Value Route (`prompt_request_classifier::derive_value_route`) answers
//! "what execution-path *form* covers this risk?", Capability Route answers
//! "which managed capability should the host be *advised to wake up* for this
//! demand, and is it reachable?". Both are advisory and deterministic; neither
//! changes the Light/Medium/Heavy task level, permission mode, Review gate,
//! Verification gate, or task-card gate. Only AGS gates can block — Capability
//! Route never does.
//!
//! # Boundaries (hard)
//!
//! - **Advise-only.** Output is a wakeup *suggestion* (`invoke_hint` strings).
//!   AGS NEVER auto-invokes a skill / MCP / CLI.
//! - **Manifest is the single source of truth.** Routing metadata comes only
//!   from the inventory (read from manifests). There is NO built-in fallback
//!   table; an unannotated capability is simply not routed.
//! - **Fail-closed is availability-only.** Conservative availability never
//!   blocks the user request — non-`routed` states yield a `fallback` hint.
//! - **`auth_status` is runtime-derived**, never read from a tracked manifest.
//! - **Enrollment is machine-local runtime evidence.** Whether a capability is
//!   routed for wakeup depends on `<runtime_home>/capability-route/enrollment.json`
//!   (mode `off`/`suite-only`/`adopted`/`review-all`), written by `ags setup` and
//!   NEVER stored in a tracked manifest. Missing / malformed evidence fail-closes
//!   to `off` (nothing enrolled → advisory degraded), and still never blocks.

use prompt_request_classifier::{classify_demand, DemandKind};
use serde::{Deserialize, Serialize};
use skill_governance::console::{
    build_inventory, ConsoleContext, CostClass, EntrypointRef, HealthStatus, HostVisibilityStatus,
    ManagedCapability, ManagedInventoryResult, ManagedKind, ManagedStatus, MutationSurface,
    ParentRef, RouteState, RoutingMetadata,
};
use std::path::{Path, PathBuf};

/// Fixed boundary statement carried on every Capability Route output.
pub const CAPABILITY_ROUTE_AUTHORITY_NOTE: &str = "Capability Route is an advisory wakeup suggestion. It does NOT auto-invoke any skill/MCP/CLI, does NOT block or override the user request, and does NOT change the Light/Medium/Heavy task level, permission mode, Review gate, Verification gate, or task-card gate. Only AGS gates can block. AGS judges, routes, and suggests explicit wakeups; the host/user owns the decision to invoke.";

/// Runtime auth posture of a capability, DERIVED at route time and never stored
/// in a tracked manifest. Task 2 has no enrollment / runtime auth registry, so a
/// `requires_auth` capability can only derive `RequiredUnknown` here;
/// `Configured` / `Failed` are reserved for a future runtime-evidence input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStatus {
    /// No external account / credential needed.
    NotRequired,
    /// Auth is required but no runtime evidence confirms it is configured.
    RequiredUnknown,
    /// Runtime evidence confirms the account is configured (reserved/future).
    Configured,
    /// Runtime evidence shows auth failed (reserved/future).
    Failed,
}

// ── Machine-local routing enrollment (runtime evidence, never a manifest) ─────
//
// Enrollment declares WHICH managed capabilities this machine opted into
// Capability Route. It is the routing-membership gate that sits in front of the
// availability axes (canonical / auth / host / health). Evidence lives ONLY in
// the AGS runtime home (`<runtime_home>/capability-route/enrollment.json`),
// written by `ags setup`; it is never stored in a tracked manifest and never
// carries real credentials. Fail-closed: missing / malformed evidence resolves
// to `Off` (nothing enrolled → advisory degraded), and still never blocks.

/// Schema version stamped into the machine-local enrollment evidence file.
pub const ENROLLMENT_SCHEMA: &str = "1.0-capability-route-enrollment";

/// Which managed capabilities this machine has enrolled into Capability Route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EnrollmentMode {
    /// No capability is routed for wakeup — every match degrades to advisory.
    #[default]
    Off,
    /// Only suite-managed capabilities (the suite skills) are enrolled.
    SuiteOnly,
    /// Suite-managed capabilities AND governed third-party MCPs are enrolled.
    Adopted,
    /// Every manifest-annotated capability is enrolled (broadest).
    ReviewAll,
}

impl EnrollmentMode {
    /// Stable kebab-case identifier (matches the CLI flag values).
    pub fn as_str(self) -> &'static str {
        match self {
            EnrollmentMode::Off => "off",
            EnrollmentMode::SuiteOnly => "suite-only",
            EnrollmentMode::Adopted => "adopted",
            EnrollmentMode::ReviewAll => "review-all",
        }
    }

    /// Parse a CLI / evidence-file value. Unknown values yield `None` so callers
    /// can fail-closed to `Off`.
    pub fn from_cli_str(s: &str) -> Option<Self> {
        match s.trim() {
            "off" => Some(EnrollmentMode::Off),
            "suite-only" => Some(EnrollmentMode::SuiteOnly),
            "adopted" => Some(EnrollmentMode::Adopted),
            "review-all" => Some(EnrollmentMode::ReviewAll),
            _ => None,
        }
    }

    /// One-line operator description of the mode's routing membership.
    pub fn description(self) -> &'static str {
        match self {
            EnrollmentMode::Off => {
                "no capability routed for wakeup; every match stays advisory degraded"
            }
            EnrollmentMode::SuiteOnly => "enroll suite-managed skills only",
            EnrollmentMode::Adopted => "enroll suite-managed skills AND governed third-party MCPs",
            EnrollmentMode::ReviewAll => "enroll every manifest-annotated capability (broadest)",
        }
    }

    /// All modes in canonical (narrow → broad) order, for plan rendering.
    pub fn all() -> [EnrollmentMode; 4] {
        [
            EnrollmentMode::Off,
            EnrollmentMode::SuiteOnly,
            EnrollmentMode::Adopted,
            EnrollmentMode::ReviewAll,
        ]
    }
}

/// Resolved machine-local enrollment evidence. `present` records whether a usable
/// evidence file was found and parsed (false ⇒ fail-closed `Off`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RuntimeEnrollment {
    pub mode: EnrollmentMode,
    pub present: bool,
}

impl RuntimeEnrollment {
    /// Back-compat default for the pure core / unit tests: everything enrolled.
    /// Production routing reads real evidence with [`read_enrollment`] instead.
    pub fn fully_enrolled() -> Self {
        Self {
            mode: EnrollmentMode::ReviewAll,
            present: true,
        }
    }

    /// Fail-closed "no usable evidence" — nothing enrolled.
    fn not_provisioned() -> Self {
        Self {
            mode: EnrollmentMode::Off,
            present: false,
        }
    }
}

/// Resolve the AGS runtime home that holds machine-local routing evidence.
/// Order: `AGS_RUNTIME_HOME` (exported by the MCP launcher) → `AGS_HOME` (the CLI
/// default) → `~/.ags/runtime`. This keeps the MCP and CLI surfaces
/// reading the same evidence in the real flow.
pub fn locate_runtime_home() -> PathBuf {
    if let Some(p) = std::env::var_os("AGS_RUNTIME_HOME") {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("AGS_HOME") {
        return PathBuf::from(p);
    }
    if let Some(home) = ags_platform::home_dir() {
        return home.join(".ags").join("runtime");
    }
    PathBuf::from(".ags").join("runtime")
}

/// The machine-local enrollment evidence file under a runtime home.
pub fn enrollment_file_path(runtime_home: &Path) -> PathBuf {
    runtime_home
        .join("capability-route")
        .join("enrollment.json")
}

/// Minimal deserialization target — we only consume `mode`. Any auth-evidence
/// placeholder in the file is ignored by routing (auth is runtime-derived).
#[derive(Deserialize)]
struct EnrollmentFileDoc {
    #[serde(default)]
    mode: Option<String>,
}

/// Read machine-local enrollment evidence from `runtime_home`. Fail-closed: a
/// missing / unreadable / malformed file, or an unknown mode, resolves to `Off`
/// with `present=false`. Routing degrades to advisory; it NEVER blocks and NEVER
/// reads credentials.
pub fn read_enrollment(runtime_home: &Path) -> RuntimeEnrollment {
    let path = enrollment_file_path(runtime_home);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return RuntimeEnrollment::not_provisioned();
    };
    let Ok(doc) = serde_json::from_str::<EnrollmentFileDoc>(&content) else {
        return RuntimeEnrollment::not_provisioned();
    };
    match doc.mode.as_deref().and_then(EnrollmentMode::from_cli_str) {
        Some(mode) => RuntimeEnrollment {
            mode,
            present: true,
        },
        None => RuntimeEnrollment::not_provisioned(),
    }
}

/// Render the canonical machine-local enrollment evidence document for `mode`.
/// It carries ONLY the mode + metadata + an empty auth-evidence placeholder. It
/// NEVER records real credentials and NEVER asserts `auth_status=configured`
/// (only `not-required` / `required-unknown` are ever permissible there, and
/// this task records none). Written by `ags setup` into the runtime home.
pub fn render_enrollment_json(mode: EnrollmentMode, generated_by: &str) -> String {
    let doc = serde_json::json!({
        "schema_version": ENROLLMENT_SCHEMA,
        "mode": mode.as_str(),
        "generated_by": generated_by,
        "auth_evidence": {
            "policy": "runtime-derived-only",
            "note": "Reserved for future runtime auth evidence. Only the values not-required / required-unknown may ever appear here; an account-present marker is never written by setup, and no account material is read.",
            "entries": {}
        }
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

/// Whether `cap` is enrolled into routing under `enrollment`. Fail-closed: with
/// no usable evidence (`present=false`) or mode `Off`, nothing is enrolled.
fn is_enrolled(cap: &ManagedCapability, enrollment: &RuntimeEnrollment) -> bool {
    if !enrollment.present {
        return false;
    }
    match enrollment.mode {
        EnrollmentMode::Off => false,
        EnrollmentMode::SuiteOnly => matches!(cap.managed_status, ManagedStatus::SuiteManaged),
        EnrollmentMode::Adopted => matches!(
            cap.managed_status,
            ManagedStatus::SuiteManaged | ManagedStatus::Governed
        ),
        EnrollmentMode::ReviewAll => true,
    }
}

/// Whether a routed capability is actually reachable for the active host.
/// Fail-closed: anything short of positive evidence is a restrictive state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityAvailability {
    /// Visible to the host, healthy, and auth satisfied.
    Available,
    /// Not enrolled into Capability Route on this machine (no runtime enrollment
    /// evidence, or the active mode excludes this capability). Fail-closed: AGS
    /// has no machine-local evidence that the operator opted this capability into
    /// routing, so it is never `Available`. Advisory only — never blocks.
    CapabilityNotEnrolled,
    /// Tagged for the demand but its canonical body is absent.
    CapabilityMiss,
    /// Canonical present but not visible / loadable for the active host.
    CapabilityUnavailable,
    /// Requires auth that is not confirmed configured.
    CapabilityAuthRequired,
    /// Runtime health is degraded / unhealthy.
    CapabilityUnhealthy,
}

impl CapabilityAvailability {
    /// Sort rank — `Available` first, the rest stable but de-prioritized.
    fn rank(self) -> u8 {
        match self {
            CapabilityAvailability::Available => 0,
            _ => 1,
        }
    }
}

/// Overall route status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityRouteStatus {
    /// At least one recommendation is `Available`.
    Routed,
    /// Recommendations exist but none are currently available.
    Degraded,
    /// No development demand detected (ordinary prose).
    NoDemandDetected,
    /// Demand detected but the inventory has no capability tagged for it.
    NoCapabilityForDemand,
}

/// One routed capability and why / whether it can be used.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRecommendation {
    pub capability_name: String,
    pub capability_kind: ManagedKind,
    pub route_priority: i32,
    pub match_reason: String,
    /// Explicit wakeup hint — a suggestion string, NEVER auto-executed.
    pub invoke_hint: String,
    pub availability: CapabilityAvailability,
    pub auth_status: AuthStatus,
    pub mutation_surface: MutationSurface,
    /// Advisory, display-only action hint (read-only / confirm / auth). NEVER
    /// consumed by any gate or policy — see [`RouteAction`].
    pub route_action: RouteAction,
    pub cost_class: CostClass,
    pub is_compatibility_alias: bool,
    /// When this recommendation is an internal-entrypoint route target, the real
    /// host-visible parent capability it derefs to (skill / mcp / cli-backed).
    /// `primary` resolves to this parent, NEVER to the route target itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    /// The specific internal entrypoint (playbook / tool / subcommand) this route
    /// target points at — display metadata; the host invokes the parent body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<EntrypointRef>,
}

/// The deterministic, advisory capability-routing recommendation. Carries no
/// task-level / permission / gate field by construction — it cannot change any
/// authority.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRoute {
    pub demand_kind: DemandKind,
    pub matched_demand_triggers: Vec<String>,
    /// The host this route was computed for (echoed for audit).
    pub active_host: String,
    pub recommendations: Vec<CapabilityRecommendation>,
    pub primary: Option<String>,
    /// When `primary` resolved through an internal-entrypoint route target, the
    /// entrypoint it points at (e.g. the `get-library-docs` tool of `context7`,
    /// or the `verification-before-completion` playbook of `superpowers`).
    /// `None` when the primary is a plain capability with no internal entrypoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<EntrypointRef>,
    pub status: CapabilityRouteStatus,
    pub fallback: String,
    /// Machine-local enrollment posture this route was computed under (echoed for
    /// audit / operator transparency). `present=false` ⇒ fail-closed `off`.
    pub enrollment: RuntimeEnrollment,
    /// Always `true` — Capability Route is advisory, never an authority.
    pub advisory: bool,
    pub authority_note: String,
}

/// Derive the runtime auth status from the stable manifest facts. Task 2 has no
/// runtime auth registry, so `requires_auth` can only derive `RequiredUnknown`;
/// `Configured` / `Failed` are reserved for a future runtime-evidence input.
fn derive_auth_status(meta: &RoutingMetadata) -> AuthStatus {
    if meta.requires_auth {
        AuthStatus::RequiredUnknown
    } else {
        AuthStatus::NotRequired
    }
}

/// Advisory, DISPLAY-ONLY action hint carried on a recommendation, derived from
/// the member's stable mutation / auth facts. HARD INVARIANT: this is a wakeup
/// *presentation* hint exactly like `invoke_hint` — NOTHING in AGS (no gate, no
/// policy resolver, no CLI/MCP control-flow path) may branch on it to decide
/// whether to proceed, and it NEVER blocks, delays, or withholds a route. It is
/// orthogonal to `availability`: reachability is computed independently in
/// `derive_availability`, which is the only signal that gates a `primary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteAction {
    /// No external side-effect or auth precondition (read-only, or local-only
    /// writes) — safe to suggest an explicit wakeup.
    InvokeReadonly,
    /// External-write capability — suggest the host CONFIRM before invoking.
    /// Advisory only: it asks the host to confirm, it does not itself block.
    ConfirmBeforeInvoke,
    /// Requires external auth that is not confirmed — surface the prerequisite so
    /// the host satisfies it first. Advisory only.
    AuthPrerequisite,
}

/// Derive the advisory `route_action` from stable facts. Auth gating takes
/// precedence (an unconfirmed credential is the first thing the host must
/// resolve); otherwise external writes ask for confirmation, and everything else
/// is a plain explicit-wakeup suggestion. Display-only — see [`RouteAction`].
fn derive_route_action(meta: &RoutingMetadata, auth: AuthStatus) -> RouteAction {
    if matches!(auth, AuthStatus::RequiredUnknown | AuthStatus::Failed) {
        return RouteAction::AuthPrerequisite;
    }
    match meta.mutation_surface {
        MutationSurface::ExternalWrite => RouteAction::ConfirmBeforeInvoke,
        MutationSurface::ReadOnly | MutationSurface::LocalWrite => RouteAction::InvokeReadonly,
    }
}

/// The active host's visibility status for a capability. `None` ⇒ the host was
/// not probed (host-agnostic mode, or host not in the inventory) — treated as
/// "no positive evidence" by the fail-closed availability rule.
fn host_status(cap: &ManagedCapability, active_host: &str) -> Option<HostVisibilityStatus> {
    if active_host.is_empty() {
        return None;
    }
    cap.host_visibility
        .iter()
        .find(|v| v.host == active_host)
        .map(|v| v.status.clone())
}

/// Fail-closed availability. Restrictive reasons take precedence (first wins):
/// not-enrolled → miss → auth-required → unavailable → unhealthy → available.
/// `Available` requires POSITIVE evidence on every axis: enrolled into routing on
/// this machine, canonical present, auth satisfied, host visibility `Visible` on
/// the ACTIVE host, and health `Healthy`. Anything else (not enrolled,
/// host-agnostic mode, not visible, or non-`Healthy` health) is restrictive —
/// never `Available`.
///
/// The enrollment check runs FIRST: it is the routing-membership gate. Without
/// machine-local evidence that the operator opted this capability into routing,
/// AGS must not claim reachability regardless of canonical/host/health state.
///
/// The active-host visibility check runs BEFORE the health check on purpose:
/// `cap.health_status` is a cross-host AGGREGATE (`derive_health` marks a skill
/// `Healthy`/`Degraded` if ANY probed host is), so a capability that is plainly
/// `NotVisible` on the active host must read as `CapabilityUnavailable` even
/// when some other probed host left the aggregate health `Degraded` — otherwise
/// the operator-facing reason would point at "health" instead of the real
/// active-host visibility gap.
fn derive_availability(
    cap: &ManagedCapability,
    auth: AuthStatus,
    active_host: &str,
    enrollment: &RuntimeEnrollment,
) -> CapabilityAvailability {
    if !is_enrolled(cap, enrollment) {
        return CapabilityAvailability::CapabilityNotEnrolled;
    }
    if !cap.canonical_present {
        return CapabilityAvailability::CapabilityMiss;
    }
    if matches!(auth, AuthStatus::RequiredUnknown | AuthStatus::Failed) {
        return CapabilityAvailability::CapabilityAuthRequired;
    }
    // Active-host visibility first — no positive `Visible` evidence (host-agnostic
    // or not visible here) is a fail-closed `CapabilityUnavailable`, regardless of
    // the cross-host aggregate health.
    if !matches!(
        host_status(cap, active_host),
        Some(HostVisibilityStatus::Visible)
    ) {
        return CapabilityAvailability::CapabilityUnavailable;
    }
    // Visible on the active host — refine by health. Only confirmed `Healthy` is
    // `Available`; anything else (Unhealthy / Degraded / Unknown) is restrictive.
    if cap.health_status == HealthStatus::Healthy {
        CapabilityAvailability::Available
    } else {
        CapabilityAvailability::CapabilityUnhealthy
    }
}

/// Scope filter. A capability with no scope tags, or a `*` tag, is scope-agnostic
/// and always passes. Otherwise at least one declared scope must appear in the
/// (lowercased) request text — a deterministic, text-only narrowing.
fn scope_matches(scope_tags: &[String], text_lower: &str) -> bool {
    if scope_tags.is_empty() {
        return true;
    }
    scope_tags
        .iter()
        .any(|s| s == "*" || text_lower.contains(&s.to_lowercase()))
}

/// Cost ordering rank — cheaper first.
fn cost_rank(c: CostClass) -> u8 {
    match c {
        CostClass::Free => 0,
        CostClass::Local => 1,
        CostClass::Network => 2,
        CostClass::Paid => 3,
    }
}

/// Back-compat pure core: derive the advisory Capability Route treating every
/// matched capability as fully enrolled (`review-all`). Production entry points
/// use [`route_request`] / [`derive_capability_route_enrolled`] with the real
/// machine-local enrollment evidence instead. Deterministic.
pub fn derive_capability_route(
    text: &str,
    inventory: &ManagedInventoryResult,
    active_host: &str,
) -> CapabilityRoute {
    derive_capability_route_enrolled(
        text,
        inventory,
        active_host,
        &RuntimeEnrollment::fully_enrolled(),
    )
}

/// Derive the advisory Capability Route for `text` against `inventory`, for
/// `active_host` (empty string ⇒ host-agnostic, fail-closed conservative), under
/// the machine-local `enrollment`. Deterministic: same inputs → same route.
/// Manifest is the sole metadata authority; an unannotated capability
/// (`routing == None`) is never routed. A capability that is not enrolled (or for
/// which there is no usable enrollment evidence) reads as `CapabilityNotEnrolled`
/// — advisory degraded, never blocked.
pub fn derive_capability_route_enrolled(
    text: &str,
    inventory: &ManagedInventoryResult,
    active_host: &str,
    enrollment: &RuntimeEnrollment,
) -> CapabilityRoute {
    let demand = classify_demand(text);
    let host_label = if active_host.is_empty() {
        "host-agnostic".to_string()
    } else {
        active_host.to_string()
    };
    let note = CAPABILITY_ROUTE_AUTHORITY_NOTE.to_string();

    if demand.kind == DemandKind::None {
        return CapabilityRoute {
            demand_kind: demand.kind,
            matched_demand_triggers: demand.matched_triggers,
            active_host: host_label,
            recommendations: Vec::new(),
            primary: None,
            entrypoint: None,
            status: CapabilityRouteStatus::NoDemandDetected,
            fallback: "No development demand detected — an ordinary answer is appropriate; no capability wakeup is suggested.".to_string(),
            enrollment: *enrollment,
            advisory: true,
            authority_note: note,
        };
    }

    let demand_tag = demand.kind.as_str();
    let text_lower = text.to_lowercase();

    let mut recs: Vec<CapabilityRecommendation> = Vec::new();
    for cap in &inventory.capabilities {
        // Manifest single authority: no routing metadata ⇒ not routable.
        let Some(meta) = &cap.routing else {
            continue;
        };
        // route_state gate: only explicitly-routable members participate. The
        // fail-closed default (not-routable) and retired members never route.
        if meta.route_state != RouteState::Routable {
            continue;
        }
        let Some(matched_tag) = meta.intent_tags.iter().find(|t| t.as_str() == demand_tag) else {
            continue;
        };
        if !scope_matches(&meta.scope_tags, &text_lower) {
            continue;
        }
        let auth = derive_auth_status(meta);
        // Internal-entrypoint route targets (routing.parent set) inherit their
        // reachability from the real parent body — the host invokes the parent,
        // the entrypoint is only a method label. A plain capability uses its own.
        let availability = if let Some(parent) = &meta.parent {
            let parent_avail = inventory
                .capabilities
                .iter()
                .find(|c| c.name == parent.name && c.kind == parent.kind)
                .map(|p| {
                    let p_auth = p
                        .routing
                        .as_ref()
                        .map(derive_auth_status)
                        .unwrap_or(AuthStatus::NotRequired);
                    derive_availability(p, p_auth, active_host, enrollment)
                })
                // Parent body missing from the inventory → fail-closed miss.
                .unwrap_or(CapabilityAvailability::CapabilityMiss);
            // The entrypoint inherits the parent's REACHABILITY (enrollment /
            // canonical / host visibility / health), but its OWN auth requirement
            // still gates it: a reachable parent does NOT satisfy the entrypoint's
            // credential. Fail-closed — an auth-gated entrypoint over an auth-free
            // parent must never read `Available` / become `primary`.
            match parent_avail {
                CapabilityAvailability::Available
                    if matches!(auth, AuthStatus::RequiredUnknown | AuthStatus::Failed) =>
                {
                    CapabilityAvailability::CapabilityAuthRequired
                }
                other => other,
            }
        } else {
            derive_availability(cap, auth, active_host, enrollment)
        };
        recs.push(CapabilityRecommendation {
            capability_name: cap.name.clone(),
            capability_kind: cap.kind.clone(),
            route_priority: meta.route_priority,
            match_reason: format!("intent_tag '{matched_tag}' matches demand '{demand_tag}'"),
            invoke_hint: meta.invoke_hint.clone(),
            availability,
            auth_status: auth,
            mutation_surface: meta.mutation_surface,
            route_action: derive_route_action(meta, auth),
            cost_class: meta.cost_class,
            is_compatibility_alias: meta.is_compatibility_alias,
            parent: meta.parent.clone(),
            entrypoint: meta.entrypoint.clone(),
        });
    }

    // Deterministic order: available first; then lower route_priority, then
    // cheaper cost, then name (stable tie-break). The auto-* compatibility
    // aliases are retired (route_state: retired → excluded from routing), so
    // there is NO alias-wins tiebreak: a demand's primary is the routable
    // canonical successor with the lowest route_priority (debug → diagnosing-bugs,
    // brainstorm → grill-with-docs, verify → verification-before-completion).
    // `is_compatibility_alias` is retained as an audit/display field but is no
    // longer a sort key.
    recs.sort_by(|a, b| {
        a.availability
            .rank()
            .cmp(&b.availability.rank())
            .then(a.route_priority.cmp(&b.route_priority))
            .then(cost_rank(a.cost_class).cmp(&cost_rank(b.cost_class)))
            .then(a.capability_name.cmp(&b.capability_name))
    });

    // Primary = the first Available recommendation, derefed to its real
    // host-visible parent when it is an internal-entrypoint route target. The
    // route target itself is NEVER the primary; `entrypoint` carries the method.
    let primary_rec = recs
        .iter()
        .find(|r| r.availability == CapabilityAvailability::Available);
    let primary = primary_rec.map(|r| {
        r.parent
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| r.capability_name.clone())
    });
    let entrypoint = primary_rec.and_then(|r| r.entrypoint.clone());

    let (status, fallback) = if recs.is_empty() {
        (
            CapabilityRouteStatus::NoCapabilityForDemand,
            format!(
                "Demand '{demand_tag}' detected but no manifest-annotated capability serves it; AGS has nothing to suggest. Annotate a capability's routing metadata in the manifest to enable routing."
            ),
        )
    } else if primary.is_some() {
        (CapabilityRouteStatus::Routed, String::new())
    } else {
        // Degraded: distinguish "nothing enrolled on this machine" (the operator
        // has not opted these capabilities into routing) from "enrolled but not
        // currently reachable" (visibility / health / auth). Both stay advisory;
        // neither blocks.
        let any_enrolled = recs
            .iter()
            .any(|r| r.availability != CapabilityAvailability::CapabilityNotEnrolled);
        let fallback = if !any_enrolled {
            format!(
                "Demand '{demand_tag}' detected and {} capability(ies) match, but none are enrolled into Capability Route on this machine (mode={}, evidence={}). Run `ags setup --capability-route <suite-only|adopted|review-all> --yes` to enroll, or invoke the capability manually; AGS does not block.",
                recs.len(),
                enrollment.mode.as_str(),
                if enrollment.present { "present" } else { "absent" },
            )
        } else {
            let host_hint = if active_host.is_empty() {
                "<host>".to_string()
            } else {
                active_host.to_string()
            };
            format!(
                "Demand '{demand_tag}' detected and {} capability(ies) match, but none are currently available for host '{host_label}'. Restore visibility/health (e.g. `ags skill verify --host {host_hint}`) or invoke manually; AGS does not block.",
                recs.len()
            )
        };
        (CapabilityRouteStatus::Degraded, fallback)
    };

    CapabilityRoute {
        demand_kind: demand.kind,
        matched_demand_triggers: demand.matched_triggers,
        active_host: host_label,
        recommendations: recs,
        primary,
        entrypoint,
        status,
        fallback,
        enrollment: *enrollment,
        advisory: true,
        authority_note: note,
    }
}

// ── Manifest-rooted convenience wiring ──────────────────────────────────────
//
// `derive_capability_route` is the pure core (text + inventory → route). The two
// helpers below are the SHARED wiring used by every production entry point — the
// MCP `ags_solution_check` tool and the CLI `gate prompt-request` /
// `gate capability-request` commands — so all of them locate the manifest root
// the same way and build the inventory with the same host-default set. Keeping
// this in one place means a routing-source change can never drift between the
// MCP and CLI surfaces.

/// Locate the manifest root for capability routing: the nearest ancestor of
/// `start` (inclusive) that contains BOTH `manifests/skills-registry.yaml` and
/// `manifests/mcp-registry.yaml`. Falls back to `start` when none is found, so a
/// caller invoked from a subdirectory still resolves to the repository root and
/// never spuriously reports `no-capability-for-demand`.
///
/// Pure path logic — no canonicalization. Callers that need a normalized start
/// (e.g. the CLI's `guard_path`) should normalize the path before calling.
pub fn locate_manifest_root(start: &Path) -> PathBuf {
    for candidate in start.ancestors() {
        if candidate.join("manifests/skills-registry.yaml").is_file()
            && candidate.join("manifests/mcp-registry.yaml").is_file()
        {
            return candidate.to_path_buf();
        }
    }
    start.to_path_buf()
}

/// Build the managed-capability inventory rooted at `manifest_root` and derive the
/// advisory Capability Route for `request` against `active_host`. This is the one
/// shared wiring path for the MCP and CLI entry points, so they read the same
/// manifest source of truth and apply the same host-default set.
///
/// `active_host` empty ⇒ host-agnostic (conservative, fail-closed): the inventory
/// is still probed for the default host set so visibility is known, but
/// `derive_capability_route` yields no positive `Available` evidence. A non-empty
/// `active_host` probes and routes for exactly that host.
///
/// Reads the machine-local enrollment evidence from the resolved runtime home
/// ([`locate_runtime_home`]) and routes under it, so non-enrolled capabilities
/// fail-closed to advisory degraded. Use [`route_request_with_runtime_home`] to
/// pin an explicit runtime home (hermetic tests).
///
/// Advisory-only by construction: the returned [`CapabilityRoute`] carries no
/// task-level / permission / gate field and can never block or change any AGS
/// gate.
pub fn route_request(request: &str, manifest_root: &Path, active_host: &str) -> CapabilityRoute {
    route_request_with_runtime_home(request, manifest_root, active_host, &locate_runtime_home())
}

/// Same as [`route_request`] but reads enrollment evidence from an explicit
/// `runtime_home` instead of the resolved default. The one shared wiring path
/// (used by the MCP `ags_solution_check` tool and the CLI `gate prompt-request` /
/// `gate capability-request` commands), so the MCP and CLI surfaces read the same
/// manifest source of truth, host-default set, and enrollment evidence.
pub fn route_request_with_runtime_home(
    request: &str,
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> CapabilityRoute {
    let ctx = ConsoleContext::system(manifest_root.to_path_buf());
    let hosts: Vec<&str> = if active_host.is_empty() {
        vec!["claude-code", "codex"]
    } else {
        vec![active_host]
    };
    let inventory = build_inventory(&ctx, &hosts);
    let enrollment = read_enrollment(runtime_home);
    derive_capability_route_enrolled(request, &inventory, active_host, &enrollment)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use skill_governance::console::EntrypointKind;
    use skill_governance::console::{
        HostVisibility, ManagedInventorySummary, ManagedStatus, RegistryStatus,
    };

    fn summary() -> ManagedInventorySummary {
        ManagedInventorySummary {
            total: 0,
            skills: 0,
            mcps: 0,
            suite_interfaces: 0,
            cli_backed: 0,
            canonical_present: 0,
            claude_visible: 0,
            risk_flagged: 0,
            routing_routable: 0,
            routing_not_routable: 0,
            routing_retired: 0,
            routing_uncovered: 0,
        }
    }

    fn inv(caps: Vec<ManagedCapability>) -> ManagedInventoryResult {
        ManagedInventoryResult {
            schema_version: "test".to_string(),
            hosts: vec!["claude-code".to_string()],
            capabilities: caps,
            summary: summary(),
            note: String::new(),
            routing_parse_failures: Vec::new(),
        }
    }

    /// Build a capability with routing metadata and one host-visibility entry.
    #[allow(clippy::too_many_arguments)]
    fn cap(
        name: &str,
        kind: ManagedKind,
        canonical: bool,
        host: &str,
        vis: HostVisibilityStatus,
        health: HealthStatus,
        routing: RoutingMetadata,
    ) -> ManagedCapability {
        ManagedCapability {
            kind,
            name: name.to_string(),
            source: None,
            profile: None,
            managed_status: ManagedStatus::SuiteManaged,
            registry_status: RegistryStatus::Registered,
            canonical_present: canonical,
            expected_hosts: vec![host.to_string()],
            host_visibility: vec![HostVisibility {
                host: host.to_string(),
                supported: true,
                status: vis,
                evidence: Vec::new(),
            }],
            health_status: health,
            actions: Vec::new(),
            risk_notes: Vec::new(),
            routing: Some(routing),
        }
    }

    fn routing(intent: &[&str], alias: bool, priority: i32) -> RoutingMetadata {
        RoutingMetadata {
            intent_tags: intent.iter().map(|s| s.to_string()).collect(),
            scope_tags: vec!["*".to_string()],
            invoke_hint: format!("[skill: {}]", intent.first().copied().unwrap_or("x")),
            route_priority: priority,
            is_compatibility_alias: alias,
            route_state: RouteState::Routable,
            ..Default::default()
        }
    }

    fn healthy_skill(name: &str, routing: RoutingMetadata) -> ManagedCapability {
        cap(
            name,
            ManagedKind::Skill,
            true,
            "claude-code",
            HostVisibilityStatus::Visible,
            HealthStatus::Healthy,
            routing,
        )
    }

    /// A healthy, visible governed third-party MCP (managed_status = Governed).
    fn governed_mcp(name: &str, routing: RoutingMetadata) -> ManagedCapability {
        let mut c = cap(
            name,
            ManagedKind::Mcp,
            true,
            "claude-code",
            HostVisibilityStatus::Visible,
            HealthStatus::Healthy,
            routing,
        );
        c.managed_status = ManagedStatus::Governed;
        c
    }

    #[test]
    fn no_demand_yields_no_demand_detected() {
        let r = derive_capability_route("解释这段代码", &inv(vec![]), "claude-code");
        assert_eq!(r.demand_kind, DemandKind::None);
        assert_eq!(r.status, CapabilityRouteStatus::NoDemandDetected);
        assert!(r.recommendations.is_empty());
        assert!(r.primary.is_none());
        assert!(r.advisory);
    }

    #[test]
    fn routes_primary_by_priority_no_alias_tiebreak() {
        // No alias-wins tiebreak (auto-* retired): the routable canonical
        // successor with the lowest route_priority is primary. diagnosing-bugs (50)
        // beats the secondary systematic-debugging (70) for the debug demand.
        let inventory = inv(vec![
            healthy_skill("systematic-debugging", routing(&["debug"], false, 70)),
            healthy_skill(
                "diagnosing-bugs",
                routing(&["debug", "root-cause"], false, 50),
            ),
        ]);
        let r = derive_capability_route("测试挂了，帮我看下", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::Debug);
        assert_eq!(r.status, CapabilityRouteStatus::Routed);
        assert_eq!(r.primary.as_deref(), Some("diagnosing-bugs"));
        assert_eq!(r.recommendations[0].capability_name, "diagnosing-bugs");
        assert_eq!(r.recommendations[1].capability_name, "systematic-debugging");
    }

    #[test]
    fn unannotated_capability_is_not_routed() {
        // A capability with no routing metadata is invisible to production
        // routing even if its name suggests it serves the demand.
        let mut c = healthy_skill("auto-debug", routing(&["debug"], true, 10));
        c.routing = None;
        let r = derive_capability_route("报错了", &inv(vec![c]), "claude-code");
        assert_eq!(r.status, CapabilityRouteStatus::NoCapabilityForDemand);
        assert!(r.recommendations.is_empty());
        assert!(!r.fallback.is_empty());
    }

    #[test]
    fn auth_required_is_never_available() {
        let mut meta = routing(&["docs-lookup"], false, 20);
        meta.requires_auth = true;
        let inventory = inv(vec![healthy_skill("context7", meta)]);
        let r = derive_capability_route("查一下 React useEffect 文档", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::DocsLookup);
        let rec = &r.recommendations[0];
        assert_eq!(rec.auth_status, AuthStatus::RequiredUnknown);
        assert_eq!(
            rec.availability,
            CapabilityAvailability::CapabilityAuthRequired
        );
        assert!(r.primary.is_none());
        assert_eq!(r.status, CapabilityRouteStatus::Degraded);
    }

    #[test]
    fn fail_closed_availability_states() {
        // miss: canonical absent
        let miss = cap(
            "verify-x",
            ManagedKind::Skill,
            false,
            "claude-code",
            HostVisibilityStatus::Visible,
            HealthStatus::Healthy,
            routing(&["verify"], false, 30),
        );
        let r = derive_capability_route("验证一下", &inv(vec![miss]), "claude-code");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::CapabilityMiss
        );

        // not visible → unavailable
        let nv = cap(
            "verify-y",
            ManagedKind::Skill,
            true,
            "claude-code",
            HostVisibilityStatus::NotVisible,
            HealthStatus::Unknown,
            routing(&["verify"], false, 30),
        );
        let r = derive_capability_route("验证一下", &inv(vec![nv]), "claude-code");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::CapabilityUnavailable
        );

        // unhealthy → unhealthy
        let uh = cap(
            "verify-z",
            ManagedKind::Mcp,
            true,
            "claude-code",
            HostVisibilityStatus::Visible,
            HealthStatus::Unhealthy,
            routing(&["verify"], false, 30),
        );
        let r = derive_capability_route("验证一下", &inv(vec![uh]), "claude-code");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::CapabilityUnhealthy
        );

        // visible + healthy + not-required → available
        let ok = healthy_skill("auto-verify", routing(&["verify"], true, 10));
        let r = derive_capability_route("验证一下", &inv(vec![ok]), "claude-code");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::Available
        );
        assert_eq!(r.recommendations[0].auth_status, AuthStatus::NotRequired);
    }

    #[test]
    fn host_agnostic_is_conservative_explicit_host_resolves() {
        let mk = || {
            inv(vec![healthy_skill(
                "auto-debug",
                routing(&["debug"], true, 10),
            )])
        };
        // host-agnostic: no visibility evidence → never available
        let r = derive_capability_route("测试挂了", &mk(), "");
        assert_eq!(r.active_host, "host-agnostic");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::CapabilityUnavailable
        );
        assert!(r.primary.is_none());
        assert_eq!(r.status, CapabilityRouteStatus::Degraded);
        // explicit host with visibility → available
        let r2 = derive_capability_route("测试挂了", &mk(), "claude-code");
        assert_eq!(r2.active_host, "claude-code");
        assert_eq!(r2.primary.as_deref(), Some("auto-debug"));
    }

    #[test]
    fn fail_closed_is_not_a_block() {
        // A degraded route still carries advisory=true and a fallback hint —
        // it never blocks. (Structurally there is no gate/level field to set.)
        let nv = cap(
            "auto-debug",
            ManagedKind::Skill,
            true,
            "claude-code",
            HostVisibilityStatus::NotVisible,
            HealthStatus::Unknown,
            routing(&["debug"], true, 10),
        );
        let r = derive_capability_route("报错了", &inv(vec![nv]), "claude-code");
        assert_eq!(r.status, CapabilityRouteStatus::Degraded);
        assert!(r.advisory);
        assert!(!r.fallback.is_empty());
    }

    #[test]
    fn json_shape_is_stable() {
        let inventory = inv(vec![healthy_skill(
            "auto-debug",
            routing(&["debug"], true, 10),
        )]);
        let r = derive_capability_route("测试挂了", &inventory, "claude-code");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["demand_kind"], "debug");
        assert_eq!(v["active_host"], "claude-code");
        assert_eq!(v["advisory"], true);
        assert_eq!(v["status"], "routed");
        assert!(v["authority_note"].is_string());
        assert!(v["recommendations"].is_array());
        let rec = &v["recommendations"][0];
        assert_eq!(rec["capability_name"], "auto-debug");
        assert_eq!(rec["availability"], "available");
        assert_eq!(rec["auth_status"], "not-required");
        assert_eq!(rec["capability_kind"], "skill");
        assert!(rec["invoke_hint"].is_string());
        assert_eq!(rec["mutation_surface"], "read-only");
        assert_eq!(rec["route_action"], "invoke-readonly");
        assert_eq!(rec["cost_class"], "free");
        assert_eq!(rec["is_compatibility_alias"], true);
        // Enrollment posture is echoed (back-compat core = fully enrolled).
        assert_eq!(v["enrollment"]["mode"], "review-all");
        assert_eq!(v["enrollment"]["present"], true);
    }

    #[test]
    fn value_route_and_capability_route_are_orthogonal() {
        // Same text feeds both signals independently — neither depends on the
        // other, and both come out advisory.
        let text = "测试挂了，帮我看下";
        let classification = prompt_request_classifier::classify(text);
        let vr = prompt_request_classifier::derive_value_route(&classification, false, false);
        assert!(vr.advisory);
        let inventory = inv(vec![healthy_skill(
            "auto-debug",
            routing(&["debug"], true, 10),
        )]);
        let cr = derive_capability_route(text, &inventory, "claude-code");
        assert!(cr.advisory);
        assert_eq!(cr.demand_kind, DemandKind::Debug);
    }

    /// The active-host visibility reason wins over the cross-host aggregate
    /// health: a capability NotVisible on the active host reads as
    /// `CapabilityUnavailable` even when (aggregate) health is `Degraded`.
    #[test]
    fn active_host_visibility_reason_beats_aggregate_health() {
        // health_status here simulates a cross-host aggregate left Degraded by
        // some OTHER probed host, while the active host (claude-code) is NotVisible.
        let c = cap(
            "auto-debug",
            ManagedKind::Skill,
            true,
            "claude-code",
            HostVisibilityStatus::NotVisible,
            HealthStatus::Degraded,
            routing(&["debug"], true, 10),
        );
        let r = derive_capability_route("报错了", &inv(vec![c]), "claude-code");
        assert_eq!(
            r.recommendations[0].availability,
            CapabilityAvailability::CapabilityUnavailable,
            "active-host NotVisible must read as unavailable, not unhealthy"
        );
    }

    /// `is_compatibility_alias` is NOT a sort key: route_priority decides among
    /// equal-availability members regardless of the alias flag. Guards against
    /// re-introducing an alias-wins tiebreak now that the auto-* aliases are
    /// retired.
    #[test]
    fn priority_decides_not_alias_flag() {
        let inventory = inv(vec![
            // A flagged alias with a WORSE (higher) priority must NOT win.
            healthy_skill("legacy-flagged", routing(&["debug"], true, 99)),
            healthy_skill("diagnosing-bugs", routing(&["debug"], false, 10)),
        ]);
        let r = derive_capability_route("测试挂了", &inventory, "claude-code");
        assert_eq!(r.primary.as_deref(), Some("diagnosing-bugs"));
        assert_eq!(r.recommendations[0].capability_name, "diagnosing-bugs");
        assert_eq!(r.recommendations[1].capability_name, "legacy-flagged");
    }

    /// Cost class is a deterministic tie-break: equal availability, equal
    /// priority, non-alias → cheaper cost_class wins.
    #[test]
    fn cost_class_breaks_ties() {
        let cheap = RoutingMetadata {
            intent_tags: vec!["verify".to_string()],
            scope_tags: vec!["*".to_string()],
            cost_class: CostClass::Free,
            route_priority: 30,
            invoke_hint: "[skill: cheap]".to_string(),
            route_state: RouteState::Routable,
            ..Default::default()
        };
        let pricey = RoutingMetadata {
            intent_tags: vec!["verify".to_string()],
            scope_tags: vec!["*".to_string()],
            cost_class: CostClass::Network,
            route_priority: 30,
            invoke_hint: "net".to_string(),
            route_state: RouteState::Routable,
            ..Default::default()
        };
        let inventory = inv(vec![
            healthy_skill("net-verify", pricey),
            healthy_skill("cheap-verify", cheap),
        ]);
        let r = derive_capability_route("验证一下", &inventory, "claude-code");
        assert_eq!(r.recommendations[0].capability_name, "cheap-verify");
        assert_eq!(r.recommendations[1].capability_name, "net-verify");
    }

    /// route_state gate: only `routable` members enter routing; `not-routable`
    /// (incl. the fail-closed default) and `retired` are excluded entirely.
    #[test]
    fn not_routable_and_retired_are_excluded_from_routing() {
        let mut not_routable = healthy_skill("ags-skill-ops", routing(&["debug"], false, 20));
        not_routable.routing.as_mut().unwrap().route_state = RouteState::NotRoutable;
        let mut retired = healthy_skill("old-debugger", routing(&["debug"], false, 20));
        retired.routing.as_mut().unwrap().route_state = RouteState::Retired;
        let routable = healthy_skill("diagnosing-bugs", routing(&["debug"], false, 50));
        let r = derive_capability_route(
            "测试挂了",
            &inv(vec![not_routable, retired, routable]),
            "claude-code",
        );
        let names: Vec<&str> = r
            .recommendations
            .iter()
            .map(|x| x.capability_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["diagnosing-bugs"],
            "only the explicitly-routable member routes; not-routable/retired excluded"
        );
    }

    /// route_action maps stable mutation/auth facts: read-only → invoke-readonly,
    /// external-write → confirm-before-invoke, requires_auth → auth-prerequisite
    /// (auth precedence). Display-only; never gates the route.
    #[test]
    fn route_action_maps_mutation_and_auth() {
        let ro = derive_capability_route(
            "验证一下",
            &inv(vec![healthy_skill(
                "auto-verify",
                routing(&["verify"], true, 10),
            )]),
            "claude-code",
        );
        assert_eq!(
            ro.recommendations[0].route_action,
            RouteAction::InvokeReadonly
        );

        let mut ew = routing(&["verify"], false, 20);
        ew.mutation_surface = MutationSurface::ExternalWrite;
        let r_ew = derive_capability_route(
            "验证一下",
            &inv(vec![healthy_skill("lark-writer", ew)]),
            "claude-code",
        );
        assert_eq!(
            r_ew.recommendations[0].route_action,
            RouteAction::ConfirmBeforeInvoke
        );

        let mut auth = routing(&["verify"], false, 20);
        auth.mutation_surface = MutationSurface::ExternalWrite;
        auth.requires_auth = true;
        let r_auth = derive_capability_route(
            "验证一下",
            &inv(vec![healthy_skill("lark-auth", auth)]),
            "claude-code",
        );
        assert_eq!(
            r_auth.recommendations[0].route_action,
            RouteAction::AuthPrerequisite
        );
    }

    // ── Enrollment gating (machine-local runtime evidence) ───────────────────

    /// One SuiteManaged alias + one Governed MCP, both matching the debug demand.
    fn suite_and_governed() -> ManagedInventoryResult {
        inv(vec![
            healthy_skill("auto-debug", routing(&["debug"], true, 10)),
            governed_mcp("context7", routing(&["debug"], false, 30)),
        ])
    }

    fn enroll(mode: EnrollmentMode) -> RuntimeEnrollment {
        RuntimeEnrollment {
            mode,
            present: true,
        }
    }

    #[test]
    fn enrollment_off_routes_nothing_available() {
        let r = derive_capability_route_enrolled(
            "测试挂了",
            &suite_and_governed(),
            "claude-code",
            &enroll(EnrollmentMode::Off),
        );
        assert_eq!(r.status, CapabilityRouteStatus::Degraded);
        assert!(r.primary.is_none());
        assert!(r
            .recommendations
            .iter()
            .all(|x| x.availability == CapabilityAvailability::CapabilityNotEnrolled));
        assert!(r
            .fallback
            .contains("none are enrolled into Capability Route"));
        // Degraded never blocks — it is still advisory.
        assert!(r.advisory);
    }

    #[test]
    fn enrollment_absent_is_fail_closed() {
        // present=false ⇒ nothing enrolled regardless of the recorded mode.
        let r = derive_capability_route_enrolled(
            "测试挂了",
            &suite_and_governed(),
            "claude-code",
            &RuntimeEnrollment {
                mode: EnrollmentMode::SuiteOnly,
                present: false,
            },
        );
        assert!(r
            .recommendations
            .iter()
            .all(|x| x.availability == CapabilityAvailability::CapabilityNotEnrolled));
        assert!(r.primary.is_none());
        assert_eq!(r.status, CapabilityRouteStatus::Degraded);
    }

    #[test]
    fn enrollment_suite_only_enrolls_suite_not_governed() {
        let r = derive_capability_route_enrolled(
            "测试挂了",
            &suite_and_governed(),
            "claude-code",
            &enroll(EnrollmentMode::SuiteOnly),
        );
        let ad = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "auto-debug")
            .unwrap();
        let c7 = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "context7")
            .unwrap();
        assert_eq!(ad.availability, CapabilityAvailability::Available);
        assert_eq!(
            c7.availability,
            CapabilityAvailability::CapabilityNotEnrolled
        );
        assert_eq!(r.primary.as_deref(), Some("auto-debug"));
        assert_eq!(r.status, CapabilityRouteStatus::Routed);
    }

    #[test]
    fn enrollment_adopted_enrolls_governed() {
        let r = derive_capability_route_enrolled(
            "测试挂了",
            &suite_and_governed(),
            "claude-code",
            &enroll(EnrollmentMode::Adopted),
        );
        let c7 = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "context7")
            .unwrap();
        assert_eq!(c7.availability, CapabilityAvailability::Available);
    }

    #[test]
    fn enrollment_review_all_enrolls_all() {
        let r = derive_capability_route_enrolled(
            "测试挂了",
            &suite_and_governed(),
            "claude-code",
            &enroll(EnrollmentMode::ReviewAll),
        );
        assert!(r
            .recommendations
            .iter()
            .all(|x| x.availability == CapabilityAvailability::Available));
    }

    #[test]
    fn read_enrollment_round_trips_and_fails_closed() {
        let base = std::env::temp_dir().join(format!("ags-enroll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("rt");

        // Missing file → fail-closed Off / not present.
        let e0 = read_enrollment(&home);
        assert_eq!(e0.mode, EnrollmentMode::Off);
        assert!(!e0.present);

        // Written by render_enrollment_json → read back the exact mode.
        let path = enrollment_file_path(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            render_enrollment_json(EnrollmentMode::SuiteOnly, "test"),
        )
        .unwrap();
        let e1 = read_enrollment(&home);
        assert_eq!(e1.mode, EnrollmentMode::SuiteOnly);
        assert!(e1.present);

        // Malformed JSON → fail-closed.
        std::fs::write(&path, "{ not json").unwrap();
        let e2 = read_enrollment(&home);
        assert_eq!(e2.mode, EnrollmentMode::Off);
        assert!(!e2.present);

        // Unknown mode value → fail-closed.
        std::fs::write(&path, "{\"mode\":\"bogus\"}").unwrap();
        let e3 = read_enrollment(&home);
        assert!(!e3.present);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn render_enrollment_json_never_asserts_account_state() {
        let s = render_enrollment_json(EnrollmentMode::Adopted, "ags setup");
        assert!(s.contains("\"mode\": \"adopted\""));
        assert!(s.contains("runtime-derived-only"));
        // Hard boundary: setup never writes a configured auth status or any
        // credential material into the evidence file.
        let lc = s.to_lowercase();
        assert!(!lc.contains("configured"));
        assert!(!lc.contains("token"));
        assert!(!lc.contains("secret"));
        assert!(!lc.contains("password"));
    }

    #[test]
    fn route_request_with_runtime_home_respects_enrollment() {
        let root = locate_manifest_root(&suite_root());
        let base = std::env::temp_dir().join(format!("ags-enroll-rr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("rt");

        // No enrollment evidence → fail-closed: diagnosing-bugs still surfaces but as
        // not-enrolled, with no primary; the route stays advisory.
        let r0 = route_request_with_runtime_home("测试挂了，帮我看下", &root, "claude-code", &home);
        assert!(r0.advisory);
        assert!(!r0.enrollment.present);
        let ad0 = r0
            .recommendations
            .iter()
            .find(|x| x.capability_name == "diagnosing-bugs")
            .expect("diagnosing-bugs should still surface as a recommendation");
        assert_eq!(
            ad0.availability,
            CapabilityAvailability::CapabilityNotEnrolled
        );
        assert!(r0.primary.is_none());

        // suite-only enrollment → diagnosing-bugs (SuiteManaged) is enrolled; its final
        // availability then depends on host visibility/health (machine dependent),
        // so we only assert it is no longer "not enrolled".
        let path = enrollment_file_path(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            render_enrollment_json(EnrollmentMode::SuiteOnly, "test"),
        )
        .unwrap();
        let r1 = route_request_with_runtime_home("测试挂了，帮我看下", &root, "claude-code", &home);
        assert!(r1.enrollment.present);
        assert_eq!(r1.enrollment.mode, EnrollmentMode::SuiteOnly);
        let ad1 = r1
            .recommendations
            .iter()
            .find(|x| x.capability_name == "diagnosing-bugs")
            .unwrap();
        assert_ne!(
            ad1.availability,
            CapabilityAvailability::CapabilityNotEnrolled
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// End-to-end through the production wiring: malformed evidence fail-closes,
    /// and adopted / review-all modes are read and echoed. Hermetic (temp home).
    #[test]
    fn route_request_with_runtime_home_malformed_and_modes() {
        let root = locate_manifest_root(&suite_root());
        let base = std::env::temp_dir().join(format!("ags-enroll-rr2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("rt");
        let path = enrollment_file_path(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Malformed evidence → fail-closed: present=false, advisory, no primary.
        std::fs::write(&path, "{ not json").unwrap();
        let rm = route_request_with_runtime_home("测试挂了", &root, "claude-code", &home);
        assert!(!rm.enrollment.present);
        assert!(rm.advisory);
        assert!(rm.primary.is_none());

        // review-all evidence → enrollment echoed as review-all/present.
        std::fs::write(
            &path,
            render_enrollment_json(EnrollmentMode::ReviewAll, "test"),
        )
        .unwrap();
        let rr = route_request_with_runtime_home("测试挂了", &root, "claude-code", &home);
        assert!(rr.enrollment.present);
        assert_eq!(rr.enrollment.mode, EnrollmentMode::ReviewAll);

        // adopted evidence → enrollment echoed as adopted/present.
        std::fs::write(
            &path,
            render_enrollment_json(EnrollmentMode::Adopted, "test"),
        )
        .unwrap();
        let ra = route_request_with_runtime_home("测试挂了", &root, "claude-code", &home);
        assert!(ra.enrollment.present);
        assert_eq!(ra.enrollment.mode, EnrollmentMode::Adopted);

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── Manifest-rooted wiring ───────────────────────────────────────────────

    /// `locate_manifest_root` resolves a subdirectory up to the manifest root, so
    /// a caller invoked from a subdir does not spuriously miss the manifests.
    #[test]
    fn locate_manifest_root_walks_up_from_subdir() {
        let base = std::env::temp_dir().join(format!(
            "ags-capability-route-locate-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let child = repo.join("crates/ags-mcp/src");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(repo.join("manifests")).unwrap();
        std::fs::write(repo.join("manifests/skills-registry.yaml"), "skills: []\n").unwrap();
        std::fs::write(repo.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();

        // Both the root and a deep subdir resolve to the same manifest root.
        assert_eq!(locate_manifest_root(&repo), repo);
        assert_eq!(locate_manifest_root(&child), repo);

        // No manifests anywhere → falls back to the start path (never errors).
        let bare = base.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        assert_eq!(locate_manifest_root(&bare), bare);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Suite root for crate-level smoke (two levels up from the crate dir).
    fn suite_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// `route_request` reads the real suite manifests and routes a debug demand,
    /// staying advisory. Availability is host/machine dependent, so the contract
    /// asserted here is only: demand is classified and the route is advisory.
    #[test]
    fn route_request_reads_suite_manifests_and_is_advisory() {
        let root = locate_manifest_root(&suite_root());
        let route = route_request("测试挂了，帮我看下", &root, "claude-code");
        assert_eq!(route.demand_kind, DemandKind::Debug);
        assert_eq!(route.active_host, "claude-code");
        assert!(route.advisory, "capability route is always advisory");
        // The suite manifests annotate diagnosing-bugs (primary) + systematic-debugging
        // for the debug demand (auto-debug retired), so the request must route to
        // the canonical successor.
        assert!(
            route
                .recommendations
                .iter()
                .any(|r| r.capability_name == "diagnosing-bugs"),
            "debug demand should surface diagnosing-bugs from the suite manifests, got {:?}",
            route
                .recommendations
                .iter()
                .map(|r| &r.capability_name)
                .collect::<Vec<_>>()
        );
    }

    /// Empty `active_host` is host-agnostic and conservative: it still reads the
    /// manifests (so a demand is classified) but yields no positive availability.
    #[test]
    fn route_request_host_agnostic_is_conservative() {
        let root = locate_manifest_root(&suite_root());
        let route = route_request("测试挂了，帮我看下", &root, "");
        assert_eq!(route.active_host, "host-agnostic");
        assert!(route.advisory);
        assert!(
            route.primary.is_none(),
            "host-agnostic must not declare a primary available capability"
        );
    }

    /// Example-driven route smoke: every manifest `examples.positive` on a
    /// routable member must route to THAT member. Hermetic on enrollment
    /// (fully_enrolled — no evidence file read); asserts MEMBERSHIP only, since
    /// final availability is host/machine dependent. This is the example-driven
    /// smoke the verify gate relies on; it reads examples from the built
    /// inventory (manifest = single source), not a duplicated fixture table.
    #[test]
    fn manifest_positive_examples_route_to_their_member() {
        let root = locate_manifest_root(&suite_root());
        let ctx = ConsoleContext::system(root);
        let inventory = build_inventory(&ctx, &["claude-code"]);
        let enrolled = RuntimeEnrollment::fully_enrolled();
        let mut checked = 0;
        for cap in &inventory.capabilities {
            let Some(meta) = &cap.routing else { continue };
            if meta.route_state != RouteState::Routable {
                continue;
            }
            for ex in &meta.examples.positive {
                let route =
                    derive_capability_route_enrolled(ex, &inventory, "claude-code", &enrolled);
                assert!(
                    route
                        .recommendations
                        .iter()
                        .any(|r| r.capability_name == cap.name),
                    "positive example {ex:?} for {} did not route to it (recs: {:?})",
                    cap.name,
                    route
                        .recommendations
                        .iter()
                        .map(|r| &r.capability_name)
                        .collect::<Vec<_>>()
                );
                checked += 1;
            }
        }
        assert!(
            checked > 0,
            "expected at least one positive example to smoke"
        );
    }

    // ── parent / entrypoint route-target deref (AGS 2.7) ────────────────────

    /// A route target (routing.parent set) derefs `primary` to its real
    /// host-visible PARENT and surfaces the entrypoint; availability is the
    /// PARENT's (superpowers is Visible+Healthy here even though the route
    /// target's own host_visibility is NotVisible). verify → superpowers,
    /// entrypoint = verification-before-completion. (acceptance criterion 2)
    #[test]
    fn verify_derefs_to_superpowers_parent_with_playbook_entrypoint() {
        let mut vbc = routing(&["verify"], false, 50);
        vbc.parent = Some(ParentRef {
            kind: ManagedKind::Skill,
            name: "superpowers".to_string(),
        });
        vbc.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Playbook,
            name: "verification-before-completion".to_string(),
        });
        let mut sp = routing(&[], false, 100);
        sp.route_state = RouteState::NotRoutable;
        let inventory = inv(vec![
            healthy_skill("superpowers", sp),
            cap(
                "verification-before-completion",
                ManagedKind::Skill,
                true,
                "claude-code",
                HostVisibilityStatus::NotVisible,
                HealthStatus::Unknown,
                vbc,
            ),
        ]);
        let r = derive_capability_route("做完了验证一下", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::Verify);
        assert_eq!(r.status, CapabilityRouteStatus::Routed);
        // primary derefs to the real parent body, NEVER the route target itself.
        assert_eq!(r.primary.as_deref(), Some("superpowers"));
        assert_ne!(r.primary.as_deref(), Some("verification-before-completion"));
        assert_eq!(
            r.entrypoint.as_ref().map(|e| e.name.as_str()),
            Some("verification-before-completion")
        );
    }

    /// debug routes to the canonical primary diagnosing-bugs (priority 50), NOT the
    /// secondary systematic-debugging playbook (priority 70). (criterion 5)
    #[test]
    fn debug_routes_to_diagnosing_bugs_not_the_playbook() {
        let mut sysdbg = routing(&["debug"], false, 70);
        sysdbg.parent = Some(ParentRef {
            kind: ManagedKind::Skill,
            name: "superpowers".to_string(),
        });
        sysdbg.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Playbook,
            name: "systematic-debugging".to_string(),
        });
        let mut sp = routing(&[], false, 100);
        sp.route_state = RouteState::NotRoutable;
        let inventory = inv(vec![
            healthy_skill("superpowers", sp),
            healthy_skill("diagnosing-bugs", routing(&["debug"], false, 50)),
            cap(
                "systematic-debugging",
                ManagedKind::Skill,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                sysdbg,
            ),
        ]);
        let r = derive_capability_route("测试挂了，帮我看下", &inventory, "claude-code");
        assert_eq!(r.primary.as_deref(), Some("diagnosing-bugs"));
        assert!(r.entrypoint.is_none());
    }

    /// brainstorm routes to grill-with-docs (50), NOT the brainstorming playbook
    /// (70). (criterion 6)
    #[test]
    fn brainstorm_routes_to_grill_not_the_playbook() {
        let mut brs = routing(&["brainstorm"], false, 70);
        brs.parent = Some(ParentRef {
            kind: ManagedKind::Skill,
            name: "superpowers".to_string(),
        });
        brs.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Playbook,
            name: "brainstorming".to_string(),
        });
        let mut sp = routing(&[], false, 100);
        sp.route_state = RouteState::NotRoutable;
        let inventory = inv(vec![
            healthy_skill("superpowers", sp),
            healthy_skill("grill-with-docs", routing(&["brainstorm"], false, 50)),
            cap(
                "brainstorming",
                ManagedKind::Skill,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                brs,
            ),
        ]);
        let r = derive_capability_route("帮我 brainstorm 一个方案", &inventory, "claude-code");
        assert_eq!(r.primary.as_deref(), Some("grill-with-docs"));
        assert!(r.entrypoint.is_none());
    }

    /// An MCP tool entrypoint derefs `primary` to the parent MCP server (kind
    /// mcp) and surfaces entrypoint=get-library-docs — proving the model is NOT
    /// superpowers-special-cased (the second example kind). (point 5)
    #[test]
    fn mcp_tool_entrypoint_derefs_to_parent_mcp() {
        let mut tool = routing(&["docs-lookup"], false, 28);
        tool.parent = Some(ParentRef {
            kind: ManagedKind::Mcp,
            name: "context7".to_string(),
        });
        tool.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Tool,
            name: "get-library-docs".to_string(),
        });
        let inventory = inv(vec![
            cap(
                "context7",
                ManagedKind::Mcp,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                routing(&["docs-lookup"], false, 30),
            ),
            cap(
                "context7:get-library-docs",
                ManagedKind::Mcp,
                true,
                "claude-code",
                HostVisibilityStatus::NotVisible,
                HealthStatus::Unknown,
                tool,
            ),
        ]);
        let r = derive_capability_route("查文档怎么用这个库", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::DocsLookup);
        assert_eq!(r.primary.as_deref(), Some("context7"));
        assert_eq!(
            r.entrypoint.as_ref().map(|e| e.name.as_str()),
            Some("get-library-docs")
        );
        let rec = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "context7:get-library-docs")
            .expect("tool route target present");
        assert_eq!(rec.capability_kind, ManagedKind::Mcp);
        assert_eq!(
            rec.parent.as_ref().map(|p| p.name.as_str()),
            Some("context7")
        );
    }

    /// A CLI-subcommand entrypoint with a DEGRADED parent stays fail-closed: the
    /// parent lark-cli is not Available, so there is no primary (advisory only),
    /// yet the route target still surfaces with its parent ref. Proves the
    /// parent-deref is fail-closed for the third kind (cli-backed).
    #[test]
    fn cli_subcommand_entrypoint_fail_closed_on_degraded_parent() {
        let mut sub = routing(&["mail-send"], false, 60);
        sub.parent = Some(ParentRef {
            kind: ManagedKind::CliBacked,
            name: "lark-cli".to_string(),
        });
        sub.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Subcommand,
            name: "mail-send".to_string(),
        });
        let mut larkr = routing(&[], false, 100);
        larkr.route_state = RouteState::NotRoutable;
        let inventory = inv(vec![
            cap(
                "lark-cli",
                ManagedKind::CliBacked,
                false,
                "claude-code",
                HostVisibilityStatus::NotVisible,
                HealthStatus::Degraded,
                larkr,
            ),
            cap(
                "lark-cli:mail-send",
                ManagedKind::CliBacked,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                sub,
            ),
        ]);
        let r = derive_capability_route("发个邮件给张三", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::MailSend);
        // degraded parent → fail-closed: no primary, route stays advisory.
        assert!(r.primary.is_none());
        let rec = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "lark-cli:mail-send")
            .expect("cli subcommand route target present");
        assert_eq!(
            rec.parent.as_ref().map(|p| p.name.as_str()),
            Some("lark-cli")
        );
    }

    /// An auth-gated entrypoint (`requires_auth`) over a VISIBLE/HEALTHY,
    /// auth-free parent must NOT inherit `Available`: the entrypoint's own
    /// credential still gates it. Guards against an auth-gated route target
    /// becoming `primary` on the strength of its parent alone. (Codex
    /// adversarial finding — fail-closed auth on the entrypoint.)
    #[test]
    fn auth_gated_entrypoint_over_healthy_parent_is_not_primary() {
        let mut sub = routing(&["mail-send"], false, 60);
        sub.parent = Some(ParentRef {
            kind: ManagedKind::CliBacked,
            name: "lark-cli".to_string(),
        });
        sub.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Subcommand,
            name: "mail-send".to_string(),
        });
        sub.requires_auth = true; // the entrypoint needs a credential
                                  // The parent lark-cli is fully reachable (visible + healthy + canonical)
                                  // and auth-free — only the entrypoint requires auth.
        let mut larkr = routing(&[], false, 100);
        larkr.route_state = RouteState::NotRoutable;
        let inventory = inv(vec![
            cap(
                "lark-cli",
                ManagedKind::CliBacked,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                larkr,
            ),
            cap(
                "lark-cli:mail-send",
                ManagedKind::CliBacked,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                sub,
            ),
        ]);
        let r = derive_capability_route("发个邮件给张三", &inventory, "claude-code");
        assert_eq!(r.demand_kind, DemandKind::MailSend);
        // auth-gated entrypoint must NOT become primary just because its parent
        // is reachable.
        assert!(
            r.primary.is_none(),
            "auth-gated entrypoint must not inherit Available from an auth-free parent"
        );
        let rec = r
            .recommendations
            .iter()
            .find(|x| x.capability_name == "lark-cli:mail-send")
            .expect("entrypoint present");
        assert_eq!(
            rec.availability,
            CapabilityAvailability::CapabilityAuthRequired
        );
    }

    /// The MCP-facing JSON (ags_solution_check serializes CapabilityRoute)
    /// surfaces the route-level `entrypoint` AND the recommendation-level
    /// `parent`, so a host sees both the host-visible parent capability and the
    /// internal entrypoint it routes through. (point 6)
    #[test]
    fn json_surfaces_parent_and_entrypoint() {
        let mut tool = routing(&["docs-lookup"], false, 28);
        tool.parent = Some(ParentRef {
            kind: ManagedKind::Mcp,
            name: "context7".to_string(),
        });
        tool.entrypoint = Some(EntrypointRef {
            kind: EntrypointKind::Tool,
            name: "get-library-docs".to_string(),
        });
        let inventory = inv(vec![
            cap(
                "context7",
                ManagedKind::Mcp,
                true,
                "claude-code",
                HostVisibilityStatus::Visible,
                HealthStatus::Healthy,
                routing(&["docs-lookup"], false, 30),
            ),
            cap(
                "context7:get-library-docs",
                ManagedKind::Mcp,
                true,
                "claude-code",
                HostVisibilityStatus::NotVisible,
                HealthStatus::Unknown,
                tool,
            ),
        ]);
        let r = derive_capability_route("查文档怎么用这个库", &inventory, "claude-code");
        let json = serde_json::to_string(&r).expect("serialize route");
        assert!(
            json.contains("\"entrypoint\""),
            "route JSON must surface entrypoint: {json}"
        );
        assert!(json.contains("\"get-library-docs\""));
        assert!(
            json.contains("\"parent\""),
            "recommendation JSON must surface parent: {json}"
        );
        assert!(json.contains("\"context7\""));
    }

    /// build_inventory synthesizes internal-entrypoint route targets from the
    /// REAL manifests: managed_status RouteTarget, routing.parent set, and NO
    /// expected-host expectation (so `ags capability verify` never fails on
    /// them). MCP-tool route targets deref to their parent MCP, not a top-level
    /// server. (acceptance criteria 3/4 + point 5, on real data)
    #[test]
    fn route_targets_synthesized_without_host_expectation() {
        let root = locate_manifest_root(&suite_root());
        let ctx = ConsoleContext::system(root);
        let inventory = build_inventory(&ctx, &["claude-code"]);
        let find = |n: &str| inventory.capabilities.iter().find(|c| c.name == n);

        let vbc = find("verification-before-completion").expect("vbc route target present");
        assert_eq!(vbc.managed_status, ManagedStatus::RouteTarget);
        assert!(vbc.is_route_target());
        assert_eq!(
            vbc.routing
                .as_ref()
                .and_then(|r| r.parent.as_ref())
                .map(|p| p.name.as_str()),
            Some("superpowers")
        );
        assert!(
            vbc.expected_hosts.is_empty(),
            "route target must NOT create a host-visible expectation"
        );

        // The real superpowers body remains a normal (non-route-target) capability.
        let sp = find("superpowers").expect("superpowers body present");
        assert!(!sp.is_route_target());

        // An MCP-tool route target derefs to its parent MCP, never a top-level server.
        let tool = find("context7:get-library-docs").expect("mcp tool route target present");
        assert_eq!(tool.kind, ManagedKind::Mcp);
        assert_eq!(tool.managed_status, ManagedStatus::RouteTarget);
        assert_eq!(
            tool.routing
                .as_ref()
                .and_then(|r| r.parent.as_ref())
                .map(|p| p.name.as_str()),
            Some("context7")
        );
        assert!(tool.expected_hosts.is_empty());
    }

    /// auto-* retirement is COMPLETE (2.7): every demand the retired aliases used
    /// to serve now has a NON-alias routable successor in the suite manifests, so
    /// no demand is orphaned — debug → diagnosing-bugs, brainstorm → grill-with-docs /
    /// prototype, verify → verification-before-completion. The retired aliases are
    /// no longer routable (removed from suite.yaml; route_state: retired in the
    /// registry → never surface as routable capabilities).
    #[test]
    fn retired_demands_have_non_alias_successors() {
        let root = locate_manifest_root(&suite_root());
        let ctx = ConsoleContext::system(root);
        let inventory = build_inventory(&ctx, &["claude-code"]);
        let non_alias_serves = |demand: &str| {
            inventory.capabilities.iter().any(|c| {
                c.routing.as_ref().is_some_and(|m| {
                    m.route_state == RouteState::Routable
                        && !m.is_compatibility_alias
                        && m.intent_tags.iter().any(|t| t == demand)
                })
            })
        };
        for demand in ["debug", "brainstorm", "verify"] {
            assert!(
                non_alias_serves(demand),
                "demand `{demand}` must have a non-alias routable successor after auto-* retirement"
            );
        }
        // The retired aliases must never surface as routable capabilities.
        let retired_routes = inventory.capabilities.iter().any(|c| {
            matches!(
                c.name.as_str(),
                "auto-brainstorm" | "auto-debug" | "auto-verify"
            ) && c
                .routing
                .as_ref()
                .is_some_and(|m| m.route_state == RouteState::Routable)
        });
        assert!(
            !retired_routes,
            "retired auto-* aliases must not be routable capabilities"
        );
    }
}
