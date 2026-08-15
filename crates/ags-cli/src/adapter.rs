use crate::cli::OutputFormat;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;

const MAX_TYPED_INPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_JSON_BYTES: usize = 16 * 1024;

pub fn read_typed_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .take((MAX_TYPED_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read outcome stdin: {error}"))?;
    } else {
        open_regular_input(path, "outcome")?
            .take((MAX_TYPED_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read outcome `{}`: {error}", path.display()))?;
    }
    if bytes.len() > MAX_TYPED_INPUT_BYTES {
        return Err("typed outcome exceeds 1 MiB".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid typed outcome JSON: {error}"))
}

fn open_regular_input(path: &Path, label: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("cannot open {label} `{}`: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} `{}`: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "{label} `{}` must be a regular file",
            path.display()
        ));
    }
    Ok(file)
}

/// Render one central control-plane response. Formatting never changes the
/// typed `OperationRequest` submitted to the workspace service.
pub fn render<T: Serialize>(value: &T, format: OutputFormat) -> Result<String, String> {
    let encoded = serde_json::to_value(value)
        .map_err(|error| format!("cannot encode contract-v2 response: {error}"))?;
    match format {
        OutputFormat::Json => {
            let output = serde_json::to_string(&encoded)
                .map_err(|error| format!("cannot encode contract-v2 response: {error}"))?;
            if output.len() > DEFAULT_JSON_BYTES {
                return Err(format!(
                    "default JSON response is {} bytes; a configured details store is required",
                    output.len()
                ));
            }
            Ok(output)
        }
        OutputFormat::Text => Ok(
            match (
                encoded.get("state").and_then(serde_json::Value::as_str),
                encoded
                    .get("action_ref")
                    .and_then(serde_json::Value::as_str),
            ) {
                (Some(state), Some(action_ref)) => {
                    format!("state: {state}\naction_ref: {action_ref}")
                }
                (Some(state), None) => render_read_or_receipt(state, &encoded),
                _ => "state: no-change".to_string(),
            },
        ),
    }
}

fn render_read_or_receipt(state: &str, encoded: &serde_json::Value) -> String {
    let mut lines = vec![format!("state: {state}")];
    if let Some(result) = encoded.get("result") {
        if let Some(status) = result.get("status").and_then(serde_json::Value::as_str) {
            lines.push(format!("status: {status}"));
        }
        if let Some(workspace) = result
            .get("canonical_workspace")
            .or_else(|| result.get("repo_root"))
            .and_then(serde_json::Value::as_str)
        {
            lines.push(format!("workspace: {workspace}"));
        }
        if let Some(summary) = result.get("summary") {
            lines.push(format!("summary: {}", compact(summary, 240)));
        } else if let Some(operations) = result
            .get("operations")
            .and_then(serde_json::Value::as_array)
        {
            lines.push(format!("operations: {}", operations.len()));
        }
        if let Some(project_tests) = result
            .get("project_tests_run")
            .and_then(serde_json::Value::as_bool)
        {
            lines.push(format!("project tests run: {project_tests}"));
        }
        if let Some(finding) = first_non_pass(result) {
            lines.push(format!("finding: {}", compact(finding, 240)));
        }
    } else if let Some(receipt) = encoded.get("receipt") {
        if let Some(id) = receipt
            .get("receipt_id")
            .and_then(serde_json::Value::as_str)
        {
            lines.push(format!("receipt: {id}"));
        }
        if let Some(status) = receipt.get("status").and_then(serde_json::Value::as_str) {
            lines.push(format!("status: {status}"));
        }
    }
    lines.truncate(5);
    lines.join("\n")
}

fn first_non_pass(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value
        .get("findings")
        .or_else(|| value.get("items"))
        .and_then(serde_json::Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                !matches!(
                    item.get("status").and_then(serde_json::Value::as_str),
                    Some("pass")
                )
            })
        })
}

fn compact(value: &serde_json::Value, limit: usize) -> String {
    let encoded = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    if encoded.len() <= limit {
        encoded
    } else {
        let content_limit = limit.saturating_sub(3);
        let boundary = encoded
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= content_limit)
            .last()
            .unwrap_or(0);
        format!("{}...", &encoded[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_respects_a_byte_budget_for_unicode() {
        let output = compact(&serde_json::json!("治理治理治理治理"), 12);
        assert!(output.len() <= 12, "{} bytes: {output}", output.len());
        assert!(output.ends_with("..."));
    }

    #[cfg(unix)]
    #[test]
    fn typed_outcome_path_rejects_symlinks_and_non_regular_files() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("outcome.json");
        let link = temp.path().join("outcome-link.json");
        let fifo = temp.path().join("outcome.fifo");
        let oversized = temp.path().join("oversized.json");
        std::fs::write(&target, br#"{"status":"succeeded"}"#).unwrap();
        symlink(&target, &link).unwrap();
        let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
        std::fs::write(&oversized, vec![b'x'; MAX_TYPED_INPUT_BYTES + 1]).unwrap();

        assert!(read_typed_json::<serde_json::Value>(&link).is_err());
        assert!(read_typed_json::<serde_json::Value>(Path::new("/dev/null")).is_err());
        assert!(read_typed_json::<serde_json::Value>(&fifo).is_err());
        assert_eq!(
            read_typed_json::<serde_json::Value>(&oversized).unwrap_err(),
            "typed outcome exceeds 1 MiB"
        );
    }
}
