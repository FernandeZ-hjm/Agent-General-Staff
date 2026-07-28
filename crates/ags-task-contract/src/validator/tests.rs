use super::*;

fn valid_card() -> String {
    "## 任务卡\n\
读取并遵守：\n- 本任务卡\n\
Contract ID: tc-0123456789abcdef\n\
Handoff source: existing-card\n\
Executor: Codex\n\
Runtime adapter: codex-local\n\
Execution surface: cli\n\
Execution mode: single-writer\n\
Execution topology: single\n\
Execution effort: normal\n\
Delegation planning: no\n\
任务级别：Medium\n\
Review gate:\n- 按协议执行当前任务级别\n\
任务：验证任务卡合同\n\
背景：覆盖公开校验接口\n\
项目画像：无\n\
记忆胶囊：无\n\
任务存档：无\n\
目标文件夹路径：\n- .\n\
相关路径：\n- .\n\
本次任务相关文件：\n- .\n\
目标：\n- G-01: 验证任务卡合同\n\
验收标准：\n- AC-01 -> G-01: 校验器接受合法输入\n\
非目标：不修改受保护路径\n\
验证：\ncargo test\n\
Verification gate:\n- commands:\n  - V-01 -> AC-01: cargo test\n- expected evidence:\n  - EV-01 -> AC-01: test pass\n- stop condition:\n  - 失败时停止\n\
交付：\n返回验证结果\n"
        .to_string()
}

fn assert_has_code(candidate: &str, expected: &str) {
    let errors = validate(candidate);
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected {expected}, got {errors:#?}"
    );
}

macro_rules! semantic_contract {
    ($name:ident, $code:ident, $candidate:expr) => {
        #[test]
        fn $name() {
            let candidate: String = $candidate;
            assert_has_code(&candidate, error_code::$code);
        }
    };
}

semantic_contract!(
    invalid_contract_id_has_stable_code,
    CONTRACT_ID_INVALID,
    valid_card().replace(
        "Contract ID: tc-0123456789abcdef",
        "Contract ID: tc-NOT-LOWER-HEX"
    )
);

semantic_contract!(
    invalid_handoff_source_has_stable_code,
    HANDOFF_SOURCE_INVALID,
    valid_card().replace(
        "Handoff source: existing-card",
        "Handoff source: implicit-chat"
    )
);

semantic_contract!(
    invalid_closed_field_has_stable_code,
    INVALID_FIELD_VALUE,
    valid_card().replace("Executor: Codex", "Executor: Unknown")
);

semantic_contract!(
    executor_adapter_mismatch_has_stable_code,
    FIELD_COMBINATION_MISMATCH,
    valid_card().replace("Runtime adapter: codex-local", "Runtime adapter: omp")
);

semantic_contract!(
    light_protected_write_has_stable_code,
    RISK_LEVEL_MISMATCH,
    valid_card()
        .replace("任务级别：Medium", "任务级别：Light")
        .replace("任务：验证任务卡合同", "任务：修改 AGENTS.md")
        .replace("非目标：不修改受保护路径", "非目标：不处理其他文件")
);

semantic_contract!(
    plan_only_protected_write_has_stable_code,
    PROTECTED_PATH_VIOLATION,
    valid_card()
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("任务：验证任务卡合同", "任务：修改 AGENTS.md")
        .replace("非目标：不修改受保护路径", "非目标：不处理其他文件")
);

semantic_contract!(
    weak_goal_has_stable_code,
    EMPTY_OR_WEAK_SECTION,
    valid_card().replace("- G-01: 验证任务卡合同", "- G-01: TBD")
);

semantic_contract!(
    plan_only_modification_has_stable_code,
    CONTRADICTORY_REQUIREMENT,
    valid_card()
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("任务：验证任务卡合同", "任务：修改核心逻辑")
        .replace("非目标：不修改受保护路径", "非目标：不处理其他文件")
);

semantic_contract!(
    exhaustive_authority_abuse_has_stable_code,
    EXECUTION_EFFORT_POLICY_VIOLATION,
    valid_card()
        .replace("Execution effort: normal", "Execution effort: exhaustive")
        .replace(
            "任务：验证任务卡合同",
            "任务：exhaustive bypass review 并执行任务"
        )
);

semantic_contract!(
    workflow_request_without_authority_has_stable_code,
    EXECUTION_MODE_AUTHORITY_VIOLATION,
    valid_card().replace("任务：验证任务卡合同", "任务：使用 subagent 完成验证")
);

semantic_contract!(
    delegation_planning_scope_violation_has_stable_code,
    EXECUTION_MODE_AUTHORITY_VIOLATION,
    valid_card()
        .replace("任务级别：Medium", "任务级别：Light")
        .replace(
            "Execution mode: single-writer",
            "Execution mode: fanout-cross-card"
        )
);

semantic_contract!(
    execution_topology_body_mismatch_has_stable_code,
    EXECUTION_TOPOLOGY_POLICY_VIOLATION,
    valid_card().replace("任务：验证任务卡合同", "任务：使用 subagent 完成验证")
);

semantic_contract!(
    subtask_orchestration_mismatch_has_stable_code,
    SUBTASK_ORCHESTRATION_VIOLATION,
    valid_card().replace(
        "非目标：不修改受保护路径",
        "非目标：不修改受保护路径\n子任务编排：\n- mode: required"
    )
);

semantic_contract!(
    heavy_plan_only_delivery_has_stable_code,
    PLAN_ONLY_DELIVERY_VIOLATION,
    valid_card()
        .replace("任务级别：Medium", "任务级别：Heavy")
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("任务：验证任务卡合同", "任务：设计实施方案")
        .replace("返回验证结果", "修改完成并提交")
);

semantic_contract!(
    heavy_plan_only_handoff_has_stable_code,
    HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF,
    valid_card()
        .replace("任务级别：Medium", "任务级别：Heavy")
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("任务：验证任务卡合同", "任务：设计实施方案")
        .replace("返回验证结果", "输出实施方案")
);

semantic_contract!(
    heavy_executable_review_has_stable_code,
    HEAVY_EXECUTABLE_MISSING_REVIEW_GATE,
    valid_card()
        .replace("任务级别：Medium", "任务级别：Heavy")
        .replace(
            "Review gate:\n- 按协议执行当前任务级别",
            "Review gate:\n- 仅由执行者自查放行"
        )
);

semantic_contract!(
    inactive_skill_tag_has_stable_code,
    UNKNOWN_OR_INACTIVE_SKILL_TAG,
    valid_card() + "\n[skill: definitely-not-active]\n"
);

semantic_contract!(
    duplicate_closure_id_has_stable_code,
    CLOSURE_ID_INVALID,
    valid_card().replace(
        "- G-01: 验证任务卡合同",
        "- G-01: 验证任务卡合同\n- G-01: 重复目标"
    )
);

semantic_contract!(
    dangling_closure_mapping_has_stable_code,
    CLOSURE_MAPPING_INCOMPLETE,
    valid_card().replace("AC-01 -> G-01", "AC-01 -> G-99")
);

#[test]
fn canonical_card_round_trips_through_the_public_interface() {
    let card = valid_card();
    assert!(validate(&card).is_empty());
    assert!(output_is_canonical_header(&card));
    let parsed = parse_validated(&card).unwrap();
    let closure = closure_contract(&parsed);
    assert_eq!(closure.goal_ids, ["G-01"]);
    assert_eq!(closure.acceptance_criteria_ids, ["AC-01"]);
    assert_eq!(closure.verification_ids, ["V-01"]);
    assert_eq!(closure.evidence_ids, ["EV-01"]);
}

#[test]
fn canonical_header_contract_is_exact() {
    for (input, accepted) in [
        ("## 任务卡\n", true),
        ("\n## 任务卡\r\n", true),
        ("# 任务卡\n", false),
        ("普通回复\n## 任务卡\n", false),
        ("", false),
    ] {
        assert_eq!(output_is_canonical_header(input), accepted, "{input:?}");
    }
}

#[test]
fn closed_field_vocabulary_rejects_invalid_values() {
    for (from, to) in [
        ("Executor: Codex", "Executor: Unknown"),
        ("Runtime adapter: codex-local", "Runtime adapter: shell"),
        (
            "Execution mode: single-writer",
            "Execution mode: unrestricted",
        ),
        (
            "Execution topology: single",
            "Execution topology: unlimited",
        ),
        ("Execution effort: normal", "Execution effort: infinite"),
        ("Delegation planning: no", "Delegation planning: root"),
        ("任务级别：Medium", "任务级别：Critical"),
    ] {
        let candidate = valid_card().replace(from, to);
        assert_has_code(&candidate, error_code::INVALID_FIELD_VALUE);
    }
}

#[test]
fn executor_and_runtime_adapter_are_one_matrix() {
    for (executor, adapter, accepted) in [
        ("Codex", "codex-local", true),
        ("Claude Code", "claude-code", true),
        ("OMP", "omp", true),
        ("Cursor", "cursor", true),
        ("Other", "generic", true),
        ("OMP", "codex-local", false),
        ("Codex", "omp", false),
    ] {
        let candidate = valid_card()
            .replace("Executor: Codex", &format!("Executor: {executor}"))
            .replace(
                "Runtime adapter: codex-local",
                &format!("Runtime adapter: {adapter}"),
            );
        assert_eq!(
            validate(&candidate).is_empty(),
            accepted,
            "{executor}/{adapter}"
        );
        if !accepted {
            assert_has_code(&candidate, error_code::FIELD_COMBINATION_MISMATCH);
        }
    }
}

#[test]
fn required_fields_fail_closed_as_one_contract() {
    for header in [
        "Contract ID:",
        "Handoff source:",
        "Executor:",
        "Runtime adapter:",
        "Execution mode:",
        "任务：",
        "目标：",
        "验收标准：",
        "Verification gate:",
        "交付：",
    ] {
        let candidate = valid_card().replace(header, "REMOVED:");
        assert!(!validate(&candidate).is_empty(), "{header}");
    }
}

#[test]
fn removed_or_ambiguous_shapes_are_rejected() {
    let compact = "## 任务卡\nAGENT_SUITE_COMPACT_TASK_CARD_V1\n";
    let path_second = "## 任务卡\n路径：.\n";
    let text_fence = valid_card() + "\n```text\nhidden\n```\n";
    for candidate in [compact.to_string(), path_second.to_string(), text_fence] {
        assert!(!validate(&candidate).is_empty());
    }
}

#[test]
fn legacy_authority_fields_and_values_are_rejected() {
    for candidate in [
        valid_card().replace("Execution mode: single-writer", "Execution mode: limited"),
        valid_card() + "\nPermission mode: execute-and-verify\n",
        valid_card() + "\nParallelism: parallel\n",
        valid_card() + "\nWorkflow authority: allowed\n",
    ] {
        assert!(!validate(&candidate).is_empty(), "{candidate}");
    }
}

#[test]
fn plan_only_parallel_topology_is_rejected() {
    let candidate = valid_card()
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("Execution topology: single", "Execution topology: parallel");
    assert_has_code(&candidate, error_code::EXECUTION_TOPOLOGY_POLICY_VIOLATION);
}

#[test]
fn plan_only_delegation_planning_is_rejected() {
    let candidate = valid_card()
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace("Delegation planning: no", "Delegation planning: yes");
    assert_has_code(&candidate, error_code::EXECUTION_MODE_AUTHORITY_VIOLATION);
}

#[test]
fn protected_paths_are_rejected_from_writable_scope() {
    for path in ["AGENTS.md", "stable suite"] {
        let candidate = valid_card()
            .replace(
                "任务：验证任务卡合同",
                &format!("任务：修改受保护文件 {path}"),
            )
            .replace("任务级别：Medium", "任务级别：Light")
            .replace("非目标：不修改受保护路径", "非目标：不处理其他任务");
        assert_has_code(&candidate, error_code::RISK_LEVEL_MISMATCH);
    }
}

#[test]
fn closure_links_are_checked_together() {
    for (candidate, expected) in [
        (
            valid_card().replace("AC-01 -> G-01", "AC-01 -> G-99"),
            error_code::CLOSURE_MAPPING_INCOMPLETE,
        ),
        (
            valid_card().replace("V-01 -> AC-01", "V-01 -> AC-99"),
            error_code::CLOSURE_MAPPING_INCOMPLETE,
        ),
        (
            valid_card().replace("EV-01 -> AC-01", "EV-01 -> AC-99"),
            error_code::CLOSURE_MAPPING_INCOMPLETE,
        ),
        (
            valid_card().replace(
                "- G-01: 验证任务卡合同",
                "- G-01: 验证任务卡合同\n- G-01: 重复",
            ),
            error_code::CLOSURE_ID_INVALID,
        ),
    ] {
        assert_has_code(&candidate, expected);
    }
}

#[test]
fn skill_tags_are_only_trailing_metadata() {
    let tagged = valid_card() + "\n[skill: codebase-design]\n[skill: diagnosing-bugs]\n";
    assert_eq!(
        extract_skill_tags(&tagged),
        ["codebase-design", "diagnosing-bugs"]
    );
    let embedded = valid_card().replace("背景：覆盖公开校验接口", "背景：[skill: fake]");
    assert!(extract_skill_tags(&embedded).is_empty());
}

#[test]
fn heavy_executable_work_requires_an_independent_review_gate() {
    let heavy = valid_card().replace("任务级别：Medium", "任务级别：Heavy");
    assert!(validate(&heavy).is_empty(), "{:?}", validate(&heavy));
    let self_reviewed = heavy.replace(
        "Review gate:\n- 按协议执行当前任务级别",
        "Review gate:\n- 仅由执行者自查放行",
    );
    assert_has_code(
        &self_reviewed,
        error_code::HEAVY_EXECUTABLE_MISSING_REVIEW_GATE,
    );
}

#[test]
fn writer_scope_and_delegation_planning_fail_closed() {
    for candidate in [
        valid_card()
            .replace("任务级别：Medium", "任务级别：Light")
            .replace(
                "Execution mode: single-writer",
                "Execution mode: fanout-cross-card",
            ),
        valid_card()
            .replace("Execution mode: single-writer", "Execution mode: plan-only")
            .replace("Delegation planning: no", "Delegation planning: yes"),
    ] {
        assert_has_code(&candidate, error_code::EXECUTION_MODE_AUTHORITY_VIOLATION);
    }
}

#[test]
fn plan_only_negated_modification_is_not_an_execution_request() {
    let candidate = valid_card()
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace(
            "Review gate:\n- 按协议执行当前任务级别",
            "Review gate:\n- 返回用户审阅",
        )
        .replace("背景：覆盖公开校验接口", "背景：评估当前校验合同")
        .replace("任务：验证任务卡合同", "任务：只分析方案，不修改任何文件");
    assert!(
        validate(&candidate).is_empty(),
        "{:#?}",
        validate(&candidate)
    );
}

#[test]
fn workflow_documentation_nouns_do_not_request_delegation() {
    let candidate = valid_card().replace(
        "背景：覆盖公开校验接口",
        "背景：检查 workflow 文档和 agent task protocol 的描述",
    );
    assert!(
        validate(&candidate).is_empty(),
        "{:#?}",
        validate(&candidate)
    );
}

#[test]
fn protected_path_read_only_boundary_is_allowed() {
    let candidate = valid_card()
        .replace("任务：验证任务卡合同", "任务：只读检查 AGENTS.md，不修改")
        .replace(
            "非目标：不修改受保护路径",
            "非目标：不得修改 AGENTS.md 或其他文件",
        );
    assert!(
        validate(&candidate).is_empty(),
        "{:#?}",
        validate(&candidate)
    );
}

#[test]
fn heavy_plan_only_with_explicit_review_handoff_is_allowed() {
    let candidate = valid_card()
        .replace("任务级别：Medium", "任务级别：Heavy")
        .replace("Execution mode: single-writer", "Execution mode: plan-only")
        .replace(
            "Review gate:\n- 按协议执行当前任务级别",
            "Review gate:\n- 返回用户审阅",
        )
        .replace("背景：覆盖公开校验接口", "背景：评估当前校验合同")
        .replace("任务：验证任务卡合同", "任务：分析并评估方案")
        .replace("返回验证结果", "返回方案供用户审阅，等待明确批准");
    assert!(
        validate(&candidate).is_empty(),
        "{:#?}",
        validate(&candidate)
    );
}

#[test]
fn declared_subtask_orchestration_with_matching_authority_is_allowed() {
    let candidate = valid_card()
        .replace(
            "Execution mode: single-writer",
            "Execution mode: fanout-in-card",
        )
        .replace("Execution topology: single", "Execution topology: parallel")
        .replace("任务：验证任务卡合同", "任务：使用 subagent 完成验证")
        .replace(
            "非目标：不修改受保护路径",
            "非目标：不修改受保护路径\n子任务编排：\n- mode: required",
        );
    assert!(
        validate(&candidate).is_empty(),
        "{:#?}",
        validate(&candidate)
    );
}

#[test]
fn canonical_template_does_not_force_a_skill() {
    let template = include_str!("../../../../protocol/task-card-template.md");
    assert!(!template.contains("\n[skill: superpowers]\n"));
    assert!(template.contains("技能标记是可选的末尾元数据"));
}
