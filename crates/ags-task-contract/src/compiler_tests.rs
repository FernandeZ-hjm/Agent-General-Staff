use super::*;

fn contract(executor: &str) -> HandoffContract {
    HandoffContract {
        schema_version: HANDOFF_CONTRACT_SCHEMA_VERSION.to_string(),
        task_level: TaskLevel::Medium,
        task: "实现已确认的变更".to_string(),
        fields: std::collections::HashMap::from([
            ("Executor:".to_string(), executor.to_string()),
            (
                "目标：".to_string(),
                "- G-01: 完成已确认的实现".to_string(),
            ),
            (
                "验收标准：".to_string(),
                "- AC-01 -> G-01: 相关验证通过".to_string(),
            ),
            (
                "Verification gate:".to_string(),
                "- commands:\n  - V-01 -> AC-01: cargo test\n- expected evidence:\n  - EV-01 -> AC-01: test pass\n- stop condition:\n  - 失败时停止"
                    .to_string(),
            ),
        ]),
    }
}

#[test]
fn typed_contract_compiles_to_a_canonical_card() {
    let (card, report) =
        compile_typed_handoff_contract(&contract("Codex"), Path::new("."), false, true, true)
            .unwrap();

    assert!(report.executable_allowed, "{:?}", report.validation_errors);
    assert!(report.validation_passed, "{:?}", report.validation_errors);
    assert_eq!(report.contract_format, "typed");
    assert!(card.starts_with("## 任务卡\n"));
    assert!(validator::validate(&card).is_empty());
}

#[test]
fn omp_is_a_first_class_executor() {
    let (card, report) =
        compile_typed_handoff_contract(&contract("OMP"), Path::new("."), false, true, true)
            .unwrap();

    assert!(report.executable_allowed, "{:?}", report.validation_errors);
    assert!(card.contains("Executor: OMP"));
    assert!(card.contains("Runtime adapter: omp"));
}

#[test]
fn untyped_or_unknown_input_fails_closed() {
    for input in [
        "任务：旧的宽松编译输入",
        r#"{"schema_version":"0.3.6-handoff-contract","task_level":"Medium","task":"x","unknown":true}"#,
        r#"{"schema_version":"0.2.0","task_level":"Medium","task":"x"}"#,
    ] {
        let (card, report) = compile_with_contract(input, Path::new("."), false, true, true);
        assert!(card.is_empty());
        assert!(!report.executable_allowed);
        assert_eq!(report.contract_format, "typed_invalid");
        assert_eq!(
            report.block_reason.as_deref(),
            Some("invalid_typed_handoff_contract")
        );
    }
}

#[test]
fn both_handoff_gates_remain_required() {
    let input = serde_json::to_string(&contract("Codex")).unwrap();
    for (requested, confirmed, expected) in [
        (false, true, "task_card_not_requested"),
        (true, false, "handoff_contract_not_confirmed"),
    ] {
        let (card, report) =
            compile_with_contract(&input, Path::new("."), false, requested, confirmed);
        assert!(card.is_empty());
        assert_eq!(report.block_reason.as_deref(), Some(expected));
    }
}

#[test]
fn check_only_never_returns_executable_text() {
    let input = serde_json::to_string(&contract("Codex")).unwrap();
    let (card, report) = compile_with_contract(&input, Path::new("."), true, true, true);
    assert!(card.is_empty());
    assert!(!report.executable_allowed);
    assert_eq!(report.block_reason.as_deref(), Some("check_only"));
}

#[test]
fn typed_members_cannot_be_overridden_through_fields() {
    let mut input = contract("Codex");
    input
        .fields
        .insert("任务级别：".to_string(), "Heavy".to_string());
    let error =
        compile_typed_handoff_contract(&input, Path::new("."), false, true, true).unwrap_err();
    assert!(error.iter().any(|item| item.contains("cannot override")));
}
