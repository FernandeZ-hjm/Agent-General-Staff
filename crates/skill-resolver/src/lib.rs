//! Deterministic skill resolution after the requirement router.
//!
//! This module has no natural-language input. It resolves one closed
//! [`request_router::SkillDemand`] against a machine-local
//! [`ActiveSkillTable`]. Missing or stale state is a governance precondition
//! failure; the resolver never substitutes a similar skill.

use request_router::SkillDemand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skill_governance::console::{
    build_inventory, inventory_snapshot_hash, ConsoleContext, HealthStatus, HostVisibilityStatus,
    ManagedCapability, ManagedKind, RouteState,
};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub const CAPABILITY_SNAPSHOT_SCHEMA_VERSION: &str = "0.2.8-capability-snapshot";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSkill {
    pub demand: SkillDemand,
    pub skill_id: String,
    pub invoke_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
}

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

pub fn locate_manifest_root(start: &Path) -> PathBuf {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    loop {
        if current.join("manifests/skills-registry.yaml").is_file() {
            return current.to_path_buf();
        }
        let Some(parent) = current.parent() else {
            return start.to_path_buf();
        };
        current = parent;
    }
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

pub fn snapshot_path(runtime_home: &Path) -> PathBuf {
    runtime_home
        .join("capability-snapshot")
        .join("capability-snapshot.json")
}

pub fn load_demand_routes(root: &Path) -> Result<Vec<DemandRoute>, RegistryError> {
    #[derive(Deserialize)]
    struct RegistryDocument {
        #[serde(default)]
        demand_routes: Vec<DemandRoute>,
    }

    let content = std::fs::read_to_string(root.join("manifests/skills-registry.yaml"))
        .map_err(RegistryError::Read)?;
    let document: RegistryDocument =
        serde_yaml::from_str(&content).map_err(RegistryError::Parse)?;
    Ok(document.demand_routes)
}

/// Build the strict runtime intersection used for routing. Inactive mappings
/// are omitted; consumers treat a missing requested demand as a governance
/// precondition failure rather than selecting a fallback.
pub fn build_active_skills(
    active_host: &str,
    routes: &[DemandRoute],
    capabilities: &[ManagedCapability],
) -> Result<Vec<ActiveSkill>, ResolveError> {
    let mut active = Vec::new();
    for route in routes {
        let Some(capability) = capabilities.iter().find(|capability| {
            capability.name == route.skill_id
                && capability.kind == ManagedKind::Skill
                && capability.canonical_present
                && capability.health_status == HealthStatus::Healthy
                && capability
                    .routing
                    .as_ref()
                    .is_some_and(|routing| routing.route_state == RouteState::Routable)
                && capability.host_visibility.iter().any(|visibility| {
                    visibility.host == active_host
                        && visibility.status == HostVisibilityStatus::Visible
                })
        }) else {
            continue;
        };
        let routing = capability.routing.as_ref().expect("checked above");
        active.push(ActiveSkill {
            demand: route.demand,
            skill_id: capability.name.clone(),
            invoke_hint: routing.invoke_hint.clone(),
            entrypoint: route.entrypoint.clone(),
        });
    }
    ActiveSkillTable::new(active_host, active).map(|table| table.active_skills())
}

#[derive(Debug, Clone)]
pub struct ActiveSkillTable {
    pub active_host: String,
    skills: HashMap<SkillDemand, ActiveSkill>,
}

impl ActiveSkillTable {
    pub fn new(
        active_host: impl Into<String>,
        active_skills: Vec<ActiveSkill>,
    ) -> Result<Self, ResolveError> {
        let mut skills = HashMap::with_capacity(active_skills.len());
        for skill in active_skills {
            let demand = skill.demand;
            if let Some(first) = skills.insert(demand, skill.clone()) {
                return Err(ResolveError::DuplicateDemand {
                    demand,
                    first_skill: first.skill_id,
                    second_skill: skill.skill_id,
                });
            }
        }
        Ok(Self {
            active_host: active_host.into(),
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
    pub demand: SkillDemand,
    pub skill_id: String,
    pub invoke_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    GovernancePrecondition(&'static str),
    DuplicateDemand {
        demand: SkillDemand,
        first_skill: String,
        second_skill: String,
    },
}

pub fn resolve_skill(
    demand: SkillDemand,
    table: &ActiveSkillTable,
) -> Result<SkillSelection, ResolveError> {
    let active = table
        .skills
        .get(&demand)
        .ok_or(ResolveError::GovernancePrecondition("skill_demand_missing"))?;
    Ok(SkillSelection {
        demand,
        skill_id: active.skill_id.clone(),
        invoke_hint: active.invoke_hint.clone(),
        entrypoint: active.entrypoint.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub schema_version: String,
    pub active_host: String,
    pub registry_hash: String,
    pub runtime_hash: String,
    pub active_table_hash: String,
    pub snapshot_hash: String,
    pub active_skills: Vec<ActiveSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    SkillSnapshotStale,
    SnapshotIntegrityFailed,
    InvalidActiveTable(ResolveError),
}

impl CapabilitySnapshot {
    pub fn new(
        active_host: impl Into<String>,
        registry_hash: impl Into<String>,
        runtime_hash: impl Into<String>,
        mut active_skills: Vec<ActiveSkill>,
    ) -> Result<Self, ResolveError> {
        let active_host = active_host.into();
        let table = ActiveSkillTable::new(active_host.clone(), active_skills)?;
        active_skills = table.active_skills();
        let active_table_hash = active_table_hash(&active_skills);
        let mut snapshot = Self {
            schema_version: CAPABILITY_SNAPSHOT_SCHEMA_VERSION.to_string(),
            active_host,
            registry_hash: registry_hash.into(),
            runtime_hash: runtime_hash.into(),
            active_table_hash,
            snapshot_hash: String::new(),
            active_skills,
        };
        snapshot.snapshot_hash = snapshot_integrity_hash(&snapshot);
        Ok(snapshot)
    }

    pub fn validate(
        &self,
        expected_host: &str,
        expected_registry_hash: &str,
        expected_runtime_hash: &str,
    ) -> Result<ActiveSkillTable, SnapshotError> {
        if self.schema_version != CAPABILITY_SNAPSHOT_SCHEMA_VERSION
            || self.active_host != expected_host
            || self.registry_hash != expected_registry_hash
            || self.runtime_hash != expected_runtime_hash
        {
            return Err(SnapshotError::SkillSnapshotStale);
        }
        if self.active_table_hash != active_table_hash(&self.active_skills)
            || self.snapshot_hash != snapshot_integrity_hash(self)
        {
            return Err(SnapshotError::SnapshotIntegrityFailed);
        }
        ActiveSkillTable::new(self.active_host.clone(), self.active_skills.clone())
            .map_err(SnapshotError::InvalidActiveTable)
    }
}

pub fn build_capability_snapshot(
    manifest_root: &Path,
    active_host: &str,
) -> Result<CapabilitySnapshot, SnapshotBuildError> {
    let context = ConsoleContext::system(manifest_root.to_path_buf());
    let inventory = build_inventory(&context, &[active_host]);
    let routes = load_demand_routes(manifest_root).map_err(SnapshotBuildError::Registry)?;
    let active_skills = build_active_skills(active_host, &routes, &inventory.capabilities)
        .map_err(SnapshotBuildError::Resolve)?;
    let registry = std::fs::read(manifest_root.join("manifests/skills-registry.yaml"))
        .map_err(SnapshotBuildError::Read)?;
    CapabilitySnapshot::new(
        active_host,
        sha256(&registry),
        inventory_snapshot_hash(&inventory),
        active_skills,
    )
    .map_err(SnapshotBuildError::Resolve)
}

#[derive(Debug)]
pub enum SnapshotBuildError {
    Read(std::io::Error),
    Registry(RegistryError),
    Resolve(ResolveError),
    Parse(serde_json::Error),
}

pub fn load_validated_snapshot(
    manifest_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<(CapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let expected =
        build_capability_snapshot(manifest_root, active_host).map_err(SnapshotLoadError::Build)?;
    let content = std::fs::read_to_string(snapshot_path(runtime_home))
        .map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let snapshot: CapabilitySnapshot =
        serde_json::from_str(&content).map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let table = snapshot
        .validate(active_host, &expected.registry_hash, &expected.runtime_hash)
        .map_err(SnapshotLoadError::Snapshot)?;
    Ok((snapshot, table))
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
                .collect::<std::collections::HashSet<_>>(),
            false,
        ),
        Err(_) => (String::new(), std::collections::HashSet::new(), true),
    };
    let verdicts: Vec<_> = tags
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
        .collect();
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

fn sort_active_skills(skills: &mut [ActiveSkill]) {
    skills.sort_by(|left, right| {
        let left = serde_json::to_string(&left.demand).unwrap_or_default();
        let right = serde_json::to_string(&right.demand).unwrap_or_default();
        left.cmp(&right).then_with(|| left.len().cmp(&right.len()))
    });
}

fn active_table_hash(active_skills: &[ActiveSkill]) -> String {
    let mut canonical = active_skills.to_vec();
    sort_active_skills(&mut canonical);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    sha256(&bytes)
}

fn snapshot_integrity_hash(snapshot: &CapabilitySnapshot) -> String {
    let basis = format!(
        "{}\n{}\n{}\n{}\n{}",
        snapshot.schema_version,
        snapshot.active_host,
        snapshot.registry_hash,
        snapshot.runtime_hash,
        snapshot.active_table_hash
    );
    sha256(basis.as_bytes())
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}
