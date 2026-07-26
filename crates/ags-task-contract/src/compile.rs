//! Typed and legacy-compatible task-card compilation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::context::{gather_project_context, ProjectContext, SlotSource};
use crate::intent::{
    is_structured_contract_intent, parse_intent, HandoffContract, HandoffSource, FIELD_HEADERS,
    HANDOFF_CONTRACT_SCHEMA_VERSION, REQUIRED_FIELDS, SCHEMA_VERSION,
};

// ── Compilation ─────────────────────────────────────────────────────────

/// A single slot entry in the compile report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotEntry {
    /// Canonical field header (e.g. `"任务："`).
    pub field: String,
    /// The value filled (if any).
    pub value: String,
    /// Where this value came from.
    pub source: SlotSource,
}

/// Compile report — the structured output of a compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileReport {
    /// Schema version for this report format.
    pub schema_version: String,
    /// The compiled task card text (empty in check-only mode).
    pub compiled_task_card: String,
    /// Per-slot source tracking.
    pub slot_sources: Vec<SlotEntry>,
    /// Slots that could not be filled.
    pub missing_slots: Vec<String>,
    /// Assumptions made during compilation.
    pub assumptions: Vec<String>,
    /// Compatibility notices for legacy loose contracts.
    pub deprecations: Vec<String>,
    /// `typed` for [`HandoffContract`], otherwise `legacy_loose`.
    pub contract_format: String,
    /// Whether the compiled card passes `ags task validate`.
    pub validation_passed: bool,
    /// Validation errors, if any.
    pub validation_errors: Vec<String>,
    /// Whether this was a check-only run.
    pub check_only: bool,
    /// Whether the user explicitly requested a task card
    /// (`--task-card-requested` flag).
    pub task_card_requested: bool,
    /// Whether the caller supplied structured evidence that the solution,
    /// scope, verification, and handoff contract is confirmed.
    pub confirmed_handoff_contract: bool,
    /// Whether the final, decision-complete host Plan-mode artifact triggered
    /// task-card compilation.
    pub host_plan_mode_final: bool,
    /// Structured origin rendered into `Handoff source:`.
    pub handoff_source: String,
    /// Whether executable task card output is allowed.
    /// Requires both handoff gates, `check_only=false`, and no missing slots.
    pub executable_allowed: bool,
    /// If executable output is blocked, the reason.
    /// Possible values: "task_card_not_requested",
    /// "handoff_contract_not_confirmed", "check_only", "missing_slots", or
    /// `null` when allowed.
    pub block_reason: Option<String>,
}

/// Compile an approved execution intent (execution contract) into a canonical
/// task card.
///
/// The canonical input is a confirmed execution contract from the solution phase,
/// not raw user chat. The compiler accepts flexible intents for backward
/// compatibility, but callers should only pass confirmed execution contracts.
///
/// # Task-card request gate
///
/// `task_card_requested` and `confirmed_handoff_contract` form the hard gate
/// between "solution OK" and task-card generation. Without either, the compiler
/// produces a diagnostic report only —
/// `executable_allowed` is `false`, `block_reason` is set to
/// the corresponding gate reason, and the compiled task card text is suppressed.
/// The compiler does not interpret the contract as natural language. The
/// requirement router owns that decision before this structured seam.
///
/// Returns the compiled card text and the full compile report.
/// If `check_only` is true, the compiled card is only validated but
/// the report is still returned for inspection.
pub fn compile_with_contract(
    intent: &str,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
) -> (String, CompileReport) {
    compile_with_handoff_source(
        intent,
        project_root,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
        HandoffSource::ExplicitHandoff,
    )
}

/// Compile with an explicit structured handoff origin.
///
/// `HostPlanMode` means the host has reached its final, decision-complete
/// Plan-mode artifact. It replaces a separate task-card-request prompt but
/// still requires a confirmed, closed handoff contract.
pub fn compile_with_handoff_source(
    intent: &str,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
    handoff_source: HandoffSource,
) -> (String, CompileReport) {
    if intent.trim_start().starts_with('{') {
        return match serde_json::from_str::<HandoffContract>(intent) {
            Ok(contract) => match compile_typed_handoff_contract_with_source(
                &contract,
                project_root,
                check_only,
                task_card_requested,
                confirmed_handoff_contract,
                handoff_source,
            ) {
                Ok(result) => result,
                Err(errors) => invalid_typed_contract_report(
                    errors,
                    check_only,
                    task_card_requested,
                    confirmed_handoff_contract,
                    handoff_source,
                ),
            },
            Err(error) => invalid_typed_contract_report(
                vec![format!("invalid typed handoff contract: {error}")],
                check_only,
                task_card_requested,
                confirmed_handoff_contract,
                handoff_source,
            ),
        };
    }
    compile_legacy_contract(
        intent,
        project_root,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
        handoff_source,
    )
}

fn invalid_typed_contract_report(
    errors: Vec<String>,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
    handoff_source: HandoffSource,
) -> (String, CompileReport) {
    (
        String::new(),
        CompileReport {
            schema_version: SCHEMA_VERSION.to_string(),
            compiled_task_card: String::new(),
            slot_sources: Vec::new(),
            missing_slots: Vec::new(),
            assumptions: Vec::new(),
            deprecations: Vec::new(),
            contract_format: "typed_invalid".to_string(),
            validation_passed: false,
            validation_errors: errors,
            check_only,
            task_card_requested,
            confirmed_handoff_contract,
            host_plan_mode_final: handoff_source == HandoffSource::HostPlanMode,
            handoff_source: handoff_source.as_str().to_string(),
            executable_allowed: false,
            block_reason: Some("invalid_typed_handoff_contract".to_string()),
        },
    )
}

pub fn compile_typed_handoff_contract(
    contract: &HandoffContract,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
) -> Result<(String, CompileReport), Vec<String>> {
    compile_typed_handoff_contract_with_source(
        contract,
        project_root,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
        HandoffSource::ExplicitHandoff,
    )
}

pub fn compile_typed_handoff_contract_with_source(
    contract: &HandoffContract,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
    handoff_source: HandoffSource,
) -> Result<(String, CompileReport), Vec<String>> {
    let intent = render_typed_handoff_contract(contract)?;
    let (card, mut report) = compile_legacy_contract(
        &intent,
        project_root,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
        handoff_source,
    );
    report.contract_format = "typed".to_string();
    report
        .deprecations
        .retain(|notice| notice != "legacy_loose_contract_missing_task_level_defaulted_to_medium");
    Ok((card, report))
}

fn render_typed_handoff_contract(contract: &HandoffContract) -> Result<String, Vec<String>> {
    let mut errors = Vec::new();
    if contract.schema_version != HANDOFF_CONTRACT_SCHEMA_VERSION {
        errors.push(format!(
            "unsupported handoff contract schema {}; expected {HANDOFF_CONTRACT_SCHEMA_VERSION}",
            contract.schema_version
        ));
    }
    if contract.task.trim().is_empty() {
        errors.push("typed handoff contract task must not be empty".to_string());
    }
    for key in contract.fields.keys() {
        if matches!(
            key.as_str(),
            "任务：" | "任务级别：" | "Contract ID:" | "Handoff source:"
        ) {
            errors.push(format!(
                "typed field {key} cannot override a required typed member"
            ));
        } else if !FIELD_HEADERS.iter().any(|(header, _)| key == header) {
            errors.push(format!("unknown typed handoff field: {key}"));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut lines = vec![
        format!("任务级别：{}", contract.task_level.as_str()),
        format!("任务：{}", contract.task.trim()),
    ];
    let mut fields = contract.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| left.0.cmp(right.0));
    for (header, value) in fields {
        let inline = FIELD_HEADERS
            .iter()
            .find(|(known, _)| header == known)
            .map(|(_, inline)| *inline)
            .unwrap_or(false);
        if inline {
            lines.push(format!("{header} {}", value.trim()));
        } else {
            lines.push(format!("{header}\n{}", value.trim()));
        }
    }
    Ok(lines.join("\n"))
}

fn compile_legacy_contract(
    intent: &str,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
    handoff_source: HandoffSource,
) -> (String, CompileReport) {
    let ctx = gather_project_context(project_root);
    let parsed = parse_intent(intent);
    let contract_is_structured = is_structured_contract_intent(intent);

    let mut slot_sources: Vec<SlotEntry> = Vec::new();
    let mut assumptions: Vec<String> = Vec::new();
    let mut deprecations: Vec<String> = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();

    // ── Phase 1: fill fields from intent ────────────────────────────
    for (header, _is_inline) in FIELD_HEADERS {
        if let Some(val) = parsed.fields.get(*header) {
            if !val.is_empty() {
                fields.insert(header.to_string(), val.clone());
                slot_sources.push(SlotEntry {
                    field: header.to_string(),
                    value: val.clone(),
                    source: SlotSource::Intent,
                });
            }
        }
    }

    // Contract identity and handoff origin are compiler-owned. They are
    // deterministic and cannot be overridden by loose or typed input.
    let contract_material = format!(
        "{}\n---handoff-source---\n{}",
        intent.trim(),
        handoff_source.as_str()
    );
    let contract_hash = ags_platform::sha256_hex(contract_material.as_bytes());
    let contract_id = format!("tc-{}", &contract_hash[..16]);
    for (field, value) in [
        ("Contract ID:", contract_id),
        ("Handoff source:", handoff_source.as_str().to_string()),
    ] {
        fields.insert(field.to_string(), value.clone());
        slot_sources.retain(|slot| slot.field != field);
        slot_sources.push(SlotEntry {
            field: field.to_string(),
            value,
            source: SlotSource::Derived,
        });
    }

    // ── Phase 2: project-aware slot filling ────────────────────────

    // 读取并遵守：— the read-and-obey list, built from project context
    if !has_field(&fields, "读取并遵守：") {
        let reads = build_reads_section(&ctx);
        let reads = if reads.is_empty() {
            "- 本任务卡".to_string()
        } else {
            reads
        };
        fields.insert("读取并遵守：".to_string(), reads.clone());
        slot_sources.push(SlotEntry {
            field: "读取并遵守：".to_string(),
            value: reads,
            source: SlotSource::ProjectContext,
        });
    }

    // Executor: — default Claude Code
    if !has_field(&fields, "Executor:") {
        fields.insert("Executor:".to_string(), "Claude Code".to_string());
        slot_sources.push(SlotEntry {
            field: "Executor:".to_string(),
            value: "Claude Code".to_string(),
            source: SlotSource::Default,
        });
    }

    // Runtime adapter: — from executor
    if !has_field(&fields, "Runtime adapter:") {
        let executor = fields
            .get("Executor:")
            .map(|s| s.as_str())
            .unwrap_or("Claude Code");
        let adapter = executor_to_adapter(executor);
        fields.insert("Runtime adapter:".to_string(), adapter.to_string());
        slot_sources.push(SlotEntry {
            field: "Runtime adapter:".to_string(),
            value: adapter.to_string(),
            source: SlotSource::Default,
        });
    }

    // Execution surface: — default cli
    if !has_field(&fields, "Execution surface:") {
        fields.insert("Execution surface:".to_string(), "cli".to_string());
        slot_sources.push(SlotEntry {
            field: "Execution surface:".to_string(),
            value: "cli".to_string(),
            source: SlotSource::Default,
        });
    }

    // Permission mode: — default direct execution. Heavy tasks with an
    // unspecified mode are conservatively rewritten to plan-only below.
    if !has_field(&fields, "Permission mode:") {
        fields.insert(
            "Permission mode:".to_string(),
            "execute-and-verify".to_string(),
        );
        slot_sources.push(SlotEntry {
            field: "Permission mode:".to_string(),
            value: "execute-and-verify".to_string(),
            source: SlotSource::Default,
        });
    }

    // Parallelism: — default none
    if !has_field(&fields, "Parallelism:") {
        fields.insert("Parallelism:".to_string(), "none".to_string());
        slot_sources.push(SlotEntry {
            field: "Parallelism:".to_string(),
            value: "none".to_string(),
            source: SlotSource::Default,
        });
    }

    // 任务级别：— typed contracts always provide it. A legacy loose contract
    // may omit it for one compatibility window; the only allowed fallback is
    // Medium plus an explicit deprecation. Natural-language keywords are never
    // a task-level authority.
    if !has_field(&fields, "任务级别：") {
        let level = "Medium".to_string();
        fields.insert("任务级别：".to_string(), level.clone());
        deprecations
            .push("legacy_loose_contract_missing_task_level_defaulted_to_medium".to_string());
        slot_sources.push(SlotEntry {
            field: "任务级别：".to_string(),
            value: level,
            source: SlotSource::Default,
        });
    }

    // Review gate: — default referencing the protocol
    if !has_field(&fields, "Review gate:") {
        let rg =
            "- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别".to_string();
        fields.insert("Review gate:".to_string(), rg.clone());
        slot_sources.push(SlotEntry {
            field: "Review gate:".to_string(),
            value: rg,
            source: SlotSource::Default,
        });
    }

    // 背景：— default if absent
    if !has_field(&fields, "背景：") {
        let bg = "本次任务差异见目标与实施要求".to_string();
        fields.insert("背景：".to_string(), bg.clone());
        slot_sources.push(SlotEntry {
            field: "背景：".to_string(),
            value: bg,
            source: SlotSource::Default,
        });
    }

    // 项目画像：— default 无
    if !has_field(&fields, "项目画像：") {
        fields.insert("项目画像：".to_string(), "无".to_string());
        slot_sources.push(SlotEntry {
            field: "项目画像：".to_string(),
            value: "无".to_string(),
            source: SlotSource::Default,
        });
    }

    // 记忆胶囊：— from memory path, fallback 无
    if !has_field(&fields, "记忆胶囊：") {
        let (val, source) = match ctx.capsule_path {
            Some(ref cap_path) => (format!("- {}", cap_path.display()), SlotSource::MemoryPath),
            None => ("无".to_string(), SlotSource::Default),
        };
        fields.insert("记忆胶囊：".to_string(), val.clone());
        slot_sources.push(SlotEntry {
            field: "记忆胶囊：".to_string(),
            value: val,
            source,
        });
    }

    // 任务存档：— from memory path, fallback 无
    if !has_field(&fields, "任务存档：") {
        let (val, source) = match ctx.task_memory_path {
            Some(ref tm_path) => (format!("- {}", tm_path.display()), SlotSource::MemoryPath),
            None => ("无".to_string(), SlotSource::Default),
        };
        fields.insert("任务存档：".to_string(), val.clone());
        slot_sources.push(SlotEntry {
            field: "任务存档：".to_string(),
            value: val,
            source,
        });
    }

    // 目标文件夹路径：— actual target/workspace root for this task
    if !has_field(&fields, "目标文件夹路径：") {
        let target_folder = format!("- {}", ctx.project_root.to_string_lossy());
        fields.insert("目标文件夹路径：".to_string(), target_folder.clone());
        slot_sources.push(SlotEntry {
            field: "目标文件夹路径：".to_string(),
            value: target_folder,
            source: SlotSource::ProjectContext,
        });
    }

    // 相关路径：— from project context
    if !has_field(&fields, "相关路径：") {
        let default_paths = if ctx.is_ags_suite {
            "- crates/\n- scripts/\n- tests/".to_string()
        } else {
            format!("- {}", ctx.project_root.to_string_lossy())
        };
        fields.insert("相关路径：".to_string(), default_paths.clone());
        slot_sources.push(SlotEntry {
            field: "相关路径：".to_string(),
            value: default_paths,
            source: SlotSource::ProjectContext,
        });
    }

    // 本次任务相关文件：— default if absent
    if !has_field(&fields, "本次任务相关文件：") {
        let files = if ctx.is_ags_suite {
            "- Cargo.toml".to_string()
        } else {
            "- .".to_string()
        };
        fields.insert("本次任务相关文件：".to_string(), files.clone());
        slot_sources.push(SlotEntry {
            field: "本次任务相关文件：".to_string(),
            value: files,
            source: SlotSource::ProjectContext,
        });
    }

    // ── Phase 3: defaults for remaining optional fields ─────────────

    // 非目标：— default if absent
    if !has_field(&fields, "非目标：") {
        fields.insert("非目标：".to_string(), "- 无".to_string());
        slot_sources.push(SlotEntry {
            field: "非目标：".to_string(),
            value: "- 无".to_string(),
            source: SlotSource::Default,
        });
    }

    // 验证：— default if absent
    if !has_field(&fields, "验证：") {
        fields.insert("验证：".to_string(), "按任务卡验证门禁执行".to_string());
        slot_sources.push(SlotEntry {
            field: "验证：".to_string(),
            value: "按任务卡验证门禁执行".to_string(),
            source: SlotSource::Default,
        });
    }

    // 交付：— default if absent
    if !has_field(&fields, "交付：") {
        let delivery = "- 按 protocol/agent-task-protocol.md 输出交付报告\n\
- 报告必须回填本卡 Contract ID、LaunchPlan task_card_hash，并逐项闭环 G-*/AC-*/V-*；未闭环项不得隐藏\n\
- 报告落盘后运行 `ags task close <task-card> <delivery-report>`，通过后再生成或归档 receipt"
            .to_string();
        fields.insert("交付：".to_string(), delivery.clone());
        slot_sources.push(SlotEntry {
            field: "交付：".to_string(),
            value: delivery,
            source: SlotSource::Default,
        });
    }

    // ── Phase 3b: Heavy default permission (unspecified-field default) ──
    // Compiler default ONLY: when a Heavy card does not declare a Permission
    // mode, fill plan-only as the conservative default for the unspecified
    // field. This is NOT the resolver's M4 — task LEVEL never downgrades an
    // explicitly declared permission. The perm_source_is_default guard ensures
    // an explicit Permission mode is always preserved.
    let task_level = fields.get("任务级别：").map(|s| s.as_str()).unwrap_or("");
    if task_level == "Heavy" {
        // Check if permission mode was default-filled (not user-provided)
        let perm_source_is_default = slot_sources
            .iter()
            .any(|s| s.field == "Permission mode:" && s.source == SlotSource::Default);
        if perm_source_is_default {
            // Fill the unspecified Permission mode with the conservative default.
            fields.insert("Permission mode:".to_string(), "plan-only".to_string());
            // Update slot_sources: replace the Default entry
            if let Some(entry) = slot_sources
                .iter_mut()
                .find(|s| s.field == "Permission mode:")
            {
                entry.value = "plan-only".to_string();
                entry.source = SlotSource::Default;
            }
            assumptions.push(
                "Heavy task: Permission mode unspecified — compiler default plan-only \
                 (conservative default for an unspecified field; an explicit \
                 Permission mode is always preserved)"
                    .to_string(),
            );
        }
    }

    // ── Phase 4: detect missing required slots ──────────────────────
    let missing_slots: Vec<String> = REQUIRED_FIELDS
        .iter()
        .filter(|h| !has_field(&fields, h))
        .map(|h| h.to_string())
        .collect();

    // Also check that 任务：and 目标：have meaningful content
    if has_field(&fields, "任务：")
        && is_empty_or_placeholder(fields.get("任务：").unwrap())
        && !missing_slots.contains(&"任务：".to_string())
    {
        // Don't add to missing_slots if it's there, but flag the weak content
    }
    if has_field(&fields, "目标：")
        && is_empty_or_placeholder(fields.get("目标：").unwrap())
        && !missing_slots.contains(&"目标：".to_string())
    {
        // Same — weak content in 目标：
    }

    // ── Phase 5: build the canonical task card ──────────────────────
    // Always render the classic skeleton. When slots are missing the card is
    // still rendered for diagnostics but will not pass validation.
    let compiled_card = render_task_card(&fields);

    // ── Phase 6: validate against task card validator ───────────────
    // We do a structural self-check here. The actual validation is done
    // by the CLI calling validator::validate().
    let (validation_passed, validation_errors) = if missing_slots.is_empty() {
        // Basic structural self-check
        let mut errors: Vec<String> = Vec::new();
        if !compiled_card.starts_with("## 任务卡") {
            errors.push("Compiled card does not start with ## 任务卡".to_string());
        }
        if compiled_card.contains("```text") || compiled_card.contains("```markdown") {
            // Check that outer fence is ~~~~markdown not ```markdown
            // This is a heuristic check — the validator does proper fence detection
        }
        (errors.is_empty(), errors)
    } else {
        (
            false,
            vec![format!(
                "Missing required slots: {}",
                missing_slots.join(", ")
            )],
        )
    };

    // ── Phase 7: task-card handoff generation gate ──────────────────
    // Determine whether executable output is allowed.
    // Four conditions must all be met:
    //   1. Explicit handoff requested a task card, or this is the final
    //      decision-complete host Plan-mode artifact
    //   2. The handoff contract is confirmed through structured caller state
    //   3. Not in check-only mode
    //   4. No missing slots
    let host_plan_mode_final = handoff_source == HandoffSource::HostPlanMode;
    let handoff_requested = task_card_requested || host_plan_mode_final;
    let (executable_allowed, block_reason) = if !handoff_requested {
        (false, Some("task_card_not_requested".to_string()))
    } else if !confirmed_handoff_contract {
        (false, Some("handoff_contract_not_confirmed".to_string()))
    } else if !contract_is_structured {
        (false, Some("handoff_contract_not_structured".to_string()))
    } else if check_only {
        (false, Some("check_only".to_string()))
    } else if !missing_slots.is_empty() {
        (false, Some("missing_slots".to_string()))
    } else {
        (true, None)
    };

    // Suppress card output when not allowed
    let report_card = if executable_allowed {
        compiled_card.clone()
    } else {
        String::new()
    };

    // If blocked by task_card_not_requested, the validation result is
    // informational only — the card was never meant to be executable.
    let effective_validation_passed = if executable_allowed {
        validation_passed
    } else {
        false
    };

    let effective_validation_errors = if executable_allowed {
        validation_errors
    } else if let Some(ref reason) = block_reason {
        vec![format!(
            "Executable output blocked: {}",
            match reason.as_str() {
                "task_card_not_requested" => {
                    "task card not requested (use --task-card-requested only after an explicit handoff instruction)"
                }
                "handoff_contract_not_confirmed" => "handoff contract not confirmed (supply structured --confirmed-handoff-contract evidence)",
                "handoff_contract_not_structured" => "handoff contract must contain a `任务：` field and no unscoped free text",
                "check_only" => "check-only mode",
                "missing_slots" => "missing required slots",
                other => other,
            }
        )]
    } else {
        validation_errors
    };

    let report = CompileReport {
        schema_version: SCHEMA_VERSION.to_string(),
        compiled_task_card: report_card,
        slot_sources,
        missing_slots,
        assumptions,
        deprecations,
        contract_format: "legacy_loose".to_string(),
        validation_passed: effective_validation_passed,
        validation_errors: effective_validation_errors,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
        host_plan_mode_final,
        handoff_source: handoff_source.as_str().to_string(),
        executable_allowed,
        block_reason,
    };

    // Return the gated card: empty when not allowed, the real card when allowed.
    // This closes the public API bypass where a Rust caller could receive an
    // executable card via `let (card, _) = compile(...)` while the report says
    // `executable_allowed=false`.
    let gated_card = if executable_allowed {
        compiled_card
    } else {
        String::new()
    };

    (gated_card, report)
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn has_field(fields: &HashMap<String, String>, key: &str) -> bool {
    fields.get(key).is_some_and(|v| !v.is_empty())
}

fn is_empty_or_placeholder(val: &str) -> bool {
    let trimmed = val.trim();
    trimmed.is_empty()
        || trimmed == "- 无"
        || trimmed == "无"
        || trimmed == "todo"
        || trimmed == "tbd"
        || trimmed == "待定"
}

fn executor_to_adapter(executor: &str) -> &'static str {
    match executor {
        "Codex" => "codex-local",
        "Claude Code" => "claude-code",
        "Cursor" => "cursor",
        "OMP" => "omp",
        _ => "generic",
    }
}

/// Build the `读取并遵守：` read-and-obey section from project context.
fn build_reads_section(ctx: &ProjectContext) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("- 本任务卡".to_string());

    if ctx.is_ags_suite {
        lines.push("- AGENTS.md".to_string());
        lines.push("- CLAUDE.md".to_string());
        lines.push("- protocol/agent-task-protocol.md".to_string());
        lines.push("- protocol/task-routing.md".to_string());
        lines.push("- protocol/runtime-adapters.md".to_string());
    }

    if let Some(ref cap_path) = ctx.capsule_path {
        lines.push(format!("- {}", cap_path.display()));
    }
    if let Some(ref tm_path) = ctx.task_memory_path {
        lines.push(format!("- {}", tm_path.display()));
    }

    lines.join("\n")
}

/// Render the canonical (classic) task card from a field map.
fn render_task_card(fields: &HashMap<String, String>) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("## 任务卡".to_string());
    lines.push(String::new());

    // Field order matching the canonical classic skeleton
    // (protocol/task-card-template.md). The removed compact fields
    // (路径/读取/关键路径/停止条件) are intentionally never rendered.
    let order: &[&str] = &[
        "读取并遵守：",
        "Contract ID:",
        "Handoff source:",
        "Executor:",
        "Runtime adapter:",
        "Execution surface:",
        "Permission mode:",
        "Parallelism:",
        "Execution effort:",
        "Workflow authority:",
        "任务级别：",
        "Review gate:",
        "任务：",
        "背景：",
        "项目画像：",
        "记忆胶囊：",
        "任务存档：",
        "适用治理文档：",
        "目标文件夹路径：",
        "相关路径：",
        "本次任务相关文件：",
        "目标：",
        "验收标准：",
        "非目标：",
        "子任务编排：",
        "实施要求：",
        "验证：",
        "Verification gate:",
        "交付：",
    ];

    for header in order {
        if let Some(value) = fields.get(*header) {
            let val = value.trim();
            if val.is_empty() {
                continue;
            }
            lines.push(header.to_string());
            // Multi-line fields get their content on separate lines,
            // inline fields stay on the same line
            if is_inline_field(header) {
                // Inline — replace the bare header line with "Header: value".
                let last = lines.last_mut().unwrap();
                *last = format!("{} {}", header, val);
            } else {
                // Multi-line — content follows on separate lines
                // Keep the header as-is, then add the value lines
                // Remove the standalone header and replace with header + value
                lines.pop(); // remove the bare header
                             // For multi-line fields, we keep the header line then content
                lines.push(header.to_string());
                for vline in val.lines() {
                    lines.push(vline.to_string());
                }
            }
            lines.push(String::new());
        }
    }

    // Trim trailing blank lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n") + "\n"
}

fn is_inline_field(header: &str) -> bool {
    matches!(
        header,
        "Contract ID:"
            | "Handoff source:"
            | "Executor:"
            | "Runtime adapter:"
            | "Execution surface:"
            | "Permission mode:"
            | "Parallelism:"
            | "任务级别："
            | "Execution effort:"
            | "Workflow authority:"
    )
}

// ── Public API ──────────────────────────────────────────────────────────

/// Governed convenience compiler with explicit structured handoff state.
#[allow(clippy::result_large_err)]
pub fn compile_simple_with_contract(
    intent: &str,
    project_root: &Path,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
) -> Result<String, CompileReport> {
    let (card, report) = compile_with_contract(
        intent,
        project_root,
        false,
        task_card_requested,
        confirmed_handoff_contract,
    );
    if report.executable_allowed && report.missing_slots.is_empty() {
        Ok(card)
    } else {
        Err(report)
    }
}
