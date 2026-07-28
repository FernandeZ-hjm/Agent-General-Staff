use super::build_policy_input;
use super::policy::format_policy_text;
use crate::cli::GateAction;
use serde::Serialize;
use std::path::Path;

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut content = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut content)
            .map_err(|error| format!("stdin read failed: {error}"))?;
        Ok(content)
    } else {
        std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))
    }
}

fn cmd_gate_check(path: &str, format: &str, approve_writes: bool, current_task_approval: bool) {
    let display_path = if path == "-" { "(stdin)" } else { path };
    let content = read_input(path).unwrap_or_else(|error| {
        let output = ags_governance_decision::policy::gate_check_failed("read_error", vec![error]);
        output_gate_error(&output, display_path, format);
        std::process::exit(1);
    });
    let card = ags_task_contract::validator::parse_validated(&content).unwrap_or_else(|errors| {
        let output =
            ags_governance_decision::policy::gate_check_failed("validation_failed", errors);
        output_gate_error(&output, display_path, format);
        std::process::exit(1);
    });
    let input = build_policy_input(&card.fields, approve_writes, current_task_approval);
    let output = ags_governance_decision::policy::gate_check(&input);
    crate::output::emit(format, &output, || {
        format!(
            "Gate Decision: {}\nTask card:     {display_path}\n\n{}",
            output.decision,
            format_policy_text(&output.resolved_policy)
        )
    });
    if output.decision == ags_governance_decision::policy::GateDecision::Stop {
        std::process::exit(1);
    }
}

fn output_gate_error(
    output: &ags_governance_decision::policy::GateErrorOutput,
    display_path: &str,
    format: &str,
) {
    crate::output::emit(format, output, || {
        let mut lines = vec![
            "Gate Decision: stop".to_string(),
            format!("Path: {display_path}"),
            format!("Error: {}", output.error_kind),
        ];
        lines.extend(output.errors.iter().map(|error| format!("  - {error}")));
        lines.join("\n")
    });
}

#[derive(Debug, Serialize)]
struct GovernanceMiss {
    detected_kind: &'static str,
    blocked_reason: &'static str,
    stage: &'static str,
    sample: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    for_request: Option<String>,
}

fn output_decision(content: &str) -> (&'static str, Option<&'static str>, Vec<String>) {
    if !ags_task_contract::validator::output_is_canonical_header(content) {
        return ("stop", Some("bad_output_shape"), Vec::new());
    }
    let errors = ags_task_contract::validator::validate(content);
    if errors.is_empty() {
        ("allow", None, errors)
    } else {
        ("stop", Some("validation_failed"), errors)
    }
}

fn cmd_gate_output(path: &str, for_request: Option<&str>, format: &str) {
    let content = read_input(path).unwrap_or_else(|error| {
        eprintln!("gate output: {error}");
        std::process::exit(1);
    });
    let (decision, block_reason, validation_errors) = output_decision(&content);
    let governance_miss = block_reason.map(|reason| GovernanceMiss {
        detected_kind: if reason == "bad_output_shape" {
            "non_canonical_task_card"
        } else {
            "invalid_task_card"
        },
        blocked_reason: reason,
        stage: if reason == "bad_output_shape" {
            "output_shape"
        } else {
            "validate"
        },
        sample: content.chars().take(240).collect(),
        for_request: for_request.map(str::to_string),
    });
    let output = serde_json::json!({
        "gate": "output",
        "decision": decision,
        "block_reason": block_reason,
        "validation_errors": validation_errors,
        "governance_miss": governance_miss,
    });
    crate::output::emit(format, &output, || {
        let mut lines = vec!["Gate: output".to_string(), format!("Decision: {decision}")];
        if let Some(reason) = block_reason {
            lines.push(format!("Block reason: {reason}"));
        }
        lines.extend(validation_errors.iter().map(|error| format!("  - {error}")));
        lines.join("\n")
    });
    if decision == "stop" {
        std::process::exit(1);
    }
}

fn cmd_gate_skill_tags(path: &str, _target: &Path, for_agent: &str, format: &str) {
    let content = read_input(path).unwrap_or_else(|error| {
        eprintln!("gate skill-tags: {error}");
        std::process::exit(1);
    });
    let tags = ags_task_contract::validator::extract_skill_tags(&content);
    let gate = ags_capability_governance::verify_skill_tags(&tags, for_agent);
    let decision = if gate.all_accepted { "allow" } else { "stop" };
    let output = serde_json::json!({
        "gate": "skill_tags",
        "decision": decision,
        "active_host": gate.active_host,
        "snapshot_hash": gate.snapshot_hash,
        "all_accepted": gate.all_accepted,
        "rejected": gate.rejected,
        "verdicts": gate.verdicts,
    });
    crate::output::emit(format, &output, || {
        let mut lines = vec![
            "Gate: skill-tags".to_string(),
            format!("Active host: {}", gate.active_host),
            format!("Snapshot: {}", gate.snapshot_hash),
            format!("Decision: {decision}"),
        ];
        for verdict in &gate.verdicts {
            lines.push(format!(
                "  - [skill: {}] {}{}",
                verdict.tag,
                if verdict.accepted { "ACCEPT" } else { "REJECT" },
                if verdict.reason.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", verdict.reason)
                }
            ));
        }
        lines.join("\n")
    });
    if !gate.all_accepted {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: GateAction) {
    match action {
        GateAction::Check {
            path,
            format,
            approve_writes,
            current_task_approval,
        } => cmd_gate_check(&path, &format, approve_writes, current_task_approval),
        GateAction::Output {
            path,
            for_request,
            format,
        } => cmd_gate_output(&path, for_request.as_deref(), &format),
        GateAction::SkillTags {
            path,
            target,
            for_agent,
            format,
        } => cmd_gate_skill_tags(&path, &target, &for_agent, &format),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_gate_rejects_non_card_shape() {
        let (decision, reason, _) = output_decision("普通回复");
        assert_eq!(decision, "stop");
        assert_eq!(reason, Some("bad_output_shape"));
    }
}
