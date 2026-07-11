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
                     host boundaries, and stop conditions. \
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
                    "Guide the user through AGS solution formation: understand the \
                     request, gather relevant project context, present the solution, \
                     and wait for user confirmation. Reminds that \"方案 OK\" \
                     authorizes neither mutation nor a task card; the next explicit \
                     instruction selects direct edit or task-card handoff."
                        .to_string(),
                ),
                arguments: Some(vec![PromptArgument {
                    name: "user_request".to_string(),
                    description: Some(
                        "The user's development request or requirement summary".to_string(),
                    ),
                    required: Some(true),
                }]),
            },
            PromptDef {
                name: "ags_task_card_request_gate".to_string(),
                description: Some(
                    "Enforce the task-card handoff gate. After solution confirmation, \
                     distinguish same-session direct execution authorization from an \
                     explicit task-card/handoff instruction. Without the latter, card output is blocked with \
                     `executable_allowed=false, block_reason=task_card_not_requested`."
                        .to_string(),
                ),
                arguments: None,
            },
            PromptDef {
                name: "ags_delivery_report".to_string(),
                description: Some(
                    "Guide the executor to produce a valid AGS delivery report. \
                     Output must be one copyable Markdown fenced block. Required \
                     sections: task status, one-line conclusion, changed files, new \
                     outputs, deleted files, verification results, risk notes, next steps."
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
             host boundaries, and stop conditions."
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
         1. **Understand the request**. Clarify ambiguities. Diagnose if it describes a problem.\n\
         2. **Read context capsule and task memory** (AGS preflight should have surfaced paths).\n\
         3. **Read relevant project docs surfaced by preflight**. Keep context gathering \
         explicit and source-backed.\n\
         4. **Form a concrete solution** — not a task card. Include: approach, impact scope, \
         risks, alternatives considered.\n\
         5. **Present the solution to the user** and wait for explicit confirmation (\"方案 OK\").\n\
         6. **Do NOT mutate or generate a task card from confirmation alone.** After \
         \"方案 OK\", wait for an explicit instruction selecting same-session direct edit \
         or task-card handoff.\n\n\
         ### Solution text must include\n\n\
         - Problem understanding and diagnosis\n\
         - Proposed approach with rationale\n\
         - Impact scope and blast radius\n\
         - Risks and mitigations\n\
         - Alternatives considered\n\n\
         ### Key rules\n\n\
         - Do NOT classify as Light/Medium/Heavy yet.\n\
         - Do NOT generate a task card yet.\n\
         - \"方案 OK\" authorizes neither mutation nor a task card.\n\n\
         ### Next phase\n\n\
         After user confirmation, use `ags_solution_check` or \
         `ags_task_card_request_gate` to distinguish direct execution authorization from \
         task-card handoff.",
        user_request
    );

    PromptGetResult {
        description: Some(
            "Guide through AGS solution formation phase — understand, gather context, \
             form solution, present, wait for confirmation."
                .to_string(),
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
        assert!(text.contains("\n````\n\n### Requirements:"));
    }
}
