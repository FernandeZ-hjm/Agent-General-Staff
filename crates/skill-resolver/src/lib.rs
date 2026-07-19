//! Deterministic host capability catalog and exact skill resolution.
//!
//! The host performs semantic selection from [`SkillCard`] metadata. AGS only
//! validates the selected `skill_id`, optional entrypoint and snapshot hash.
//! Legacy `SkillDemand` mappings remain catalog metadata for migration; they
//! are not a production routing authority.

use request_governance::SkillDemand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skill_governance::console::{
    build_inventory, inventory_snapshot_hash, CommandOutcome, CommandRunner, ConsoleContext,
    HealthStatus, HostVisibilityStatus, ManagedCapability, ManagedKind, ManagedStatus,
    RegistryStatus, RouteState,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION: &str = "0.3.0-host-capability-snapshot";
pub const CAPABILITY_SNAPSHOT_SCHEMA_VERSION: &str = HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION;
pub const USER_OVERLAY_SCHEMA_VERSION: &str = "0.3.0-user-skill-overlay";
pub const OVERLAY_MUTATION_EVENT_SCHEMA_VERSION: &str = "0.3.0-overlay-mutation-receipt";
pub const SKILL_USAGE_EVENT_SCHEMA_VERSION: &str = "0.3.0-skill-usage-event";
pub const SKILL_REASON_CODES: &[&str] = &[
    "candidate_requires_adoption",
    "registry_not_routable",
    "retired",
    "canonical_missing",
    "host_not_visible",
    "health_degraded",
    "auth_required",
    "metadata_incomplete",
    "snapshot_stale",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemandRoute {
    pub demand: SkillDemand,
    pub skill_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
}

#[derive(Debug)]
pub enum RegistryError {
    Read(std::io::Error),
    Parse(serde_yaml::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAuthorityError {
    pub tried: Vec<String>,
}

impl fmt::Display for CapabilityAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "no AGS capability authority root found; checked {}",
            self.tried.join(", ")
        )
    }
}

impl std::error::Error for CapabilityAuthorityError {}

fn is_capability_authority_root(path: &Path) -> bool {
    path.join("manifests/skills-registry.yaml").is_file()
        && path.join("manifests/mcp-registry.yaml").is_file()
}

fn normalized_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

fn installed_source_root(runtime_home: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(runtime_home.join("install-manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&content).ok()?;
    manifest
        .get("source_root")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
}

pub fn resolve_capability_authority_root(
    target: &Path,
    runtime_home: &Path,
    explicit: Option<PathBuf>,
) -> Result<PathBuf, CapabilityAuthorityError> {
    let mut tried = Vec::new();
    let mut candidates = Vec::new();
    if let Some(path) = explicit {
        candidates.push(("AGS_SOURCE_ROOT", path));
    }
    if let Some(path) = installed_source_root(runtime_home) {
        candidates.push(("runtime install manifest", path));
    }

    for (origin, candidate) in candidates {
        let candidate = normalized_path(&candidate);
        if is_capability_authority_root(&candidate) {
            return Ok(candidate);
        }
        tried.push(format!("{origin}: {}", candidate.display()));
    }

    let normalized_target = normalized_path(target);
    let mut current = if normalized_target.is_file() {
        normalized_target
            .parent()
            .unwrap_or(&normalized_target)
            .to_path_buf()
    } else {
        normalized_target.clone()
    };
    loop {
        if is_capability_authority_root(&current) {
            return Ok(current);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    tried.push(format!("target ancestry: {}", normalized_target.display()));
    Err(CapabilityAuthorityError { tried })
}

pub fn locate_runtime_home() -> PathBuf {
    if let Some(path) = std::env::var_os("AGS_RUNTIME_HOME") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("AGS_HOME") {
        return PathBuf::from(path);
    }
    ags_platform::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ags/runtime")
}

fn safe_host(active_host: &str) -> String {
    let host = active_host.trim();
    if host.is_empty() {
        return "host-agnostic".to_string();
    }
    host.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub fn snapshot_path(runtime_home: &Path, active_host: &str) -> PathBuf {
    runtime_home
        .join("capability-snapshot")
        .join(format!("{}.json", safe_host(active_host)))
}

pub fn overlay_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("skill-registry/user-overlay.yaml")
}

pub fn overlay_events_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("skill-registry/user-overlay-events.ndjson")
}

pub fn usage_path(runtime_home: &Path, active_host: &str) -> PathBuf {
    runtime_home
        .join("skill-usage")
        .join(format!("{}.ndjson", safe_host(active_host)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    NotRequired,
    Satisfied,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceState {
    Discovered,
    Candidate,
    ManagedInactive,
    Active,
    Ignored,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AvailabilityState {
    Ready,
    Degraded { reason_codes: Vec<String> },
    Unavailable { reason_codes: Vec<String> },
}

impl AvailabilityState {
    pub fn reason_codes(&self) -> &[String] {
        match self {
            Self::Ready => &[],
            Self::Degraded { reason_codes } | Self::Unavailable { reason_codes } => reason_codes,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    #[default]
    Unobserved,
    Warm,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Suite,
    HostSystem,
    UserInstalled,
    ProjectLocal,
    EnabledPlugin,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCard {
    pub skill_id: String,
    pub display_name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoints: Vec<String>,
    pub source_kind: SkillSourceKind,
    pub governance: GovernanceState,
    pub availability: AvailabilityState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
    pub requires_auth: bool,
    pub auth_state: AuthState,
    #[serde(default)]
    pub activity: ActivityState,
    pub version: String,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveSkill {
    pub skill_id: String,
    pub invoke_hint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_entrypoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_demands: Vec<SkillDemand>,
    pub source_hash: String,
}

#[derive(Debug, Clone)]
pub struct ActiveSkillTable {
    pub active_host: String,
    snapshot_hash: String,
    skills: HashMap<String, ActiveSkill>,
}

impl ActiveSkillTable {
    pub fn new(
        active_host: impl Into<String>,
        snapshot_hash: impl Into<String>,
        active_skills: Vec<ActiveSkill>,
    ) -> Result<Self, ResolveError> {
        let mut skills = HashMap::with_capacity(active_skills.len());
        for skill in active_skills {
            let skill_id = skill.skill_id.clone();
            if skills.insert(skill_id.clone(), skill).is_some() {
                return Err(ResolveError::DuplicateSkill { skill_id });
            }
        }
        Ok(Self {
            active_host: active_host.into(),
            snapshot_hash: snapshot_hash.into(),
            skills,
        })
    }

    pub fn active_skills(&self) -> Vec<ActiveSkill> {
        let mut skills: Vec<_> = self.skills.values().cloned().collect();
        sort_active_skills(&mut skills);
        skills
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSelection {
    pub skill_id: String,
    pub invoke_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    GovernancePrecondition(&'static str),
    DuplicateSkill {
        skill_id: String,
    },
    EntrypointNotAllowed {
        skill_id: String,
        entrypoint: String,
    },
    SnapshotHashMismatch {
        expected: String,
        supplied: String,
    },
}

pub fn resolve_skill(
    skill_id: &str,
    entrypoint: Option<&str>,
    snapshot_hash: &str,
    table: &ActiveSkillTable,
) -> Result<SkillSelection, ResolveError> {
    if snapshot_hash != table.snapshot_hash {
        return Err(ResolveError::SnapshotHashMismatch {
            expected: table.snapshot_hash.clone(),
            supplied: snapshot_hash.to_string(),
        });
    }
    let active = table
        .skills
        .get(skill_id)
        .ok_or(ResolveError::GovernancePrecondition("skill_not_active"))?;
    if let Some(entrypoint) = entrypoint {
        if !active
            .allowed_entrypoints
            .iter()
            .any(|allowed| allowed == entrypoint)
        {
            return Err(ResolveError::EntrypointNotAllowed {
                skill_id: skill_id.to_string(),
                entrypoint: entrypoint.to_string(),
            });
        }
    }
    Ok(SkillSelection {
        skill_id: active.skill_id.clone(),
        invoke_hint: active.invoke_hint.clone(),
        entrypoint: entrypoint.map(str::to_string),
        snapshot_hash: snapshot_hash.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilitySnapshot {
    pub schema_version: String,
    pub host: String,
    pub registry_hash: String,
    pub overlay_hash: String,
    pub runtime_hash: String,
    pub catalog_hash: String,
    pub active_table_hash: String,
    pub snapshot_hash: String,
    pub catalog: Vec<SkillCard>,
    pub active_skills: Vec<ActiveSkill>,
}

pub type CapabilitySnapshot = HostCapabilitySnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    SkillSnapshotStale,
    SnapshotIntegrityFailed,
    InvalidActiveTable(ResolveError),
}

impl HostCapabilitySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host: impl Into<String>,
        registry_hash: impl Into<String>,
        overlay_hash: impl Into<String>,
        runtime_hash: impl Into<String>,
        mut catalog: Vec<SkillCard>,
        mut active_skills: Vec<ActiveSkill>,
    ) -> Result<Self, ResolveError> {
        let host = host.into();
        sort_skill_cards(&mut catalog);
        let table = ActiveSkillTable::new(host.clone(), "pending", active_skills)?;
        active_skills = table.active_skills();
        let catalog_hash = catalog_hash(&catalog);
        let active_table_hash = active_table_hash(&active_skills);
        let mut snapshot = Self {
            schema_version: HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION.to_string(),
            host,
            registry_hash: registry_hash.into(),
            overlay_hash: overlay_hash.into(),
            runtime_hash: runtime_hash.into(),
            catalog_hash,
            active_table_hash,
            snapshot_hash: String::new(),
            catalog,
            active_skills,
        };
        snapshot.snapshot_hash = snapshot_integrity_hash(&snapshot);
        Ok(snapshot)
    }

    pub fn validate(
        &self,
        expected_host: &str,
        expected_registry_hash: &str,
        expected_overlay_hash: &str,
        expected_runtime_hash: &str,
    ) -> Result<ActiveSkillTable, SnapshotError> {
        if self.schema_version != HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION
            || self.host != expected_host
            || self.registry_hash != expected_registry_hash
            || self.overlay_hash != expected_overlay_hash
            || self.runtime_hash != expected_runtime_hash
        {
            return Err(SnapshotError::SkillSnapshotStale);
        }
        if self.catalog_hash != catalog_hash(&self.catalog)
            || self.active_table_hash != active_table_hash(&self.active_skills)
            || self.snapshot_hash != snapshot_integrity_hash(self)
        {
            return Err(SnapshotError::SnapshotIntegrityFailed);
        }
        ActiveSkillTable::new(
            self.host.clone(),
            self.snapshot_hash.clone(),
            self.active_skills.clone(),
        )
        .map_err(SnapshotError::InvalidActiveTable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayEntryState {
    Active,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserOverlayEntry {
    pub skill_id: String,
    pub state: OverlayEntryState,
    pub revision: u64,
    pub source_hash: String,
    pub metadata_version: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub entrypoints: Vec<String>,
    #[serde(default)]
    pub invoke_hint: String,
    #[serde(default)]
    pub requires_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSkillOverlay {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub entries: Vec<UserOverlayEntry>,
}

impl Default for UserSkillOverlay {
    fn default() -> Self {
        Self {
            schema_version: USER_OVERLAY_SCHEMA_VERSION.to_string(),
            revision: 0,
            entries: Vec::new(),
        }
    }
}

pub fn load_user_overlay(runtime_home: &Path) -> Result<UserSkillOverlay, String> {
    let path = overlay_path(runtime_home);
    if !path.exists() {
        return Ok(UserSkillOverlay::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let overlay: UserSkillOverlay = serde_yaml::from_str(&content)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if overlay.schema_version != USER_OVERLAY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported overlay schema {}; expected {USER_OVERLAY_SCHEMA_VERSION}",
            overlay.schema_version
        ));
    }
    let mut seen = HashSet::new();
    if overlay
        .entries
        .iter()
        .any(|entry| !seen.insert(entry.skill_id.clone()))
    {
        return Err("duplicate skill_id in user overlay".to_string());
    }
    Ok(overlay)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMutationOperation {
    Adopt,
    Ignore,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayMutationReceipt {
    pub schema_version: String,
    pub event_id: String,
    pub timestamp_unix: u64,
    pub operation: OverlayMutationOperation,
    pub skill_id: String,
    pub from_overlay_revision: u64,
    pub to_overlay_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_entry: Option<UserOverlayEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_entry: Option<UserOverlayEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayMutationResult {
    pub schema_version: String,
    pub operation: OverlayMutationOperation,
    pub skill_id: String,
    pub dry_run: bool,
    pub applied: bool,
    pub changed: bool,
    pub status: String,
    pub overlay_revision: u64,
    pub overlay_relative_path: String,
    pub receipt_relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_entry: Option<UserOverlayEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_event_id: Option<String>,
}

/// Plan or apply a machine-private skill lifecycle mutation. The tracked suite
/// registry always wins: any identifier declared there (including retired
/// entries) is rejected before the overlay is considered.
#[allow(clippy::too_many_arguments)]
pub fn mutate_user_overlay(
    manifest_root: &Path,
    runtime_home: &Path,
    host_home: &Path,
    active_host: &str,
    skill_id: &str,
    operation: OverlayMutationOperation,
    restored_from_revision: Option<u64>,
    apply: bool,
) -> Result<OverlayMutationResult, String> {
    if !safe_skill_id(skill_id) {
        return Err("invalid_skill_id".to_string());
    }
    let registry = load_registry_document(manifest_root)
        .map_err(|error| format!("cannot load official registry: {error:?}"))?;
    if registry.skills.iter().any(|skill| skill.name == skill_id) {
        return Err("official_registry_precedence".to_string());
    }

    let mut overlay = load_user_overlay(runtime_home)?;
    let before_entry = overlay
        .entries
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned();
    let next_revision = overlay.revision.saturating_add(1);
    let candidate = || external_candidate_card(manifest_root, host_home, active_host, skill_id);

    let mut after_entry = match operation {
        OverlayMutationOperation::Adopt => {
            let card = candidate()?;
            validate_overlay_candidate(&card)?;
            Some(overlay_entry_from_card(
                &card,
                OverlayEntryState::Active,
                next_revision,
            ))
        }
        OverlayMutationOperation::Ignore => {
            if let Some(mut existing) = before_entry.clone() {
                existing.state = OverlayEntryState::Ignored;
                existing.revision = next_revision;
                Some(existing)
            } else {
                let card = candidate()?;
                validate_overlay_candidate(&card)?;
                Some(overlay_entry_from_card(
                    &card,
                    OverlayEntryState::Ignored,
                    next_revision,
                ))
            }
        }
        OverlayMutationOperation::Rollback => {
            let revision =
                restored_from_revision.ok_or_else(|| "rollback_revision_required".to_string())?;
            if revision == 0 {
                None
            } else {
                let events = load_overlay_mutation_receipts(runtime_home)?;
                let mut restored = events
                    .iter()
                    .rev()
                    .flat_map(|event| [event.after_entry.as_ref(), event.before_entry.as_ref()])
                    .flatten()
                    .find(|entry| entry.skill_id == skill_id && entry.revision == revision)
                    .cloned()
                    .ok_or_else(|| "overlay_revision_not_found".to_string())?;
                restored.revision = next_revision;
                Some(restored)
            }
        }
    };

    let changed = !overlay_entries_semantically_equal(before_entry.as_ref(), after_entry.as_ref());
    if !changed {
        return Ok(OverlayMutationResult {
            schema_version: "0.3.0-overlay-mutation-result".to_string(),
            operation,
            skill_id: skill_id.to_string(),
            dry_run: !apply,
            applied: false,
            changed: false,
            status: "noop".to_string(),
            overlay_revision: overlay.revision,
            overlay_relative_path: "skill-registry/user-overlay.yaml".to_string(),
            receipt_relative_path: "skill-registry/user-overlay-events.ndjson".to_string(),
            restored_from_revision,
            proposed_entry: before_entry,
            receipt_event_id: None,
        });
    }

    overlay.entries.retain(|entry| entry.skill_id != skill_id);
    if let Some(entry) = after_entry.take() {
        overlay.entries.push(entry);
    }
    overlay
        .entries
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    overlay.revision = next_revision;
    let after_entry = overlay
        .entries
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned();
    let timestamp_unix = unix_timestamp();
    let event_id = sha256(
        format!(
            "overlay\n{operation:?}\n{skill_id}\n{}\n{next_revision}\n{timestamp_unix}",
            overlay.revision.saturating_sub(1)
        )
        .as_bytes(),
    );
    let receipt = OverlayMutationReceipt {
        schema_version: OVERLAY_MUTATION_EVENT_SCHEMA_VERSION.to_string(),
        event_id: event_id.clone(),
        timestamp_unix,
        operation,
        skill_id: skill_id.to_string(),
        from_overlay_revision: overlay.revision.saturating_sub(1),
        to_overlay_revision: overlay.revision,
        restored_from_revision,
        before_entry: before_entry.clone(),
        after_entry: after_entry.clone(),
    };

    if apply {
        let path = overlay_path(runtime_home);
        let previous = read_existing_private_file(&path)?;
        let capability_path = snapshot_path(runtime_home, active_host);
        let previous_snapshot = read_existing_private_file(&capability_path)?;
        let receipt_path = overlay_events_path(runtime_home);
        let previous_receipts = read_existing_private_file(&receipt_path)?;
        let receipt_bytes = render_overlay_receipt_append(previous_receipts.as_deref(), &receipt)?;
        let serialized = serde_yaml::to_string(&overlay)
            .map_err(|error| format!("cannot serialize user overlay: {error}"))?;
        write_private_atomic(&path, serialized.as_bytes())?;
        let commit = (|| {
            let snapshot = build_capability_snapshot_with_roots(
                manifest_root,
                active_host,
                runtime_home,
                host_home,
            )
            .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
            let snapshot_json = serde_json::to_string_pretty(&snapshot)
                .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
            write_private_atomic(&capability_path, (snapshot_json + "\n").as_bytes())?;
            write_private_atomic(&receipt_path, &receipt_bytes)
        })();
        if let Err(error) = commit {
            let overlay_rollback = restore_private_file(&path, previous);
            let snapshot_rollback = restore_private_file(&capability_path, previous_snapshot);
            return Err(match (overlay_rollback, snapshot_rollback) {
                (Ok(()), Ok(())) => {
                    format!("overlay transaction failed and was rolled back: {error}")
                }
                (overlay_result, snapshot_result) => format!(
                    "overlay transaction failed: {error}; rollback failed: overlay={overlay_result:?}, snapshot={snapshot_result:?}"
                ),
            });
        }
    }

    Ok(OverlayMutationResult {
        schema_version: "0.3.0-overlay-mutation-result".to_string(),
        operation,
        skill_id: skill_id.to_string(),
        dry_run: !apply,
        applied: apply,
        changed: true,
        status: if apply { "applied" } else { "planned" }.to_string(),
        overlay_revision: overlay.revision,
        overlay_relative_path: "skill-registry/user-overlay.yaml".to_string(),
        receipt_relative_path: "skill-registry/user-overlay-events.ndjson".to_string(),
        restored_from_revision,
        proposed_entry: after_entry,
        receipt_event_id: apply.then_some(event_id),
    })
}

fn restore_private_file(path: &Path, previous: Option<Vec<u8>>) -> Result<(), String> {
    if let Some(bytes) = previous {
        return write_private_atomic(path, &bytes);
    }
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| {
            format!(
                "cannot remove rollback artifact {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn read_existing_private_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot read existing private file {} before mutation: {error}",
            path.display()
        )),
    }
}

fn external_candidate_card(
    manifest_root: &Path,
    host_home: &Path,
    active_host: &str,
    skill_id: &str,
) -> Result<SkillCard, String> {
    let context = ConsoleContext::new(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        Box::new(NoProcessDiscovery),
    );
    let inventory = build_inventory(&context, &[active_host]);
    let capability = inventory
        .capabilities
        .iter()
        .find(|capability| capability.kind == ManagedKind::Skill && capability.name == skill_id)
        .ok_or_else(|| "skill_candidate_not_found".to_string())?;
    Ok(skill_card(
        capability,
        None,
        None,
        &[],
        AuthState::NotRequired,
    ))
}

fn validate_overlay_candidate(card: &SkillCard) -> Result<(), String> {
    if !matches!(
        card.source_kind,
        SkillSourceKind::UserInstalled
            | SkillSourceKind::ProjectLocal
            | SkillSourceKind::EnabledPlugin
            | SkillSourceKind::External
    ) {
        return Err("overlay_source_not_adoptable".to_string());
    }
    if card
        .reason_codes
        .iter()
        .any(|reason| reason == "metadata_incomplete")
    {
        return Err("metadata_incomplete".to_string());
    }
    Ok(())
}

fn overlay_entry_from_card(
    card: &SkillCard,
    state: OverlayEntryState,
    revision: u64,
) -> UserOverlayEntry {
    UserOverlayEntry {
        skill_id: card.skill_id.clone(),
        state,
        revision,
        source_hash: card.source_hash.clone(),
        metadata_version: if card.version == "registry" {
            "skillcard-v1".to_string()
        } else {
            card.version.clone()
        },
        display_name: card.display_name.clone(),
        summary: card.summary.clone(),
        intent_tags: card.intent_tags.clone(),
        entrypoints: card.entrypoints.clone(),
        invoke_hint: format!("[skill: {}]", card.skill_id),
        requires_auth: card.requires_auth,
    }
}

fn overlay_entries_semantically_equal(
    left: Option<&UserOverlayEntry>,
    right: Option<&UserOverlayEntry>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            let mut left = left.clone();
            let mut right = right.clone();
            left.revision = 0;
            right.revision = 0;
            left == right
        }
        _ => false,
    }
}

fn safe_skill_id(skill_id: &str) -> bool {
    !skill_id.is_empty()
        && skill_id.len() <= 128
        && skill_id
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

fn load_overlay_mutation_receipts(
    runtime_home: &Path,
) -> Result<Vec<OverlayMutationReceipt>, String> {
    let path = overlay_events_path(runtime_home);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let receipt: OverlayMutationReceipt = serde_json::from_str(line).map_err(|error| {
                format!("invalid overlay receipt at line {}: {error}", index + 1)
            })?;
            if receipt.schema_version != OVERLAY_MUTATION_EVENT_SCHEMA_VERSION {
                return Err(format!(
                    "unsupported overlay receipt schema at line {}",
                    index + 1
                ));
            }
            Ok(receipt)
        })
        .collect()
}

fn overlay_active_since(receipts: &[OverlayMutationReceipt], skill_id: &str) -> Option<u64> {
    receipts
        .iter()
        .rev()
        .find(|receipt| {
            receipt.skill_id == skill_id
                && receipt
                    .after_entry
                    .as_ref()
                    .is_some_and(|entry| entry.state == OverlayEntryState::Active)
        })
        .map(|receipt| receipt.timestamp_unix)
}

fn render_overlay_receipt_append(
    previous: Option<&[u8]>,
    receipt: &OverlayMutationReceipt,
) -> Result<Vec<u8>, String> {
    let line = serde_json::to_string(receipt).map_err(|error| error.to_string())?;
    if let Some(previous) = previous {
        std::str::from_utf8(previous)
            .map_err(|error| format!("overlay receipt ledger is not UTF-8: {error}"))?;
    }
    let mut bytes = previous.unwrap_or_default().to_vec();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
thread_local! {
    static INJECT_PRIVATE_SYNC_FAILURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn inject_private_sync_failure(file_name: Option<&str>) {
    INJECT_PRIVATE_SYNC_FAILURE.with(|slot| {
        *slot.borrow_mut() = file_name.map(str::to_string);
    });
}

#[cfg(test)]
fn private_sync_failure_is_injected(path: &Path) -> bool {
    INJECT_PRIVATE_SYNC_FAILURE.with(|slot| {
        slot.borrow().as_deref() == path.file_name().and_then(|file_name| file_name.to_str())
    })
}

pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

    let parent = path
        .parent()
        .ok_or_else(|| "private file has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "private file name is not UTF-8".to_string())?;
    let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
    let stage = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| -> Result<(), String> {
        let mut file = options
            .open(&stage)
            .map_err(|error| format!("cannot stage {}: {error}", stage.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write {}: {error}", stage.display()))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync {}: {error}", stage.display()))?;
        #[cfg(test)]
        if private_sync_failure_is_injected(path) {
            return Err(format!(
                "injected sync failure before replacing {}",
                path.display()
            ));
        }
        commit_private_stage(&stage, path)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    write_result
}

/// Prepare every fallible property on the stage path first. The rename is the
/// final operation, so a successful replacement can never be reported as a
/// failed transaction by a later chmod or metadata step.
fn commit_private_stage(stage: &Path, path: &Path) -> Result<(), String> {
    set_private_permissions(stage)?;
    std::fs::rename(stage, path).map_err(|error| {
        format!(
            "cannot atomically replace {} from {}: {error}",
            path.display(),
            stage.display()
        )
    })
}

#[derive(Debug, Deserialize)]
struct RegistryDocument {
    #[serde(default)]
    skills: Vec<RegistrySkill>,
    #[serde(default)]
    demand_routes: Vec<DemandRoute>,
}

#[derive(Debug, Deserialize)]
struct RegistrySkill {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    routing: Option<RegistryRouting>,
}

#[derive(Debug, Deserialize)]
struct RegistryRouting {
    #[serde(default)]
    intent_tags: Vec<String>,
    #[serde(default)]
    requires_auth: bool,
    #[serde(default)]
    invoke_hint: String,
    #[serde(default)]
    route_state: RouteState,
}

#[derive(Debug, Deserialize, Default)]
struct SkillFileMetadata {
    #[serde(default)]
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    intent_tags: Vec<String>,
    #[serde(default)]
    entrypoints: Vec<String>,
    #[serde(default)]
    invoke_hint: String,
    #[serde(default)]
    requires_auth: bool,
    #[serde(default)]
    version: String,
}

fn load_skill_file_metadata(capability: &ManagedCapability) -> SkillFileMetadata {
    let Some(source) = capability.source.as_deref() else {
        return SkillFileMetadata::default();
    };
    let path = Path::new(source);
    let skill_md = if path.is_dir() {
        path.join("SKILL.md")
    } else {
        path.to_path_buf()
    };
    let Ok(content) = std::fs::read_to_string(skill_md) else {
        return SkillFileMetadata::default();
    };
    let Some(rest) = content.strip_prefix("---") else {
        return SkillFileMetadata::default();
    };
    let Some((frontmatter, _)) = rest.split_once("\n---") else {
        return SkillFileMetadata::default();
    };
    serde_yaml::from_str(frontmatter).unwrap_or_default()
}

fn load_registry_document(root: &Path) -> Result<RegistryDocument, RegistryError> {
    let content = std::fs::read_to_string(root.join("manifests/skills-registry.yaml"))
        .map_err(RegistryError::Read)?;
    serde_yaml::from_str(&content).map_err(RegistryError::Parse)
}

pub fn load_demand_routes(root: &Path) -> Result<Vec<DemandRoute>, RegistryError> {
    load_registry_document(root).map(|document| document.demand_routes)
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AuthStateDocument {
    #[serde(default)]
    skills: BTreeMap<String, AuthState>,
}

fn load_auth_states(runtime_home: &Path, host: &str) -> (AuthStateDocument, String) {
    let path = runtime_home
        .join("auth-state")
        .join(format!("{}.json", safe_host(host)));
    let Ok(bytes) = std::fs::read(path) else {
        return (AuthStateDocument::default(), sha256(b"missing-auth-state"));
    };
    let document = serde_json::from_slice(&bytes).unwrap_or_default();
    (document, sha256(&bytes))
}

pub fn build_capability_snapshot(
    manifest_root: &Path,
    active_host: &str,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    build_capability_snapshot_with_runtime_home(manifest_root, active_host, &locate_runtime_home())
}

pub fn build_capability_snapshot_with_runtime_home(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, &host_home)
}

/// Build a snapshot with explicit machine roots and a no-process discovery
/// runner. This is the production seam used by routing as well as the test seam:
/// capability catalog generation never launches host CLIs.
pub fn build_capability_snapshot_with_roots(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let context = ConsoleContext::new(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        Box::new(NoProcessDiscovery),
    );
    let inventory = build_inventory(&context, &[active_host]);
    let registry_document =
        load_registry_document(manifest_root).map_err(SnapshotBuildError::Registry)?;
    let registry_bytes = std::fs::read(manifest_root.join("manifests/skills-registry.yaml"))
        .map_err(SnapshotBuildError::Read)?;
    let registry_active_since =
        std::fs::metadata(manifest_root.join("manifests/skills-registry.yaml"))
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
    let overlay = load_user_overlay(runtime_home).map_err(SnapshotBuildError::Overlay)?;
    let overlay_bytes = std::fs::read(overlay_path(runtime_home)).unwrap_or_default();
    let overlay_modified_since = std::fs::metadata(overlay_path(runtime_home))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let overlay_hash = if overlay_bytes.is_empty() {
        sha256(b"empty-user-overlay")
    } else {
        sha256(&overlay_bytes)
    };
    let (auth_states, auth_hash) = load_auth_states(runtime_home, active_host);
    let usage_events = load_usage_events(runtime_home, active_host);
    let overlay_receipts = load_overlay_mutation_receipts(runtime_home).unwrap_or_default();
    let now_unix = unix_timestamp();

    let metadata: HashMap<_, _> = registry_document
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill))
        .collect();
    let overlay_entries: HashMap<_, _> = overlay
        .entries
        .iter()
        .map(|entry| (entry.skill_id.as_str(), entry))
        .collect();
    let routes_by_skill = routes_by_skill(&registry_document.demand_routes);
    let official_ids: HashSet<_> = inventory
        .capabilities
        .iter()
        .filter(|capability| {
            capability.kind == ManagedKind::Skill
                && capability.registry_status == RegistryStatus::Registered
        })
        .map(|capability| capability.name.as_str())
        .collect();

    let mut catalog = Vec::new();
    let mut active_skills = Vec::new();
    for capability in inventory.capabilities.iter().filter(|capability| {
        capability.kind == ManagedKind::Skill
            && capability.managed_status != ManagedStatus::RouteTarget
    }) {
        let official = official_ids.contains(capability.name.as_str());
        let registry = metadata.get(capability.name.as_str()).copied();
        let overlay_entry = if official {
            None
        } else {
            overlay_entries.get(capability.name.as_str()).copied()
        };
        let legacy_routes = routes_by_skill
            .get(capability.name.as_str())
            .cloned()
            .unwrap_or_default();
        let file_requires_auth = load_skill_file_metadata(capability).requires_auth;
        let auth_state = auth_state_for(
            registry
                .and_then(|item| item.routing.as_ref())
                .is_some_and(|routing| routing.requires_auth)
                || overlay_entry.is_some_and(|entry| entry.requires_auth)
                || file_requires_auth,
            auth_states.skills.get(&capability.name).copied(),
        );
        let mut card = skill_card(
            capability,
            registry,
            overlay_entry,
            &legacy_routes,
            auth_state,
        );
        let active_since = overlay_entry
            .filter(|entry| entry.state == OverlayEntryState::Active)
            .and_then(|_| {
                overlay_active_since(&overlay_receipts, &card.skill_id).or(overlay_modified_since)
            })
            .or_else(|| {
                (card.governance == GovernanceState::Active)
                    .then_some(registry_active_since)
                    .flatten()
            });
        card.activity = activity_for_skill(&card.skill_id, &usage_events, now_unix, active_since);
        if card.governance == GovernanceState::Active && card.availability.is_ready() {
            let invoke_hint = registry
                .and_then(|item| item.routing.as_ref())
                .map(|routing| routing.invoke_hint.clone())
                .filter(|hint| !hint.is_empty())
                .or_else(|| {
                    overlay_entry
                        .map(|entry| entry.invoke_hint.clone())
                        .filter(|hint| !hint.is_empty())
                })
                .unwrap_or_else(|| format!("[skill: {}]", capability.name));
            let mut allowed_entrypoints = card.entrypoints.clone();
            allowed_entrypoints.sort();
            allowed_entrypoints.dedup();
            active_skills.push(ActiveSkill {
                skill_id: card.skill_id.clone(),
                invoke_hint,
                allowed_entrypoints,
                intent_tags: card.intent_tags.clone(),
                legacy_demands: legacy_routes.iter().map(|route| route.demand).collect(),
                source_hash: card.source_hash.clone(),
            });
        }
        catalog.push(card);
    }

    let runtime_hash =
        sha256(format!("{}\n{auth_hash}", inventory_snapshot_hash(&inventory)).as_bytes());
    HostCapabilitySnapshot::new(
        active_host,
        sha256(&registry_bytes),
        overlay_hash,
        runtime_hash,
        catalog,
        active_skills,
    )
    .map_err(SnapshotBuildError::Resolve)
}

struct NoProcessDiscovery;

impl CommandRunner for NoProcessDiscovery {
    fn run(&self, _program: &str, _args: &[&str]) -> CommandOutcome {
        CommandOutcome::Unavailable
    }
}

fn routes_by_skill(routes: &[DemandRoute]) -> HashMap<&str, Vec<DemandRoute>> {
    let mut result: HashMap<&str, Vec<DemandRoute>> = HashMap::new();
    for route in routes {
        result
            .entry(route.skill_id.as_str())
            .or_default()
            .push(route.clone());
    }
    result
}

fn auth_state_for(requires_auth: bool, observed: Option<AuthState>) -> AuthState {
    if !requires_auth {
        AuthState::NotRequired
    } else {
        observed.unwrap_or(AuthState::Unknown)
    }
}

fn skill_card(
    capability: &ManagedCapability,
    registry: Option<&RegistrySkill>,
    overlay: Option<&UserOverlayEntry>,
    legacy_routes: &[DemandRoute],
    auth_state: AuthState,
) -> SkillCard {
    let file_metadata = load_skill_file_metadata(capability);
    let routing = registry.and_then(|item| item.routing.as_ref());
    let retired = routing.is_some_and(|routing| routing.route_state == RouteState::Retired);
    let ignored = overlay.is_some_and(|entry| entry.state == OverlayEntryState::Ignored)
        || capability.managed_status == ManagedStatus::Ignored;
    let routable = routing.is_some_and(|routing| routing.route_state == RouteState::Routable)
        || overlay.is_some_and(|entry| entry.state == OverlayEntryState::Active);
    let registered = capability.registry_status == RegistryStatus::Registered;
    let governance = if retired {
        GovernanceState::Retired
    } else if ignored {
        GovernanceState::Ignored
    } else if routable {
        GovernanceState::Active
    } else if registered {
        GovernanceState::ManagedInactive
    } else if capability.canonical_present {
        GovernanceState::Candidate
    } else {
        GovernanceState::Discovered
    };

    let mut reasons = Vec::new();
    if governance == GovernanceState::Candidate || governance == GovernanceState::Discovered {
        reasons.push("candidate_requires_adoption".to_string());
    }
    if governance == GovernanceState::ManagedInactive {
        reasons.push("registry_not_routable".to_string());
    }
    if retired {
        reasons.push("retired".to_string());
    }
    if !capability.canonical_present {
        reasons.push("canonical_missing".to_string());
    }
    if capability.health_status != HealthStatus::Healthy {
        reasons.push("health_degraded".to_string());
    }
    if !capability
        .host_visibility
        .iter()
        .any(|visibility| visibility.status == HostVisibilityStatus::Visible)
    {
        reasons.push("host_not_visible".to_string());
    }
    if matches!(auth_state, AuthState::Missing | AuthState::Unknown) {
        reasons.push("auth_required".to_string());
    }
    if overlay.is_some_and(|entry| entry.source_hash != source_hash(capability)) {
        reasons.push("source_hash_changed".to_string());
    }

    let declared_summary = registry
        .map(|item| item.description.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .or_else(|| overlay.map(|entry| entry.summary.trim().to_string()))
        .or_else(|| {
            let summary = if file_metadata.summary.trim().is_empty() {
                file_metadata.description.trim()
            } else {
                file_metadata.summary.trim()
            };
            (!summary.is_empty()).then(|| summary.to_string())
        })
        .filter(|summary| !summary.is_empty());
    let summary = declared_summary
        .clone()
        .unwrap_or_else(|| capability.name.clone());
    let mut intent_tags = routing
        .map(|routing| routing.intent_tags.clone())
        .or_else(|| overlay.map(|entry| entry.intent_tags.clone()))
        .unwrap_or_else(|| file_metadata.intent_tags.clone());
    if intent_tags.is_empty() && declared_summary.is_some() {
        intent_tags.push(capability.name.clone());
    }
    for route in legacy_routes {
        intent_tags.push(legacy_demand_tag(route.demand));
    }
    intent_tags.sort();
    intent_tags.dedup();
    let mut entrypoints = legacy_routes
        .iter()
        .filter_map(|route| route.entrypoint.clone())
        .collect::<Vec<_>>();
    if let Some(entry) = overlay {
        entrypoints.extend(entry.entrypoints.clone());
    } else {
        entrypoints.extend(file_metadata.entrypoints.clone());
    }
    entrypoints.sort();
    entrypoints.dedup();
    let invoke_hint_present = routing.is_some_and(|routing| !routing.invoke_hint.trim().is_empty())
        || overlay.is_some_and(|entry| !entry.invoke_hint.trim().is_empty())
        || !file_metadata.invoke_hint.trim().is_empty();
    if declared_summary.is_none() || intent_tags.is_empty() || (routable && !invoke_hint_present) {
        reasons.push("metadata_incomplete".to_string());
    }

    let availability = if governance == GovernanceState::Active && reasons.is_empty() {
        AvailabilityState::Ready
    } else if governance == GovernanceState::Active
        && reasons.iter().all(|reason| reason == "health_degraded")
    {
        AvailabilityState::Degraded {
            reason_codes: reasons.clone(),
        }
    } else {
        AvailabilityState::Unavailable {
            reason_codes: reasons.clone(),
        }
    };
    SkillCard {
        skill_id: capability.name.clone(),
        display_name: overlay
            .map(|entry| entry.display_name.trim().to_string())
            .filter(|display| !display.is_empty())
            .or_else(|| {
                let display = if file_metadata.display_name.trim().is_empty() {
                    file_metadata.name.trim()
                } else {
                    file_metadata.display_name.trim()
                };
                (!display.is_empty()).then(|| display.to_string())
            })
            .unwrap_or_else(|| capability.name.clone()),
        summary,
        intent_tags,
        entrypoints,
        source_kind: source_kind(capability),
        governance,
        availability,
        reason_codes: reasons,
        requires_auth: routing.is_some_and(|routing| routing.requires_auth)
            || overlay.is_some_and(|entry| entry.requires_auth)
            || file_metadata.requires_auth,
        auth_state,
        activity: ActivityState::Unobserved,
        version: overlay
            .map(|entry| entry.metadata_version.clone())
            .filter(|version| !version.is_empty())
            .or_else(|| {
                (!file_metadata.version.trim().is_empty())
                    .then(|| file_metadata.version.trim().to_string())
            })
            .unwrap_or_else(|| "registry".to_string()),
        source_hash: overlay
            .map(|entry| entry.source_hash.clone())
            .filter(|hash| !hash.is_empty())
            .unwrap_or_else(|| source_hash(capability)),
    }
}

fn source_kind(capability: &ManagedCapability) -> SkillSourceKind {
    match capability.managed_status {
        ManagedStatus::SuiteManaged => SkillSourceKind::Suite,
        ManagedStatus::HostSystem => SkillSourceKind::HostSystem,
        ManagedStatus::ProjectLocal => SkillSourceKind::ProjectLocal,
        ManagedStatus::Discovered => {
            if capability
                .source
                .as_deref()
                .is_some_and(|source| source.contains("plugins/cache"))
            {
                SkillSourceKind::EnabledPlugin
            } else {
                SkillSourceKind::UserInstalled
            }
        }
        _ => SkillSourceKind::External,
    }
}

fn source_hash(capability: &ManagedCapability) -> String {
    let Some(source) = capability.source.as_deref() else {
        return sha256(capability.name.as_bytes());
    };
    let path = Path::new(source);
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    let hashed = if path.is_dir() {
        append_source_directory(path, path, &mut canonical)
    } else {
        append_source_node(
            path.parent().unwrap_or_else(|| Path::new(".")),
            path,
            &mut canonical,
        )
    };
    if hashed {
        sha256(&canonical)
    } else {
        sha256(format!("unreadable-skill-source\n{}", capability.name).as_bytes())
    }
}

/// Hash the complete skill body without timestamps or absolute paths. This
/// catches changes in referenced scripts/assets as well as `SKILL.md`. Symlinks
/// are represented by their link target and are never followed, avoiding
/// cycles or accidental traversal outside the skill body.
fn append_source_directory(root: &Path, directory: &Path, canonical: &mut Vec<u8>) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .all(|path| append_source_node(root, path, canonical))
}

fn append_source_node(root: &Path, path: &Path, canonical: &mut Vec<u8>) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if metadata.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(path) else {
            return false;
        };
        canonical.extend_from_slice(b"L\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(target.to_string_lossy().as_bytes());
        canonical.push(0);
        true
    } else if metadata.is_dir() {
        canonical.extend_from_slice(b"D\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        append_source_directory(root, path, canonical)
    } else if metadata.is_file() {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        canonical.extend_from_slice(b"F\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        canonical.extend_from_slice(&bytes);
        true
    } else {
        false
    }
}

fn legacy_demand_tag(demand: SkillDemand) -> String {
    serde_json::to_value(demand)
        .ok()
        .and_then(|value| {
            Some(format!(
                "legacy:{}:{}",
                value.get("category")?.as_str()?,
                value.get("demand")?.as_str()?
            ))
        })
        .unwrap_or_else(|| "legacy:unknown".to_string())
}

#[derive(Debug)]
pub enum SnapshotBuildError {
    Read(std::io::Error),
    Registry(RegistryError),
    Resolve(ResolveError),
    Parse(serde_json::Error),
    Overlay(String),
}

pub fn load_validated_snapshot(
    manifest_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<(HostCapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    load_validated_snapshot_with_roots(manifest_root, runtime_home, active_host, &host_home)
}

/// Hermetic validation seam: compare the persisted snapshot with a freshly
/// rebuilt catalog from the same explicit host roots. A snapshot that is
/// internally self-consistent but no longer matches current skill metadata or
/// bodies is stale, not valid.
pub fn load_validated_snapshot_with_roots(
    manifest_root: &Path,
    runtime_home: &Path,
    active_host: &str,
    host_home: &Path,
) -> Result<(HostCapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let expected =
        build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, host_home)
            .map_err(SnapshotLoadError::Build)?;
    let content = std::fs::read_to_string(snapshot_path(runtime_home, active_host))
        .map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let snapshot: HostCapabilitySnapshot =
        serde_json::from_str(&content).map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let _persisted_table = snapshot
        .validate(
            active_host,
            &expected.registry_hash,
            &expected.overlay_hash,
            &expected.runtime_hash,
        )
        .map_err(SnapshotLoadError::Snapshot)?;
    if snapshot.catalog_hash != expected.catalog_hash
        || snapshot.active_table_hash != expected.active_table_hash
        || snapshot.snapshot_hash != expected.snapshot_hash
    {
        return Err(SnapshotLoadError::Snapshot(
            SnapshotError::SkillSnapshotStale,
        ));
    }
    // Activity is advisory and deliberately excluded from catalog/snapshot
    // hashes. Return the freshly observed in-memory catalog after the persisted
    // authority snapshot passes validation, so Cold/Warm can advance without a
    // snapshot rewrite or lease invalidation.
    let table = ActiveSkillTable::new(
        expected.host.clone(),
        expected.snapshot_hash.clone(),
        expected.active_skills.clone(),
    )
    .map_err(|error| SnapshotLoadError::Snapshot(SnapshotError::InvalidActiveTable(error)))?;
    Ok((expected, table))
}

#[derive(Debug)]
pub enum SnapshotLoadError {
    SkillSnapshotStale,
    Build(SnapshotBuildError),
    Snapshot(SnapshotError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillTagVerdict {
    pub tag: String,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillTagGate {
    pub active_host: String,
    pub snapshot_hash: String,
    pub verdicts: Vec<SkillTagVerdict>,
    pub all_accepted: bool,
    pub rejected: Vec<String>,
}

pub fn verify_skill_tags(tags: &[String], manifest_root: &Path, active_host: &str) -> SkillTagGate {
    verify_skill_tags_with_runtime_home(tags, manifest_root, active_host, &locate_runtime_home())
}

pub fn verify_skill_tags_with_runtime_home(
    tags: &[String],
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> SkillTagGate {
    let host = if active_host.is_empty() {
        "<host-agnostic>"
    } else {
        active_host
    };
    let loaded = load_validated_snapshot(manifest_root, runtime_home, active_host);
    let (snapshot_hash, active_ids, stale) = match loaded {
        Ok((snapshot, table)) => (
            snapshot.snapshot_hash,
            table
                .active_skills()
                .into_iter()
                .map(|skill| skill.skill_id)
                .collect::<HashSet<_>>(),
            false,
        ),
        Err(_) => (String::new(), HashSet::new(), true),
    };
    let verdicts = tags
        .iter()
        .map(|tag| {
            let accepted = !stale && active_ids.contains(tag);
            SkillTagVerdict {
                tag: tag.clone(),
                accepted,
                reason: if accepted {
                    String::new()
                } else if stale {
                    "skill_snapshot_stale; run `ags capability snapshot --write`".to_string()
                } else {
                    format!("`[skill: {tag}]` is not active for host '{host}'")
                },
            }
        })
        .collect::<Vec<_>>();
    let rejected = verdicts
        .iter()
        .filter(|verdict| !verdict.accepted)
        .map(|verdict| verdict.tag.clone())
        .collect::<Vec<_>>();
    SkillTagGate {
        active_host: host.to_string(),
        snapshot_hash,
        all_accepted: rejected.is_empty(),
        rejected,
        verdicts,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillOutcome {
    Succeeded,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillUsageEvent {
    pub schema_version: String,
    pub event_id: String,
    pub timestamp_unix: u64,
    pub request_fingerprint: String,
    pub proposal_id: String,
    pub decision_id: String,
    pub lease_id: String,
    pub skill_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub outcome: SkillOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
}

pub fn append_usage_event(
    runtime_home: &Path,
    active_host: &str,
    event: &SkillUsageEvent,
) -> Result<PathBuf, String> {
    validate_usage_event(event)?;
    let path = usage_path(runtime_home, active_host);
    let parent = path
        .parent()
        .ok_or_else(|| "usage ledger has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let line = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("cannot append {}: {error}", path.display()))?;
    set_private_permissions(&path)?;
    Ok(path)
}

pub fn load_usage_events(runtime_home: &Path, active_host: &str) -> Vec<SkillUsageEvent> {
    let Ok(content) = std::fs::read_to_string(usage_path(runtime_home, active_host)) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn activity_for_skill(
    skill_id: &str,
    events: &[SkillUsageEvent],
    now_unix: u64,
    active_since_unix: Option<u64>,
) -> ActivityState {
    let last = events
        .iter()
        .filter(|event| event.skill_id == skill_id)
        .map(|event| event.timestamp_unix)
        .max();
    match last {
        Some(timestamp) if now_unix.saturating_sub(timestamp) > 90 * 86_400 => ActivityState::Cold,
        Some(_) => ActivityState::Warm,
        None if active_since_unix
            .is_some_and(|since| now_unix.saturating_sub(since) > 30 * 86_400) =>
        {
            ActivityState::Cold
        }
        None => ActivityState::Unobserved,
    }
}

fn validate_usage_event(event: &SkillUsageEvent) -> Result<(), String> {
    if event.schema_version != SKILL_USAGE_EVENT_SCHEMA_VERSION {
        return Err("invalid skill usage event schema".to_string());
    }
    if event.quality.is_some_and(|quality| quality > 100) {
        return Err("quality must be in 0..=100".to_string());
    }
    for (field, value) in [
        ("event_id", event.event_id.as_str()),
        ("request_fingerprint", event.request_fingerprint.as_str()),
        ("proposal_id", event.proposal_id.as_str()),
        ("decision_id", event.decision_id.as_str()),
        ("lease_id", event.lease_id.as_str()),
        ("skill_id", event.skill_id.as_str()),
    ] {
        validate_usage_identifier(field, value)?;
    }
    if let Some(entrypoint) = event.entrypoint.as_deref() {
        validate_usage_identifier("entrypoint", entrypoint)?;
    }
    let serialized = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let forbidden = [
        "raw_prompt",
        "credential",
        "secret",
        "token",
        "/Users/",
        "/home/",
    ];
    if forbidden.iter().any(|needle| serialized.contains(needle)) {
        return Err("skill usage event contains forbidden sensitive/path material".to_string());
    }
    Ok(())
}

fn validate_usage_identifier(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(format!(
            "{field} must be a non-path, non-whitespace identifier of at most 256 bytes"
        ));
    }
    Ok(())
}

fn set_private_permissions(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot chmod {}: {error}", _path.display()))?;
    }
    Ok(())
}

fn sort_active_skills(skills: &mut [ActiveSkill]) {
    skills.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

fn sort_skill_cards(cards: &mut [SkillCard]) {
    cards.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

fn active_table_hash(active_skills: &[ActiveSkill]) -> String {
    let mut canonical = active_skills.to_vec();
    sort_active_skills(&mut canonical);
    sha256(&serde_json::to_vec(&canonical).unwrap_or_default())
}

fn catalog_hash(catalog: &[SkillCard]) -> String {
    let mut canonical = catalog.to_vec();
    for card in &mut canonical {
        card.activity = ActivityState::Unobserved;
    }
    sort_skill_cards(&mut canonical);
    sha256(&serde_json::to_vec(&canonical).unwrap_or_default())
}

fn snapshot_integrity_hash(snapshot: &HostCapabilitySnapshot) -> String {
    sha256(
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            snapshot.schema_version,
            snapshot.host,
            snapshot.registry_hash,
            snapshot.overlay_hash,
            snapshot.runtime_hash,
            snapshot.catalog_hash,
            snapshot.active_table_hash
        )
        .as_bytes(),
    )
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_skill() -> ActiveSkill {
        ActiveSkill {
            skill_id: "codebase-design".to_string(),
            invoke_hint: "[skill: codebase-design]".to_string(),
            allowed_entrypoints: vec!["module-design".to_string()],
            intent_tags: vec!["module-design".to_string()],
            legacy_demands: Vec::new(),
            source_hash: "sha256:source".to_string(),
        }
    }

    fn card() -> SkillCard {
        SkillCard {
            skill_id: "codebase-design".to_string(),
            display_name: "Codebase Design".to_string(),
            summary: "Deep module design".to_string(),
            intent_tags: vec!["module-design".to_string()],
            entrypoints: vec!["module-design".to_string()],
            source_kind: SkillSourceKind::Suite,
            governance: GovernanceState::Active,
            availability: AvailabilityState::Ready,
            reason_codes: Vec::new(),
            requires_auth: false,
            auth_state: AuthState::NotRequired,
            activity: ActivityState::Unobserved,
            version: "registry".to_string(),
            source_hash: "sha256:source".to_string(),
        }
    }

    #[test]
    fn exact_skill_and_entrypoint_resolution() {
        let table =
            ActiveSkillTable::new("codex", "sha256:snapshot", vec![active_skill()]).unwrap();
        let selection = resolve_skill(
            "codebase-design",
            Some("module-design"),
            "sha256:snapshot",
            &table,
        )
        .unwrap();
        assert_eq!(selection.skill_id, "codebase-design");
        assert_eq!(selection.snapshot_hash, "sha256:snapshot");
    }

    #[test]
    fn exact_skill_resolution_rejects_a_different_snapshot_hash() {
        let table =
            ActiveSkillTable::new("codex", "sha256:expected", vec![active_skill()]).unwrap();
        assert!(matches!(
            resolve_skill("codebase-design", None, "sha256:stale", &table),
            Err(ResolveError::SnapshotHashMismatch { .. })
        ));
    }

    #[test]
    fn entrypoint_fails_closed_without_fallback() {
        let table =
            ActiveSkillTable::new("codex", "sha256:snapshot", vec![active_skill()]).unwrap();
        assert!(matches!(
            resolve_skill(
                "codebase-design",
                Some("brainstorming"),
                "sha256:snapshot",
                &table
            ),
            Err(ResolveError::EntrypointNotAllowed { .. })
        ));
    }

    #[test]
    fn snapshot_hash_is_deterministic_and_binds_catalog() {
        let one = HostCapabilitySnapshot::new(
            "codex",
            "sha256:registry",
            "sha256:overlay",
            "sha256:runtime",
            vec![card()],
            vec![active_skill()],
        )
        .unwrap();
        let two = HostCapabilitySnapshot::new(
            "codex",
            "sha256:registry",
            "sha256:overlay",
            "sha256:runtime",
            vec![card()],
            vec![active_skill()],
        )
        .unwrap();
        assert_eq!(one.snapshot_hash, two.snapshot_hash);
        assert!(one
            .validate(
                "codex",
                "sha256:registry",
                "sha256:overlay",
                "sha256:runtime"
            )
            .is_ok());
    }

    #[test]
    fn snapshot_deserialization_rejects_unknown_top_level_and_nested_fields() {
        let snapshot = HostCapabilitySnapshot::new(
            "codex",
            "sha256:registry",
            "sha256:overlay",
            "sha256:runtime",
            vec![card()],
            vec![active_skill()],
        )
        .unwrap();
        let mut top = serde_json::to_value(&snapshot).unwrap();
        top["raw_prompt"] = serde_json::json!("must not be ignored");
        assert!(serde_json::from_value::<HostCapabilitySnapshot>(top).is_err());

        let mut nested = serde_json::to_value(snapshot).unwrap();
        nested["catalog"][0]["raw_prompt"] = serde_json::json!("must not be ignored");
        assert!(serde_json::from_value::<HostCapabilitySnapshot>(nested).is_err());
    }

    #[test]
    fn activity_thresholds_are_advisory_only() {
        let now = 100 * 86_400;
        assert_eq!(
            activity_for_skill("x", &[], now, Some(60 * 86_400)),
            ActivityState::Cold
        );
        assert_eq!(
            activity_for_skill("x", &[], now, Some(90 * 86_400)),
            ActivityState::Unobserved
        );
    }

    #[test]
    fn activity_does_not_change_catalog_or_snapshot_hash() {
        let mut cold_card = card();
        cold_card.activity = ActivityState::Cold;
        let cold = HostCapabilitySnapshot::new(
            "codex",
            "sha256:registry",
            "sha256:overlay",
            "sha256:runtime",
            vec![cold_card],
            vec![active_skill()],
        )
        .unwrap();
        let warm = HostCapabilitySnapshot::new(
            "codex",
            "sha256:registry",
            "sha256:overlay",
            "sha256:runtime",
            vec![card()],
            vec![active_skill()],
        )
        .unwrap();
        assert_eq!(cold.catalog_hash, warm.catalog_hash);
        assert_eq!(cold.snapshot_hash, warm.snapshot_hash);
    }

    #[test]
    fn reason_code_contract_covers_all_governance_failures() {
        for required in [
            "candidate_requires_adoption",
            "registry_not_routable",
            "retired",
            "canonical_missing",
            "host_not_visible",
            "health_degraded",
            "auth_required",
            "metadata_incomplete",
            "snapshot_stale",
        ] {
            assert!(SKILL_REASON_CODES.contains(&required), "missing {required}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_overlay_adopt_ignore_and_rollback_are_versioned_and_private() {
        use std::os::unix::fs::PermissionsExt;

        let base =
            std::env::temp_dir().join(format!("ags-overlay-lifecycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let runtime = base.join("runtime");
        let home = base.join("home");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let skill_id = "machine-private-demo";
        let body = home.join(".agents/skills").join(skill_id);
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(
            body.join("SKILL.md"),
            "---\nname: machine-private-demo\ndescription: A private test skill.\nintent_tags: [private-demo]\n---\nbody\n",
        )
        .unwrap();

        let dry_run = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Adopt,
            None,
            false,
        )
        .unwrap();
        assert!(dry_run.dry_run && dry_run.changed && !dry_run.applied);
        assert!(!overlay_path(&runtime).exists());

        let adopted = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Adopt,
            None,
            true,
        )
        .unwrap();
        assert_eq!(adopted.overlay_revision, 1);
        assert_eq!(
            std::fs::metadata(overlay_path(&runtime))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(overlay_events_path(&runtime))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        // An adopted entry remains manageable even when its downloaded body
        // later disappears; ignore uses the versioned overlay metadata.
        std::fs::remove_dir_all(&body).unwrap();

        let ignored = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Ignore,
            None,
            true,
        )
        .unwrap();
        assert_eq!(ignored.overlay_revision, 2);
        assert_eq!(
            load_user_overlay(&runtime).unwrap().entries[0].state,
            OverlayEntryState::Ignored
        );

        let rolled_back = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Rollback,
            Some(1),
            true,
        )
        .unwrap();
        assert_eq!(rolled_back.overlay_revision, 3);
        let overlay = load_user_overlay(&runtime).unwrap();
        assert_eq!(overlay.entries[0].state, OverlayEntryState::Active);
        assert_eq!(overlay.entries[0].revision, 3);
        assert_eq!(load_overlay_mutation_receipts(&runtime).unwrap().len(), 3);

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn private_overlay_cannot_shadow_official_registry() {
        let base = std::env::temp_dir().join(format!(
            "ags-overlay-official-precedence-{}",
            std::process::id()
        ));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let error = mutate_user_overlay(
            &root,
            &base.join("runtime"),
            &base.join("home"),
            "codex",
            "diagnosing-bugs",
            OverlayMutationOperation::Adopt,
            None,
            true,
        )
        .unwrap_err();
        assert_eq!(error, "official_registry_precedence");
        assert!(!overlay_path(&base.join("runtime")).exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn unreadable_snapshot_backup_blocks_before_mutating_previous_revision() {
        let base = std::env::temp_dir().join(format!(
            "ags-overlay-snapshot-rollback-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime = base.join("runtime");
        let home = base.join("home");
        let skill_id = "snapshot-rollback-demo";
        let body = home.join(".agents/skills").join(skill_id);
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(
            body.join("SKILL.md"),
            "---\nname: snapshot-rollback-demo\ndescription: rollback test\nintent_tags: [rollback]\n---\n",
        )
        .unwrap();
        mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Adopt,
            None,
            true,
        )
        .unwrap();
        let overlay_before = std::fs::read(overlay_path(&runtime)).unwrap();
        let receipts_before = std::fs::read(overlay_events_path(&runtime)).unwrap();
        let saved_snapshot = snapshot_path(&runtime, "codex");
        std::fs::remove_file(&saved_snapshot).unwrap();
        std::fs::create_dir(&saved_snapshot).unwrap();

        let error = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            skill_id,
            OverlayMutationOperation::Ignore,
            None,
            true,
        )
        .unwrap_err();
        assert!(error.contains("cannot read existing private file"));
        assert_eq!(
            std::fs::read(overlay_path(&runtime)).unwrap(),
            overlay_before
        );
        assert_eq!(
            std::fs::read(overlay_events_path(&runtime)).unwrap(),
            receipts_before
        );
        assert!(saved_snapshot.is_dir());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn receipt_sync_failure_atomically_restores_overlay_and_snapshot() {
        let base =
            std::env::temp_dir().join(format!("ags-overlay-receipt-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime = base.join("runtime");
        let home = base.join("home");
        for skill_id in ["receipt-atomic-one", "receipt-atomic-two"] {
            let body = home.join(".agents/skills").join(skill_id);
            std::fs::create_dir_all(&body).unwrap();
            std::fs::write(
                body.join("SKILL.md"),
                format!(
                    "---\nname: {skill_id}\ndescription: receipt atomic test\nintent_tags: [atomic]\n---\n"
                ),
            )
            .unwrap();
        }
        mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            "receipt-atomic-one",
            OverlayMutationOperation::Adopt,
            None,
            true,
        )
        .unwrap();
        let overlay_before = std::fs::read(overlay_path(&runtime)).unwrap();
        let snapshot_before = std::fs::read(snapshot_path(&runtime, "codex")).unwrap();
        let receipts_before = std::fs::read(overlay_events_path(&runtime)).unwrap();

        inject_private_sync_failure(Some("user-overlay-events.ndjson"));
        let error = mutate_user_overlay(
            &root,
            &runtime,
            &home,
            "codex",
            "receipt-atomic-two",
            OverlayMutationOperation::Adopt,
            None,
            true,
        )
        .unwrap_err();
        inject_private_sync_failure(None);
        assert!(error.contains("injected sync failure"));
        assert_eq!(
            std::fs::read(overlay_path(&runtime)).unwrap(),
            overlay_before
        );
        assert_eq!(
            std::fs::read(snapshot_path(&runtime, "codex")).unwrap(),
            snapshot_before
        );
        assert_eq!(
            std::fs::read(overlay_events_path(&runtime)).unwrap(),
            receipts_before
        );
        assert_eq!(load_overlay_mutation_receipts(&runtime).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_is_permissioned_before_the_final_rename() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!(
            "ags-private-stage-final-rename-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let stage = base.join("stage");
        let destination = base.join("destination");
        std::fs::write(&stage, b"new").unwrap();
        std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&destination, b"old").unwrap();

        commit_private_stage(&stage, &destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!stage.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn usage_event_rejects_absolute_or_prompt_like_identifier_material() {
        let event = SkillUsageEvent {
            schema_version: SKILL_USAGE_EVENT_SCHEMA_VERSION.to_string(),
            event_id: "event-1".to_string(),
            timestamp_unix: 1,
            request_fingerprint: "sha256:fingerprint".to_string(),
            proposal_id: "proposal-1".to_string(),
            decision_id: "decision-1".to_string(),
            lease_id: "lease-1".to_string(),
            skill_id: "/Volumes/private/skill".to_string(),
            entrypoint: None,
            outcome: SkillOutcome::Failed,
            quality: None,
        };
        assert!(validate_usage_event(&event).is_err());

        let mut prompt_like = event;
        prompt_like.skill_id = "safe-skill".to_string();
        prompt_like.request_fingerprint = "please reveal data".to_string();
        assert!(validate_usage_event(&prompt_like).is_err());
    }
}
