//! Public third-party skill recommendation surface.
//!
//! Reads Skill entries from `manifests/third-party-capabilities.yaml` and
//! computes READ-ONLY local-install + host-visibility status by
//! filesystem stat only. Skill installation is never offered by onboarding;
//! an explicit external-manager install/update must finish before one static
//! host snapshot refresh.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Read-only public-skill view of the unified third-party capability manifest.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecommendationsDoc {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub principle: String,
    #[serde(default)]
    pub skills: Vec<CatalogEntry>,
}

/// A discovery-only catalog entry (upstream canonical name). It deliberately
/// carries no local installation or activation state.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CatalogEntry {
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
    pub tracking_ref: Option<String>,
    #[serde(default)]
    pub integrity: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
}

/// Compatibility name for callers of the earlier recommendation surface.
pub type Recommendation = CatalogEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogLayerState {
    Recommended,
    Unlisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallationLayerState {
    NotInstalled,
    Installed,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationLayerState {
    NotInstalled,
    NotActivated,
    Partial,
    RouteVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateLayerState {
    NotInstalled,
    Notify,
    Manual,
    Pinned,
    RebindRequired,
}

/// Canonical layered read model consumed by current JSON clients. Catalog,
/// installation and activation are independent facts; none implies another.
#[derive(Debug, Clone, Serialize)]
pub struct SkillStatusProjection {
    pub schema_version: &'static str,
    pub skill_id: String,
    pub catalog: CatalogLayer,
    pub installation: InstallationLayer,
    pub activation: ActivationLayer,
    pub update: UpdateLayer,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogLayer {
    pub state: CatalogLayerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<CatalogEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallationLayer {
    pub state: InstallationLayerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::skill_adoption::InstalledSkillRecord>,
    pub unmanaged_body_observed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivationLayer {
    pub state: ActivationLayerState,
    pub routes: crate::skill_adoption::AdoptionRouteStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateLayer {
    pub state: UpdateLayerState,
    pub upstream_bound: bool,
}

/// Read public Skill entries from the unified third-party capability manifest.
/// Missing or malformed manifest → an empty doc (setup degrades gracefully).
pub fn read_recommendations(repo_root: &Path) -> RecommendationsDoc {
    let Ok(manifest) = crate::third_party_manifest::read_third_party_manifest(repo_root) else {
        return RecommendationsDoc::default();
    };
    let skills = manifest
        .capabilities
        .into_iter()
        .filter(|capability| {
            capability.kind == crate::third_party_manifest::CapabilityKind::Skill
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
            CatalogEntry {
                id: capability.id,
                name: capability.name,
                tier: capability.tier,
                purpose: capability.purpose,
                recommendation_only: true,
                source_kind: capability.source.manager,
                source,
                upstream: capability.source.repository,
                revision: capability.source.revision,
                tracking_ref: capability.source.tracking_ref,
                integrity: capability.source.integrity,
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

/// Join the discovery-only catalog with the two machine-local fact layers.
/// Filesystem coincidence cannot manufacture an installation claim.
pub fn skill_status_projection(
    repo_root: &Path,
    runtime_home: &Path,
    home: &Path,
    skill_id: &str,
) -> Result<SkillStatusProjection, String> {
    let skill_id = skill_id.trim();
    if skill_id.is_empty() {
        return Err("Skill id is empty".to_string());
    }
    skill_status_projections(repo_root, runtime_home, home, &[skill_id.to_string()])?
        .into_iter()
        .next()
        .ok_or_else(|| "Skill status projection was not produced".to_string())
}

/// Build layered status for multiple Skills from one catalog/index/snapshot
/// read. This is the canonical JSON-client path; the singular helper is only a
/// convenience wrapper over it.
pub fn skill_status_projections(
    repo_root: &Path,
    runtime_home: &Path,
    home: &Path,
    skill_ids: &[String],
) -> Result<Vec<SkillStatusProjection>, String> {
    if skill_ids.iter().any(|skill_id| skill_id.trim().is_empty()) {
        return Err("Skill id is empty".to_string());
    }
    let catalog = read_recommendations(repo_root)
        .skills
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let index = crate::skill_adoption::load_installed_skills(runtime_home)?;
    let mut routes =
        crate::skill_adoption::verify_adoption_routes_batch(runtime_home, home, skill_ids)?;
    skill_ids
        .iter()
        .map(|skill_id| {
            let catalog_entry = catalog.get(skill_id).cloned();
            let record = index.skills.get(skill_id).cloned();
            let routes = routes
                .remove(skill_id)
                .ok_or_else(|| format!("missing route projection for `{skill_id}`"))?;
            Ok(project_status(
                skill_id,
                catalog_entry,
                record,
                routes,
                home,
            ))
        })
        .collect()
}

fn project_status(
    skill_id: &str,
    catalog_entry: Option<CatalogEntry>,
    record: Option<crate::skill_adoption::InstalledSkillRecord>,
    routes: crate::skill_adoption::AdoptionRouteStatus,
    home: &Path,
) -> SkillStatusProjection {
    let unmanaged_body_observed = catalog_entry
        .as_ref()
        .is_some_and(|entry| local_body_present(entry, home));
    let installation_state = match record.as_ref() {
        None => InstallationLayerState::NotInstalled,
        Some(_) if !routes.installation.body_present || !routes.installation.body_hash_matches => {
            InstallationLayerState::Invalid
        }
        Some(_) => InstallationLayerState::Installed,
    };
    let activation_state = match installation_state {
        InstallationLayerState::NotInstalled => ActivationLayerState::NotInstalled,
        InstallationLayerState::Invalid => ActivationLayerState::NotActivated,
        InstallationLayerState::Installed if routes.verified_on_all_targets() => {
            ActivationLayerState::RouteVerified
        }
        InstallationLayerState::Installed
            if routes
                .activations
                .iter()
                .any(|route| route.visible || route.snapshot_loaded || route.route_verified) =>
        {
            ActivationLayerState::Partial
        }
        InstallationLayerState::Installed => ActivationLayerState::NotActivated,
    };
    let upstream_bound = record
        .as_ref()
        .is_some_and(|record| record.source_spec.is_upstream_bound());
    let update_state = match record.as_ref() {
        None => UpdateLayerState::NotInstalled,
        Some(_) if !upstream_bound => UpdateLayerState::RebindRequired,
        Some(record) => match record.update_policy {
            crate::skill_adoption::UpdatePolicy::Notify => UpdateLayerState::Notify,
            crate::skill_adoption::UpdatePolicy::Manual => UpdateLayerState::Manual,
            crate::skill_adoption::UpdatePolicy::Pinned => UpdateLayerState::Pinned,
        },
    };
    let next_action = match (installation_state, activation_state, update_state) {
        (InstallationLayerState::NotInstalled, _, _) => "inspect-and-install",
        (InstallationLayerState::Invalid, _, _) => "recover-or-rollback",
        (_, _, UpdateLayerState::RebindRequired) => "reinstall-from-explicit-upstream",
        (_, ActivationLayerState::RouteVerified, UpdateLayerState::Pinned) => "none-pinned",
        (_, ActivationLayerState::RouteVerified, _) => "check-upstream",
        _ => "verify-activation",
    }
    .to_string();
    SkillStatusProjection {
        schema_version: "0.4.13-skill-status-projection",
        skill_id: skill_id.to_string(),
        catalog: CatalogLayer {
            state: if catalog_entry.is_some() {
                CatalogLayerState::Recommended
            } else {
                CatalogLayerState::Unlisted
            },
            entry: catalog_entry,
        },
        installation: InstallationLayer {
            state: installation_state,
            record,
            unmanaged_body_observed,
        },
        activation: ActivationLayer {
            state: activation_state,
            routes,
        },
        update: UpdateLayer {
            state: update_state,
            upstream_bound,
        },
        next_action,
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
        // crate dir → workspace root (…/crates/ags-capability-governance → repo root).
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
            "grilling",
            "code-review",
            "wayfinder",
            "resolving-merge-conflicts",
            "to-spec",
            "to-tickets",
            "triage",
            "handoff",
            "diagnosing-bugs",
            "grill-with-docs",
            "writing-for-agents",
            "improve-codebase-architecture",
        ] {
            assert!(ids.contains(&want), "missing recommendation id: {want}");
        }
        // Old local aliases must NOT be exposed as active recommendations.
        for forbidden in [
            concat!("cave", "man", "-", "com", "mit"),
            concat!("cave", "man", "-", "re", "view"),
            concat!("diag", "nose"),
            "review",
            "decision-mapping",
            "to-prd",
            "to-issues",
            "writing-great-skills",
            concat!("t", "d", "d"),
            "test-driven-development",
            "using-git-worktrees",
            "obsidian-vault",
            "teach",
            "grill-me",
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
        assert!(doc.skills.iter().all(|skill| {
            skill
                .tracking_ref
                .as_deref()
                .is_some_and(|value| !value.is_empty())
                && skill
                    .integrity
                    .as_deref()
                    .is_some_and(|value| value.starts_with("sha256:"))
        }));
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
        let runtime = home.join("runtime");
        let root = repo_root();
        let st = skill_status_projection(&root, &runtime, &home, &rec.id).unwrap();
        assert_eq!(st.installation.state, InstallationLayerState::NotInstalled);
        assert_eq!(st.activation.state, ActivationLayerState::NotInstalled);
        assert_eq!(st.next_action, "inspect-and-install");
    }

    #[test]
    fn unmanaged_body_never_becomes_installed_or_active() {
        let home = std::env::temp_dir().join(format!("ags-rec-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let body = home.join(".agents/skills/code-review");
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(body.join("SKILL.md"), "---\nname: code-review\n---\n").unwrap();
        let rec = Recommendation {
            id: "code-review".to_string(),
            install_location: Some("$HOME/.agents/skills/code-review/".to_string()),
            ..Default::default()
        };
        let runtime = home.join("runtime");
        let root = repo_root();
        let st = skill_status_projection(&root, &runtime, &home, &rec.id).unwrap();
        assert_eq!(st.installation.state, InstallationLayerState::NotInstalled);
        assert!(st.installation.unmanaged_body_observed);
        assert_eq!(st.activation.state, ActivationLayerState::NotInstalled);
        let _ = std::fs::remove_dir_all(&home);
    }
}
