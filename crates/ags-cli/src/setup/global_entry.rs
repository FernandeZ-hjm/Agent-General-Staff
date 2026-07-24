use crate::host_platforms::AGENT_PLATFORM_SPECS;
use crate::setup::plan::PrivateInstallPlan;
use std::path::Path;

const AGS_ENTRY_BLOCK_BEGIN: &str = "<!-- BEGIN AGS managed entry -->";
const AGS_ENTRY_BLOCK_END: &str = "<!-- END AGS managed entry -->";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryBlockOutcome {
    Created,
    Updated,
    Conflict,
}
/// Merge an AGS managed block into an existing entry-file body. A single
/// well-formed existing block is replaced (Updated); an absent block is
/// appended (Created); a malformed / duplicated marker yields Conflict and the
/// content is returned unchanged so the caller stops instead of overwriting
/// user content.
fn merge_ags_entry_block(existing: &str, body: &str) -> (String, EntryBlockOutcome) {
    let begin = existing.matches(AGS_ENTRY_BLOCK_BEGIN).count();
    let end = existing.matches(AGS_ENTRY_BLOCK_END).count();
    let block = format!(
        "{AGS_ENTRY_BLOCK_BEGIN}\n{}\n{AGS_ENTRY_BLOCK_END}",
        body.trim_end()
    );
    if begin == 0 && end == 0 {
        let trimmed = existing.trim_end_matches('\n');
        if trimmed.is_empty() {
            return (format!("{block}\n"), EntryBlockOutcome::Created);
        }
        return (
            format!("{trimmed}\n\n{block}\n"),
            EntryBlockOutcome::Created,
        );
    }
    if begin == 1 && end == 1 {
        if let (Some(bi), Some(ei)) = (
            existing.find(AGS_ENTRY_BLOCK_BEGIN),
            existing.find(AGS_ENTRY_BLOCK_END),
        ) {
            if bi < ei {
                let before = &existing[..bi];
                let after = &existing[ei + AGS_ENTRY_BLOCK_END.len()..];
                return (
                    format!("{before}{block}{after}"),
                    EntryBlockOutcome::Updated,
                );
            }
        }
    }
    (existing.to_string(), EntryBlockOutcome::Conflict)
}
/// Write the AGS-owned global entry managed block into
/// `<target>/ags-global-entry.md` (an AGS runtime file — never a host config).
/// Incremental: updates an existing block, appends a missing one, and stops on
/// a malformed block. Confirm-gated because it only runs on the setup apply path.
pub(in crate::setup) fn write_ags_global_entry(target: &Path) -> suite_doctor::Finding {
    let path = target.join("ags-global-entry.md");
    let body = "@AGENTS.md\n@CLAUDE.md\n@hosts/host-entry-policy.md\nAGS managed global entry — five-segment chain: setup → agents → skill → init → update.";
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let (content, outcome) = merge_ags_entry_block(&existing, body);
    if outcome == EntryBlockOutcome::Conflict {
        return suite_doctor::Finding::fail(
            "global-entry-managed-block",
            "ags-global-entry.md has a malformed AGS managed block; not overwriting",
            "fix or remove the AGS managed block manually",
        );
    }
    match std::fs::write(&path, content) {
        Ok(()) => suite_doctor::Finding::pass(
            "global-entry-managed-block",
            format!(
                "ags-global-entry.md managed block {}",
                if outcome == EntryBlockOutcome::Created {
                    "created"
                } else {
                    "updated"
                }
            ),
        ),
        Err(e) => suite_doctor::Finding::fail(
            "global-entry-managed-block",
            "could not write ags-global-entry.md",
            e.to_string(),
        ),
    }
}
#[derive(Debug, Clone)]
pub(in crate::setup) struct GlobalEntryTemplate {
    id: String,
    /// ags-self | host-entry | project-init
    class: &'static str,
    target_path: String,
    /// managed-block | advise-only | use-ags-init
    write_method: &'static str,
    /// present | missing | advise-only | per-project
    status: String,
    confirm_needed: bool,
}
/// Inventory the AGS-relevant global entry protocol templates in three classes.
pub(in crate::setup) fn global_entry_protocol_plan(
    plan: &PrivateInstallPlan,
) -> Vec<GlobalEntryTemplate> {
    let mut out = Vec::new();
    // Class 1 — AGS-self global kernel templates (staged under the runtime home).
    for f in &plan.files {
        let rel = f
            .path
            .strip_prefix(&plan.target)
            .unwrap_or(&f.path)
            .to_string_lossy()
            .to_string();
        out.push(GlobalEntryTemplate {
            id: rel,
            class: "ags-self",
            target_path: f.path.display().to_string(),
            write_method: "managed-block",
            status: if f.path.exists() {
                "present"
            } else {
                "missing"
            }
            .to_string(),
            confirm_needed: !f.path.exists(),
        });
    }
    out.push(GlobalEntryTemplate {
        id: "ags-global-entry.md".to_string(),
        class: "ags-self",
        target_path: plan
            .target
            .join("ags-global-entry.md")
            .display()
            .to_string(),
        write_method: "managed-block",
        status: if plan.target.join("ags-global-entry.md").exists() {
            "present"
        } else {
            "missing"
        }
        .to_string(),
        confirm_needed: true,
    });
    // Class 2 — host global entries (advise-only; AGS never writes host config).
    for spec in AGENT_PLATFORM_SPECS {
        let config_targets = spec.config_subdirs.join(" / ");
        out.push(GlobalEntryTemplate {
            id: format!("host:{} ({})", spec.id, spec.display),
            class: "host-entry",
            target_path: format!("{config_targets} config (advise only)"),
            write_method: "advise-only",
            status: "advise-only".to_string(),
            confirm_needed: false,
        });
    }
    // Class 3 — project-init entries (owned by `ags init`).
    out.push(GlobalEntryTemplate {
        id: "project: entry files + memory capsule".to_string(),
        class: "project-init",
        target_path: "<project root> via `ags init`".to_string(),
        write_method: "use-ags-init",
        status: "per-project".to_string(),
        confirm_needed: false,
    });
    out
}
pub(in crate::setup) fn render_global_entry_protocol_text(
    entries: &[GlobalEntryTemplate],
) -> String {
    let mut lines = vec![
        "Global Entry Protocol Templates".to_string(),
        "===============================".to_string(),
        "Mode: plan-only by default. AGS-self templates are confirm-gated by `--yes`; host entries are advise-only — AGS never writes host config (~/.claude, ~/.codex, WorkBuddy, CodeBuddy).".to_string(),
    ];
    for class in ["ags-self", "host-entry", "project-init"] {
        let group: Vec<&GlobalEntryTemplate> =
            entries.iter().filter(|e| e.class == class).collect();
        if group.is_empty() {
            continue;
        }
        lines.push(format!("  [{class}]"));
        for e in group {
            lines.push(format!(
                "    - {:<46} {:<12} via {} ({})",
                e.id, e.status, e.write_method, e.target_path
            ));
        }
    }
    lines.push("NOTE: this gate always runs in `ags setup`; without confirmation no user global entry file is written.".to_string());
    lines.join("\n")
}
pub(in crate::setup) fn global_entry_protocol_json(
    entries: &[GlobalEntryTemplate],
) -> serde_json::Value {
    serde_json::json!({
        "gate": "always-shown",
        "templates": entries
            .iter()
            .map(|e| serde_json::json!({
                "id": e.id,
                "class": e.class,
                "target_path": e.target_path,
                "write_method": e.write_method,
                "status": e.status,
                "confirm_needed": e.confirm_needed,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod entry_block_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_home(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-xplat-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn merge_ags_entry_block_create_update_conflict() {
        // absent → created (appended after existing user content, preserved).
        let (c1, o1) = merge_ags_entry_block("# user notes\n", "AGS body");
        assert_eq!(o1, EntryBlockOutcome::Created);
        assert!(c1.contains("# user notes"));
        assert!(c1.contains("BEGIN AGS managed entry"));
        // present → updated (single well-formed block replaced, idempotent).
        let (c2, o2) = merge_ags_entry_block(&c1, "AGS body v2");
        assert_eq!(o2, EntryBlockOutcome::Updated);
        assert!(c2.contains("AGS body v2"));
        assert!(!c2.contains("AGS body\n")); // old body replaced
        assert_eq!(c2.matches("BEGIN AGS managed entry").count(), 1);
        // malformed (begin without end) → conflict, content unchanged.
        let malformed = "x\n<!-- BEGIN AGS managed entry -->\nstray\n";
        let (c3, o3) = merge_ags_entry_block(malformed, "AGS body");
        assert_eq!(o3, EntryBlockOutcome::Conflict);
        assert_eq!(c3, malformed);
    }

    #[test]
    fn write_ags_global_entry_is_incremental_and_stops_on_conflict() {
        let target = temp_home("global-entry");
        // first write → created.
        let f1 = write_ags_global_entry(&target);
        assert!(f1.message.contains("created"));
        let path = target.join("ags-global-entry.md");
        assert!(path.is_file());
        let first = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first.matches("BEGIN AGS managed entry").count(), 1);
        // second write → updated, still a single block (idempotent shape).
        let _f2 = write_ags_global_entry(&target);
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(second.matches("BEGIN AGS managed entry").count(), 1);
        // malformed block → conflict, file left unchanged (no overwrite).
        std::fs::write(&path, "<!-- BEGIN AGS managed entry -->\nbroken\n").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let f3 = write_ags_global_entry(&target);
        assert!(f3.message.contains("malformed"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&target);
    }
}
