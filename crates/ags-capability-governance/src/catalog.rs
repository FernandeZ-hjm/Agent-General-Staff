#[allow(unused_imports)]
use super::hashing::*;
use super::*;
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
    /// Host-facing examples for semantic target selection. AGS never evaluates
    /// them; they are hashed catalog evidence for the host's single NL pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
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
pub struct ThirdPartyCapabilityCard {
    pub capability_id: String,
    pub kind: String,
    pub display_name: String,
    pub purpose: String,
    pub profiles: Vec<String>,
    pub required: bool,
    pub route_state: String,
    pub availability: AvailabilityState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
    pub requires_auth: bool,
    pub auth_state: AuthState,
    pub health_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoke_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
    pub routing_surface: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hook_events: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
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
    pub third_party_registry_url: String,
    pub third_party_manifest_hash: String,
    pub third_party_catalog: Vec<ThirdPartyCapabilityCard>,
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
        third_party_registry_url: impl Into<String>,
        third_party_manifest_hash: impl Into<String>,
        mut third_party_catalog: Vec<ThirdPartyCapabilityCard>,
        mut active_skills: Vec<ActiveSkill>,
    ) -> Result<Self, ResolveError> {
        let host = host.into();
        sort_skill_cards(&mut catalog);
        sort_third_party_cards(&mut third_party_catalog);
        let table = ActiveSkillTable::new(host.clone(), "pending", active_skills)?;
        active_skills = table.active_skills();
        let catalog_hash = catalog_hash(&catalog, &third_party_catalog);
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
            third_party_registry_url: third_party_registry_url.into(),
            third_party_manifest_hash: third_party_manifest_hash.into(),
            third_party_catalog,
            active_skills,
        };
        snapshot.snapshot_hash = snapshot_integrity_hash(&snapshot);
        Ok(snapshot)
    }

    /// Validate only facts sealed inside the persisted snapshot.
    ///
    /// Runtime request paths use this check. They deliberately do not rebuild
    /// the catalog from PATH, auth, host visibility, or mutable skill roots.
    /// Those observations are sampled only by an explicit snapshot refresh.
    pub fn validate_integrity(
        &self,
        expected_host: &str,
    ) -> Result<ActiveSkillTable, SnapshotError> {
        if self.schema_version != HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION
            || self.host != expected_host
        {
            return Err(SnapshotError::SkillSnapshotStale);
        }
        if self.catalog_hash != catalog_hash(&self.catalog, &self.third_party_catalog)
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
