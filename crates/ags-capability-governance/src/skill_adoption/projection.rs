use super::model::{ActivatedCapability, AdoptionRouteStatus, AdoptionStatus};
use super::store::{body_path, installed_skill_index_hash, load_installed_skills};
use crate::{
    hash_skill_source, load_static_snapshot, ActiveSkill, AuthState, AvailabilityState,
    GovernanceState, SkillBodyRef, SkillCard, SkillRoutingSurface, SkillSourceKind,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct PrivateProjection {
    pub cards: Vec<SkillCard>,
    pub active: Vec<ActiveSkill>,
    pub installed_skill_index_hash: String,
}

pub(crate) fn project_installed_skills_with_overlay(
    runtime_home: &Path,
    host_home: &Path,
    host: &str,
    official_ids: &HashSet<String>,
    overlay: Option<&super::model::SkillPostStateOverlay>,
) -> Result<PrivateProjection, String> {
    let registry = overlay
        .map(|overlay| Ok(overlay.installed_skills.clone()))
        .unwrap_or_else(|| load_installed_skills(runtime_home))?;
    let mut cards = Vec::new();
    let mut active = Vec::new();
    for record in registry.skills.values() {
        if official_ids.contains(&record.skill_id)
            || !record.target_hosts.iter().any(|item| item == host)
        {
            continue;
        }
        let body = body_path(runtime_home, record);
        let target_overlay = overlay.filter(|overlay| overlay.target_skill_id == record.skill_id);
        let body_present = target_overlay
            .map(|overlay| overlay.target_body_hash_matches)
            .unwrap_or_else(|| body.join("SKILL.md").is_file());
        let body_hash_matches = target_overlay
            .map(|overlay| overlay.target_body_hash_matches)
            .unwrap_or_else(|| {
                body_present
                    && hash_skill_source(&body).is_ok_and(|actual| actual == record.source_hash)
            });
        let visible = target_overlay
            .map(|overlay| overlay.target_visible_hosts.contains(host))
            .unwrap_or_else(|| {
                host_index_path(host_home, host, &record.skill_id)
                    .is_some_and(|index| index_points_to(&index, &body))
            });
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
                body_ref: SkillBodyRef::new(
                    &record.skill_id,
                    record.body_revision.clone(),
                    record.source_hash.clone(),
                ),
            });
        }
        cards.push(card);
    }
    Ok(PrivateProjection {
        cards,
        active,
        installed_skill_index_hash: if overlay.is_some() {
            ags_platform::sha256(
                serde_json::to_vec(&registry)
                    .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?,
            )
        } else {
            installed_skill_index_hash(runtime_home)?
        },
    })
}

pub fn inspect_adoption(
    runtime_home: &Path,
    host_home: &Path,
    skill_id: &str,
) -> Result<AdoptionStatus, String> {
    let registry = load_installed_skills(runtime_home)?;
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

/// Verify installed state, Host visibility and an exact runtime route for each
/// target Host. This is the only adoption API allowed to claim route_verified.
pub fn verify_adoption_routes(
    runtime_home: &Path,
    host_home: &Path,
    skill_id: &str,
) -> Result<AdoptionRouteStatus, String> {
    let installation = inspect_adoption(runtime_home, host_home, skill_id)?;
    let activations = installation
        .target_hosts
        .iter()
        .map(|host| {
            let visible = installation.visible_hosts.contains(host);
            match load_static_snapshot(runtime_home, host) {
                Ok((snapshot, tables)) => {
                    let resolution = crate::resolve_skill(
                        skill_id,
                        None,
                        &snapshot.snapshot_hash,
                        &tables.skills,
                    );
                    match resolution {
                        Ok(selection) => ActivatedCapability {
                            skill_id: skill_id.to_string(),
                            host: host.clone(),
                            visible,
                            snapshot_loaded: true,
                            route_verified: visible
                                && selection.skill_id == skill_id
                                && selection.snapshot_hash == snapshot.snapshot_hash,
                            snapshot_hash: Some(snapshot.snapshot_hash),
                            evidence: format!(
                                "exact resolve_skill selected `{}`",
                                selection.skill_id
                            ),
                        },
                        Err(error) => ActivatedCapability {
                            skill_id: skill_id.to_string(),
                            host: host.clone(),
                            visible,
                            snapshot_loaded: true,
                            route_verified: false,
                            snapshot_hash: Some(snapshot.snapshot_hash),
                            evidence: format!("exact resolve_skill failed: {error:?}"),
                        },
                    }
                }
                Err(error) => ActivatedCapability {
                    skill_id: skill_id.to_string(),
                    host: host.clone(),
                    visible,
                    snapshot_loaded: false,
                    route_verified: false,
                    snapshot_hash: None,
                    evidence: format!("sealed snapshot load failed: {error:?}"),
                },
            }
        })
        .collect();
    Ok(AdoptionRouteStatus {
        installation,
        activations,
    })
}

/// Batch form used by status/catalog projections. Registry and each Host
/// snapshot are loaded once, eliminating the old N-by-host repeated scan.
pub fn verify_adoption_routes_batch(
    runtime_home: &Path,
    host_home: &Path,
    skill_ids: &[String],
) -> Result<BTreeMap<String, AdoptionRouteStatus>, String> {
    let registry = load_installed_skills(runtime_home)?;
    let hosts = skill_ids
        .iter()
        .filter_map(|skill_id| registry.skills.get(skill_id))
        .flat_map(|record| record.target_hosts.iter().cloned())
        .collect::<BTreeSet<_>>();
    let snapshots = hosts
        .into_iter()
        .map(|host| {
            let loaded = load_static_snapshot(runtime_home, &host)
                .map_err(|error| format!("sealed snapshot load failed: {error:?}"));
            (host, loaded)
        })
        .collect::<BTreeMap<_, _>>();
    let mut statuses = BTreeMap::new();
    for skill_id in skill_ids {
        let installation = match registry.skills.get(skill_id) {
            None => AdoptionStatus {
                skill_id: skill_id.clone(),
                registered: false,
                body_present: false,
                body_hash_matches: false,
                target_hosts: Vec::new(),
                visible_hosts: Vec::new(),
                active_hosts: Vec::new(),
                source: None,
                source_hash: None,
            },
            Some(record) => {
                let body = body_path(runtime_home, record);
                let body_present = body.join("SKILL.md").is_file();
                let body_hash_matches = body_present
                    && hash_skill_source(&body).is_ok_and(|actual| actual == record.source_hash);
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
                        snapshots.get(*host).is_some_and(|loaded| {
                            loaded.as_ref().is_ok_and(|(snapshot, _)| {
                                snapshot
                                    .active_skills
                                    .iter()
                                    .any(|skill| skill.skill_id == *skill_id)
                            })
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                AdoptionStatus {
                    skill_id: skill_id.clone(),
                    registered: true,
                    body_present,
                    body_hash_matches,
                    target_hosts: record.target_hosts.clone(),
                    visible_hosts,
                    active_hosts,
                    source: Some(record.source.clone()),
                    source_hash: Some(record.source_hash.clone()),
                }
            }
        };
        let activations = installation
            .target_hosts
            .iter()
            .map(|host| {
                let visible = installation.visible_hosts.contains(host);
                match snapshots.get(host) {
                    Some(Ok((snapshot, tables))) => match crate::resolve_skill(
                        skill_id,
                        None,
                        &snapshot.snapshot_hash,
                        &tables.skills,
                    ) {
                        Ok(selection) => ActivatedCapability {
                            skill_id: skill_id.clone(),
                            host: host.clone(),
                            visible,
                            snapshot_loaded: true,
                            route_verified: visible
                                && selection.skill_id == *skill_id
                                && selection.snapshot_hash == snapshot.snapshot_hash,
                            snapshot_hash: Some(snapshot.snapshot_hash.clone()),
                            evidence: format!(
                                "exact resolve_skill selected `{}`",
                                selection.skill_id
                            ),
                        },
                        Err(error) => ActivatedCapability {
                            skill_id: skill_id.clone(),
                            host: host.clone(),
                            visible,
                            snapshot_loaded: true,
                            route_verified: false,
                            snapshot_hash: Some(snapshot.snapshot_hash.clone()),
                            evidence: format!("exact resolve_skill failed: {error:?}"),
                        },
                    },
                    Some(Err(error)) => ActivatedCapability {
                        skill_id: skill_id.clone(),
                        host: host.clone(),
                        visible,
                        snapshot_loaded: false,
                        route_verified: false,
                        snapshot_hash: None,
                        evidence: error.clone(),
                    },
                    None => ActivatedCapability {
                        skill_id: skill_id.clone(),
                        host: host.clone(),
                        visible,
                        snapshot_loaded: false,
                        route_verified: false,
                        snapshot_hash: None,
                        evidence: "target Host snapshot was not planned".to_string(),
                    },
                }
            })
            .collect();
        statuses.insert(
            skill_id.clone(),
            AdoptionRouteStatus {
                installation,
                activations,
            },
        );
    }
    Ok(statuses)
}

pub(super) fn host_index_path(home: &Path, host: &str, skill_id: &str) -> Option<PathBuf> {
    ags_host_integration::managed_skill_root(home, host).map(|root| root.join(skill_id))
}

pub(super) fn host_index_paths(home: &Path, host: &str, skill_id: &str) -> Vec<PathBuf> {
    let mut paths = host_index_path(home, host, skill_id)
        .into_iter()
        .collect::<Vec<_>>();
    paths.extend(
        ags_host_integration::static_skill_roots(home, host)
            .into_iter()
            .map(|root| root.join(skill_id)),
    );
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn index_points_to(index: &Path, body: &Path) -> bool {
    fs::symlink_metadata(index).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::canonicalize(index).ok() == fs::canonicalize(body).ok()
}
