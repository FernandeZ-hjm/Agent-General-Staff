use super::assets::*;

/// Outcome of merging the memory-capture step into a Stop pipeline value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    /// A memory-capture command already existed — nothing changed.
    AlreadyPresent,
    /// A memory-capture command was inserted into the pipeline.
    Wired,
}

fn command_str(hook: &serde_json::Value) -> Option<&str> {
    hook.get("command").and_then(|c| c.as_str())
}

fn hook_has_marker(hook: &serde_json::Value, marker: &str) -> bool {
    command_str(hook)
        .map(|c| c.contains(marker))
        .unwrap_or(false)
}

fn group_has_marker(group: &serde_json::Value, marker: &str) -> bool {
    let nested = group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| hooks.iter().any(|h| hook_has_marker(h, marker)))
        .unwrap_or(false);
    let flat = hook_has_marker(group, marker);
    nested || flat
}

pub(super) fn hooks_contain(groups: &[serde_json::Value], marker: &str) -> bool {
    groups.iter().any(|group| group_has_marker(group, marker))
}

fn memory_hook_entry(command: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": 10,
    })
}

fn raw_guard_hook_entry() -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": raw_guard_command(),
        "timeout": 2,
    })
}

fn memory_start_hook_entry(command: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": 5,
    })
}

/// Merge a read-only project-memory injection step into `hooks.SessionStart`.
///
/// The start hook reads the current repository's local AGS memory capsule and
/// task-memory file, then returns a bounded `additionalContext` block to the
/// host. It must not write memory files.
pub(in crate::setup) fn merge_memory_start(
    value: &mut serde_json::Value,
    command: &str,
) -> MergeOutcome {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let root = value.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks_obj = hooks.as_object_mut().expect("hooks object");
    let start = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    if !start.is_array() {
        *start = serde_json::json!([]);
    }
    let start_arr = start.as_array_mut().expect("SessionStart array");

    if hooks_contain(start_arr, MEMORY_START_MARKER) {
        return MergeOutcome::AlreadyPresent;
    }

    let entry = memory_start_hook_entry(command);
    if let Some(group) = start_arr
        .iter_mut()
        .find(|group| group.get("hooks").and_then(|h| h.as_array()).is_some())
    {
        group
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array")
            .push(entry);
    } else {
        start_arr.push(serde_json::json!({ "hooks": [entry] }));
    }
    MergeOutcome::Wired
}

/// Merge Codex's native SessionEnd capture hook. SessionEnd handlers have a
/// hard three-second ceiling, so this entry is deliberately bounded to 3s.
/// The merge also retires the historical per-prompt shell injection: memory is
/// injected once by SessionStart, not repeatedly by UserPromptSubmit.
pub(crate) fn merge_codex_memory_lifecycle(
    value: &mut serde_json::Value,
    start_command: &str,
    close_command: &str,
) -> MergeOutcome {
    let mut changed = merge_memory_start(value, start_command) == MergeOutcome::Wired;
    let root = value.as_object_mut().expect("object");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("hooks object");

    if let Some(prompt_groups) = hooks
        .get_mut("UserPromptSubmit")
        .and_then(|groups| groups.as_array_mut())
    {
        let before = prompt_groups.len();
        prompt_groups.retain(|group| !group_has_marker(group, "memory-start-context.sh"));
        changed |= before != prompt_groups.len();
    }

    let end = hooks
        .entry("SessionEnd")
        .or_insert_with(|| serde_json::json!([]));
    if !end.is_array() {
        *end = serde_json::json!([]);
        changed = true;
    }
    let end_arr = end.as_array_mut().expect("SessionEnd array");
    if !hooks_contain(end_arr, MEMORY_CAPTURE_MARKER) {
        end_arr.push(serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": close_command,
                "timeout": 3
            }]
        }));
        changed = true;
    }

    if changed {
        MergeOutcome::Wired
    } else {
        MergeOutcome::AlreadyPresent
    }
}

fn insert_raw_guard(stop_arr: &mut Vec<serde_json::Value>) {
    let mut first_nested_group: Option<usize> = None;
    let mut preferred_group: Option<usize> = None;
    for (idx, group) in stop_arr.iter().enumerate() {
        if group.get("hooks").and_then(|h| h.as_array()).is_some() {
            if first_nested_group.is_none() {
                first_nested_group = Some(idx);
            }
            if group_has_marker(group, MEMORY_CAPTURE_MARKER) {
                preferred_group = Some(idx);
                break;
            }
        }
    }

    if let Some(gi) = preferred_group.or(first_nested_group) {
        stop_arr[gi]
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array")
            .insert(0, raw_guard_hook_entry());
    } else {
        stop_arr.insert(0, serde_json::json!({ "hooks": [raw_guard_hook_entry()] }));
    }
}

/// Merge a project-memory-capture step into `value`'s `hooks.Stop` pipeline.
///
/// The raw guard is restored as the first hook in the relevant Stop group when
/// missing. Every existing hook is preserved. Idempotent: if both raw guard and
/// capture command are already present anywhere in the Stop pipeline, the value
/// is left unchanged.
pub(in crate::setup) fn merge_memory_capture(
    value: &mut serde_json::Value,
    command: &str,
) -> MergeOutcome {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let root = value.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks_obj = hooks.as_object_mut().expect("hooks object");
    let stop = hooks_obj
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]));
    if !stop.is_array() {
        *stop = serde_json::json!([]);
    }
    let stop_arr = stop.as_array_mut().expect("stop array");

    let mut changed = false;
    if !hooks_contain(stop_arr, RAW_GUARD_MARKER) {
        insert_raw_guard(stop_arr);
        changed = true;
    }

    if hooks_contain(stop_arr, MEMORY_CAPTURE_MARKER) {
        return if changed {
            MergeOutcome::Wired
        } else {
            MergeOutcome::AlreadyPresent
        };
    }

    let entry = memory_hook_entry(command);

    let first_nested_group = stop_arr.iter().position(|group| {
        group
            .get("hooks")
            .and_then(|hooks| hooks.as_array())
            .is_some()
    });
    if let Some(gi) = first_nested_group {
        stop_arr[gi]
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array")
            .push(entry);
    } else {
        stop_arr.push(serde_json::json!({ "hooks": [entry] }));
    }

    MergeOutcome::Wired
}

/// Describe the ordering of the relevant Stop steps for diagnostics.
pub(super) fn describe_order(value: &serde_json::Value) -> String {
    let mut seen: Vec<&str> = Vec::new();
    if let Some(stop) = value
        .get("hooks")
        .and_then(|h| h.get("Stop"))
        .and_then(|s| s.as_array())
    {
        for group in stop {
            if let Some(arr) = group.get("hooks").and_then(|h| h.as_array()) {
                for h in arr {
                    if let Some(c) = command_str(h) {
                        if c.contains(RAW_GUARD_MARKER) && !seen.contains(&"raw-guard") {
                            seen.push("raw-guard");
                        } else if c.contains(MEMORY_CAPTURE_MARKER)
                            && !seen.contains(&"memory-capture")
                        {
                            seen.push("memory-capture");
                        }
                    }
                }
            }
        }
    }
    seen.join(" → ")
}
