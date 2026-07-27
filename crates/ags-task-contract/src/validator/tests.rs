use super::*;

fn valid_card() -> String {
    "## 任务卡\n\
读取并遵守：\n- 本任务卡\n\
Contract ID: tc-0123456789abcdef\n\
Handoff source: existing-card\n\
Executor: Codex\n\
Runtime adapter: codex-local\n\
Execution surface: cli\n\
Permission mode: execute-and-verify\n\
Parallelism: none\n\
Execution effort: normal\n\
Workflow authority: none\n\
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
            "Permission mode: execute-and-verify",
            "Permission mode: unrestricted",
        ),
        ("Parallelism: none", "Parallelism: unlimited"),
        ("Execution effort: normal", "Execution effort: infinite"),
        ("Workflow authority: none", "Workflow authority: root"),
        ("任务级别：Medium", "任务级别：Critical"),
    ] {
        let candidate = valid_card().replace(from, to);
        assert!(!validate(&candidate).is_empty(), "{to}");
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
    }
}

#[test]
fn required_fields_fail_closed_as_one_contract() {
    for header in [
        "Contract ID:",
        "Handoff source:",
        "Executor:",
        "Runtime adapter:",
        "Permission mode:",
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
fn protected_paths_are_rejected_from_writable_scope() {
    for path in ["AGENTS.md", "stable suite"] {
        let candidate = valid_card()
            .replace(
                "任务：验证任务卡合同",
                &format!("任务：修改受保护文件 {path}"),
            )
            .replace("任务级别：Medium", "任务级别：Light")
            .replace("非目标：不修改受保护路径", "非目标：不处理其他任务");
        assert!(!validate(&candidate).is_empty(), "{path}");
    }
}

#[test]
fn closure_links_are_checked_together() {
    for candidate in [
        valid_card().replace("AC-01 -> G-01", "AC-01 -> G-99"),
        valid_card().replace("V-01 -> AC-01", "V-01 -> AC-99"),
        valid_card().replace("EV-01 -> AC-01", "EV-01 -> AC-99"),
        valid_card().replace(
            "- G-01: 验证任务卡合同",
            "- G-01: 验证任务卡合同\n- G-01: 重复",
        ),
    ] {
        assert!(!validate(&candidate).is_empty());
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
    assert!(!validate(&self_reviewed).is_empty());
}

#[test]
fn workflow_authority_cannot_exceed_task_or_permission_scope() {
    for candidate in [
        valid_card()
            .replace("任务级别：Medium", "任务级别：Light")
            .replace("Workflow authority: none", "Workflow authority: allowed"),
        valid_card()
            .replace(
                "Permission mode: execute-and-verify",
                "Permission mode: plan-only",
            )
            .replace("Workflow authority: none", "Workflow authority: allowed"),
        valid_card()
            .replace(
                "Permission mode: execute-and-verify",
                "Permission mode: plan-only",
            )
            .replace(
                "Workflow authority: none",
                "Workflow authority: within-card",
            ),
    ] {
        assert!(!validate(&candidate).is_empty());
    }
}

#[test]
fn canonical_template_does_not_force_a_skill() {
    let template = include_str!("../../../../protocol/task-card-template.md");
    assert!(!template.contains("\n[skill: superpowers]\n"));
    assert!(template.contains("技能标记是可选的末尾元数据"));
}
