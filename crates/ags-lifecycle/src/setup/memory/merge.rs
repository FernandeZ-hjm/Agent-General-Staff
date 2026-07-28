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

const LEGACY_AGS_MEMORY_MARKERS: &[&str] = &[
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
    "raw-tool-call-stop-guard.js",
    "memory-start-context.sh",
    "context-memory.sh",
    "stop-archive-hook.sh",
];

fn is_legacy_ags_memory_hook(hook: &serde_json::Value) -> bool {
    command_str(hook).is_some_and(|command| {
        LEGACY_AGS_MEMORY_MARKERS
            .iter()
            .any(|marker| command.contains(marker))
    })
}

fn retire_legacy_ags_memory_hooks(groups: &mut Vec<serde_json::Value>) -> bool {
    let before = serde_json::to_vec(groups).unwrap_or_default();
    let mut kept = Vec::with_capacity(groups.len());
    for mut group in groups.drain(..) {
        if is_legacy_ags_memory_hook(&group) {
            continue;
        }
        if let Some(nested) = group
            .get_mut("hooks")
            .and_then(|hooks| hooks.as_array_mut())
        {
            nested.retain(|hook| !is_legacy_ags_memory_hook(hook));
            let ags_only_empty_group = nested.is_empty()
                && group
                    .as_object()
                    .is_some_and(|object| object.keys().all(|key| key == "hooks"));
            if ags_only_empty_group {
                continue;
            }
        }
        kept.push(group);
    }
    *groups = kept;
    serde_json::to_vec(groups).unwrap_or_default() != before
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

fn raw_guard_hook_entry(command: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": command,
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
    let changed = retire_legacy_ags_memory_hooks(start_arr);

    if hooks_contain(start_arr, MEMORY_START_MARKER) {
        return if changed {
            MergeOutcome::Wired
        } else {
            MergeOutcome::AlreadyPresent
        };
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
    guard_command: &str,
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
        changed |= retire_legacy_ags_memory_hooks(prompt_groups);
    }

    let end = hooks
        .entry("SessionEnd")
        .or_insert_with(|| serde_json::json!([]));
    if !end.is_array() {
        *end = serde_json::json!([]);
        changed = true;
    }
    let end_arr = end.as_array_mut().expect("SessionEnd array");
    changed |= retire_legacy_ags_memory_hooks(end_arr);
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

    let stop = hooks.entry("Stop").or_insert_with(|| serde_json::json!([]));
    if !stop.is_array() {
        *stop = serde_json::json!([]);
        changed = true;
    }
    let stop_arr = stop.as_array_mut().expect("Stop array");
    changed |= retire_legacy_ags_memory_hooks(stop_arr);
    if !hooks_contain(stop_arr, RAW_GUARD_MARKER) {
        stop_arr.insert(
            0,
            serde_json::json!({ "hooks": [raw_guard_hook_entry(guard_command)] }),
        );
        changed = true;
    }

    if changed {
        MergeOutcome::Wired
    } else {
        MergeOutcome::AlreadyPresent
    }
}

fn insert_raw_guard(stop_arr: &mut Vec<serde_json::Value>, command: &str) {
    let mut first_nested_group: Option<usize> = None;
    let mut preferred_group: Option<usize> = None;
    for (idx, group) in stop_arr.iter().enumerate() {
        if group.get("hooks").and_then(|h| h.as_array()).is_some() {
            if first_nested_group.is_none() {
                first_nested_group = Some(idx);
            }
            if group_has_marker(group, MEMORY_CAPTURE_MARKER)
                || group_has_marker(group, EVOLVER_MARKER)
            {
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
            .insert(0, raw_guard_hook_entry(command));
    } else {
        stop_arr.insert(
            0,
            serde_json::json!({ "hooks": [raw_guard_hook_entry(command)] }),
        );
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
    guard_command: &str,
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

    let mut changed = retire_legacy_ags_memory_hooks(stop_arr);
    if !hooks_contain(stop_arr, RAW_GUARD_MARKER) {
        insert_raw_guard(stop_arr, guard_command);
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

    let mut evolver_group: Option<usize> = None;
    let mut first_nested_group: Option<usize> = None;
    for (idx, group) in stop_arr.iter().enumerate() {
        if let Some(arr) = group.get("hooks").and_then(|hooks| hooks.as_array()) {
            if first_nested_group.is_none() {
                first_nested_group = Some(idx);
            }
            if arr.iter().any(|hook| hook_has_marker(hook, EVOLVER_MARKER)) {
                evolver_group = Some(idx);
                break;
            }
        }
    }

    if let Some(gi) = evolver_group {
        let hooks = stop_arr[gi]
            .get_mut("hooks")
            .and_then(|hooks| hooks.as_array_mut())
            .expect("nested hooks array");
        let position = hooks
            .iter()
            .position(|hook| hook_has_marker(hook, EVOLVER_MARKER))
            .unwrap_or(hooks.len());
        hooks.insert(position, entry);
    } else if let Some(gi) = first_nested_group {
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

/// Merge Cursor's native flat command-hook format while preserving all
/// non-AGS entries.
pub(crate) fn merge_cursor_memory_lifecycle(
    value: &mut serde_json::Value,
    start_command: &str,
    close_command: &str,
    guard_command: &str,
) -> Result<MergeOutcome, String> {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let root = value.as_object_mut().expect("object");
    match root.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) | None => {}
        Some(version) => {
            return Err(format!(
                "unsupported Cursor hooks version {version}; expected version 1"
            ));
        }
    }
    root.entry("version").or_insert(serde_json::json!(1));
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        return Err("Cursor hooks field must be an object".to_string());
    }
    let hooks = hooks.as_object_mut().expect("hooks object");
    let mut changed = false;
    for (event, command, marker, timeout) in [
        ("sessionStart", start_command, MEMORY_START_MARKER, 5),
        ("sessionEnd", close_command, MEMORY_CAPTURE_MARKER, 3),
        ("stop", guard_command, RAW_GUARD_MARKER, 2),
    ] {
        let entries = hooks.entry(event).or_insert_with(|| serde_json::json!([]));
        if !entries.is_array() {
            return Err(format!("Cursor hooks.{event} must be an array"));
        }
        let entries = entries.as_array_mut().expect("event array");
        if !hooks_contain(entries, marker) {
            entries.push(serde_json::json!({
                "type": "command",
                "command": command,
                "timeout": timeout,
            }));
            changed = true;
        }
    }
    Ok(if changed {
        MergeOutcome::Wired
    } else {
        MergeOutcome::AlreadyPresent
    })
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
                        } else if c.contains(EVOLVER_MARKER) && !seen.contains(&"evolver") {
                            seen.push("evolver");
                        }
                    }
                }
            }
        }
    }
    seen.join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop_commands(value: &serde_json::Value) -> Vec<&str> {
        value["hooks"]["Stop"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
            .filter_map(command_str)
            .collect()
    }

    #[test]
    fn memory_and_evolver_stop_hooks_coexist_in_safe_order() {
        let mut settings = serde_json::json!({
            "project_owned": {"keep": true},
            "hooks": {
                "Stop": [{
                    "hooks": [
                        {"type": "command", "command": "echo project-owned"},
                        {"type": "command", "command": "node .claude/hooks/evolver-session-end.js"}
                    ]
                }]
            }
        });

        assert_eq!(
            merge_memory_capture(
                &mut settings,
                &memory_capture_command("claude-code"),
                &raw_guard_command("claude-code")
            ),
            MergeOutcome::Wired
        );
        let commands = stop_commands(&settings);
        let raw = commands
            .iter()
            .position(|command| command.contains(RAW_GUARD_MARKER))
            .unwrap();
        let memory = commands
            .iter()
            .position(|command| command.contains(MEMORY_CAPTURE_MARKER))
            .unwrap();
        let evolver = commands
            .iter()
            .position(|command| command.contains(EVOLVER_MARKER))
            .unwrap();

        assert!(raw < memory && memory < evolver);
        assert!(commands.contains(&"echo project-owned"));
        assert_eq!(settings["project_owned"]["keep"], true);
        assert_eq!(
            merge_memory_capture(
                &mut settings,
                &memory_capture_command("claude-code"),
                &raw_guard_command("claude-code")
            ),
            MergeOutcome::AlreadyPresent
        );
    }

    #[test]
    fn cursor_lifecycle_is_flat_idempotent_and_preserves_existing_hooks() {
        let mut hooks = serde_json::json!({
            "version": 1,
            "project_owned": true,
            "hooks": {
                "sessionStart": [{"type": "command", "command": "echo keep-start"}],
                "stop": [{"type": "command", "command": "echo keep-stop"}]
            }
        });
        assert_eq!(
            merge_cursor_memory_lifecycle(
                &mut hooks,
                &memory_start_command("cursor"),
                &memory_capture_command("cursor"),
                &raw_guard_command("cursor"),
            )
            .unwrap(),
            MergeOutcome::Wired
        );
        assert_eq!(hooks["project_owned"], true);
        for (event, marker, host) in [
            ("sessionStart", MEMORY_START_MARKER, "--host cursor"),
            ("sessionEnd", MEMORY_CAPTURE_MARKER, "--host cursor"),
            ("stop", RAW_GUARD_MARKER, "--host cursor"),
        ] {
            let entries = hooks["hooks"][event].as_array().unwrap();
            assert!(hooks_contain(entries, marker));
            assert!(entries
                .iter()
                .any(|entry| command_str(entry).is_some_and(|command| command.contains(host))));
        }
        assert_eq!(
            merge_cursor_memory_lifecycle(
                &mut hooks,
                &memory_start_command("cursor"),
                &memory_capture_command("cursor"),
                &raw_guard_command("cursor"),
            )
            .unwrap(),
            MergeOutcome::AlreadyPresent
        );
    }

    #[test]
    fn cursor_merge_fails_closed_on_unknown_schema() {
        let mut hooks = serde_json::json!({"version": 2, "hooks": {}});
        let original = hooks.clone();
        assert!(merge_cursor_memory_lifecycle(
            &mut hooks,
            &memory_start_command("cursor"),
            &memory_capture_command("cursor"),
            &raw_guard_command("cursor"),
        )
        .is_err());
        assert_eq!(hooks, original);
    }

    #[test]
    fn legacy_wiring_is_replaced_without_removing_evolver_or_user_hooks() {
        let mut settings = serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [
                        {"command": "node keep-start.js"},
                        {"command": "$HOME/.agents/scripts/context-memory-start.py"}
                    ]
                }],
                "Stop": [{
                    "hooks": [
                        {"command": "node \"$HOME/.agents/scripts/raw-tool-call-stop-guard.js\""},
                        {"command": "$HOME/.agents/scripts/claude-stop-memory-capture.py"},
                        {"command": "node .claude/hooks/evolver-session-end.js"},
                        {"command": "node keep-stop.js"}
                    ]
                }]
            }
        });

        assert_eq!(
            merge_memory_start(&mut settings, &memory_start_command("claude-code")),
            MergeOutcome::Wired
        );
        assert_eq!(
            merge_memory_capture(
                &mut settings,
                &memory_capture_command("claude-code"),
                &raw_guard_command("claude-code"),
            ),
            MergeOutcome::Wired
        );
        let rendered = settings.to_string();
        for marker in LEGACY_AGS_MEMORY_MARKERS {
            assert!(!rendered.contains(marker));
        }
        assert!(rendered.contains("keep-start.js"));
        assert!(rendered.contains("keep-stop.js"));
        assert!(rendered.contains("evolver-session-end.js"));
        assert!(rendered.contains("--host claude-code"));
    }
}
