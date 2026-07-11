use super::build_policy_input;
use super::policy::format_policy_text;
use crate::cli::GateAction;
use std::path::{Path, PathBuf};

/// Shared dispatch: `gate check` — always outputs structured JSON even on
/// validation failure (decision=stop with error details).
fn cmd_gate_check(path: &str, format: &str, approve_writes: bool, current_task_approval: bool) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            let err_output = execution_policy::gate_check_failed(
                "read_error",
                vec![format!("Failed to read stdin: {}", e)],
            );
            output_gate_result(&err_output, &display_path, format);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                let err_output = execution_policy::gate_check_failed(
                    "read_error",
                    vec![format!("Failed to read {}: {}", display_path, e)],
                );
                output_gate_result(&err_output, &display_path, format);
                std::process::exit(1);
            }
        }
    };

    // Validate
    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            let err_output =
                execution_policy::gate_check_failed("validation_failed", errors.clone());
            output_gate_result(&err_output, &display_path, format);
            // Write validation errors to stderr for visibility
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
            std::process::exit(1);
        }
    };

    // Resolve and gate check
    let input = build_policy_input(&card.fields, approve_writes, current_task_approval);
    let output = execution_policy::gate_check(&input);

    match format {
        "json" => match serde_json::to_string_pretty(&output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_gate_check_text(&output, &display_path));
        }
    }

    // Exit code IS the gate contract for callers that gate on process status:
    //   allow → 0 (proceed under the resolved two-mode execution policy)
    //   stop  → 1 (blocked / validation failure)
    match output.decision {
        execution_policy::GateDecision::Stop => std::process::exit(1),
        execution_policy::GateDecision::Allow => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExistingTaskCardState {
    RawRequest,
    Valid,
    Invalid(Vec<String>),
}

/// Discriminate a canonical task-card payload before running the natural-language
/// request classifier. The canonical header is intentionally the boundary: raw
/// requests that merely mention task cards keep their existing classifier path,
/// while a payload claiming to be a card must validate or fail closed.
fn task_card_entry_state(input: &str) -> ExistingTaskCardState {
    if !task_card_validator::output_is_canonical_header(input) {
        return ExistingTaskCardState::RawRequest;
    }

    match task_card_validator::parse_validated(input) {
        Ok(_) => ExistingTaskCardState::Valid,
        Err(errors) => ExistingTaskCardState::Invalid(errors),
    }
}

fn existing_task_card_decision(
    state: &ExistingTaskCardState,
    preflight_should_stop: bool,
) -> Option<(&'static str, Option<&'static str>)> {
    match state {
        ExistingTaskCardState::RawRequest => None,
        ExistingTaskCardState::Invalid(_) => Some(("stop", Some("validation_failed"))),
        ExistingTaskCardState::Valid if preflight_should_stop => {
            Some(("stop", Some("preflight_failed")))
        }
        ExistingTaskCardState::Valid => Some(("execute_task_card", None)),
    }
}

/// Compute the prompt-request gate `decision` + `block_reason`. Deliberately a
/// PURE function of the preflight + classification signals only — it takes NO
/// `capability_route` (and no `value_route`), so an advisory / degraded /
/// not-enrolled Capability Route can never change the gate conclusion. The
/// decoupling is enforced at the type level by this parameter list.
fn prompt_request_decision(
    preflight_should_stop: bool,
    is_task_card_request: bool,
    detected_advisory_intent: bool,
    mutation_allowed: bool,
) -> (&'static str, Option<&'static str>) {
    if preflight_should_stop {
        ("stop", Some("preflight_failed"))
    } else if is_task_card_request {
        ("require_task_card", None)
    } else if detected_advisory_intent && !mutation_allowed {
        ("advisory_no_mutation", Some("advisory_intent_no_mutation"))
    } else {
        ("allow", None)
    }
}
/// Shared dispatch: `gate prompt-request` — deterministic entry intent gate.
fn cmd_gate_prompt_request(
    request_arg: &str,
    target: &Path,
    for_agent: &str,
    no_preflight: bool,
    format: &str,
) {
    use std::io::Read;

    let request = if request_arg == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("gate prompt-request: 读取失败 — {}", e);
            std::process::exit(1);
        }
        buf
    } else {
        request_arg.to_string()
    };

    let route_target = capability_route_root(target);
    let (preflight_ran, preflight_should_stop, preflight_status) = if no_preflight {
        (false, false, "skipped".to_string())
    } else {
        match project_discovery::AgentType::from_str(for_agent) {
            Ok(agent) => {
                let pf = project_discovery::run_session_preflight(&route_target, &agent);
                (true, pf.should_stop, format!("{:?}", pf.overall_status))
            }
            Err(_) => (false, false, "skipped".to_string()),
        }
    };

    // A complete task card is already past solution formation and compilation.
    // Validate it before the keyword classifier sees its body. Invalid payloads
    // that claim the canonical header fail closed and never fall back to the
    // new-card generation route.
    let task_card_state = task_card_entry_state(&request);
    if let Some((decision, block_reason)) =
        existing_task_card_decision(&task_card_state, preflight_should_stop)
    {
        let (entry_kind, task_card_valid, validation_errors, next_step) =
            match &task_card_state {
                ExistingTaskCardState::Valid if preflight_should_stop => (
                    "existing_task_card",
                    true,
                    Vec::new(),
                    "AGS preflight reports should_stop — resolve project/protocol health before executing the validated task card.",
                ),
                ExistingTaskCardState::Valid => (
                    "existing_task_card",
                    true,
                    Vec::new(),
                    "Existing canonical task card validated. Skip generation and continue with `ags policy resolve <task-card>` followed by `ags run <task-card>`.",
                ),
                ExistingTaskCardState::Invalid(errors) => (
                    "invalid_task_card",
                    false,
                    errors.clone(),
                    "Fix the task-card validation errors and resubmit this card; do not treat it as a new-card request.",
                ),
                ExistingTaskCardState::RawRequest => unreachable!(),
            };

        match format {
            "json" => {
                let out = serde_json::json!({
                    "gate": "prompt_request",
                    "entry_kind": entry_kind,
                    "decision": decision,
                    "block_reason": block_reason,
                    "task_card_valid": task_card_valid,
                    "task_card_generation_required": false,
                    "next_command": if task_card_valid {
                        Some("ags policy resolve")
                    } else {
                        None
                    },
                    "validation_errors": validation_errors,
                    "is_task_card_request": false,
                    "preflight": {
                        "ran": preflight_ran,
                        "should_stop": preflight_should_stop,
                        "status": preflight_status,
                    },
                    "next_step": next_step,
                });
                match serde_json::to_string_pretty(&out) {
                    Ok(s) => println!("{}", s),
                    Err(e) => {
                        eprintln!("JSON serialization error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            _ => {
                println!("Gate: prompt-request");
                println!("Entry kind: {}", entry_kind);
                println!("Decision: {}", decision);
                println!("Task-card valid: {}", task_card_valid);
                if preflight_ran {
                    println!(
                        "Preflight: status={} should_stop={}",
                        preflight_status, preflight_should_stop
                    );
                }
                if let Some(reason) = block_reason {
                    println!("Block reason: {}", reason);
                }
                for error in &validation_errors {
                    println!("  - {}", error);
                }
                println!("Next: {}", next_step);
            }
        }

        if decision == "stop" {
            std::process::exit(1);
        }
        return;
    }

    let classification = prompt_request_classifier::classify(&request);
    let direct_execution_authorized =
        prompt_request_classifier::detect_current_task_approval(&request)
            && !classification.is_task_card_request;

    // Value Route (效价比路由): minimal execution-path form for this request.
    // Advisory and deterministic. The entry gate distinguishes current direct
    // execution authorization from task-card/handoff intent without changing
    // task level, permission, or independent gates.
    let value_route = prompt_request_classifier::derive_value_route(
        &classification,
        false,
        direct_execution_authorized,
    );

    // Capability Route (能力路由): advisory wakeup suggestion for the request's
    // demand, for the active host, read from the manifest root resolved from
    // `target` (or any subdirectory of it). Parallel to Value Route. Advisory and
    // additive — it never changes `decision`, `block_reason`, the task level,
    // permission mode, Review gate, or Verification gate, and never auto-invokes.
    let capability_route = capability_route::route_request(&request, &route_target, for_agent);

    let (decision, block_reason): (&str, Option<&str>) = prompt_request_decision(
        preflight_should_stop,
        classification.is_task_card_request,
        classification.detected_advisory_intent,
        classification.mutation_allowed,
    );

    let next_step = match decision {
        "stop" => {
            "AGS preflight reports should_stop — resolve project/protocol health before generating any task card."
        }
        "require_task_card" => {
            "Task-card/prompt request detected. Route through AGS preflight → `ags task compile --task-card-requested` → `ags gate output`; the foreground answer MUST be a canonical `## 任务卡`."
        }
        "advisory_no_mutation" => {
            "Advisory/consultation intent detected. Host may perform preflight, read-only retrieval, diagnosis, solution formation, and risk explanation, but must NOT perform write-type tool calls, dependency installs, or implementation. Explicit execution authorization required to clear this block."
        }
        _ if direct_execution_authorized => {
            "Explicit same-session direct execution authorization detected. Proceed with host-native editing and verification without compiling a task card; independent stop conditions still apply."
        }
        _ => "No task-card/prompt request or direct execution authorization detected. An ordinary prose answer is allowed.",
    };

    match format {
        "json" => {
            let mut out = serde_json::json!({
                "gate": "prompt_request",
                "decision": decision,
                "block_reason": block_reason,
                "is_task_card_request": classification.is_task_card_request,
                "detected_advisory_intent": classification.detected_advisory_intent,
                "mutation_allowed": classification.mutation_allowed,
                "direct_execution_authorized": direct_execution_authorized,
                "classification": serde_json::to_value(&classification)
                    .unwrap_or(serde_json::Value::Null),
                "preflight": {
                    "ran": preflight_ran,
                    "should_stop": preflight_should_stop,
                    "status": preflight_status,
                },
                "value_route": serde_json::to_value(&value_route)
                    .unwrap_or(serde_json::Value::Null),
                "capability_route": serde_json::to_value(&capability_route)
                    .unwrap_or(serde_json::Value::Null),
                "next_step": next_step,
            });
            if !classification.advisory_override_triggers.is_empty() {
                out["advisory_override_triggers"] =
                    serde_json::to_value(&classification.advisory_override_triggers)
                        .unwrap_or(serde_json::Value::Null);
            }
            match serde_json::to_string_pretty(&out) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Gate: prompt-request");
            println!("Decision: {}", decision);
            println!("Detected kind: {}", classification.kind.as_str());
            println!("Task-card request: {}", classification.is_task_card_request);
            println!(
                "Direct execution authorized: {}",
                direct_execution_authorized
            );
            if classification.detected_advisory_intent {
                println!(
                    "Advisory intent: detected (mutation_allowed={})",
                    classification.mutation_allowed
                );
            }
            if !classification.matched_triggers.is_empty() {
                println!(
                    "Matched triggers: {}",
                    classification.matched_triggers.join(", ")
                );
            }
            if !classification.advisory_override_triggers.is_empty() {
                println!(
                    "Override triggers: {}",
                    classification.advisory_override_triggers.join(", ")
                );
            }
            if preflight_ran {
                println!(
                    "Preflight: status={} should_stop={}",
                    preflight_status, preflight_should_stop
                );
            }
            if let Some(r) = block_reason {
                println!("Block reason: {}", r);
            }
            println!(
                "Value route: {} (user confirmation: {})",
                value_route.recommended_path.as_str(),
                if value_route.requires_user_confirmation {
                    "required"
                } else {
                    "not required"
                }
            );
            println!(
                "Capability route: demand={} host={} status={} (advisory — does not change decision or any gate)",
                capability_route.demand_kind.as_str(),
                capability_route.active_host,
                val_str(serde_json::to_value(capability_route.status)),
            );
            match &capability_route.primary {
                Some(p) => println!("  primary: {}", p),
                None => println!("  primary: (none)"),
            }
            if let Some(subroute) = &capability_route.subroute {
                println!(
                    "  subroute: {} [{}]",
                    subroute.family,
                    subroute.selected_capabilities.join(", ")
                );
            }
            for rec in &capability_route.recommendations {
                println!(
                    "  - {} [{} → {}] priority={} {}{}",
                    rec.capability_name,
                    val_str(serde_json::to_value(rec.availability)),
                    val_str(serde_json::to_value(rec.route_action)),
                    rec.route_priority,
                    rec.invoke_hint,
                    if rec.is_compatibility_alias {
                        " (alias)"
                    } else {
                        ""
                    }
                );
            }
            if !capability_route.fallback.is_empty() {
                println!("  fallback: {}", capability_route.fallback);
            }
            println!("Next: {}", next_step);
        }
    }

    if decision == "stop" {
        std::process::exit(1);
    }
}
/// Shared dispatch: `gate output` — frontstage output-shape gate.
fn cmd_gate_output(path: &str, for_request: Option<&str>, format: &str) {
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

    // Distinguish a bad foreground shape (not even a `## 任务卡`) from a card that
    // claims to be one but fails the canonical validator. Both are blocked; the
    // block_reason differs so governance_miss samples are actionable.
    let shape_ok = task_card_validator::output_is_canonical_header(&content);
    let (decision, block_reason, stage, validation_errors): (
        &str,
        Option<&str>,
        &str,
        Vec<String>,
    ) = if !shape_ok {
        ("stop", Some("bad_output_shape"), "output_shape", Vec::new())
    } else {
        let errs = task_card_validator::validate(&content);
        if errs.is_empty() {
            ("allow", None, "", Vec::new())
        } else {
            ("stop", Some("validation_failed"), "validate", errs)
        }
    };

    let governance_miss = block_reason.map(|reason| {
        prompt_request_classifier::GovernanceMiss::new(reason, stage, &content, for_request)
    });

    match format {
        "json" => {
            let out = serde_json::json!({
                "gate": "output",
                "decision": decision,
                "block_reason": block_reason,
                "validation_errors": validation_errors,
                "governance_miss": governance_miss
                    .as_ref()
                    .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null)),
            });
            match serde_json::to_string_pretty(&out) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Gate: output");
            println!("Path: {}", display_path);
            println!("Decision: {}", decision);
            if let Some(r) = block_reason {
                println!("Block reason: {}", r);
            }
            for e in &validation_errors {
                println!("  - {}", e);
            }
            if let Some(m) = &governance_miss {
                println!(
                    "governance_miss: detected_kind={} reason={} stage={}",
                    m.detected_kind, m.blocked_reason, m.stage
                );
            }
        }
    }

    if decision == "stop" {
        std::process::exit(1);
    }
}
/// Output a gate result (GateCheckOutput or GateErrorOutput) in the requested format.
fn output_gate_result(
    error_output: &execution_policy::GateErrorOutput,
    display_path: &str,
    format: &str,
) {
    match format {
        "json" => match serde_json::to_string_pretty(error_output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
            }
        },
        _ => {
            println!("Gate Decision: stop");
            println!("Path: {}", display_path);
            println!("Error: {}", error_output.error_kind);
            for (i, err) in error_output.errors.iter().enumerate() {
                println!("  {}. {}", i + 1, err);
            }
        }
    }
}
/// Format a GateCheckOutput as human-readable text.
fn format_gate_check_text(
    output: &execution_policy::GateCheckOutput,
    display_path: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Gate Decision: {}", output.decision));
    lines.push(format!("Task card:     {}", display_path));
    lines.push(format!("Schema:        {}", output.schema_version));
    lines.push(String::new());
    lines.push(format_policy_text(&output.resolved_policy));
    lines.join("\n")
}

/// Render a Serialize-able unit enum as its kebab-case string (via JSON), for
/// the human text branch. Empty on any serialization hiccup.
fn val_str(v: Result<serde_json::Value, serde_json::Error>) -> String {
    v.ok()
        .and_then(|x| x.as_str().map(String::from))
        .unwrap_or_default()
}

/// Resolve the manifest root for capability routing from an explicit `target`.
/// Normalizes via `guard_path` (canonicalize) first, then delegates the ancestor
/// walk to the shared `capability_route::locate_manifest_root` so the CLI and the
/// MCP `ags_solution_check` resolve the manifest root identically. Walking up from
/// the target (not the process cwd) keeps a subdirectory invocation from
/// spuriously reporting `no-capability-for-demand`.
fn capability_route_root(target: &Path) -> PathBuf {
    capability_route::locate_manifest_root(&crate::context::guard_path(target))
}

/// Shared dispatch: `gate capability-request` — hidden minimal entry for the
/// deterministic advisory Capability Route. Builds the inventory for the active
/// host, derives the route, and prints it. The same route is also surfaced by
/// `ags_solution_check` and `gate prompt-request`. Advisory only; never blocks.
fn cmd_gate_capability_request(request_arg: &str, target: &Path, for_agent: &str, format: &str) {
    use std::io::Read;

    let request = if request_arg == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("gate capability-request: 读取失败 — {}", e);
            std::process::exit(1);
        }
        buf
    } else {
        request_arg.to_string()
    };

    // Shared wiring: locate the manifest root from the explicit target, build the
    // inventory for the active host, and derive the advisory route.
    let root = capability_route_root(target);
    let route = capability_route::route_request(&request, &root, for_agent);

    match format {
        "json" => match serde_json::to_string_pretty(&route) {
            Ok(s) => println!("{}", s),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("Gate: capability-request");
            println!("Demand: {}", route.demand_kind.as_str());
            println!("Active host: {}", route.active_host);
            println!("Status: {}", val_str(serde_json::to_value(route.status)));
            match &route.primary {
                Some(p) => println!("Primary: {}", p),
                None => println!("Primary: (none)"),
            }
            if let Some(subroute) = &route.subroute {
                println!(
                    "Subroute: {} [{}]",
                    subroute.family,
                    subroute.selected_capabilities.join(", ")
                );
            }
            for rec in &route.recommendations {
                println!(
                    "  - {} [{} → {}] priority={} {}{}",
                    rec.capability_name,
                    val_str(serde_json::to_value(rec.availability)),
                    val_str(serde_json::to_value(rec.route_action)),
                    rec.route_priority,
                    rec.invoke_hint,
                    if rec.is_compatibility_alias {
                        " (alias)"
                    } else {
                        ""
                    }
                );
            }
            if !route.fallback.is_empty() {
                println!("Fallback: {}", route.fallback);
            }
            println!("Advisory: {}", route.advisory);
        }
    }
    // Advisory: never a blocking exit code.
}

/// Shared dispatch: `gate skill-tags` — runtime availability gate for a task
/// card's trailing `[skill: …]` tags. Static gate (registry routable + legal
/// invoke_hint) is enforced by the validator at compile time; this adds the live
/// snapshot availability check. A rejected tag → decision = stop (exit 1).
fn cmd_gate_skill_tags(path: &str, target: &Path, for_agent: &str, format: &str) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };
    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("gate skill-tags: 读取失败 — {}", e);
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

    let tags = task_card_validator::extract_skill_tags(&content);
    let root = capability_route_root(target);
    let gate = capability_route::verify_skill_tags(&tags, &root, for_agent);
    let decision = if gate.all_accepted { "allow" } else { "stop" };

    match format {
        "json" => {
            let out = serde_json::json!({
                "gate": "skill_tags",
                "decision": decision,
                "active_host": gate.active_host,
                "snapshot_hash": gate.snapshot_hash,
                "all_accepted": gate.all_accepted,
                "rejected": gate.rejected,
                "verdicts": serde_json::to_value(&gate.verdicts).unwrap_or(serde_json::Value::Null),
            });
            match serde_json::to_string_pretty(&out) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Gate: skill-tags");
            println!("Path: {}", display_path);
            println!("Active host: {}", gate.active_host);
            println!("Snapshot: {}", gate.snapshot_hash);
            println!("Decision: {}", decision);
            if tags.is_empty() {
                println!("  (no [skill: …] tags found)");
            }
            for v in &gate.verdicts {
                println!(
                    "  - [skill: {}] {} (routable={}, availability={})",
                    v.tag,
                    if v.accepted { "ACCEPT" } else { "REJECT" },
                    v.registry_routable,
                    val_str(serde_json::to_value(v.availability)),
                );
                if !v.accepted {
                    println!("      {}", v.reason);
                }
            }
        }
    }

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
        GateAction::PromptRequest {
            request,
            target,
            for_agent,
            no_preflight,
            format,
        } => cmd_gate_prompt_request(&request, &target, &for_agent, no_preflight, &format),
        GateAction::Output {
            path,
            for_request,
            format,
        } => cmd_gate_output(&path, for_request.as_deref(), &format),
        GateAction::CapabilityRequest {
            request,
            target,
            for_agent,
            format,
        } => cmd_gate_capability_request(&request, &target, &for_agent, &format),
        GateAction::SkillTags {
            path,
            target,
            for_agent,
            format,
        } => cmd_gate_skill_tags(&path, &target, &for_agent, &format),
    }
}

#[cfg(test)]
mod capability_request_tests {
    use super::{
        capability_route_root, existing_task_card_decision, prompt_request_decision,
        task_card_entry_state, ExistingTaskCardState,
    };

    fn valid_heavy_card(permission_mode: &str) -> String {
        let card = include_str!("../../../../tests/fixtures/valid-full.md")
            .replace("任务级别：Light", "任务级别：Heavy")
            .replace(
                "Permission mode: execute-and-verify",
                &format!("Permission mode: {permission_mode}"),
            )
            .replace(
                "- Light review",
                "- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。",
            );
        if permission_mode == "plan-only" {
            card.replace(
                "交付：\n按协议输出测试通过结果",
                "交付：\n返回用户/Codex 审阅，等待明确批准，不得直接修改。",
            )
        } else {
            card
        }
    }

    #[test]
    fn entry_gate_recognizes_valid_existing_cards_before_prompt_classification() {
        for mode in ["plan-only", "execute-and-verify"] {
            let card = valid_heavy_card(mode);
            assert_eq!(
                task_card_entry_state(&card),
                ExistingTaskCardState::Valid,
                "valid existing {mode} card must bypass the generation classifier"
            );
            assert_eq!(
                existing_task_card_decision(&ExistingTaskCardState::Valid, false),
                Some(("execute_task_card", None)),
                "valid existing {mode} card must route to execution"
            );
        }
    }

    #[test]
    fn entry_gate_fails_closed_for_invalid_card_shaped_input() {
        let input = "## 任务卡\n\nExecutor: Codex\nPermission mode: execute-and-verify\n";
        let ExistingTaskCardState::Invalid(errors) = task_card_entry_state(input) else {
            panic!("card-shaped invalid input must be rejected as a card");
        };
        assert!(!errors.is_empty());
        assert_eq!(
            existing_task_card_decision(&ExistingTaskCardState::Invalid(errors), false),
            Some(("stop", Some("validation_failed")))
        );
    }

    #[test]
    fn entry_gate_leaves_raw_new_card_requests_on_classifier_path() {
        assert_eq!(
            task_card_entry_state("按这个方案生成一张任务卡"),
            ExistingTaskCardState::RawRequest
        );
        assert_eq!(
            existing_task_card_decision(&ExistingTaskCardState::RawRequest, false),
            None
        );
    }

    /// The gate decision is computed only from preflight + classification — never
    /// from `capability_route`. `prompt_request_decision` takes no route argument,
    /// so a degraded / not-enrolled Capability Route cannot change `decision` or
    /// `block_reason`. This locks the decoupling the user required.
    #[test]
    fn prompt_request_decision_is_decoupled_from_capability_route() {
        // preflight stop wins over everything.
        assert_eq!(
            prompt_request_decision(true, false, false, true),
            ("stop", Some("preflight_failed"))
        );
        // task-card request.
        assert_eq!(
            prompt_request_decision(false, true, false, true),
            ("require_task_card", None)
        );
        // advisory intent with mutation NOT allowed.
        assert_eq!(
            prompt_request_decision(false, false, true, false),
            ("advisory_no_mutation", Some("advisory_intent_no_mutation"))
        );
        // advisory intent but mutation allowed → allow.
        assert_eq!(
            prompt_request_decision(false, false, true, true),
            ("allow", None)
        );
        // ordinary prose → allow. (Same inputs a debug/docs demand would carry —
        // the route status is irrelevant because it is not an input here.)
        assert_eq!(
            prompt_request_decision(false, false, false, true),
            ("allow", None)
        );
    }

    #[test]
    fn capability_route_root_uses_explicit_target_not_calling_cwd() {
        let base = std::env::temp_dir().join(format!(
            "ags-capability-route-target-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let child = repo.join("crates/ags-cli");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(repo.join("manifests")).unwrap();
        std::fs::write(repo.join("manifests/skills-registry.yaml"), "skills: []\n").unwrap();
        std::fs::write(repo.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();

        assert_eq!(capability_route_root(&repo), repo.canonicalize().unwrap());
        assert_eq!(capability_route_root(&child), repo.canonicalize().unwrap());

        let _ = std::fs::remove_dir_all(&base);
    }
}
