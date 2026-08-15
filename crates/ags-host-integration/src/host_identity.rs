use super::*;
use std::fmt;
use std::str::FromStr;

/// Canonical identity for any governed host, including third-party and future
/// Generic Agents. Official adapters may add probes and lifecycle codecs, but
/// they are not an admission allowlist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HostId(String);

impl HostId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, String> {
        normalize_agent_id(value.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for HostId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for HostId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HostId> for String {
    fn from(value: HostId) -> Self {
        value.0
    }
}

/// Host-facing execution surfaces accepted by the Generic Agent contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSurface {
    Cli,
    Mcp,
    Hybrid,
}

pub const HOST_REGISTRATION_SCHEMA_VERSION: &str = "ags://schema/contract/v2/host-registration";
pub const HOST_REGISTRATION_CONTRACT_VERSION: &str = "2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedOperation {
    AgsDecide,
    AgsApply,
}

pub const GOVERNED_OPERATIONS: [GovernedOperation; 2] =
    [GovernedOperation::AgsDecide, GovernedOperation::AgsApply];

/// AGS-owned authority record for one governed host.
///
/// Official adapter identity is integrity-bound probe metadata. It is not an
/// admission allowlist and may be `null` for a Generic host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostRegistration {
    pub schema_version: String,
    pub host_id: HostId,
    pub surface: AgentSurface,
    pub contract_version: String,
    pub governed_operations: [GovernedOperation; 2],
    pub official_adapter: Option<String>,
    pub registration_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostRegistrationWire {
    schema_version: String,
    host_id: HostId,
    surface: AgentSurface,
    contract_version: String,
    governed_operations: [GovernedOperation; 2],
    #[serde(deserialize_with = "required_optional_string")]
    official_adapter: Option<String>,
    registration_hash: String,
}

#[derive(Serialize)]
struct HostRegistrationHashMaterial<'a> {
    schema_version: &'a str,
    host_id: &'a HostId,
    surface: AgentSurface,
    contract_version: &'a str,
    governed_operations: &'a [GovernedOperation; 2],
    official_adapter: &'a Option<String>,
}

fn required_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

impl HostRegistration {
    pub fn new(host_id: HostId, surface: AgentSurface, official_adapter: Option<String>) -> Self {
        let mut registration = Self {
            schema_version: HOST_REGISTRATION_SCHEMA_VERSION.to_string(),
            host_id,
            surface,
            contract_version: HOST_REGISTRATION_CONTRACT_VERSION.to_string(),
            governed_operations: GOVERNED_OPERATIONS,
            official_adapter,
            registration_hash: String::new(),
        };
        registration.registration_hash = registration.semantic_hash();
        registration
    }

    pub fn semantic_hash(&self) -> String {
        let material = HostRegistrationHashMaterial {
            schema_version: &self.schema_version,
            host_id: &self.host_id,
            surface: self.surface,
            contract_version: &self.contract_version,
            governed_operations: &self.governed_operations,
            official_adapter: &self.official_adapter,
        };
        let mut canonical = b"ags-host-registration-v2\n".to_vec();
        canonical.extend(serde_json::to_vec(&material).unwrap_or_default());
        ags_platform::sha256(canonical)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != HOST_REGISTRATION_SCHEMA_VERSION {
            return Err("host registration schema_version is not contract-v2".to_string());
        }
        if self.contract_version != HOST_REGISTRATION_CONTRACT_VERSION {
            return Err("host registration contract_version is not 2".to_string());
        }
        if self.governed_operations != GOVERNED_OPERATIONS {
            return Err("host registration governed_operations are not canonical".to_string());
        }
        if self
            .official_adapter
            .as_ref()
            .is_some_and(|adapter| adapter.trim().is_empty())
        {
            return Err("host registration official_adapter is empty".to_string());
        }
        if self.registration_hash != self.semantic_hash() {
            return Err("host registration hash mismatch".to_string());
        }
        Ok(())
    }
}

impl TryFrom<HostRegistrationWire> for HostRegistration {
    type Error = String;

    fn try_from(wire: HostRegistrationWire) -> Result<Self, Self::Error> {
        let registration = Self {
            schema_version: wire.schema_version,
            host_id: wire.host_id,
            surface: wire.surface,
            contract_version: wire.contract_version,
            governed_operations: wire.governed_operations,
            official_adapter: wire.official_adapter,
            registration_hash: wire.registration_hash,
        };
        registration.validate()?;
        Ok(registration)
    }
}

impl<'de> Deserialize<'de> for HostRegistration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = HostRegistrationWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl FromStr for AgentSurface {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cli" => Ok(Self::Cli),
            "mcp" => Ok(Self::Mcp),
            "hybrid" => Ok(Self::Hybrid),
            _ => Err(format!(
                "invalid Agent surface `{value}`; expected cli, mcp, or hybrid"
            )),
        }
    }
}

/// Normalized host declaration accepted independently of optional official
/// probe support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenericAgent {
    pub host_id: HostId,
    pub surface: AgentSurface,
}

impl GenericAgent {
    pub fn new(host_id: impl AsRef<str>, surface: AgentSurface) -> Result<Self, String> {
        Ok(Self {
            host_id: HostId::new(host_id)?,
            surface,
        })
    }

    /// Official protocol metadata is an optional enhancement, never a gate.
    pub fn official_adapter(&self) -> Option<&'static AgentPlatformSpec> {
        platform_spec(self.host_id.as_str())
    }
}

// ── Agent instructions ─────────────────────────────────────────────────────

/// Agent type for instruction generation.
///
/// Known hosts get tailored instructions. Unknown non-empty host identifiers
/// fall back to a generic governed-host profile so new desktop modes do not
/// fail just because their agent string is new.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentType {
    Codex,
    ClaudeCode,
    Cursor,
    Generic(String),
}

impl AgentType {
    #[allow(clippy::should_implement_trait)] // inherent parser with domain String error; intentionally not std::str::FromStr
    pub fn from_str(s: &str) -> Result<Self, String> {
        let normalized = normalize_agent_id(s)?;
        match normalized.as_str() {
            "codex" => Ok(AgentType::Codex),
            "claude" | "claude-code" | "claudecode" => Ok(AgentType::ClaudeCode),
            "cursor" => Ok(AgentType::Cursor),
            other => Ok(AgentType::Generic(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            AgentType::Codex => "codex",
            AgentType::ClaudeCode => "claude-code",
            AgentType::Cursor => "cursor",
            AgentType::Generic(agent) => agent.as_str(),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            AgentType::Codex => "Codex".to_string(),
            AgentType::ClaudeCode => "Claude Code".to_string(),
            AgentType::Cursor => "Cursor".to_string(),
            AgentType::Generic(agent) => match recognized_host_display(agent) {
                Some(name) => name.to_string(),
                None => format!("Generic Agent ({agent})"),
            },
        }
    }

    pub fn is_generic(&self) -> bool {
        matches!(self, AgentType::Generic(_))
    }
}

pub(super) fn normalize_agent_id(input: &str) -> Result<String, String> {
    let mut normalized = String::new();
    let mut last_was_sep = false;

    for ch in input.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            normalized.push(lower);
            last_was_sep = false;
        } else if matches!(lower, '-' | '_' | '.' | ' ' | '\t' | '\n') && !last_was_sep {
            normalized.push('-');
            last_was_sep = true;
        } else if !matches!(lower, '-' | '_' | '.' | ' ' | '\t' | '\n') {
            return Err(format!(
                "invalid agent type: unsupported character `{ch}`; use ASCII letters, digits, spaces, dot, underscore, or hyphen"
            ));
        }
    }

    while normalized.ends_with('-') {
        normalized.pop();
    }

    if normalized.is_empty() {
        Err("invalid agent type: empty or unsupported identifier".to_string())
    } else if normalized.len() > 64 {
        Err("invalid agent type: normalized identifier exceeds 64 bytes".to_string())
    } else {
        Ok(normalized)
    }
}

/// Recognized governed-host display names, keyed by normalized agent id.
///
/// These hosts are still carried as `AgentType::Generic` — they get a branded
/// display name, but add NO new canonical runtime adapter and do NOT change the
/// generic fallback for unknown hosts. `normalize_agent_id` folds input
/// casing/spacing into these keys (e.g. "WorkBuddy" → "workbuddy",
/// "Oh My Pi" → "oh-my-pi"). Tencent Agent is the umbrella adapter entry;
/// WorkBuddy and CodeBuddy-Code are its host clients.
pub(super) const RECOGNIZED_HOST_DISPLAY: &[(&str, &str)] = &[
    ("tencent-agent", "Tencent Agent"),
    ("tencent", "Tencent Agent"),
    ("workbuddy", "Tencent Agent (WorkBuddy)"),
    ("codebuddy-code", "Tencent Agent (CodeBuddy-Code)"),
    ("codebuddy", "Tencent Agent (CodeBuddy-Code)"),
    ("omp", "Oh My Pi (OMP)"),
    ("oh-my-pi", "Oh My Pi (OMP)"),
];

/// Branded display name for a recognized governed host, or `None` for an unknown
/// generic host (which keeps the plain `Generic Agent (x)` form). The input must
/// already be normalized via `normalize_agent_id`.
#[doc(hidden)]
pub fn recognized_host_display(normalized: &str) -> Option<&'static str> {
    RECOGNIZED_HOST_DISPLAY
        .iter()
        .find(|(key, _)| *key == normalized)
        .map(|(_, name)| *name)
}

impl Serialize for AgentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        AgentType::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod generic_agent_tests {
    use super::*;

    #[test]
    fn unknown_normalized_host_is_governable_on_every_surface() {
        for surface in [AgentSurface::Cli, AgentSurface::Mcp, AgentSurface::Hybrid] {
            let agent = GenericAgent::new("  Hermes Agent.v2  ", surface).unwrap();
            assert_eq!(agent.host_id.as_str(), "hermes-agent-v2");
            assert_eq!(agent.surface, surface);
            assert!(agent.official_adapter().is_none());
        }
    }

    #[test]
    fn official_adapter_is_optional_probe_metadata() {
        let official = GenericAgent::new("Codex", AgentSurface::Hybrid).unwrap();
        assert_eq!(official.host_id.as_str(), "codex");
        assert_eq!(official.official_adapter().unwrap().id, "codex");

        assert!(GenericAgent::new("---", AgentSurface::Mcp).is_err());
        assert!("socket".parse::<AgentSurface>().is_err());
    }

    #[test]
    fn host_id_rejects_collision_prone_or_oversized_input() {
        for invalid in ["a/b", "a:b", "agent🚀", "代理"] {
            assert!(HostId::new(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(HostId::new("a".repeat(65)).is_err());
        assert_ne!(HostId::new("a-b").unwrap(), HostId::new("ab").unwrap());
        assert_eq!(
            HostId::new("  Hermes...Agent__v2  ").unwrap().as_str(),
            "hermes-agent-v2"
        );
    }

    #[test]
    fn host_registration_is_strict_hash_bound_contract_v2() {
        let registration =
            HostRegistration::new(HostId::new("hermes").unwrap(), AgentSurface::Hybrid, None);
        assert!(registration.validate().is_ok());
        assert_eq!(registration.governed_operations, GOVERNED_OPERATIONS);

        let encoded = serde_json::to_value(&registration).unwrap();
        for required in [
            "schema_version",
            "host_id",
            "surface",
            "contract_version",
            "governed_operations",
            "official_adapter",
            "registration_hash",
        ] {
            let mut incomplete = encoded.clone();
            incomplete.as_object_mut().unwrap().remove(required);
            assert!(
                serde_json::from_value::<HostRegistration>(incomplete).is_err(),
                "missing required registration field {required} was accepted"
            );
        }

        let mut unknown = encoded.clone();
        unknown["legacy_capabilities"] = serde_json::json!({"cli": true});
        assert!(serde_json::from_value::<HostRegistration>(unknown).is_err());

        let mut tampered = encoded;
        tampered["surface"] = serde_json::json!("cli");
        assert!(serde_json::from_value::<HostRegistration>(tampered).is_err());
    }

    #[test]
    fn host_registration_rejects_legacy_generic_agent_wire() {
        let legacy = serde_json::json!({
            "schema_version": "ags://schema/contract/v2/generic-agent-registration",
            "host_id": "hermes",
            "surface": "hybrid",
            "capabilities": {
                "cli": true,
                "mcp": true,
                "governed_operations": ["ags_decide", "ags_apply"]
            },
            "official_adapter": null,
            "third_party_configuration": "advice_only"
        });
        assert!(serde_json::from_value::<HostRegistration>(legacy).is_err());
    }

    #[test]
    fn generic_hermes_registration_is_valid_on_every_surface_without_an_adapter() {
        for surface in [AgentSurface::Cli, AgentSurface::Mcp, AgentSurface::Hybrid] {
            let registration = HostRegistration::new(HostId::new("hermes").unwrap(), surface, None);
            let decoded: HostRegistration =
                serde_json::from_slice(&serde_json::to_vec(&registration).unwrap()).unwrap();
            assert_eq!(decoded.host_id.as_str(), "hermes");
            assert_eq!(decoded.surface, surface);
            assert!(decoded.official_adapter.is_none());
            assert!(decoded.validate().is_ok());
        }
    }
}
