//! Diagnostic check functions for suite-doctor.
//!
//! Each check runs a lightweight diagnostic and returns a `Finding`.
//! Checks that shell out are behind simple functions so they can be
//! replaced with stubs in tests — no dynamic dispatch needed.

use crate::types::{Finding, HealthReport};
use serde_yaml::Value as YamlValue;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Edition detection ────────────────────────────────────────────────

fn is_public_edition(repo_root: &Path) -> bool {
    let workspace = repo_root.join("WORKSPACE.md");
    let claude = repo_root.join("CLAUDE.md");
    for path in [workspace, claude] {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if raw.contains("Public Edition") || raw.contains("public distributable edition") {
                return true;
            }
        }
    }
    false
}

/// Check that `manifests/runtime-profiles.yaml` exists and contains the
/// expected top-level keys.
pub fn runtime_profile_declared(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/runtime-profiles.yaml");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Finding::skip(
                "runtime_profile_declared",
                "public edition ships runtime profile templates, not installed private runtime-profiles.yaml",
            );
        }
        return Finding::fail(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml not found",
            format!(
                "Expected at: {}. Create it with claude-code-executor and planner profiles.",
                path.display()
            ),
        );
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Finding::fail(
                "runtime_profile_declared",
                "cannot read manifests/runtime-profiles.yaml",
                format!("{e}"),
            );
        }
    };

    let parsed: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Finding::fail(
                "runtime_profile_declared",
                "manifests/runtime-profiles.yaml is not valid YAML",
                format!("{e}"),
            );
        }
    };

    let has_schema = yaml_get(&parsed, "schema_version").is_some();
    let profiles = yaml_get(&parsed, "profiles");
    let has_claude_code = profiles
        .and_then(|profiles| yaml_get(profiles, "claude-code-executor"))
        .is_some();
    let has_planner = profiles
        .and_then(|profiles| yaml_get(profiles, "planner"))
        .is_some();

    if has_schema && has_claude_code && has_planner {
        Finding::pass(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml present with claude-code-executor and planner profiles",
        )
    } else {
        let mut missing = Vec::new();
        if !has_schema {
            missing.push("schema_version");
        }
        if !has_claude_code {
            missing.push("claude-code-executor profile");
        }
        if !has_planner {
            missing.push("planner profile");
        }
        Finding::fail(
            "runtime_profile_declared",
            "manifests/runtime-profiles.yaml missing required profiles",
            format!("Missing: {}", missing.join(", ")),
        )
    }
}

fn mcp_registry_entry_status(repo_root: &Path, mcp_name: &str) -> Result<Option<String>, Finding> {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Err(Finding::info(
                format!("mcp_registry_{mcp_name}_adopted"),
                "public edition does not require private mcp-registry.yaml",
            ));
        }
        return Err(Finding::warn(
            format!("mcp_registry_{mcp_name}_adopted"),
            "manifests/mcp-registry.yaml not found",
            format!("Expected at: {}", path.display()),
        ));
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Err(Finding::warn(
                format!("mcp_registry_{mcp_name}_adopted"),
                "cannot read manifests/mcp-registry.yaml",
                format!("{e}"),
            ));
        }
    };

    let parsed: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Err(Finding::warn(
                format!("mcp_registry_{mcp_name}_adopted"),
                "manifests/mcp-registry.yaml is not valid YAML",
                format!("{e}"),
            ));
        }
    };

    Ok(yaml_get(&parsed, "mcps")
        .and_then(|mcps| mcps.as_sequence())
        .and_then(|mcps| {
            mcps.iter().find_map(|entry| {
                let name = yaml_get(entry, "name").and_then(|value| value.as_str());
                if name == Some(mcp_name) {
                    yaml_get(entry, "status").and_then(|value| value.as_str())
                } else {
                    None
                }
                .map(|status| status.to_string())
            })
        }))
}

fn mcp_registry_adopted_check(repo_root: &Path, mcp_name: &str, display_name: &str) -> Finding {
    let check_name = format!("mcp_registry_{mcp_name}_adopted");
    let status = match mcp_registry_entry_status(repo_root, mcp_name) {
        Ok(status) => status,
        Err(finding) => return finding,
    };

    let has_mcp = status.is_some();
    let has_adopted = status.as_deref() == Some("adopted");

    if has_mcp && has_adopted {
        Finding::info(
            check_name,
            format!("{display_name} MCP registered and adopted in mcp-registry.yaml"),
        )
    } else if has_mcp {
        Finding::warn(
            check_name,
            format!("{display_name} MCP found but status is not adopted"),
            format!("Review manifests/mcp-registry.yaml {mcp_name} entry"),
        )
    } else {
        Finding::info(
            check_name,
            format!("{display_name} MCP not registered in mcp-registry.yaml"),
        )
    }
}

/// Check that `manifests/mcp-registry.yaml` has a `codegraph` MCP entry with
/// `status: adopted`. This is an **info** check — host verification checks
/// enforce actual Claude Code registration.
pub fn mcp_registry_codegraph_adopted(repo_root: &Path) -> Finding {
    mcp_registry_adopted_check(repo_root, "codegraph", "CodeGraph")
}

fn yaml_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value
        .as_mapping()
        .and_then(|map| map.get(YamlValue::String(key.to_string())))
}

// ── Public check functions ───────────────────────────────────────────────

/// Run `git status --porcelain` and report uncommitted changes.
pub fn git_status_check(repo_root: &Path) -> Finding {
    match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Finding::fail(
                    "git-status",
                    "git status failed",
                    format!(
                        "git exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let changed: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
            if changed.is_empty() {
                Finding::pass("git-status", "working tree clean")
            } else {
                Finding::warn(
                    "git-status",
                    format!("{} uncommitted file(s)", changed.len()),
                    format!("Changed: {}", changed.join(", ")),
                )
            }
        }
        Err(e) => Finding::fail(
            "git-status",
            "git not available",
            format!("Failed to run git: {e}"),
        ),
    }
}

fn project_integration_check(identity: &project_discovery::ProjectIdentity) -> Finding {
    use project_discovery::IntegrationStatus;

    match identity.integration_status {
        IntegrationStatus::Suite => Finding::pass(
            "project-integration",
            "target is the AGS suite authority workspace",
        ),
        IntegrationStatus::Integrated => Finding::pass(
            "project-integration",
            "target is registered with a complete AGS integration identity",
        ),
        IntegrationStatus::Partial => Finding::fail(
            "project-integration",
            "target has a partial AGS integration",
            format!("Gaps: {}", identity.gaps.join("; ")),
        ),
        IntegrationStatus::NotIntegrated => Finding::fail(
            "project-integration",
            "target is not managed by AGS",
            "Run `ags init --target <project>` before using it as a governed project.",
        ),
    }
}

fn project_protocol_check(repo_root: &Path) -> Finding {
    let status = project_discovery::check_protocol_status(repo_root);
    if !status.failures.is_empty() {
        Finding::fail(
            "project-protocol",
            "AGS protocol or validator projection is incomplete",
            status.failures.join("; "),
        )
    } else if !status.warnings.is_empty() {
        Finding::warn(
            "project-protocol",
            format!(
                "AGS protocol projection is usable with {} warning(s)",
                status.warnings.len()
            ),
            status.warnings.join("; "),
        )
    } else {
        Finding::pass(
            "project-protocol",
            format!(
                "AGS protocol projection complete ({}/{} files, validator available)",
                status.present_count,
                status.files.len()
            ),
        )
    }
}

// ── Context memory checks ──────────────────────────────────────────────

const MEMORY_CAPTURE_MARKER: &str = "claude-stop-memory-capture";
const RAW_GUARD_MARKER: &str = "raw-tool-call-stop-guard";

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn sanitize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}

fn project_slug(repo_root: &Path) -> String {
    let profile = repo_root.join("config/agent-project-profile.yaml");
    if let Ok(raw) = std::fs::read_to_string(profile) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&raw) {
            if let Some(slug) = doc
                .get("project")
                .and_then(|p| p.get("slug"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                return sanitize_slug(slug);
            }
        }
    }
    let name = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    sanitize_slug(name)
}

fn project_memory_dir_at(repo_root: &Path, home: &Path) -> PathBuf {
    home.join(".agents")
        .join("memory")
        .join("projects")
        .join(project_slug(repo_root))
}

fn stop_hook_commands(settings_path: &Path, check_name: &str) -> Result<Vec<String>, Finding> {
    let raw = std::fs::read_to_string(settings_path).map_err(|e| {
        Finding::warn(
            check_name,
            format!("cannot read {}", settings_path.display()),
            e.to_string(),
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        Finding::warn(
            check_name,
            format!("{} is not valid JSON", settings_path.display()),
            e.to_string(),
        )
    })?;
    let mut commands = Vec::new();
    if let Some(stop) = value
        .get("hooks")
        .and_then(|h| h.get("Stop"))
        .and_then(|s| s.as_array())
    {
        for group in stop {
            if let Some(command) = group.get("command").and_then(|c| c.as_str()) {
                commands.push(command.to_string());
            }
            if let Some(hooks) = group.get("hooks").and_then(|h| h.as_array()) {
                for hook in hooks {
                    if let Some(command) = hook.get("command").and_then(|c| c.as_str()) {
                        commands.push(command.to_string());
                    }
                }
            }
        }
    }
    Ok(commands)
}

fn memory_pipeline_order(commands: &[String]) -> String {
    let mut seen = Vec::new();
    for command in commands {
        if command.contains(RAW_GUARD_MARKER) && !seen.contains(&"raw-guard") {
            seen.push("raw-guard");
        } else if command.contains(MEMORY_CAPTURE_MARKER) && !seen.contains(&"memory-capture") {
            seen.push("memory-capture");
        }
    }
    seen.join(" → ")
}

fn memory_capture_scripts_present_at(home: &Path) -> Finding {
    let context = home.join(".agents/scripts/context-memory.sh");
    let bridge = home.join(".agents/scripts/claude-stop-memory-capture.py");
    let mut missing = Vec::new();
    if !context.is_file() {
        missing.push(context.display().to_string());
    }
    if !bridge.is_file() {
        missing.push(bridge.display().to_string());
    }
    if missing.is_empty() {
        Finding::pass(
            "memory-capture-scripts-present",
            "context memory scripts installed",
        )
    } else {
        Finding::warn(
            "memory-capture-scripts-present",
            "context memory scripts missing",
            format!(
                "Run `ags setup --yes --register-claude`. Missing: {}",
                missing.join(", ")
            ),
        )
    }
}

pub fn memory_capture_scripts_present() -> Finding {
    let Some(home) = home_dir() else {
        return Finding::warn(
            "memory-capture-scripts-present",
            "HOME not set — cannot locate context memory scripts",
            "Set HOME or run doctor in a normal user shell.",
        );
    };
    memory_capture_scripts_present_at(&home)
}

pub fn claude_code_memory_capture_wired(repo_root: &Path) -> Finding {
    let check = "memory-capture-stop-hook-wired";
    let settings = repo_root.join(".claude/settings.json");
    if !settings.exists() {
        return Finding::warn(
            check,
            "Claude Stop hook settings not found",
            format!(
                "Run `ags setup --yes --register-claude` from this workspace to wire {}",
                settings.display()
            ),
        );
    }
    let commands = match stop_hook_commands(&settings, check) {
        Ok(commands) => commands,
        Err(finding) => return finding,
    };
    if commands
        .iter()
        .any(|command| command.contains(MEMORY_CAPTURE_MARKER))
    {
        Finding::pass(
            check,
            format!(
                "project memory capture wired in Claude Stop hooks (order: {})",
                memory_pipeline_order(&commands)
            ),
        )
    } else {
        Finding::warn(
            check,
            "project memory capture missing from Claude Stop hooks",
            "Run `ags setup --yes --register-claude` to merge the capture step while preserving existing hooks.",
        )
    }
}

pub fn raw_tool_call_stop_guard_present(repo_root: &Path) -> Finding {
    let check = "raw-tool-call-stop-guard-present";
    let settings = repo_root.join(".claude/settings.json");
    if !settings.exists() {
        return Finding::warn(
            check,
            "Claude Stop hook settings not found",
            format!("Cannot verify raw guard at {}", settings.display()),
        );
    }
    let commands = match stop_hook_commands(&settings, check) {
        Ok(commands) => commands,
        Err(finding) => return finding,
    };
    if commands
        .iter()
        .any(|command| command.contains(RAW_GUARD_MARKER))
    {
        Finding::pass(check, "raw tool-call Stop guard present")
    } else {
        Finding::warn(
            check,
            "raw tool-call Stop guard not found",
            "If you use raw tool-call Stop guarding, keep it in the Stop pipeline when adding memory capture.",
        )
    }
}

fn project_task_memory_status_at(repo_root: &Path, home: &Path) -> Finding {
    let path = project_memory_dir_at(repo_root, home).join("task-memory.md");
    if !path.exists() {
        return Finding::warn(
            "project-task-memory-status",
            "task-memory.md missing for this project",
            format!("Run `ags init --target {}` or `ags setup --yes --register-claude` to initialize it.", repo_root.display()),
        );
    }
    match std::fs::metadata(&path).and_then(|m| m.modified()) {
        Ok(modified) => {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default();
            if age > std::time::Duration::from_secs(60 * 60 * 24 * 30) {
                Finding::warn(
                    "project-task-memory-status",
                    "task-memory.md is present but old",
                    format!("Last modified more than 30 days ago: {}", path.display()),
                )
            } else {
                Finding::pass(
                    "project-task-memory-status",
                    format!("task-memory.md present: {}", path.display()),
                )
            }
        }
        Err(e) => Finding::warn(
            "project-task-memory-status",
            format!("cannot inspect {}", path.display()),
            e.to_string(),
        ),
    }
}

pub fn project_task_memory_status(repo_root: &Path) -> Finding {
    let Some(home) = home_dir() else {
        return Finding::warn(
            "project-task-memory-status",
            "HOME not set — cannot locate project task memory",
            "Set HOME or run doctor in a normal user shell.",
        );
    };
    project_task_memory_status_at(repo_root, &home)
}

fn context_capsule_integrity_at(repo_root: &Path, home: &Path) -> Finding {
    let path = project_memory_dir_at(repo_root, home).join("context-capsule.md");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            return Finding::warn(
                "context-capsule-integrity",
                "context-capsule.md missing or unreadable",
                format!("{} ({e})", path.display()),
            );
        }
    };
    if raw.contains("## 项目设计目的") {
        Finding::pass(
            "context-capsule-integrity",
            "context capsule contains the manual design-purpose section",
        )
    } else {
        Finding::warn(
            "context-capsule-integrity",
            "context capsule is missing the manual design-purpose section",
            format!("Add `## 项目设计目的` to {}", path.display()),
        )
    }
}

pub fn context_capsule_integrity(repo_root: &Path) -> Finding {
    let Some(home) = home_dir() else {
        return Finding::warn(
            "context-capsule-integrity",
            "HOME not set — cannot locate context capsule",
            "Set HOME or run doctor in a normal user shell.",
        );
    };
    context_capsule_integrity_at(repo_root, &home)
}

/// Check that `manifests/templates/runtime-profiles.template.yaml` exists.
pub fn runtime_profile_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/runtime-profiles.template.yaml");
    if path.exists() {
        Finding::info(
            "runtime_profile_template_exists",
            "manifests/templates/runtime-profiles.template.yaml found",
        )
    } else {
        Finding::warn(
            "runtime_profile_template_exists",
            "manifests/templates/runtime-profiles.template.yaml missing",
            format!(
                "Portable runtime profile template not found at {}. Bootstrap and migration may lack runtime profile support.",
                path.display()
            ),
        )
    }
}

/// Check that `manifests/templates/hooks/codex-planner-recall.template.json` exists.
pub fn codex_planner_hook_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/hooks/codex-planner-recall.template.json");
    if path.exists() {
        Finding::info(
            "codex_planner_hook_template_exists",
            "manifests/templates/hooks/codex-planner-recall.template.json found",
        )
    } else {
        Finding::warn(
            "codex_planner_hook_template_exists",
            "manifests/templates/hooks/codex-planner-recall.template.json missing",
            format!(
                "Codex planner hook template not found at {}. Planner pre-solution recall setup may be incomplete.",
                path.display()
            ),
        )
    }
}

/// Check that `manifests/templates/hooks/claude-code-executor-stop.template.js`
/// exists and passes `node --check`.
pub fn claude_code_stop_hook_template_exists(repo_root: &Path) -> Finding {
    let path = repo_root.join("manifests/templates/hooks/claude-code-executor-stop.template.js");
    if !path.exists() {
        return Finding::warn(
            "claude_code_stop_hook_template_exists",
            "manifests/templates/hooks/claude-code-executor-stop.template.js missing",
            format!(
                "Claude Code Stop hook template not found at {}. Bootstrap and migration may lack hook support.",
                path.display()
            ),
        );
    }

    match Command::new("node")
        .args([
            "--check",
            "manifests/templates/hooks/claude-code-executor-stop.template.js",
        ])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Finding::info(
                    "claude_code_stop_hook_template_exists",
                    "manifests/templates/hooks/claude-code-executor-stop.template.js found and syntax OK",
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Finding::warn(
                    "claude_code_stop_hook_template_exists",
                    "claude-code-executor-stop.template.js syntax error",
                    format!("node --check failed: {}", stderr.trim()),
                )
            }
        }
        Err(e) => Finding::warn(
            "claude_code_stop_hook_template_exists",
            "node not available",
            format!("Cannot run node --check on template: {e}"),
        ),
    }
}

/// Run all default suite-doctor checks and populate a `HealthReport`.
///
/// The `repo_root` is typically the current working directory or a
/// configured suite root.
/// Credential-shaped key SUFFIXES (normalized to lowercase alphanumerics). A
/// normalized key ending with any of these is treated as tracked credential
/// evidence. Suffix (not substring) matching avoids false positives on legitimate
/// keys like `authority` and `requires_auth`, while still catching compound keys
/// like `client_secret`, `mcp_api_key`, and `bearer_token`.
const CRED_KEY_SUFFIXES: &[&str] = &[
    "token",
    "secret",
    "secretkey",
    "password",
    "passwd",
    "passphrase",
    "apikey",
    "credential",
    "credentials",
    "authorization",
    "bearer",
    "privatekey",
    "accesskey",
    "clientsecret",
];

/// Normalize a YAML key to lowercase alphanumerics, so `api_key`, `API-KEY`,
/// `apiKey`, and `api key` all collapse to `apikey`.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn is_credential_key(normalized: &str) -> bool {
    CRED_KEY_SUFFIXES.iter().any(|t| normalized.ends_with(t))
}

/// Recursively collect credential-evidence violations from a parsed YAML value.
/// Flags (a) any mapping KEY shaped like a credential and (b) an `auth_status`
/// key whose scalar value asserts `configured` (case-insensitive). Inspects KEYS,
/// not arbitrary prose values — a `denied:` note mentioning "tokens" is not a hit.
fn scan_yaml_credentials(value: &YamlValue, path: &str, out: &mut Vec<String>) {
    match value {
        YamlValue::Mapping(map) => {
            for (k, v) in map.iter() {
                let key = k.as_str().unwrap_or("");
                let here = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                let norm = normalize_key(key);
                if is_credential_key(&norm) {
                    out.push(here.clone());
                } else if norm == "authstatus"
                    && v.as_str()
                        .map(|s| s.to_ascii_lowercase().contains("configured"))
                        .unwrap_or(false)
                {
                    out.push(format!("{here}=configured"));
                }
                scan_yaml_credentials(v, &here, out);
            }
        }
        YamlValue::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                scan_yaml_credentials(item, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Read-only Capability Route drift check. Three boundaries, never writes, never
/// probes a host CLI:
///  1. **manifest routing lifecycle** — each `auto-*` alias must have an
///     explicit lifecycle posture: either retired (`route_state: retired`) or
///     still active as a compatibility alias (`route_state: routable` +
///     `is_compatibility_alias: true`). Anything else warns (route degrades,
///     never blocks).
///  2. **auth-evidence boundary** — NO tracked manifest may carry a credential
///     key or assert a configured auth status. A violation is the one blocking
///     capability-route FAIL: runtime auth posture is runtime-derived only and
///     must never be tracked. Mirrors the credential grep in the verification gate.
///  3. **runtime enrollment** — machine-local evidence presence + mode. Absent ⇒
///     warn (advisory degraded), never a fail; enrollment lives in the runtime
///     home, never in a tracked manifest.
pub fn capability_route_drift_check(repo_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // 1. Manifest routing lifecycle — auto-* aliases may be retired OR remain
    //    active compatibility aliases while a successor gap exists (notably
    //    verify). Both states are explicit; anything else warns.
    let skills_path = repo_root.join("manifests/skills-registry.yaml");
    match std::fs::read_to_string(&skills_path) {
        Ok(content) => match serde_yaml::from_str::<YamlValue>(&content) {
            Ok(doc) => {
                let skills = doc.get("skills").and_then(|v| v.as_sequence());
                let invalid_aliases: Vec<&str> = ["auto-brainstorm", "auto-debug", "auto-verify"]
                    .into_iter()
                    .filter(|alias| {
                        let valid = skills
                            .into_iter()
                            .flatten()
                            .filter(|item| {
                                item.get("name").and_then(|n| n.as_str()) == Some(*alias)
                            })
                            .any(|item| {
                                let routing = item.get("routing");
                                let route_state = routing
                                    .and_then(|r| r.get("route_state"))
                                    .and_then(|s| s.as_str());
                                let is_alias = routing
                                    .and_then(|r| r.get("is_compatibility_alias"))
                                    .and_then(|b| b.as_bool())
                                    .unwrap_or(false);
                                route_state == Some("retired")
                                    || (route_state == Some("routable") && is_alias)
                            });
                        !valid
                    })
                    .collect();
                if invalid_aliases.is_empty() {
                    findings.push(Finding::pass(
                        "capability-route-manifest-routing",
                        "auto-* aliases have explicit lifecycle state",
                    ));
                } else {
                    findings.push(Finding::warn(
                        "capability-route-manifest-routing",
                        "auto-* alias lifecycle is not explicit",
                        format!(
                            "Set each alias to either route_state: retired, or route_state: routable with is_compatibility_alias: true. Invalid/missing: {}. Route degrades but never blocks.",
                            invalid_aliases.join(", ")
                        ),
                    ));
                }
            }
            Err(e) => findings.push(Finding::skip(
                "capability-route-manifest-routing",
                format!("skills-registry.yaml unparseable: {e}"),
            )),
        },
        Err(_) => findings.push(Finding::skip(
            "capability-route-manifest-routing",
            "skills-registry.yaml not present (non-suite edition)",
        )),
    }

    // 2. Auth-evidence boundary — tracked manifests must carry no credential key
    // and assert no configured auth status. Parses each manifest and walks it
    // recursively, normalizing KEYS case-insensitively, so credential-shaped
    // fields (`api_key`, `Authorization`, `client_secret`, spaced/cased/nested
    // variants) are caught — not just the three lowercase substrings the older
    // line scan looked for.
    let mut violations: Vec<String> = Vec::new();
    for rel in [
        "manifests/skills-registry.yaml",
        "manifests/mcp-registry.yaml",
        "manifests/suite.yaml",
    ] {
        if let Ok(content) = std::fs::read_to_string(repo_root.join(rel)) {
            if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&content) {
                let mut hits = Vec::new();
                scan_yaml_credentials(&doc, "", &mut hits);
                for h in hits {
                    violations.push(format!("{rel}:{h}"));
                }
            }
        }
    }
    if violations.is_empty() {
        findings.push(Finding::pass(
            "capability-route-auth-boundary",
            "no credential key or configured auth status in tracked manifests",
        ));
    } else {
        findings.push(Finding::fail(
            "capability-route-auth-boundary",
            "tracked manifest carries a credential key or configured auth status",
            format!(
                "auth_status is runtime-derived and must never be tracked. Offending line(s): {}",
                violations.join(", ")
            ),
        ));
    }

    // 3. Runtime enrollment evidence (machine-local). Absent ⇒ advisory warn,
    // never a fail; it lives in the runtime home, never a tracked manifest.
    let runtime_home = capability_route::locate_runtime_home();
    let evidence = capability_route::enrollment_file_path(&runtime_home);
    let enrollment = capability_route::read_enrollment(&runtime_home);
    if enrollment.present {
        findings.push(Finding::info(
            "capability-route-enrollment",
            format!(
                "Capability Route enrolled: mode={} (evidence at {})",
                enrollment.mode.as_str(),
                evidence.display()
            ),
        ));
    } else {
        findings.push(Finding::warn(
            "capability-route-enrollment",
            "no machine-local Capability Route enrollment evidence",
            format!(
                "Run `ags setup --capability-route <suite-only|adopted|review-all> --yes` to enroll (expected at {}). Routing degrades to advisory; it never blocks.",
                evidence.display()
            ),
        ));
    }

    findings
}

/// Read-only routing-COVERAGE gate (manifest hygiene). Every adopted capability
/// — suite.yaml required/optional/personal skills and governed MCPs — must carry
/// an explicit `routing.route_state` (routable / not-routable / retired) in the
/// routing-source manifests. A missing route_state is exactly the
/// indistinguishable "forgot to annotate" gap the 2.7 closure removes, so it is a
/// FAIL — but it gates the MANIFEST AUTHOR (CI / doctor), never a live route:
/// Capability Route stays advisory regardless of this finding. Hermetic: reads
/// manifests only, never probes a host.
pub fn capability_route_coverage_check(repo_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Classify one registry entry's `routing` block EXACTLY as production
    // `collect_routing` does: a typed `RoutingMetadata` parse must succeed AND
    // `route_state` must be declared explicitly. A present-but-malformed block
    // (typo'd enum like `routeable`, non-mapping, invalid field) passes a naive
    // key-presence check yet is silently dropped from the production routing map,
    // so it must FAIL coverage rather than pass it.
    #[derive(PartialEq)]
    enum RouteCoverage {
        Covered,
        Malformed,
        Missing,
    }
    fn classify_routing(item: &YamlValue) -> RouteCoverage {
        let Some(block) = item.get("routing") else {
            return RouteCoverage::Missing;
        };
        if serde_yaml::from_value::<skill_governance::console::RoutingMetadata>(block.clone())
            .is_err()
        {
            return RouteCoverage::Malformed;
        }
        if block.get("route_state").is_none() {
            return RouteCoverage::Missing;
        }
        RouteCoverage::Covered
    }

    // name → coverage verdict from skills-registry (typed parse, not key presence).
    let sr = repo_root.join("manifests/skills-registry.yaml");
    let skill_cov: std::collections::HashMap<String, RouteCoverage> =
        match std::fs::read_to_string(&sr) {
            Ok(c) => match serde_yaml::from_str::<YamlValue>(&c) {
                Ok(doc) => doc
                    .get("skills")
                    .and_then(|v| v.as_sequence())
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|it| {
                                it.get("name")
                                    .and_then(|n| n.as_str())
                                    .map(|n| (n.to_string(), classify_routing(it)))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(e) => {
                    return vec![Finding::skip(
                        "capability-route-coverage",
                        format!("skills-registry.yaml unparseable: {e}"),
                    )]
                }
            },
            Err(_) => {
                return vec![Finding::skip(
                    "capability-route-coverage",
                    "skills-registry.yaml not present (non-suite edition)",
                )]
            }
        };

    // Adopted skill names from suite.yaml (required + optional lists, personal map).
    let mut adopted: Vec<String> = Vec::new();
    if let Ok(c) = std::fs::read_to_string(repo_root.join("manifests/suite.yaml")) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&c) {
            let suite = doc.get("suite");
            for sect in ["required", "optional"] {
                if let Some(seq) = suite
                    .and_then(|s| s.get(sect))
                    .and_then(|v| v.as_sequence())
                {
                    adopted.extend(
                        seq.iter()
                            .filter_map(|it| it.get("name").and_then(|n| n.as_str()))
                            .map(String::from),
                    );
                }
            }
            if let Some(map) = suite
                .and_then(|s| s.get("personal"))
                .and_then(|v| v.as_mapping())
            {
                adopted.extend(map.keys().filter_map(|k| k.as_str()).map(String::from));
            }
        }
    }

    let mut malformed: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in adopted {
        match skill_cov.get(&name) {
            Some(RouteCoverage::Covered) => {}
            Some(RouteCoverage::Malformed) => malformed.push(name),
            _ => missing.push(name),
        }
    }
    if malformed.is_empty() && missing.is_empty() {
        findings.push(Finding::pass(
            "capability-route-coverage",
            "every adopted skill declares a valid, explicit routing.route_state (typed parse)",
        ));
    } else {
        let mut detail = String::new();
        if !malformed.is_empty() {
            detail.push_str(&format!(
                "malformed routing block (fails typed parse, dropped from routing): {}. ",
                malformed.join(", ")
            ));
        }
        if !missing.is_empty() {
            detail.push_str(&format!(
                "missing explicit route_state (routable | not-routable | retired, never defaulted): {}.",
                missing.join(", ")
            ));
        }
        findings.push(Finding::fail(
            "capability-route-coverage",
            "adopted skills with invalid or missing routing.route_state",
            detail,
        ));
    }

    // Governed MCPs: same typed-parse coverage (key presence is not enough).
    if let Ok(c) = std::fs::read_to_string(repo_root.join("manifests/mcp-registry.yaml")) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&c) {
            if let Some(seq) = doc.get("mcps").and_then(|v| v.as_sequence()) {
                let mcp_bad: Vec<String> = seq
                    .iter()
                    .filter(|it| classify_routing(it) != RouteCoverage::Covered)
                    .filter_map(|it| it.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect();
                if mcp_bad.is_empty() {
                    findings.push(Finding::pass(
                        "capability-route-coverage-mcp",
                        "every governed MCP declares a valid, explicit routing.route_state",
                    ));
                } else {
                    findings.push(Finding::fail(
                        "capability-route-coverage-mcp",
                        "governed MCPs with invalid or missing routing.route_state",
                        format!(
                            "Fix routing.route_state (valid + explicit) in mcp-registry.yaml for: {}.",
                            mcp_bad.join(", ")
                        ),
                    ));
                }
            }
        }
    }

    findings
}

pub fn run_checks(report: &mut HealthReport, repo_root: &Path) {
    let identity = project_discovery::detect_project(repo_root);
    report.add(git_status_check(repo_root));
    report.add(project_integration_check(&identity));
    report.add(project_protocol_check(repo_root));

    // Source-policy checks apply only to the AGS suite itself. Managed target
    // projects never inherit Cargo or suite workspace layout requirements;
    // source formatting/build checks belong to `ags verify`.
    if identity.is_ags_suite {
        for finding in capability_route_drift_check(repo_root) {
            report.add(finding);
        }
        for finding in capability_route_coverage_check(repo_root) {
            report.add(finding);
        }
        report.add(runtime_profile_declared(repo_root));
        report.add(mcp_registry_codegraph_adopted(repo_root));
        report.add(runtime_profile_template_exists(repo_root));
        report.add(codex_planner_hook_template_exists(repo_root));
        report.add(claude_code_stop_hook_template_exists(repo_root));
    }

    // ── Context memory checks (advisory, read-only) ────────────────────
    report.add(memory_capture_scripts_present());
    report.add(claude_code_memory_capture_wired(repo_root));
    report.add(raw_tool_call_stop_guard_present(repo_root));
    report.add(project_task_memory_status(repo_root));
    report.add(context_capsule_integrity(repo_root));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CheckStatus, Severity};

    /// Coverage gate FAILs when an adopted skill (suite.yaml) has no route_state
    /// in skills-registry; passes when every adopted skill is covered.
    #[test]
    fn coverage_flags_adopted_skill_missing_route_state() {
        let base = std::env::temp_dir().join(format!("ags-cov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        std::fs::write(
            base.join("manifests/skills-registry.yaml"),
            "skills:\n  - name: routed-skill\n    routing:\n      route_state: routable\n  - name: parked-skill\n    routing:\n      route_state: not-routable\n",
        )
        .unwrap();
        std::fs::write(
            base.join("manifests/suite.yaml"),
            "suite:\n  required:\n    - name: \"routed-skill\"\n    - name: \"orphan-skill\"\n  optional:\n    - name: \"parked-skill\"\n  personal:\n    \"routed-skill\":\n      version: x\n",
        )
        .unwrap();
        let findings = capability_route_coverage_check(&base);
        let cov = findings
            .iter()
            .find(|f| f.check_name == "capability-route-coverage")
            .expect("coverage finding present");
        assert_eq!(cov.status, CheckStatus::Fail);
        assert!(cov.detail.as_deref().unwrap_or("").contains("orphan-skill"));
        assert!(!cov.detail.as_deref().unwrap_or("").contains("parked-skill"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Codex adversarial finding: a typo'd / non-mapping route_state must NOT pass
    /// coverage. It passes a naive key-presence check but is dropped by the typed
    /// production parser — so typed-parse coverage must FAIL it, for skills AND
    /// governed MCPs.
    #[test]
    fn coverage_rejects_malformed_route_state() {
        let base = std::env::temp_dir().join(format!("ags-cov-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        std::fs::write(
            base.join("manifests/skills-registry.yaml"),
            "skills:\n  - name: typo-skill\n    routing:\n      route_state: routeable\n  - name: nonmap-skill\n    routing: \"not-a-mapping\"\n  - name: ok-skill\n    routing:\n      route_state: routable\n",
        )
        .unwrap();
        std::fs::write(
            base.join("manifests/suite.yaml"),
            "suite:\n  required:\n    - name: \"typo-skill\"\n    - name: \"nonmap-skill\"\n    - name: \"ok-skill\"\n",
        )
        .unwrap();
        std::fs::write(
            base.join("manifests/mcp-registry.yaml"),
            "mcps:\n  - name: \"bad-mcp\"\n    routing:\n      route_state: nope\n",
        )
        .unwrap();
        let findings = capability_route_coverage_check(&base);
        let cov = findings
            .iter()
            .find(|f| f.check_name == "capability-route-coverage")
            .expect("coverage finding present");
        assert_eq!(
            cov.status,
            CheckStatus::Fail,
            "typo'd / non-mapping route_state must fail coverage"
        );
        let d = cov.detail.as_deref().unwrap_or("");
        assert!(d.contains("typo-skill"), "typo-skill flagged: {d}");
        assert!(d.contains("nonmap-skill"), "nonmap-skill flagged: {d}");
        assert!(!d.contains("ok-skill"), "ok-skill must not be flagged: {d}");
        let mcp = findings
            .iter()
            .find(|f| f.check_name == "capability-route-coverage-mcp")
            .expect("mcp coverage finding present");
        assert_eq!(
            mcp.status,
            CheckStatus::Fail,
            "typo'd MCP route_state fails"
        );
        assert!(mcp.detail.as_deref().unwrap_or("").contains("bad-mcp"));
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── capability_route_drift_check (read-only, hermetic temp manifests) ──

    fn write_clean_skills(dir: &Path) {
        std::fs::write(
            dir.join("manifests/skills-registry.yaml"),
            "skills:\n  - name: auto-brainstorm\n    routing:\n      route_state: retired\n      intent_tags: []\n  - name: auto-debug\n    routing:\n      route_state: retired\n      intent_tags: []\n  - name: auto-verify\n    routing:\n      route_state: retired\n      intent_tags: []\n",
        )
        .unwrap();
    }

    #[test]
    fn capability_route_drift_auth_boundary_fails_on_configured() {
        let base = std::env::temp_dir().join(format!("ags-doctor-cr-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        write_clean_skills(&base);
        // A tracked manifest must never assert a configured auth status.
        std::fs::write(
            base.join("manifests/mcp-registry.yaml"),
            "mcps:\n  - name: x\n    auth_status: configured\n",
        )
        .unwrap();
        let findings = capability_route_drift_check(&base);
        let auth = findings
            .iter()
            .find(|f| f.check_name == "capability-route-auth-boundary")
            .unwrap();
        assert_eq!(auth.status, CheckStatus::Fail);
        let routing = findings
            .iter()
            .find(|f| f.check_name == "capability-route-manifest-routing")
            .unwrap();
        assert_eq!(routing.status, CheckStatus::Pass);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_route_auth_boundary_catches_varied_credential_keys() {
        // Codex review fix: the scan must catch credential-shaped keys beyond the
        // three lowercase substrings — different keys, casing, spacing, nesting.
        let cases = [
            ("apikey", "mcps:\n  - name: x\n    api_key: abc\n"),
            ("credential", "mcps:\n  - name: x\n    credential: abc\n"),
            (
                "authzcased",
                "mcps:\n  - name: x\n    Authorization: Bearer z\n",
            ),
            ("spacedkey", "mcps:\n  - name: x\n    token : abc\n"),
            (
                "nested",
                "mcps:\n  - name: x\n    install:\n      client_secret: abc\n",
            ),
            (
                "authstatus",
                "mcps:\n  - name: x\n    auth_status: configured\n",
            ),
        ];
        for (tag, mcp) in cases {
            let base = std::env::temp_dir()
                .join(format!("ags-doctor-cr-cred-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(base.join("manifests")).unwrap();
            write_clean_skills(&base);
            std::fs::write(base.join("manifests/mcp-registry.yaml"), mcp).unwrap();
            let findings = capability_route_drift_check(&base);
            let auth = findings
                .iter()
                .find(|f| f.check_name == "capability-route-auth-boundary")
                .unwrap();
            assert_eq!(
                auth.status,
                CheckStatus::Fail,
                "should flag credential variant: {tag}"
            );
            let _ = std::fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn capability_route_auth_boundary_no_false_positive_on_legit_keys() {
        // authority / requires_auth / auth_kind / is_compatibility_alias are all
        // legitimate keys in the real manifests and must NOT trip the gate.
        let base = std::env::temp_dir().join(format!("ags-doctor-cr-legit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        write_clean_skills(&base);
        std::fs::write(
            base.join("manifests/mcp-registry.yaml"),
            "mcps:\n  - name: x\n    requires_auth: false\n    auth_kind: feishu\n    authority:\n      allowed:\n        - do a thing\n      denied:\n        - Read or write real tokens, secrets, or settings.\n",
        )
        .unwrap();
        let findings = capability_route_drift_check(&base);
        let auth = findings
            .iter()
            .find(|f| f.check_name == "capability-route-auth-boundary")
            .unwrap();
        assert_eq!(
            auth.status,
            CheckStatus::Pass,
            "legit auth-ish keys must not false-positive"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_route_auth_boundary_clean_on_real_manifests() {
        // The hardened scanner must not regress the real repo manifests (which have
        // `authority:` blocks, `requires_auth:`, and prose mentioning tokens/secrets).
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let findings = capability_route_drift_check(repo_root);
        let auth = findings
            .iter()
            .find(|f| f.check_name == "capability-route-auth-boundary")
            .unwrap();
        assert_eq!(
            auth.status,
            CheckStatus::Pass,
            "real tracked manifests must pass the hardened auth-boundary scan"
        );
    }

    #[test]
    fn capability_route_drift_passes_clean_and_writes_nothing() {
        let base = std::env::temp_dir().join(format!("ags-doctor-cr-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        write_clean_skills(&base);
        std::fs::write(base.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
        let findings = capability_route_drift_check(&base);
        let auth = findings
            .iter()
            .find(|f| f.check_name == "capability-route-auth-boundary")
            .unwrap();
        assert_eq!(auth.status, CheckStatus::Pass);
        // Read-only: the drift check writes nothing (manifests dir unchanged).
        let n = std::fs::read_dir(base.join("manifests")).unwrap().count();
        assert_eq!(n, 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_route_drift_allows_active_compat_aliases() {
        let base =
            std::env::temp_dir().join(format!("ags-doctor-cr-active-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        std::fs::write(
            base.join("manifests/skills-registry.yaml"),
            "skills:\n  - name: auto-brainstorm\n    routing:\n      route_state: routable\n      intent_tags: [brainstorm]\n      is_compatibility_alias: true\n  - name: auto-debug\n    routing:\n      route_state: routable\n      intent_tags: [debug]\n      is_compatibility_alias: true\n  - name: auto-verify\n    routing:\n      route_state: routable\n      intent_tags: [verify]\n      is_compatibility_alias: true\n",
        )
        .unwrap();
        std::fs::write(base.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
        let findings = capability_route_drift_check(&base);
        let routing = findings
            .iter()
            .find(|f| f.check_name == "capability-route-manifest-routing")
            .unwrap();
        assert_eq!(routing.status, CheckStatus::Pass);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_route_enrollment_finding_is_never_a_fail() {
        // The enrollment finding reflects machine-local state (present ⇒ Info,
        // absent ⇒ Warn). Either way it must NEVER be a Fail: a missing enrollment
        // is advisory degraded, not a blocking failure. Locks that contract.
        let base = std::env::temp_dir().join(format!("ags-doctor-cr-enr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        write_clean_skills(&base);
        std::fs::write(base.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
        let findings = capability_route_drift_check(&base);
        let enr = findings
            .iter()
            .find(|f| f.check_name == "capability-route-enrollment")
            .expect("enrollment finding is always emitted");
        assert_ne!(enr.status, CheckStatus::Fail);
        assert_ne!(enr.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn capability_route_drift_warns_on_missing_alias_lifecycle() {
        let base = std::env::temp_dir().join(format!("ags-doctor-cr-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("manifests")).unwrap();
        // auto-debug present but neither retired nor explicitly marked as a
        // compatibility alias; the other two absent → warn.
        std::fs::write(
            base.join("manifests/skills-registry.yaml"),
            "skills:\n  - name: auto-debug\n    routing:\n      intent_tags: [debug]\n",
        )
        .unwrap();
        std::fs::write(base.join("manifests/mcp-registry.yaml"), "mcps: []\n").unwrap();
        let findings = capability_route_drift_check(&base);
        let routing = findings
            .iter()
            .find(|f| f.check_name == "capability-route-manifest-routing")
            .unwrap();
        assert_eq!(routing.status, CheckStatus::Warn);
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── Context memory checks ─────────────────────────────────────────

    fn mem_tmp(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "ags-suite-doctor-memory-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn write_project_slug(repo: &Path, slug: &str) {
        std::fs::create_dir_all(repo.join("config")).unwrap();
        std::fs::write(
            repo.join("config/agent-project-profile.yaml"),
            format!("schema_version: 1\nproject:\n  slug: {slug}\n"),
        )
        .unwrap();
    }

    #[test]
    fn memory_scripts_present_pass_and_warn() {
        let home = mem_tmp("scripts");
        let f = memory_capture_scripts_present_at(&home);
        assert_eq!(f.status, CheckStatus::Warn);

        let dir = home.join(".agents/scripts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("context-memory.sh"), "#!/usr/bin/env bash\n").unwrap();
        std::fs::write(
            dir.join("claude-stop-memory-capture.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();
        let f = memory_capture_scripts_present_at(&home);
        assert_eq!(f.status, CheckStatus::Pass);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn memory_capture_wired_distinguishes_states() {
        let repo = mem_tmp("wired");
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::write(
            repo.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[{"hooks":[
                {"command":"node /x/raw-tool-call-stop-guard.js"},
                {"command":"python3 \"$HOME/.agents/scripts/claude-stop-memory-capture.py\""}
            ]}]}}"#,
        )
        .unwrap();
        let f = claude_code_memory_capture_wired(&repo);
        assert_eq!(f.status, CheckStatus::Pass);
        assert!(f.message.contains("raw-guard"));
        assert!(f.message.contains("memory-capture"));
        let f = raw_tool_call_stop_guard_present(&repo);
        assert_eq!(f.status, CheckStatus::Pass);

        std::fs::write(
            repo.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"node /x/user-stop-hook.js"}]}]}}"#,
        )
        .unwrap();
        let f = claude_code_memory_capture_wired(&repo);
        assert_eq!(f.status, CheckStatus::Warn);
        let f = raw_tool_call_stop_guard_present(&repo);
        assert_eq!(f.status, CheckStatus::Warn);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn task_memory_and_capsule_integrity_states() {
        let root = mem_tmp("store");
        let repo = root.join("repo");
        let home = root.join("home");
        std::fs::create_dir_all(&repo).unwrap();
        write_project_slug(&repo, "doctor-memory");

        let missing_task = project_task_memory_status_at(&repo, &home);
        assert_eq!(missing_task.status, CheckStatus::Warn);
        let missing_capsule = context_capsule_integrity_at(&repo, &home);
        assert_eq!(missing_capsule.status, CheckStatus::Warn);

        let memory_dir = project_memory_dir_at(&repo, &home);
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("task-memory.md"), "# Task Memory\n").unwrap();
        std::fs::write(
            memory_dir.join("context-capsule.md"),
            "# Context Capsule\n\n## 项目设计目的\n\nfixture\n",
        )
        .unwrap();

        let task = project_task_memory_status_at(&repo, &home);
        assert_eq!(task.status, CheckStatus::Pass);
        let capsule = context_capsule_integrity_at(&repo, &home);
        assert_eq!(capsule.status, CheckStatus::Pass);

        std::fs::write(memory_dir.join("context-capsule.md"), "# Context Capsule\n").unwrap();
        let capsule = context_capsule_integrity_at(&repo, &home);
        assert_eq!(capsule.status, CheckStatus::Warn);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── Finding constructors smoke ────────────────────────────────────

    #[test]
    fn git_status_pass_has_correct_shape() {
        let f = Finding::pass("git-status", "working tree clean");
        assert_eq!(f.check_name, "git-status");
        assert_eq!(f.severity, Severity::Info);
        assert!(f.detail.is_none());
    }

    #[test]
    fn cargo_fmt_fail_has_detail() {
        let f = Finding::fail("cargo-fmt", "failed", "run cargo fmt");
        assert_eq!(f.severity, Severity::Fail);
        assert_eq!(f.detail.as_deref(), Some("run cargo fmt"));
    }

    #[test]
    fn runtime_profile_declared_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = runtime_profile_declared(repo_root);
        if is_public_edition(repo_root) {
            assert_eq!(f.status, CheckStatus::Skip);
        } else {
            // Private/stable runtime should declare installed profiles.
            assert_eq!(f.status, CheckStatus::Pass);
        }
        assert_eq!(f.check_name, "runtime_profile_declared");
    }

    #[test]
    fn runtime_profile_declared_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-runtime-profile-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = runtime_profile_declared(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn runtime_profile_declared_fails_on_invalid_yaml() {
        let tmp = std::env::temp_dir().join("ags-runtime-profile-invalid-yaml-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("manifests")).unwrap();
        std::fs::write(
            tmp.join("manifests/runtime-profiles.yaml"),
            "schema_version: \"1.0\"\nprofiles: [claude-code-executor\n",
        )
        .unwrap();
        let f = runtime_profile_declared(&tmp);
        assert_eq!(f.status, CheckStatus::Fail);
        assert_eq!(f.severity, Severity::Fail);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mcp_registry_codegraph_adopted_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = mcp_registry_codegraph_adopted(repo_root);
        // Adoption is an info-level check — it never blocks (Fail), regardless of
        // edition or whether the codegraph MCP is registered in this repo.
        assert_ne!(f.severity, Severity::Fail);
        assert_eq!(f.check_name, "mcp_registry_codegraph_adopted");
    }
    // ── Template existence tests ───────────────────────────────────────

    #[test]
    fn runtime_profile_template_exists_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = runtime_profile_template_exists(repo_root);
        assert_eq!(f.status, CheckStatus::Warn);
        assert_eq!(f.check_name, "runtime_profile_template_exists");
    }

    #[test]
    fn runtime_profile_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = runtime_profile_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn codex_planner_hook_template_exists_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = codex_planner_hook_template_exists(repo_root);
        assert_eq!(f.status, CheckStatus::Warn);
        assert_eq!(f.check_name, "codex_planner_hook_template_exists");
    }

    #[test]
    fn codex_planner_hook_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-planner-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = codex_planner_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn claude_code_stop_hook_template_exists_in_ags_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let f = claude_code_stop_hook_template_exists(repo_root);
        assert_eq!(f.status, CheckStatus::Warn);
        assert_eq!(f.check_name, "claude_code_stop_hook_template_exists");
    }

    #[test]
    fn claude_code_stop_hook_template_fail_on_missing() {
        let tmp = std::env::temp_dir().join("ags-stop-template-missing-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = claude_code_stop_hook_template_exists(&tmp);
        assert_eq!(f.status, CheckStatus::Warn);
        assert!(f.message.contains("missing"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn managed_project_doctor_skips_source_quality_checks() {
        let tmp = std::env::temp_dir().join(format!(
            "ags-doctor-managed-project-scope-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("config")).unwrap();
        std::fs::write(
            tmp.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: managed-project-scope\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("AGENTS.md"),
            "# AGENTS.md\n\nThis project uses AGENT_SUITE_PROTOCOL.md.\n",
        )
        .unwrap();

        let mut report = HealthReport::new("managed-project-doctor");
        run_checks(&mut report, &tmp);

        for forbidden in [
            "cargo-fmt",
            "structure-Cargo.toml",
            "structure-crates",
            "structure-scripts-verify.sh",
        ] {
            assert!(
                report
                    .findings
                    .iter()
                    .all(|finding| finding.check_name != forbidden),
                "managed project doctor must not emit {forbidden}: {:?}",
                report.findings
            );
        }
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.check_name == "project-integration"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.check_name == "project-protocol"));
        let _ = std::fs::remove_dir_all(tmp);
    }
}
