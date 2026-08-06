//! `ags setup` — third-party core capability recommendation block.
//!
//! READ-ONLY display sourced from the unified third-party capability manifest.
//! Installation is never silent: explicit install/update owns any source
//! change and refreshes the static host snapshot once after verification.

use ags_capability_governance::skill_body::recommendations::{
    read_recommendations, skill_status_projections,
};
use std::collections::BTreeMap;
use std::path::Path;

pub(in crate::setup) fn third_party_recommendations_json(
    source_root: &Path,
    runtime_home: &Path,
    home: &Path,
) -> serde_json::Value {
    let doc = read_recommendations(source_root);
    let ids = doc
        .skills
        .iter()
        .map(|rec| rec.id.clone())
        .collect::<Vec<_>>();
    let statuses = skill_status_projections(source_root, runtime_home, home, &ids);
    let status_by_id = statuses
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|status| (status.skill_id.as_str(), status))
                .collect::<BTreeMap<_, _>>()
        })
        .ok();
    let items: Vec<serde_json::Value> = doc
        .skills
        .iter()
        .map(|rec| {
            let status = status_by_id
                .as_ref()
                .and_then(|items| items.get(rec.id.as_str()))
                .copied();
            serde_json::json!({
                "id": rec.id,
                "name": rec.name,
                "tier": rec.tier,
                "recommendation_only": rec.recommendation_only,
                "source_kind": rec.source_kind,
                "source": rec.source,
                "upstream": rec.upstream,
                "risk": rec.risk,
                "install_location": rec.install_location,
                "status": status,
                "status_error": statuses.as_ref().err(),
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": doc.schema_version,
        "principle": doc.principle,
        "boundary": "No silent installation. Setup is read-only; explicit install/update owns source changes and one static snapshot refresh.",
        "write_mode": "setup read-only; install/update is confirmation-protected",
        "items": items,
    })
}

pub(in crate::setup) fn render_third_party_recommendations_text(
    source_root: &Path,
    runtime_home: &Path,
    home: &Path,
) -> String {
    let doc = read_recommendations(source_root);
    let ids = doc
        .skills
        .iter()
        .map(|rec| rec.id.clone())
        .collect::<Vec<_>>();
    let statuses = skill_status_projections(source_root, runtime_home, home, &ids);
    let status_by_id = statuses
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|status| (status.skill_id.as_str(), status))
                .collect::<BTreeMap<_, _>>()
        })
        .ok();
    let mut lines =
        vec!["Third-Party Core Capability Recommendations (no silent install)".to_string()];
    if doc.skills.is_empty() {
        lines.push(
            "  (third-party capability manifest unavailable — no recommendations to show)"
                .to_string(),
        );
        return lines.join("\n");
    }
    // Group by tier, preserving first-seen order.
    let mut tiers: Vec<String> = Vec::new();
    for rec in &doc.skills {
        if !tiers.contains(&rec.tier) {
            tiers.push(rec.tier.clone());
        }
    }
    for tier in &tiers {
        let label = if tier.is_empty() { "other" } else { tier };
        lines.push(format!("  [{label}]"));
        for rec in doc.skills.iter().filter(|r| &r.tier == tier) {
            let status = status_by_id
                .as_ref()
                .and_then(|items| items.get(rec.id.as_str()))
                .copied();
            let src = rec
                .source
                .clone()
                .unwrap_or_else(|| "(community-maintained)".to_string());
            lines.push(format!(
                "    - {:<28} install:{:?} activate:{:?} update:{:?} src:{}",
                rec.id,
                status
                    .map(|status| format!("{:?}", status.installation.state))
                    .unwrap_or_else(|| "Error".to_string()),
                status
                    .map(|status| format!("{:?}", status.activation.state))
                    .unwrap_or_else(|| "Error".to_string()),
                status
                    .map(|status| format!("{:?}", status.update.state))
                    .unwrap_or_else(|| "Error".to_string()),
                src,
            ));
        }
    }
    lines.push(
        "  Boundary: setup never installs; explicit install/update owns source changes and refreshes the static snapshot once."
            .to_string(),
    );
    lines.push(
        "  Next: run `ags onboarding plan`, approve one item, then verify the host.".to_string(),
    );
    lines.join("\n")
}
