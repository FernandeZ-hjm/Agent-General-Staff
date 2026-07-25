use super::*;
#[allow(unused_imports)]
use super::{authority::*, catalog::*, hashing::*, private_store::*};
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

pub(super) fn validate_usage_event(event: &SkillUsageEvent) -> Result<(), String> {
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

pub(super) fn validate_usage_identifier(field: &str, value: &str) -> Result<(), String> {
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

pub(super) fn set_private_permissions(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot chmod {}: {error}", _path.display()))?;
    }
    Ok(())
}
