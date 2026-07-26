use super::model::SCHEMA_VERSION;
use serde::Serialize;
use std::path::Path;

/// A single skill discovered on disk.
#[derive(Debug, Clone, Serialize)]
pub struct SkillInventoryEntry {
    pub name: String,
    pub path: String,
    /// "global" | "optional" | "personal"
    pub source_category: String,
    pub has_skill_md: bool,
    pub description_present: bool,
    pub risk_hints: Vec<String>,
    pub public_allowed_guess: bool,
    /// SKILL.md last-modified marker (`epoch:<secs>`), when available.
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInventorySummary {
    pub total: usize,
    pub global: usize,
    pub optional: usize,
    pub personal: usize,
    pub with_skill_md: usize,
    pub with_description: usize,
    pub public_allowed: usize,
    pub flagged_risk: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInventoryResult {
    pub schema_version: String,
    pub roots_scanned: Vec<String>,
    pub entries: Vec<SkillInventoryEntry>,
    pub summary: SkillInventorySummary,
}

/// High-signal substrings hinting a skill may carry elevated risk or be
/// unsuitable for public distribution. Scanned only against SKILL.md text.
const RISK_HINT_KEYWORDS: &[&str] = &[
    "secret",
    "token",
    "credential",
    "password",
    "api key",
    "api_key",
    "bearer",
    "node_secret",
    "rm -rf",
    "sudo ",
    "git reset --hard",
    "--force",
    "force push",
    "drop table",
];

/// Scan `global-skills/` and `skill-packs/{optional,personal}/` for skill
/// assets, reading each `SKILL.md` front-matter. Read-only over SKILL.md.
pub fn scan_skill_inventory(root: &Path) -> SkillInventoryResult {
    let roots: [(&str, std::path::PathBuf); 3] = [
        ("global", root.join("global-skills")),
        ("optional", root.join("skill-packs/optional")),
        ("personal", root.join("skill-packs/personal")),
    ];

    let mut entries: Vec<SkillInventoryEntry> = Vec::new();
    let mut roots_scanned: Vec<String> = Vec::new();

    for (category, dir) in &roots {
        if !dir.is_dir() {
            continue;
        }
        roots_scanned.push(dir.display().to_string());
        let mut skill_dirs: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(_) => continue,
        };
        skill_dirs.sort();
        for skill_dir in skill_dirs {
            entries.push(inventory_entry(category, &skill_dir));
        }
    }

    let summary = SkillInventorySummary {
        total: entries.len(),
        global: entries
            .iter()
            .filter(|e| e.source_category == "global")
            .count(),
        optional: entries
            .iter()
            .filter(|e| e.source_category == "optional")
            .count(),
        personal: entries
            .iter()
            .filter(|e| e.source_category == "personal")
            .count(),
        with_skill_md: entries.iter().filter(|e| e.has_skill_md).count(),
        with_description: entries.iter().filter(|e| e.description_present).count(),
        public_allowed: entries.iter().filter(|e| e.public_allowed_guess).count(),
        flagged_risk: entries.iter().filter(|e| !e.risk_hints.is_empty()).count(),
    };

    SkillInventoryResult {
        schema_version: SCHEMA_VERSION.to_string(),
        roots_scanned,
        entries,
        summary,
    }
}

fn inventory_entry(category: &str, skill_dir: &Path) -> SkillInventoryEntry {
    let dir_name = skill_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    let skill_md = skill_dir.join("SKILL.md");
    let has_skill_md = skill_md.is_file();

    let mut name = dir_name;
    let mut description_present = false;
    let mut risk_hints: Vec<String> = Vec::new();
    let mut last_seen: Option<String> = None;

    if has_skill_md {
        if let Ok(meta) = std::fs::metadata(&skill_md) {
            if let Ok(modified) = meta.modified() {
                last_seen = format_system_time(modified);
            }
        }
        // Read ONLY SKILL.md — never other files in the skill directory.
        if let Ok(text) = std::fs::read_to_string(&skill_md) {
            let (fm_name, fm_desc) = parse_front_matter(&text);
            if let Some(n) = fm_name {
                if !n.trim().is_empty() {
                    name = n.trim().to_string();
                }
            }
            description_present = fm_desc.map(|d| !d.trim().is_empty()).unwrap_or(false);
            let lower = text.to_lowercase();
            for kw in RISK_HINT_KEYWORDS {
                if lower.contains(kw) {
                    risk_hints.push((*kw).trim().to_string());
                }
            }
        }
    }

    // public_allowed guess: personal skills are never public; others are
    // public-safe candidates only when no risk hints were detected.
    let public_allowed_guess = category != "personal" && risk_hints.is_empty();

    SkillInventoryEntry {
        name,
        path: skill_dir.display().to_string(),
        source_category: category.to_string(),
        has_skill_md,
        description_present,
        risk_hints,
        public_allowed_guess,
        last_seen,
    }
}

/// Extract `name:` and `description:` from a leading `--- ... ---` YAML
/// front-matter block. Returns (name, description); robust to absent fields.
pub(crate) fn parse_front_matter(text: &str) -> (Option<String>, Option<String>) {
    let trimmed = text.trim_start();
    let Some(after_open) = trimmed.strip_prefix("---") else {
        return (None, None);
    };
    let Some(end) = after_open.find("\n---") else {
        return (None, None);
    };
    let yaml = &after_open[..end];
    match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
        Ok(value) => {
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let description = value
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (name, description)
        }
        Err(_) => (None, None),
    }
}

/// Format a SystemTime as an epoch-seconds marker (avoids extra date deps).
fn format_system_time(t: std::time::SystemTime) -> Option<String> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| format!("epoch:{}", d.as_secs()))
}
