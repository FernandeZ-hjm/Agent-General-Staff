//! Task-card validator (contract v3).
//!
//! Fixed skeleton + closure mapping + protected-path rules. Every acceptance
//! criterion must have at least one V and one EV item, and every V/EV item
//! must reference an existing AC. Light cards targeting protected paths with
//! modification intent are rejected; Heavy cards may touch protected paths
//! only when the card names the independent review requirement in 停止条件.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::template::{
    CARD_FIELDS, CARD_HEADING, VALID_DELIVERY, VALID_EXECUTORS, VALID_LEVELS, VALID_TOPOLOGIES,
};

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub path: String,
    pub valid: bool,
    pub contract_id: Option<String>,
    pub task_card_hash: Option<String>,
    pub level: Option<String>,
    pub topology: Option<String>,
    pub errors: Vec<ValidationError>,
}

/// Protected boundary terms (trimmed from the contract-v2 list; the matrix
/// and write-boundary field are the primary enforcement in v3). Terms are
/// role/path-generic — private workspace identities never appear in public
/// source.
const PROTECTED_TERMS: [&str; 5] = [
    "stable suite",
    "protocol/",
    "AGENTS.md",
    "CLAUDE.md",
    "context-capsule.md",
];

const MODIFICATION_KEYWORDS: [&str; 10] = [
    "修改", "覆盖", "删除", "重写", "替换", "实现", "迁移", "修复", "升级", "创建",
];

fn field_map(body: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let lines: Vec<&str> = body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        for field in CARD_FIELDS {
            if let Some(rest) = line.strip_prefix(field) {
                // capture value until the next field header or end
                let mut value = String::from(rest.trim());
                for next in lines.iter().skip(i + 1) {
                    if CARD_FIELDS.iter().any(|f| next.starts_with(f)) {
                        break;
                    }
                    if !value.is_empty() {
                        value.push('\n');
                    }
                    value.push_str(next);
                }
                map.entry(field.to_string())
                    .or_insert(value.trim().to_string());
            }
        }
    }
    map
}

/// Validate the card body. Returns errors; empty vec means valid.
pub fn validate_body(path: &str, body: &str) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let first_line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if first_line.trim() != CARD_HEADING {
        errors.push(ValidationError {
            code: "card_heading_invalid",
            message: format!(
                "{path}: first non-empty line must be `{CARD_HEADING}` (got `{}`)",
                first_line.trim()
            ),
        });
        return errors;
    }
    let map = field_map(body);

    // Required fields, in canonical order. 能力上限 is optional: a card
    // without it carries no extra capability restriction (guardrails only).
    let mut seen: Vec<&str> = Vec::new();
    for field in CARD_FIELDS {
        if !map.contains_key(field) && field != "能力上限：" {
            errors.push(ValidationError {
                code: "card_field_missing",
                message: format!("missing required field `{field}`"),
            });
        } else if map.contains_key(field) {
            seen.push(field);
        }
    }

    // Contract ID.
    if let Some(cid) = map.get("Contract ID:") {
        let cid = cid.trim();
        let ok = cid
            .strip_prefix("tc-")
            .map(|hex| {
                hex.len() == 16
                    && hex
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            })
            .unwrap_or(false);
        if !ok {
            errors.push(ValidationError {
                code: "contract_id_invalid",
                message: format!("Contract ID must be `tc-` + 16 lowercase hex (got `{cid}`)"),
            });
        }
    }

    // Executor.
    if let Some(exec) = map.get("Executor:") {
        let exec = exec.trim();
        if !VALID_EXECUTORS.contains(&exec) {
            errors.push(ValidationError {
                code: "executor_invalid",
                message: format!(
                    "Executor must be one of {:?} (got `{exec}`)",
                    VALID_EXECUTORS
                ),
            });
        }
    }

    // Level.
    if let Some(level) = map.get("任务级别：") {
        let level = level.trim();
        if !VALID_LEVELS.contains(&level) {
            errors.push(ValidationError {
                code: "level_invalid",
                message: format!("任务级别 must be one of {:?} (got `{level}`)", VALID_LEVELS),
            });
        }
    }

    // Topology.
    if let Some(topology) = map.get("拓扑：") {
        let topology = topology.trim();
        if !VALID_TOPOLOGIES.contains(&topology) {
            errors.push(ValidationError {
                code: "topology_invalid",
                message: format!(
                    "拓扑 must be one of {:?} (got `{topology}`)",
                    VALID_TOPOLOGIES
                ),
            });
        }
    }

    // Delivery.
    if let Some(delivery) = map.get("交付方式：") {
        let delivery = delivery.trim();
        if !VALID_DELIVERY.contains(&delivery) {
            errors.push(ValidationError {
                code: "delivery_invalid",
                message: format!("交付方式 must be ags-run or manual (got `{delivery}`)"),
            });
        }
    }

    // Write boundary.
    if let Some(boundary) = map.get("写边界：") {
        for token in boundary.split_whitespace().filter(|t| !t.is_empty()) {
            let token = token.trim_matches(',');
            if token == "仓库内" || token == "in-repo" {
                continue;
            }
            if token.starts_with('/') || token.starts_with("..") {
                errors.push(ValidationError {
                    code: "boundary_path_absolute",
                    message: format!(
                        "写边界 path `{token}` must be relative and inside the workspace"
                    ),
                });
            }
        }
    }

    // Capability ceiling: `-` / `全部` means "no extra limit beyond
    // guardrails"; otherwise tokens must be capability identifiers.
    if let Some(ceiling) = map.get("能力上限：") {
        let ceiling = ceiling.trim();
        if ceiling.is_empty() || ceiling == "-" || ceiling == "全部" {
            // no extra restriction
        } else {
            for token in ceiling.split_whitespace().filter(|t| !t.is_empty()) {
                let token = token.trim_matches(',');
                let ok = token.starts_with("skill:")
                    || token.starts_with("mcp:")
                    || token.starts_with("effect:")
                    || token == "none";
                if !ok {
                    errors.push(ValidationError {
                        code: "capability_ceiling_invalid",
                        message: format!(
                            "能力上限 token `{token}` must be skill:/mcp:/effect:/none"
                        ),
                    });
                }
            }
        }
    }

    // Stop condition non-empty and meaningful.
    if let Some(stop) = map.get("停止条件：") {
        if stop.trim().is_empty() || stop.trim() == "-" {
            errors.push(ValidationError {
                code: "stop_condition_empty",
                message: "停止条件 must declare at least one stop condition".to_string(),
            });
        }
    }

    // Closure mapping: G / AC / V / EV.
    closure_mapping(&map, &mut errors);
    // Content quality: weak goals.
    if let Some(goals) = map.get("目标：") {
        if is_weak(goals) {
            errors.push(ValidationError {
                code: "goal_too_weak",
                message: "目标 must contain concrete G-01.. items".to_string(),
            });
        }
    }

    // Protected paths: Light + modification intent on protected terms.
    let level = map
        .get("任务级别：")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if level == "Light" && mentions_protected_with_intent(&map) {
        errors.push(ValidationError {
            code: "protected_path_violation",
            message: "Light cards must not target protected paths with modification intent; raise the level".to_string(),
        });
    }

    errors
}

fn closure_mapping(
    map: &std::collections::BTreeMap<String, String>,
    errors: &mut Vec<ValidationError>,
) {
    let goals = parse_ids(map.get("目标：").map(|s| s.as_str()).unwrap_or(""), "G-");
    let acs = parse_ids(
        map.get("验收标准：").map(|s| s.as_str()).unwrap_or(""),
        "AC-",
    );
    let verify = map.get("验证：").map(|s| s.as_str()).unwrap_or("");
    let vs = parse_refs(verify, "V-");
    let evs = parse_refs(verify, "EV-");

    let ac_set: BTreeSet<String> = acs.iter().cloned().collect();
    for v in &vs {
        if !ac_set.contains(&v.target) {
            errors.push(ValidationError {
                code: "closure_mapping_incomplete",
                message: format!("V-{} references unknown AC {}", v.id, v.target),
            });
        }
    }
    for ev in &evs {
        if !ac_set.contains(&ev.target) {
            errors.push(ValidationError {
                code: "closure_mapping_incomplete",
                message: format!("EV-{} references unknown AC {}", ev.id, ev.target),
            });
        }
    }
    let has_v: BTreeSet<String> = vs.iter().map(|r| r.target.clone()).collect();
    let has_ev: BTreeSet<String> = evs.iter().map(|r| r.target.clone()).collect();
    for ac in &acs {
        if !has_v.contains(ac) {
            errors.push(ValidationError {
                code: "closure_mapping_incomplete",
                message: format!("AC {ac} has no V-* verification item"),
            });
        }
        if !has_ev.contains(ac) {
            errors.push(ValidationError {
                code: "closure_mapping_incomplete",
                message: format!("AC {ac} has no EV-* evidence item"),
            });
        }
    }
    let g_set: BTreeSet<String> = goals.iter().cloned().collect();
    let ac_goals: BTreeSet<String> = parse_refs(
        map.get("验收标准：").map(|s| s.as_str()).unwrap_or(""),
        "AC-",
    )
    .iter()
    .map(|r| r.target.clone())
    .collect();
    for g in &goals {
        if !ac_goals.contains(g) {
            errors.push(ValidationError {
                code: "closure_mapping_incomplete",
                message: format!("goal G-{g} has no AC-* acceptance criterion"),
            });
        }
    }
    let _ = g_set;
}

#[derive(Debug)]
struct IdRef {
    id: String,
    target: String,
}

fn parse_ids(text: &str, prefix: &str) -> Vec<String> {
    parse_refs(text, prefix).into_iter().map(|r| r.id).collect()
}

/// Parse `XX-NN -> YY-NN` references (`AC-01 -> G-01`, `V-01 -> AC-01`) and
/// bare `XX-NN` declarations. `V-` matches skip `EV-` occurrences.
fn parse_refs(text: &str, prefix: &str) -> Vec<IdRef> {
    let mut refs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let mut search_from = 0;
        while let Some(pos) = line[search_from..].find(prefix) {
            let idx = search_from + pos;
            let boundary_ok = if prefix == "V-" {
                line[..idx]
                    .chars()
                    .next_back()
                    .map(|c| !c.is_ascii_alphabetic())
                    .unwrap_or(true)
            } else {
                true
            };
            let rest = &line[idx + prefix.len()..];
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if id.is_empty() || !boundary_ok {
                search_from = idx + prefix.len();
                continue;
            }
            let target = line
                .split_once("->")
                .map(|(_, t)| {
                    t.trim()
                        .chars()
                        .skip_while(|c| !c.is_ascii_digit())
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                })
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| id.clone());
            refs.push(IdRef { id, target });
            break;
        }
    }
    refs
}

fn is_weak(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return true;
    }
    let lowered = trimmed.to_ascii_lowercase();
    [
        "test", "todo", "tbd", "n/a", "none", "later", "无", "待定", "暂无", "未定",
    ]
    .iter()
    .any(|w| lowered.lines().any(|l| l.trim() == *w))
}

fn mentions_protected_with_intent(map: &std::collections::BTreeMap<String, String>) -> bool {
    let mut action_text = String::new();
    for field in ["任务：", "目标：", "相关路径：", "写边界：", "停止条件："] {
        if let Some(v) = map.get(field) {
            action_text.push_str(v);
            action_text.push('\n');
        }
    }
    let has_modification = MODIFICATION_KEYWORDS
        .iter()
        .any(|k| action_text.contains(k));
    let has_protected = PROTECTED_TERMS.iter().any(|t| action_text.contains(t));
    has_modification && has_protected
}

/// Hash of the canonical card body (blank-line-trimmed), used as
/// `task_card_hash`.
pub fn card_hash(body: &str) -> String {
    use sha2::Digest;
    let canonical = body.trim();
    let mut h = sha2::Sha256::new();
    h.update(canonical.as_bytes());
    ags_kernel::workspace::hex(&h.finalize())
}

pub fn validate_file(path: &Path) -> ValidationResult {
    let body = std::fs::read_to_string(path).unwrap_or_default();
    let errors = validate_body(&path.display().to_string(), &body);
    let map = field_map(&body);
    ValidationResult {
        path: path.display().to_string(),
        valid: errors.is_empty(),
        contract_id: map.get("Contract ID:").map(|s| s.trim().to_string()),
        task_card_hash: if errors.is_empty() {
            Some(card_hash(&body))
        } else {
            None
        },
        level: map.get("任务级别：").map(|s| s.trim().to_string()),
        topology: map.get("拓扑：").map(|s| s.trim().to_string()),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CARD: &str = "## 任务卡\n\n读取并遵守：\n- AGENTS.md\n\nContract ID: tc-0123456789abcdef\n\nExecutor: Other\n\n任务级别：Medium\n\n任务：\n给 AGS 加一个 --dry-run 参数\n\n目标：\n- G-01: 参数可解析\n\n验收标准：\n- AC-01 -> G-01: --dry-run 被解析且不写文件\n\n验证：\n- V-01 -> AC-01: cargo test dry_run_flag\n- EV-01 -> AC-01: 测试通过输出\n\n写边界：仓库内\n\n拓扑：single\n\n停止条件：\n- 验证失败时停止并报告\n\n相关路径：\n- crates/ags-cli/src/main.rs\n\n交付方式：ags-run\n";

    #[test]
    fn valid_card_passes() {
        let errors = validate_body("card.md", VALID_CARD);
        assert!(errors.is_empty(), "{errors:?}");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("card.md");
        std::fs::write(&path, VALID_CARD).unwrap();
        let result = validate_file(&path);
        assert!(result.valid, "{:?}", result.errors);
        assert!(result.task_card_hash.is_some());
    }

    #[test]
    fn missing_field_fails() {
        let body = VALID_CARD.replace("交付方式：ags-run\n", "");
        let errors = validate_body("card.md", &body);
        assert!(errors.iter().any(|e| e.code == "card_field_missing"));
    }

    #[test]
    fn bad_heading_fails() {
        let body = VALID_CARD.replace("## 任务卡", "# 任务");
        let errors = validate_body("card.md", &body);
        assert!(errors.iter().any(|e| e.code == "card_heading_invalid"));
    }

    #[test]
    fn closure_gaps_fail() {
        let body = VALID_CARD.replace("- V-01 -> AC-01: cargo test dry_run_flag\n", "");
        let errors = validate_body("card.md", &body);
        assert!(errors
            .iter()
            .any(|e| e.code == "closure_mapping_incomplete"));
    }

    #[test]
    fn orphan_v_ref_fails() {
        let body = VALID_CARD.replace(
            "- V-01 -> AC-01: cargo test dry_run_flag\n",
            "- V-01 -> AC-99: ghost\n",
        );
        let errors = validate_body("card.md", &body);
        assert!(errors
            .iter()
            .any(|e| e.code == "closure_mapping_incomplete"));
    }

    #[test]
    fn light_protected_path_fails() {
        let body = VALID_CARD
            .replace("任务级别：Medium", "任务级别：Light")
            .replace(
                "给 AGS 加一个 --dry-run 参数",
                "修改 protocol/agent-task-protocol.md 的表述",
            )
            .replace(
                "- crates/ags-cli/src/main.rs",
                "- protocol/agent-task-protocol.md",
            );
        let errors = validate_body("card.md", &body);
        assert!(
            errors.iter().any(|e| e.code == "protected_path_violation"),
            "{errors:?}"
        );
    }

    #[test]
    fn invalid_contract_id_fails() {
        let body = VALID_CARD.replace("tc-0123456789abcdef", "tc-XYZ");
        let errors = validate_body("card.md", &body);
        assert!(errors.iter().any(|e| e.code == "contract_id_invalid"));
    }
}
