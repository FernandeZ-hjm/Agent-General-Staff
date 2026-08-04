use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PRIVATE_SKILL_REGISTRY_SCHEMA: &str = "0.4.0-private-skill-registry";
pub const ADOPTION_PLAN_SCHEMA: &str = "0.4.0-skill-adoption-plan";
pub const ADOPTION_RECEIPT_SCHEMA: &str = "0.4.0-skill-adoption-receipt";

#[derive(Debug, Clone)]
pub struct AdoptionContext {
    pub authority_root: PathBuf,
    pub runtime_home: PathBuf,
    pub host_home: PathBuf,
    pub snapshot_discovery: SnapshotDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotDiscovery {
    Live,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateSkillRecord {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateSkillRegistry {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub skills: BTreeMap<String, PrivateSkillRecord>,
}

impl Default for PrivateSkillRegistry {
    fn default() -> Self {
        Self {
            schema_version: PRIVATE_SKILL_REGISTRY_SCHEMA.to_string(),
            revision: 0,
            skills: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptionPlan {
    pub schema_version: String,
    pub operation: String,
    pub plan_hash: String,
    pub skill_id: String,
    pub source: String,
    pub source_hash: String,
    pub license_path: String,
    pub license_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata_hash: Option<String>,
    pub body_path: String,
    pub registry_path: String,
    pub target_hosts: Vec<String>,
    pub host_indexes: Vec<String>,
    pub planned_writes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptionReceipt {
    pub schema_version: String,
    pub operation: String,
    pub plan_hash: String,
    pub skill_id: String,
    pub registry_revision: u64,
    pub body_path: String,
    pub host_indexes: Vec<String>,
    pub snapshot_hashes: BTreeMap<String, String>,
    pub requires_repreflight: bool,
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
