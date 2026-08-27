//! Read-only skill lifecycle view.
//!
//! Installed state is derived from `~/.agents/skills`; discovery metadata is
//! embedded from the recommendation catalog. No second registry is written.

use std::collections::HashSet;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct InstalledSkill {
    pub id: String,
    pub name: Option<String>,
    pub path: String,
    pub source: Option<String>,
    pub routable: bool,
    /// Present in the existing machine lock. This is a lifecycle status, not
    /// another integrity calculation; `ags route` remains the readiness gate.
    pub registered: bool,
    pub managed: bool,
    pub body_hash: Option<String>,
    pub original_source: Option<String>,
    pub previous_revisions: Vec<String>,
    pub update_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecommendationCatalog {
    skills: Vec<RecommendationEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecommendationEntry {
    id: String,
    source: String,
    description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecommendedSkill {
    pub id: String,
    pub source: String,
    pub description: String,
    pub installed: bool,
}

pub fn list_installed() -> Result<Vec<InstalledSkill>> {
    let dir = crate::sync::skills_dir()?;
    let lock = crate::sync::load_machine_lock()?;
    let adoption = crate::skill_adoption::load_registry()?;
    let registered: HashSet<&str> = lock.entries.iter().map(|e| e.id.as_str()).collect();
    let mut rows = Vec::new();
    if !dir.is_dir() {
        return Ok(rows);
    }
    for entry in fs::read_dir(&dir).map_err(|e| crate::error::io("skills_scan_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("skills_scan_failed", &e))?;
        let id = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let skill_md = path.join("SKILL.md");
        let fm = fs::read_to_string(&skill_md)
            .ok()
            .map(|text| crate::route::parse_skill_frontmatter(&text));
        let source = fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.file_type().is_symlink())
            .and_then(|_| fs::read_link(&path).ok())
            .map(|p| p.to_string_lossy().to_string());
        rows.push(InstalledSkill {
            id: id.clone(),
            name: fm.as_ref().and_then(|f| f.name.clone()),
            path: path.to_string_lossy().to_string(),
            source,
            routable: fm.map(|f| !f.triggers.is_empty()).unwrap_or(false),
            registered: registered.contains(id.as_str()),
            managed: adoption.skills.contains_key(&id),
            body_hash: adoption
                .skills
                .get(&id)
                .map(|record| record.source_sha256.clone()),
            original_source: adoption.skills.get(&id).map(|record| record.source.clone()),
            previous_revisions: adoption
                .skills
                .get(&id)
                .map(|record| record.previous_revisions.clone())
                .unwrap_or_default(),
            update_policy: adoption
                .skills
                .get(&id)
                .map(|record| record.update_policy.clone()),
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rows)
}

pub fn recommendations(query: Option<&str>) -> Result<Vec<RecommendedSkill>> {
    let catalog: RecommendationCatalog =
        serde_json::from_str(include_str!("../templates/recommended.json"))
            .map_err(|e| Error::new("recommended_catalog_parse_failed", e.to_string()))?;
    let installed: HashSet<String> = list_installed()?.into_iter().map(|s| s.id).collect();
    let terms: Vec<String> = query
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let mut rows: Vec<RecommendedSkill> = catalog
        .skills
        .into_iter()
        .filter(|entry| {
            if terms.is_empty() {
                return true;
            }
            let haystack =
                format!("{} {} {}", entry.id, entry.description, entry.source).to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .map(|entry| RecommendedSkill {
            installed: installed.contains(&entry.id),
            id: entry.id,
            source: entry.source,
            description: entry.description,
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_catalog_is_live_and_searchable() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let all = recommendations(None).unwrap();
        assert!(all.iter().any(|r| r.id == "superpowers"));
        let filtered = recommendations(Some("database migration")).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "database-migration");
    }
}
