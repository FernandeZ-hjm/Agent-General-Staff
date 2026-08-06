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

/// The only host-facing route surface accepted for one catalog row.
///
/// A host command may be installed as a foreground skill for discoverability,
/// but it is never an AGS `SkillTarget`. Keeping the distinction in the sealed
/// snapshot prevents hosts from inferring routability from installation alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillRoutingSurface {
    SkillTarget,
    HostCommand,
    #[default]
    NotRoutable,
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
    #[serde(default)]
    pub routing_surface: SkillRoutingSurface,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_hint: Option<String>,
    pub source_kind: SkillSourceKind,
    pub governance: GovernanceState,
    pub availability: AvailabilityState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
    pub requires_auth: bool,
    pub auth_state: AuthState,
    pub version: String,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCard {
    pub mcp_id: String,
    pub display_name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    pub invoke_hint: String,
    pub route_state: String,
    pub mutation_surface: String,
    pub availability: AvailabilityState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
    pub requires_auth: bool,
    pub auth_state: AuthState,
    pub health_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyCapabilityCard {
    pub capability_id: String,
    pub kind: String,
    /// Catalog entries are discovery/review facts. They never enter the active
    /// route table by themselves.
    pub catalog_state: String,
    /// For Skills this is derived only from InstalledSkillRecord projection;
    /// conventional-path bodies and suite-owned bodies cannot satisfy it.
    pub installation_state: String,
    pub display_name: String,
    pub purpose: String,
    pub profiles: Vec<String>,
    pub required: bool,
    pub route_state: String,
    /// Clarifies that `route_state` describes the contract after successful
    /// installation and activation, not the current machine route state.
    pub route_state_semantics: String,
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
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveMcp {
    pub mcp_id: String,
    pub invoke_hint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    pub mutation_surface: String,
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

#[derive(Debug, Clone)]
pub struct ActiveMcpTable {
    pub active_host: String,
    snapshot_hash: String,
    mcps: HashMap<String, ActiveMcp>,
}

impl ActiveMcpTable {
    pub fn new(
        active_host: impl Into<String>,
        snapshot_hash: impl Into<String>,
        active_mcps: Vec<ActiveMcp>,
    ) -> Result<Self, ResolveError> {
        let mut mcps = HashMap::with_capacity(active_mcps.len());
        for mcp in active_mcps {
            let mcp_id = mcp.mcp_id.clone();
            if mcps.insert(mcp_id.clone(), mcp).is_some() {
                return Err(ResolveError::DuplicateMcp { mcp_id });
            }
        }
        Ok(Self {
            active_host: active_host.into(),
            snapshot_hash: snapshot_hash.into(),
            mcps,
        })
    }

    pub fn active_mcps(&self) -> Vec<ActiveMcp> {
        let mut mcps: Vec<_> = self.mcps.values().cloned().collect();
        sort_active_mcps(&mut mcps);
        mcps
    }
}

#[derive(Debug, Clone)]
pub struct ActiveCapabilityTables {
    pub skills: ActiveSkillTable,
    pub mcps: ActiveMcpTable,
}

impl ActiveCapabilityTables {
    pub fn active_skills(&self) -> Vec<ActiveSkill> {
        self.skills.active_skills()
    }

    pub fn active_mcps(&self) -> Vec<ActiveMcp> {
        self.mcps.active_mcps()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSelection {
    pub mcp_id: String,
    pub invoke_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub mutation_surface: String,
    pub snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    GovernancePrecondition(&'static str),
    DuplicateSkill {
        skill_id: String,
    },
    DuplicateMcp {
        mcp_id: String,
    },
    EntrypointNotAllowed {
        skill_id: String,
        entrypoint: String,
    },
    ToolNotAllowed {
        mcp_id: String,
        tool: String,
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

pub fn resolve_mcp(
    mcp_id: &str,
    tool: Option<&str>,
    snapshot_hash: &str,
    table: &ActiveMcpTable,
) -> Result<McpSelection, ResolveError> {
    if snapshot_hash != table.snapshot_hash {
        return Err(ResolveError::SnapshotHashMismatch {
            expected: table.snapshot_hash.clone(),
            supplied: snapshot_hash.to_string(),
        });
    }
    let active = table
        .mcps
        .get(mcp_id)
        .ok_or(ResolveError::GovernancePrecondition("mcp_not_active"))?;
    if let Some(tool) = tool {
        if !active.allowed_tools.iter().any(|allowed| allowed == tool) {
            return Err(ResolveError::ToolNotAllowed {
                mcp_id: mcp_id.to_string(),
                tool: tool.to_string(),
            });
        }
    }
    Ok(McpSelection {
        mcp_id: active.mcp_id.clone(),
        invoke_hint: active.invoke_hint.clone(),
        tool: tool.map(str::to_string),
        mutation_surface: active.mutation_surface.clone(),
        snapshot_hash: snapshot_hash.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilitySnapshot {
    pub schema_version: String,
    pub host: String,
    pub registry_hash: String,
    pub runtime_hash: String,
    pub snapshot_hash: String,
    pub catalog: Vec<SkillCard>,
    pub mcp_catalog: Vec<McpCard>,
    pub third_party_registry_url: String,
    pub third_party_manifest_hash: String,
    pub third_party_catalog: Vec<ThirdPartyCapabilityCard>,
    pub active_skills: Vec<ActiveSkill>,
    pub active_mcps: Vec<ActiveMcp>,
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
        runtime_hash: impl Into<String>,
        mut catalog: Vec<SkillCard>,
        mut mcp_catalog: Vec<McpCard>,
        third_party_registry_url: impl Into<String>,
        third_party_manifest_hash: impl Into<String>,
        mut third_party_catalog: Vec<ThirdPartyCapabilityCard>,
        mut active_skills: Vec<ActiveSkill>,
        mut active_mcps: Vec<ActiveMcp>,
    ) -> Result<Self, ResolveError> {
        let host = host.into();
        sort_skill_cards(&mut catalog);
        sort_mcp_cards(&mut mcp_catalog);
        sort_third_party_cards(&mut third_party_catalog);
        let table = ActiveSkillTable::new(host.clone(), "pending", active_skills)?;
        active_skills = table.active_skills();
        let table = ActiveMcpTable::new(host.clone(), "pending", active_mcps)?;
        active_mcps = table.active_mcps();
        let mut snapshot = Self {
            schema_version: HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION.to_string(),
            host,
            registry_hash: registry_hash.into(),
            runtime_hash: runtime_hash.into(),
            snapshot_hash: String::new(),
            catalog,
            mcp_catalog,
            third_party_registry_url: third_party_registry_url.into(),
            third_party_manifest_hash: third_party_manifest_hash.into(),
            third_party_catalog,
            active_skills,
            active_mcps,
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
    ) -> Result<ActiveCapabilityTables, SnapshotError> {
        if self.schema_version != HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION
            || self.host != expected_host
        {
            return Err(SnapshotError::SkillSnapshotStale);
        }
        if self.snapshot_hash != snapshot_integrity_hash(self) {
            return Err(SnapshotError::SnapshotIntegrityFailed);
        }
        let skills = ActiveSkillTable::new(
            self.host.clone(),
            self.snapshot_hash.clone(),
            self.active_skills.clone(),
        )
        .map_err(SnapshotError::InvalidActiveTable)?;
        let mcps = ActiveMcpTable::new(
            self.host.clone(),
            self.snapshot_hash.clone(),
            self.active_mcps.clone(),
        )
        .map_err(SnapshotError::InvalidActiveTable)?;
        Ok(ActiveCapabilityTables { skills, mcps })
    }
}
