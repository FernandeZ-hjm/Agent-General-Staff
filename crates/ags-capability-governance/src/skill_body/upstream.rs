use super::model::SCHEMA_VERSION;
use super::read_model::yaml_field;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
struct SkillsRegistryDoc {
    registry: Option<RegistrySection>,
    skills: Option<Vec<RegistrySkill>>,
    candidate_skills: Option<Vec<RegistryCandidate>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySection {
    #[allow(dead_code)]
    version: Option<serde_yaml::Value>,
    update_policy: Option<String>,
    upstreams: Option<serde_yaml::Mapping>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySkill {
    name: Option<String>,
    profile: Option<String>,
    source: Option<RegistrySource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryCandidate {
    name: Option<String>,
    adoption_priority: Option<String>,
    adoption_mode: Option<String>,
    source: Option<RegistrySource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySource {
    #[serde(rename = "type")]
    source_type: Option<String>,
    upstream: Option<String>,
    path: Option<String>,
    relationship: Option<String>,
    update_policy: Option<String>,
}

/// A declared upstream comparison source (read-only crawl seed).
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamSourceInfo {
    pub name: String,
    pub kind: Option<String>,
    pub url: Option<String>,
    pub web: Option<String>,
    pub reference: Option<String>,
    pub cli: Option<String>,
    pub crawl: bool,
}

/// A suite skill that tracks an upstream comparison source.
#[derive(Debug, Clone, Serialize)]
pub struct WatchedSkill {
    pub name: String,
    pub profile: Option<String>,
    pub source_type: Option<String>,
    pub upstream: Option<String>,
    pub upstream_path: Option<String>,
    pub relationship: Option<String>,
    pub update_policy: Option<String>,
}

/// A declared candidate skill (evaluate-only; not yet adopted).
#[derive(Debug, Clone, Serialize)]
pub struct CandidateSkillInfo {
    pub name: String,
    pub upstream: Option<String>,
    pub adoption_priority: Option<String>,
    pub adoption_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamProposalSummary {
    pub upstreams: usize,
    pub watched_skills: usize,
    pub candidates: usize,
    /// Always `false` in this stub — no crawl/fetch is performed.
    pub crawl_performed: bool,
}

/// Result of [`upstream_proposal`].
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamProposalResult {
    pub schema_version: String,
    pub registry_present: bool,
    pub registry_parseable: bool,
    pub registry_path: String,
    pub update_policy: Option<String>,
    pub upstreams: Vec<UpstreamSourceInfo>,
    pub watched_skills: Vec<WatchedSkill>,
    pub candidates: Vec<CandidateSkillInfo>,
    pub summary: UpstreamProposalSummary,
    pub note: String,
}

/// Build a read-only upstream-comparison proposal skeleton from
/// `manifests/skills-registry.yaml`. Performs NO network access.
pub fn upstream_proposal(root: &Path) -> UpstreamProposalResult {
    let registry_path = root.join("manifests/skills-registry.yaml");
    let rel_path = "manifests/skills-registry.yaml".to_string();

    let mut result = UpstreamProposalResult {
        schema_version: SCHEMA_VERSION.to_string(),
        registry_present: registry_path.exists(),
        registry_parseable: false,
        registry_path: rel_path,
        update_policy: None,
        upstreams: Vec::new(),
        watched_skills: Vec::new(),
        candidates: Vec::new(),
        summary: UpstreamProposalSummary {
            upstreams: 0,
            watched_skills: 0,
            candidates: 0,
            crawl_performed: false,
        },
        note: UPSTREAM_STUB_NOTE.to_string(),
    };

    let Ok(content) = std::fs::read_to_string(&registry_path) else {
        return result;
    };
    let Ok(doc) = serde_yaml::from_str::<SkillsRegistryDoc>(&content) else {
        return result;
    };
    result.registry_parseable = true;

    if let Some(registry) = doc.registry {
        result.update_policy = registry.update_policy;
        if let Some(upstreams) = registry.upstreams {
            for (key, value) in &upstreams {
                let Some(name) = key.as_str() else { continue };
                result.upstreams.push(UpstreamSourceInfo {
                    name: name.to_string(),
                    kind: yaml_field(value, "type"),
                    url: yaml_field(value, "url"),
                    web: yaml_field(value, "web"),
                    reference: yaml_field(value, "ref"),
                    cli: yaml_field(value, "cli"),
                    crawl: value
                        .get("crawl")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                });
            }
        }
    }
    result.upstreams.sort_by(|a, b| a.name.cmp(&b.name));

    // A skill is "watched" when its source declares an upstream comparison
    // source (i.e. it is not a purely local canonical skill).
    if let Some(skills) = doc.skills {
        for skill in skills {
            let Some(name) = skill.name else { continue };
            if let Some(source) = skill.source {
                if source.upstream.is_some() {
                    result.watched_skills.push(WatchedSkill {
                        name,
                        profile: skill.profile,
                        source_type: source.source_type,
                        upstream: source.upstream,
                        upstream_path: source.path,
                        relationship: source.relationship,
                        update_policy: source.update_policy,
                    });
                }
            }
        }
    }

    if let Some(candidates) = doc.candidate_skills {
        for candidate in candidates {
            let Some(name) = candidate.name else { continue };
            result.candidates.push(CandidateSkillInfo {
                name,
                upstream: candidate.source.and_then(|s| s.upstream),
                adoption_priority: candidate.adoption_priority,
                adoption_mode: candidate.adoption_mode,
            });
        }
    }

    result.summary = UpstreamProposalSummary {
        upstreams: result.upstreams.len(),
        watched_skills: result.watched_skills.len(),
        candidates: result.candidates.len(),
        crawl_performed: false,
    };
    result
}

const UPSTREAM_STUB_NOTE: &str = "STUB — no network crawl, clone, or fetch was performed and no concrete diff is proposed. This lists the upstream comparison sources and the suite skills that watch them, per manifests/skills-registry.yaml. Suite-owned local files and declared external-manager bodies retain their respective canonical ownership; real crawl_then_diff_proposal is deferred to a future task. AGS never runs `npx skills` or auto-installs from upstream.";
