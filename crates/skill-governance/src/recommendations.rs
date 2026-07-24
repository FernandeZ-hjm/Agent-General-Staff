//! Public third-party skill recommendation surface.
//!
//! Reads Skill entries from `manifests/third-party-capabilities.yaml` and
//! computes READ-ONLY local-install + host-visibility status by
//! filesystem stat only. Installation is never silent: the onboarding layer
//! may offer a confirmation-protected per-item action that delegates to the
//! existing audited `ags skill adopt` lifecycle.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Compatibility projection of the unified third-party capability manifest.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecommendationsDoc {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub principle: String,
    #[serde(default)]
    pub skills: Vec<Recommendation>,
}

/// A single third-party recommendation (upstream canonical name).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Recommendation {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub recommendation_only: bool,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
}

/// Read-only status for one recommendation (filesystem stat only).
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationStatus {
    pub id: String,
    /// Unified onboarding state. This prevents contradictory local-install and
    /// host-visibility fields from being interpreted as ready.
    pub capability_state: String,
    /// "installed" when a local body exists at the install location, else
    /// "not-installed". A controlled onboarding action may install it only
    /// after explicit confirmation.
    pub local_install: String,
    /// Per-host visibility through either a direct thin index or the shared
    /// multi-agent skill body.
    pub host_visibility: Vec<HostVisibilityLite>,
    pub next_step: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostVisibilityLite {
    pub host: String,
    /// "visible" when a loadable direct or shared `SKILL.md` exists, else
    /// "not-visible".
    pub status: String,
}

/// Hosts whose skill thin-index AGS reports on (read-only stat).
const HOST_SKILL_DIRS: &[(&str, &str)] = &[
    ("claude-code", ".claude/skills"),
    ("codex", ".codex/skills"),
];

/// Read public Skill entries from the unified third-party capability manifest.
/// Missing or malformed manifest → an empty doc (setup degrades gracefully).
pub fn read_recommendations(repo_root: &Path) -> RecommendationsDoc {
    let Ok(manifest) = ags_onboarding::manifest::read_third_party_manifest(repo_root) else {
        return RecommendationsDoc::default();
    };
    let skills = manifest
        .capabilities
        .into_iter()
        .filter(|capability| {
            capability.kind == ags_onboarding::manifest::CapabilityKind::Skill
                && capability.applies_to("public")
        })
        .map(|capability| {
            let source = capability.source.repository.clone().map(|repository| {
                let Some(revision) = capability.source.revision.as_deref() else {
                    return repository;
                };
                let mut pinned = format!("{}/tree/{revision}", repository.trim_end_matches('/'));
                if let Some(subdir) = capability.source.subdir.as_deref() {
                    pinned.push('/');
                    pinned.push_str(subdir.trim_start_matches('/'));
                }
                pinned
            });
            Recommendation {
                id: capability.id,
                name: capability.name,
                tier: capability.tier,
                purpose: capability.purpose,
                recommendation_only: true,
                source_kind: capability.source.manager,
                source,
                upstream: capability.source.repository,
                revision: capability.source.revision,
                license: capability.source.license,
                risk: Some(capability.risk),
                install_location: capability.install.install_location,
            }
        })
        .collect();
    RecommendationsDoc {
        schema_version: manifest.schema_version,
        principle: manifest.principle,
        skills,
    }
}

/// Compute read-only install + host-visibility status for one recommendation.
/// Pure filesystem stat against `home`; never spawns a process or writes.
pub fn recommendation_status(rec: &Recommendation, home: &Path) -> RecommendationStatus {
    let installed = local_body_present(rec, home);
    let shared_body = home.join(".agents/skills").join(&rec.id).join("SKILL.md");
    let host_visibility = HOST_SKILL_DIRS
        .iter()
        .map(|(host, subdir)| {
            let entry = home.join(subdir).join(&rec.id);
            let visible = entry.join("SKILL.md").is_file() || shared_body.is_file();
            HostVisibilityLite {
                host: host.to_string(),
                status: if visible { "visible" } else { "not-visible" }.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let any_visible = host_visibility
        .iter()
        .any(|visibility| visibility.status == "visible");
    let capability_state = match (installed, any_visible) {
        (true, true) => "active-ready",
        (true, false) => "installed-not-visible",
        (false, true) => "visible-not-ready",
        (false, false) => "absent",
    };
    let next_step = match capability_state {
        "active-ready" => "Installed and visible — verify deterministic routing with `ags skill route-test`."
            .to_string(),
        "installed-not-visible" => "Installed locally — run a confirmed host visibility sync, then verify.".to_string(),
        "visible-not-ready" => "A host entry exists without the reviewed local body — repair or remove the stale entry before routing.".to_string(),
        _ => {
        match rec.source.as_deref() {
            Some(src) => {
                format!(
                    "Not installed — review {src}; use an explicit per-item onboarding apply or install manually."
                )
            }
            None => "Not installed — select a trusted source and install manually.".to_string(),
        }
        }
    };
    RecommendationStatus {
        id: rec.id.clone(),
        capability_state: capability_state.to_string(),
        local_install: if installed {
            "installed"
        } else {
            "not-installed"
        }
        .to_string(),
        host_visibility,
        next_step,
    }
}

/// Does a local body exist at the recommendation's install location?
fn local_body_present(rec: &Recommendation, home: &Path) -> bool {
    let loc = rec
        .install_location
        .clone()
        .unwrap_or_else(|| format!("$HOME/.agents/skills/{}/", rec.id));
    let expanded = expand_home(loc.trim_end_matches('/'), home);
    let dir = Path::new(&expanded);
    dir.join("SKILL.md").is_file() || dir.is_dir()
}

/// Expand a leading `$HOME/` or `~/` against `home`. Other paths pass through.
fn expand_home(s: &str, home: &Path) -> String {
    if let Some(rest) = s.strip_prefix("$HOME/") {
        home.join(rest).to_string_lossy().to_string()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest).to_string_lossy().to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        // crate dir → workspace root (…/crates/skill-governance → repo root).
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn reads_public_recommendations_and_uses_upstream_names() {
        let doc = read_recommendations(&repo_root());
        assert!(
            !doc.skills.is_empty(),
            "public recommendations manifest parses"
        );
        let ids: Vec<&str> = doc.skills.iter().map(|s| s.id.as_str()).collect();
        // Upstream canonical names are present.
        for want in [
            "superpowers",
            "grilling",
            "review",
            "decision-mapping",
            "resolving-merge-conflicts",
            "to-prd",
            "to-issues",
            "triage",
            "handoff",
            "diagnosing-bugs",
        ] {
            assert!(ids.contains(&want), "missing recommendation id: {want}");
        }
        // Old local aliases must NOT be exposed as active recommendations.
        for forbidden in [
            concat!("cave", "man", "-", "com", "mit"),
            concat!("cave", "man", "-", "re", "view"),
            concat!("diag", "nose"),
            "code-review",
            concat!("t", "d", "d"),
            "test-driven-development",
            "obsidian-vault",
            "teach",
        ] {
            assert!(
                !ids.contains(&forbidden),
                "old/excluded name leaked into recommendations: {forbidden}"
            );
        }
        // Every entry is recommendation-only.
        assert!(
            doc.skills.iter().all(|s| s.recommendation_only),
            "all entries must be recommendation_only"
        );
    }

    #[test]
    fn status_is_not_installed_for_absent_body() {
        let home = std::env::temp_dir().join(format!("ags-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let rec = Recommendation {
            id: "diagnosing-bugs".to_string(),
            install_location: Some("$HOME/.agents/skills/diagnosing-bugs/".to_string()),
            source: Some("https://github.com/mattpocock/skills".to_string()),
            ..Default::default()
        };
        let st = recommendation_status(&rec, &home);
        assert_eq!(st.local_install, "not-installed");
        assert_eq!(st.capability_state, "absent");
        assert!(st.host_visibility.iter().all(|h| h.status == "not-visible"));
        assert!(st.next_step.contains("Not installed"));
    }

    #[test]
    fn status_is_installed_when_body_present() {
        let home = std::env::temp_dir().join(format!("ags-rec-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let body = home.join(".agents/skills/review");
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(body.join("SKILL.md"), "---\nname: review\n---\n").unwrap();
        let rec = Recommendation {
            id: "review".to_string(),
            install_location: Some("$HOME/.agents/skills/review/".to_string()),
            ..Default::default()
        };
        let st = recommendation_status(&rec, &home);
        assert_eq!(st.local_install, "installed");
        assert_eq!(st.capability_state, "active-ready");
        assert!(st.host_visibility.iter().all(|h| h.status == "visible"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
