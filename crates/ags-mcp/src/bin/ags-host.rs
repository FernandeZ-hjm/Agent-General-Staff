//! `ags-host` — lifecycle callback adapter (contract v3).
//!
//! The kernel remains evidence-only. This host adapter records session
//! boundaries and projects verified closure facts into machine-local user
//! memory. It never infers task state from transcripts.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ags_kernel::evidence::{Event, EvidenceLog};
use ags_kernel::workspace::{self, WorkspaceBinding};
use serde_json::{json, Value};

const CAPSULE_LIMIT: usize = 12_000;
const TASK_MEMORY_LIMIT: usize = 8_000;
const CLOSURE_REPORT_LIMIT: usize = 4_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut event = String::new();
    let mut workspace_arg: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--event" => {
                i += 1;
                event = args.get(i).cloned().unwrap_or_default();
            }
            "--workspace" => {
                i += 1;
                workspace_arg = args.get(i).map(PathBuf::from);
            }
            _ => {}
        }
        i += 1;
    }
    if !matches!(
        event.as_str(),
        "session-start" | "session-end" | "stop-guard"
    ) {
        println!(
            "{}",
            json!({"error": {"code": "lifecycle_event_invalid", "message": format!("event must be session-start/session-end/stop-guard (got `{event}`)")}})
        );
        std::process::exit(1);
    }

    let input = read_hook_input();
    let root = resolve_root(workspace_arg, &input);
    let binding = match workspace::bind(&root) {
        Ok(binding) => binding,
        Err(_) => {
            println!(
                "{}",
                json!({"decision": "allow", "reason": "workspace-unbound-noop"})
            );
            return;
        }
    };
    let log = EvidenceLog::new(binding.evidence_dir.clone());
    let event_record = log.append(
        "session",
        &binding.slug,
        None,
        "local",
        json!({"boundary": event}),
    );
    let hook_event_name = input
        .get("hook_event_name")
        .or_else(|| input.get("hookEventName"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| default_hook_event(&event).to_string());
    let memory_root = default_memory_root();

    let (additional_context, projected) = match event.as_str() {
        "session-start" => (
            build_start_context(&binding, &memory_root).unwrap_or_default(),
            0,
        ),
        "session-end" => (
            String::new(),
            project_closures(&binding, &log, &memory_root).unwrap_or(0),
        ),
        _ => (String::new(), 0),
    };
    let event_id = event_record.ok().map(|record| record.event_id);
    println!(
        "{}",
        json!({
            "decision": "allow",
            "version": env!("AGS_PRODUCT_VERSION"),
            "build": env!("AGS_BUILD_ID"),
            "hookSpecificOutput": {
                "hookEventName": hook_event_name,
                "additionalContext": additional_context,
                "eventId": event_id,
                "projectedClosures": projected,
            }
        })
    );
}

fn read_hook_input() -> Value {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    serde_json::from_str(&input).unwrap_or_else(|_| json!({}))
}

fn resolve_root(explicit: Option<PathBuf>, input: &Value) -> PathBuf {
    if let Some(path) = explicit {
        if path.join(workspace::AGS_TOML).is_file() {
            return path;
        }
        if let Ok(root) = workspace::find_workspace(&path) {
            return root;
        }
    }
    for key in ["cwd", "workspace", "workspace_dir", "workspaceDir"] {
        if let Some(path) = input.get(key).and_then(Value::as_str).map(PathBuf::from) {
            if let Ok(root) = workspace::find_workspace(&path) {
                return root;
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_hook_event(event: &str) -> &'static str {
    match event {
        "session-start" => "SessionStart",
        "session-end" => "SessionEnd",
        _ => "Stop",
    }
}

fn default_memory_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents/memory/projects")
}

fn build_start_context(binding: &WorkspaceBinding, memory_root: &Path) -> Result<String, String> {
    let dir = checked_memory_dir(memory_root, &binding.slug)?;
    let capsule = bounded_read(&dir.join("context-capsule.md"), CAPSULE_LIMIT);
    let task_memory = bounded_read(&dir.join("task-memory.md"), TASK_MEMORY_LIMIT);
    let mut parts = vec![
        "## AGS Project Memory Context".to_string(),
        String::new(),
        "Read-only startup context projected by ags-host.".to_string(),
        format!("Repository: {}", binding.root.display()),
        format!("Memory store: {}", dir.display()),
    ];
    if capsule.is_empty() && task_memory.is_empty() {
        parts.extend([
            String::new(),
            "No project memory or verified task closure has been recorded yet.".to_string(),
        ]);
    }
    if !capsule.is_empty() {
        parts.extend([
            String::new(),
            "### context-capsule.md".to_string(),
            String::new(),
            capsule,
        ]);
    }
    if !task_memory.is_empty() {
        parts.extend([
            String::new(),
            "### task-memory.md".to_string(),
            String::new(),
            task_memory,
        ]);
    }
    Ok(parts.join("\n"))
}

fn bounded_read(path: &Path, limit: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    truncate_chars(text.trim_end(), limit)
}

fn project_closures(
    binding: &WorkspaceBinding,
    log: &EvidenceLog,
    memory_root: &Path,
) -> Result<usize, String> {
    let dir = checked_memory_dir(memory_root, &binding.slug)?;
    let events = log.read_all().map_err(|error| error.to_string())?;
    EvidenceLog::verify_chain(&events).map_err(|error| error.to_string())?;
    let closures: Vec<&Event> = events
        .iter()
        .filter(|event| event.event_type == "closure")
        .collect();
    if closures.is_empty() {
        return Ok(0);
    }
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let target = dir.join("task-memory.md");
    let mut current = fs::read_to_string(&target).unwrap_or_default();
    let mut projected = 0;
    for event in closures {
        let marker = format!("<!-- ags-closure:{} -->", event.event_id);
        if current.contains(&marker) {
            continue;
        }
        let task = event.task_card_hash.as_deref().unwrap_or("unknown");
        let report = event.payload.get("report").cloned().unwrap_or(Value::Null);
        let report = truncate_chars(&report.to_string(), CLOSURE_REPORT_LIMIT);
        if !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        current.push_str(&format!(
            "\n{marker}\n## Verified task {task}\n\n- Closure event: `{}`\n- Closed at: `{}`\n- Report: {report}\n",
            event.event_id, event.ts
        ));
        projected += 1;
    }
    if projected > 0 {
        let tmp = target.with_extension("tmp");
        fs::write(&tmp, current).map_err(|error| error.to_string())?;
        fs::rename(&tmp, &target).map_err(|error| error.to_string())?;
    }
    Ok(projected)
}

fn checked_memory_dir(memory_root: &Path, slug: &str) -> Result<PathBuf, String> {
    if !ags_kernel::config::valid_slug(slug) {
        return Err("workspace_slug_invalid".to_string());
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if memory_root.starts_with(&home) {
            reject_symlinks_between(&home, memory_root)?;
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(memory_root) {
        if metadata.file_type().is_symlink() {
            return Err("memory_root_symlink_rejected".to_string());
        }
    }
    let dir = memory_root.join(slug);
    if let Ok(metadata) = fs::symlink_metadata(&dir) {
        if metadata.file_type().is_symlink() {
            return Err("memory_slug_symlink_rejected".to_string());
        }
    }
    if !dir.starts_with(memory_root) {
        return Err("memory_path_escape".to_string());
    }
    if memory_root.exists() {
        let root = fs::canonicalize(memory_root).map_err(|error| error.to_string())?;
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            if memory_root.starts_with(&home) {
                let home = fs::canonicalize(home).map_err(|error| error.to_string())?;
                if !root.starts_with(home) {
                    return Err("memory_root_escape".to_string());
                }
            }
        }
        if dir.exists() {
            let target = fs::canonicalize(&dir).map_err(|error| error.to_string())?;
            if !target.starts_with(root) {
                return Err("memory_canonical_escape".to_string());
            }
        }
    }
    Ok(dir)
}

fn reject_symlinks_between(base: &Path, target: &Path) -> Result<(), String> {
    let relative = target
        .strip_prefix(base)
        .map_err(|_| "memory_root_outside_home".to_string())?;
    let mut cursor = base.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "memory_parent_symlink_rejected: {}",
                    cursor.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut value: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        value.push_str("...[truncated]");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(root: &Path) -> WorkspaceBinding {
        let ags_dir = root.join(".ags");
        WorkspaceBinding {
            root: root.to_path_buf(),
            slug: "demo".to_string(),
            role: "A".to_string(),
            evidence_dir: ags_dir.join("evidence"),
            state_dir: ags_dir.join("state"),
            ags_dir,
        }
    }

    #[test]
    fn start_context_uses_canonical_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let memory = tmp.path().join("memory");
        fs::create_dir_all(memory.join("demo")).unwrap();
        fs::write(memory.join("demo/context-capsule.md"), "# Capsule\n").unwrap();
        let context = build_start_context(&binding(tmp.path()), &memory).unwrap();
        assert!(context.contains("# Capsule"));
        assert!(context.contains("Memory store:"));
    }

    #[test]
    fn first_session_has_nonempty_context_without_memory_files() {
        let tmp = tempfile::tempdir().unwrap();
        let memory = tmp.path().join("memory");
        let context = build_start_context(&binding(tmp.path()), &memory).unwrap();
        assert!(context.contains("## AGS Project Memory Context"));
        assert!(context.contains("No project memory or verified task closure"));
        assert!(context.contains("Memory store:"));
        assert!(!memory.exists());
    }

    #[test]
    fn closure_projection_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding(tmp.path());
        let memory = tmp.path().join("memory");
        let log = EvidenceLog::new(binding.evidence_dir.clone());
        log.append(
            "closure",
            "demo",
            Some("task-1"),
            "local",
            json!({"report": {"status": "ok"}}),
        )
        .unwrap();
        assert_eq!(project_closures(&binding, &log, &memory).unwrap(), 1);
        assert_eq!(project_closures(&binding, &log, &memory).unwrap(), 0);
    }

    #[test]
    fn malicious_slug_cannot_escape_memory_root() {
        let tmp = tempfile::tempdir().unwrap();
        let mut binding = binding(tmp.path());
        binding.slug = "../../../escape".to_string();
        let memory = tmp.path().join("memory");
        assert!(build_start_context(&binding, &memory).is_err());
        let log = EvidenceLog::new(binding.evidence_dir.clone());
        assert!(project_closures(&binding, &log, &memory).is_err());
        assert!(!tmp.path().join("escape").exists());
    }

    #[test]
    fn symlinked_memory_slug_cannot_escape_root() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding(tmp.path());
        let memory = tmp.path().join("memory");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&memory).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("context-capsule.md"), "outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, memory.join("demo")).unwrap();
        assert!(build_start_context(&binding, &memory).is_err());
        let log = EvidenceLog::new(binding.evidence_dir.clone());
        assert!(project_closures(&binding, &log, &memory).is_err());
        assert!(!outside.join("task-memory.md").exists());
    }

    #[test]
    fn missing_memory_tail_cannot_follow_home_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join(".agents")).unwrap();
        let target = home.join(".agents/memory/projects");
        assert!(reject_symlinks_between(&home, &target).is_err());
        assert!(!outside.join("memory/projects").exists());
    }

    #[test]
    fn tampered_evidence_is_never_projected() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding(tmp.path());
        let memory = tmp.path().join("memory");
        let log = EvidenceLog::new(binding.evidence_dir.clone());
        log.append(
            "closure",
            "demo",
            Some("task-1"),
            "local",
            json!({"report": {"status": "ok"}}),
        )
        .unwrap();
        let path = binding.evidence_dir.join(ags_kernel::evidence::LOG_FILE);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("\"ok\"", "\"tampered\"");
        fs::write(path, text).unwrap();
        assert!(project_closures(&binding, &log, &memory).is_err());
        assert!(!memory.join("demo/task-memory.md").exists());
    }
}
