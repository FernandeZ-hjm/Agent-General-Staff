//! AGS MCP Prompts — governance kernel prompts for agent hosts.
//!
//! Prompts are short, executable instruction templates that guide
//! MCP hosts through key AGS lifecycle phases. They are intentionally
//! concise — hosts should reference the full protocol resources
//! via `ags://` URIs rather than duplicating all protocol text.
//!
//! # Initialization Gate
//!
//! `ags_global_kernel` prompt leads with the mandatory initialization gate.
//! Hosts must call `ags_preflight` (or CLI fallback) before any other AGS
//! tool — prompts for later phases (solution, routing, delivery) assume
//! preflight has already completed.

use crate::protocol::{
    PromptArgument, PromptContent, PromptDef, PromptGetResult, PromptListResult, PromptMessage,
};

// ── Prompt Definitions ───────────────────────────────────────────────────────

/// Generate MCP `prompts/list` response with all available prompts.
pub fn list_prompts() -> PromptListResult {
    PromptListResult {
        prompts: vec![
            PromptDef {
                name: "ags_global_kernel".to_string(),
                description: Some(
                    "Load the AGS global governance kernel — initialization gate \
                     (call ags_preflight FIRST), mandatory lifecycle, critical rules, \
                     host boundaries and stop conditions. \
                     Best loaded at session start or when the host first encounters \
                     a development-related request. The initialization gate is \
                     non-negotiable: MCP preflight or CLI fallback must complete \
                     before any other AGS tool or lifecycle phase."
                        .to_string(),
                ),
                arguments: None,
            },
            PromptDef {
                name: "ags_solution_phase".to_string(),
                description: Some(
                    "Guide solution formation only after the typed RouteResolution \
                     shows unresolved design work. Keep context host-owned, \
                     context, and do not turn solution confirmation into task-card authority."
                        .to_string(),
                ),
                arguments: Some(vec![PromptArgument {
                    name: "user_request".to_string(),
                    description: Some(
                        "The complete current request context the host uses to form a typed proposal."
                            .to_string(),
                    ),
                    required: Some(true),
                }]),
            },
            PromptDef {
                name: "ags_task_card_request_gate".to_string(),
                description: Some(
                    "Enforce the task-card handoff gate. Distinguish same-session \
                     execution from handoff. Explicit handoff requires a task-card \
                     instruction; host Plan-mode finalization uses the canonical card \
                     itself. Both require structured confirmed-contract evidence. \
                     Missing or reopened solution state blocks card output."
                        .to_string(),
                ),
                arguments: None,
            },
            PromptDef {
                name: "ags_delivery_report".to_string(),
                description: Some(
                    "Guide the executor to close an AGS task card with a \
                     machine-checkable delivery report. Bind Contract ID, task-card \
                     hash, receipt ID, and every G/AC/V item, then run `ags task close`."
                        .to_string(),
                ),
                arguments: None,
            },
        ],
    }
}

/// Get a specific prompt by name with optional arguments.
pub fn get_prompt(name: &str, _arguments: &serde_json::Value) -> Result<PromptGetResult, String> {
    match name {
        "ags_global_kernel" => Ok(prompt_global_kernel()),
        "ags_solution_phase" => Ok(prompt_solution_phase(_arguments)),
        "ags_task_card_request_gate" => Ok(prompt_task_card_request_gate()),
        "ags_delivery_report" => Ok(prompt_delivery_report()),
        other => Err(format!("Unknown prompt: {}", other)),
    }
}

// ── Prompt Content Providers ─────────────────────────────────────────────────

fn prompt_global_kernel() -> PromptGetResult {
    PromptGetResult {
        description: Some(
            "AGS global governance kernel — load at session start or upon first \
             development request. Leads with mandatory initialization gate \
             (call ags_preflight FIRST), then establishes lifecycle, critical rules, \
             host boundaries and stop conditions."
                .to_string(),
        ),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent {
                r#type: "text".to_string(),
                text: include_str!("prompts/global_kernel.txt").to_string(),
            },
        }],
    }
}

fn prompt_solution_phase(arguments: &serde_json::Value) -> PromptGetResult {
    let user_request = arguments
        .get("user_request")
        .and_then(|v| v.as_str())
        .unwrap_or("(no user request provided)");

    let text = format!(
        "## AGS Solution Phase\n\n\
         **User request**: {}\n\n\
         ### Instructions\n\n\
         0. **Resolve once first**: read `ags://capabilities/current-host`, use complete \
         host-owned context to form a typed HostRouteProposal, then call read-only \
         `ags_route_request`. Continue only when the proposal phase is solution_formation. \
         DirectResponse delivers and stops; held machine actions require explicit apply.\n\
         1. **Understand unresolved decisions**. Clarify ambiguities. Diagnose if needed.\n\
         2. **Read context capsule and task memory** (AGS preflight should have surfaced paths).\n\
         3. **Use only explicitly available methods**; external advice cannot override AGS gates.\n\
         4. **Form a concrete solution**. Outside host Plan mode this is not yet a task card. \
         Include: approach, impact scope, risks, alternatives considered.\n\
         5. **Close the contract**. Outside host Plan mode, present the solution and wait for \
         explicit confirmation. Inside host Plan mode, continue until the implementation \
         contract is decision-complete.\n\
         6. **Finalize by host state**. Outside Plan mode, confirmation alone still does not \
         create a task card. Inside host Plan mode, the final artifact is compiled directly \
         with `--host-plan-mode-final --confirmed-handoff-contract` as the canonical \
         `## 任务卡`; do not create a separate final-plan document.\n\n\
         ### Solution text must include\n\n\
         - Problem understanding and diagnosis\n\
         - Proposed approach with rationale\n\
         - Impact scope and blast radius\n\
         - Risks and mitigations\n\
         - Alternatives considered\n\
         ### Key rules\n\n\
         - Do NOT classify as Light/Medium/Heavy yet.\n\
         - Do NOT generate a task card while material decisions remain open.\n\
         - Outside host Plan mode, \"方案 OK\" authorizes neither mutation nor a task card.\n\
         - Host Plan-mode finalization generates the card but does not authorize mutation; \
           the Plan UI must switch to execution mode before dispatch.\n\
         - AGS is the governance authority.\n\n\
         ### Next phase\n\n\
         After confirmation or Plan-mode contract closure, the host forms a new typed \
         proposal from the updated context. Explicit handoff uses \
         `--task-card-requested`; Plan-mode finalization uses \
         `--host-plan-mode-final`. Both require the confirmed contract gate.",
        user_request
    );

    PromptGetResult {
        description: Some(
            "Guard and guide the conditional AGS solution formation phase.".to_string(),
        ),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent {
                r#type: "text".to_string(),
                text,
            },
        }],
    }
}

fn prompt_task_card_request_gate() -> PromptGetResult {
    PromptGetResult {
        description: Some(
            "Enforce the task-card handoff gate and distinguish it from authorized \
             same-session direct execution."
                .to_string(),
        ),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent {
                r#type: "text".to_string(),
                text: include_str!("prompts/task_card_request_gate.txt").to_string(),
            },
        }],
    }
}

fn prompt_delivery_report() -> PromptGetResult {
    PromptGetResult {
        description: Some(
            "Guide the executor to produce a valid AGS delivery report \
             after task completion as one copyable Markdown fenced block."
                .to_string(),
        ),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent {
                r#type: "text".to_string(),
                text: include_str!("prompts/delivery_report.txt").to_string(),
            },
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_report_prompt_requires_copyable_markdown_block() {
        let prompt = prompt_delivery_report();
        let text = &prompt.messages[0].content.text;

        assert!(text.contains("copyable Markdown fenced block"));
        assert!(text.contains("````markdown\n# 任务交付报告"));
        assert!(text.contains("Closure schema: 1.0"));
        assert!(text.contains("ags task close"));
    }
}
