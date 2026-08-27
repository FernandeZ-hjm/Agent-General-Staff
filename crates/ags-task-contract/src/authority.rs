//! TaskAuthority — the permission source for one task execution.
//!
//! Derived from the task card at prepare time and recorded inside the
//! prepare evidence event (the evidence log stays the single source of
//! truth). Task state is also derived from evidence events; no separate
//! state file is ever written.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::template::CARD_FIELDS;
use crate::validator::validate_body;

/// What one task execution is allowed to do. `writable_resources` and
/// `capability_ceiling` intersect with the workspace guardrails; an empty
/// ceiling means "no extra limit beyond guardrails".
#[derive(Debug, Clone, Serialize)]
pub struct TaskAuthority {
    pub card_hash: String,
    pub goals: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub writable_resources: Vec<String>,
    pub capability_ceiling: Vec<String>,
    pub verification: Vec<String>,
    pub review_required: bool,
}

/// Derive the authority from a validated card body.
pub fn derive_authority(body: &str, card_hash: &str) -> Result<TaskAuthority, String> {
    let errors = validate_body("card", body);
    if !errors.is_empty() {
        return Err(format!("card is not valid: {:?}", errors[0].code));
    }
    let map = field_map(body);
    let level = map.get("任务级别：").map(|s| s.trim()).unwrap_or("");
    let stop = map.get("停止条件：").map(|s| s.as_str()).unwrap_or("");
    let review_required = level == "Heavy"
        || stop.contains("评审")
        || stop.contains("review")
        || stop.contains("Review");
    Ok(TaskAuthority {
        card_hash: card_hash.to_string(),
        goals: ids(map.get("目标：").map(|s| s.as_str()).unwrap_or(""), "G-"),
        acceptance_criteria: ids(
            map.get("验收标准：").map(|s| s.as_str()).unwrap_or(""),
            "AC-",
        ),
        writable_resources: writable(map.get("写边界：").map(|s| s.as_str()).unwrap_or("仓库内")),
        capability_ceiling: ceiling(map.get("能力上限：").map(|s| s.as_str()).unwrap_or("-")),
        verification: ids(map.get("验证：").map(|s| s.as_str()).unwrap_or(""), "V-"),
        review_required,
    })
}

/// Derived task state from evidence events. Order of precedence: CLOSED >
/// VERIFIED > INTEGRATED > RETURNED > ACCEPTED > DELEGATION_ISSUED >
/// PREPARED > UNKNOWN. Verification is only valid when it comes after the
/// last integration of the current round — the caller passes the events in
/// chain order, so a stale verify can never outrank a later RETURNED.
pub fn derive_state(events: &[ags_kernel::evidence::Event]) -> &'static str {
    let mut has_prepare = false;
    let mut verify_index: Option<usize> = None;
    let mut last_integrate_index: Option<usize> = None;
    for (i, event) in events.iter().enumerate() {
        match event.event_type.as_str() {
            "execution" => {
                if event.payload.get("phase").and_then(|v| v.as_str()) == Some("prepare") {
                    has_prepare = true;
                }
            }
            "delegation.integrate" => last_integrate_index = Some(i),
            "test" => {
                if event.payload.get("all_succeeded").and_then(|v| v.as_bool()) == Some(true) {
                    verify_index = Some(i);
                }
            }
            "closure" => return "CLOSED",
            _ => {}
        }
    }
    // A verify only counts when it happened after the last integration of
    // the current round; a stale verify cannot outrank a later return.
    let verified = match (verify_index, last_integrate_index) {
        (Some(vi), Some(li)) => vi > li,
        (Some(_), None) => true,
        _ => false,
    };
    if verified {
        return "VERIFIED";
    }
    if last_integrate_index.is_some() {
        return "INTEGRATED";
    }
    if events.iter().any(|e| e.event_type == "delegation.return") {
        return "RETURNED";
    }
    if events.iter().any(|e| e.event_type == "delegation.accept") {
        return "ACCEPTED";
    }
    if events.iter().any(|e| e.event_type == "delegation.issue") {
        return "DELEGATION_ISSUED";
    }
    if has_prepare {
        return "PREPARED";
    }
    "UNKNOWN"
}

fn field_map(body: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let lines: Vec<&str> = body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        for field in CARD_FIELDS {
            if let Some(rest) = line.strip_prefix(field) {
                let mut value = String::from(rest.trim());
                for next in lines.iter().skip(i + 1) {
                    if CARD_FIELDS.iter().any(|f| next.starts_with(f)) {
                        break;
                    }
                    value.push('\n');
                    value.push_str(next);
                }
                map.insert(field.to_string(), value);
            }
        }
    }
    map
}

/// Collect `PREFIX\d+` identifiers from a card field, e.g. "G-01: goal".
fn ids(field: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in field.split_whitespace() {
        let token = token.trim_matches(|c: char| c == ',' || c == ':' || c == '。' || c == '，');
        if let Some(rest) = token.strip_prefix(prefix) {
            if rest
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                out.push(token.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 写边界 → resource list; "仓库内"/"in-repo" expands to the workspace root.
fn writable(field: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in field.split_whitespace().filter(|t| !t.is_empty()) {
        let token = token.trim_matches(',');
        if token == "仓库内" || token == "in-repo" {
            out.push(".".to_string());
        } else {
            out.push(token.to_string());
        }
    }
    out
}

/// 能力上限 → ceiling list; `-` / `全部` means no extra limit.
fn ceiling(field: &str) -> Vec<String> {
    let field = field.trim();
    if field.is_empty() || field == "-" || field == "全部" {
        return vec![];
    }
    field
        .split_whitespace()
        .map(|t| t.trim_matches(',').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CARD: &str = "## 任务卡

读取并遵守：
- AGENTS.md

Contract ID: tc-0123456789abcdef
Executor: Other
任务级别：Medium
任务：demo
目标：
- G-01: do the thing
- G-02: keep it clean
验收标准：
- AC-01 -> G-01: result exists
- AC-02 -> G-02: clean state
验证：
- V-01 -> AC-01: cargo test
- EV-01 -> AC-01: test output
- V-02 -> AC-02: git status clean
- EV-02 -> AC-02: clean output
能力上限：skill:database-migration mcp:lark-doc
写边界：src tests
拓扑：single
停止条件：
- 需要独立评审时停下
相关路径：- src/
交付方式：ags-run
";

    #[test]
    fn authority_derives_from_card() {
        let a = derive_authority(CARD, "h1").unwrap();
        assert_eq!(a.goals, vec!["G-01", "G-02"]);
        assert_eq!(a.acceptance_criteria, vec!["AC-01", "AC-02"]);
        assert_eq!(a.verification, vec!["V-01", "V-02"]);
        assert_eq!(a.writable_resources, vec!["src", "tests"]);
        assert_eq!(
            a.capability_ceiling,
            vec!["skill:database-migration", "mcp:lark-doc"]
        );
        assert!(a.review_required);
    }

    #[test]
    fn in_repo_boundary_and_dash_ceiling() {
        let card = CARD
            .replace(
                "能力上限：skill:database-migration mcp:lark-doc",
                "能力上限：-",
            )
            .replace("写边界：src tests", "写边界：仓库内");
        let a = derive_authority(&card, "h2").unwrap();
        assert_eq!(a.writable_resources, vec!["."]);
        assert!(a.capability_ceiling.is_empty());
    }

    #[test]
    fn state_derives_from_events() {
        let events = |types: &[&str]| {
            types
                .iter()
                .enumerate()
                .map(|(i, t)| ags_kernel::evidence::Event {
                    v: 3,
                    ts: format!("t{i}"),
                    event_type: t.to_string(),
                    event_id: format!("e{i}"),
                    workspace: "ws".to_string(),
                    task_card_hash: Some("h".to_string()),
                    scope: "local".to_string(),
                    agent_instance_id: None,
                    parent_instance_id: None,
                    payload: if *t == "test" {
                        json!({"all_succeeded": true})
                    } else if *t == "execution" {
                        json!({"phase": "prepare"})
                    } else {
                        json!({})
                    },
                    prev_sha256: None,
                    sha256: String::new(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(derive_state(&events(&[])), "UNKNOWN");
        assert_eq!(derive_state(&events(&["execution"])), "PREPARED");
        assert_eq!(derive_state(&events(&["execution", "test"])), "VERIFIED");
        assert_eq!(
            derive_state(&events(&[
                "execution",
                "delegation.issue",
                "delegation.accept"
            ])),
            "ACCEPTED"
        );
        assert_eq!(
            derive_state(&events(&[
                "execution",
                "delegation.issue",
                "delegation.return",
                "delegation.integrate",
                "test"
            ])),
            "VERIFIED"
        );
        assert_eq!(
            derive_state(&events(&["execution", "delegation.return"])),
            "RETURNED"
        );
        // A stale verify BEFORE integration must not count.
        assert_eq!(
            derive_state(&events(&[
                "execution",
                "test",
                "delegation.issue",
                "delegation.return",
                "delegation.integrate"
            ])),
            "INTEGRATED"
        );
        assert_eq!(
            derive_state(&events(&["execution", "test", "closure"])),
            "CLOSED"
        );
    }
}
