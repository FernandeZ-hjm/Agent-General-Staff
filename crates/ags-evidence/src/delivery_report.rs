//! Machine-checkable closure between a canonical AGS task card and its
//! delivery report.

use serde::Serialize;
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: &str = "0.3.4-delivery-closure";

#[derive(Debug, Clone, Serialize)]
pub struct ClosureCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryClosureResult {
    pub schema_version: String,
    pub valid: bool,
    pub contract_id: String,
    pub task_card_hash: String,
    pub delivery_report_hash: String,
    pub receipt_id: String,
    pub task_status: String,
    pub review_gate: String,
    pub checks: Vec<ClosureCheck>,
}

#[derive(Debug, Clone)]
struct ClosureRow {
    id: String,
    status: String,
    detail: String,
}

pub fn validate(task_card: &str, report: &str) -> DeliveryClosureResult {
    let task_card_hash = crate::sha256_hex(task_card.as_bytes());
    let delivery_report_hash = crate::sha256_hex(report.as_bytes());
    let receipt_id = format!("receipt-{}", &task_card_hash[..12]);
    let mut checks = Vec::new();

    let card = match ags_task_contract::validator::parse_validated(task_card) {
        Ok(card) => card,
        Err(errors) => {
            checks.push(ClosureCheck {
                name: "task-card-valid".to_string(),
                passed: false,
                detail: errors.join("; "),
            });
            return finish(
                checks,
                String::new(),
                task_card_hash,
                delivery_report_hash,
                receipt_id,
                String::new(),
                String::new(),
            );
        }
    };
    checks.push(pass(
        "task-card-valid",
        "canonical task card passed validation",
    ));
    let contract = ags_task_contract::validator::closure_contract(&card);

    let schema = inline_value(report, "Closure schema:");
    checks.push(equals_check(
        "closure-schema",
        &schema,
        "1.0",
        "delivery report closure schema",
    ));
    let report_contract_id = inline_value(report, "Contract ID:");
    checks.push(equals_check(
        "contract-id-binding",
        &report_contract_id,
        &contract.contract_id,
        "delivery report Contract ID",
    ));
    let report_task_hash = inline_value(report, "task-card-hash:");
    checks.push(equals_check(
        "task-card-hash-binding",
        &report_task_hash,
        &task_card_hash,
        "delivery report task-card-hash",
    ));
    let report_receipt_id = inline_value(report, "receipt-id:");
    checks.push(equals_check(
        "receipt-id-binding",
        &report_receipt_id,
        &receipt_id,
        "delivery report receipt-id",
    ));

    let task_status = inline_value(report, "状态:");
    let review_gate = inline_value(report, "review-gate:");
    checks.push(member_check(
        "task-status",
        &task_status,
        &["completed", "partial", "blocked"],
    ));
    checks.push(member_check(
        "review-gate",
        &review_gate,
        &["pending-review", "passed", "n/a"],
    ));

    let goal_rows = section_rows(report, "## 目标闭环", "G-");
    let criterion_rows = section_rows(report, "## 验收闭环", "AC-");
    let verification_rows = section_rows(report, "## 验证闭环", "V-");
    checks.push(rows_check(
        "goal-closure",
        &contract.goal_ids,
        &goal_rows,
        &["done", "partial", "skipped"],
    ));
    checks.push(rows_check(
        "acceptance-closure",
        &contract.acceptance_criteria_ids,
        &criterion_rows,
        &["pass", "fail", "not-run"],
    ));
    checks.push(rows_check(
        "verification-closure",
        &contract.verification_ids,
        &verification_rows,
        &["pass", "fail", "not-run"],
    ));

    let unresolved = section_body(report, "## 未闭环项");
    let unresolved_none = unresolved
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .any(|line| line == "none");
    if task_status == "completed" {
        let all_goals_done = goal_rows.iter().all(|row| row.status == "done");
        let all_criteria_pass = criterion_rows.iter().all(|row| row.status == "pass");
        let all_verifications_pass = verification_rows.iter().all(|row| row.status == "pass");
        let review_closed = matches!(review_gate.as_str(), "passed" | "n/a");
        checks.push(ClosureCheck {
            name: "completed-state-consistency".to_string(),
            passed: all_goals_done
                && all_criteria_pass
                && all_verifications_pass
                && review_closed
                && unresolved_none,
            detail: format!(
                "goals_done={all_goals_done}, criteria_pass={all_criteria_pass}, \
                 verifications_pass={all_verifications_pass}, review_closed={review_closed}, \
                 unresolved_none={unresolved_none}"
            ),
        });
    } else {
        let mut expected_unresolved = goal_rows
            .iter()
            .filter(|row| row.status != "done")
            .map(|row| row.id.clone())
            .chain(
                criterion_rows
                    .iter()
                    .filter(|row| row.status != "pass")
                    .map(|row| row.id.clone()),
            )
            .chain(
                verification_rows
                    .iter()
                    .filter(|row| row.status != "pass")
                    .map(|row| row.id.clone()),
            )
            .collect::<BTreeSet<_>>();
        if review_gate == "pending-review" {
            expected_unresolved.insert("review-gate".to_string());
        }
        let parsed_unresolved = unresolved_ids(&unresolved);
        let (passed, detail) = match parsed_unresolved {
            Ok(actual) => (
                !expected_unresolved.is_empty() && actual == expected_unresolved,
                format!(
                    "expected={expected_unresolved:?}, actual={actual:?}; partial/blocked requires exact unresolved ID equality"
                ),
            ),
            Err(error) => (false, error),
        };
        checks.push(ClosureCheck {
            name: "open-state-consistency".to_string(),
            passed,
            detail,
        });
    }

    finish(
        checks,
        contract.contract_id,
        task_card_hash,
        delivery_report_hash,
        receipt_id,
        task_status,
        review_gate,
    )
}

fn finish(
    checks: Vec<ClosureCheck>,
    contract_id: String,
    task_card_hash: String,
    delivery_report_hash: String,
    receipt_id: String,
    task_status: String,
    review_gate: String,
) -> DeliveryClosureResult {
    DeliveryClosureResult {
        schema_version: SCHEMA_VERSION.to_string(),
        valid: checks.iter().all(|check| check.passed),
        contract_id,
        task_card_hash,
        delivery_report_hash,
        receipt_id,
        task_status,
        review_gate,
        checks,
    }
}

fn pass(name: &str, detail: &str) -> ClosureCheck {
    ClosureCheck {
        name: name.to_string(),
        passed: true,
        detail: detail.to_string(),
    }
}

fn equals_check(name: &str, actual: &str, expected: &str, label: &str) -> ClosureCheck {
    ClosureCheck {
        name: name.to_string(),
        passed: actual == expected,
        detail: format!("{label}: expected `{expected}`, actual `{actual}`"),
    }
}

fn member_check(name: &str, actual: &str, allowed: &[&str]) -> ClosureCheck {
    ClosureCheck {
        name: name.to_string(),
        passed: allowed.contains(&actual),
        detail: format!("actual `{actual}`, allowed {}", allowed.join(", ")),
    }
}

fn rows_check(
    name: &str,
    expected_ids: &[String],
    rows: &[ClosureRow],
    allowed_statuses: &[&str],
) -> ClosureCheck {
    let expected = expected_ids.iter().cloned().collect::<BTreeSet<_>>();
    let actual = rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<BTreeSet<_>>();
    let no_duplicates = actual.len() == rows.len();
    let statuses_valid = rows
        .iter()
        .all(|row| allowed_statuses.contains(&row.status.as_str()));
    let evidence_present = rows.iter().all(|row| !row.detail.trim().is_empty());
    ClosureCheck {
        name: name.to_string(),
        passed: expected == actual && no_duplicates && statuses_valid && evidence_present,
        detail: format!(
            "expected={expected:?}, actual={actual:?}, no_duplicates={no_duplicates}, \
             statuses_valid={statuses_valid}, evidence_present={evidence_present}"
        ),
    }
}

fn unresolved_ids(body: &str) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    let mut row_count = 0usize;
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let trimmed = line.trim();
        let row = trimmed.strip_prefix('-').map(str::trim).ok_or_else(|| {
            format!("invalid unresolved row `{trimmed}`: expected `- ID: reason`")
        })?;
        let (id, reason) = row
            .split_once(':')
            .ok_or_else(|| format!("invalid unresolved row `{row}`: missing `: reason`"))?;
        let id = id.trim();
        let valid_id = valid_indexed_id(id, "G-")
            || valid_indexed_id(id, "AC-")
            || valid_indexed_id(id, "V-")
            || id == "review-gate";
        if !valid_id {
            return Err(format!("invalid unresolved ID `{id}`"));
        }
        if reason.trim().is_empty() {
            return Err(format!("unresolved ID `{id}` is missing a reason"));
        }
        row_count += 1;
        if !ids.insert(id.to_string()) {
            return Err(format!("duplicate unresolved ID `{id}`"));
        }
    }
    if row_count == 0 {
        return Err("partial/blocked report has no unresolved rows".to_string());
    }
    Ok(ids)
}

fn inline_value(input: &str, key: &str) -> String {
    input
        .lines()
        .find_map(|line| line.trim().strip_prefix(key).map(str::trim))
        .unwrap_or("")
        .to_string()
}

fn section_body(input: &str, header: &str) -> String {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with("## ") {
            break;
        }
        if in_section {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_string()
}

fn section_rows(input: &str, header: &str, prefix: &str) -> Vec<ClosureRow> {
    section_body(input, header)
        .lines()
        .filter_map(|line| {
            let body = line.trim().trim_start_matches('-').trim();
            let (id, rest) = body.split_once(':')?;
            let id = id.trim();
            if !valid_indexed_id(id, prefix) {
                return None;
            }
            let (status, detail) = rest
                .trim()
                .split_once('—')
                .or_else(|| rest.trim().split_once('-'))?;
            Some(ClosureRow {
                id: id.to_string(),
                status: status.trim().to_string(),
                detail: detail.trim().to_string(),
            })
        })
        .collect()
}

fn valid_indexed_id(value: &str, prefix: &str) -> bool {
    let Some(number) = value.strip_prefix(prefix) else {
        return false;
    };
    number.len() == 2 && number.chars().all(|character| character.is_ascii_digit())
}

pub fn render_text(result: &DeliveryClosureResult) -> String {
    let mut lines = vec![
        "AGS Delivery Closure".to_string(),
        format!("valid: {}", result.valid),
        format!("contract_id: {}", result.contract_id),
        format!("task_card_hash: {}", result.task_card_hash),
        format!("delivery_report_hash: {}", result.delivery_report_hash),
        format!("receipt_id: {}", result.receipt_id),
    ];
    for check in &result.checks {
        lines.push(format!(
            "- {}: {} — {}",
            check.name,
            if check.passed { "pass" } else { "fail" },
            check.detail
        ));
    }
    lines.join("\n")
}

pub fn render_json(result: &DeliveryClosureResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_card() -> String {
        "## 任务卡\n\
读取并遵守：\n- 本任务卡\n\
Contract ID: tc-0123456789abcdef\n\
Handoff source: host-plan-mode\n\
Executor: Claude Code\n\
Runtime adapter: claude-code\n\
Execution surface: cli\n\
Permission mode: execute-and-verify\n\
Parallelism: none\n\
Execution effort: normal\n\
Workflow authority: none\n\
任务级别：Medium\n\
Review gate:\n- 按协议执行\n\
任务：实现并验证闭环\n\
背景：测试闭环\n\
项目画像：无\n\
记忆胶囊：无\n\
任务存档：无\n\
目标文件夹路径：\n- .\n\
相关路径：\n- .\n\
本次任务相关文件：\n- .\n\
目标：\n- G-01: 完成闭环\n\
验收标准：\n- AC-01 -> G-01: 闭环校验通过\n\
非目标：\n- 不发布\n\
验证：\n- 运行闭环校验\n\
Verification gate:\n- commands:\n  - V-01 -> AC-01: cargo test\n- expected evidence:\n  - EV-01 -> AC-01: test pass\n- stop condition:\n  - 失败时停止\n\
交付：\n- 输出交付报告\n"
            .to_string()
    }

    fn report(card: &str) -> String {
        let hash = crate::sha256_hex(card.as_bytes());
        format!(
            "# 任务交付报告\n\
Closure schema: 1.0\n\
Contract ID: tc-0123456789abcdef\n\
task-card-hash: {hash}\n\
receipt-id: receipt-{}\n\
状态: completed\n\
review-gate: passed\n\
## 目标闭环\n- G-01: done — 已完成\n\
## 验收闭环\n- AC-01: pass — evidence: closure validator passed\n\
## 验证闭环\n- V-01: pass — cargo test exit 0\n\
## 改动与边界\n- changed: test\n\
## 未闭环项\n- none\n",
            &hash[..12]
        )
    }

    #[test]
    fn validates_complete_report() {
        let card = task_card();
        let result = validate(&card, &report(&card));
        assert!(result.valid, "{:#?}", result.checks);
    }

    #[test]
    fn rejects_missing_acceptance_row() {
        let card = task_card();
        let bad = report(&card).replace("- AC-01: pass — evidence: closure validator passed\n", "");
        let result = validate(&card, &bad);
        assert!(!result.valid);
        assert!(result
            .checks
            .iter()
            .any(|check| check.name == "acceptance-closure" && !check.passed));
    }

    #[test]
    fn completed_cannot_hide_failed_verification() {
        let card = task_card();
        let bad = report(&card).replace("V-01: pass", "V-01: fail");
        let result = validate(&card, &bad);
        assert!(!result.valid);
        assert!(result
            .checks
            .iter()
            .any(|check| check.name == "completed-state-consistency" && !check.passed));
    }

    #[test]
    fn partial_rejects_arbitrary_unresolved_text() {
        let card = task_card();
        let bad = report(&card)
            .replace("状态: completed", "状态: partial")
            .replace("V-01: pass", "V-01: fail")
            .replace("- none", "- something remains");
        let result = validate(&card, &bad);
        assert!(!result.valid);
        assert!(result
            .checks
            .iter()
            .any(|check| check.name == "open-state-consistency" && !check.passed));
    }

    #[test]
    fn partial_requires_exact_unresolved_id_set() {
        let card = task_card();
        let bad = report(&card)
            .replace("状态: completed", "状态: partial")
            .replace("AC-01: pass", "AC-01: fail")
            .replace("V-01: pass", "V-01: fail")
            .replace("- none", "- V-01: command failed");
        let result = validate(&card, &bad);
        assert!(!result.valid);
        assert!(result.checks.iter().any(|check| {
            check.name == "open-state-consistency"
                && !check.passed
                && check.detail.contains("AC-01")
        }));
    }

    #[test]
    fn partial_accepts_exact_unresolved_id_set() {
        let card = task_card();
        let partial = report(&card)
            .replace("状态: completed", "状态: partial")
            .replace("AC-01: pass", "AC-01: fail")
            .replace("V-01: pass", "V-01: fail")
            .replace(
                "- none",
                "- AC-01: acceptance evidence failed\n- V-01: command failed",
            );
        let result = validate(&card, &partial);
        assert!(result.valid, "{:#?}", result.checks);
    }
}
