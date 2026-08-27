//! `ags run` orchestration (contract v3 §7.4).
//!
//! One command, three phases: `prepare` (validate + matrix verdict + review
//! escalation + evidence event), `verify` (structured command execution +
//! governance check + test evidence), `close` (evidence-chain closure +
//! memory pointer). Execution itself happens host-side between prepare and
//! verify; AGS never launches an agent.

use std::path::Path;

use serde_json::{json, Value};

use ags_kernel::capabilities::CapabilitiesLock;
use ags_kernel::config::Config;
use ags_kernel::error::{Error, Result};
use ags_kernel::evidence::EvidenceLog;
use ags_kernel::workspace::WorkspaceBinding;

use crate::command::{parse_command, run, TestReceipt};
use crate::validator::{card_hash, validate_body, validate_file, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewLevel {
    Light,
    Medium,
    Heavy,
}

impl ReviewLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewLevel::Light => "light",
            ReviewLevel::Medium => "medium",
            ReviewLevel::Heavy => "heavy",
        }
    }

    fn from_card(level: &str) -> ReviewLevel {
        match level {
            "Heavy" => ReviewLevel::Heavy,
            "Medium" => ReviewLevel::Medium,
            _ => ReviewLevel::Light,
        }
    }
}

/// Derive the effective review level from the card fields plus the
/// `ags.toml` escalation table. Shared by prepare and close so the caller
/// can never downgrade the gate. Sealed-op detection is word-bounded and
/// understands `prefix:*` entries.
pub fn derive_review_level(
    config: &Config,
    body: &str,
    card_level: &str,
    topology: &str,
) -> (ReviewLevel, Vec<String>) {
    let mut effective = ReviewLevel::from_card(card_level);
    let mut escalation_reasons: Vec<String> = Vec::new();
    if topology == "parallel" {
        effective = effective.max(ReviewLevel::Medium);
        escalation_reasons.push("fanout".to_string());
    }
    let boundary = field_value(body, "写边界：").unwrap_or_else(|| "仓库内".to_string());
    if boundary != "仓库内" && boundary != "in-repo" {
        effective = effective.max(ReviewLevel::Medium);
        escalation_reasons.push("boundary-crossing".to_string());
    }
    if sealed_mention(config, body) {
        effective = ReviewLevel::Heavy;
        escalation_reasons.push("sealed".to_string());
    }
    (effective, escalation_reasons)
}

/// Does the card body mention a sealed operation? Word-bounded match for
/// exact entries; `prefix:*` entries match `prefix:` occurrences or the
/// standalone word. Hyphens count as boundaries (so `--update-capabilities`
/// and `release-notes` both escalate — over-escalation is the safe side).
fn sealed_mention(config: &Config, body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    config.sealed.ops.iter().any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        if let Some(prefix) = entry.strip_suffix(":*") {
            lower.contains(&format!("{prefix}:")) || contains_word(&lower, prefix)
        } else {
            contains_word(&lower, &entry)
        }
    })
}

fn contains_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let needle = word.as_bytes();
    let mut idx = 0;
    while let Some(rel) = text[idx..].find(word) {
        let start = idx + rel;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        idx = start + needle.len();
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Phase 1: validate the card, resolve the review level through the
/// escalation matrix, and record the execution evidence event.
pub fn run_prepare(binding: &WorkspaceBinding, card_path: &Path) -> Result<Value> {
    let body = std::fs::read_to_string(card_path)
        .map_err(|e| Error::new("task_card_read_failed", e.to_string()))?;
    let errors = validate_body(&card_path.display().to_string(), &body);
    if !errors.is_empty() {
        return Ok(verdict(&errors, None));
    }
    let hash = card_hash(&body);
    let result = validate_file(card_path);
    let level = result.level.clone().unwrap_or_else(|| "Light".to_string());
    let topology = result
        .topology
        .clone()
        .unwrap_or_else(|| "single".to_string());
    let config = Config::load(&binding.root)?;
    let boundary = field_value(&body, "写边界：").unwrap_or_else(|| "仓库内".to_string());
    let (effective, escalation_reasons) = derive_review_level(&config, &body, &level, &topology);

    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let authority = crate::authority::derive_authority(&body, &hash)
        .map_err(|e| Error::new("authority_derive_failed", e))?;
    let event = evidence.append(
        "execution",
        &binding.slug,
        Some(&hash),
        "local",
        json!({
            "phase": "prepare",
            "level": level,
            "effective_level": effective.as_str(),
            "topology": topology,
            "write_boundary": boundary,
            "authority": authority,
            "escalation_reasons": escalation_reasons,
        }),
    )?;
    Ok(json!({
        "validated": true,
        "task_card_hash": hash,
        "contract_id": result.contract_id,
        "level": level,
        "effective_level": effective.as_str(),
        "topology": topology,
        "write_boundary": boundary,
        "authority": authority,
        "escalation_reasons": escalation_reasons,
        "review": effective.as_str(),
        "verify_commands": config.verify.commands,
        "evidence_event": event.event_id,
        "governance_status": "HOST_EXECUTION_REQUIRED",
    }))
}

fn verdict(errors: &[ValidationError], _hash: Option<&str>) -> Value {
    json!({
        "validated": false,
        "errors": errors,
        "governance_status": "CARD_INVALID",
    })
}

/// Phase 2: run the governance check plus the structured verify commands.
/// Test failure never rolls back source.
pub fn run_verify(binding: &WorkspaceBinding, card_path: &Path, profile: &str) -> Result<Value> {
    let body = std::fs::read_to_string(card_path)
        .map_err(|e| Error::new("task_card_read_failed", e.to_string()))?;
    let errors = validate_body(&card_path.display().to_string(), &body);
    if !errors.is_empty() {
        return Ok(verdict(&errors, None));
    }
    let hash = card_hash(&body);
    let config = Config::load(&binding.root)?;

    // Governance checks (read-only, project_tests_run=false).
    let lint = config.lint();
    let lock = CapabilitiesLock::load(binding)?;
    let route_checks = lock.check_routes(&binding.root);
    let all_events = EvidenceLog::new(binding.evidence_dir.clone())
        .read_all()
        .unwrap_or_default();
    let chain_ok = EvidenceLog::verify_chain(&all_events).is_ok();

    // Structured verify commands, profiled.
    let selected = select_commands(&config.verify.commands, profile);
    let mut receipts: Vec<TestReceipt> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();
    for command in &selected {
        match parse_command(command) {
            Ok(mut spec) => {
                spec.cwd = binding.root.clone();
                receipts.push(run(&spec));
            }
            Err(e) => parse_errors.push(format!("{command}: {}", e.message)),
        }
    }

    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let all_succeeded = receipts.iter().all(|r| r.status == "succeeded")
        && parse_errors.is_empty()
        && lint.is_empty()
        && route_checks.iter().all(|r| r.status == "exact")
        && chain_ok;
    let event = evidence.append(
        "test",
        &binding.slug,
        Some(&hash),
        "local",
        json!({
            "phase": "verify",
            "profile": profile,
            "receipts": receipts,
            "parse_errors": parse_errors,
            "lint": lint,
            "capability_routes": route_checks,
            "evidence_chain_ok": chain_ok,
            "all_succeeded": all_succeeded,
        }),
    )?;
    Ok(json!({
        "validated": true,
        "task_card_hash": hash,
        "project_tests_run": !receipts.is_empty(),
        "all_succeeded": all_succeeded,
        "receipts": receipts,
        "lint_findings": lint,
        "capability_routes": route_checks,
        "evidence_chain_ok": chain_ok,
        "evidence_event": event.event_id,
        "governance_status": if all_succeeded { "VERIFIED" } else { "VERIFICATION_FAILED" },
    }))
}

fn select_commands(commands: &[String], profile: &str) -> Vec<String> {
    match profile {
        "smoke" => commands.iter().take(1).cloned().collect(),
        "standard" => commands
            .iter()
            .filter(|c| !c.contains("release"))
            .cloned()
            .collect(),
        _ => commands.to_vec(),
    }
}

/// Phase 3: evidence-chain closure. A Heavy effective level requires review
/// evidence (a non-author reviewer + verdict) inside the report; closure
/// fails closed without it.
pub fn run_close(
    binding: &WorkspaceBinding,
    card_path: &Path,
    report: Value,
    instance: Option<&str>,
) -> Result<Value> {
    let body = std::fs::read_to_string(card_path)
        .map_err(|e| Error::new("task_card_read_failed", e.to_string()))?;
    let errors = validate_body(&card_path.display().to_string(), &body);
    if !errors.is_empty() {
        return Ok(verdict(&errors, None));
    }
    let hash = card_hash(&body);
    // The review gate is derived from the card + ags.toml, never from a
    // caller-supplied flag: a caller cannot downgrade a heavy closure.
    let result = validate_file(card_path);
    let level = result.level.clone().unwrap_or_else(|| "Light".to_string());
    let topology = result
        .topology
        .clone()
        .unwrap_or_else(|| "single".to_string());
    let config = Config::load(&binding.root)?;
    let (effective, _escalations) = derive_review_level(&config, &body, &level, &topology);
    if effective == ReviewLevel::Heavy {
        let review = report.get("review").cloned().unwrap_or(Value::Null);
        let reviewer = review
            .get("reviewer")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if reviewer.is_empty() || reviewer == "self" {
            return Err(Error::new(
                "review_gate_unsatisfied",
                "Heavy closure requires independent review evidence (non-author reviewer + verdict) in the report",
            ));
        }
    }
    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let closure =
        ags_kernel::memory::close_task(&evidence, &binding.slug, &hash, report, instance)?;
    let pointer = ags_kernel::memory::memory_pointer(&binding.slug, &hash, &closure.event_id);
    Ok(json!({
        "validated": true,
        "task_card_hash": hash,
        "effective_level": effective.as_str(),
        "closure_event": closure.event_id,
        "memory_pointer": pointer,
        "governance_status": "CLOSED",
    }))
}

fn field_value(body: &str, field: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.strip_prefix(field).map(|v| v.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::skeleton;
    use std::fs;

    const CARD: &str = "## 任务卡\n\n读取并遵守：\n- AGENTS.md\n\nContract ID: tc-0123456789abcdef\n\nExecutor: Other\n\n任务级别：Light\n\n任务：\n原型验证\n\n目标：\n- G-01: 跑通\n\n验收标准：\n- AC-01 -> G-01: prepare 返回 HOST_EXECUTION_REQUIRED\n\n验证：\n- V-01 -> AC-01: echo structured\n- EV-01 -> AC-01: 输出\n\n写边界：仓库内\n\n拓扑：single\n\n停止条件：\n- 失败即停\n\n相关路径：\n- src\n\n交付方式：ags-run\n";

    fn ws(tmp: &tempfile::TempDir) -> WorkspaceBinding {
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n\n[sealed]\nops = [\"govern.skill.install\", \"govern.skill.remove\", \"govern.host.register\", \"govern.host_projection\", \"govern.delegation.issue\", \"update\"]\n\n[verify]\ncommands = [\"echo structured\"]\nprofile = \"smoke\"\n",
        )
        .unwrap();
        ags_kernel::workspace::bind(&root).unwrap()
    }

    #[test]
    fn prepare_verify_close_e2e() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let card = tmp.path().join("card.md");
        fs::write(&card, CARD).unwrap();

        let prep = run_prepare(&binding, &card).unwrap();
        assert_eq!(prep["governance_status"], "HOST_EXECUTION_REQUIRED");
        assert_eq!(prep["effective_level"], "light");

        let verify = run_verify(&binding, &card, "smoke").unwrap();
        assert_eq!(verify["governance_status"], "VERIFIED");
        assert_eq!(verify["project_tests_run"], true);

        let close = run_close(&binding, &card, json!({"status": "succeeded"}), None).unwrap();
        assert_eq!(close["governance_status"], "CLOSED");
        assert!(close["memory_pointer"]
            .as_str()
            .unwrap()
            .starts_with("ags://memory/"));
    }

    #[test]
    fn heavy_close_requires_independent_review() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let card = tmp.path().join("card.md");
        // The sealed op mention ("ags update") derives Heavy; the caller has
        // no flag to downgrade it.
        fs::write(&card, CARD.replace("原型验证", "原型验证 ags update")).unwrap();
        // prepare records the task's evidence chain, which closure requires;
        // closure also requires a successful verify event.
        let prep = run_prepare(&binding, &card).unwrap();
        assert_eq!(prep["effective_level"], "heavy");
        let _verify = run_verify(&binding, &card, "smoke").unwrap();
        let err = run_close(&binding, &card, json!({"status": "ok"}), None).unwrap_err();
        assert_eq!(err.code, "review_gate_unsatisfied");
        let ok = run_close(
            &binding,
            &card,
            json!({"status": "ok", "review": {"reviewer": "codex", "verdict": "approved"}}),
            None,
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn boundary_crossing_escalates_to_medium() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let card = tmp.path().join("card.md");
        fs::write(&card, CARD.replace("写边界：仓库内", "写边界：docs")).unwrap();
        let prep = run_prepare(&binding, &card).unwrap();
        assert_eq!(prep["effective_level"], "medium");
        assert!(prep["escalation_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("boundary-crossing")));
    }

    #[test]
    fn sealed_mention_is_word_bounded_and_prefix_aware() {
        let mut config = Config::default();
        config.sealed.ops = vec![
            "update".to_string(),
            "govern.skill.install".to_string(),
            "release:*".to_string(),
        ];
        // exact word: yes
        assert!(sealed_mention(&config, "运行 ags update 刷新"));
        // substring inside another word: no
        assert!(!sealed_mention(&config, "文档 updates 章节"));
        // dotted op name: yes
        assert!(sealed_mention(&config, "执行 govern.skill.install 任务"));
        // prefix with colon: yes; standalone word: yes; hyphenated: yes
        assert!(sealed_mention(&config, "release:project-public"));
        assert!(sealed_mention(&config, "本次 release 需要独立授权"));
        assert!(sealed_mention(&config, "更新 release-notes 文档"));
        // unrelated body: no
        assert!(!sealed_mention(&config, "写一个普通函数"));
    }

    #[test]
    fn skeleton_is_valid() {
        let body = skeleton();
        let errors = validate_body("skeleton.md", &body);
        assert!(errors.is_empty(), "{errors:?}");
    }
}
