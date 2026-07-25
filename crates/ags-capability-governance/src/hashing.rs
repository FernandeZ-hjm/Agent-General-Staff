#[allow(unused_imports)]
use super::catalog::*;
use super::*;
pub(super) fn sort_active_skills(skills: &mut [ActiveSkill]) {
    skills.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

pub(super) fn sort_skill_cards(cards: &mut [SkillCard]) {
    cards.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

pub(super) fn sort_third_party_cards(cards: &mut [ThirdPartyCapabilityCard]) {
    cards.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.capability_id.cmp(&right.capability_id))
    });
}

pub(super) fn active_table_hash(active_skills: &[ActiveSkill]) -> String {
    let mut canonical = active_skills.to_vec();
    sort_active_skills(&mut canonical);
    sha256(&serde_json::to_vec(&canonical).unwrap_or_default())
}

pub(super) fn catalog_hash(
    catalog: &[SkillCard],
    third_party_catalog: &[ThirdPartyCapabilityCard],
) -> String {
    let mut canonical = catalog.to_vec();
    for card in &mut canonical {
        card.activity = ActivityState::Unobserved;
    }
    sort_skill_cards(&mut canonical);
    let mut third_party = third_party_catalog.to_vec();
    sort_third_party_cards(&mut third_party);
    sha256(&serde_json::to_vec(&(canonical, third_party)).unwrap_or_default())
}

pub(super) fn snapshot_integrity_hash(snapshot: &HostCapabilitySnapshot) -> String {
    sha256(
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            snapshot.schema_version,
            snapshot.host,
            snapshot.registry_hash,
            snapshot.overlay_hash,
            snapshot.runtime_hash,
            snapshot.third_party_registry_url,
            snapshot.third_party_manifest_hash,
            snapshot.catalog_hash,
            snapshot.active_table_hash
        )
        .as_bytes(),
    )
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub(super) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
