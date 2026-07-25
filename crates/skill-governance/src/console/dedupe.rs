use super::*;
#[allow(unused_imports)]
use super::{actions::*, apply_transaction::*, host_probe::*, inventory::*, model::*};
// ── Skill deduplication ──────────────────────────────────────────────────────
//
// Detects skills that appear under more than one canonical store (a name
// collision) or whose SKILL.md front-matter `name` disagrees with the directory
// name. Default dry-run: a proposal only. With `apply`, the non-keeper copies of
// a name collision are *quarantined* (moved, never deleted) into
// `governance/backups/dedupe-<stamp>/` — an AGS-owned, reversible location
// inside the repo. Canonical bodies are never deleted and host directories are
// never touched. Front-matter mismatches are always advise-only.

/// One copy of a (potentially) duplicated capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateEntry {
    pub path: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_name: Option<String>,
}

/// A planned reversible quarantine move (never a delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineMove {
    pub from: String,
    pub to: String,
}

/// A group of copies sharing one capability name (or a single mismatch entry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub name: String,
    /// name-collision | front-matter-name-mismatch
    pub reason: String,
    pub entries: Vec<DuplicateEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    pub quarantine: Vec<QuarantineMove>,
    pub advice: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeSummary {
    pub groups: usize,
    pub duplicate_entries: usize,
    pub planned_quarantines: usize,
    pub applied: usize,
    pub blocked: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeResult {
    pub schema_version: String,
    pub apply_requested: bool,
    /// dry-run | applied | failed | nothing-to-do | blocked
    pub apply_status: String,
    pub groups: Vec<DuplicateGroup>,
    pub applied_writes: Vec<String>,
    /// Successful quarantine moves (from → to) for rollback-plan construction.
    /// Cleared to empty when a partial failure is rolled back (nothing applied).
    pub applied_moves: Vec<QuarantineMove>,
    pub apply_errors: Vec<String>,
    pub summary: DedupeSummary,
    pub note: String,
}

pub(super) fn category_rank(category: &str) -> u8 {
    match category {
        "global" => 0,
        "optional" => 1,
        "personal" => 2,
        _ => 3,
    }
}

pub(super) fn dedupe_stamp() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Detect duplicate skills across the canonical stores. Read-only unless
/// `apply`. Never deletes canonical bodies; never touches host directories.
pub fn analyze_duplicates(repo_root: &Path, apply: bool) -> DedupeResult {
    use std::collections::BTreeMap;

    let scan = crate::scan_skill_inventory(repo_root);
    let stamp = dedupe_stamp();

    let mut by_name: BTreeMap<String, Vec<&crate::SkillInventoryEntry>> = BTreeMap::new();
    for e in &scan.entries {
        by_name.entry(e.name.clone()).or_default().push(e);
    }

    let mut groups: Vec<DuplicateGroup> = Vec::new();

    // 1) name collisions across stores.
    for (name, entries) in &by_name {
        if entries.len() < 2 {
            continue;
        }
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| {
            category_rank(&a.source_category)
                .cmp(&category_rank(&b.source_category))
                .then(a.path.cmp(&b.path))
        });
        let keeper = sorted.first().map(|e| e.path.clone());
        let mut quarantine = Vec::new();
        for e in sorted.iter().skip(1) {
            let base = Path::new(&e.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
            let to = repo_root
                .join("governance/backups")
                .join(format!("dedupe-{stamp}"))
                .join(format!("{}__{}", e.source_category, base))
                .display()
                .to_string();
            quarantine.push(QuarantineMove {
                from: e.path.clone(),
                to,
            });
        }
        groups.push(DuplicateGroup {
            name: name.clone(),
            reason: "name-collision".to_string(),
            entries: sorted
                .iter()
                .map(|e| DuplicateEntry {
                    path: e.path.clone(),
                    category: e.source_category.clone(),
                    declared_name: None,
                })
                .collect(),
            keeper,
            quarantine,
            advice: vec![
                "Keeper is the highest-priority store copy; review before quarantining."
                    .to_string(),
            ],
            blocked_reasons: Vec::new(),
        });
    }

    // 2) front-matter name mismatch (advise-only; a rename is a human decision).
    for e in &scan.entries {
        let base = Path::new(&e.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !base.is_empty() && base != e.name {
            groups.push(DuplicateGroup {
                name: e.name.clone(),
                reason: "front-matter-name-mismatch".to_string(),
                entries: vec![DuplicateEntry {
                    path: e.path.clone(),
                    category: e.source_category.clone(),
                    declared_name: Some(e.name.clone()),
                }],
                keeper: None,
                quarantine: Vec::new(),
                advice: vec![format!(
                    "Directory `{base}` declares front-matter name `{}`; rename is manual.",
                    e.name
                )],
                blocked_reasons: vec!["rename-is-manual".to_string()],
            });
        }
    }

    // apply: stage + validate the ENTIRE move set first, then execute; roll back
    // every successful move if any later move fails. Canonical bodies are never
    // deleted; a failure never leaves a half-quarantined set on disk.
    let mut applied_writes: Vec<String> = Vec::new();
    let mut applied_moves: Vec<QuarantineMove> = Vec::new();
    let mut apply_errors: Vec<String> = Vec::new();
    let backups_root = repo_root.join("governance/backups");
    if apply {
        let all_moves: Vec<&QuarantineMove> =
            groups.iter().flat_map(|g| g.quarantine.iter()).collect();
        // 1) pre-validate containment + destination availability for ALL moves.
        let mut staging_errors: Vec<String> = Vec::new();
        for mv in &all_moves {
            let from = Path::new(&mv.from);
            let to = Path::new(&mv.to);
            if !canonical_within_store(repo_root, from) {
                staging_errors.push(format!(
                    "blocked (source outside canonical store): {}",
                    mv.from
                ));
            } else if !to.starts_with(&backups_root) {
                staging_errors.push(format!(
                    "blocked (dest outside governance/backups): {}",
                    mv.to
                ));
            } else if to.exists() {
                staging_errors.push(format!(
                    "blocked (quarantine dest already exists): {}",
                    mv.to
                ));
            }
        }
        if !staging_errors.is_empty() {
            // zero-change abort: nothing is moved.
            apply_errors = staging_errors;
        } else {
            // 2) execute; track successful moves for rollback.
            let mut failed = false;
            for mv in &all_moves {
                let from = Path::new(&mv.from);
                let to = Path::new(&mv.to);
                if let Some(parent) = to.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        apply_errors.push(format!("mkdir {}: {e}", parent.display()));
                        failed = true;
                        break;
                    }
                }
                match std::fs::rename(from, to) {
                    Ok(()) => {
                        applied_writes.push(mv.to.clone());
                        applied_moves.push((*mv).clone());
                    }
                    Err(e) => {
                        apply_errors.push(format!("rename {} -> {}: {e}", mv.from, mv.to));
                        failed = true;
                        break;
                    }
                }
            }
            // 3) on failure, roll back successful moves (reverse order).
            if failed {
                for mv in applied_moves.iter().rev() {
                    if let Err(e) = std::fs::rename(&mv.to, &mv.from) {
                        apply_errors.push(format!("rollback failed {} -> {}: {e}", mv.to, mv.from));
                    }
                }
                applied_writes.clear();
                applied_moves.clear();
            }
        }
    }

    let groups_len = groups.len();
    let planned: usize = groups.iter().map(|g| g.quarantine.len()).sum();
    let blocked: usize = groups
        .iter()
        .filter(|g| !g.blocked_reasons.is_empty())
        .count();
    let duplicate_entries: usize = groups
        .iter()
        .filter(|g| g.reason == "name-collision")
        .map(|g| g.entries.len())
        .sum();
    let applied_len = applied_writes.len();
    let failed_len = apply_errors.len();

    let apply_status = if !apply {
        "dry-run"
    } else if !apply_errors.is_empty() {
        "failed"
    } else if planned == 0 {
        "nothing-to-do"
    } else if applied_writes.is_empty() {
        "blocked"
    } else {
        "applied"
    }
    .to_string();

    DedupeResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        apply_requested: apply,
        apply_status,
        groups,
        applied_writes,
        applied_moves,
        apply_errors,
        summary: DedupeSummary {
            groups: groups_len,
            duplicate_entries,
            planned_quarantines: planned,
            applied: applied_len,
            blocked,
            failed: failed_len,
        },
        note: "Canonical bodies are never deleted; non-keeper copies are quarantined into governance/backups (reversible). Host directories are never touched."
            .to_string(),
    }
}

/// Render a dedupe result as pretty JSON.
pub fn render_dedupe_json(result: &DedupeResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {e}"}}"#))
}

/// Render a dedupe result as human-readable text (quiet-by-default).
pub fn render_dedupe_text(result: &DedupeResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Deduplication".to_string());
    lines.push("===================".to_string());
    lines.push(format!(
        "Mode: {} | groups {} | duplicate entries {} | planned quarantines {} | applied {} | blocked {} | failed {}",
        result.apply_status,
        result.summary.groups,
        result.summary.duplicate_entries,
        result.summary.planned_quarantines,
        result.summary.applied,
        result.summary.blocked,
        result.summary.failed,
    ));
    if result.groups.is_empty() {
        lines.push("  No duplicates detected.".to_string());
    }
    for g in &result.groups {
        lines.push(format!("  [{}] {}", g.reason, g.name));
        if let Some(keeper) = &g.keeper {
            lines.push(format!("    keeper: {keeper}"));
        }
        for mv in &g.quarantine {
            lines.push(format!("    quarantine: {} -> {}", mv.from, mv.to));
        }
        for b in &g.blocked_reasons {
            lines.push(format!("    blocked: {b}"));
        }
    }
    if !result.apply_errors.is_empty() {
        lines.push("Errors:".to_string());
        for e in &result.apply_errors {
            lines.push(format!("  - {e}"));
        }
    }
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}
