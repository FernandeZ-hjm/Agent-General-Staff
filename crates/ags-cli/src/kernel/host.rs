use crate::cli::HostAction;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const MAX_CAPSULE_CHARS: usize = 12_000;
const MAX_TASK_MEMORY_CHARS: usize = 8_000;

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
    match event {
        "session-start" => session_start(host, target),
        "session-end" => session_end(host, target, &payload),
        "stop-guard" => Ok(stop_guard(host, &payload)),
        _ => Err(format!("unsupported lifecycle event `{event}`")),
    }
}

fn session_start(host: &str, target: &Path) -> Result<Value, String> {
    let memory_dir = memory_dir(target);
    let capsule = bounded_read(&memory_dir.join("context-capsule.md"), MAX_CAPSULE_CHARS)?;
    let task_memory = bounded_read(&memory_dir.join("task-memory.md"), MAX_TASK_MEMORY_CHARS)?;
    let mut parts = vec![
        "## AGS Project Memory Context".to_string(),
        String::new(),
        "Read-only startup context. This is a derived view; receipt-bound raw artifacts are authoritative.".to_string(),
        format!("Repository: {}", target.display()),
        format!("Memory store: {}", memory_dir.display()),
    ];
    if let Some(content) = capsule {
        parts.extend([String::new(), "### context-capsule.md".to_string(), content]);
    }
    if let Some(content) = task_memory {
        parts.extend([String::new(), "### task-memory.md".to_string(), content]);
    }
    let additional_context = (parts.len() > 5).then(|| parts.join("\n"));
    let mut response = json!({
        "schema_version": "0.3.6-host-lifecycle",
        "host": host,
        "event": "session-start",
        "status": if additional_context.is_some() { "ready" } else { "empty" },
    });
    let context = additional_context.unwrap_or_default();
    match lifecycle_output_protocol(host)? {
        ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible => {
            response["suppressOutput"] = json!(true);
            response["hookSpecificOutput"] = json!({
                "hookEventName": "SessionStart",
                "additionalContext": context,
            });
        }
        ags_host_integration::LifecycleOutputProtocol::Cursor => {
            response["additional_context"] = json!(context);
        }
    }
    Ok(response)
}

fn session_end(host: &str, target: &Path, payload: &Value) -> Result<Value, String> {
    let pointer_path = target.join(".ags/state/closure-pointer.json");
    let (status, reason, archive) = if pointer_path.is_file() {
        let pointer: Value = serde_json::from_slice(
            &std::fs::read(&pointer_path)
                .map_err(|error| format!("cannot read closure pointer: {error}"))?,
        )
        .map_err(|error| format!("invalid closure pointer JSON: {error}"))?;
        let receipt_path = pointer
            .get("receipt_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "closure pointer has no receipt_path".to_string())?;
        let result = ags_evidence::memory::archive(Path::new(receipt_path), &memory_dir(target))?;
        (
            if result.idempotent {
                "already-archived"
            } else {
                "archived"
            },
            "verified closure pointer",
            Some(serde_json::to_value(result).map_err(|error| error.to_string())?),
        )
    } else {
        (
            "skipped",
            "no verified task-close closure pointer; transcript inference is forbidden",
            None,
        )
    };
    let receipt = json!({
        "schema_version": "0.3.6-memory-close-receipt",
        "host": host,
        "event": "session-end",
        "session_id": payload.get("session_id").and_then(Value::as_str).unwrap_or(""),
        "status": status,
        "reason": reason,
        "archive": archive,
    });
    let record = target
        .join(".ags/state/lifecycle")
        .join(format!("{host}-session-end.json"));
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
    ags_platform::atomic_write(&record, &bytes)?;
    Ok(receipt)
}

fn stop_guard(host: &str, payload: &Value) -> Value {
    let text = payload
        .get("last_assistant_message")
        .or_else(|| payload.get("lastAssistantMessage"))
        .map(text_content)
        .unwrap_or_default();
    let normalized = text.to_ascii_lowercase();
    let blocked = normalized.contains("<invoke ")
        || normalized.contains("<parameter ")
        || normalized.contains("</invoke>");
    let message = "The previous assistant message leaked raw tool-call markup. Continue with a real tool call; do not expose tool markup.";
    let mut response = json!({
        "schema_version": "0.3.6-host-lifecycle",
        "host": host,
        "event": "stop-guard",
        "status": if blocked { "blocked" } else { "clear" },
    });
    match lifecycle_output_protocol(host) {
        Ok(ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible) => {
            response["suppressOutput"] = json!(blocked);
            response["hookSpecificOutput"] = if blocked {
                json!({
                    "hookEventName": "Stop",
                    "additionalContext": message,
                })
            } else {
                Value::Null
            };
        }
        Ok(ags_host_integration::LifecycleOutputProtocol::Cursor) => {
            if blocked {
                response["followup_message"] = json!(message);
            }
        }
        Err(_) => {}
    }
    response
}

fn lifecycle_output_protocol(
    host: &str,
) -> Result<ags_host_integration::LifecycleOutputProtocol, String> {
    ags_host_integration::platform_spec(host)
        .and_then(|spec| spec.lifecycle_output)
        .ok_or_else(|| format!("host `{host}` has no lifecycle output protocol"))
}

fn text_content(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .map(str::to_string)
                    .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
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

fn bounded_read(path: &Path, limit: usize) -> Result<Option<String>, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let bounded = content.chars().take(limit).collect::<String>();
    Ok(Some(if content.chars().count() > limit {
        format!("{bounded}\n\n[truncated by AGS at {limit} characters]")
    } else {
        bounded
    }))
}

fn memory_dir(target: &Path) -> PathBuf {
    ags_host_integration::project_memory_dir_at(target, &ags_platform::home_dir_or_temp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_hosts_share_the_same_stop_guard_contract() {
        let payload = json!({"last_assistant_message": "<invoke name='x'>"});
        for host in ["codex", "claude-code", "cursor", "omp"] {
            assert_eq!(stop_guard(host, &payload)["status"], "blocked");
        }
        assert!(stop_guard("cursor", &payload)["followup_message"]
            .as_str()
            .unwrap()
            .contains("raw tool-call"));
        assert!(
            stop_guard("codex", &payload)["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("raw tool-call")
        );
    }

    #[test]
    fn cursor_and_claude_compatible_hosts_use_their_native_output_envelopes() {
        let root = tempfile::tempdir().unwrap();
        let cursor = session_start("cursor", root.path()).unwrap();
        assert!(cursor.get("additional_context").is_some());
        assert!(cursor.get("hookSpecificOutput").is_none());

        for host in ["codex", "claude-code", "omp"] {
            let output = session_start(host, root.path()).unwrap();
            assert!(output.get("additional_context").is_none());
            assert_eq!(
                output["hookSpecificOutput"]["hookEventName"],
                "SessionStart"
            );
        }
    }

    #[test]
    fn all_hosts_share_start_and_safe_idempotent_session_end_contract() {
        for host in ["codex", "claude-code", "cursor", "omp"] {
            let root = tempfile::tempdir().unwrap();
            let start = session_start(host, root.path()).unwrap();
            assert_eq!(start["schema_version"], "0.3.6-host-lifecycle");
            assert_eq!(start["host"], host);
            assert_eq!(start["event"], "session-start");

            let payload = json!({
                "session_id": format!("{host}-session"),
                "messages": [{"role": "user", "content": "## 任务卡"}]
            });
            let first = session_end(host, root.path(), &payload).unwrap();
            let second = session_end(host, root.path(), &payload).unwrap();
            assert_eq!(first, second);
            assert_eq!(first["status"], "skipped");
            assert!(first["reason"]
                .as_str()
                .unwrap()
                .contains("inference is forbidden"));
        }
    }
}
