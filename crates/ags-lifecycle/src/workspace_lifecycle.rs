use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

pub const LIFECYCLE_SCHEMA_VERSION: &str = "0.4.0-workspace-lifecycle";
pub const CLOSURE_POINTER_SCHEMA_VERSION: &str = "0.4.0-closure-pointer";
const LEGACY_CLOSURE_POINTER_SCHEMA_VERSION: &str = "0.3.6-closure-pointer";
const MAX_CAPSULE_CHARS: usize = 12_000;
const MAX_TASK_MEMORY_CHARS: usize = 8_000;
const MAX_COMPLETED_SESSION_ENDS: usize = 256;
const MAX_RECENT_DECISIONS: usize = 256;
const CLOSURE_POINTERS_DIR: &str = ".ags/state/closure-pointers";
const LEGACY_CLOSURE_POINTER_PATH: &str = ".ags/state/closure-pointer.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosurePointer {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_identity: Option<String>,
    pub receipt_id: String,
    pub receipt_path: String,
    pub task_card_hash: String,
    pub launch_plan_hash: String,
    pub delivery_report_hash: String,
}

impl ClosurePointer {
    fn receipt_path_for(
        &self,
        workspace: &Path,
        expected_identity: &str,
    ) -> Result<PathBuf, String> {
        if let Some(observed) = self.workspace_identity.as_deref() {
            if observed != expected_identity {
                return Err("closure pointer workspace identity mismatch".to_string());
            }
        }
        if let Some(observed) = self.canonical_workspace.as_deref() {
            if observed != workspace.to_string_lossy() {
                return Err("closure pointer canonical workspace mismatch".to_string());
            }
        }

        let raw = PathBuf::from(&self.receipt_path);
        match self.schema_version.as_str() {
            CLOSURE_POINTER_SCHEMA_VERSION => {
                if self.workspace_identity.as_deref() != Some(expected_identity)
                    || self.canonical_workspace.as_deref()
                        != Some(workspace.to_string_lossy().as_ref())
                {
                    return Err(
                        "current closure pointer is missing canonical workspace identity"
                            .to_string(),
                    );
                }
                if !raw.is_absolute() {
                    return Err("current closure pointer receipt path must be absolute".to_string());
                }
                let canonical = raw
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve closure receipt path: {error}"))?;
                require_workspace_path(canonical, workspace, "current closure pointer")
            }
            LEGACY_CLOSURE_POINTER_SCHEMA_VERSION => {
                let candidate = if raw.is_absolute() {
                    raw
                } else {
                    workspace.join(raw)
                };
                let canonical = candidate.canonicalize().map_err(|error| {
                    format!("cannot resolve legacy closure receipt path: {error}")
                })?;
                require_workspace_path(canonical, workspace, "legacy closure pointer")
            }
            other => Err(format!("unsupported closure pointer schema `{other}`")),
        }
    }

    fn verified_receipt_path_for(
        &self,
        workspace: &Path,
        expected_identity: &str,
    ) -> Result<PathBuf, String> {
        let receipt_path = self.receipt_path_for(workspace, expected_identity)?;
        let receipt: ags_evidence::Receipt = serde_json::from_slice(
            &std::fs::read(&receipt_path)
                .map_err(|error| format!("cannot read closure receipt: {error}"))?,
        )
        .map_err(|error| format!("invalid closure receipt JSON: {error}"))?;
        if receipt.receipt_id != self.receipt_id
            || receipt.task_card_hash != self.task_card_hash
            || receipt.launch_plan_hash != self.launch_plan_hash
            || receipt.delivery_report_hash != self.delivery_report_hash
        {
            return Err(format!(
                "closure pointer metadata does not match receipt `{}`",
                self.receipt_id
            ));
        }
        ags_evidence::verify_receipt_artifacts(&receipt)?;
        Ok(receipt_path)
    }
}

fn require_workspace_path(
    canonical: PathBuf,
    workspace: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    if canonical.starts_with(workspace) {
        Ok(canonical)
    } else {
        Err(format!("{label} receipt is not bound to this workspace"))
    }
}

pub fn write_closure_pointer(
    workspace_start: &Path,
    receipt_path: &Path,
    receipt: &ags_evidence::Receipt,
) -> Result<PathBuf, String> {
    let workspace = ags_platform::canonical_workspace_root(workspace_start)?;
    let receipt_candidate = if receipt_path.is_absolute() {
        receipt_path.to_path_buf()
    } else {
        workspace_start.join(receipt_path)
    };
    let receipt_path = receipt_candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve closure receipt path: {error}"))?;
    let receipt_path = require_workspace_path(receipt_path, &workspace, "closure pointer")?;
    let pointer = ClosurePointer {
        schema_version: CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
        canonical_workspace: Some(workspace.to_string_lossy().to_string()),
        workspace_identity: Some(workspace_identity(&workspace)),
        receipt_id: receipt.receipt_id.clone(),
        receipt_path: receipt_path.to_string_lossy().to_string(),
        task_card_hash: receipt.task_card_hash.clone(),
        launch_plan_hash: receipt.launch_plan_hash.clone(),
        delivery_report_hash: receipt.delivery_report_hash.clone(),
    };
    let receipt_id = pointer.receipt_id.as_str();
    if receipt_id.is_empty()
        || receipt_id == "."
        || receipt_id == ".."
        || !receipt_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("closure receipt id is not path-safe".to_string());
    }
    let path = workspace
        .join(CLOSURE_POINTERS_DIR)
        .join(format!("{receipt_id}.json"));
    ags_platform::atomic_write(
        &path,
        &serde_json::to_vec_pretty(&pointer).map_err(|error| error.to_string())?,
    )?;
    Ok(path)
}

#[derive(Debug)]
struct ResolvedClosure {
    pointer: ClosurePointer,
    receipt_path: PathBuf,
    pointer_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEnvelope {
    pub schema_version: String,
    pub canonical_workspace: String,
    pub workspace_identity: String,
    pub host: String,
    pub host_session_id: String,
    pub event: String,
    pub event_id: String,
    #[serde(default)]
    pub payload: Value,
}

impl LifecycleEnvelope {
    pub fn new(workspace: &Path, host: &str, event: &str, payload: Value) -> Result<Self, String> {
        let workspace = ags_platform::canonical_workspace_root(workspace)?;
        let canonical_workspace = workspace.to_string_lossy().to_string();
        let workspace_identity = workspace_identity(&workspace);
        let explicit_event_id = payload
            .get("event_id")
            .or_else(|| payload.get("eventId"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let host_session_id = payload
            .get("session_id")
            .or_else(|| payload.get("sessionId"))
            .or_else(|| payload.get("conversation_id"))
            .or_else(|| payload.get("conversationId"))
            .or_else(|| payload.get("thread_id"))
            .or_else(|| payload.get("threadId"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                explicit_event_id
                    .as_ref()
                    .map(|event_id| format!("event-{event_id}"))
            })
            .unwrap_or_else(|| {
                let body = serde_json::json!({
                    "workspace": canonical_workspace,
                    "host": host,
                    "event": event,
                    "payload": payload,
                });
                stable_json_id("synthetic-session", &body)
            });
        let event_id = explicit_event_id.unwrap_or_else(|| {
            let body = serde_json::json!({
                "workspace": canonical_workspace,
                "host": host,
                "host_session_id": host_session_id,
                "event": event,
                "payload": payload,
            });
            stable_json_id("event", &body)
        });
        Ok(Self {
            schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
            canonical_workspace,
            workspace_identity,
            host: host.to_string(),
            host_session_id,
            event: event.to_string(),
            event_id,
            payload,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifecycleDecision {
    pub schema_version: String,
    pub workspace_identity: String,
    pub host: String,
    pub host_session_id: String,
    pub event: String,
    pub event_id: String,
    pub status: String,
    pub duplicate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<Value>,
}

#[derive(Debug, Clone)]
struct CachedDecision {
    host: String,
    host_session_id: String,
    event_id: String,
    decision: LifecycleDecision,
}

#[derive(Debug, Default)]
struct LifecycleState {
    completed_session_ends: VecDeque<CachedDecision>,
    recent_decisions: VecDeque<CachedDecision>,
    inflight_session_ends: BTreeSet<(String, String)>,
}

#[derive(Debug)]
pub struct LifecycleKernel {
    workspace: PathBuf,
    workspace_identity: String,
    home: PathBuf,
    state: Mutex<LifecycleState>,
    session_end_ready: Condvar,
}

impl LifecycleKernel {
    pub fn new(workspace: PathBuf, home: PathBuf) -> Result<Self, String> {
        let workspace = ags_platform::canonical_workspace_root(&workspace)?;
        Ok(Self {
            workspace_identity: workspace_identity(&workspace),
            workspace,
            home,
            state: Mutex::new(LifecycleState::default()),
            session_end_ready: Condvar::new(),
        })
    }

    pub fn process(&self, envelope: LifecycleEnvelope) -> Result<LifecycleDecision, String> {
        self.validate(&envelope)?;
        if envelope.event == "session-end" {
            return self.coordinate_session_end(&envelope, || self.perform_session_end(&envelope));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "lifecycle state lock poisoned".to_string())?;
        if let Some(duplicate) = recent_duplicate(&state, &envelope) {
            return Ok(duplicate);
        }
        let decision = match envelope.event.as_str() {
            "session-start" => self.session_start(&envelope)?,
            "stop-guard" => self.stop_guard(&envelope),
            other => return Err(format!("unsupported lifecycle event `{other}`")),
        };
        remember_recent_decision(&mut state, &envelope, &decision);
        Ok(decision)
    }

    fn validate(&self, envelope: &LifecycleEnvelope) -> Result<(), String> {
        if envelope.schema_version != LIFECYCLE_SCHEMA_VERSION {
            return Err("workspace lifecycle schema mismatch".to_string());
        }
        if envelope.canonical_workspace != self.workspace.to_string_lossy()
            || envelope.workspace_identity != self.workspace_identity
        {
            return Err("workspace lifecycle identity mismatch".to_string());
        }
        let spec = ags_host_integration::platform_spec(&envelope.host)
            .ok_or_else(|| format!("unsupported host `{}`", envelope.host))?;
        if spec.lifecycle.is_none() {
            return Err(format!("host `{}` has no lifecycle adapter", envelope.host));
        }
        Ok(())
    }

    fn session_start(&self, envelope: &LifecycleEnvelope) -> Result<LifecycleDecision, String> {
        let memory_dir = ags_host_integration::project_memory_dir_at(&self.workspace, &self.home);
        let capsule = bounded_read(&memory_dir.join("context-capsule.md"), MAX_CAPSULE_CHARS)?;
        let task_memory = bounded_read(&memory_dir.join("task-memory.md"), MAX_TASK_MEMORY_CHARS)?;
        let mut parts = vec![
            "## AGS Project Memory Context".to_string(),
            String::new(),
            "Read-only startup context. Receipt-bound raw artifacts remain authoritative."
                .to_string(),
            format!("Repository: {}", self.workspace.display()),
            format!("Memory store: {}", memory_dir.display()),
        ];
        if let Some(content) = capsule {
            parts.extend([String::new(), "### context-capsule.md".to_string(), content]);
        }
        if let Some(content) = task_memory {
            parts.extend([String::new(), "### task-memory.md".to_string(), content]);
        }
        let additional_context = (parts.len() > 5).then(|| parts.join("\n"));
        Ok(self.decision(
            envelope,
            if additional_context.is_some() {
                "ready"
            } else {
                "empty"
            },
            additional_context,
            None,
            None,
        ))
    }

    fn stop_guard(&self, envelope: &LifecycleEnvelope) -> LifecycleDecision {
        let text = envelope
            .payload
            .get("last_assistant_message")
            .or_else(|| envelope.payload.get("lastAssistantMessage"))
            .map(text_content)
            .unwrap_or_default();
        let normalized = text.to_ascii_lowercase();
        let blocked = normalized.contains("<invoke ")
            || normalized.contains("<parameter ")
            || normalized.contains("</invoke>");
        self.decision(
            envelope,
            if blocked { "blocked" } else { "clear" },
            blocked.then(|| "The previous assistant message leaked raw tool-call markup. Continue with a real tool call; do not expose tool markup.".to_string()),
            None,
            None,
        )
    }

    fn pending_closures(&self) -> Result<Vec<ResolvedClosure>, String> {
        let mut pointer_paths = Vec::new();
        let pointer_dir = self.workspace.join(CLOSURE_POINTERS_DIR);
        if pointer_dir.is_dir() {
            let entries = std::fs::read_dir(&pointer_dir)
                .map_err(|error| format!("cannot read closure pointer directory: {error}"))?;
            for entry in entries {
                let path = entry
                    .map_err(|error| format!("cannot read closure pointer entry: {error}"))?
                    .path();
                if path.is_file()
                    && path.extension().and_then(|value| value.to_str()) == Some("json")
                {
                    pointer_paths.push(path);
                }
            }
        }
        pointer_paths.sort();
        let legacy_pointer = self.workspace.join(LEGACY_CLOSURE_POINTER_PATH);
        if legacy_pointer.is_file() {
            pointer_paths.push(legacy_pointer);
        }

        let mut closures = BTreeMap::<String, ResolvedClosure>::new();
        for pointer_path in pointer_paths {
            let pointer: ClosurePointer =
                serde_json::from_slice(&std::fs::read(&pointer_path).map_err(|error| {
                    format!(
                        "cannot read closure pointer `{}`: {error}",
                        pointer_path.display()
                    )
                })?)
                .map_err(|error| {
                    format!(
                        "invalid closure pointer JSON `{}`: {error}",
                        pointer_path.display()
                    )
                })?;
            let receipt_path = pointer
                .verified_receipt_path_for(&self.workspace, &self.workspace_identity)
                .map_err(|error| {
                    format!(
                        "invalid closure pointer `{}`: {error}",
                        pointer_path.display()
                    )
                })?;
            match closures.entry(pointer.receipt_id.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(ResolvedClosure {
                        pointer,
                        receipt_path,
                        pointer_paths: vec![pointer_path],
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let existing = entry.get_mut();
                    if existing.pointer != pointer || existing.receipt_path != receipt_path {
                        return Err(format!(
                            "conflicting closure pointers for receipt `{}`",
                            pointer.receipt_id
                        ));
                    }
                    existing.pointer_paths.push(pointer_path);
                }
            }
        }
        Ok(closures.into_values().collect())
    }

    fn coordinate_session_end<F>(
        &self,
        envelope: &LifecycleEnvelope,
        perform: F,
    ) -> Result<LifecycleDecision, String>
    where
        F: FnOnce() -> Result<LifecycleDecision, String>,
    {
        let key = (envelope.host.clone(), envelope.host_session_id.clone());
        loop {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "lifecycle state lock poisoned".to_string())?;
            if let Some(duplicate) = recent_duplicate(&state, envelope) {
                return Ok(duplicate);
            }
            if let Some(previous) = state
                .completed_session_ends
                .iter()
                .find(|previous| {
                    previous.host == envelope.host
                        && previous.host_session_id == envelope.host_session_id
                })
                .cloned()
            {
                if previous.event_id == envelope.event_id {
                    return Ok(LifecycleDecision {
                        duplicate: true,
                        ..previous.decision
                    });
                }
                let decision = self.decision(
                    envelope,
                    "already-ended",
                    None,
                    Some("host session was already closed".to_string()),
                    None,
                );
                remember_recent_decision(&mut state, envelope, &decision);
                return Ok(decision);
            }
            if state.inflight_session_ends.contains(&key) {
                drop(
                    self.session_end_ready
                        .wait(state)
                        .map_err(|_| "lifecycle state lock poisoned".to_string())?,
                );
                continue;
            }
            state.inflight_session_ends.insert(key.clone());
            break;
        }

        let result = perform();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "lifecycle state lock poisoned".to_string())?;
        state.inflight_session_ends.remove(&key);
        let result = result.map(|decision| {
            if state.completed_session_ends.len() == MAX_COMPLETED_SESSION_ENDS {
                state.completed_session_ends.pop_front();
            }
            state.completed_session_ends.push_back(CachedDecision {
                host: envelope.host.clone(),
                host_session_id: envelope.host_session_id.clone(),
                event_id: envelope.event_id.clone(),
                decision: decision.clone(),
            });
            remember_recent_decision(&mut state, envelope, &decision);
            decision
        });
        self.session_end_ready.notify_all();
        result
    }

    fn perform_session_end(
        &self,
        envelope: &LifecycleEnvelope,
    ) -> Result<LifecycleDecision, String> {
        let closures = self.pending_closures()?;
        let (status, reason, archive) = if closures.is_empty() {
            (
                "skipped",
                "no verified task-close closure pointer; transcript inference is forbidden"
                    .to_string(),
                None,
            )
        } else {
            let memory_dir =
                ags_host_integration::project_memory_dir_at(&self.workspace, &self.home);
            let mut results = Vec::with_capacity(closures.len());
            for closure in closures {
                let result = ags_evidence::memory::archive(&closure.receipt_path, &memory_dir)?;
                for pointer_path in closure.pointer_paths {
                    match std::fs::remove_file(&pointer_path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(format!(
                                "cannot remove archived closure pointer `{}`: {error}",
                                pointer_path.display()
                            ));
                        }
                    }
                }
                results.push(result);
            }
            let all_idempotent = results.iter().all(|result| result.idempotent);
            let count = results.len();
            (
                if all_idempotent {
                    "already-archived"
                } else {
                    "archived"
                },
                if count == 1 {
                    "verified closure pointer".to_string()
                } else {
                    format!("{count} verified closure pointers")
                },
                Some(serde_json::to_value(results).map_err(|error| error.to_string())?),
            )
        };
        let decision = self.decision(envelope, status, None, Some(reason.to_string()), archive);
        let record_key = ags_platform::sha256_hex(
            format!("{}\0{}", envelope.host, envelope.host_session_id).as_bytes(),
        );
        let record = self
            .workspace
            .join(".ags/state/lifecycle")
            .join(format!("session-end-{record_key}.json"));
        ags_platform::atomic_write(
            &record,
            &serde_json::to_vec_pretty(&decision).map_err(|error| error.to_string())?,
        )?;
        Ok(decision)
    }

    fn decision(
        &self,
        envelope: &LifecycleEnvelope,
        status: &str,
        additional_context: Option<String>,
        reason: Option<String>,
        archive: Option<Value>,
    ) -> LifecycleDecision {
        LifecycleDecision {
            schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
            workspace_identity: self.workspace_identity.clone(),
            host: envelope.host.clone(),
            host_session_id: envelope.host_session_id.clone(),
            event: envelope.event.clone(),
            event_id: envelope.event_id.clone(),
            status: status.to_string(),
            duplicate: false,
            additional_context,
            reason,
            archive,
        }
    }
}

fn recent_duplicate(
    state: &LifecycleState,
    envelope: &LifecycleEnvelope,
) -> Option<LifecycleDecision> {
    state
        .recent_decisions
        .iter()
        .find(|previous| {
            previous.host == envelope.host
                && previous.host_session_id == envelope.host_session_id
                && previous.event_id == envelope.event_id
        })
        .map(|previous| LifecycleDecision {
            duplicate: true,
            ..previous.decision.clone()
        })
}

fn remember_recent_decision(
    state: &mut LifecycleState,
    envelope: &LifecycleEnvelope,
    decision: &LifecycleDecision,
) {
    if state.recent_decisions.len() == MAX_RECENT_DECISIONS {
        state.recent_decisions.pop_front();
    }
    state.recent_decisions.push_back(CachedDecision {
        host: envelope.host.clone(),
        host_session_id: envelope.host_session_id.clone(),
        event_id: envelope.event_id.clone(),
        decision: decision.clone(),
    });
}

pub fn workspace_identity(workspace: &Path) -> String {
    ags_platform::sha256_hex(workspace.to_string_lossy().as_bytes())
}

fn stable_json_id(prefix: &str, value: &Value) -> String {
    format!(
        "{prefix}-{}",
        ags_platform::sha256_hex(
            serde_json::to_vec(value).expect("lifecycle event is serializable")
        )
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    fn kernel() -> (tempfile::TempDir, PathBuf, LifecycleKernel) {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let home = root.path().join("home");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let kernel = LifecycleKernel::new(workspace.clone(), home).unwrap();
        (root, workspace, kernel)
    }

    fn event(
        workspace: &Path,
        host: &str,
        name: &str,
        session_id: &str,
        event_id: &str,
    ) -> LifecycleEnvelope {
        LifecycleEnvelope::new(
            workspace,
            host,
            name,
            serde_json::json!({"session_id": session_id, "event_id": event_id}),
        )
        .unwrap()
    }

    fn valid_receipt(workspace: &Path, suffix: &str) -> (ags_evidence::Receipt, PathBuf) {
        let artifact_dir = workspace.join(".ags/test-artifacts").join(suffix);
        std::fs::create_dir_all(&artifact_dir).unwrap();
        let task_card_path = artifact_dir.join("task-card.md");
        let launch_plan_path = artifact_dir.join("launch-plan.json");
        let delivery_report_path = artifact_dir.join("delivery-report.md");
        std::fs::write(&task_card_path, format!("# Task {suffix}\n")).unwrap();
        std::fs::write(
            &delivery_report_path,
            format!("# Delivery {suffix}\n\nStatus: complete\n"),
        )
        .unwrap();
        let task_card_hash = ags_evidence::hash_file(&task_card_path).unwrap();
        let delivery_report_hash = ags_evidence::hash_file(&delivery_report_path).unwrap();
        let mut launch_plan = serde_json::json!({
            "schema_version": "0.3.6-launch-plan",
            "test_case": suffix,
        });
        let launch_plan_hash = ags_platform::sha256_hex(&serde_json::to_vec(&launch_plan).unwrap());
        launch_plan["launch_plan_hash"] = serde_json::json!(launch_plan_hash.clone());
        std::fs::write(
            &launch_plan_path,
            serde_json::to_vec_pretty(&launch_plan).unwrap(),
        )
        .unwrap();
        let receipt_id = ags_evidence::receipt_id(&task_card_hash, &launch_plan_hash);
        let receipt: ags_evidence::Receipt = serde_json::from_value(serde_json::json!({
            "schema_version": "0.3.6-task-receipt",
            "receipt_id": receipt_id,
            "timestamp": "unix-0",
            "task_card_hash": task_card_hash,
            "launch_plan_hash": launch_plan_hash,
            "task_card_path": task_card_path,
            "launch_plan_path": launch_plan_path,
            "delivery_report_path": delivery_report_path,
            "gate_result": {"decision": "allow"},
            "verification_results": [],
            "delivery_report_hash": delivery_report_hash,
            "execution_footprint": {
                "execution_mode_used": "single-writer",
                "execution_topology_used": "single",
                "delegation_used": "no"
            },
            "closure_status": "done",
            "exit_code": 0
        }))
        .unwrap();
        let receipt_path = workspace
            .join(".ags/receipts")
            .join(format!("{}.json", receipt.receipt_id));
        std::fs::create_dir_all(receipt_path.parent().unwrap()).unwrap();
        std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        (receipt, receipt_path)
    }

    #[test]
    fn subdirectory_close_writes_a_workspace_bound_pointer() {
        let (root, workspace, kernel) = kernel();
        let subdirectory = workspace.join("nested/work");
        std::fs::create_dir_all(&subdirectory).unwrap();
        let (receipt, receipt_path) = valid_receipt(&workspace, "subdirectory");

        let pointer_path = write_closure_pointer(&subdirectory, &receipt_path, &receipt).unwrap();
        let canonical_workspace = ags_platform::canonical_workspace_root(&workspace).unwrap();
        assert_eq!(
            pointer_path,
            canonical_workspace
                .join(CLOSURE_POINTERS_DIR)
                .join(format!("{}.json", receipt.receipt_id))
        );
        assert!(!subdirectory.join(CLOSURE_POINTERS_DIR).exists());
        let mut pointer: ClosurePointer =
            serde_json::from_slice(&std::fs::read(&pointer_path).unwrap()).unwrap();
        let identity = workspace_identity(&canonical_workspace);
        assert_eq!(
            pointer.workspace_identity.as_deref(),
            Some(identity.as_str())
        );
        assert_eq!(
            pointer
                .receipt_path_for(&canonical_workspace, &identity)
                .unwrap(),
            receipt_path.canonicalize().unwrap()
        );

        let outside = root.path().join("outside-receipt.json");
        std::fs::write(&outside, "{}").unwrap();
        let mut current_unbound = pointer.clone();
        current_unbound.receipt_path = outside.to_string_lossy().to_string();
        assert!(current_unbound
            .receipt_path_for(&canonical_workspace, &identity)
            .unwrap_err()
            .contains("not bound"));
        assert!(write_closure_pointer(&workspace, &outside, &receipt)
            .unwrap_err()
            .contains("not bound"));

        pointer.workspace_identity = Some("wrong-workspace".to_string());
        assert!(pointer
            .receipt_path_for(&canonical_workspace, &identity)
            .unwrap_err()
            .contains("identity mismatch"));
        std::fs::write(&pointer_path, serde_json::to_vec(&pointer).unwrap()).unwrap();
        assert!(kernel
            .process(event(
                &workspace,
                "codex",
                "session-end",
                "wrong-pointer",
                "wrong-pointer-end",
            ))
            .unwrap_err()
            .contains("identity mismatch"));

        let legacy = ClosurePointer {
            schema_version: LEGACY_CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: None,
            workspace_identity: None,
            receipt_id: receipt.receipt_id,
            receipt_path: receipt_path
                .strip_prefix(&workspace)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            task_card_hash: receipt.task_card_hash,
            launch_plan_hash: receipt.launch_plan_hash,
            delivery_report_hash: receipt.delivery_report_hash,
        };
        assert_eq!(
            legacy
                .receipt_path_for(&canonical_workspace, &identity)
                .unwrap(),
            receipt_path.canonicalize().unwrap()
        );
        let mut unbound = legacy;
        unbound.receipt_path = outside.to_string_lossy().to_string();
        assert!(unbound
            .receipt_path_for(&canonical_workspace, &identity)
            .unwrap_err()
            .contains("not bound"));
    }

    #[test]
    fn session_end_archives_every_pending_receipt_and_consumes_legacy_latest_pointer() {
        let (_root, workspace, kernel) = kernel();
        let (first_receipt, first_receipt_path) = valid_receipt(&workspace, "first");
        let (second_receipt, second_receipt_path) = valid_receipt(&workspace, "second");
        let first_pointer =
            write_closure_pointer(&workspace, &first_receipt_path, &first_receipt).unwrap();
        let second_pointer =
            write_closure_pointer(&workspace, &second_receipt_path, &second_receipt).unwrap();
        assert_ne!(first_pointer, second_pointer);
        assert!(first_pointer.is_file());
        assert!(second_pointer.is_file());
        assert!(!workspace.join(LEGACY_CLOSURE_POINTER_PATH).exists());

        let decision = kernel
            .process(event(
                &workspace,
                "codex",
                "session-end",
                "archive-all",
                "archive-all-end",
            ))
            .unwrap();
        assert_eq!(decision.status, "archived");
        let archived = decision.archive.unwrap();
        let archived = archived.as_array().unwrap();
        assert_eq!(archived.len(), 2);
        let archived_ids = archived
            .iter()
            .filter_map(|item| item.get("receipt_id").and_then(Value::as_str))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            archived_ids,
            [
                first_receipt.receipt_id.as_str(),
                second_receipt.receipt_id.as_str()
            ]
            .into_iter()
            .collect()
        );
        let memory_dir = ags_host_integration::project_memory_dir_at(&workspace, &kernel.home);
        for receipt in [&first_receipt, &second_receipt] {
            assert!(memory_dir
                .join("task-archive")
                .join(&receipt.receipt_id)
                .join("receipt.json")
                .is_file());
        }
        assert!(!first_pointer.exists());
        assert!(!second_pointer.exists());
        assert_eq!(
            kernel
                .process(event(
                    &workspace,
                    "codex",
                    "session-end",
                    "after-archive",
                    "after-archive-end",
                ))
                .unwrap()
                .status,
            "skipped"
        );

        let (legacy_receipt, legacy_receipt_path) = valid_receipt(&workspace, "legacy");
        let legacy_pointer = ClosurePointer {
            schema_version: LEGACY_CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: None,
            workspace_identity: None,
            receipt_id: legacy_receipt.receipt_id.clone(),
            receipt_path: legacy_receipt_path.to_string_lossy().to_string(),
            task_card_hash: legacy_receipt.task_card_hash.clone(),
            launch_plan_hash: legacy_receipt.launch_plan_hash.clone(),
            delivery_report_hash: legacy_receipt.delivery_report_hash.clone(),
        };
        let legacy_pointer_path = workspace.join(LEGACY_CLOSURE_POINTER_PATH);
        ags_platform::atomic_write(
            &legacy_pointer_path,
            &serde_json::to_vec_pretty(&legacy_pointer).unwrap(),
        )
        .unwrap();
        let legacy_decision = kernel
            .process(event(
                &workspace,
                "codex",
                "session-end",
                "legacy",
                "legacy-end",
            ))
            .unwrap();
        assert_eq!(legacy_decision.status, "archived");
        assert!(!legacy_pointer_path.exists());
        assert!(memory_dir
            .join("task-archive")
            .join(legacy_receipt.receipt_id)
            .join("receipt.json")
            .is_file());
    }

    #[test]
    fn session_end_is_workspace_bound_and_duplicate_delivery_is_idempotent() {
        let (root, workspace, kernel) = kernel();
        let envelope = event(&workspace, "codex", "session-end", "s1", "end-1");
        let first = kernel.process(envelope.clone()).unwrap();
        let second = kernel.process(envelope).unwrap();
        assert!(!first.duplicate);
        assert!(second.duplicate);
        assert_eq!(first.status, "skipped");

        let later_event = event(&workspace, "codex", "session-end", "s1", "end-2");
        let already_ended = kernel.process(later_event).unwrap();
        assert!(!already_ended.duplicate);
        assert_eq!(already_ended.status, "already-ended");

        let other_session_same_event = event(&workspace, "codex", "session-end", "s2", "end-1");
        let other_session = kernel.process(other_session_same_event).unwrap();
        assert!(!other_session.duplicate);
        assert_eq!(other_session.status, "skipped");

        let other_host_same_session_and_event =
            event(&workspace, "omp", "session-end", "s1", "end-1");
        let other_host = kernel.process(other_host_same_session_and_event).unwrap();
        assert!(!other_host.duplicate);
        assert_eq!(other_host.status, "skipped");

        let other = root.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let wrong = event(&other, "codex", "session-start", "s1", "start-1");
        assert!(kernel
            .process(wrong)
            .unwrap_err()
            .contains("identity mismatch"));
    }

    #[test]
    fn pure_start_and_stop_events_use_bounded_idempotency_state() {
        let (_root, workspace, kernel) = kernel();
        let start = event(&workspace, "codex", "session-start", "s1", "start-1");
        assert!(!kernel.process(start.clone()).unwrap().duplicate);
        assert!(kernel.process(start).unwrap().duplicate);

        let stop = event(&workspace, "codex", "stop-guard", "s1", "stop-1");
        assert!(!kernel.process(stop.clone()).unwrap().duplicate);
        assert!(kernel.process(stop).unwrap().duplicate);
        let state = kernel.state.lock().unwrap();
        assert!(state.completed_session_ends.is_empty());
        assert_eq!(state.recent_decisions.len(), 2);
    }

    #[test]
    fn missing_or_empty_host_session_ids_do_not_collapse_unrelated_ends() {
        let (_root, workspace, kernel) = kernel();
        let first = LifecycleEnvelope::new(
            &workspace,
            "omp",
            "session-end",
            serde_json::json!({"session_id": "", "event_id": "omp-end-1"}),
        )
        .unwrap();
        let second = LifecycleEnvelope::new(
            &workspace,
            "omp",
            "session-end",
            serde_json::json!({"event_id": "omp-end-2"}),
        )
        .unwrap();
        assert_ne!(first.host_session_id, second.host_session_id);
        assert!(!first.host_session_id.is_empty());
        assert!(!second.host_session_id.is_empty());
        assert_eq!(kernel.process(first).unwrap().status, "skipped");
        assert_eq!(kernel.process(second).unwrap().status, "skipped");
    }

    #[test]
    fn failed_session_end_is_retryable_and_completed_state_is_bounded() {
        let (_root, workspace, kernel) = kernel();
        let pointer = workspace.join(".ags/state/closure-pointer.json");
        std::fs::create_dir_all(pointer.parent().unwrap()).unwrap();
        std::fs::write(&pointer, b"{invalid").unwrap();
        let retry = event(&workspace, "codex", "session-end", "retry", "retry-end");
        assert!(kernel.process(retry.clone()).is_err());
        std::fs::remove_file(pointer).unwrap();
        assert_eq!(kernel.process(retry).unwrap().status, "skipped");

        for index in 0..=MAX_COMPLETED_SESSION_ENDS {
            let envelope = event(
                &workspace,
                "codex",
                "session-end",
                &format!("bounded-{index}"),
                &format!("bounded-end-{index}"),
            );
            kernel.process(envelope).unwrap();
        }
        assert_eq!(
            kernel.state.lock().unwrap().completed_session_ends.len(),
            MAX_COMPLETED_SESSION_ENDS
        );
        assert_eq!(
            kernel.state.lock().unwrap().recent_decisions.len(),
            MAX_RECENT_DECISIONS
        );
    }

    #[test]
    fn same_host_sessions_get_unique_path_safe_records() {
        let (_root, workspace, kernel) = kernel();
        let sessions = ["../../escape", "second session / ?"];
        for (index, session) in sessions.iter().enumerate() {
            kernel
                .process(event(
                    &workspace,
                    "codex",
                    "session-end",
                    session,
                    &format!("end-{index}"),
                ))
                .unwrap();
        }

        let records = std::fs::read_dir(workspace.join(".ags/state/lifecycle"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), sessions.len());
        let mut observed_sessions = BTreeSet::new();
        for record in records {
            let name = record.file_name().unwrap().to_string_lossy();
            let digest = name
                .strip_prefix("session-end-")
                .and_then(|value| value.strip_suffix(".json"))
                .unwrap();
            assert_eq!(digest.len(), 64);
            assert!(digest
                .chars()
                .all(|character| character.is_ascii_hexdigit()));
            let decision: LifecycleDecision =
                serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
            observed_sessions.insert(decision.host_session_id);
        }
        assert_eq!(
            observed_sessions,
            sessions.into_iter().map(str::to_string).collect()
        );
        assert!(!workspace.parent().unwrap().join("escape").exists());
    }

    #[test]
    fn session_end_io_does_not_hold_the_global_state_lock() {
        let (_root, workspace, kernel) = kernel();
        let kernel = Arc::new(kernel);
        let end = event(&workspace, "codex", "session-end", "slow", "slow-end");
        let end_for_decision = end.clone();
        let io_kernel = Arc::clone(&kernel);
        let decision_kernel = Arc::clone(&kernel);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let end_worker = thread::spawn(move || {
            io_kernel.coordinate_session_end(&end, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(decision_kernel.decision(
                    &end_for_decision,
                    "skipped",
                    None,
                    Some("test side effect".to_string()),
                    None,
                ))
            })
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let duplicate_kernel = Arc::clone(&kernel);
        let duplicate_end = event(&workspace, "codex", "session-end", "slow", "slow-end");
        let (duplicate_tx, duplicate_rx) = mpsc::channel();
        let duplicate_worker = thread::spawn(move || {
            duplicate_tx
                .send(duplicate_kernel.process(duplicate_end))
                .unwrap();
        });
        assert!(matches!(
            duplicate_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        let stop = event(&workspace, "codex", "stop-guard", "other", "stop-1");
        let stop_kernel = Arc::clone(&kernel);
        let (stop_tx, stop_rx) = mpsc::channel();
        let stop_worker = thread::spawn(move || {
            stop_tx.send(stop_kernel.process(stop)).unwrap();
        });
        let stop_result = stop_rx.recv_timeout(Duration::from_millis(500));
        release_tx.send(()).unwrap();
        let end_result = end_worker.join().unwrap();
        let duplicate_result = duplicate_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        duplicate_worker.join().unwrap();
        stop_worker.join().unwrap();

        assert!(
            stop_result.is_ok(),
            "stop event was blocked behind session-end I/O"
        );
        assert_eq!(stop_result.unwrap().unwrap().status, "clear");
        assert_eq!(end_result.unwrap().status, "skipped");
        assert!(duplicate_result.unwrap().duplicate);
        assert!(kernel
            .state
            .lock()
            .unwrap()
            .inflight_session_ends
            .is_empty());
    }
}
