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

pub(super) fn snapshot_integrity_hash(snapshot: &HostCapabilitySnapshot) -> String {
    let mut catalog = snapshot.catalog.clone();
    sort_skill_cards(&mut catalog);
    let mut third_party = snapshot.third_party_catalog.clone();
    sort_third_party_cards(&mut third_party);
    let mut active_skills = snapshot.active_skills.clone();
    sort_active_skills(&mut active_skills);
    sha256(
        &serde_json::to_vec(&(
            &snapshot.schema_version,
            &snapshot.host,
            &snapshot.registry_hash,
            &snapshot.runtime_hash,
            &snapshot.third_party_registry_url,
            &snapshot.third_party_manifest_hash,
            catalog,
            third_party,
            active_skills,
        ))
        .unwrap_or_default(),
    )
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}
