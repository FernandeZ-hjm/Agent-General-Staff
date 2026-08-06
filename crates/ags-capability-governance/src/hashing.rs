#[allow(unused_imports)]
use super::catalog::*;
pub(super) fn sort_active_skills(skills: &mut [ActiveSkill]) {
    skills.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

pub(super) fn sort_skill_cards(cards: &mut [SkillCard]) {
    cards.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
}

pub(super) fn sort_mcp_cards(cards: &mut [McpCard]) {
    cards.sort_by(|left, right| left.mcp_id.cmp(&right.mcp_id));
}

pub(super) fn sort_active_mcps(mcps: &mut [ActiveMcp]) {
    mcps.sort_by(|left, right| left.mcp_id.cmp(&right.mcp_id));
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
    let mut mcp_catalog = snapshot.mcp_catalog.clone();
    sort_mcp_cards(&mut mcp_catalog);
    let mut third_party = snapshot.third_party_catalog.clone();
    sort_third_party_cards(&mut third_party);
    let mut active_skills = snapshot.active_skills.clone();
    sort_active_skills(&mut active_skills);
    let mut active_mcps = snapshot.active_mcps.clone();
    sort_active_mcps(&mut active_mcps);
    ags_platform::sha256(
        serde_json::to_vec(&(
            &snapshot.schema_version,
            &snapshot.host,
            &snapshot.registry_hash,
            &snapshot.runtime_hash,
            &snapshot.third_party_registry_url,
            &snapshot.third_party_manifest_hash,
            catalog,
            mcp_catalog,
            third_party,
            active_skills,
            active_mcps,
        ))
        .unwrap_or_default(),
    )
}
