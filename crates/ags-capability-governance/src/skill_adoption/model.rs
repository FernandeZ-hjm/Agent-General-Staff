use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const INSTALLED_SKILL_INDEX_SCHEMA: &str = "ags://schema/contract/v2/installed-skill-index";

#[derive(Debug, Clone)]
pub struct AdoptionContext {
    pub authority_root: PathBuf,
    pub runtime_home: PathBuf,
    /// Disposable authority used to observe remote candidates while sealing
    /// a plan. Production never points this at the installed runtime.
    pub candidate_home: PathBuf,
    pub host_home: PathBuf,
    pub snapshot_discovery: SnapshotDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotDiscovery {
    Live,
    Offline,
}

/// The user-selected identity of a Skill source.
///
/// `Local` is intentionally retained as a first-class value.  A local source
/// has no upstream identity merely because its path contains a Git checkout;
/// only a `GitHub`/`Git` source can produce an update candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSpec {
    Local {
        path: String,
    },
    GitHub {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracking_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
    /// A generic Git URL is kept for hermetic local/file-backed test seams.
    /// User-facing GitHub parsing never produces this variant.
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracking_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
}

impl Default for SourceSpec {
    fn default() -> Self {
        Self::Local {
            path: String::new(),
        }
    }
}

impl SourceSpec {
    pub fn local(path: impl Into<String>) -> Self {
        Self::Local { path: path.into() }
    }

    pub fn github(
        url: impl Into<String>,
        requested_ref: Option<String>,
        subdir: Option<String>,
    ) -> Self {
        let tracking_ref = requested_ref.clone();
        Self::GitHub {
            url: url.into(),
            requested_ref,
            tracking_ref,
            subdir,
        }
    }

    pub fn with_tracking_ref(mut self, tracking_ref: Option<String>) -> Self {
        match &mut self {
            Self::GitHub {
                tracking_ref: current,
                ..
            }
            | Self::Git {
                tracking_ref: current,
                ..
            } => *current = tracking_ref,
            Self::Local { .. } => {}
        }
        self
    }

    pub fn tracking_candidate(&self) -> Option<Self> {
        let mut candidate = self.clone();
        match &mut candidate {
            Self::GitHub {
                requested_ref,
                tracking_ref,
                ..
            }
            | Self::Git {
                requested_ref,
                tracking_ref,
                ..
            } => {
                *requested_ref = tracking_ref.clone();
                Some(candidate)
            }
            Self::Local { .. } => None,
        }
    }

    pub fn repository_url(&self) -> Option<&str> {
        match self {
            Self::GitHub { url, .. } | Self::Git { url, .. } => Some(url),
            Self::Local { .. } => None,
        }
    }

    pub fn requested_ref(&self) -> Option<&str> {
        match self {
            Self::GitHub { requested_ref, .. } | Self::Git { requested_ref, .. } => {
                requested_ref.as_deref()
            }
            Self::Local { .. } => None,
        }
    }

    pub fn tracking_ref(&self) -> Option<&str> {
        match self {
            Self::GitHub { tracking_ref, .. } | Self::Git { tracking_ref, .. } => {
                tracking_ref.as_deref()
            }
            Self::Local { .. } => None,
        }
    }

    pub fn subdir(&self) -> Option<&str> {
        match self {
            Self::GitHub { subdir, .. } | Self::Git { subdir, .. } => subdir.as_deref(),
            Self::Local { .. } => None,
        }
    }

    pub fn is_upstream_bound(&self) -> bool {
        matches!(self, Self::GitHub { .. } | Self::Git { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSource {
    pub source_spec: SourceSpec,
    pub resolved_commit: String,
    pub body_hash: String,
    pub candidate_identity: String,
    #[serde(default)]
    pub subdir: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    #[default]
    Notify,
    Manual,
    Pinned,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogReviewStatus {
    #[default]
    Unreviewed,
    Acknowledged,
    Reviewed,
    Rejected,
}

/// The set of deterministic risk identifiers explicitly acknowledged by the
/// caller of a plan-bound apply operation.
pub type RiskAcknowledgements = BTreeSet<String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskFinding {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// This is deliberately a bounded, non-secret explanation.  Findings
    /// never contain matching file bytes or suspected credential material.
    pub detail: String,
    #[serde(default = "default_true")]
    pub acknowledgement_required: bool,
}

fn default_true() -> bool {
    true
}

impl RiskFinding {
    pub fn acknowledgement(
        code: impl Into<String>,
        path: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            path,
            detail: detail.into(),
            acknowledgement_required: true,
        }
    }

    /// Return the stable acknowledgement key for this finding.
    ///
    /// Findings without a path use their code.  Path-scoped findings append
    /// the normalized relative path, so two script or sensitive-content
    /// findings cannot be acknowledged accidentally as one another.
    pub fn acknowledgement_id(&self) -> String {
        match &self.path {
            Some(path) => format!("{}@{}", self.code, path.replace('\\', "/")),
            None => self.code.clone(),
        }
    }

    pub fn id(&self) -> String {
        self.acknowledgement_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyRevision {
    pub revision: String,
    pub source_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_source: Option<ResolvedSource>,
    #[serde(default)]
    pub created_at: u64,
    /// Complete immutable metadata for the installed body revision. During
    /// one-way registry migration, records without it are materialized from
    /// the current installed record and then written in the new schema.
    #[serde(default)]
    pub metadata: InstalledSkillMetadata,
}

/// All mutable registry metadata that belongs to one immutable body revision.
/// Keeping this as a value object makes rollback restore provenance and host
/// routing semantics together with the body hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledSkillMetadata {
    pub skill_id: String,
    pub source: String,
    pub source_hash: String,
    pub license_path: String,
    pub license_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_hash: Option<String>,
    pub body_revision: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoints: Vec<String>,
    pub invoke_hint: String,
    pub requires_auth: bool,
    pub version: String,
    pub target_hosts: Vec<String>,
    #[serde(default)]
    pub source_spec: SourceSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_source: Option<ResolvedSource>,
    #[serde(default)]
    pub update_policy: UpdatePolicy,
    #[serde(default)]
    pub catalog_review: CatalogReviewStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_findings: Vec<RiskFinding>,
    #[serde(default)]
    pub installed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledSkillRecord {
    pub skill_id: String,
    /// A stable display/source path kept for the existing projection API.
    pub source: String,
    pub source_hash: String,
    pub license_path: String,
    pub license_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_hash: Option<String>,
    pub body_revision: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoints: Vec<String>,
    pub invoke_hint: String,
    pub requires_auth: bool,
    pub version: String,
    pub target_hosts: Vec<String>,
    #[serde(default)]
    pub source_spec: SourceSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_source: Option<ResolvedSource>,
    #[serde(default)]
    pub update_policy: UpdatePolicy,
    #[serde(default)]
    pub catalog_review: CatalogReviewStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_findings: Vec<RiskFinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_revisions: Vec<BodyRevision>,
    #[serde(default)]
    pub installed_at: u64,
}

impl InstalledSkillMetadata {
    pub fn from_record(record: &InstalledSkillRecord) -> Self {
        Self {
            skill_id: record.skill_id.clone(),
            source: record.source.clone(),
            source_hash: record.source_hash.clone(),
            license_path: record.license_path.clone(),
            license_hash: record.license_hash.clone(),
            routing_metadata_path: record.routing_metadata_path.clone(),
            routing_metadata_hash: record.routing_metadata_hash.clone(),
            body_revision: record.body_revision.clone(),
            summary: record.summary.clone(),
            intent_tags: record.intent_tags.clone(),
            positive_examples: record.positive_examples.clone(),
            negative_examples: record.negative_examples.clone(),
            entrypoints: record.entrypoints.clone(),
            invoke_hint: record.invoke_hint.clone(),
            requires_auth: record.requires_auth,
            version: record.version.clone(),
            target_hosts: record.target_hosts.clone(),
            source_spec: record.source_spec.clone(),
            resolved_source: record.resolved_source.clone(),
            update_policy: record.update_policy,
            catalog_review: record.catalog_review,
            risk_findings: record.risk_findings.clone(),
            installed_at: record.installed_at,
        }
    }

    pub fn restore_record(&self, body_revisions: Vec<BodyRevision>) -> InstalledSkillRecord {
        InstalledSkillRecord {
            skill_id: self.skill_id.clone(),
            source: self.source.clone(),
            source_hash: self.source_hash.clone(),
            license_path: self.license_path.clone(),
            license_hash: self.license_hash.clone(),
            routing_metadata_path: self.routing_metadata_path.clone(),
            routing_metadata_hash: self.routing_metadata_hash.clone(),
            body_revision: self.body_revision.clone(),
            summary: self.summary.clone(),
            intent_tags: self.intent_tags.clone(),
            positive_examples: self.positive_examples.clone(),
            negative_examples: self.negative_examples.clone(),
            entrypoints: self.entrypoints.clone(),
            invoke_hint: self.invoke_hint.clone(),
            requires_auth: self.requires_auth,
            version: self.version.clone(),
            target_hosts: self.target_hosts.clone(),
            source_spec: self.source_spec.clone(),
            resolved_source: self.resolved_source.clone(),
            update_policy: self.update_policy,
            catalog_review: self.catalog_review,
            risk_findings: self.risk_findings.clone(),
            body_revisions,
            installed_at: self.installed_at,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.skill_id.is_empty() && self.body_revision.is_empty() && self.source_hash.is_empty()
    }
}

impl BodyRevision {
    pub fn from_record(record: &InstalledSkillRecord) -> Self {
        Self {
            revision: record.body_revision.clone(),
            source_hash: record.source_hash.clone(),
            resolved_source: record.resolved_source.clone(),
            created_at: record.installed_at,
            metadata: InstalledSkillMetadata::from_record(record),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledSkillIndex {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub skills: BTreeMap<String, InstalledSkillRecord>,
}

impl Default for InstalledSkillIndex {
    fn default() -> Self {
        Self {
            schema_version: INSTALLED_SKILL_INDEX_SCHEMA.to_string(),
            revision: 0,
            skills: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparedSkillChangeContract {
    #[serde(rename = "ags-prepared-skill-change-v2")]
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedSkillChange {
    pub contract_schema: PreparedSkillChangeContract,
    pub operation: String,
    pub skill_id: String,
    pub source: String,
    pub source_hash: String,
    pub license_path: String,
    pub license_hash: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub routing_metadata_path: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub routing_metadata_hash: Option<String>,
    pub body_path: String,
    pub installed_skill_index_path: String,
    pub target_hosts: Vec<String>,
    pub host_indexes: Vec<String>,
    /// Obsolete AGS-owned indexes in other roots read by the same Host. They
    /// are removed in the same WAL transaction before snapshot activation.
    pub retired_host_indexes: Vec<String>,
    pub planned_writes: Vec<String>,
    pub warnings: Vec<String>,
    pub source_spec: SourceSpec,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub resolved_source: Option<ResolvedSource>,
    pub body_hash: String,
    pub candidate_identity: String,
    pub update_policy: UpdatePolicy,
    pub catalog_review: CatalogReviewStatus,
    pub risk_findings: Vec<RiskFinding>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub candidate_path: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub previous_body_revision: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub rollback_revision: Option<String>,
    /// Registry revision and complete previous identity form a compare-and-
    /// swap binding.  They are checked again while holding the transaction
    /// lock immediately before mutation.
    pub registry_revision: u64,
    /// Descriptor-bound registry observation used by both planning and
    /// materialization. Absence is a first-class seal, not an unchecked
    /// `exists()` branch.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub registry_read_input: Option<ReadInputSeal>,
    pub registry_semantic_hash: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub previous_record: Option<InstalledSkillRecord>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub previous_record_hash: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub previous_body_hash: Option<String>,
    /// Complete target registry record computed during planning. Apply-time
    /// materialization never re-audits or rereads candidate paths to rebuild
    /// this value.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub target_record: Option<InstalledSkillRecord>,
    /// Descriptor-relevant identities of every object in the candidate tree
    /// as observed while the plan was prepared. Materialization must validate
    /// these after reading the bounded tree so same-size rewrites and inode
    /// replacement cannot reuse an older plan.
    pub candidate_read_inputs: Vec<ReadInputSeal>,
    /// Exact raw target bytes (or absence) of every host index observed while
    /// planning. Materialization is a byte-exact compare-and-swap.
    pub expected_link_targets: BTreeMap<String, Option<Vec<u8>>>,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadInputKind {
    Absent,
    Directory,
    RegularFile,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInputIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInputSeal {
    pub root: String,
    pub relative_path: String,
    pub kind: ReadInputKind,
    pub mode: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ReadInputIdentity>,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterializedBodyNode {
    Directory {
        relative_path: String,
        mode: u32,
    },
    RegularFile {
        relative_path: String,
        bytes: Vec<u8>,
        mode: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedBodyTree {
    pub root: String,
    pub root_mode: u32,
    pub parent_directories: Vec<MaterializedDirectory>,
    pub nodes: Vec<MaterializedBodyNode>,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedDirectory {
    pub path: String,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum MaterializedBodyDisposition {
    CreateExact(MaterializedBodyTree),
    AlreadyExact { root: String, manifest_hash: String },
    UnchangedRetained { root: String, manifest_hash: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedRegularFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_bytes: Option<Vec<u8>>,
    pub post_bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_mode: Option<u32>,
    pub post_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedSymlink {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_target: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_target: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedSnapshot {
    pub host: String,
    pub snapshot_hash: String,
    pub file: MaterializedRegularFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedSkillChange {
    pub operation: String,
    pub skill_id: String,
    pub registry_revision: u64,
    pub registry: MaterializedRegularFile,
    /// Every directory which is absent at materialization time and may be
    /// created by applying this sealed change, across all artifact families.
    pub parent_directories: Vec<MaterializedDirectory>,
    pub body: MaterializedBodyDisposition,
    pub links: Vec<MaterializedSymlink>,
    pub snapshots: Vec<MaterializedSnapshot>,
    pub read_inputs: Vec<ReadInputSeal>,
    pub materialization_hash: String,
}

/// In-memory view of the installed registry, immutable body and host indexes
/// after one materialized Skill change. Snapshot compilation consumes this
/// value instead of rereading paths which have deliberately not been written.
#[derive(Debug, Clone)]
pub(crate) struct SkillPostStateOverlay {
    pub target_skill_id: String,
    pub installed_skills: InstalledSkillIndex,
    pub target_body_hash_matches: bool,
    pub target_visible_hosts: BTreeSet<String>,
}

impl MaterializedSkillChange {
    pub fn write_paths(&self) -> Vec<String> {
        let mut paths = vec![self.registry.path.clone()];
        paths.extend(
            self.parent_directories
                .iter()
                .map(|directory| directory.path.clone()),
        );
        if let MaterializedBodyDisposition::CreateExact(body) = &self.body {
            paths.extend(
                body.parent_directories
                    .iter()
                    .map(|directory| directory.path.clone()),
            );
            paths.push(body.root.clone());
            paths.extend(body.nodes.iter().map(|node| {
                let relative = match node {
                    MaterializedBodyNode::Directory { relative_path, .. }
                    | MaterializedBodyNode::RegularFile { relative_path, .. } => relative_path,
                };
                std::path::Path::new(&body.root)
                    .join(relative)
                    .to_string_lossy()
                    .into_owned()
            }));
        }
        paths.extend(
            self.links
                .iter()
                .filter(|link| link.previous_target != link.post_target)
                .map(|link| link.path.clone()),
        );
        paths.extend(
            self.snapshots
                .iter()
                .map(|snapshot| snapshot.file.path.clone()),
        );
        paths.sort();
        paths.dedup();
        paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptionStatus {
    pub skill_id: String,
    pub registered: bool,
    pub body_present: bool,
    pub body_hash_matches: bool,
    pub target_hosts: Vec<String>,
    pub visible_hosts: Vec<String>,
    pub active_hosts: Vec<String>,
    pub source: Option<String>,
    pub source_hash: Option<String>,
}

/// One Host's current activated fact. This is produced by loading the sealed
/// snapshot and executing the same exact resolver used by runtime routing; a
/// snapshot membership check alone is insufficient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivatedCapability {
    pub skill_id: String,
    pub host: String,
    pub visible: bool,
    pub snapshot_loaded: bool,
    pub route_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_hash: Option<String>,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptionRouteStatus {
    pub installation: AdoptionStatus,
    pub activations: Vec<ActivatedCapability>,
}

impl AdoptionRouteStatus {
    pub fn verified_on_all_targets(&self) -> bool {
        self.installation.registered
            && self.installation.body_present
            && self.installation.body_hash_matches
            && !self.installation.target_hosts.is_empty()
            && self.activations.len() == self.installation.target_hosts.len()
            && self.activations.iter().all(|item| item.route_verified)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptionRoutingMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoke_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_auth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}
