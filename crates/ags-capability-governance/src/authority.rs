use super::*;

pub const HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION: &str = "0.3.1-host-capability-snapshot";
pub const CAPABILITY_SNAPSHOT_SCHEMA_VERSION: &str = HOST_CAPABILITY_SNAPSHOT_SCHEMA_VERSION;
pub const USER_OVERLAY_SCHEMA_VERSION: &str = "0.3.0-user-skill-overlay";
pub const USER_SOURCE_REGISTRY_SCHEMA_VERSION: &str = "0.3.0-user-skill-sources";
pub const USER_SOURCE_AUDIT_VERSION: &str = "skill-source-audit-v1";
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
    "source_hash_changed",
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

pub(super) fn is_capability_authority_root(path: &Path) -> bool {
    path.join("manifests/skills-registry.yaml").is_file()
        && path.join("manifests/mcp-registry.yaml").is_file()
}

pub(super) fn normalized_path(path: &Path) -> PathBuf {
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

pub(super) fn installed_source_root(runtime_home: &Path) -> Option<PathBuf> {
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
        .join(".ags/private-runtime")
}

pub(super) fn safe_host(active_host: &str) -> String {
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
