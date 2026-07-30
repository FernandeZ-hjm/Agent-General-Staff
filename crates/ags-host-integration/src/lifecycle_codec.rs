use crate::{platform_spec, HostLifecycleSpec, LifecycleProjectionFamily};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const OMP_LIFECYCLE_TEMPLATE: &str = include_str!("../../../scripts/ags-memory-lifecycle-omp.js");
const LEGACY_MARKERS: &[&str] = &[
    "context-memory-start.py",
    "claude-stop-memory-capture.py",
    "raw-tool-call-stop-guard.js",
    "memory-start-context.sh",
    "context-memory.sh",
    "stop-archive-hook.sh",
    ".hookSpecificOutput == null then del(.hookSpecificOutput)",
];
const DRIFT_MARKERS: &[&str] = &["--target .", "ctx.cwd"];
const RETIRED_CLAUDE_STOP_ADAPTER_PREFIX: &str =
    "set -o pipefail; ags host lifecycle --event stop-guard --host claude-code --target ";
const RETIRED_CLAUDE_STOP_ADAPTER_SUFFIX: &str =
    " | jq -c 'if .hookSpecificOutput == null then del(.hookSpecificOutput) else . end'";
const LEGACY_COMMANDS: &[&str] = &[
    r#"python3 "$HOME/.agents/scripts/context-memory-start.py""#,
    r#"python3 "$HOME/.agents/scripts/claude-stop-memory-capture.py""#,
    r#"node "$HOME/.agents/scripts/raw-tool-call-stop-guard.js""#,
    r#"bash "$HOME/.agents/scripts/memory-start-context.sh""#,
    r#"bash "$HOME/.agents/scripts/context-memory.sh""#,
    r#"bash "$HOME/.agents/scripts/stop-archive-hook.sh""#,
];

pub type OwnedLifecycleProjection = BTreeMap<String, Vec<serde_json::Value>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEventObservation {
    pub session_start: bool,
    pub stop_guard: bool,
    pub session_end: bool,
}

impl LifecycleEventObservation {
    pub fn complete(&self) -> bool {
        self.session_start && self.stop_guard && self.session_end
    }

    pub fn any(&self) -> bool {
        self.session_start || self.stop_guard || self.session_end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleCodecObservation {
    pub desired_hash: String,
    pub observed_hash: Option<String>,
    pub events: LifecycleEventObservation,
    pub canonical_target: bool,
    pub current: bool,
    pub detail: String,
}

/// Pure host-format codec for one canonical workspace.
///
/// It owns the exact AGS hook entries and their observation semantics. File
/// reads, merge policy, installation, and manifests remain lifecycle concerns.
#[derive(Debug, Clone)]
pub struct HostLifecycleCodec {
    workspace: PathBuf,
    spec: HostLifecycleSpec,
}

impl HostLifecycleCodec {
    pub fn new(workspace: &Path, host: &str) -> Result<Self, String> {
        let workspace = ags_platform::canonical_workspace_root(workspace)?;
        let spec = platform_spec(host)
            .and_then(|platform| platform.lifecycle)
            .ok_or_else(|| format!("unsupported lifecycle host `{host}`"))?;
        Ok(Self { workspace, spec })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn spec(&self) -> HostLifecycleSpec {
        self.spec
    }

    pub fn path(&self) -> PathBuf {
        self.spec.workspace_config_path(&self.workspace)
    }

    pub fn desired_hash(&self) -> String {
        match self.spec.projection_family {
            LifecycleProjectionFamily::OmpExtension => {
                ags_platform::sha256_hex(self.desired_omp_body().as_bytes())
            }
            LifecycleProjectionFamily::CommandHooks
            | LifecycleProjectionFamily::CursorCommandHooks => {
                hash_owned_projection(&self.desired_owned_projection())
            }
        }
    }

    pub fn desired_omp_body(&self) -> String {
        OMP_LIFECYCLE_TEMPLATE.replace(
            "__AGS_CANONICAL_WORKSPACE__",
            &serde_json::to_string(&self.workspace.to_string_lossy())
                .unwrap_or_else(|_| "\"\"".to_string()),
        )
    }

    pub fn desired_owned_projection(&self) -> OwnedLifecycleProjection {
        lifecycle_events(self.spec)
            .into_iter()
            .map(|(native_event, event, timeout)| {
                (
                    native_event.to_string(),
                    vec![owned_hook_group(
                        self.spec.projection_family,
                        lifecycle_command(event, self.spec.host_id, &self.workspace),
                        timeout,
                    )],
                )
            })
            .collect()
    }

    pub fn observe_body(&self, body: &str) -> Result<LifecycleCodecObservation, String> {
        let desired_hash = self.desired_hash();
        match self.spec.projection_family {
            LifecycleProjectionFamily::OmpExtension => {
                let desired = self.desired_omp_body();
                let current = body == desired;
                let serialized_workspace = serde_json::to_string(&self.workspace.to_string_lossy())
                    .unwrap_or_else(|_| "\"\"".to_string());
                Ok(LifecycleCodecObservation {
                    desired_hash,
                    observed_hash: Some(ags_platform::sha256_hex(body.as_bytes())),
                    events: LifecycleEventObservation {
                        session_start: current,
                        stop_guard: current,
                        session_end: current,
                    },
                    canonical_target: body.contains(&serialized_workspace)
                        && text_drift_markers(body).is_empty(),
                    current,
                    detail: if current {
                        "desired projection equals observed projection".to_string()
                    } else {
                        "OMP extension differs from the canonical generator".to_string()
                    },
                })
            }
            LifecycleProjectionFamily::CommandHooks
            | LifecycleProjectionFamily::CursorCommandHooks => {
                let value: serde_json::Value = serde_json::from_str(body)
                    .map_err(|error| format!("invalid lifecycle projection JSON: {error}"))?;
                let desired = self.desired_owned_projection();
                let observed = owned_hook_projection(&value, self.spec.host_id);
                let events = LifecycleEventObservation {
                    session_start: event_matches(
                        &desired,
                        &observed,
                        self.spec.native_events.session_start,
                    ),
                    stop_guard: event_matches(
                        &desired,
                        &observed,
                        self.spec.native_events.stop_guard,
                    ),
                    session_end: event_matches(
                        &desired,
                        &observed,
                        self.spec.native_events.session_end,
                    ),
                };
                let observed_hash =
                    (!observed.is_empty()).then(|| hash_owned_projection(&observed));
                let events_complete = events.complete();
                let commands = collect_current_commands(&value, self.spec.host_id);
                let canonical_target = !commands.is_empty()
                    && commands.iter().all(|command| {
                        command.contains(&self.workspace.to_string_lossy().to_string())
                            && text_drift_markers(command).is_empty()
                    });
                let current = observed_hash.as_deref() == Some(desired_hash.as_str())
                    && events_complete
                    && canonical_target;
                Ok(LifecycleCodecObservation {
                    desired_hash,
                    observed_hash,
                    events,
                    canonical_target,
                    current,
                    detail: if current {
                        "desired AGS commands equal observed commands".to_string()
                    } else {
                        format!(
                            "events_complete={}, canonical_target={canonical_target}",
                            events_complete
                        )
                    },
                })
            }
        }
    }
}

fn text_drift_markers(body: &str) -> Vec<&'static str> {
    LEGACY_MARKERS
        .iter()
        .chain(DRIFT_MARKERS)
        .copied()
        .filter(|marker| body.contains(marker))
        .collect()
}

pub fn lifecycle_config_drift_markers(spec: HostLifecycleSpec, body: &str) -> Vec<&'static str> {
    if spec.projection_family == LifecycleProjectionFamily::OmpExtension {
        return text_drift_markers(body);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    collect_commands(&value, &mut commands);
    commands
        .iter()
        .filter(|command| lifecycle_command_is_owned(spec.host_id, command))
        .flat_map(|command| text_drift_markers(command))
        .collect()
}

pub fn lifecycle_body_contains_owned(spec: HostLifecycleSpec, body: &str) -> bool {
    if spec.projection_family == LifecycleProjectionFamily::OmpExtension {
        return body.contains("ags")
            && body.contains("host")
            && body.contains("lifecycle")
            && body.contains("session-start");
    }
    body.contains("ags host lifecycle") && body.contains(&format!("--host {}", spec.host_id))
        || spec.host_id == "claude-code"
            && LEGACY_MARKERS.iter().any(|marker| body.contains(marker))
}

pub fn lifecycle_command_is_owned(host: &str, command: &str) -> bool {
    current_owned_command(command, host)
        || host == "claude-code"
            && (LEGACY_COMMANDS.contains(&command.trim()) || retired_claude_stop_adapter(command))
}

fn retired_claude_stop_adapter(command: &str) -> bool {
    command
        .trim()
        .strip_prefix(RETIRED_CLAUDE_STOP_ADAPTER_PREFIX)
        .and_then(|rest| rest.strip_suffix(RETIRED_CLAUDE_STOP_ADAPTER_SUFFIX))
        .is_some_and(valid_single_shell_word)
}

pub fn remove_owned_lifecycle_entries(value: &mut serde_json::Value, host: &str) -> bool {
    let before = value.clone();
    if let Some(hooks) = value
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
    {
        hooks
            .values_mut()
            .filter_map(serde_json::Value::as_array_mut)
            .for_each(|groups| {
                groups.retain_mut(|group| {
                    retain_hook_group(
                        group,
                        &|command| lifecycle_command_is_owned(host, command),
                        false,
                    )
                });
            });
    }
    *value != before
}

pub fn lifecycle_owned_event_counts(spec: HostLifecycleSpec, body: &str) -> [usize; 3] {
    if spec.projection_family == LifecycleProjectionFamily::OmpExtension {
        return [
            usize::from(body.contains("session-start")),
            usize::from(body.contains("stop-guard")),
            usize::from(body.contains("session-end")),
        ];
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return [0; 3];
    };
    let commands = collect_current_commands(&value, spec.host_id);
    ["session-start", "stop-guard", "session-end"].map(|event| {
        commands
            .iter()
            .filter(|command| command.contains(&format!("--event {event}")))
            .count()
    })
}

fn lifecycle_events(spec: HostLifecycleSpec) -> [(&'static str, &'static str, u64); 3] {
    [
        (spec.native_events.session_start, "session-start", 5),
        (spec.native_events.stop_guard, "stop-guard", 2),
        (spec.native_events.session_end, "session-end", 3),
    ]
}

fn lifecycle_command(event: &str, host: &str, workspace: &Path) -> String {
    format!(
        "ags host lifecycle --event {event} --host {host} --target {}",
        shell_quote(&workspace.to_string_lossy())
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn owned_hook_group(
    family: LifecycleProjectionFamily,
    command: String,
    timeout: u64,
) -> serde_json::Value {
    let entry = serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": timeout,
    });
    if family == LifecycleProjectionFamily::CursorCommandHooks {
        entry
    } else {
        serde_json::json!({"hooks": [entry]})
    }
}

fn event_matches(
    desired: &OwnedLifecycleProjection,
    observed: &OwnedLifecycleProjection,
    event: &str,
) -> bool {
    desired.get(event) == observed.get(event)
}

fn current_owned_command(command: &str, host: &str) -> bool {
    let Some(rest) = command.trim().strip_prefix("ags host lifecycle --event ") else {
        return false;
    };
    let Some((event, rest)) = rest.split_once(" --host ") else {
        return false;
    };
    if !matches!(event, "session-start" | "stop-guard" | "session-end") {
        return false;
    }
    let Some((observed_host, target)) = rest.split_once(" --target ") else {
        return false;
    };
    observed_host == host && valid_single_shell_word(target)
}

fn valid_single_shell_word(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        Unquoted,
        Single,
        Double,
    }
    let mut quote = Quote::Unquoted;
    for character in value.chars() {
        quote = match (quote, character) {
            (Quote::Unquoted, '\'') => Quote::Single,
            (Quote::Unquoted, '"') => Quote::Double,
            (Quote::Single, '\'') => Quote::Unquoted,
            (Quote::Double, '"') => Quote::Unquoted,
            (Quote::Unquoted, character)
                if character.is_whitespace()
                    || matches!(
                        character,
                        '&' | '|' | ';' | '<' | '>' | '`' | '$' | '(' | ')'
                    ) =>
            {
                return false;
            }
            (_, '\n' | '\r') => return false,
            (current, _) => current,
        };
    }
    quote == Quote::Unquoted
}

fn collect_current_commands(value: &serde_json::Value, host: &str) -> Vec<String> {
    let mut commands = Vec::new();
    collect_commands(value, &mut commands);
    commands.retain(|command| current_owned_command(command, host));
    commands
}

fn collect_commands(value: &serde_json::Value, commands: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(command) = object.get("command").and_then(serde_json::Value::as_str) {
                commands.push(command.to_string());
            }
            object
                .values()
                .for_each(|child| collect_commands(child, commands));
        }
        serde_json::Value::Array(items) => items
            .iter()
            .for_each(|item| collect_commands(item, commands)),
        _ => {}
    }
}

fn owned_hook_projection(value: &serde_json::Value, host: &str) -> OwnedLifecycleProjection {
    let mut projection = OwnedLifecycleProjection::new();
    let Some(hooks) = value.get("hooks").and_then(serde_json::Value::as_object) else {
        return projection;
    };
    for (event, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let mut owned = group.clone();
            if retain_hook_group(
                &mut owned,
                &|command| current_owned_command(command, host),
                true,
            ) {
                projection.entry(event.clone()).or_default().push(owned);
            }
        }
    }
    projection
        .values_mut()
        .for_each(|groups| groups.sort_by_key(|group| group.to_string()));
    projection
}

fn retain_hook_group(
    group: &mut serde_json::Value,
    is_owned: &impl Fn(&str) -> bool,
    retain_owned: bool,
) -> bool {
    let command_is_owned =
        |value: &serde_json::Value| value["command"].as_str().is_some_and(is_owned);
    if command_is_owned(group) {
        return retain_owned;
    }
    let Some(entries) = group
        .get_mut("hooks")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return !retain_owned;
    };
    entries.retain(|entry| command_is_owned(entry) == retain_owned);
    let has_entries = !entries.is_empty();
    has_entries
        || !retain_owned
            && group
                .as_object()
                .is_some_and(|object| object.keys().any(|key| key != "hooks"))
}

fn hash_owned_projection(projection: &OwnedLifecycleProjection) -> String {
    ags_platform::sha256_hex(
        serde_json::to_vec(projection).expect("lifecycle hook JSON is always serializable"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        root
    }

    #[test]
    fn five_host_codecs_reject_entry_schema_and_target_drift() {
        let root = workspace();
        for spec in crate::lifecycle_specs() {
            let codec = HostLifecycleCodec::new(root.path(), spec.host_id).unwrap();
            let body = match spec.projection_family {
                LifecycleProjectionFamily::OmpExtension => codec.desired_omp_body(),
                _ => serde_json::json!({"hooks": codec.desired_owned_projection()}).to_string(),
            };
            let current = codec.observe_body(&body).unwrap();
            assert!(current.current, "{}: {}", spec.host_id, current.detail);
            assert!(current.events.complete());
            assert!(current.canonical_target);

            let drifted = match spec.projection_family {
                LifecycleProjectionFamily::OmpExtension => body.replace("3000", "3001"),
                _ => body.replacen("\"timeout\":2", "\"timeout\":99", 1),
            };
            assert!(!codec.observe_body(&drifted).unwrap().current);
        }
    }

    #[test]
    fn command_codec_requires_native_event_wrapper_type_timeout_and_absolute_target() {
        let root = workspace();
        let codec = HostLifecycleCodec::new(root.path(), "claude-code").unwrap();
        let body = serde_json::json!({"hooks": codec.desired_owned_projection()}).to_string();
        for (label, drifted) in [
            (
                "native event",
                body.replacen("\"Stop\"", "\"WrongStop\"", 1),
            ),
            ("wrapper", body.replacen("\"hooks\"", "\"wrongWrapper\"", 1)),
            (
                "entry type",
                body.replacen("\"type\":\"command\"", "\"type\":\"wrong\"", 1),
            ),
            (
                "timeout",
                body.replacen("\"timeout\":2", "\"timeout\":99", 1),
            ),
            (
                "target",
                body.replace(
                    serde_json::to_string(&format!("--target '{}'", codec.workspace().display()))
                        .unwrap()
                        .trim_matches('"'),
                    "--target .",
                ),
            ),
        ] {
            assert!(!codec.observe_body(&drifted).unwrap().current, "{label}");
        }
    }

    #[test]
    fn compound_user_commands_are_never_claimed_as_ags_owned() {
        let root = workspace();
        let codec = HostLifecycleCodec::new(root.path(), "claude-code").unwrap();
        let projection = codec.desired_owned_projection();
        let command = projection["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(lifecycle_command_is_owned("claude-code", command));
        assert!(!lifecycle_command_is_owned(
            "claude-code",
            &format!("{command} && ./notify.sh")
        ));
        assert!(!lifecycle_command_is_owned(
            "claude-code",
            &format!("echo before; {command}")
        ));
        assert!(lifecycle_command_is_owned(
            "claude-code",
            r#"python3 "$HOME/.agents/scripts/context-memory-start.py""#
        ));
        assert!(!lifecycle_command_is_owned(
            "claude-code",
            r#"python3 "$HOME/.agents/scripts/context-memory-start.py" && ./notify.sh"#
        ));
    }

    #[test]
    fn exact_retired_macbook_jq_adapter_is_removed_without_claiming_user_pipelines() {
        let retired = concat!(
            "set -o pipefail; ags host lifecycle --event stop-guard ",
            "--host claude-code --target . | jq -c 'if .hookSpecificOutput == null ",
            "then del(.hookSpecificOutput) else . end'"
        );
        assert!(lifecycle_command_is_owned("claude-code", retired));
        assert!(!lifecycle_command_is_owned(
            "claude-code",
            &format!("{retired} && ./notify.sh")
        ));
        assert!(!lifecycle_command_is_owned(
            "claude-code",
            &retired.replace("del(.hookSpecificOutput)", "del(.reason)")
        ));

        let mut config = serde_json::json!({
            "hooks": {
                "Stop": [{
                    "hooks": [
                        {"type": "command", "command": retired},
                        {"type": "command", "command": "./notify.sh"}
                    ]
                }]
            },
            "mcpServers": {"user-owned": {"command": "user-mcp"}}
        });
        assert!(remove_owned_lifecycle_entries(&mut config, "claude-code"));
        assert_eq!(
            config["hooks"]["Stop"][0]["hooks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            config["hooks"]["Stop"][0]["hooks"][0]["command"],
            "./notify.sh"
        );
        assert_eq!(config["mcpServers"]["user-owned"]["command"], "user-mcp");
    }

    #[test]
    fn omp_production_path_centralizes_one_session_identity_for_all_three_events() {
        let root = workspace();
        let body = HostLifecycleCodec::new(root.path(), "omp")
            .unwrap()
            .desired_omp_body();
        assert_eq!(body.matches("session_id:").count(), 1);
        assert!(body.contains("contextSessionId || payload.session_id || \"\""));
        for call in [
            "lifecycle(\"session-start\", ctx, event)",
            "lifecycle(\"stop-guard\", ctx, event)",
            "lifecycle(\"session-end\", ctx, event)",
        ] {
            assert!(body.contains(call), "{call}");
        }
    }

    #[test]
    fn config_drift_only_scans_owned_lifecycle_commands() {
        let spec = crate::platform_spec("claude-code")
            .unwrap()
            .lifecycle
            .unwrap();
        let user_only = serde_json::json!({
            "hooks": {"Stop": [{"command": "echo --target ."}]}
        })
        .to_string();
        assert!(lifecycle_config_drift_markers(spec, &user_only).is_empty());

        let relative_ags = serde_json::json!({
            "hooks": {"Stop": [{
                "hooks": [{
                    "type": "command",
                    "command": "ags host lifecycle --event stop-guard --host claude-code --target .",
                    "timeout": 2
                }]
            }]}
        })
        .to_string();
        assert_eq!(
            lifecycle_config_drift_markers(spec, &relative_ags),
            ["--target ."]
        );
    }
}
