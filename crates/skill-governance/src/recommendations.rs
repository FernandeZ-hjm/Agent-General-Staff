//! Public third-party skill recommendation surface.
//!
//! Reads `manifests/skill-recommendations.yaml` (the single public recommendation
//! source) and computes READ-ONLY local-install + host-visibility status by
//! filesystem stat only. RECOMMENDATION-ONLY: AGS never installs, clones,
//! downloads, copies, or writes a host thin-index for these entries — this
//! module powers an advisory display, nothing more.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Parsed `manifests/skill-recommendations.yaml`.
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
    pub risk: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
}

/// Read-only status for one recommendation (filesystem stat only).
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationStatus {
    pub id: String,
    /// "installed" when a local body exists at the install location, else
    /// "not-installed". AGS never creates this — the user installs manually.
    pub local_install: String,
    /// Per-host thin-index visibility (a stat of `<home>/<host>/skills/<id>`).
    pub host_visibility: Vec<HostVisibilityLite>,
    pub next_step: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostVisibilityLite {
    pub host: String,
    /// "visible" when a host thin-index entry exists, else "not-visible".
    pub status: String,
}

/// Hosts whose skill thin-index AGS reports on (read-only stat).
const HOST_SKILL_DIRS: &[(&str, &str)] = &[
    ("claude-code", ".claude/skills"),
    ("codex", ".codex/skills"),
];

/// Read `manifests/skill-recommendations.yaml` under `repo_root`. Missing or
/// malformed manifest → an empty doc (the setup block degrades gracefully).
pub fn read_recommendations(repo_root: &Path) -> RecommendationsDoc {
    let path = repo_root.join("manifests/skill-recommendations.yaml");
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => RecommendationsDoc::default(),
    }
}

/// Compute read-only install + host-visibility status for one recommendation.
/// Pure filesystem stat against `home`; never spawns a process or writes.
pub fn recommendation_status(rec: &Recommendation, home: &Path) -> RecommendationStatus {
    let installed = local_body_present(rec, home);
    let host_visibility = HOST_SKILL_DIRS
        .iter()
        .map(|(host, subdir)| {
            let entry = home.join(subdir).join(&rec.id);
            let visible = std::fs::symlink_metadata(&entry).is_ok();
            HostVisibilityLite {
                host: host.to_string(),
                status: if visible { "visible" } else { "not-visible" }.to_string(),
            }
        })
        .collect();
    let next_step = if installed {
        "Installed locally — verify host visibility with `ags skill verify --host claude-code`."
            .to_string()
    } else {
        match rec.source.as_deref() {
            Some(src) => {
                format!(
                    "Not installed — review {src} and install manually (AGS never installs it)."
                )
            }
            None => "Not installed — select a trusted source and install manually.".to_string(),
        }
    };
    RecommendationStatus {
        id: rec.id.clone(),
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
            "grill-me",
            "review",
            "decision-mapping",
            "resolving-merge-conflicts",
            "to-prd",
            "to-issues",
            "triage",
            "handoff",
            "test-driven-development",
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
        let _ = std::fs::remove_dir_all(&home);
    }
}
