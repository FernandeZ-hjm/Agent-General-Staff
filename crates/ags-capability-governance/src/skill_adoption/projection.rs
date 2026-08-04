use super::model::AdoptionStatus;
use super::store::{body_path, load_registry, registry_hash};
use crate::{
    hash_skill_source, load_static_snapshot, ActiveSkill, AuthState, AvailabilityState,
    GovernanceState, SkillCard, SkillRoutingSurface, SkillSourceKind,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct PrivateProjection {
    pub cards: Vec<SkillCard>,
    pub active: Vec<ActiveSkill>,
    pub registry_hash: String,
}

pub(crate) fn project_private_skills(
    runtime_home: &Path,
    host_home: &Path,
    host: &str,
    official_ids: &HashSet<String>,
) -> Result<PrivateProjection, String> {
    let registry = load_registry(runtime_home)?;
    let mut cards = Vec::new();
    let mut active = Vec::new();
    for record in registry.skills.values() {
        if official_ids.contains(&record.skill_id)
            || !record.target_hosts.iter().any(|item| item == host)
        {
            continue;
        }
        let body = body_path(runtime_home, record);
        let body_present = body.join("SKILL.md").is_file();
        let body_hash_matches = body_present
            && hash_skill_source(&body).is_ok_and(|actual| actual == record.source_hash);
        let visible = host_index_path(host_home, host, &record.skill_id)
            .is_some_and(|index| index_points_to(&index, &body));
        let mut reasons = Vec::new();
        if !body_present {
            reasons.push("canonical_missing".to_string());
        } else if !body_hash_matches {
            reasons.push("source_hash_changed".to_string());
        }
        if !visible {
            reasons.push("host_not_visible".to_string());
        }
        if record.requires_auth {
            reasons.push("auth_required".to_string());
        }
        let availability = if reasons.is_empty() {
            AvailabilityState::Ready
        } else {
            AvailabilityState::Unavailable {
                reason_codes: reasons.clone(),
            }
        };
        let card = SkillCard {
            skill_id: record.skill_id.clone(),
            display_name: record.skill_id.clone(),
            summary: record.summary.clone(),
            intent_tags: record.intent_tags.clone(),
            positive_examples: record.positive_examples.clone(),
            negative_examples: record.negative_examples.clone(),
            entrypoints: record.entrypoints.clone(),
            routing_surface: SkillRoutingSurface::SkillTarget,
            routing_hint: Some(record.invoke_hint.clone()),
            source_kind: SkillSourceKind::External,
            governance: GovernanceState::Active,
            availability: availability.clone(),
            reason_codes: reasons,
            requires_auth: record.requires_auth,
            auth_state: if record.requires_auth {
                AuthState::Unknown
            } else {
                AuthState::NotRequired
            },
            version: record.version.clone(),
            source_hash: record.source_hash.clone(),
        };
        if availability.is_ready() {
            active.push(ActiveSkill {
                skill_id: record.skill_id.clone(),
                invoke_hint: record.invoke_hint.clone(),
                allowed_entrypoints: record.entrypoints.clone(),
                intent_tags: record.intent_tags.clone(),
                source_hash: record.source_hash.clone(),
            });
        }
        cards.push(card);
    }
    Ok(PrivateProjection {
        cards,
        active,
        registry_hash: registry_hash(runtime_home)?,
    })
}

pub fn inspect_adoption(
    runtime_home: &Path,
    host_home: &Path,
    skill_id: &str,
) -> Result<AdoptionStatus, String> {
    let registry = load_registry(runtime_home)?;
    let Some(record) = registry.skills.get(skill_id) else {
        return Ok(AdoptionStatus {
            skill_id: skill_id.to_string(),
            registered: false,
            body_present: false,
            body_hash_matches: false,
            target_hosts: Vec::new(),
            visible_hosts: Vec::new(),
            active_hosts: Vec::new(),
            source: None,
            source_hash: None,
        });
    };
    let body = body_path(runtime_home, record);
    let body_present = body.join("SKILL.md").is_file();
    let body_hash_matches =
        body_present && hash_skill_source(&body).is_ok_and(|actual| actual == record.source_hash);
    let visible_hosts = record
        .target_hosts
        .iter()
        .filter(|host| {
            host_index_path(host_home, host, skill_id)
                .is_some_and(|index| index_points_to(&index, &body))
        })
        .cloned()
        .collect::<Vec<_>>();
    let active_hosts = record
        .target_hosts
        .iter()
        .filter(|host| {
            load_static_snapshot(runtime_home, host).is_ok_and(|(snapshot, _)| {
                snapshot
                    .active_skills
                    .iter()
                    .any(|skill| skill.skill_id == skill_id)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(AdoptionStatus {
        skill_id: skill_id.to_string(),
        registered: true,
        body_present,
        body_hash_matches,
        target_hosts: record.target_hosts.clone(),
        visible_hosts,
        active_hosts,
        source: Some(record.source.clone()),
        source_hash: Some(record.source_hash.clone()),
    })
}

pub(super) fn host_index_path(home: &Path, host: &str, skill_id: &str) -> Option<PathBuf> {
    ags_host_integration::static_skill_roots(home, host)
        .into_iter()
        .next()
        .map(|root| root.join(skill_id))
}

pub(super) fn index_points_to(index: &Path, body: &Path) -> bool {
    fs::symlink_metadata(index).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::canonicalize(index).ok() == fs::canonicalize(body).ok()
}
