//! `ags setup` — third-party core capability recommendation block.
//!
//! READ-ONLY display sourced from the unified third-party capability manifest.
//! Installation is never silent: confirmed per-item changes are delegated to
//! the audited onboarding/skill-adoption lifecycle.

use ags_capability_governance::skill_body::recommendations::{
    read_recommendations, recommendation_status,
};
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
                "capability_state": st.capability_state,
                "local_install": st.local_install,
                "host_visibility": st.host_visibility,
                "next_step": st.next_step,
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": doc.schema_version,
        "principle": doc.principle,
        "boundary": "No silent installation. Setup is read-only; explicit per-item onboarding apply delegates to audited skill adoption.",
        "write_mode": "setup read-only; onboarding apply is confirmation-protected",
        "items": items,
    })
}

pub(in crate::setup) fn render_third_party_recommendations_text(
    source_root: &Path,
    home: &Path,
) -> String {
    let doc = read_recommendations(source_root);
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
                "    - {:<28} state:{:<22} install:{:<14} hosts:{:<26} src:{}",
                rec.id,
                st.capability_state,
                st.local_install,
                hosts.join(","),
                src,
            ));
        }
    }
    lines.push(
        "  Boundary: setup never installs; explicit per-item onboarding apply uses audited adoption."
            .to_string(),
    );
    lines.push(
        "  Next: run `ags onboarding plan`, approve one item, then verify the host.".to_string(),
    );
    lines.join("\n")
}
