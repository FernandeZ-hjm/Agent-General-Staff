use super::*;
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
        }
    }

    while normalized.ends_with('-') {
        normalized.pop();
    }

    if normalized.is_empty() {
        Err("invalid agent type: empty or unsupported identifier".to_string())
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
