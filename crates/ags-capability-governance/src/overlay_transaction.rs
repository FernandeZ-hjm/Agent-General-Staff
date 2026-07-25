use super::*;
#[allow(unused_imports)]
use super::{
    authority::*, catalog::*, hashing::*, private_store::*, snapshot_compiler::*, usage_ledger::*,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayEntryState {
    Active,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserOverlayEntry {
    pub skill_id: String,
    pub state: OverlayEntryState,
    pub revision: u64,
    pub source_hash: String,
    pub metadata_version: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub entrypoints: Vec<String>,
    #[serde(default)]
    pub invoke_hint: String,
    #[serde(default)]
    pub requires_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSkillOverlay {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub entries: Vec<UserOverlayEntry>,
}

impl Default for UserSkillOverlay {
    fn default() -> Self {
        Self {
            schema_version: USER_OVERLAY_SCHEMA_VERSION.to_string(),
            revision: 0,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserSourceKind {
    Local,
    Github,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSourceEntry {
    pub skill_id: String,
    pub source_kind: UserSourceKind,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
    pub source_hash: String,
    pub license: String,
    pub canonical_path: String,
    pub audit_version: String,
    pub target_hosts: Vec<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub entrypoints: Vec<String>,
    #[serde(default)]
    pub requires_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSourceRegistry {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub entries: Vec<UserSourceEntry>,
}

impl Default for UserSourceRegistry {
    fn default() -> Self {
        Self {
            schema_version: USER_SOURCE_REGISTRY_SCHEMA_VERSION.to_string(),
            revision: 0,
            entries: Vec::new(),
        }
    }
}

pub fn user_source_registry_path(runtime_home: &Path) -> PathBuf {
    runtime_home
        .join("skill-registry")
        .join("user-sources.yaml")
}

pub fn user_skill_body_root(runtime_home: &Path) -> PathBuf {
    runtime_home.join("skill-bodies")
}

pub fn load_user_source_registry(runtime_home: &Path) -> Result<UserSourceRegistry, String> {
    let path = user_source_registry_path(runtime_home);
    if !path.exists() {
        return Ok(UserSourceRegistry::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let registry: UserSourceRegistry = serde_yaml::from_str(&content)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if registry.schema_version != USER_SOURCE_REGISTRY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported source registry schema {}; expected {USER_SOURCE_REGISTRY_SCHEMA_VERSION}",
            registry.schema_version
        ));
    }
    let body_root = user_skill_body_root(runtime_home);
    let real_body_root = std::fs::canonicalize(&body_root).map_err(|error| {
        format!(
            "cannot canonicalize user skill body root {}: {error}",
            body_root.display()
        )
    })?;
    let mut seen = HashSet::new();
    for entry in &registry.entries {
        if !seen.insert(entry.skill_id.clone()) {
            return Err("duplicate skill_id in user source registry".to_string());
        }
        if !safe_skill_id(&entry.skill_id)
            || !is_sha256(&entry.source_hash)
            || !is_known_source_license(&entry.license)
            || entry.summary.trim().is_empty()
            || entry.intent_tags.is_empty()
            || entry.audit_version != USER_SOURCE_AUDIT_VERSION
        {
            return Err(format!(
                "invalid metadata for user source {}",
                entry.skill_id
            ));
        }
        let mut seen_hosts = HashSet::new();
        if entry.target_hosts.is_empty()
            || entry.target_hosts.iter().any(|host| {
                !matches!(
                    host.as_str(),
                    "claude-code" | "codex" | "omp" | "codebuddy-code" | "cursor"
                ) || !seen_hosts.insert(host.as_str())
            })
        {
            return Err(format!(
                "invalid target_hosts for user source {}",
                entry.skill_id
            ));
        }
        match entry.source_kind {
            UserSourceKind::Github if !valid_github_source_provenance(entry) => {
                return Err(format!(
                    "github user source {} is not safely pinned",
                    entry.skill_id
                ));
            }
            UserSourceKind::Local
                if entry.resolved_ref.is_some()
                    || entry.subdir.is_some()
                    || !Path::new(&entry.source).is_absolute() =>
            {
                return Err(format!(
                    "local user source {} has invalid provenance",
                    entry.skill_id
                ));
            }
            _ => {}
        }
        let canonical = std::fs::canonicalize(&entry.canonical_path).map_err(|error| {
            format!(
                "cannot canonicalize source body {}: {error}",
                entry.canonical_path
            )
        })?;
        if !canonical.starts_with(&real_body_root) {
            return Err(format!(
                "user source {} escapes the private body store",
                entry.skill_id
            ));
        }
        let expected = std::fs::canonicalize(body_root.join(&entry.skill_id)).map_err(|error| {
            format!(
                "cannot canonicalize expected source body for {}: {error}",
                entry.skill_id
            )
        })?;
        if canonical != expected {
            return Err(format!(
                "user source {} does not use its canonical private body-store location",
                entry.skill_id
            ));
        }
        let metadata = load_skill_metadata_path(&canonical.join("SKILL.md"));
        if metadata.name != entry.skill_id {
            return Err(format!(
                "user source {} SKILL.md declares a different canonical name",
                entry.skill_id
            ));
        }
    }
    Ok(registry)
}

pub fn write_user_source_registry(
    runtime_home: &Path,
    registry: &UserSourceRegistry,
) -> Result<(), String> {
    let mut registry = registry.clone();
    registry
        .entries
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    for entry in &mut registry.entries {
        entry.intent_tags.sort();
        entry.intent_tags.dedup();
        entry.entrypoints.sort();
        entry.entrypoints.dedup();
        entry.target_hosts.sort();
        entry.target_hosts.dedup();
    }
    let serialized = serde_yaml::to_string(&registry)
        .map_err(|error| format!("cannot serialize source registry: {error}"))?;
    write_private_atomic(
        &user_source_registry_path(runtime_home),
        serialized.as_bytes(),
    )
}

pub fn load_user_overlay(runtime_home: &Path) -> Result<UserSkillOverlay, String> {
    let path = overlay_path(runtime_home);
    if !path.exists() {
        return Ok(UserSkillOverlay::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let overlay: UserSkillOverlay = serde_yaml::from_str(&content)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if overlay.schema_version != USER_OVERLAY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported overlay schema {}; expected {USER_OVERLAY_SCHEMA_VERSION}",
            overlay.schema_version
        ));
    }
    let mut seen = HashSet::new();
    if overlay
        .entries
        .iter()
        .any(|entry| !seen.insert(entry.skill_id.clone()))
    {
        return Err("duplicate skill_id in user overlay".to_string());
    }
    Ok(overlay)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMutationOperation {
    Adopt,
    Ignore,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayMutationReceipt {
    pub schema_version: String,
    pub event_id: String,
    pub timestamp_unix: u64,
    pub operation: OverlayMutationOperation,
    pub skill_id: String,
    pub from_overlay_revision: u64,
    pub to_overlay_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_entry: Option<UserOverlayEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_entry: Option<UserOverlayEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayMutationResult {
    pub schema_version: String,
    pub operation: OverlayMutationOperation,
    pub skill_id: String,
    pub dry_run: bool,
    pub applied: bool,
    pub changed: bool,
    pub status: String,
    pub overlay_revision: u64,
    pub overlay_relative_path: String,
    pub receipt_relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_entry: Option<UserOverlayEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_event_id: Option<String>,
}

/// Plan or apply a machine-private skill lifecycle mutation. The tracked suite
/// registry always wins: any identifier declared there (including retired
/// entries) is rejected before the overlay is considered.
#[allow(clippy::too_many_arguments)]
pub fn mutate_user_overlay(
    manifest_root: &Path,
    runtime_home: &Path,
    host_home: &Path,
    active_host: &str,
    skill_id: &str,
    operation: OverlayMutationOperation,
    restored_from_revision: Option<u64>,
    apply: bool,
) -> Result<OverlayMutationResult, String> {
    if !safe_skill_id(skill_id) {
        return Err("invalid_skill_id".to_string());
    }
    let registry = load_registry_document(manifest_root)
        .map_err(|error| format!("cannot load official registry: {error:?}"))?;
    if registry.skills.iter().any(|skill| skill.name == skill_id) {
        return Err("official_registry_precedence".to_string());
    }

    let mut overlay = load_user_overlay(runtime_home)?;
    let before_entry = overlay
        .entries
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned();
    let next_revision = overlay.revision.saturating_add(1);
    let candidate = || {
        external_candidate_card(
            manifest_root,
            runtime_home,
            host_home,
            active_host,
            skill_id,
        )
    };

    let mut after_entry = match operation {
        OverlayMutationOperation::Adopt => {
            let card = candidate()?;
            validate_overlay_candidate(&card)?;
            Some(overlay_entry_from_card(
                &card,
                OverlayEntryState::Active,
                next_revision,
            ))
        }
        OverlayMutationOperation::Ignore => {
            if let Some(mut existing) = before_entry.clone() {
                existing.state = OverlayEntryState::Ignored;
                existing.revision = next_revision;
                Some(existing)
            } else {
                let card = candidate()?;
                validate_overlay_candidate(&card)?;
                Some(overlay_entry_from_card(
                    &card,
                    OverlayEntryState::Ignored,
                    next_revision,
                ))
            }
        }
        OverlayMutationOperation::Rollback => {
            let revision =
                restored_from_revision.ok_or_else(|| "rollback_revision_required".to_string())?;
            if revision == 0 {
                None
            } else {
                let events = load_overlay_mutation_receipts(runtime_home)?;
                let mut restored = events
                    .iter()
                    .rev()
                    .flat_map(|event| [event.after_entry.as_ref(), event.before_entry.as_ref()])
                    .flatten()
                    .find(|entry| entry.skill_id == skill_id && entry.revision == revision)
                    .cloned()
                    .ok_or_else(|| "overlay_revision_not_found".to_string())?;
                restored.revision = next_revision;
                Some(restored)
            }
        }
    };

    let changed = !overlay_entries_semantically_equal(before_entry.as_ref(), after_entry.as_ref());
    if !changed {
        return Ok(OverlayMutationResult {
            schema_version: "0.3.0-overlay-mutation-result".to_string(),
            operation,
            skill_id: skill_id.to_string(),
            dry_run: !apply,
            applied: false,
            changed: false,
            status: "noop".to_string(),
            overlay_revision: overlay.revision,
            overlay_relative_path: "skill-registry/user-overlay.yaml".to_string(),
            receipt_relative_path: "skill-registry/user-overlay-events.ndjson".to_string(),
            restored_from_revision,
            proposed_entry: before_entry,
            receipt_event_id: None,
        });
    }

    overlay.entries.retain(|entry| entry.skill_id != skill_id);
    if let Some(entry) = after_entry.take() {
        overlay.entries.push(entry);
    }
    overlay
        .entries
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    overlay.revision = next_revision;
    let after_entry = overlay
        .entries
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned();
    let timestamp_unix = unix_timestamp();
    let event_id = sha256(
        format!(
            "overlay\n{operation:?}\n{skill_id}\n{}\n{next_revision}\n{timestamp_unix}",
            overlay.revision.saturating_sub(1)
        )
        .as_bytes(),
    );
    let receipt = OverlayMutationReceipt {
        schema_version: OVERLAY_MUTATION_EVENT_SCHEMA_VERSION.to_string(),
        event_id: event_id.clone(),
        timestamp_unix,
        operation,
        skill_id: skill_id.to_string(),
        from_overlay_revision: overlay.revision.saturating_sub(1),
        to_overlay_revision: overlay.revision,
        restored_from_revision,
        before_entry: before_entry.clone(),
        after_entry: after_entry.clone(),
    };

    if apply {
        let path = overlay_path(runtime_home);
        let previous = read_existing_private_file(&path)?;
        let capability_path = snapshot_path(runtime_home, active_host);
        let previous_snapshot = read_existing_private_file(&capability_path)?;
        let receipt_path = overlay_events_path(runtime_home);
        let previous_receipts = read_existing_private_file(&receipt_path)?;
        let receipt_bytes = render_overlay_receipt_append(previous_receipts.as_deref(), &receipt)?;
        let serialized = serde_yaml::to_string(&overlay)
            .map_err(|error| format!("cannot serialize user overlay: {error}"))?;
        write_private_atomic(&path, serialized.as_bytes())?;
        let commit = (|| {
            let snapshot = build_capability_snapshot_with_roots(
                manifest_root,
                active_host,
                runtime_home,
                host_home,
            )
            .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
            let snapshot_json = serde_json::to_string_pretty(&snapshot)
                .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
            write_private_atomic(&capability_path, (snapshot_json + "\n").as_bytes())?;
            write_private_atomic(&receipt_path, &receipt_bytes)
        })();
        if let Err(error) = commit {
            let overlay_rollback = restore_private_file(&path, previous);
            let snapshot_rollback = restore_private_file(&capability_path, previous_snapshot);
            return Err(match (overlay_rollback, snapshot_rollback) {
                (Ok(()), Ok(())) => {
                    format!("overlay transaction failed and was rolled back: {error}")
                }
                (overlay_result, snapshot_result) => format!(
                    "overlay transaction failed: {error}; rollback failed: overlay={overlay_result:?}, snapshot={snapshot_result:?}"
                ),
            });
        }
    }

    Ok(OverlayMutationResult {
        schema_version: "0.3.0-overlay-mutation-result".to_string(),
        operation,
        skill_id: skill_id.to_string(),
        dry_run: !apply,
        applied: apply,
        changed: true,
        status: if apply { "applied" } else { "planned" }.to_string(),
        overlay_revision: overlay.revision,
        overlay_relative_path: "skill-registry/user-overlay.yaml".to_string(),
        receipt_relative_path: "skill-registry/user-overlay-events.ndjson".to_string(),
        restored_from_revision,
        proposed_entry: after_entry,
        receipt_event_id: apply.then_some(event_id),
    })
}

pub(super) fn restore_private_file(path: &Path, previous: Option<Vec<u8>>) -> Result<(), String> {
    if let Some(bytes) = previous {
        return write_private_atomic(path, &bytes);
    }
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| {
            format!(
                "cannot remove rollback artifact {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn read_existing_private_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot read existing private file {} before mutation: {error}",
            path.display()
        )),
    }
}

pub(super) fn external_candidate_card(
    manifest_root: &Path,
    runtime_home: &Path,
    host_home: &Path,
    active_host: &str,
    skill_id: &str,
) -> Result<SkillCard, String> {
    if let Some(source) = load_user_source_registry(runtime_home)?
        .entries
        .into_iter()
        .find(|entry| entry.skill_id == skill_id)
    {
        let canonical = Path::new(&source.canonical_path);
        if !canonical.join("SKILL.md").is_file() {
            return Err("canonical_missing".to_string());
        }
        return Ok(SkillCard {
            skill_id: source.skill_id,
            display_name: source.display_name,
            summary: source.summary,
            intent_tags: source.intent_tags,
            positive_examples: Vec::new(),
            negative_examples: Vec::new(),
            entrypoints: source.entrypoints,
            source_kind: SkillSourceKind::External,
            governance: GovernanceState::Candidate,
            availability: AvailabilityState::Unavailable {
                reason_codes: vec!["candidate_requires_adoption".to_string()],
            },
            reason_codes: vec!["candidate_requires_adoption".to_string()],
            requires_auth: source.requires_auth,
            auth_state: if source.requires_auth {
                AuthState::Unknown
            } else {
                AuthState::NotRequired
            },
            activity: ActivityState::Unobserved,
            version: source.audit_version,
            source_hash: source.source_hash,
        });
    }
    let context = ConsoleContext::new(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        Box::new(NoProcessDiscovery),
    );
    let inventory = build_inventory(&context, &[active_host]);
    let capability = inventory
        .capabilities
        .iter()
        .find(|capability| capability.kind == ManagedKind::Skill && capability.name == skill_id)
        .ok_or_else(|| "skill_candidate_not_found".to_string())?;
    Ok(skill_card(
        manifest_root,
        capability,
        None,
        None,
        &[],
        AuthState::NotRequired,
    ))
}

pub(super) fn validate_overlay_candidate(card: &SkillCard) -> Result<(), String> {
    if !matches!(
        card.source_kind,
        SkillSourceKind::UserInstalled
            | SkillSourceKind::ProjectLocal
            | SkillSourceKind::EnabledPlugin
            | SkillSourceKind::External
    ) {
        return Err("overlay_source_not_adoptable".to_string());
    }
    if card
        .reason_codes
        .iter()
        .any(|reason| reason == "metadata_incomplete")
    {
        return Err("metadata_incomplete".to_string());
    }
    Ok(())
}

pub(super) fn overlay_entry_from_card(
    card: &SkillCard,
    state: OverlayEntryState,
    revision: u64,
) -> UserOverlayEntry {
    UserOverlayEntry {
        skill_id: card.skill_id.clone(),
        state,
        revision,
        source_hash: card.source_hash.clone(),
        metadata_version: if card.version == "registry" {
            "skillcard-v1".to_string()
        } else {
            card.version.clone()
        },
        display_name: card.display_name.clone(),
        summary: card.summary.clone(),
        intent_tags: card.intent_tags.clone(),
        entrypoints: card.entrypoints.clone(),
        invoke_hint: format!("[skill: {}]", card.skill_id),
        requires_auth: card.requires_auth,
    }
}

pub(super) fn overlay_entries_semantically_equal(
    left: Option<&UserOverlayEntry>,
    right: Option<&UserOverlayEntry>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            let mut left = left.clone();
            let mut right = right.clone();
            left.revision = 0;
            right.revision = 0;
            left == right
        }
        _ => false,
    }
}

pub(super) fn safe_skill_id(skill_id: &str) -> bool {
    !skill_id.is_empty()
        && skill_id.len() <= 128
        && skill_id
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

pub(super) fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
}

pub(super) fn is_known_source_license(value: &str) -> bool {
    matches!(
        value,
        "MIT"
            | "Apache-2.0"
            | "MPL-2.0"
            | "BSD-2-Clause"
            | "BSD-3-Clause"
            | "GPL-3.0-only"
            | "GPL-3.0-or-later"
            | "LGPL-3.0-only"
            | "LGPL-3.0-or-later"
            | "AGPL-3.0-only"
            | "AGPL-3.0-or-later"
    )
}

pub(super) fn valid_github_source_provenance(entry: &UserSourceEntry) -> bool {
    let Some(resolved_ref) = entry.resolved_ref.as_deref() else {
        return false;
    };
    if resolved_ref.len() != 40
        || !resolved_ref
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return false;
    }
    let Some(rest) = entry.source.strip_prefix("https://github.com/") else {
        return false;
    };
    if rest.contains(['?', '#']) {
        return false;
    }
    let parts = rest.split('/').collect::<Vec<_>>();
    if parts.len() < 5 || parts[2] != "tree" {
        return false;
    }
    if [parts[0], parts[1], parts[3]].iter().any(|part| {
        part.is_empty()
            || part.len() > 128
            || !part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    }) {
        return false;
    }
    let subdir = parts[4..].join("/");
    let subdir_path = Path::new(&subdir);
    !subdir.is_empty()
        && !subdir_path.is_absolute()
        && subdir_path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        && entry.subdir.as_deref() == Some(subdir.as_str())
}

pub(super) fn load_overlay_mutation_receipts(
    runtime_home: &Path,
) -> Result<Vec<OverlayMutationReceipt>, String> {
    let path = overlay_events_path(runtime_home);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let receipt: OverlayMutationReceipt = serde_json::from_str(line).map_err(|error| {
                format!("invalid overlay receipt at line {}: {error}", index + 1)
            })?;
            if receipt.schema_version != OVERLAY_MUTATION_EVENT_SCHEMA_VERSION {
                return Err(format!(
                    "unsupported overlay receipt schema at line {}",
                    index + 1
                ));
            }
            Ok(receipt)
        })
        .collect()
}

pub(super) fn overlay_active_since(
    receipts: &[OverlayMutationReceipt],
    skill_id: &str,
) -> Option<u64> {
    receipts
        .iter()
        .rev()
        .find(|receipt| {
            receipt.skill_id == skill_id
                && receipt
                    .after_entry
                    .as_ref()
                    .is_some_and(|entry| entry.state == OverlayEntryState::Active)
        })
        .map(|receipt| receipt.timestamp_unix)
}

pub(super) fn render_overlay_receipt_append(
    previous: Option<&[u8]>,
    receipt: &OverlayMutationReceipt,
) -> Result<Vec<u8>, String> {
    let line = serde_json::to_string(receipt).map_err(|error| error.to_string())?;
    if let Some(previous) = previous {
        std::str::from_utf8(previous)
            .map_err(|error| format!("overlay receipt ledger is not UTF-8: {error}"))?;
    }
    let mut bytes = previous.unwrap_or_default().to_vec();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    Ok(bytes)
}
