//! Hidden kernel commands (MCP / CI / compatibility surface).

pub(crate) mod awareness;
pub(crate) mod bootstrap;
pub(crate) mod compliance;
pub(crate) mod gate;
pub(crate) mod hooks;
pub(crate) mod mcp;
pub(crate) mod policy;
pub(crate) mod receipt;
pub(crate) mod release;
pub(crate) mod rollback;
pub(crate) mod runner;
pub(crate) mod sync;
pub(crate) mod task;
pub(crate) mod verify;

/// Shared helper: read a task card (file or stdin) and validate+parse it.
/// Returns (content, parsed_fields, display_path) or exits on failure.
pub(in crate::kernel) fn read_and_validate_task_card(
    path: &str,
) -> (String, task_card_validator::ParsedTaskCard, String) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
            std::process::exit(1);
        }
    };

    (content, card, display_path)
}
/// Build a TaskPolicyInput from parsed fields + structured approval signals.
///
/// Approval is decoupled from the task LEVEL and never read from task-card text.
/// Both signals are audit/hint only — the resolver no longer downgrades a card
/// by task level, so a Heavy card is executable from its declared permission
/// mode. `approve_writes` (CLI flag / runner
/// env) may additionally act as the M9 generic-adapter capability override.
/// `current_task_approval` is the host-detected current-task instruction signal
/// (an explicit "实现 / 修复 / 做完" on the live request via
/// `prompt_request_classifier`). The stronger source wins when both are present.
pub(in crate::kernel) fn build_policy_input(
    fields: &std::collections::HashMap<String, String>,
    approve_writes: bool,
    current_task_approval: bool,
) -> execution_policy::TaskPolicyInput {
    execution_policy::TaskPolicyInput::from_fields_with_approval(
        fields,
        approve_writes,
        current_task_approval,
    )
}
