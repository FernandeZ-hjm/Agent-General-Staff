//! Host connection and lifecycle projection owned by AGS.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::error::{Error, Result};

const AGS_COMMANDS: &[&str] = &["ags-policy", "ags-host"];
const LEGACY_HOOK_PATHS: &[&str] = &[".claude/settings.local.json"];

struct HookSpec {
    path: &'static str,
    host: &'static str,
    stop_event: &'static str,
}

const HOOK_SPECS: &[HookSpec] = &[
    HookSpec {
        path: ".claude/settings.json",
        host: "claude-code",
        stop_event: "Stop",
    },
    HookSpec {
        path: ".codex/hooks.json",
        host: "codex",
        stop_event: "SessionEnd",
    },
    HookSpec {
        path: ".cursor/hooks.json",
        host: "cursor",
        stop_event: "Stop",
    },
    HookSpec {
        path: ".codebuddy/settings.local.json",
        host: "codebuddy",
        stop_event: "Stop",
    },
];

pub type FileSnapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

pub fn snapshot_workspace_hooks(root: &Path) -> Result<FileSnapshot> {
    HOOK_SPECS
        .iter()
        .map(|spec| spec.path)
        .chain(LEGACY_HOOK_PATHS.iter().copied())
        .map(|relative| {
            crate::projection::reject_symlink_path(root, Path::new(relative))?;
            let path = root.join(relative);
            let bytes = read_optional_file(&path, "host_hook_snapshot_failed")?;
            Ok((path, bytes))
        })
        .collect()
}

pub fn restore_files(snapshot: &FileSnapshot) -> Result<()> {
    for (path, bytes) in snapshot {
        match bytes {
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| crate::error::io("host_projection_rollback_failed", &e))?;
                }
                fs::write(path, bytes)
                    .map_err(|e| crate::error::io("host_projection_rollback_failed", &e))?;
            }
            None => {
                if path.is_file() {
                    fs::remove_file(path)
                        .map_err(|e| crate::error::io("host_projection_rollback_failed", &e))?;
                }
            }
        }
    }
    Ok(())
}

pub fn read_optional_file(path: &Path, code: &'static str) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::error::io(code, &error)),
    }
}

pub fn sync_workspace_hooks(root: &Path) -> Result<Vec<String>> {
    let mut writes = clean_legacy_hooks(root)?;
    for spec in HOOK_SPECS {
        crate::projection::reject_symlink_path(root, Path::new(spec.path))?;
        let path = root.join(spec.path);
        let current = if path.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|e| crate::error::io("host_hook_read_failed", &e))?;
            serde_json::from_str::<Value>(&text).map_err(|e| {
                Error::new("host_hook_parse_failed", format!("{}: {e}", path.display()))
            })?
        } else {
            json!({})
        };
        let next = merge_hooks(current, spec.host, spec.stop_event)?;
        let text = serde_json::to_string_pretty(&next)
            .map_err(|e| Error::new("host_hook_encode_failed", e.to_string()))?;
        let mut text = text;
        text.push('\n');
        if fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| crate::error::io("host_hook_dir_failed", &e))?;
        }
        let tmp = path.with_extension("ags.tmp");
        fs::write(&tmp, text).map_err(|e| crate::error::io("host_hook_write_failed", &e))?;
        fs::rename(&tmp, &path).map_err(|e| crate::error::io("host_hook_write_failed", &e))?;
        writes.push(format!("host-hook:{}", spec.path));
    }
    Ok(writes)
}

pub fn preflight_workspace_hooks(root: &Path) -> Result<()> {
    for relative in HOOK_SPECS
        .iter()
        .map(|spec| spec.path)
        .chain(LEGACY_HOOK_PATHS.iter().copied())
    {
        crate::projection::reject_symlink_path(root, Path::new(relative))?;
        let path = root.join(relative);
        if !path.is_file() {
            continue;
        }
        let text =
            fs::read_to_string(&path).map_err(|e| crate::error::io("host_hook_read_failed", &e))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            Error::new("host_hook_parse_failed", format!("{}: {e}", path.display()))
        })?;
        if value.get("hooks").is_some() && value.get("hooks").and_then(Value::as_object).is_none() {
            return Err(Error::new(
                "host_hook_shape_invalid",
                format!("{} hooks must be an object", path.display()),
            ));
        }
    }
    Ok(())
}

pub fn experience_status(root: &Path, config: &crate::config::Config) -> Result<Value> {
    let mut hook_status = Vec::new();
    for spec in HOOK_SPECS {
        if !config.hosts.iter().any(|host| {
            crate::hosts::normalize_host_id(&host.id).ok().as_deref() == Some(spec.host)
        }) {
            continue;
        }
        let text = fs::read_to_string(root.join(spec.path)).unwrap_or_default();
        hook_status.push(json!({
            "host": spec.host,
            "path": spec.path,
            "session_start": text.contains("ags-host --event session-start"),
            "session_end": text.contains("ags-host --event session-end"),
        }));
    }
    let hooks_ready = hook_status.iter().all(|status| {
        status.get("session_start").and_then(Value::as_bool) == Some(true)
            && status.get("session_end").and_then(Value::as_bool) == Some(true)
    });
    let slug_aligned = crate::config::valid_slug(&config.workspace.slug);
    let memory = legacy_memory_dir(&config.workspace.slug)?;
    let clients: Vec<Value> = config
        .hosts
        .iter()
        .filter_map(|host| {
            let id = crate::hosts::normalize_host_id(&host.id).ok()?;
            if !matches!(id.as_str(), "codex" | "claude-code")
                || !matches!(host.surface.as_str(), "mcp" | "hybrid")
            {
                return None;
            }
            Some(json!({"host": id, "mcp_ready": client_mcp_ready(&id)}))
        })
        .collect();
    let clients_ready = clients
        .iter()
        .all(|client| client.get("mcp_ready").and_then(Value::as_bool) == Some(true));
    let legacy_hooks_clean = legacy_hooks_clean(root);
    Ok(json!({
        "healthy": hooks_ready && clients_ready && slug_aligned && legacy_hooks_clean,
        "slug_aligned": slug_aligned,
        "hooks_ready": hooks_ready,
        "hooks": hook_status,
        "clients_ready": clients_ready,
        "clients": clients,
        "legacy_hooks_clean": legacy_hooks_clean,
        "memory_store": memory,
        "context_capsule_present": memory.join("context-capsule.md").is_file(),
        "task_memory_present": memory.join("task-memory.md").is_file(),
    }))
}

fn client_mcp_ready(host: &str) -> bool {
    match host {
        "codex" => codex_mcp_ready(),
        "claude-code" => claude_mcp_ready(),
        _ => true,
    }
}

fn claude_mcp_ready() -> bool {
    let Ok(home) = crate::sync::machine_home() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(home.join(".claude.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let Some((command, args_ok)) = claude_mcp_registration(&value) else {
        return false;
    };
    let command_ok = match (fs::canonicalize(command), fs::canonicalize(canonical_ags())) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    };
    let version_ok = Command::new(command)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("v0.4.21"))
        .unwrap_or(false);
    command_ok && args_ok && version_ok
}

fn claude_mcp_registration(value: &Value) -> Option<(&str, bool)> {
    let ags = value.get("mcpServers")?.get("ags")?;
    let command = ags.get("command")?.as_str()?;
    let args_ok = ags
        .get("args")
        .and_then(Value::as_array)
        .map(|args| args.len() == 1 && args[0].as_str() == Some("mcp"))
        .unwrap_or(false);
    Some((command, args_ok))
}

fn codex_mcp_ready() -> bool {
    let Ok(home) = crate::sync::machine_home() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(home.join(".codex/config.toml")) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    let Some(ags) = value.get("mcp_servers").and_then(|value| value.get("ags")) else {
        return false;
    };
    let registered_command = ags.get("command").and_then(toml::Value::as_str);
    let command_ok = registered_command
        .map(|command| {
            let expected = canonical_ags();
            match (fs::canonicalize(command), fs::canonicalize(expected)) {
                (Ok(actual), Ok(expected)) => actual == expected,
                _ => false,
            }
        })
        .unwrap_or(false);
    let args_ok = ags
        .get("args")
        .and_then(toml::Value::as_array)
        .map(|args| args.len() == 1 && args[0].as_str() == Some("mcp"))
        .unwrap_or(false);
    let version_ok = registered_command
        .and_then(|command| Command::new(command).arg("--version").output().ok())
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("v0.4.21"))
        .unwrap_or(false);
    command_ok && args_ok && version_ok
}

fn merge_hooks(mut root: Value, host: &str, stop_event: &str) -> Result<Value> {
    let object = root.as_object_mut().ok_or_else(|| {
        Error::new(
            "host_hook_shape_invalid",
            "hook config root must be an object",
        )
    })?;
    object.insert("_ags_managed".to_string(), Value::Bool(true));
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::new("host_hook_shape_invalid", "hooks must be an object"))?;
    for entries in hooks.values_mut() {
        if let Some(array) = entries.as_array_mut() {
            strip_ags_entries(array);
        }
    }
    hooks.retain(|_, entries| {
        entries
            .as_array()
            .map(|array| !array.is_empty())
            .unwrap_or(true)
    });

    let desired = [
        (
            "SessionStart",
            "ags-host --event session-start --workspace .".to_string(),
        ),
        ("PreToolUse", format!("ags-policy --host {host}")),
        ("PermissionRequest", format!("ags-policy --host {host}")),
        ("PostToolUse", format!("ags-policy --host {host}")),
        (
            stop_event,
            "ags-host --event session-end --workspace .".to_string(),
        ),
    ];
    for (event, command) in desired {
        let entries = hooks.entry(event).or_insert_with(|| json!([]));
        let array = entries.as_array_mut().ok_or_else(|| {
            Error::new(
                "host_hook_shape_invalid",
                format!("hooks.{event} must be an array"),
            )
        })?;
        array.push(json!({
            "hooks": [{"type": "command", "command": command, "timeout": 8}]
        }));
    }
    Ok(root)
}

fn strip_ags_entries(entries: &mut Vec<Value>) {
    let mut kept = Vec::new();
    for mut entry in entries.drain(..) {
        if is_ags_command(&entry) {
            continue;
        }
        if let Some(children) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
            children.retain(|child| !is_ags_command(child));
            if children.is_empty() {
                continue;
            }
        }
        kept.push(entry);
    }
    *entries = kept;
}

fn clean_legacy_hooks(root: &Path) -> Result<Vec<String>> {
    let mut writes = Vec::new();
    for relative in LEGACY_HOOK_PATHS {
        crate::projection::reject_symlink_path(root, Path::new(relative))?;
        let path = root.join(relative);
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| crate::error::io("host_hook_read_failed", &error))?;
        let mut root: Value = serde_json::from_str(&text).map_err(|error| {
            Error::new(
                "host_hook_parse_failed",
                format!("{}: {error}", path.display()),
            )
        })?;
        let before = root.clone();
        let object = root.as_object_mut().ok_or_else(|| {
            Error::new(
                "host_hook_shape_invalid",
                "legacy hook config root must be an object",
            )
        })?;
        if let Some(hooks) = object.get_mut("hooks").and_then(Value::as_object_mut) {
            for entries in hooks.values_mut() {
                if let Some(entries) = entries.as_array_mut() {
                    strip_ags_entries(entries);
                }
            }
            hooks.retain(|_, entries| {
                entries
                    .as_array()
                    .map(|entries| !entries.is_empty())
                    .unwrap_or(true)
            });
        }
        if root == before {
            continue;
        }
        let object = root.as_object_mut().expect("validated object");
        object.remove("_ags_managed");
        if object
            .get("hooks")
            .and_then(Value::as_object)
            .is_some_and(|hooks| hooks.is_empty())
        {
            object.remove("hooks");
        }
        if object.is_empty() {
            fs::remove_file(&path)
                .map_err(|error| crate::error::io("host_hook_write_failed", &error))?;
            writes.push(format!("host-hook-remove:{relative}"));
            continue;
        }
        let mut text = serde_json::to_string_pretty(&root)
            .map_err(|error| Error::new("host_hook_encode_failed", error.to_string()))?;
        text.push('\n');
        let tmp = path.with_extension("ags.tmp");
        fs::write(&tmp, text)
            .map_err(|error| crate::error::io("host_hook_write_failed", &error))?;
        fs::rename(&tmp, &path)
            .map_err(|error| crate::error::io("host_hook_write_failed", &error))?;
        writes.push(format!("host-hook-clean:{relative}"));
    }
    Ok(writes)
}

fn legacy_hooks_clean(root: &Path) -> bool {
    LEGACY_HOOK_PATHS.iter().all(|relative| {
        let path = root.join(relative);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return false;
        };
        let Some(hooks) = value.get("hooks") else {
            return true;
        };
        let Some(hooks) = hooks.as_object() else {
            return false;
        };
        hooks.values().all(|entries| {
            entries
                .as_array()
                .map(|entries| {
                    entries.iter().all(|entry| {
                        !is_ags_command(entry)
                            && entry
                                .get("hooks")
                                .and_then(Value::as_array)
                                .map(|children| children.iter().all(|child| !is_ags_command(child)))
                                .unwrap_or(true)
                    })
                })
                .unwrap_or(false)
        })
    })
}

fn is_ags_command(value: &Value) -> bool {
    let Some(command) = value.get("command").and_then(Value::as_str) else {
        return false;
    };
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some(program) = words.first().map(|word| word.trim_matches(['\'', '"'])) else {
        return false;
    };
    let Some(name) = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    AGS_COMMANDS.contains(&name)
        || (name == "ags" && words.get(1) == Some(&"host") && words.get(2) == Some(&"lifecycle"))
}

pub fn connection_descriptor(surface: &str) -> Result<Value> {
    match surface {
        "cli" => Ok(json!({"name": "ags", "surface": "cli", "command": canonical_ags()})),
        "mcp" => Ok(
            json!({"name": "ags", "surface": "mcp", "command": canonical_ags(), "args": ["mcp"], "tools": ["ags_decide", "ags_apply"]}),
        ),
        _ => Err(Error::new(
            "host_surface_invalid",
            "host surface must be cli or mcp",
        )),
    }
}

pub fn reconcile_client(root: &Path, host: &str, surface: &str) -> Result<Vec<String>> {
    if surface != "mcp" {
        return Ok(Vec::new());
    }
    if client_mcp_ready(host) {
        return Ok(Vec::new());
    }
    let snapshot = snapshot_client_files(root, host)?;
    let ags = canonical_ags();
    let result = match host {
        "codex" => {
            let _ = run_optional("codex", &["mcp", "remove", "ags"]);
            run_required("codex", &["mcp", "add", "ags", "--", &ags, "mcp"])
                .map(|_| vec!["host-client:codex:mcp:ags".to_string()])
        }
        "claude-code" => {
            for scope in ["local", "project", "user"] {
                let _ = run_optional("claude", &["mcp", "remove", "ags", "-s", scope]);
            }
            run_required(
                "claude",
                &["mcp", "add", "-s", "user", "ags", "--", &ags, "mcp"],
            )
            .map(|_| vec!["host-client:claude-code:mcp:ags".to_string()])
        }
        _ => Ok(Vec::new()),
    };
    if result.is_err() {
        restore_files(&snapshot)?;
    }
    result
}

fn snapshot_client_files(root: &Path, host: &str) -> Result<FileSnapshot> {
    let home = crate::sync::machine_home()?;
    let paths: Vec<PathBuf> = match host {
        "codex" => vec![home.join(".codex/config.toml")],
        "claude-code" => vec![
            home.join(".claude.json"),
            home.join(".claude/.mcp.json"),
            root.join(".mcp.json"),
        ],
        _ => Vec::new(),
    };
    paths
        .into_iter()
        .map(|path| {
            let bytes = read_optional_file(&path, "host_client_snapshot_failed")?;
            Ok((path, bytes))
        })
        .collect()
}

pub fn preflight_client(host: &str, surface: &str) -> Result<()> {
    if surface != "mcp" {
        return Ok(());
    }
    let program = match host {
        "codex" => "codex",
        "claude-code" => "claude",
        _ => return Ok(()),
    };
    let output = Command::new(program)
        .arg("--version")
        .output()
        .map_err(|e| Error::new("host_client_missing", format!("{program}: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(
            "host_client_missing",
            format!("{program} is unavailable"),
        ))
    }
}

fn canonical_ags() -> String {
    let current = std::env::current_exe().ok();
    if let Some(path) = current.as_ref() {
        if path.file_name().and_then(|name| name.to_str()) == Some("ags") {
            return path.display().to_string();
        }
        if let Some(parent) = path.parent() {
            let sibling = parent.join("ags");
            if sibling.is_file() {
                return sibling.display().to_string();
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("ags"))
            .find(|candidate| candidate.is_file())
    }) {
        return path.display().to_string();
    }
    "ags".to_string()
}

fn run_optional(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_required(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| Error::new("host_client_command_failed", format!("{program}: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(
            "host_client_command_failed",
            format!(
                "{program} exited with {}: {}",
                output.status.code().unwrap_or(1),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

pub fn memory_dir(slug: &str) -> Result<PathBuf> {
    if !crate::config::valid_slug(slug) {
        return Err(Error::new(
            "workspace_slug_invalid",
            "workspace slug must use ASCII letters, numbers, dot, underscore or dash",
        ));
    }
    let home = crate::sync::machine_home()?;
    Ok(home.join(".agents/memory/projects").join(slug))
}

pub fn legacy_memory_dir(slug: &str) -> Result<PathBuf> {
    if slug.is_empty() || slug == "." || slug == ".." || slug.contains('/') || slug.contains('\\') {
        return Err(Error::new(
            "workspace_slug_invalid",
            "legacy workspace slug contains a path boundary",
        ));
    }
    let home = crate::sync::machine_home()?;
    Ok(home.join(".agents/memory/projects").join(slug))
}

#[derive(Debug)]
pub enum MemoryMigration {
    Rename {
        old: PathBuf,
        new: PathBuf,
    },
    Merge {
        old: PathBuf,
        imported: PathBuf,
        task_memory: PathBuf,
        previous_task_memory: Option<Vec<u8>>,
    },
}

pub fn migrate_memory(old: &Path, new: &Path) -> Result<Option<MemoryMigration>> {
    if old == new || !old.exists() {
        return Ok(None);
    }
    if !new.exists() {
        fs::rename(old, new)
            .map_err(|error| crate::error::io("memory_slug_migration_failed", &error))?;
        return Ok(Some(MemoryMigration::Rename {
            old: old.to_path_buf(),
            new: new.to_path_buf(),
        }));
    }

    let import_id: String = crate::workspace::sha256_hex(old.to_string_lossy().as_bytes())
        .chars()
        .take(16)
        .collect();
    let imported = new.join("legacy-imports").join(import_id);
    if imported.exists() {
        return Err(Error::new(
            "memory_slug_conflict",
            format!(
                "legacy import target already exists: {}",
                imported.display()
            ),
        ));
    }
    let old_task = read_optional_file(&old.join("task-memory.md"), "memory_slug_snapshot_failed")?;
    let task_memory = new.join("task-memory.md");
    let previous_task_memory = read_optional_file(&task_memory, "memory_slug_snapshot_failed")?;
    if let Some(parent) = imported.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| crate::error::io("memory_slug_migration_failed", &error))?;
    }
    fs::rename(old, &imported)
        .map_err(|error| crate::error::io("memory_slug_migration_failed", &error))?;

    if let Some(old_task) = old_task {
        let marker = format!("<!-- imported-memory:{} -->", imported.display());
        let mut current = previous_task_memory
            .as_deref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        if !current.contains(&marker) {
            if !current.is_empty() && !current.ends_with('\n') {
                current.push('\n');
            }
            current.push_str(&format!(
                "\n{marker}\n## Imported legacy task memory\n\n{}\n",
                String::from_utf8_lossy(&old_task).trim()
            ));
            if let Err(error) = write_file_atomic(&task_memory, current.as_bytes()) {
                let _ = fs::rename(&imported, old);
                restore_one(&task_memory, previous_task_memory.as_deref())?;
                return Err(error);
            }
        }
    }

    Ok(Some(MemoryMigration::Merge {
        old: old.to_path_buf(),
        imported,
        task_memory,
        previous_task_memory,
    }))
}

pub fn rollback_memory(migration: &MemoryMigration) -> Result<()> {
    match migration {
        MemoryMigration::Rename { old, new } => {
            if new.exists() && !old.exists() {
                fs::rename(new, old)
                    .map_err(|error| crate::error::io("memory_slug_rollback_failed", &error))?;
            }
        }
        MemoryMigration::Merge {
            old,
            imported,
            task_memory,
            previous_task_memory,
        } => {
            restore_one(task_memory, previous_task_memory.as_deref())?;
            if imported.exists() && !old.exists() {
                fs::rename(imported, old)
                    .map_err(|error| crate::error::io("memory_slug_rollback_failed", &error))?;
            }
        }
    }
    Ok(())
}

fn restore_one(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    match bytes {
        Some(bytes) => write_file_atomic(path, bytes),
        None => {
            if path.is_file() {
                fs::remove_file(path)
                    .map_err(|error| crate::error::io("memory_slug_rollback_failed", &error))?;
            }
            Ok(())
        }
    }
}

fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| crate::error::io("memory_slug_migration_failed", &error))?;
    }
    let tmp = path.with_extension(format!("ags-tmp-{}", std::process::id()));
    fs::write(&tmp, bytes)
        .map_err(|error| crate::error::io("memory_slug_migration_failed", &error))?;
    fs::rename(&tmp, path).map_err(|error| crate::error::io("memory_slug_migration_failed", &error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_merge_preserves_user_hooks_and_replaces_ags_hooks() {
        let source = json!({
            "user_setting": "preserve",
            "hooks": {
                "SessionStart": [{"hooks": [{"command": "ags-policy --probe"}, {"command": "user-start"}, {"command": "echo ags-host"}]}],
                "Stop": [{"hooks": [{"command": "node evolver.js"}]}]
            }
        });
        let merged = merge_hooks(source, "claude-code", "Stop").unwrap();
        let text = merged.to_string();
        assert!(text.contains("user-start"));
        assert!(text.contains("preserve"));
        assert!(text.contains("echo ags-host"));
        assert!(text.contains("evolver.js"));
        assert!(text.contains("ags-host --event session-start"));
        assert!(text.contains("ags-host --event session-end"));
        assert_eq!(text.matches("ags-policy --probe").count(), 0);
    }

    #[test]
    fn descriptor_is_surface_neutral() {
        let mcp = connection_descriptor("mcp").unwrap();
        assert_eq!(mcp["name"], "ags");
        assert_eq!(mcp["args"], json!(["mcp"]));
    }

    #[test]
    fn claude_registration_is_read_from_structured_config() {
        let config = json!({
            "mcpServers": {
                "ags": {"command": "/tmp/ags", "args": ["mcp"]}
            }
        });
        assert_eq!(claude_mcp_registration(&config), Some(("/tmp/ags", true)));
        let stale = json!({
            "mcpServers": {
                "ags": {"command": "/tmp/ags", "args": ["mcp", "serve"]}
            }
        });
        assert_eq!(claude_mcp_registration(&stale), Some(("/tmp/ags", false)));
        assert_eq!(claude_mcp_registration(&json!({})), None);
    }

    #[test]
    fn unowned_hook_file_is_structurally_adopted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"hooks\":{\"Stop\":[{\"hooks\":[{\"command\":\"user-stop\"}]}]}}\n",
        )
        .unwrap();
        sync_workspace_hooks(tmp.path()).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("user-stop"));
        assert!(text.contains("ags-host --event session-start"));
        assert!(text.contains("ags-host --event session-end"));
        assert!(text.contains("\"_ags_managed\": true"));
    }

    #[test]
    fn legacy_claude_hooks_are_removed_without_touching_user_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/settings.local.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "user_setting": "preserve",
                "hooks": {
                    "SessionStart": [{"hooks": [
                        {"command": "ags host lifecycle --event session-start --target ."},
                        {"command": "user-start"}
                    ]}],
                    "Stop": [{"hooks": [{"command": "ags host lifecycle --event session-end --target ."}]}]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(!legacy_hooks_clean(tmp.path()));
        let writes = sync_workspace_hooks(tmp.path()).unwrap();
        assert!(writes
            .iter()
            .any(|write| write == "host-hook-clean:.claude/settings.local.json"));
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("user_setting"));
        assert!(text.contains("user-start"));
        assert!(!text.contains("ags host lifecycle"));
        assert!(legacy_hooks_clean(tmp.path()));
    }

    #[test]
    fn ags_only_legacy_claude_hook_file_is_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude/settings.local.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"hooks\":{\"Stop\":[{\"hooks\":[{\"command\":\"ags host lifecycle --event session-end --target .\"}]}]}}",
        )
        .unwrap();
        let writes = sync_workspace_hooks(tmp.path()).unwrap();
        assert!(writes
            .iter()
            .any(|write| write == "host-hook-remove:.claude/settings.local.json"));
        assert!(!path.exists());
    }

    #[test]
    fn workspace_hook_parent_symlink_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join(".codex")).unwrap();
        let error = sync_workspace_hooks(tmp.path()).unwrap_err();
        assert_eq!(error.code, "projection_symlink_rejected");
        assert!(!outside.path().join("hooks.json").exists());
    }

    #[test]
    fn conflicting_memory_dirs_merge_without_data_loss_and_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("context-capsule.md"), "old capsule\n").unwrap();
        fs::write(old.join("task-memory.md"), "old task\n").unwrap();
        fs::write(new.join("context-capsule.md"), "new capsule\n").unwrap();
        fs::write(new.join("task-memory.md"), "new task\n").unwrap();
        let migration = migrate_memory(&old, &new).unwrap().unwrap();
        assert!(!old.exists());
        let task = fs::read_to_string(new.join("task-memory.md")).unwrap();
        assert!(task.contains("new task"));
        assert!(task.contains("old task"));
        assert!(new.join("legacy-imports").is_dir());
        rollback_memory(&migration).unwrap();
        assert_eq!(
            fs::read_to_string(old.join("task-memory.md")).unwrap(),
            "old task\n"
        );
        assert_eq!(
            fs::read_to_string(new.join("task-memory.md")).unwrap(),
            "new task\n"
        );
    }
}
