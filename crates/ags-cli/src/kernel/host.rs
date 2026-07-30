use crate::cli::HostAction;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;

const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;

pub(crate) fn run(action: HostAction) {
    match action {
        HostAction::Lifecycle {
            event,
            host,
            target,
            input,
        } => {
            let result = lifecycle(&event, &host, &target, &input).unwrap_or_else(|error| {
                eprintln!("host lifecycle: {error}");
                std::process::exit(1);
            });
            println!(
                "{}",
                serde_json::to_string(&result).expect("lifecycle result is serializable")
            );
        }
    }
}

fn lifecycle(event: &str, host: &str, target: &Path, input: &str) -> Result<Value, String> {
    if ags_host_integration::platform_spec(host).is_none() {
        return Err(format!("unsupported host `{host}`"));
    }
    let payload = read_payload(input)?;
    let envelope =
        ags_lifecycle::workspace_lifecycle::LifecycleEnvelope::new(target, host, event, payload)?;
    let result = ags_session::dispatch_workspace_command(
        target,
        "lifecycle",
        serde_json::to_value(envelope)
            .map_err(|error| format!("lifecycle envelope encode failed: {error}"))?,
    )?;
    let decision: ags_lifecycle::workspace_lifecycle::LifecycleDecision =
        serde_json::from_value(result)
            .map_err(|error| format!("lifecycle decision decode failed: {error}"))?;
    format_lifecycle_decision(host, &decision)
}

fn format_lifecycle_decision(
    host: &str,
    decision: &ags_lifecycle::workspace_lifecycle::LifecycleDecision,
) -> Result<Value, String> {
    let mut response = json!({
        "schema_version": decision.schema_version,
        "workspace_identity": decision.workspace_identity,
        "host": decision.host,
        "host_session_id": decision.host_session_id,
        "event": decision.event,
        "event_id": decision.event_id,
        "status": decision.status,
        "duplicate": decision.duplicate,
    });
    if let Some(reason) = &decision.reason {
        response["reason"] = json!(reason);
    }
    if let Some(archive) = &decision.archive {
        response["archive"] = archive.clone();
    }
    match (decision.event.as_str(), lifecycle_output_protocol(host)?) {
        (
            "session-start",
            ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible
            | ags_host_integration::LifecycleOutputProtocol::CodeBuddy,
        ) => {
            response["suppressOutput"] = json!(true);
            response["hookSpecificOutput"] = json!({
                "hookEventName": "SessionStart",
                "additionalContext": decision.additional_context.clone().unwrap_or_default(),
            });
        }
        ("session-start", ags_host_integration::LifecycleOutputProtocol::Cursor) => {
            response["additional_context"] =
                json!(decision.additional_context.clone().unwrap_or_default());
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible) => {
            let blocked = decision.status == "blocked";
            response["suppressOutput"] = json!(blocked);
            if blocked {
                response["hookSpecificOutput"] = json!({
                    "hookEventName": "Stop",
                    "additionalContext": decision.additional_context.clone().unwrap_or_default(),
                });
            }
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::CodeBuddy) => {
            let blocked = decision.status == "blocked";
            response["continue"] = json!(!blocked);
            response["suppressOutput"] = json!(true);
            if blocked {
                response["reason"] = json!(decision.additional_context.clone().unwrap_or_default());
            }
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::Cursor) => {
            if let Some(message) = &decision.additional_context {
                response["followup_message"] = json!(message);
            }
        }
        ("session-end", _) => {}
        (other, _) => return Err(format!("unsupported lifecycle event `{other}`")),
    }
    Ok(response)
}

fn lifecycle_output_protocol(
    host: &str,
) -> Result<ags_host_integration::LifecycleOutputProtocol, String> {
    ags_host_integration::platform_spec(host)
        .and_then(|spec| spec.lifecycle.map(|lifecycle| lifecycle.output))
        .ok_or_else(|| format!("host `{host}` has no lifecycle output protocol"))
}

fn read_payload(path: &str) -> Result<Value, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin()
            .take((MAX_HOOK_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read hook input: {error}"))?;
    } else {
        bytes = std::fs::read(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    }
    if bytes.len() > MAX_HOOK_INPUT_BYTES {
        return Err("hook input exceeds 1 MiB".to_string());
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid hook JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_host_integration::{lifecycle_specs, LifecycleOutputProtocol, AGENT_PLATFORM_SPECS};
    use ags_lifecycle::workspace_lifecycle::LifecycleDecision;

    fn decision(
        host: &str,
        event: &str,
        status: &str,
        additional_context: Option<&str>,
    ) -> LifecycleDecision {
        LifecycleDecision {
            schema_version: "test-lifecycle-schema".to_string(),
            workspace_identity: "workspace-identity".to_string(),
            host: host.to_string(),
            host_session_id: "host-session".to_string(),
            event: event.to_string(),
            event_id: "event-id".to_string(),
            status: status.to_string(),
            duplicate: false,
            additional_context: additional_context.map(str::to_string),
            reason: None,
            archive: None,
        }
    }

    #[test]
    fn production_formatter_emits_each_registered_hosts_native_start_schema() {
        for lifecycle in lifecycle_specs() {
            let output = format_lifecycle_decision(
                lifecycle.host_id,
                &decision(
                    lifecycle.host_id,
                    "session-start",
                    "ready",
                    Some("memory context"),
                ),
            )
            .unwrap();

            assert_eq!(output["host"], lifecycle.host_id, "{lifecycle:?}");
            assert_eq!(output["schema_version"], "test-lifecycle-schema");
            match lifecycle.output {
                LifecycleOutputProtocol::ClaudeCompatible | LifecycleOutputProtocol::CodeBuddy => {
                    assert_eq!(output["suppressOutput"], true, "{lifecycle:?}");
                    assert_eq!(
                        output["hookSpecificOutput"]["hookEventName"], "SessionStart",
                        "{lifecycle:?}"
                    );
                    assert_eq!(
                        output["hookSpecificOutput"]["additionalContext"], "memory context",
                        "{lifecycle:?}"
                    );
                    assert!(output.get("additional_context").is_none(), "{lifecycle:?}");
                }
                LifecycleOutputProtocol::Cursor => {
                    assert_eq!(
                        output["additional_context"], "memory context",
                        "{lifecycle:?}"
                    );
                    assert!(output.get("hookSpecificOutput").is_none(), "{lifecycle:?}");
                }
            }
        }
    }

    #[test]
    fn production_formatter_omits_clear_stop_objects_and_preserves_blocking_context() {
        for lifecycle in lifecycle_specs() {
            let clear = format_lifecycle_decision(
                lifecycle.host_id,
                &decision(lifecycle.host_id, "stop-guard", "clear", None),
            )
            .unwrap();
            assert_eq!(clear["status"], "clear", "{lifecycle:?}");
            assert!(clear.get("hookSpecificOutput").is_none(), "{lifecycle:?}");
            assert!(clear.get("followup_message").is_none(), "{lifecycle:?}");
            if lifecycle.output == LifecycleOutputProtocol::CodeBuddy {
                assert_eq!(clear["continue"], true, "{lifecycle:?}");
                assert_eq!(clear["suppressOutput"], true, "{lifecycle:?}");
            }

            let blocked = format_lifecycle_decision(
                lifecycle.host_id,
                &decision(
                    lifecycle.host_id,
                    "stop-guard",
                    "blocked",
                    Some("blocking context"),
                ),
            )
            .unwrap();
            match lifecycle.output {
                LifecycleOutputProtocol::ClaudeCompatible => {
                    assert_eq!(blocked["suppressOutput"], true, "{lifecycle:?}");
                    assert_eq!(
                        blocked["hookSpecificOutput"]["hookEventName"], "Stop",
                        "{lifecycle:?}"
                    );
                    assert_eq!(
                        blocked["hookSpecificOutput"]["additionalContext"], "blocking context",
                        "{lifecycle:?}"
                    );
                }
                LifecycleOutputProtocol::CodeBuddy => {
                    assert_eq!(blocked["continue"], false, "{lifecycle:?}");
                    assert_eq!(blocked["reason"], "blocking context", "{lifecycle:?}");
                    assert!(blocked.get("hookSpecificOutput").is_none(), "{lifecycle:?}");
                }
                LifecycleOutputProtocol::Cursor => {
                    assert_eq!(
                        blocked["followup_message"], "blocking context",
                        "{lifecycle:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn production_formatter_rejects_hosts_without_lifecycle_and_unknown_events() {
        let unsupported = AGENT_PLATFORM_SPECS
            .iter()
            .find(|spec| spec.lifecycle.is_none())
            .expect("registry contains an advisory-only host");
        assert!(format_lifecycle_decision(
            unsupported.id,
            &decision(unsupported.id, "session-start", "ready", None)
        )
        .unwrap_err()
        .contains("no lifecycle output protocol"));

        let supported = AGENT_PLATFORM_SPECS
            .iter()
            .find(|spec| spec.lifecycle.is_some())
            .unwrap();
        assert!(format_lifecycle_decision(
            supported.id,
            &decision(supported.id, "round-ended", "clear", None)
        )
        .unwrap_err()
        .contains("unsupported lifecycle event"));
    }
}
