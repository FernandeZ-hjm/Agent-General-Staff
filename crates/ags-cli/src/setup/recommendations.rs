//! `ags setup` — third-party core capability recommendation block.
//!
//! READ-ONLY display sourced from `manifests/skill-recommendations.yaml`. AGS
//! never clones, installs, downloads, or writes host config / skill thin-index
//! for these — the block reports local-install + host-visibility status (via
//! filesystem stat only) and a manual next step. Mirrors the other setup
//! section renderers (`capability_route_enrollment_*`).

use skill_governance::recommendations::{read_recommendations, recommendation_status};
use std::path::Path;

pub(in crate::setup) fn third_party_recommendations_json(
    source_root: &Path,
    home: &Path,
) -> serde_json::Value {
    let doc = read_recommendations(source_root);
    let items: Vec<serde_json::Value> = doc
        .skills
        .iter()
        .map(|rec| {
            let st = recommendation_status(rec, home);
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
                "local_install": st.local_install,
                "host_visibility": st.host_visibility,
                "next_step": st.next_step,
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": doc.schema_version,
        "principle": doc.principle,
        "boundary": "Recommendation-only. AGS never clones, installs, downloads, or writes host config / skill thin-index for these. Status is read-only (filesystem stat).",
        "write_mode": "read-only (no install, no host write)",
        "items": items,
    })
}

pub(in crate::setup) fn render_third_party_recommendations_text(
    source_root: &Path,
    home: &Path,
) -> String {
    let doc = read_recommendations(source_root);
    let mut lines = vec![
        "Third-Party Core Capability Recommendations (recommendation-only · AGS never installs)"
            .to_string(),
    ];
    if doc.skills.is_empty() {
        lines.push(
            "  (manifests/skill-recommendations.yaml not found — no recommendations to show)"
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
            let st = recommendation_status(rec, home);
            let hosts: Vec<String> = st
                .host_visibility
                .iter()
                .map(|h| format!("{}={}", h.host, h.status))
                .collect();
            let src = rec
                .source
                .clone()
                .unwrap_or_else(|| "(community-maintained)".to_string());
            lines.push(format!(
                "    - {:<28} install:{:<14} hosts:{:<26} src:{}",
                rec.id,
                st.local_install,
                hosts.join(","),
                src,
            ));
        }
    }
    lines.push(
        "  Boundary: recommendation-only; AGS never clones, installs, or writes host config / thin-index."
            .to_string(),
    );
    lines.push(
        "  Next: review each source and install manually, then `ags skill verify --host <host>`."
            .to_string(),
    );
    lines.join("\n")
}
