//! Read model for the shared launcher's signed update state.
//!
//! Network fetch, signature verification, update planning and activation have
//! one owner: the npm launcher core used by both public entrances. Rust only
//! projects that authenticated state into CLI and MCP notifications.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const UPDATE_CHECK_STATE_SCHEMA: &str = "0.4.13-update-check-state";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckState {
    pub schema_version: String,
    pub enabled: bool,
    pub channel: String,
    pub ignored_versions: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_index_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Default for UpdateCheckState {
    fn default() -> Self {
        Self {
            schema_version: UPDATE_CHECK_STATE_SCHEMA.to_string(),
            enabled: true,
            channel: "stable".to_string(),
            ignored_versions: BTreeSet::new(),
            snoozed_until_unix: None,
            last_checked_at_unix: None,
            latest_version: None,
            release_index_hash: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateNoticeStatus {
    Current,
    Available,
    Snoozed,
    Ignored,
    Disabled,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateNotice {
    pub status: UpdateNoticeStatus,
    pub current: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at_unix: Option<u64>,
    pub channel: String,
    pub actions: Vec<String>,
    pub detail: String,
}

pub fn default_update_state_root() -> PathBuf {
    if let Some(cache) = std::env::var_os("AGS_CACHE_DIR") {
        return PathBuf::from(cache).join("launcher-state");
    }
    ags_platform::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ags")
        .join("launcher-state")
}

/// Interpret the last authenticated launcher observation. Missing, offline or
/// malformed state is non-blocking and never changes the installed runtime.
pub fn cached_update_notice(
    state_root: &Path,
    current_version: &str,
    now_unix: u64,
) -> UpdateNotice {
    let state = match load_update_state(state_root) {
        Ok(state) => state,
        Err(error) => {
            return notice(
                &UpdateCheckState::default(),
                current_version,
                UpdateNoticeStatus::Offline,
                &format!(
                    "signed launcher check is unavailable; installed AGS remains usable ({error})"
                ),
            );
        }
    };
    if !state.enabled {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Disabled,
            "release checks are disabled",
        );
    }
    if let Some(error) = &state.last_error {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Offline,
            &format!(
                "signed launcher check is unavailable; installed AGS remains usable ({error})"
            ),
        );
    }
    let Some(latest) = state.latest_version.as_deref() else {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Offline,
            "the launcher has not recorded a verified release index",
        );
    };
    if !version_is_newer(latest, current_version) {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Current,
            "installed AGS is current for this channel",
        );
    }
    if state.ignored_versions.contains(latest) {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Ignored,
            "this AGS version is ignored",
        );
    }
    if state
        .snoozed_until_unix
        .is_some_and(|until| now_unix < until)
    {
        return notice(
            &state,
            current_version,
            UpdateNoticeStatus::Snoozed,
            "AGS update notice is snoozed",
        );
    }
    notice(
        &state,
        current_version,
        UpdateNoticeStatus::Available,
        "a newer signed AGS release is available",
    )
}

pub fn set_update_checks_enabled(state_root: &Path, enabled: bool) -> Result<(), String> {
    let mut state = load_update_state(state_root).unwrap_or_default();
    state.enabled = enabled;
    save_update_state(state_root, &state)
}

pub fn set_update_channel(state_root: &Path, channel: &str) -> Result<(), String> {
    if channel != "stable" {
        return Err("only the signed stable update channel is available".to_string());
    }
    let mut state = load_update_state(state_root).unwrap_or_default();
    state.channel = channel.to_string();
    state.last_checked_at_unix = None;
    state.latest_version = None;
    state.release_index_hash = None;
    state.last_error = None;
    save_update_state(state_root, &state)
}

pub fn ignore_update_version(state_root: &Path, version: &str) -> Result<(), String> {
    if !valid_version(version) {
        return Err("ignored version is invalid".to_string());
    }
    let mut state = load_update_state(state_root).unwrap_or_default();
    state.ignored_versions.insert(version.to_string());
    save_update_state(state_root, &state)
}

pub fn snooze_update_notices(state_root: &Path, until_unix: u64) -> Result<(), String> {
    let mut state = load_update_state(state_root).unwrap_or_default();
    state.snoozed_until_unix = Some(until_unix);
    save_update_state(state_root, &state)
}

pub fn load_update_state(state_root: &Path) -> Result<UpdateCheckState, String> {
    let path = state_root.join("update-check.json");
    let bytes = fs::read(&path)
        .map_err(|error| format!("cannot read update state {}: {error}", path.display()))?;
    let state: UpdateCheckState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid update state {}: {error}", path.display()))?;
    if state.schema_version != UPDATE_CHECK_STATE_SCHEMA {
        return Err("update state schema mismatch".to_string());
    }
    if state.channel != "stable"
        || state
            .release_index_hash
            .as_deref()
            .is_some_and(|hash| !ags_platform::is_sha256_hex(hash))
        || state
            .latest_version
            .as_deref()
            .is_some_and(|version| !valid_version(version))
    {
        return Err("update state identity is invalid".to_string());
    }
    Ok(state)
}

fn save_update_state(state_root: &Path, state: &UpdateCheckState) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("cannot serialize update state: {error}"))?;
    bytes.push(b'\n');
    ags_platform::atomic_write(&state_root.join("update-check.json"), &bytes)
}

fn notice(
    state: &UpdateCheckState,
    current: &str,
    status: UpdateNoticeStatus,
    detail: &str,
) -> UpdateNotice {
    UpdateNotice {
        status,
        current: current.to_string(),
        latest: state.latest_version.clone(),
        checked_at_unix: state.last_checked_at_unix,
        channel: state.channel.clone(),
        actions: if status == UpdateNoticeStatus::Available {
            vec!["review".into(), "snooze".into(), "ignore".into()]
        } else {
            Vec::new()
        },
        detail: detail.to_string(),
    }
}

fn valid_version(value: &str) -> bool {
    let value = value.trim_start_matches('v');
    let stable = value.split_once('-').map_or(value, |(stable, _)| stable);
    let parts = stable.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> Option<[u64; 3]> {
        let stable = version
            .trim_start_matches('v')
            .split_once('-')
            .map_or(version.trim_start_matches('v'), |(stable, _)| stable);
        let parsed = stable
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (parsed.len() == 3).then(|| [parsed[0], parsed[1], parsed[2]])
    }
    matches!((parts(candidate), parts(current)), (Some(left), Some(right)) if left > right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn projects_verified_launcher_state_without_network() {
        let root = state_root();
        let state = UpdateCheckState {
            latest_version: Some("0.4.13".to_string()),
            release_index_hash: Some("a".repeat(64)),
            last_checked_at_unix: Some(100),
            ..UpdateCheckState::default()
        };
        save_update_state(root.path(), &state).unwrap();
        let notice = cached_update_notice(root.path(), "0.4.12", 101);
        assert_eq!(notice.status, UpdateNoticeStatus::Available);

        ignore_update_version(root.path(), "0.4.13").unwrap();
        assert_eq!(
            cached_update_notice(root.path(), "0.4.12", 102).status,
            UpdateNoticeStatus::Ignored
        );
    }

    #[test]
    fn missing_or_offline_state_never_blocks_installed_ags() {
        let root = state_root();
        assert_eq!(
            cached_update_notice(root.path(), "0.4.12", 0).status,
            UpdateNoticeStatus::Offline
        );
        let state = UpdateCheckState {
            last_error: Some("offline".to_string()),
            ..UpdateCheckState::default()
        };
        save_update_state(root.path(), &state).unwrap();
        assert_eq!(
            cached_update_notice(root.path(), "0.4.12", 1).status,
            UpdateNoticeStatus::Offline
        );
    }

    #[test]
    fn snooze_and_disable_are_projected() {
        let root = state_root();
        let state = UpdateCheckState {
            latest_version: Some("0.4.13".to_string()),
            release_index_hash: Some("b".repeat(64)),
            ..UpdateCheckState::default()
        };
        save_update_state(root.path(), &state).unwrap();
        snooze_update_notices(root.path(), 500).unwrap();
        assert_eq!(
            cached_update_notice(root.path(), "0.4.12", 100).status,
            UpdateNoticeStatus::Snoozed
        );
        set_update_checks_enabled(root.path(), false).unwrap();
        assert_eq!(
            cached_update_notice(root.path(), "0.4.12", 100).status,
            UpdateNoticeStatus::Disabled
        );
    }
}
