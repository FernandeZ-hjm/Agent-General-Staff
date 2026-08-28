//! Canonical machine-runtime upgrade engine (contract v3).
//!
//! Distribution adapters may obtain an initial trusted runtime, but version
//! selection, signed-index verification, plan binding, activation, recovery,
//! receipts and update reminders live here. CLI and MCP are projections over
//! this module; the npm launcher only prepares and starts a verified binary.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::seal::ApplyOutput;
use crate::workspace::WorkspaceBinding;

pub const UPGRADE_OPERATION: &str = "upgrade";
pub const CHECK_INTERVAL_SECONDS: i64 = 7 * 24 * 60 * 60;
pub const FAILURE_RETRY_SECONDS: i64 = 24 * 60 * 60;
const PLAN_TTL_SECONDS: i64 = 30 * 60;
const MAX_INDEX_BYTES: u64 = 256 * 1024;
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const REPOSITORY: &str = "FernandeZ-hjm/Agent-General-Staff";
const PUBLIC_KEY_PEM: &str =
    include_str!("../../../packages/ags-launcher/release-signing-public.pem");
const BINARIES: [&str; 5] = ["ags", "ags-mcp", "ags-host", "ags-policy", "ags-release"];
const OFFICIAL_SKILLS: [&str; 5] = [
    "ags-agent",
    "ags-doctor",
    "ags-govern",
    "ags-init",
    "ags-setup",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub version: String,
    pub triple: String,
    pub source_root: PathBuf,
    pub binary_root: PathBuf,
    pub runtime_root: PathBuf,
    pub binaries: BTreeMap<String, String>,
    pub executables_sha256: String,
    pub runtime_sha256: String,
    pub asset_name: String,
    pub asset_sha256: String,
    pub release_index_sha256: String,
    pub signed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePointer {
    pub schema_version: u32,
    pub version: String,
    pub triple: String,
    pub binary_name: String,
    pub asset_name: String,
    pub asset_sha256: String,
    pub binary_sha256: String,
    #[serde(default)]
    pub executables_sha256: String,
    pub runtime_sha256: String,
    pub release_index_sha256: String,
    #[serde(default)]
    pub runtime_root: PathBuf,
    #[serde(default)]
    pub activated_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UpgradeEnvironment {
    install_kind: String,
    install_source_root: PathBuf,
    machine_root: PathBuf,
    versions_root: PathBuf,
    target_root: PathBuf,
    launcher_state_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeActivationJournal {
    schema_version: u32,
    plan_hash: String,
    target_root: PathBuf,
    stage_root: PathBuf,
    backup_root: PathBuf,
    previous: RuntimePointer,
    candidate_hashes: BTreeMap<String, String>,
    moved: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseIndex {
    schema_version: String,
    version: String,
    channel: String,
    repository: String,
    tag: String,
    commit: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseAsset {
    name: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpgradeCheckState {
    pub schema_version: String,
    pub enabled: bool,
    pub channel: String,
    pub last_checked_at_unix: Option<i64>,
    pub last_attempt_at_unix: Option<i64>,
    pub ignored_versions: Vec<String>,
    pub snoozed_until_unix: Option<i64>,
    pub latest_version: Option<String>,
    pub last_error: Option<String>,
    pub last_notified_version: Option<String>,
}

impl Default for UpgradeCheckState {
    fn default() -> Self {
        Self {
            schema_version: "ags://schema/contract/v3/upgrade-check-state".to_string(),
            enabled: true,
            channel: "stable".to_string(),
            last_checked_at_unix: None,
            last_attempt_at_unix: None,
            ignored_versions: Vec::new(),
            snoozed_until_unix: None,
            latest_version: None,
            last_error: None,
            last_notified_version: None,
        }
    }
}

pub fn prepare_plan(binding: &WorkspaceBinding, request: &Value) -> Result<Value> {
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("activate");
    if action == "recover" {
        return prepare_recovery(binding);
    }
    if action != "activate" {
        return Err(Error::new(
            "upgrade_action_invalid",
            "upgrade action must be activate or recover",
        ));
    }

    let current = ensure_current_pointer()?;
    let explicit_source = request.get("source_root").and_then(Value::as_str);
    let candidate = if let Some(root) = explicit_source {
        inspect_source(Path::new(root), false, None, None)?
    } else {
        install_signed_candidate("stable")?
    };
    let current_version = Version::parse(current.version.trim_start_matches('v')).map_err(|e| {
        Error::new(
            "upgrade_current_version_invalid",
            format!("active runtime version is invalid: {e}"),
        )
    })?;
    let candidate_version =
        Version::parse(candidate.version.trim_start_matches('v')).map_err(|e| {
            Error::new(
                "upgrade_candidate_version_invalid",
                format!("candidate runtime version is invalid: {e}"),
            )
        })?;
    if candidate_version < current_version {
        return Err(Error::new(
            "upgrade_downgrade_requires_recover",
            "upgrade cannot activate an older runtime; use a sealed upgrade recover plan",
        ));
    }
    if explicit_source.is_none() && candidate_version == current_version {
        return Err(Error::new(
            "no_update_available",
            "signed stable channel does not contain a newer runtime",
        ));
    }
    if candidate.version == current.version
        && candidate.executables_sha256 == current.executables_sha256
        && candidate.runtime_sha256 == current.runtime_sha256
    {
        return Err(Error::new(
            "no_update_available",
            "candidate runtime is byte-identical to the active runtime",
        ));
    }
    let now = now_unix();
    let environment = environment_binding()?;
    Ok(json!({
        "schema_version": "ags://schema/contract/v3/upgrade-plan",
        "action": "activate",
        "workspace_binding": binding.slug,
        "install_kind": environment.install_kind.clone(),
        "environment": environment.clone(),
        "current": current,
        "candidate": candidate,
        "target_root": environment.target_root.clone(),
        "created_at_unix": now,
        "expires_at_unix": now + PLAN_TTL_SECONDS,
    }))
}

fn prepare_recovery(binding: &WorkspaceBinding) -> Result<Value> {
    let current = current_pointer()?.ok_or_else(|| {
        Error::new(
            "upgrade_current_missing",
            "active runtime pointer is missing",
        )
    })?;
    let previous = previous_pointer()?.ok_or_else(|| {
        Error::new(
            "upgrade_previous_missing",
            "previous runtime pointer is missing",
        )
    })?;
    let candidate = inspect_cached_pointer(&previous)?;
    let now = now_unix();
    let environment = environment_binding()?;
    Ok(json!({
        "schema_version": "ags://schema/contract/v3/upgrade-plan",
        "action": "recover",
        "workspace_binding": binding.slug,
        "install_kind": environment.install_kind.clone(),
        "environment": environment.clone(),
        "current": current,
        "candidate": candidate,
        "target_root": environment.target_root.clone(),
        "created_at_unix": now,
        "expires_at_unix": now + PLAN_TTL_SECONDS,
    }))
}

pub fn apply(payload: &Value, _binding: &WorkspaceBinding) -> Result<ApplyOutput> {
    let created = required_i64(payload, "created_at_unix")?;
    let expires = required_i64(payload, "expires_at_unix")?;
    let now = now_unix();
    if now < created || now > expires {
        return Err(Error::new(
            "upgrade_plan_expired",
            "upgrade plan is outside its approved time window",
        ));
    }
    let approved_environment: UpgradeEnvironment = serde_json::from_value(
        payload
            .get("environment")
            .cloned()
            .ok_or_else(|| Error::new("upgrade_plan_invalid", "plan requires environment"))?,
    )
    .map_err(|e| Error::new("upgrade_plan_invalid", e.to_string()))?;
    let observed_environment = environment_binding()?;
    if observed_environment != approved_environment {
        return Err(Error::new(
            "upgrade_environment_drift",
            "installation kind or machine/cache/target roots changed after sealing; re-plan",
        ));
    }
    let approved_current: RuntimePointer = serde_json::from_value(
        payload
            .get("current")
            .cloned()
            .ok_or_else(|| Error::new("upgrade_plan_invalid", "plan requires current"))?,
    )
    .map_err(|e| Error::new("upgrade_plan_invalid", e.to_string()))?;
    let approved: Candidate = serde_json::from_value(
        payload
            .get("candidate")
            .cloned()
            .ok_or_else(|| Error::new("upgrade_plan_invalid", "plan requires candidate"))?,
    )
    .map_err(|e| Error::new("upgrade_plan_invalid", e.to_string()))?;
    let observed_current = ensure_current_pointer()?;
    if observed_current != approved_current {
        return Err(Error::new(
            "upgrade_current_drift",
            "active runtime changed after the plan was sealed; re-plan",
        ));
    }
    let observed = inspect_source(
        &approved.source_root,
        approved.signed,
        Some(&approved.asset_name),
        Some((&approved.asset_sha256, &approved.release_index_sha256)),
    )?;
    if observed != approved {
        return Err(Error::new(
            "upgrade_candidate_drift",
            "candidate runtime changed after the plan was sealed; re-plan",
        ));
    }
    let cached = materialize_candidate(&approved)?;
    let next = pointer_from_candidate(&cached);
    let kind = approved_environment.install_kind.as_str();
    let target_root = approved_environment.target_root;

    let plan_hash = sha256_bytes(canonical_json(payload)?.as_bytes());
    if kind == "launcher" {
        activate_launcher(&approved_current, &next)?;
    } else {
        activate_native(&approved_current, &cached, &target_root, &plan_hash)?;
        if let Err(error) = write_common_pointers(&approved_current, &next) {
            return Err(with_rollback_result(
                "upgrade_pointer_activation_failed",
                &error,
                rollback_activation(kind, &approved_current, &target_root),
            ));
        }
    }

    if let Err(error) = crate::sync::setup(&cached.runtime_root) {
        return Err(with_rollback_result(
            "upgrade_setup_failed",
            &error,
            rollback_activation(kind, &approved_current, &target_root),
        ));
    }
    if let Err(error) = verify_activation(kind, &next, &target_root) {
        return Err(with_rollback_result(
            "upgrade_activation_failed",
            &error,
            rollback_activation(kind, &approved_current, &target_root),
        ));
    }

    let receipt = json!({
        "schema_version": "ags://schema/contract/v3/upgrade-receipt",
        "plan_hash": plan_hash,
        "action": payload.get("action").cloned().unwrap_or(json!("activate")),
        "previous": approved_current,
        "active": next,
        "verified": true,
        "reconnect_required": true,
        "applied_at_unix": now,
    });
    if let Err(error) = write_receipt(&plan_hash, &receipt) {
        return Err(with_rollback_result(
            "upgrade_receipt_write_failed",
            &error,
            rollback_activation(kind, &approved_current, &target_root),
        ));
    }
    if kind == "native" {
        commit_native_activation(&target_root)?;
    }
    Ok(ApplyOutput {
        observed_write_set: vec![
            "machine:runtime-pointer".to_string(),
            "machine:five-binary-runtime".to_string(),
            "machine:install.json".to_string(),
            format!("machine:upgrade-receipt:{plan_hash}"),
        ],
        result: Some(receipt),
    })
}

pub fn status(action_ref: &str, binding: &WorkspaceBinding) -> Result<Value> {
    let plan = crate::seal::SealStore::new(binding).load_plan(action_ref)?;
    if plan.operation != UPGRADE_OPERATION {
        return Err(Error::new(
            "upgrade_action_ref_invalid",
            "action_ref does not reference an upgrade plan",
        ));
    }
    let plan_hash = sha256_bytes(canonical_json(&plan.payload)?.as_bytes());
    Ok(json!({
        "schema_version": "ags://schema/contract/v3/upgrade-status",
        "action_ref": action_ref,
        "seal_state": plan.state,
        "plan_hash": plan_hash,
        "plan": plan.payload,
        "receipt": read_receipt(&plan_hash)?,
        "active": current_pointer()?,
        "previous": previous_pointer()?,
    }))
}

pub fn verify(action_ref: &str, binding: &WorkspaceBinding) -> Result<Value> {
    let status = status(action_ref, binding)?;
    let receipt = status
        .get("receipt")
        .filter(|v| !v.is_null())
        .ok_or_else(|| Error::new("upgrade_receipt_missing", "upgrade has no applied receipt"))?;
    if receipt.get("verified").and_then(Value::as_bool) != Some(true) {
        return Err(Error::new(
            "upgrade_receipt_invalid",
            "upgrade receipt is not verified",
        ));
    }
    let active = current_pointer()?.ok_or_else(|| {
        Error::new(
            "upgrade_current_missing",
            "active runtime pointer is missing",
        )
    })?;
    let plan = status
        .get("plan")
        .ok_or_else(|| Error::new("upgrade_plan_invalid", "status requires plan"))?;
    let environment: UpgradeEnvironment = serde_json::from_value(
        plan.get("environment")
            .cloned()
            .ok_or_else(|| Error::new("upgrade_plan_invalid", "plan requires environment"))?,
    )
    .map_err(|e| Error::new("upgrade_plan_invalid", e.to_string()))?;
    verify_activation(&environment.install_kind, &active, &environment.target_root)?;
    let receipt_active: RuntimePointer = serde_json::from_value(
        receipt
            .get("active")
            .cloned()
            .ok_or_else(|| Error::new("upgrade_receipt_invalid", "receipt requires active"))?,
    )
    .map_err(|e| Error::new("upgrade_receipt_invalid", e.to_string()))?;
    if receipt_active != active {
        return Err(Error::new(
            "upgrade_active_drift",
            "active runtime no longer matches the applied receipt",
        ));
    }
    Ok(json!({
        "schema_version": "ags://schema/contract/v3/upgrade-verification",
        "status": "verified",
        "action_ref": action_ref,
        "active_version": active.version,
        "executables_sha256": active.executables_sha256,
        "runtime_sha256": active.runtime_sha256,
        "reconnect_required": true,
    }))
}

pub fn check(current_version: &str, force: bool) -> Value {
    check_at(current_version, force, now_unix(), fetch_latest_index)
}

fn check_at<F>(current_version: &str, force: bool, now: i64, fetch: F) -> Value
where
    F: FnOnce() -> Result<(ReleaseIndex, String)>,
{
    let mut state = read_check_state().unwrap_or_default();
    if !force {
        if !state.enabled {
            return json!({"checked": false, "skipped": "disabled", "state": state});
        }
        if state.snoozed_until_unix.is_some_and(|until| until > now) {
            return json!({"checked": false, "skipped": "snoozed", "state": state});
        }
        if state
            .last_checked_at_unix
            .is_some_and(|last| now - last < CHECK_INTERVAL_SECONDS)
        {
            return json!({"checked": false, "skipped": "fresh", "state": state});
        }
        if state.last_error.is_some()
            && state
                .last_attempt_at_unix
                .is_some_and(|last| now - last < FAILURE_RETRY_SECONDS)
        {
            return json!({"checked": false, "skipped": "failure-backoff", "state": state});
        }
    }
    state.last_attempt_at_unix = Some(now);
    match fetch() {
        Ok((index, index_hash)) => {
            state.last_checked_at_unix = Some(now);
            state.latest_version = Some(index.version.clone());
            state.last_error = None;
            let available = Version::parse(&index.version)
                .ok()
                .zip(Version::parse(current_version.trim_start_matches('v')).ok())
                .filter(|(latest, current)| latest > current)
                .map(|_| index.version.clone())
                .filter(|version| !state.ignored_versions.contains(version));
            let _ = write_check_state(&state);
            json!({
                "checked": true,
                "available": available.as_ref().map(|version| json!({
                    "version": version,
                    "current_version": current_version,
                    "channel": "stable",
                    "url": format!("https://github.com/{REPOSITORY}/releases/tag/v{version}"),
                })),
                "release_index_sha256": index_hash,
                "state": state,
            })
        }
        Err(error) => {
            state.last_error = Some(if error.code.contains("signature") {
                "unavailable".to_string()
            } else {
                "offline".to_string()
            });
            let _ = write_check_state(&state);
            json!({
                "checked": true,
                "available": Value::Null,
                "offline": state.last_error.as_deref() == Some("offline"),
                "unavailable": state.last_error.as_deref() == Some("unavailable"),
                "error": {"code": error.code, "message": error.message},
                "state": state,
            })
        }
    }
}

pub fn configure(
    enabled: Option<bool>,
    ignore_version: Option<&str>,
    snooze_until_unix: Option<i64>,
    reset: bool,
) -> Result<Value> {
    let mut state = if reset {
        UpgradeCheckState::default()
    } else {
        read_check_state()?
    };
    if let Some(value) = enabled {
        state.enabled = value;
    }
    if let Some(version) = ignore_version {
        Version::parse(version.trim_start_matches('v')).map_err(|e| {
            Error::new(
                "upgrade_version_invalid",
                format!("invalid ignored version: {e}"),
            )
        })?;
        let normalized = version.trim_start_matches('v').to_string();
        if !state.ignored_versions.contains(&normalized) {
            state.ignored_versions.push(normalized);
            state.ignored_versions.sort();
        }
    }
    if let Some(until) = snooze_until_unix {
        if until < 0 {
            return Err(Error::new(
                "upgrade_snooze_invalid",
                "snooze timestamp must be non-negative",
            ));
        }
        state.snoozed_until_unix = Some(until);
    }
    write_check_state(&state)?;
    Ok(json!(state))
}

pub fn maybe_notify(current_version: &str) {
    if std::env::var_os("AGS_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let now = now_unix();
    let mut state = read_check_state().unwrap_or_default();
    let available = state
        .latest_version
        .as_deref()
        .and_then(|latest| {
            Version::parse(latest)
                .ok()
                .zip(Version::parse(current_version.trim_start_matches('v')).ok())
                .filter(|(latest, current)| latest > current)
                .map(|_| latest.to_string())
        })
        .filter(|latest| !state.ignored_versions.contains(latest));
    let fresh = state
        .last_checked_at_unix
        .is_some_and(|last| now - last < CHECK_INTERVAL_SECONDS);
    let snoozed = state.snoozed_until_unix.is_some_and(|until| until > now);
    if state.enabled
        && fresh
        && !snoozed
        && available.as_deref() != state.last_notified_version.as_deref()
    {
        let Some(version) = available else {
            return;
        };
        eprintln!(
            "AGS {version} is available (current {current_version}); run `ags upgrade check` then `ags upgrade plan`."
        );
        state.last_notified_version = Some(version);
        let _ = write_check_state(&state);
        return;
    }
    let failure_backoff = state.last_error.is_some()
        && state
            .last_attempt_at_unix
            .is_some_and(|last| now - last < FAILURE_RETRY_SECONDS);
    if !state.enabled || snoozed || fresh || failure_backoff {
        return;
    }
    state.last_attempt_at_unix = Some(now);
    if write_check_state(&state).is_err() {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    if Command::new(executable)
        .args(["upgrade", "check"])
        .env("AGS_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_err()
    {
        state.last_error = Some("spawn-failed".to_string());
        let _ = write_check_state(&state);
    }
}

pub fn inspect_setup_bundle(source_root: &Path) -> Result<Candidate> {
    inspect_source(source_root, false, None, None)
}

fn install_signed_candidate(channel: &str) -> Result<Candidate> {
    if channel != "stable" {
        return Err(Error::new(
            "upgrade_channel_invalid",
            "only the signed stable channel is supported",
        ));
    }
    let (index, index_hash) = fetch_latest_index()?;
    let triple = platform_triple()?;
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let asset_name = format!("ags-v{}-{triple}.{extension}", index.version);
    let asset = index
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| {
            Error::new(
                "upgrade_asset_missing",
                format!("signed release index has no {asset_name}"),
            )
        })?;
    let cache = versions_root()?.join(&index.version).join(&triple);
    if cache.is_dir() {
        let candidate = inspect_source(
            &cache,
            true,
            Some(&asset_name),
            Some((&asset.sha256, &index_hash)),
        )?;
        verify_cache_marker(&cache, &candidate)?;
        return Ok(candidate);
    }
    let parent = cache
        .parent()
        .ok_or_else(|| Error::new("upgrade_cache_invalid", "version cache has no parent"))?;
    fs::create_dir_all(parent).map_err(|e| crate::error::io("upgrade_cache_create_failed", &e))?;
    let stage = tempfile::Builder::new()
        .prefix(".ags-upgrade-")
        .tempdir_in(parent)
        .map_err(|e| crate::error::io("upgrade_stage_failed", &e))?;
    let asset_path = stage.path().join(&asset_name);
    let url = format!(
        "https://github.com/{REPOSITORY}/releases/download/v{}/{asset_name}",
        index.version
    );
    download_to(&url, &asset_path, MAX_ASSET_BYTES)?;
    verify_asset_checksum(&asset_path, &asset.sha256)?;
    let extracted = stage.path().join("extracted");
    fs::create_dir_all(&extracted).map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
    extract_archive(&asset_path, extension, &extracted)?;
    let candidate = inspect_source(
        &extracted,
        true,
        Some(&asset_name),
        Some((&asset.sha256, &index_hash)),
    )?;
    let staged_cache = stage.path().join("cache");
    materialize_into(&candidate, &staged_cache)?;
    fs::rename(&staged_cache, &cache)
        .map_err(|e| crate::error::io("upgrade_cache_activate_failed", &e))?;
    inspect_source(
        &cache,
        true,
        Some(&asset_name),
        Some((&asset.sha256, &index_hash)),
    )
}

fn fetch_latest_index() -> Result<(ReleaseIndex, String)> {
    let index_url =
        format!("https://github.com/{REPOSITORY}/releases/latest/download/release-index.json");
    let signature_url =
        format!("https://github.com/{REPOSITORY}/releases/latest/download/release-index.sig");
    let bytes = fetch_bytes(&index_url, MAX_INDEX_BYTES)?;
    let signature = fetch_bytes(&signature_url, MAX_INDEX_BYTES)?;
    verify_release_index(&bytes, &signature)?;
    let index: ReleaseIndex = serde_json::from_slice(&bytes)
        .map_err(|e| Error::new("upgrade_index_invalid", e.to_string()))?;
    validate_release_index(&index)?;
    Ok((index, sha256_bytes(&bytes)))
}

fn verify_release_index(bytes: &[u8], signature_bytes: &[u8]) -> Result<()> {
    let key = VerifyingKey::from_public_key_pem(PUBLIC_KEY_PEM)
        .map_err(|e| Error::new("upgrade_signature_key_invalid", e.to_string()))?;
    let decoded = BASE64
        .decode(String::from_utf8_lossy(signature_bytes).trim())
        .map_err(|e| Error::new("upgrade_signature_invalid", e.to_string()))?;
    let signature = Signature::from_slice(&decoded)
        .map_err(|e| Error::new("upgrade_signature_invalid", e.to_string()))?;
    key.verify(bytes, &signature).map_err(|_| {
        Error::new(
            "upgrade_signature_invalid",
            "release index signature rejected",
        )
    })
}

fn verify_asset_checksum(path: &Path, expected: &str) -> Result<()> {
    let observed = sha256_file(path)?;
    if observed != expected {
        return Err(Error::new(
            "upgrade_asset_checksum_mismatch",
            format!("expected {expected}, observed {observed}"),
        ));
    }
    Ok(())
}

fn validate_release_index(index: &ReleaseIndex) -> Result<()> {
    if index.schema_version != "1.0-signed-release-index"
        || index.channel != "stable"
        || index.repository != REPOSITORY
        || index.tag != format!("v{}", index.version)
        || index.commit.len() != 40
        || !index.commit.bytes().all(|b| b.is_ascii_hexdigit())
        || Version::parse(&index.version).is_err()
    {
        return Err(Error::new(
            "upgrade_index_invalid",
            "signed release index identity is invalid",
        ));
    }
    for asset in &index.assets {
        if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::new(
                "upgrade_index_invalid",
                "signed release index contains an invalid asset hash",
            ));
        }
    }
    Ok(())
}

fn inspect_source(
    source_root: &Path,
    signed: bool,
    asset_name: Option<&str>,
    signed_hashes: Option<(&str, &str)>,
) -> Result<Candidate> {
    let source_root = source_root.canonicalize().map_err(|e| {
        Error::new(
            "upgrade_source_missing",
            format!("cannot resolve {}: {e}", source_root.display()),
        )
    })?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let direct_binary = source_root.join(format!("ags{suffix}"));
    let target_binary = source_root
        .join("target")
        .join("release")
        .join(format!("ags{suffix}"));
    let binary_root = if direct_binary.is_file() {
        source_root.clone()
    } else if target_binary.is_file() {
        source_root.join("target").join("release")
    } else {
        return Err(Error::new(
            "upgrade_binary_inventory_missing",
            format!(
                "{} contains no release AGS binary bundle",
                source_root.display()
            ),
        ));
    };
    let runtime_root = if source_root.join("runtime/ags-skills").is_dir() {
        source_root.join("runtime")
    } else if source_root.join("ags-skills").is_dir() {
        source_root.clone()
    } else {
        return Err(Error::new(
            "upgrade_runtime_profile_missing",
            "candidate contains no runtime/ags-skills or ags-skills profile",
        ));
    };
    for skill in OFFICIAL_SKILLS {
        for relative in ["SKILL.md", "agents/openai.yaml"] {
            let required = runtime_root.join("ags-skills").join(skill).join(relative);
            if !required.is_file() {
                return Err(Error::new(
                    "upgrade_runtime_profile_incomplete",
                    format!("candidate is missing ags-skills/{skill}/{relative}"),
                ));
            }
        }
    }
    let mut binaries = BTreeMap::new();
    let mut version: Option<String> = None;
    for name in BINARIES {
        let file_name = format!("{name}{suffix}");
        let path = binary_root.join(&file_name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            Error::new(
                "upgrade_binary_inventory_missing",
                format!("missing {file_name}"),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(Error::new(
                "upgrade_binary_inventory_invalid",
                format!("{file_name} is not a regular file"),
            ));
        }
        let observed_version = executable_version(&path)?;
        match &version {
            Some(expected) if expected != &observed_version => {
                return Err(Error::new(
                    "upgrade_binary_version_mismatch",
                    format!("{file_name} reports {observed_version}, expected {expected}"),
                ))
            }
            None => version = Some(observed_version),
            _ => {}
        }
        binaries.insert(file_name, sha256_file(&path)?);
    }
    let version = version.ok_or_else(|| {
        Error::new(
            "upgrade_binary_inventory_missing",
            "candidate has no version",
        )
    })?;
    let executables_sha256 = hash_named_files(&binaries);
    let runtime_sha256 = sha256_runtime(&runtime_root.join("ags-skills"))?;
    let triple = platform_triple()?;
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let default_asset = format!("ags-v{version}-{triple}.{extension}");
    let (asset_sha256, release_index_sha256) = signed_hashes
        .map(|(asset, index)| (asset.to_string(), index.to_string()))
        .unwrap_or_else(|| {
            let identity =
                sha256_bytes(format!("{executables_sha256}\n{runtime_sha256}\n").as_bytes());
            (identity.clone(), identity)
        });
    Ok(Candidate {
        version,
        triple,
        source_root,
        binary_root,
        runtime_root,
        binaries,
        executables_sha256,
        runtime_sha256,
        asset_name: asset_name.unwrap_or(&default_asset).to_string(),
        asset_sha256,
        release_index_sha256,
        signed,
    })
}

fn materialize_candidate(candidate: &Candidate) -> Result<Candidate> {
    let destination = versions_root()?
        .join(&candidate.version)
        .join(&candidate.triple);
    if destination.is_dir() {
        let cached = inspect_source(
            &destination,
            candidate.signed,
            Some(&candidate.asset_name),
            Some((&candidate.asset_sha256, &candidate.release_index_sha256)),
        )?;
        verify_cache_marker(&destination, &cached)?;
        if cached.executables_sha256 != candidate.executables_sha256
            || cached.runtime_sha256 != candidate.runtime_sha256
        {
            return Err(Error::new(
                "upgrade_cache_collision",
                "immutable version cache contains different content",
            ));
        }
        return Ok(cached);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| Error::new("upgrade_cache_invalid", "cache path has no parent"))?;
    fs::create_dir_all(parent).map_err(|e| crate::error::io("upgrade_cache_create_failed", &e))?;
    let stage = tempfile::Builder::new()
        .prefix(".ags-upgrade-")
        .tempdir_in(parent)
        .map_err(|e| crate::error::io("upgrade_stage_failed", &e))?;
    let staged = stage.path().join("candidate");
    materialize_into(candidate, &staged)?;
    fs::rename(&staged, &destination)
        .map_err(|e| crate::error::io("upgrade_cache_activate_failed", &e))?;
    inspect_source(
        &destination,
        candidate.signed,
        Some(&candidate.asset_name),
        Some((&candidate.asset_sha256, &candidate.release_index_sha256)),
    )
}

fn materialize_into(candidate: &Candidate, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination.join("runtime/ags-skills"))
        .map_err(|e| crate::error::io("upgrade_stage_failed", &e))?;
    for name in candidate.binaries.keys() {
        let source = candidate.binary_root.join(name);
        let target = destination.join(name);
        fs::copy(&source, &target).map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .map_err(|e| crate::error::io("upgrade_permissions_failed", &e))?;
        }
    }
    copy_tree(
        &candidate.runtime_root.join("ags-skills"),
        &destination.join("runtime/ags-skills"),
    )?;
    let marker = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        candidate.asset_name,
        candidate.asset_sha256,
        candidate
            .binaries
            .get(if cfg!(windows) { "ags.exe" } else { "ags" })
            .unwrap_or(&candidate.executables_sha256),
        candidate.executables_sha256,
        candidate.runtime_sha256,
        candidate.release_index_sha256,
    );
    fs::write(destination.join(".verified-sha256"), marker)
        .map_err(|e| crate::error::io("upgrade_marker_failed", &e))?;
    Ok(())
}

fn verify_cache_marker(root: &Path, candidate: &Candidate) -> Result<()> {
    let marker_path = root.join(".verified-sha256");
    let text = fs::read_to_string(&marker_path)
        .map_err(|e| crate::error::io("upgrade_cache_marker_missing", &e))?;
    let lines: Vec<&str> = text.lines().collect();
    let ags_name = executable_name("ags");
    let expected = [
        candidate.asset_name.as_str(),
        candidate.asset_sha256.as_str(),
        candidate.binaries[&ags_name].as_str(),
        candidate.executables_sha256.as_str(),
        candidate.runtime_sha256.as_str(),
        candidate.release_index_sha256.as_str(),
    ];
    if lines != expected {
        return Err(Error::new(
            "upgrade_cache_marker_mismatch",
            format!(
                "verified cache marker does not match runtime content at {}",
                root.display()
            ),
        ));
    }
    Ok(())
}

fn activate_launcher(previous: &RuntimePointer, next: &RuntimePointer) -> Result<()> {
    write_pointer(&launcher_previous_path()?, previous)?;
    if let Err(error) = write_pointer(&launcher_current_path()?, next) {
        let _ = write_pointer(&launcher_current_path()?, previous);
        return Err(error);
    }
    if let Err(error) = write_common_pointers(previous, next) {
        let _ = write_pointer(&launcher_current_path()?, previous);
        let _ = write_pointer(&common_current_path()?, previous);
        return Err(error);
    }
    Ok(())
}

fn activate_native(
    previous: &RuntimePointer,
    candidate: &Candidate,
    target: &Path,
    plan_hash: &str,
) -> Result<()> {
    fs::create_dir_all(target).map_err(|e| crate::error::io("upgrade_target_failed", &e))?;
    recover_native_if_needed(target)?;
    let previous_candidate = inspect_cached_pointer(previous)?;
    for (name, expected_hash) in &previous_candidate.binaries {
        let live = target.join(name);
        if !live.is_file() || sha256_file(&live)? != *expected_hash {
            return Err(Error::new(
                "upgrade_native_target_mismatch",
                format!(
                    "native target {} does not contain the active {name}; repair the installation before upgrading",
                    target.display()
                ),
            ));
        }
    }
    let unique = format!("{}-{}", std::process::id(), now_unix());
    let stage = target.join(format!(".ags-upgrade-stage-{unique}"));
    let backup = target.join(format!(".ags-upgrade-backup-{unique}"));
    if stage.exists() || backup.exists() {
        return Err(Error::new(
            "upgrade_stage_collision",
            "upgrade stage or backup already exists",
        ));
    }
    fs::create_dir(&stage).map_err(|e| crate::error::io("upgrade_stage_failed", &e))?;
    fs::create_dir(&backup).map_err(|e| crate::error::io("upgrade_backup_failed", &e))?;
    let mut journal = NativeActivationJournal {
        schema_version: 1,
        plan_hash: plan_hash.to_string(),
        target_root: target.to_path_buf(),
        stage_root: stage.clone(),
        backup_root: backup.clone(),
        previous: previous.clone(),
        candidate_hashes: candidate.binaries.clone(),
        moved: Vec::new(),
    };
    if let Err(error) = write_native_journal(target, &journal) {
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_dir_all(&backup);
        return Err(error);
    }
    let result = (|| {
        for name in candidate.binaries.keys() {
            let staged = stage.join(name);
            fs::copy(candidate.binary_root.join(name), &staged)
                .map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
            File::open(&staged)
                .and_then(|file| file.sync_all())
                .map_err(|e| crate::error::io("upgrade_stage_sync_failed", &e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
                    .map_err(|e| crate::error::io("upgrade_permissions_failed", &e))?;
            }
            if sha256_file(&staged)? != candidate.binaries[name] {
                return Err(Error::new(
                    "upgrade_stage_hash_mismatch",
                    format!("staged {name} hash mismatch"),
                ));
            }
        }
        let ags_name = executable_name("ags");
        let mut activation_order: Vec<String> = candidate
            .binaries
            .keys()
            .filter(|name| *name != &ags_name)
            .cloned()
            .collect();
        // The user-facing `ags` executable is switched last. An interruption
        // before that point leaves the old CLI runnable; the journal restores
        // all helper binaries before another new-runtime command proceeds.
        activation_order.push(ags_name);
        for name in &activation_order {
            let live = target.join(name);
            if live.exists() {
                fs::rename(&live, backup.join(name))
                    .map_err(|e| crate::error::io("upgrade_backup_failed", &e))?;
            }
            fs::rename(stage.join(name), &live)
                .map_err(|e| crate::error::io("upgrade_activate_failed", &e))?;
            journal.moved.push(name.clone());
            write_native_journal(target, &journal)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match restore_native_from_journal(target) {
            Ok(()) => Err(error),
            Err(rollback) => Err(Error::new(
                "upgrade_activation_rollback_failed",
                format!("{}; rollback failed: {}", error.message, rollback.message),
            )),
        };
    }
    Ok(())
}

fn rollback_activation(kind: &str, previous: &RuntimePointer, target: &Path) -> Result<()> {
    let previous_candidate = inspect_cached_pointer(previous)?;
    if kind == "launcher" {
        write_pointer(&launcher_current_path()?, previous)?;
        write_pointer(&common_current_path()?, previous)?;
    } else {
        restore_native_from_journal(target)?;
        write_pointer(&common_current_path()?, previous)?;
    }
    crate::sync::setup(&previous_candidate.runtime_root)?;
    Ok(())
}

fn with_rollback_result(code: &'static str, cause: &Error, rollback: Result<()>) -> Error {
    match rollback {
        Ok(()) => Error::new(
            code,
            format!("{}; previous runtime restored", cause.message),
        ),
        Err(error) => Error::new(
            "upgrade_rollback_failed",
            format!("{}; rollback failed: {}", cause.message, error.message),
        ),
    }
}

fn native_journal_path(target: &Path) -> PathBuf {
    target.join(".ags-upgrade-journal.json")
}

fn write_native_journal(target: &Path, journal: &NativeActivationJournal) -> Result<()> {
    atomic_write_json(
        &native_journal_path(target),
        journal,
        "upgrade_journal_write_failed",
    )
}

fn read_native_journal(target: &Path) -> Result<Option<NativeActivationJournal>> {
    let path = native_journal_path(target);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| crate::error::io("upgrade_journal_read_failed", &e))?;
    let journal: NativeActivationJournal = serde_json::from_str(&text)
        .map_err(|e| Error::new("upgrade_journal_invalid", e.to_string()))?;
    if journal.schema_version != 1 || journal.target_root != target {
        return Err(Error::new(
            "upgrade_journal_invalid",
            "native activation journal does not match the target root",
        ));
    }
    Ok(Some(journal))
}

fn restore_native_from_journal(target: &Path) -> Result<()> {
    let Some(journal) = read_native_journal(target)? else {
        return Err(Error::new(
            "upgrade_journal_missing",
            "native activation journal is missing; refusing to claim rollback",
        ));
    };
    for name in journal.candidate_hashes.keys().rev() {
        let live = target.join(name);
        let old = journal.backup_root.join(name);
        if old.is_file() {
            if live.exists() {
                fs::remove_file(&live)
                    .map_err(|e| crate::error::io("upgrade_rollback_failed", &e))?;
            }
            fs::rename(&old, &live).map_err(|e| crate::error::io("upgrade_rollback_failed", &e))?;
        } else if live.is_file()
            && sha256_file(&live).ok() == journal.candidate_hashes.get(name).cloned()
        {
            fs::remove_file(&live).map_err(|e| crate::error::io("upgrade_rollback_failed", &e))?;
        }
    }
    sync_directory(target, "upgrade_rollback_failed")?;
    cleanup_native_journal(&journal)
}

fn commit_native_activation(target: &Path) -> Result<()> {
    let journal = read_native_journal(target)?.ok_or_else(|| {
        Error::new(
            "upgrade_journal_missing",
            "native activation journal is missing at commit",
        )
    })?;
    cleanup_native_journal(&journal)
}

fn cleanup_native_journal(journal: &NativeActivationJournal) -> Result<()> {
    if journal.stage_root.exists() {
        fs::remove_dir_all(&journal.stage_root)
            .map_err(|e| crate::error::io("upgrade_stage_cleanup_failed", &e))?;
    }
    if journal.backup_root.exists() {
        fs::remove_dir_all(&journal.backup_root)
            .map_err(|e| crate::error::io("upgrade_backup_cleanup_failed", &e))?;
    }
    fs::remove_file(native_journal_path(&journal.target_root))
        .map_err(|e| crate::error::io("upgrade_journal_cleanup_failed", &e))?;
    sync_directory(&journal.target_root, "upgrade_journal_cleanup_failed")
}

fn recover_native_if_needed(target: &Path) -> Result<()> {
    let Some(journal) = read_native_journal(target)? else {
        return Ok(());
    };
    let receipt_exists = upgrade_state_root()?
        .join("receipts")
        .join(format!("{}.json", journal.plan_hash))
        .is_file();
    if receipt_exists {
        cleanup_native_journal(&journal)
    } else {
        restore_native_from_journal(target)?;
        write_pointer(&common_current_path()?, &journal.previous)?;
        let previous = inspect_cached_pointer(&journal.previous)?;
        crate::sync::setup(&previous.runtime_root)?;
        Ok(())
    }
}

pub fn recover_interrupted_activation() -> Result<()> {
    if std::env::var_os("AGS_UPGRADE_ACTIVATION_PROBE").is_some() {
        return Ok(());
    }
    if install_kind() == "native" {
        recover_native_if_needed(&native_target_root()?)?;
    }
    Ok(())
}

fn ensure_current_pointer() -> Result<RuntimePointer> {
    if let Some(pointer) = current_pointer()? {
        verify_pointer(&pointer)?;
        return Ok(pointer);
    }
    let source = crate::sync::install_info()?.source_root;
    let mut current = inspect_source(&source, false, None, None)?;
    let current_exe = std::env::current_exe()
        .map_err(|e| Error::new("upgrade_current_exe_failed", e.to_string()))?;
    let parent = current_exe.parent().ok_or_else(|| {
        Error::new(
            "upgrade_current_exe_failed",
            "current executable has no parent directory",
        )
    })?;
    let missing: Vec<String> = BINARIES
        .iter()
        .map(|name| executable_name(name))
        .filter(|name| !parent.join(name).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(Error::new(
            "upgrade_current_inventory_incomplete",
            format!(
                "active binary directory {} is missing {}; repair the installation before upgrading",
                parent.display(),
                missing.join(", ")
            ),
        ));
    }
    current.binary_root = parent.to_path_buf();
    current = inspect_source_with_roots(&current.source_root, parent, &current.runtime_root)?;
    let cached = materialize_candidate(&current)?;
    let pointer = pointer_from_candidate(&cached);
    write_pointer(&common_current_path()?, &pointer)?;
    if install_kind() == "launcher" {
        write_pointer(&launcher_current_path()?, &pointer)?;
    }
    Ok(pointer)
}

fn inspect_source_with_roots(source: &Path, binaries: &Path, runtime: &Path) -> Result<Candidate> {
    let synthetic =
        tempfile::tempdir().map_err(|e| crate::error::io("upgrade_probe_failed", &e))?;
    let root = synthetic.path();
    for name in BINARIES {
        let file_name = executable_name(name);
        fs::copy(binaries.join(&file_name), root.join(&file_name))
            .map_err(|e| crate::error::io("upgrade_probe_failed", &e))?;
    }
    copy_tree(&runtime.join("ags-skills"), &root.join("ags-skills"))?;
    let mut candidate = inspect_source(root, false, None, None)?;
    candidate.source_root = source.to_path_buf();
    candidate.binary_root = binaries.to_path_buf();
    candidate.runtime_root = runtime.to_path_buf();
    Ok(candidate)
}

fn inspect_cached_pointer(pointer: &RuntimePointer) -> Result<Candidate> {
    let root = versions_root()?
        .join(&pointer.version)
        .join(&pointer.triple);
    let candidate = inspect_source(
        &root,
        pointer.release_index_sha256 != pointer.asset_sha256,
        Some(&pointer.asset_name),
        Some((&pointer.asset_sha256, &pointer.release_index_sha256)),
    )?;
    verify_cache_marker(&root, &candidate)?;
    if candidate.executables_sha256 != pointer.executables_sha256
        || candidate.runtime_sha256 != pointer.runtime_sha256
    {
        return Err(Error::new(
            "upgrade_pointer_drift",
            "cached runtime does not match pointer hashes",
        ));
    }
    Ok(candidate)
}

fn verify_pointer(pointer: &RuntimePointer) -> Result<()> {
    let candidate = inspect_cached_pointer(pointer)?;
    if candidate.version != pointer.version || candidate.triple != pointer.triple {
        return Err(Error::new(
            "upgrade_pointer_drift",
            "cached runtime identity does not match pointer",
        ));
    }
    Ok(())
}

fn verify_activation(kind: &str, pointer: &RuntimePointer, target: &Path) -> Result<()> {
    verify_pointer(pointer)?;
    if kind == "launcher" {
        return Ok(());
    }
    let candidate = inspect_cached_pointer(pointer)?;
    for (name, expected_hash) in &candidate.binaries {
        let live = target.join(name);
        if sha256_file(&live)? != *expected_hash {
            return Err(Error::new(
                "upgrade_active_hash_mismatch",
                format!("active {name} does not match the sealed candidate"),
            ));
        }
        if executable_version(&live)? != candidate.version {
            return Err(Error::new(
                "upgrade_active_version_mismatch",
                format!("active {name} does not report {}", candidate.version),
            ));
        }
    }
    Ok(())
}

fn pointer_from_candidate(candidate: &Candidate) -> RuntimePointer {
    let binary_name = executable_name("ags");
    RuntimePointer {
        schema_version: 1,
        version: candidate.version.clone(),
        triple: candidate.triple.clone(),
        binary_name: binary_name.clone(),
        asset_name: candidate.asset_name.clone(),
        asset_sha256: candidate.asset_sha256.clone(),
        binary_sha256: candidate.binaries[&binary_name].clone(),
        executables_sha256: candidate.executables_sha256.clone(),
        runtime_sha256: candidate.runtime_sha256.clone(),
        release_index_sha256: candidate.release_index_sha256.clone(),
        runtime_root: candidate.runtime_root.clone(),
        activated_at_unix: now_unix(),
    }
}

fn current_pointer() -> Result<Option<RuntimePointer>> {
    if install_kind() == "launcher" {
        if let Some(pointer) = read_pointer(&launcher_current_path()?)? {
            return Ok(Some(pointer));
        }
    }
    read_pointer(&common_current_path()?)
}

fn previous_pointer() -> Result<Option<RuntimePointer>> {
    if install_kind() == "launcher" {
        if let Some(pointer) = read_pointer(&launcher_previous_path()?)? {
            return Ok(Some(pointer));
        }
    }
    read_pointer(&common_previous_path()?)
}

fn write_common_pointers(previous: &RuntimePointer, next: &RuntimePointer) -> Result<()> {
    write_pointer(&common_previous_path()?, previous)?;
    write_pointer(&common_current_path()?, next)
}

fn read_pointer(path: &Path) -> Result<Option<RuntimePointer>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .map_err(|e| crate::error::io("upgrade_pointer_read_failed", &e))?;
    let mut pointer: RuntimePointer = serde_json::from_str(&text)
        .map_err(|e| Error::new("upgrade_pointer_invalid", e.to_string()))?;
    if pointer.executables_sha256.is_empty() || pointer.runtime_root.as_os_str().is_empty() {
        let root = versions_root()?
            .join(&pointer.version)
            .join(&pointer.triple);
        let candidate = inspect_source(
            &root,
            true,
            Some(&pointer.asset_name),
            Some((&pointer.asset_sha256, &pointer.release_index_sha256)),
        )?;
        verify_cache_marker(&root, &candidate)?;
        if candidate.runtime_sha256 != pointer.runtime_sha256
            || candidate.binaries[&pointer.binary_name] != pointer.binary_sha256
        {
            return Err(Error::new(
                "upgrade_pointer_drift",
                "launcher pointer does not match verified cache content",
            ));
        }
        pointer.executables_sha256 = candidate.executables_sha256;
        pointer.runtime_root = candidate.runtime_root;
    }
    Ok(Some(pointer))
}

fn write_pointer(path: &Path, pointer: &RuntimePointer) -> Result<()> {
    atomic_write_json(path, pointer, "upgrade_pointer_write_failed")
}

fn write_receipt(plan_hash: &str, receipt: &Value) -> Result<()> {
    atomic_write_json(
        &upgrade_state_root()?
            .join("receipts")
            .join(format!("{plan_hash}.json")),
        receipt,
        "upgrade_receipt_write_failed",
    )
}

fn read_receipt(plan_hash: &str) -> Result<Value> {
    let path = upgrade_state_root()?
        .join("receipts")
        .join(format!("{plan_hash}.json"));
    if !path.is_file() {
        return Ok(Value::Null);
    }
    let text = fs::read_to_string(path)
        .map_err(|e| crate::error::io("upgrade_receipt_read_failed", &e))?;
    serde_json::from_str(&text).map_err(|e| Error::new("upgrade_receipt_invalid", e.to_string()))
}

fn read_check_state() -> Result<UpgradeCheckState> {
    let path = check_state_path()?;
    if path.is_file() {
        return read_state_file(&path);
    }
    let legacy = launcher_state_root()?.join("update-check.json");
    if legacy.is_file() {
        let text = fs::read_to_string(&legacy)
            .map_err(|e| crate::error::io("upgrade_state_read_failed", &e))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| Error::new("upgrade_state_invalid", e.to_string()))?;
        let mut state = UpgradeCheckState::default();
        state.enabled = value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        state.channel = value
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or("stable")
            .to_string();
        state.last_checked_at_unix = value.get("last_checked_at_unix").and_then(Value::as_i64);
        state.last_attempt_at_unix = state.last_checked_at_unix;
        state.ignored_versions = value
            .get("ignored_versions")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        state.snoozed_until_unix = value.get("snoozed_until_unix").and_then(Value::as_i64);
        state.latest_version = value
            .get("latest_version")
            .and_then(Value::as_str)
            .map(str::to_string);
        state.last_error = value
            .get("last_error")
            .and_then(Value::as_str)
            .map(str::to_string);
        write_check_state(&state)?;
        return Ok(state);
    }
    Ok(UpgradeCheckState::default())
}

fn read_state_file(path: &Path) -> Result<UpgradeCheckState> {
    let text =
        fs::read_to_string(path).map_err(|e| crate::error::io("upgrade_state_read_failed", &e))?;
    serde_json::from_str(&text).map_err(|e| Error::new("upgrade_state_invalid", e.to_string()))
}

fn write_check_state(state: &UpgradeCheckState) -> Result<()> {
    atomic_write_json(&check_state_path()?, state, "upgrade_state_write_failed")
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T, code: &'static str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io(code, &e))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| Error::new(code, e.to_string()))?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp).map_err(|e| crate::error::io(code, &e))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| crate::error::io(code, &e))?;
    }
    fs::rename(&tmp, path).map_err(|e| crate::error::io(code, &e))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent, code)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path, code: &'static str) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| crate::error::io(code, &e))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path, _code: &'static str) -> Result<()> {
    Ok(())
}

fn fetch_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", concat!("ags/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| Error::new("upgrade_network_failed", e.to_string()))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| crate::error::io("upgrade_download_failed", &e))?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::new(
            "upgrade_download_too_large",
            format!("download exceeded {max_bytes} bytes"),
        ));
    }
    Ok(bytes)
}

fn download_to(url: &str, path: &Path, max_bytes: u64) -> Result<()> {
    let bytes = fetch_bytes(url, max_bytes)?;
    fs::write(path, bytes).map_err(|e| crate::error::io("upgrade_download_failed", &e))
}

fn extract_archive(asset: &Path, extension: &str, destination: &Path) -> Result<()> {
    if extension == "tar.gz" {
        let file = File::open(asset).map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        let mut total = 0_u64;
        let entries = archive
            .entries()
            .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
        for entry in entries {
            let mut entry = entry.map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            let path = entry
                .path()
                .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?
                .into_owned();
            let output = safe_archive_output(destination, &path)?;
            if entry.header().entry_type().is_dir() {
                fs::create_dir_all(&output)
                    .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
                continue;
            }
            if !entry.header().entry_type().is_file() {
                return Err(Error::new(
                    "upgrade_archive_unsafe",
                    "release archive contains a non-regular entry",
                ));
            }
            total = total.saturating_add(entry.size());
            if total > MAX_EXTRACTED_BYTES {
                return Err(Error::new(
                    "upgrade_archive_too_large",
                    "extracted release exceeds size limit",
                ));
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            }
            let mut target = File::create(&output)
                .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            io::copy(&mut entry, &mut target)
                .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            mark_release_executable(destination, &output)?;
        }
    } else if extension == "zip" {
        let file = File::open(asset).map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| Error::new("upgrade_extract_failed", e.to_string()))?;
        let mut total = 0_u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| Error::new("upgrade_extract_failed", e.to_string()))?;
            let enclosed = entry.enclosed_name().ok_or_else(|| {
                Error::new("upgrade_archive_unsafe", "zip entry escapes destination")
            })?;
            let output = safe_archive_output(destination, &enclosed)?;
            if entry.is_dir() {
                fs::create_dir_all(&output)
                    .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
                continue;
            }
            total = total.saturating_add(entry.size());
            if total > MAX_EXTRACTED_BYTES {
                return Err(Error::new(
                    "upgrade_archive_too_large",
                    "extracted release exceeds size limit",
                ));
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            }
            let mut target = File::create(&output)
                .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            io::copy(&mut entry, &mut target)
                .map_err(|e| crate::error::io("upgrade_extract_failed", &e))?;
            mark_release_executable(destination, &output)?;
        }
    } else {
        return Err(Error::new(
            "upgrade_archive_invalid",
            "release archive must be tar.gz or zip",
        ));
    }
    Ok(())
}

fn mark_release_executable(destination: &Path, output: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let relative = output
            .strip_prefix(destination)
            .map_err(|e| Error::new("upgrade_extract_failed", e.to_string()))?;
        if relative.components().count() == 1
            && BINARIES
                .iter()
                .map(|name| executable_name(name))
                .any(|name| relative == Path::new(&name))
        {
            fs::set_permissions(output, fs::Permissions::from_mode(0o755))
                .map_err(|e| crate::error::io("upgrade_permissions_failed", &e))?;
        }
    }
    Ok(())
}

fn safe_archive_output(destination: &Path, path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::new(
            "upgrade_archive_unsafe",
            format!("unsafe archive path {}", path.display()),
        ));
    }
    let first = path
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        });
    let allowed_binary = first.is_some_and(|name| {
        BINARIES
            .iter()
            .map(|binary| executable_name(binary))
            .any(|binary| binary == name)
    });
    if !allowed_binary && first != Some("runtime") {
        return Err(Error::new(
            "upgrade_archive_unsafe",
            format!("unexpected release entry {}", path.display()),
        ));
    }
    Ok(destination.join(path))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|e| crate::error::io("upgrade_runtime_profile_missing", &e))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            "upgrade_runtime_symlink_rejected",
            format!("{} is a symlink", source.display()),
        ));
    }
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
        }
        fs::copy(source, destination).map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
    for entry in fs::read_dir(source).map_err(|e| crate::error::io("upgrade_copy_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("upgrade_copy_failed", &e))?;
        copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

/// Match the npm launcher's runtime identity exactly: paths are rooted at the
/// release `runtime/` directory, include file sizes, and contain only the
/// public `ags-skills/` subtree.
fn sha256_runtime(skills_root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(skills_root, skills_root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = Sha256::new();
    for (relative, path) in files {
        hash.update(b"ags-skills/");
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(
            fs::metadata(&path)
                .map_err(|e| crate::error::io("upgrade_hash_failed", &e))?
                .len()
                .to_string()
                .as_bytes(),
        );
        hash.update([0]);
        hash.update(sha256_file(&path)?.as_bytes());
        hash.update(b"\n");
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let metadata =
        fs::symlink_metadata(current).map_err(|e| crate::error::io("upgrade_hash_failed", &e))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            "upgrade_runtime_symlink_rejected",
            format!("{} is a symlink", current.display()),
        ));
    }
    if metadata.is_file() {
        let relative = current
            .strip_prefix(root)
            .map_err(|e| Error::new("upgrade_hash_failed", e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        files.push((relative, current.to_path_buf()));
        return Ok(());
    }
    for entry in fs::read_dir(current).map_err(|e| crate::error::io("upgrade_hash_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("upgrade_hash_failed", &e))?;
        collect_files(root, &entry.path(), files)?;
    }
    Ok(())
}

fn executable_version(path: &Path) -> Result<String> {
    let output = Command::new(path)
        .arg("--version")
        .env("AGS_NO_UPDATE_CHECK", "1")
        .env("AGS_UPGRADE_ACTIVATION_PROBE", "1")
        .output()
        .map_err(|e| {
            Error::new(
                "upgrade_binary_start_failed",
                format!("{}: {e}", path.display()),
            )
        })?;
    if !output.status.success() {
        return Err(Error::new(
            "upgrade_binary_start_failed",
            format!("{} --version exited {}", path.display(), output.status),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .map(|word| {
            word.trim_start_matches('v')
                .trim_matches(|c: char| c == '(' || c == ')')
        })
        .find(|word| Version::parse(word).is_ok())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::new(
                "upgrade_binary_version_invalid",
                format!("{} --version returned no semantic version", path.display()),
            )
        })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| crate::error::io("upgrade_hash_failed", &e))?;
    let mut hash = Sha256::new();
    io::copy(&mut file, &mut hash).map_err(|e| crate::error::io("upgrade_hash_failed", &e))?;
    Ok(format!("{:x}", hash.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_named_files(files: &BTreeMap<String, String>) -> String {
    let mut hash = Sha256::new();
    for (name, digest) in files {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(digest.as_bytes());
        hash.update(b"\n");
    }
    format!("{:x}", hash.finalize())
}

fn canonical_json(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::new("upgrade_plan_invalid", e.to_string()))
}

fn required_i64(value: &Value, name: &str) -> Result<i64> {
    value
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::new("upgrade_plan_invalid", format!("plan requires {name}")))
}

fn platform_triple() -> Result<String> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => {
            return Err(Error::new(
                "upgrade_platform_unsupported",
                format!("unsupported platform {os}/{arch}"),
            ))
        }
    };
    Ok(triple.to_string())
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn install_kind() -> &'static str {
    if std::env::var("AGS_INSTALL_KIND").as_deref() == Ok("launcher") {
        "launcher"
    } else {
        "native"
    }
}

fn environment_binding() -> Result<UpgradeEnvironment> {
    let kind = install_kind().to_string();
    let install_source_root = crate::sync::install_info()?.source_root;
    Ok(UpgradeEnvironment {
        install_kind: kind.clone(),
        install_source_root: absolute_path(install_source_root)?,
        machine_root: absolute_path(machine_root()?)?,
        versions_root: absolute_path(versions_root()?)?,
        target_root: absolute_path(native_target_root()?)?,
        launcher_state_root: if kind == "launcher" {
            Some(absolute_path(launcher_state_root()?)?)
        } else {
            None
        },
    })
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|e| Error::new("upgrade_environment_invalid", e.to_string()))
    }
}

fn native_target_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGS_NATIVE_BIN_DIR") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
        .map_err(|e| Error::new("upgrade_current_exe_failed", e.to_string()))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::new("upgrade_current_exe_failed", "executable has no parent"))
}

fn machine_root() -> Result<PathBuf> {
    crate::sync::machine_home().map(|home| home.join(".ags"))
}

fn upgrade_state_root() -> Result<PathBuf> {
    Ok(machine_root()?.join("v3/upgrade"))
}

fn versions_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGS_VERSIONS_ROOT") {
        return Ok(PathBuf::from(path));
    }
    Ok(machine_root()?.join("versions"))
}

fn launcher_state_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGS_LAUNCHER_STATE_ROOT") {
        return Ok(PathBuf::from(path));
    }
    Ok(machine_root()?.join("launcher-state"))
}

fn launcher_current_path() -> Result<PathBuf> {
    Ok(launcher_state_root()?.join("current.json"))
}

fn launcher_previous_path() -> Result<PathBuf> {
    Ok(launcher_state_root()?.join("previous.json"))
}

fn common_current_path() -> Result<PathBuf> {
    Ok(upgrade_state_root()?.join("current.json"))
}

fn common_previous_path() -> Result<PathBuf> {
    Ok(upgrade_state_root()?.join("previous.json"))
}

fn check_state_path() -> Result<PathBuf> {
    Ok(machine_root()?.join("v3/upgrade-check.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifier_uses_seven_day_success_gate_and_daily_failure_backoff() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        let mut calls = 0;
        let first = check_at("0.4.20", false, 1_000_000, || {
            calls += 1;
            Ok((fixture_index("0.4.21"), "a".repeat(64)))
        });
        assert_eq!(first["available"]["version"], "0.4.21");
        let fresh = check_at("0.4.20", false, 1_000_100, || {
            calls += 1;
            Ok((fixture_index("0.4.21"), "a".repeat(64)))
        });
        assert_eq!(fresh["skipped"], "fresh");
        assert_eq!(calls, 1);

        let state = UpgradeCheckState {
            last_attempt_at_unix: Some(2_000_000),
            last_error: Some("offline".to_string()),
            ..UpgradeCheckState::default()
        };
        write_check_state(&state).unwrap();
        let backed_off = check_at("0.4.20", false, 2_000_100, || {
            panic!("failure backoff must not fetch")
        });
        assert_eq!(backed_off["skipped"], "failure-backoff");
    }

    #[test]
    fn notifier_controls_cover_disable_snooze_ignore_and_reset() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());

        configure(Some(false), None, None, false).unwrap();
        let disabled = check_at("0.4.20", false, 1_000, || panic!("disabled must not fetch"));
        assert_eq!(disabled["skipped"], "disabled");

        configure(Some(true), Some("0.4.21"), Some(5_000), false).unwrap();
        let snoozed = check_at("0.4.20", false, 2_000, || panic!("snoozed must not fetch"));
        assert_eq!(snoozed["skipped"], "snoozed");
        configure(None, None, Some(0), false).unwrap();
        let ignored = check_at("0.4.20", false, 6_000, || {
            Ok((fixture_index("0.4.21"), "a".repeat(64)))
        });
        assert!(ignored["available"].is_null());

        let reset = configure(None, None, None, true).unwrap();
        assert_eq!(reset["enabled"], true);
        assert_eq!(reset["ignored_versions"], json!([]));
        assert!(reset["snoozed_until_unix"].is_null());
    }

    #[test]
    fn invalid_release_signature_is_rejected_by_rust_authority() {
        let error = verify_release_index(b"{}", b"not-base64").unwrap_err();
        assert_eq!(error.code, "upgrade_signature_invalid");
    }

    #[test]
    fn signed_asset_hash_mismatch_is_rejected_before_extraction() {
        let root = tempfile::tempdir().unwrap();
        let asset = root.path().join("asset.tar.gz");
        fs::write(&asset, "tampered").unwrap();
        let error = verify_asset_checksum(&asset, &"0".repeat(64)).unwrap_err();
        assert_eq!(error.code, "upgrade_asset_checksum_mismatch");
    }

    #[test]
    fn automatic_notification_consumes_fresh_local_state_once() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::remove_var("AGS_NO_UPDATE_CHECK");
        let state = UpgradeCheckState {
            latest_version: Some("0.4.22".to_string()),
            last_checked_at_unix: Some(now_unix()),
            ..UpgradeCheckState::default()
        };
        write_check_state(&state).unwrap();
        maybe_notify("0.4.21");
        let notified = read_check_state().unwrap();
        assert_eq!(notified.last_notified_version.as_deref(), Some("0.4.22"));
        maybe_notify("0.4.21");
        assert_eq!(
            read_check_state().unwrap().last_notified_version,
            notified.last_notified_version
        );
    }

    #[test]
    fn legacy_notifier_state_migrates_without_losing_preferences() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        let legacy = home.path().join(".ags/launcher-state/update-check.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(
            &legacy,
            serde_json::to_vec(&json!({
                "schema_version": "ags://schema/contract/v3/update-check-state",
                "enabled": false,
                "channel": "stable",
                "last_checked_at_unix": 123,
                "ignored_versions": ["0.4.21"],
                "snoozed_until_unix": 456,
                "latest_version": "0.4.21",
                "last_error": null,
            }))
            .unwrap(),
        )
        .unwrap();
        let state = read_check_state().unwrap();
        assert!(!state.enabled);
        assert_eq!(state.ignored_versions, vec!["0.4.21"]);
        assert_eq!(state.last_attempt_at_unix, Some(123));
        assert!(home.path().join(".ags/v3/upgrade-check.json").is_file());
    }

    #[test]
    fn archive_paths_reject_traversal_and_unknown_roots() {
        let root = Path::new("/tmp/ags-test-extract");
        assert!(safe_archive_output(root, Path::new("../escape")).is_err());
        assert!(safe_archive_output(root, Path::new("secret.txt")).is_err());
        assert!(safe_archive_output(root, Path::new("runtime/ags-skills/x/SKILL.md")).is_ok());
        assert!(safe_archive_output(root, Path::new(&executable_name("ags"))).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn tar_release_extracts_runnable_five_binary_inventory() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("fixture.tar.gz");
        let file = File::create(&archive_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for name in BINARIES {
            let body = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"{name} v0.4.21\"; fi\nexit 0\n"
            );
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, executable_name(name), body.as_bytes())
                .unwrap();
        }
        for skill_name in OFFICIAL_SKILLS {
            for (relative, body) in [
                (
                    "SKILL.md",
                    format!("---\nname: {skill_name}\ndescription: fixture\n---\n"),
                ),
                (
                    "agents/openai.yaml",
                    format!("interface:\n  display_name: {skill_name}\n"),
                ),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                archive
                    .append_data(
                        &mut header,
                        format!("runtime/ags-skills/{skill_name}/{relative}"),
                        body.as_bytes(),
                    )
                    .unwrap();
            }
        }
        archive.into_inner().unwrap().finish().unwrap();

        let extracted = root.path().join("extracted");
        fs::create_dir(&extracted).unwrap();
        extract_archive(&archive_path, "tar.gz", &extracted).unwrap();
        assert_eq!(
            fs::metadata(extracted.join("ags"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
        let candidate = inspect_source(&extracted, false, None, None).unwrap();
        assert_eq!(candidate.version, "0.4.21");
        assert_eq!(candidate.binaries.len(), 5);
    }

    #[test]
    fn setup_inventory_requires_all_five_version_aligned_binaries() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        write_fixture_runtime(root.path(), "0.4.21", "inventory");
        for name in ["ags-mcp", "ags-host", "ags-policy", "ags-release"] {
            fs::remove_file(root.path().join(executable_name(name))).unwrap();
        }
        let error = inspect_setup_bundle(root.path()).unwrap_err();
        assert_eq!(error.code, "upgrade_binary_inventory_missing");
    }

    #[test]
    fn setup_inventory_requires_the_complete_official_skill_profile() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("ags-skills/ags-setup/agents")).unwrap();
        fs::write(
            root.path().join("ags-skills/ags-setup/SKILL.md"),
            "---\nname: ags-setup\ndescription: fixture\n---\n",
        )
        .unwrap();
        fs::write(
            root.path().join("ags-skills/ags-setup/agents/openai.yaml"),
            "interface:\n  display_name: ags-setup\n",
        )
        .unwrap();
        for name in BINARIES {
            write_fixture_binary(root.path(), name, "0.4.21");
        }
        let error = inspect_setup_bundle(root.path()).unwrap_err();
        assert_eq!(error.code, "upgrade_runtime_profile_incomplete");
    }

    #[test]
    fn failed_setup_never_writes_the_install_commit_marker() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        write_fixture_runtime(source.path(), "0.4.21", "setup-failure");
        let conflict = home.path().join(".agents/skills/ags-demo");
        fs::create_dir_all(&conflict).unwrap();
        fs::write(conflict.join("SKILL.md"), "unmanaged\n").unwrap();
        assert!(crate::sync::setup(source.path()).is_err());
        let error = crate::sync::install_info().unwrap_err();
        assert_eq!(error.code, "install_info_missing");
    }

    #[test]
    fn rust_runtime_hash_matches_the_node_launcher_contract() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("ags-skills/demo");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("SKILL.md"), "abc\n").unwrap();
        fs::write(skills.join("openai.yaml"), "xy\n").unwrap();
        let mut expected = Sha256::new();
        for (relative, body) in [("SKILL.md", "abc\n"), ("openai.yaml", "xy\n")] {
            expected.update(format!(
                "ags-skills/demo/{relative}\0{}\0{}\n",
                body.len(),
                sha256_bytes(body.as_bytes())
            ));
        }
        assert_eq!(
            sha256_runtime(&root.path().join("ags-skills")).unwrap(),
            format!("{:x}", expected.finalize())
        );
    }

    #[test]
    fn source_plan_detects_drift_and_atomic_apply_can_recover() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::remove_var("AGS_INSTALL_KIND");
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("live"));

        let current_root = work.path().join("current");
        let target_root = work.path().join("target");
        write_fixture_runtime(&current_root, "0.4.20", "current");
        write_fixture_runtime(&target_root, "0.4.21", "target");
        let current = inspect_source(&current_root, false, None, None).unwrap();
        let cached_current = materialize_candidate(&current).unwrap();
        let current_pointer = pointer_from_candidate(&cached_current);
        write_pointer(&common_current_path().unwrap(), &current_pointer).unwrap();
        crate::sync::setup(&cached_current.runtime_root).unwrap();
        fs::create_dir_all(work.path().join("live")).unwrap();
        for name in current.binaries.keys() {
            fs::copy(
                current.binary_root.join(name),
                work.path().join("live").join(name),
            )
            .unwrap();
        }

        let workspace_root = work.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        fs::write(
            workspace_root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"upgrade\", \"update\"]\n",
        )
        .unwrap();
        let binding = crate::workspace::bind(&workspace_root).unwrap();
        let payload = prepare_plan(
            &binding,
            &json!({"source_root": target_root, "action": "activate"}),
        )
        .unwrap();
        let mut expired = payload.clone();
        expired["expires_at_unix"] = json!(0);
        let expired_error = apply(&expired, &binding).unwrap_err();
        assert_eq!(expired_error.code, "upgrade_plan_expired");
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("drifted-live"));
        let environment_drift = apply(&payload, &binding).unwrap_err();
        assert_eq!(environment_drift.code, "upgrade_environment_drift");
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("live"));
        fs::write(target_root.join("ags-skills/ags-demo/SKILL.md"), "drift\n").unwrap();
        let drift = apply(&payload, &binding).unwrap_err();
        assert_eq!(drift.code, "upgrade_candidate_drift");
        assert_eq!(
            sha256_file(&work.path().join("live").join(executable_name("ags"))).unwrap(),
            current.binaries[&executable_name("ags")]
        );

        write_fixture_runtime(&target_root, "0.4.22", "setup-failure-target");
        fs::create_dir_all(target_root.join("ags-skills/ags-blocked")).unwrap();
        fs::write(
            target_root.join("ags-skills/ags-blocked/SKILL.md"),
            "---\nname: ags-blocked\ndescription: fixture\n---\n",
        )
        .unwrap();
        let unmanaged = home.path().join(".agents/skills/ags-blocked");
        fs::create_dir_all(&unmanaged).unwrap();
        fs::write(unmanaged.join("SKILL.md"), "unmanaged\n").unwrap();
        let payload = prepare_plan(
            &binding,
            &json!({"source_root": target_root, "action": "activate"}),
        )
        .unwrap();
        let setup_failure = apply(&payload, &binding).unwrap_err();
        assert_eq!(setup_failure.code, "upgrade_setup_failed");
        assert_eq!(
            sha256_file(&work.path().join("live").join(executable_name("ags"))).unwrap(),
            current.binaries[&executable_name("ags")]
        );
        fs::remove_dir_all(unmanaged).unwrap();

        write_fixture_runtime(&target_root, "0.4.21", "target");
        let payload = prepare_plan(
            &binding,
            &json!({"source_root": target_root, "action": "activate"}),
        )
        .unwrap();
        let store = crate::seal::SealStore::new(&binding);
        let action = store
            .seal_plan(UPGRADE_OPERATION, &payload, &binding)
            .unwrap();
        store
            .apply_with_result(&action.token, &binding, |plan| {
                apply(&plan.payload, &binding)
            })
            .unwrap();
        let verification = verify(&action.token, &binding).unwrap();
        assert_eq!(verification["active_version"], "0.4.21");
        let target = inspect_source(&target_root, false, None, None).unwrap();
        assert_eq!(
            sha256_file(&work.path().join("live").join(executable_name("ags"))).unwrap(),
            target.binaries[&executable_name("ags")]
        );
        fs::write(
            work.path().join("live").join(executable_name("ags")),
            "tampered\n",
        )
        .unwrap();
        let active_drift = verify(&action.token, &binding).unwrap_err();
        assert_eq!(active_drift.code, "upgrade_active_hash_mismatch");
        fs::copy(
            target.binary_root.join(executable_name("ags")),
            work.path().join("live").join(executable_name("ags")),
        )
        .unwrap();

        let recovery = prepare_plan(&binding, &json!({"action": "recover"})).unwrap();
        let recovered = apply(&recovery, &binding).unwrap();
        assert_eq!(
            recovered.result.as_ref().unwrap()["active"]["version"],
            "0.4.20"
        );
        assert_eq!(
            sha256_file(&work.path().join("live").join(executable_name("ags"))).unwrap(),
            current.binaries[&executable_name("ags")]
        );
        std::env::remove_var("AGS_NATIVE_BIN_DIR");
    }

    #[test]
    fn interrupted_native_activation_restores_old_five_binary_set() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::remove_var("AGS_INSTALL_KIND");
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("live"));

        let old_root = work.path().join("old");
        let new_root = work.path().join("new");
        let live = work.path().join("live");
        write_fixture_runtime(&old_root, "0.4.20", "old");
        write_fixture_runtime(&new_root, "0.4.21", "new");
        let old = inspect_source(&old_root, false, None, None).unwrap();
        let cached_old = materialize_candidate(&old).unwrap();
        let previous = pointer_from_candidate(&cached_old);
        write_pointer(&common_current_path().unwrap(), &previous).unwrap();
        crate::sync::setup(&cached_old.runtime_root).unwrap();
        fs::create_dir_all(&live).unwrap();
        for name in old.binaries.keys() {
            fs::copy(old.binary_root.join(name), live.join(name)).unwrap();
        }
        let new = inspect_source(&new_root, false, None, None).unwrap();
        activate_native(&previous, &new, &live, &"f".repeat(64)).unwrap();
        let journal = read_native_journal(&live).unwrap().unwrap();
        assert_eq!(journal.moved.last(), Some(&executable_name("ags")));
        assert_eq!(
            executable_version(&live.join(executable_name("ags"))).unwrap(),
            "0.4.21"
        );

        std::env::set_var("AGS_UPGRADE_ACTIVATION_PROBE", "1");
        recover_interrupted_activation().unwrap();
        assert!(native_journal_path(&live).exists());
        std::env::remove_var("AGS_UPGRADE_ACTIVATION_PROBE");
        recover_interrupted_activation().unwrap();
        assert!(!native_journal_path(&live).exists());
        for (name, expected) in &old.binaries {
            assert_eq!(sha256_file(&live.join(name)).unwrap(), *expected);
        }
        std::env::remove_var("AGS_NATIVE_BIN_DIR");
    }

    #[test]
    fn rust_reads_and_migrates_a_node_launcher_pointer() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("AGS_INSTALL_KIND", "launcher");
        std::env::set_var(
            "AGS_LAUNCHER_STATE_ROOT",
            work.path().join("launcher-state"),
        );
        std::env::set_var("AGS_VERSIONS_ROOT", work.path().join("versions"));

        let source = work.path().join("source");
        write_fixture_runtime(&source, "0.4.21", "node-pointer");
        let candidate = inspect_source(&source, false, None, None).unwrap();
        let cached = materialize_candidate(&candidate).unwrap();
        let rust_pointer = pointer_from_candidate(&cached);
        let node_pointer = json!({
            "schema_version": 1,
            "version": rust_pointer.version,
            "triple": rust_pointer.triple,
            "binary_name": rust_pointer.binary_name,
            "asset_name": rust_pointer.asset_name,
            "asset_sha256": rust_pointer.asset_sha256,
            "binary_sha256": rust_pointer.binary_sha256,
            "runtime_sha256": rust_pointer.runtime_sha256,
            "release_index_sha256": rust_pointer.release_index_sha256,
            "activated_at": "2026-08-27T00:00:00.000Z"
        });
        atomic_write_json(
            &launcher_current_path().unwrap(),
            &node_pointer,
            "fixture_write_failed",
        )
        .unwrap();
        let migrated = read_pointer(&launcher_current_path().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(migrated.executables_sha256, cached.executables_sha256);
        assert_eq!(migrated.runtime_sha256, cached.runtime_sha256);
        assert_eq!(migrated.runtime_root, cached.runtime_root);
        fs::write(
            versions_root()
                .unwrap()
                .join("0.4.21")
                .join(platform_triple().unwrap())
                .join(".verified-sha256"),
            "tampered\n",
        )
        .unwrap();
        let marker_error = inspect_cached_pointer(&migrated).unwrap_err();
        assert_eq!(marker_error.code, "upgrade_cache_marker_mismatch");
        std::env::remove_var("AGS_INSTALL_KIND");
        std::env::remove_var("AGS_LAUNCHER_STATE_ROOT");
        std::env::remove_var("AGS_VERSIONS_ROOT");
    }

    #[test]
    fn launcher_activation_and_recover_use_the_same_rust_plan() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("AGS_INSTALL_KIND", "launcher");
        std::env::set_var(
            "AGS_LAUNCHER_STATE_ROOT",
            work.path().join("launcher-state"),
        );
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("unused-native"));

        let current_root = work.path().join("current");
        let target_root = work.path().join("target");
        write_fixture_runtime(&current_root, "0.4.20", "current");
        write_fixture_runtime(&target_root, "0.4.21", "target");
        let current = inspect_source(&current_root, false, None, None).unwrap();
        let cached = materialize_candidate(&current).unwrap();
        let pointer = pointer_from_candidate(&cached);
        write_pointer(&launcher_current_path().unwrap(), &pointer).unwrap();
        write_pointer(&common_current_path().unwrap(), &pointer).unwrap();
        crate::sync::setup(&cached.runtime_root).unwrap();

        let workspace_root = work.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        fs::write(
            workspace_root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"upgrade\", \"update\"]\n",
        )
        .unwrap();
        let binding = crate::workspace::bind(&workspace_root).unwrap();
        let payload = prepare_plan(
            &binding,
            &json!({"source_root": target_root, "action": "activate"}),
        )
        .unwrap();
        let applied = apply(&payload, &binding).unwrap();
        assert_eq!(
            applied.result.as_ref().unwrap()["active"]["version"],
            "0.4.21"
        );
        assert_eq!(
            read_pointer(&launcher_current_path().unwrap())
                .unwrap()
                .unwrap()
                .version,
            "0.4.21"
        );

        let recovery = prepare_plan(&binding, &json!({"action": "recover"})).unwrap();
        apply(&recovery, &binding).unwrap();
        assert_eq!(
            read_pointer(&launcher_current_path().unwrap())
                .unwrap()
                .unwrap()
                .version,
            "0.4.20"
        );
        std::env::remove_var("AGS_INSTALL_KIND");
        std::env::remove_var("AGS_LAUNCHER_STATE_ROOT");
        std::env::remove_var("AGS_NATIVE_BIN_DIR");
    }

    #[test]
    fn source_plan_rejects_downgrade_without_recover() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::remove_var("AGS_INSTALL_KIND");
        std::env::set_var("AGS_NATIVE_BIN_DIR", work.path().join("live"));

        let current_root = work.path().join("current");
        let older_root = work.path().join("older");
        write_fixture_runtime(&current_root, "0.4.21", "current");
        write_fixture_runtime(&older_root, "0.4.20", "older");
        let current = inspect_source(&current_root, false, None, None).unwrap();
        let cached = materialize_candidate(&current).unwrap();
        write_pointer(
            &common_current_path().unwrap(),
            &pointer_from_candidate(&cached),
        )
        .unwrap();
        crate::sync::setup(&cached.runtime_root).unwrap();

        let workspace_root = work.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        fs::write(
            workspace_root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"upgrade\", \"update\"]\n",
        )
        .unwrap();
        let binding = crate::workspace::bind(&workspace_root).unwrap();
        let error = prepare_plan(
            &binding,
            &json!({"source_root": older_root, "action": "activate"}),
        )
        .unwrap_err();
        assert_eq!(error.code, "upgrade_downgrade_requires_recover");
        std::env::remove_var("AGS_NATIVE_BIN_DIR");
    }

    fn write_fixture_runtime(root: &Path, version: &str, body: &str) {
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
        fs::create_dir_all(root.join("ags-skills/ags-demo")).unwrap();
        fs::write(
            root.join("ags-skills/ags-demo/SKILL.md"),
            format!("---\nname: ags-demo\ndescription: fixture {body}\n---\n# demo\n"),
        )
        .unwrap();
        for skill in OFFICIAL_SKILLS {
            fs::create_dir_all(root.join("ags-skills").join(skill).join("agents")).unwrap();
            fs::write(
                root.join("ags-skills").join(skill).join("SKILL.md"),
                format!("---\nname: {skill}\ndescription: fixture {body}\n---\n"),
            )
            .unwrap();
            fs::write(
                root.join("ags-skills")
                    .join(skill)
                    .join("agents/openai.yaml"),
                format!("interface:\n  display_name: {skill}\n"),
            )
            .unwrap();
        }
        for name in BINARIES {
            write_fixture_binary(root, name, version);
        }
    }

    fn write_fixture_binary(root: &Path, name: &str, version: &str) {
        let path = root.join(executable_name(name));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(
                &path,
                format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"{name} v{version}\"; fi\nexit 0\n"),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        {
            let _ = (path, name, version);
            panic!("fixture executable helper requires a Windows-specific test binary");
        }
    }

    fn fixture_index(version: &str) -> ReleaseIndex {
        ReleaseIndex {
            schema_version: "1.0-signed-release-index".to_string(),
            version: version.to_string(),
            channel: "stable".to_string(),
            repository: REPOSITORY.to_string(),
            tag: format!("v{version}"),
            commit: "a".repeat(40),
            assets: vec![],
        }
    }
}
