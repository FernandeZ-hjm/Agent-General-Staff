//! Compiler regression tests preserved from the former crate-root monolith.

use super::*;

// Most compiler tests exercise rendering and validation after a governed
// handoff. Keep that premise explicit while public legacy callers remain
// fail-closed by default.
fn with_test_closure_contract(intent: &str) -> String {
    if intent.trim_start().starts_with('{') {
        return intent.to_string();
    }
    let mut enriched = intent.to_string();
    if let Some(goal_start) = enriched.find("目标：") {
        let value_start = goal_start + "目标：".len();
        let value_end = FIELD_HEADERS
            .iter()
            .filter_map(|(header, _)| {
                enriched[value_start..]
                    .find(&format!("\n{header}"))
                    .map(|position| value_start + position)
            })
            .min()
            .unwrap_or(enriched.len());
        let original = enriched[value_start..value_end]
            .trim()
            .trim_start_matches('-')
            .trim_start_matches(|character: char| {
                character.is_ascii_digit() || character == '.' || character.is_ascii_whitespace()
            })
            .trim()
            .replace('\n', " ");
        if !original.starts_with("G-") {
            enriched.replace_range(value_start..value_end, &format!("\n- G-01: {original}"));
        }
        if !enriched.contains("验收标准：") {
            enriched.push_str("\n验收标准：\n- AC-01 -> G-01: 测试结果达到预期\n");
        }
        if !enriched.contains("Verification gate:") {
            enriched.push_str(
                "\nVerification gate:\n- commands:\n  - V-01 -> AC-01: cargo test\n\
- expected evidence:\n  - EV-01 -> AC-01: test pass\n\
- stop condition:\n  - 失败时停止\n",
            );
        }
    }
    enriched
}

fn compile(
    intent: &str,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
) -> (String, CompileReport) {
    super::compile_with_contract(
        &with_test_closure_contract(intent),
        project_root,
        check_only,
        task_card_requested,
        true,
    )
}

fn compile_simple(
    intent: &str,
    project_root: &Path,
    task_card_requested: bool,
) -> Result<String, Box<CompileReport>> {
    super::compile_simple_with_contract(
        &with_test_closure_contract(intent),
        project_root,
        task_card_requested,
        true,
    )
    .map_err(Box::new)
}

#[test]
fn test_parse_intent_simple_inline() {
    let input = "Executor: Claude Code\n任务级别：Medium\n任务：测试编译";
    let parsed = parse_intent(input);
    assert_eq!(parsed.fields.get("Executor:").unwrap(), "Claude Code");
    assert_eq!(parsed.fields.get("任务级别：").unwrap(), "Medium");
    assert_eq!(parsed.fields.get("任务：").unwrap(), "测试编译");
}

#[test]
fn test_parse_intent_multiline() {
    let input = "目标：\n1. goal_1\n2. goal_2\n\n非目标：\n- non_goal";
    let parsed = parse_intent(input);
    assert_eq!(parsed.fields.get("目标：").unwrap(), "1. goal_1\n2. goal_2");
    assert_eq!(parsed.fields.get("非目标：").unwrap(), "- non_goal");
}

#[test]
fn test_parse_intent_free_text() {
    let input = "这是一段自由文本\n描述任务内容\n\n任务级别：Medium";
    let parsed = parse_intent(input);
    assert_eq!(parsed.free_text, "这是一段自由文本\n描述任务内容");
    assert_eq!(parsed.fields.get("任务级别：").unwrap(), "Medium");
}

#[test]
fn test_parse_intent_mixed() {
    let input =
        "Executor: Claude Code\n\nimplement a feature\n\n目标：\n1. do something\n2. verify";
    let parsed = parse_intent(input);
    assert_eq!(parsed.fields.get("Executor:").unwrap(), "Claude Code");
    assert_eq!(
        parsed.fields.get("目标：").unwrap(),
        "1. do something\n2. verify"
    );
    assert_eq!(parsed.free_text, "implement a feature");
}

#[test]
fn test_normalise_key() {
    assert_eq!(normalise_key("Executor:"), Some("Executor:"));
    assert_eq!(normalise_key("任务："), Some("任务："));
    assert_eq!(normalise_key("Executor"), Some("Executor:"));
    assert_eq!(normalise_key("任务"), Some("任务："));
    assert_eq!(normalise_key("unknown"), None);
}

#[test]
fn test_compile_minimal_intent() {
    let intent = "任务：test compilation\n目标：verify compiler works";
    let project_root = Path::new(".");
    let (card, report) = compile(intent, project_root, false, true);

    // Should have no missing slots
    assert!(
        report.missing_slots.is_empty(),
        "Missing slots: {:?}",
        report.missing_slots
    );

    // Card should start with ## 任务卡
    assert!(
        card.starts_with("## 任务卡"),
        "Card does not start with ## 任务卡:\n{}",
        card
    );

    // Should contain key fields
    assert!(card.contains("Executor:"));
    assert!(card.contains("任务："));
    assert!(card.contains("目标："));
}

#[test]
fn test_compile_missing_task_and_goal() {
    let intent = "Executor: Claude Code";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    // Should report 目标：as a default-filled slot (not missing since we default it)
    // 任务：should be missing if no free text
    let has_task = report.slot_sources.iter().any(|s| s.field == "任务：");
    eprintln!(
        "has_task: {}, missing: {:?}",
        has_task, report.missing_slots
    );
}

#[test]
fn test_compile_maps_omp_executor_to_native_adapter() {
    let intent = "Executor: OMP\n任务：run with OMP\n目标：verify native adapter";
    let project_root = Path::new(".");
    let (card, report) = compile(intent, project_root, false, true);

    assert!(report.validation_passed, "{:?}", report.validation_errors);
    assert!(card.contains("Executor: OMP"));
    assert!(card.contains("Runtime adapter: omp"));
}

#[test]
fn test_compile_includes_slot_sources() {
    let intent = "任务：test\n目标：verify";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    // Every slot in the report should have a source
    for slot in &report.slot_sources {
        assert!(!slot.field.is_empty());
        assert!(slot.source != SlotSource::Missing || report.missing_slots.contains(&slot.field));
    }
}

#[test]
fn test_render_report_json_is_valid() {
    let intent = "任务：test\n目标：verify";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    let json = render_report_json(&report);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
    assert!(parsed.is_ok(), "JSON parse error: {:?}", parsed.err());

    let v = parsed.unwrap();
    assert_eq!(v["schema_version"], SCHEMA_VERSION);
    assert!(v["slot_sources"].is_array());
}

#[test]
fn test_compile_check_only_in_report() {
    let intent = "任务：test\n目标：verify";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, true, true);
    assert!(report.check_only);
    assert!(
        report.compiled_task_card.is_empty(),
        "check-only report must not expose an executable compiled_task_card"
    );
}

#[test]
fn test_legacy_missing_task_level_defaults_medium_with_deprecation() {
    let (_card, report) = compile(
        "任务：数据迁移和向量库重建\n目标：保护 baseline",
        Path::new("."),
        false,
        true,
    );
    let level = report
        .slot_sources
        .iter()
        .find(|slot| slot.field == "任务级别：")
        .unwrap();
    assert_eq!(level.value, "Medium");
    assert!(report
        .deprecations
        .iter()
        .any(|notice| notice == "legacy_loose_contract_missing_task_level_defaulted_to_medium"));
}

#[test]
fn test_compile_report_json_schema() {
    let intent = "任务：test task\n目标：verify something";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    let json = render_report_json(&report);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Required top-level fields
    assert!(v.get("schema_version").is_some(), "missing schema_version");
    assert!(
        v.get("compiled_task_card").is_some(),
        "missing compiled_task_card"
    );
    assert!(v.get("slot_sources").is_some(), "missing slot_sources");
    assert!(v.get("missing_slots").is_some(), "missing missing_slots");
    assert!(v.get("assumptions").is_some(), "missing assumptions");
    assert!(
        v.get("validation_passed").is_some(),
        "missing validation_passed"
    );
    assert!(
        v.get("validation_errors").is_some(),
        "missing validation_errors"
    );
    assert!(v.get("check_only").is_some(), "missing check_only");
    assert!(
        v.get("task_card_requested").is_some(),
        "missing task_card_requested"
    );
    assert!(
        v.get("confirmed_handoff_contract").is_some(),
        "missing confirmed_handoff_contract"
    );
    assert!(
        v.get("executable_allowed").is_some(),
        "missing executable_allowed"
    );
}

// ── Task-card request gate tests ──────────────────────────────

#[test]
fn test_task_card_not_requested_blocks_executable_output() {
    // Without task_card_requested, executable output must be blocked.
    let intent = "任务：test gate\n目标：verify blocking";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, false);

    assert!(
        !report.task_card_requested,
        "task_card_requested must be false"
    );
    assert!(
        !report.executable_allowed,
        "executable_allowed must be false when task_card_requested is false"
    );
    assert_eq!(
        report.block_reason,
        Some("task_card_not_requested".to_string()),
        "block_reason must be task_card_not_requested"
    );
    assert!(
        report.compiled_task_card.is_empty(),
        "compiled_task_card must be empty when executable is blocked"
    );
    assert!(
        !report.validation_passed,
        "validation_passed must be false when executable is blocked"
    );
}

#[test]
fn raw_chat_is_not_a_compiler_contract() {
    let intent = "请把已确认方案整理成任务卡并交给 Claude Code";
    let project_root = Path::new(".");
    let (card, report) = super::compile_with_contract(intent, project_root, false, true, true);

    assert!(card.is_empty());
    assert!(!report.executable_allowed);
    assert_eq!(
        report.block_reason,
        Some("handoff_contract_not_structured".to_string())
    );
}

#[test]
fn malformed_typed_contract_fails_closed_without_legacy_fallback() {
    for intent in [
        r#"{"schema_version":"0.3.0-handoff-contract","task":"missing level"}"#,
        r#"{"schema_version":"0.2.8-handoff-contract","task_level":"Medium","task":"old schema"}"#,
        r#"{"schema_version":"0.3.0-handoff-contract","task_level":"Medium","task":"unknown field","surprise":true}"#,
    ] {
        let (card, report) =
            super::compile_with_contract(intent, Path::new("."), false, true, true);
        assert!(card.is_empty());
        assert_eq!(report.contract_format, "typed_invalid");
        assert!(!report.executable_allowed);
        assert_eq!(
            report.block_reason.as_deref(),
            Some("invalid_typed_handoff_contract")
        );
        assert!(!report.validation_errors.is_empty());
        assert!(report.deprecations.is_empty());
    }
}

#[test]
fn confirmed_contract_text_is_not_reclassified_by_compiler() {
    let intent = "任务：设计跨 MCP、CLI、Vault 的新架构并生成任务卡\n目标：定义新架构边界";
    let project_root = Path::new(".");
    let (card, report) = super::compile_with_contract(intent, project_root, false, true, true);

    assert!(report.confirmed_handoff_contract);
    assert_eq!(card.is_empty(), !report.executable_allowed);
}

#[test]
fn test_task_card_requested_allows_executable_output() {
    // With task_card_requested=true, executable output must be allowed.
    let intent = "任务：test gate\n目标：verify allowed";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    assert!(
        report.task_card_requested,
        "task_card_requested must be true"
    );
    assert!(
        report.executable_allowed,
        "executable_allowed must be true when task_card_requested is true"
    );
    assert!(
        report.block_reason.is_none(),
        "block_reason must be None when allowed"
    );
    assert!(
        !report.compiled_task_card.is_empty(),
        "compiled_task_card must NOT be empty when executable is allowed"
    );
}

#[test]
fn host_plan_mode_final_is_the_task_card_handoff() {
    let intent =
        with_test_closure_contract("任务：执行已封闭的 Plan 模式方案\n目标：按原计划完成实现");
    let project_root = Path::new(".");
    let (card, report) = super::compile_with_handoff_source(
        &intent,
        project_root,
        false,
        false,
        true,
        HandoffSource::HostPlanMode,
    );

    assert!(report.host_plan_mode_final);
    assert!(!report.task_card_requested);
    assert!(report.executable_allowed);
    assert_eq!(report.handoff_source, "host-plan-mode");
    assert!(card.contains("Handoff source: host-plan-mode"));
    assert!(card.contains("Contract ID: tc-"));

    let (same_card, _) = super::compile_with_handoff_source(
        &intent,
        project_root,
        false,
        false,
        true,
        HandoffSource::HostPlanMode,
    );
    assert_eq!(
        card, same_card,
        "closed Plan input must compile deterministically"
    );
}

#[test]
fn host_plan_mode_still_requires_a_confirmed_contract() {
    let intent =
        with_test_closure_contract("任务：执行仍未确认的 Plan 模式方案\n目标：不得提前生成任务卡");
    let (card, report) = super::compile_with_handoff_source(
        &intent,
        Path::new("."),
        false,
        false,
        false,
        HandoffSource::HostPlanMode,
    );

    assert!(card.is_empty());
    assert!(!report.executable_allowed);
    assert_eq!(
        report.block_reason.as_deref(),
        Some("handoff_contract_not_confirmed")
    );
}

#[test]
fn test_check_only_blocks_executable_even_when_requested() {
    // check_only takes precedence over task_card_requested.
    let intent = "任务：test check only gate\n目标：verify precedence";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, true, true);

    assert!(report.check_only, "check_only must be true");
    assert!(
        report.task_card_requested,
        "task_card_requested must be true"
    );
    assert!(
        !report.executable_allowed,
        "executable_allowed must be false in check-only mode"
    );
    assert_eq!(report.block_reason, Some("check_only".to_string()));
    assert!(report.compiled_task_card.is_empty());
}

#[test]
fn test_text_report_shows_gate_status() {
    // Text report must include task_card_requested and executable_allowed.
    let intent = "任务：gate display test\n目标：verify display";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, false);
    let text = render_report_text(&report);

    assert!(
        text.contains("Task card requested:"),
        "text report must show Task card requested status"
    );
    assert!(
        text.contains("Executable allowed:"),
        "text report must show Executable allowed status"
    );
    assert!(
        text.contains("Block reason:"),
        "text report must show Block reason"
    );
    assert!(
        !text.contains("Compiled Task Card:"),
        "text report must NOT show Compiled Task Card when blocked"
    );
}

#[test]
fn test_missing_slots_blocks_executable_even_when_requested() {
    // Even with task_card_requested=true, missing slots block executable output.
    let intent = "Executor: Claude Code\n任务：structured but incomplete contract";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);

    assert!(report.task_card_requested);
    assert!(
        !report.executable_allowed,
        "executable_allowed must be false when slots are missing"
    );
    assert_eq!(report.block_reason, Some("missing_slots".to_string()));
}

#[test]
fn test_compile_simple_errors_when_not_requested() {
    let intent = "任务：simple gate test\n目标：verify simple blocks";
    let project_root = Path::new(".");
    let result = compile_simple(intent, project_root, false);
    assert!(
        result.is_err(),
        "compile_simple must return Err when task_card_requested=false"
    );
    let err = result.unwrap_err();
    assert!(!err.executable_allowed);
    assert_eq!(
        err.block_reason,
        Some("task_card_not_requested".to_string())
    );
}

#[test]
fn test_compile_simple_succeeds_when_requested() {
    let intent = "任务：simple ok test\n目标：verify simple allows";
    let project_root = Path::new(".");
    let result = compile_simple(intent, project_root, true);
    assert!(
        result.is_ok(),
        "compile_simple must return Ok when task_card_requested=true"
    );
}

#[test]
fn test_compile_tuple_card_empty_when_gate_blocked() {
    // Regression: the public API tuple (card, report) must NOT leak an
    // executable card when the gate blocks it. Any Rust caller using
    // `let (card, _) = compile(...)` directly (bypassing the CLI) must
    // receive an empty card string when task_card_requested=false.
    let intent = "任务：tuple bypass test\n目标：verify tuple safety";
    let project_root = Path::new(".");

    // Without task_card_requested → card must be empty
    let (card, report) = compile(intent, project_root, false, false);
    assert!(
        card.is_empty(),
        "tuple card must be empty when task_card_requested=false, got {} bytes",
        card.len()
    );
    assert!(!report.executable_allowed);

    // With check_only → card must be empty
    let (card2, report2) = compile(intent, project_root, true, true);
    assert!(
        card2.is_empty(),
        "tuple card must be empty when check_only=true"
    );
    assert!(!report2.executable_allowed);

    // With missing slots → card must be empty
    let (card3, report3) = compile("Executor: Claude Code", project_root, false, true);
    assert!(
        card3.is_empty(),
        "tuple card must be empty when slots are missing"
    );
    assert!(!report3.executable_allowed);

    // With task_card_requested=true, no missing slots → card must be non-empty
    let (card4, report4) = compile(intent, project_root, false, true);
    assert!(
        !card4.is_empty(),
        "tuple card must be non-empty when allowed"
    );
    assert!(card4.starts_with("## 任务卡"));
    assert!(report4.executable_allowed);
}

// ── P1 regression: task level aliases ──────────────────────────

#[test]
fn test_parse_intent_task_level_english_colon() {
    // "Task level: Heavy" with ASCII colon — must be recognized
    let input = "Task level: Heavy\n任务：test alias parsing";
    let parsed = parse_intent(input);
    assert_eq!(
        parsed.fields.get("任务级别：").unwrap(),
        "Heavy",
        "English alias 'Task level:' with ASCII colon must map to 任务级别："
    );
}

#[test]
fn test_parse_intent_english_key_fullwidth_colon_does_not_panic() {
    let parsed = parse_intent("Task level：Heavy\n任务：fullwidth colon regression");
    assert_eq!(parsed.fields.get("任务级别：").unwrap(), "Heavy");
    assert_eq!(
        parsed.fields.get("任务：").unwrap(),
        "fullwidth colon regression"
    );
}

#[test]
fn typed_handoff_requires_explicit_task_level_and_preserves_it() {
    let json = serde_json::json!({
            "schema_version": HANDOFF_CONTRACT_SCHEMA_VERSION,
            "task_level": "Heavy",
            "task": "typed contract",
            "fields": {
                "目标：": "- G-01: verify typed path",
                "验收标准：": "- AC-01 -> G-01: typed path is preserved",
                "Verification gate:": "- commands:\n  - V-01 -> AC-01: cargo test -p task-compiler\n- expected evidence:\n  - EV-01 -> AC-01: test pass\n- stop condition:\n  - test failure"
            }
        })
        .to_string();
    let (card, report) = compile(&json, Path::new("."), false, true);
    assert!(card.contains("任务级别： Heavy"));
    assert_eq!(report.contract_format, "typed");
    assert!(report.deprecations.is_empty());

    let missing_level = serde_json::json!({
        "schema_version": HANDOFF_CONTRACT_SCHEMA_VERSION,
        "task": "missing level",
        "fields": {}
    });
    assert!(serde_json::from_value::<HandoffContract>(missing_level).is_err());
}

#[test]
fn test_parse_intent_chinese_ascii_colon() {
    // "任务级别: Heavy" with ASCII colon instead of FULLWIDTH
    let input = "任务级别: Heavy\n任务：test alias";
    let parsed = parse_intent(input);
    assert_eq!(
        parsed.fields.get("任务级别：").unwrap(),
        "Heavy",
        "Chinese key '任务级别:' with ASCII colon must map to 任务级别："
    );
}

#[test]
fn test_parse_intent_colonless_key_not_treated_as_free_text() {
    // "任务级别 Heavy" without any colon should NOT be treated as free text
    // The colonless key should be recognized by normalise_key
    let input = "任务级别 Heavy\n任务：test";
    let parsed = parse_intent(input);
    assert!(
        parsed.fields.contains_key("任务级别：") || !parsed.free_text.contains("任务级别"),
        "Colon-less 任务级别 should be recognized; got fields={:?}, free_text={:?}",
        parsed.fields.keys().collect::<Vec<_>>(),
        parsed.free_text
    );
}

#[test]
fn test_task_level_alias_heavy_is_not_downgraded_to_medium() {
    // Explicit "Task level: Heavy" must produce Heavy, not Medium
    let intent = "Task level: Heavy\n任务：critical task\n目标：verify heavy level";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let task_level_slot = report
        .slot_sources
        .iter()
        .find(|s| s.field == "任务级别：")
        .expect("task level slot must exist");
    assert_eq!(
        task_level_slot.value, "Heavy",
        "Explicit Task level: Heavy must be preserved, got '{}'",
        task_level_slot.value
    );
}

// ── P5 regression: Heavy → plan-only default ───────────────────

#[test]
fn test_heavy_task_defaults_to_plan_only_permission() {
    let intent = "任务级别：Heavy\n任务：critical change\n目标：verify safety";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let perm_slot = report
        .slot_sources
        .iter()
        .find(|s| s.field == "Permission mode:")
        .expect("permission mode slot must exist");
    assert_eq!(
        perm_slot.value, "plan-only",
        "Heavy task must default to plan-only, got '{}'",
        perm_slot.value
    );
}

#[test]
fn test_medium_task_defaults_to_direct_execution() {
    let intent = "任务级别：Medium\n任务：normal change\n目标：verify default";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let perm_slot = report
        .slot_sources
        .iter()
        .find(|s| s.field == "Permission mode:")
        .expect("permission mode slot must exist");
    assert_eq!(
        perm_slot.value, "execute-and-verify",
        "Medium task must default to direct execution, got '{}'",
        perm_slot.value
    );
}

#[test]
fn test_heavy_task_with_explicit_permission_is_preserved() {
    let intent =
        "任务级别：Heavy\nPermission mode: execute-and-verify\n任务：explicit perm\n目标：verify";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let perm_slot = report
        .slot_sources
        .iter()
        .find(|s| s.field == "Permission mode:")
        .expect("permission mode slot must exist");
    assert_eq!(
        perm_slot.value, "execute-and-verify",
        "Explicit permission mode must be preserved even for Heavy tasks, got '{}'",
        perm_slot.value
    );
    assert_eq!(
        perm_slot.source,
        SlotSource::Intent,
        "Explicit permission mode source must be Intent"
    );
}

// ── P2.2 regression: UTF-8 safe truncation ─────────────────────

#[test]
fn test_utf8_safe_truncation_does_not_panic() {
    // A long Chinese string — slicing at byte boundaries could panic
    let long_chinese =
        "任务：这是一个很长的中文任务描述用来测试UTF8截断安全性确保不会在字节边界处崩溃";
    let intent = format!("{}\n目标：verify truncation safety", long_chinese);
    let project_root = Path::new(".");
    let (_card, report) = compile(&intent, project_root, false, true);
    // render_report_text must not panic on mixed ASCII/Chinese content
    let text = render_report_text(&report);
    assert!(!text.is_empty(), "report text should be non-empty");
}

// ── P2.3 regression: check-only suppresses card output ──────────

#[test]
fn test_check_only_suppresses_compiled_card_in_text() {
    let intent = "任务：check only test\n目标：verify suppression";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, true, true);
    let text = render_report_text(&report);
    assert!(
        !text.contains("Compiled Task Card:"),
        "check-only text output must NOT contain 'Compiled Task Card:' section"
    );
    assert!(
        report.compiled_task_card.is_empty(),
        "check-only report must not include executable card text"
    );
}

#[test]
fn test_normal_mode_includes_card_in_text() {
    let intent = "任务：normal mode\n目标：verify card included";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let text = render_report_text(&report);
    assert!(
        text.contains("Compiled Task Card:"),
        "normal (non-check-only) text output must include 'Compiled Task Card:' section"
    );
}

// ── P2.1 regression: plain card output ─────────────────────────

#[test]
fn test_plain_card_output_starts_with_task_card_header() {
    let intent = "任务：pipe test\n目标：verify pipeable output";
    let project_root = Path::new(".");
    let (_card, report) = compile(intent, project_root, false, true);
    let card_text = render_card_text(&report);
    assert!(
        card_text.starts_with("## 任务卡"),
        "Plain card output must start with '## 任务卡', got: {:?}",
        &card_text[..30.min(card_text.len())]
    );
}

// ── Hard validation: compiled card passes the REAL validator ───────

#[test]
fn compiled_card_passes_real_validator() {
    // The compiled card must pass the REAL task_card_validator, not just
    // the compiler's heuristic self-check (closes the gap where compact
    // output was never validated against the canonical gate).
    let intent = "任务：测试编译器输出能通过真实校验器\n\
                      目标：验证 ags task compile 产出经典骨架并通过 validator";
    let project_root = Path::new(".");
    let (card, report) = compile(intent, project_root, false, true);

    assert!(
        report.executable_allowed,
        "card should be executable: {:?}",
        report.block_reason
    );
    assert!(
        card.starts_with("## 任务卡"),
        "card must start with ## 任务卡"
    );
    assert!(
        !card.contains("AGENT_SUITE_COMPACT_TASK_CARD_V1"),
        "compiled card must not contain the removed compact marker"
    );
    // The second non-empty line must be the classic discriminator.
    let second = card
        .lines()
        .filter(|l| !l.trim().is_empty())
        .nth(1)
        .unwrap_or("");
    assert!(
        second.starts_with("读取并遵守："),
        "second non-empty line must be 读取并遵守：, got: {}",
        second
    );

    let errors = validator::validate(&card);
    assert!(
        errors.is_empty(),
        "compiled card must pass the real validator, errors: {:?}\ncard:\n{}",
        errors,
        card
    );
    let target_folder = report
        .slot_sources
        .iter()
        .find(|slot| slot.field == "目标文件夹路径：")
        .and_then(|slot| slot.value.strip_prefix("- "))
        .expect("compiled report must contain the target folder slot");
    assert!(
        std::path::Path::new(target_folder).is_absolute(),
        "compiled card must render an absolute target folder path, got {target_folder:?}:\n{card}"
    );
}

// ── Single canonical template: level is a field, not a template file ───

#[test]
fn compiler_always_emits_single_canonical_skeleton() {
    // Light / Medium / Heavy-shaped intents must all compile to the SAME
    // canonical skeleton (protocol/task-card-template.md). Task level is a
    // `任务级别：` field value, never a different per-level template file,
    // and the compiler must never emit a compact or fallback skeleton.
    let project_root = Path::new(".");
    let intents = [
        "任务：改个变量名\n目标：把 foo 改成 bar",
        "任务：跨文件重构共享模块\n目标：调整配置与共享 helper 行为",
        "任务：数据库迁移与向量库 baseline 重建\n目标：执行不可逆迁移并保留 baseline",
    ];
    for intent in intents {
        let (card, _report) = compile(intent, project_root, false, true);
        assert!(
            card.starts_with("## 任务卡"),
            "must start with ## 任务卡 for intent {intent:?}"
        );
        let second = card
            .lines()
            .filter(|l| !l.trim().is_empty())
            .nth(1)
            .unwrap_or("");
        assert!(
            second.starts_with("读取并遵守："),
            "second non-empty line must be the single canonical discriminator \
                 读取并遵守：, got {second:?} for intent {intent:?}"
        );
        assert!(
            card.contains("任务级别："),
            "canonical skeleton must carry the 任务级别： field for intent {intent:?}"
        );
        assert!(
            !card.contains("AGENT_SUITE_COMPACT_TASK_CARD_V1"),
            "compiled card must not contain the removed compact marker: {intent:?}"
        );
        assert!(
            !card.contains("fallback-task-cards"),
            "compiled card must not reference a fallback template set: {intent:?}"
        );
        assert!(
            !card.contains("task-template.md"),
            "compiled card must not reference a per-level template file: {intent:?}"
        );
    }
}
