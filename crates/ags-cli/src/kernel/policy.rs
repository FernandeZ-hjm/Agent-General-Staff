use super::{build_policy_input, read_and_validate_task_card};
use crate::cli::PolicyAction;

/// Format a `ResolvedExecutionPolicy` as human-readable text.
pub(in crate::kernel) fn format_policy_text(
    policy: &execution_policy::ResolvedExecutionPolicy,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Resolved Execution Policy".to_string());
    lines.push("=========================".to_string());
    lines.push(format!("Executor:          {}", policy.executor));
    lines.push(format!("Runtime adapter:   {}", policy.runtime_adapter));
    lines.push(format!(
        "Permission mode:   {}",
        policy.effective_permission_mode
    ));
    lines.push(format!(
        "Parallelism:       {}",
        policy.effective_parallelism
    ));
    lines.push(format!(
        "Exec surface:      {}",
        policy.effective_execution_surface
    ));
    lines.push(format!("Execution effort:  {}", policy.execution_effort));
    lines.push(format!("Exhaustive mode:   {}", policy.is_exhaustive_mode));
    lines.push(String::new());

    let args_str = if policy.allowed_launch_args.is_empty() {
        "(none)".to_string()
    } else {
        policy.allowed_launch_args.join(" ")
    };
    lines.push(format!("Launch args:       {}", args_str));

    lines.push(format!("Stop before launch: {}", policy.stop_before_launch));
    if !policy.stop_reasons.is_empty() {
        lines.push("Stop reasons:".to_string());
        for (i, reason) in policy.stop_reasons.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, reason));
        }
    }

    lines.push(format!(
        "Requires confirmation gate: {}",
        policy.requires_confirmation_gate
    ));
    lines.push(format!("Approval source:   {}", policy.approval_source));
    lines.push(String::new());

    if policy.was_downgraded {
        lines.push("Downgrades:".to_string());
        for (i, reason) in policy.downgrade_reasons.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, reason));
        }
    } else {
        lines.push("Downgrades:        none".to_string());
    }

    lines.join("\n")
}
/// Shared dispatch: `policy resolve` / `resolve-policy`
pub(crate) fn cmd_policy_resolve(
    path: &str,
    format: &str,
    approve_writes: bool,
    current_task_approval: bool,
) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    // Read input (file or stdin)
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

    // Phase 1: validate and parse
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

    // Phase 2: build policy input from parsed fields and structured approval
    // signals. Task-card text is never an approval source.
    let input = build_policy_input(&card.fields, approve_writes, current_task_approval);

    // Phase 3: resolve execution policy
    let policy = execution_policy::resolve_policy(input);

    // Phase 4: output
    match format {
        "json" => match serde_json::to_string_pretty(&policy) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_policy_text(&policy));
        }
    }
}
/// Shared dispatch: `policy explain`
fn cmd_policy_explain(path: &str, format: &str, approve_writes: bool, current_task_approval: bool) {
    let (_, card, display_path) = read_and_validate_task_card(path);
    let input = build_policy_input(&card.fields, approve_writes, current_task_approval);
    let output = execution_policy::explain_policy(&input);

    match format {
        "json" => match serde_json::to_string_pretty(&output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_explain_text(&output, &display_path));
        }
    }
}
/// Shared dispatch: `policy check` — exit 0 if no stop, 1 if stop/validation.
fn cmd_policy_check(path: &str, format: &str, approve_writes: bool, current_task_approval: bool) {
    let (_, card, _display_path) = read_and_validate_task_card(path);
    let input = build_policy_input(&card.fields, approve_writes, current_task_approval);
    let policy = execution_policy::resolve_policy(input);

    match format {
        "json" => match serde_json::to_string_pretty(&policy) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_policy_text(&policy));
        }
    }

    if policy.stop_before_launch {
        std::process::exit(1);
    }
}
/// Format a PolicyExplainOutput as human-readable text.
fn format_explain_text(
    output: &execution_policy::PolicyExplainOutput,
    display_path: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Policy Explanation".to_string());
    lines.push("==================".to_string());
    lines.push(format!("Task card:  {}", display_path));
    lines.push(format!("Schema:     {}", output.schema_version));
    lines.push(format!("Executor:   {}", output.task_summary.executor));
    lines.push(format!("Task level: {}", output.task_summary.task_level));
    lines.push(format!(
        "Permission: {}",
        output.task_summary.permission_mode
    ));
    lines.push(String::new());

    lines.push("Rule-by-Rule Explanation".to_string());
    lines.push("-----------------------".to_string());
    for explanation in &output.explanations {
        let field_note = match &explanation.field {
            Some(f) => format!(" [{}]", f),
            None => String::new(),
        };
        lines.push(format!(
            "  [{}] {} — {}{}",
            explanation.rule_id, explanation.decision, explanation.rule_name, field_note
        ));
        lines.push(format!("        {}", explanation.detail));
    }
    lines.push(String::new());

    lines.push("Safety Assertions".to_string());
    lines.push("-----------------".to_string());
    for (i, assertion) in output.safety_assertions.iter().enumerate() {
        lines.push(format!("  {}. {}", i + 1, assertion));
    }
    lines.push(String::new());

    lines.push("Resolved Execution Policy".to_string());
    lines.push("=========================".to_string());
    lines.push(format_policy_text(&output.resolved_policy));

    lines.join("\n")
}

pub(crate) fn run(action: PolicyAction) {
    match action {
        PolicyAction::Resolve {
            path,
            format,
            approve_writes,
            current_task_approval,
        } => cmd_policy_resolve(&path, &format, approve_writes, current_task_approval),
        PolicyAction::Explain {
            path,
            format,
            approve_writes,
            current_task_approval,
        } => cmd_policy_explain(&path, &format, approve_writes, current_task_approval),
        PolicyAction::Check {
            path,
            format,
            approve_writes,
            current_task_approval,
        } => cmd_policy_check(&path, &format, approve_writes, current_task_approval),
    }
}
