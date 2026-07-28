//! Canonical host platform facts and deterministic detection.
//!
//! Callers provide executable and application-bundle probes. This module owns
//! host identities, supported signals, primary-host selection, registration
//! advice, and capability-verification support.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpListFormat {
    Claude,
    Codex,
    Omp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProbeProtocol {
    DirectCommand,
    JsonlRpcCommand { command: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryProtocol {
    ClaudeCommandHooks,
    CodexCommandHooks,
    CursorCommandHooks,
    OmpExtension,
}

impl MemoryProtocol {
    pub fn adapter_id(self) -> &'static str {
        match self {
            Self::ClaudeCommandHooks => "claude-command-hooks",
            Self::CodexCommandHooks => "codex-command-hooks",
            Self::CursorCommandHooks => "cursor-command-hooks",
            Self::OmpExtension => "omp-extension",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleOutputProtocol {
    ClaudeCompatible,
    Cursor,
}

#[derive(Debug, Clone, Copy)]
pub struct McpProbeSpec {
    pub protocol: McpProbeProtocol,
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub env: &'static [(&'static str, &'static str)],
    pub format: McpListFormat,
    pub evidence_source: &'static str,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct AgentPlatformSpec {
    pub id: &'static str,
    pub display: &'static str,
    pub cli_names: &'static [&'static str],
    pub config_subdirs: &'static [&'static str],
    pub app_bundles: &'static [&'static str],
    pub mcp_host_command: &'static str,
    pub native_skill_subdir: Option<&'static str>,
    pub loads_shared_agent_skills: bool,
    pub loads_codex_plugin_skills: bool,
    pub mcp_probe: Option<McpProbeSpec>,
    pub mcp_registrar: Option<&'static str>,
    pub memory_protocol: Option<MemoryProtocol>,
    pub lifecycle_output: Option<LifecycleOutputProtocol>,
    pub verify_supported: bool,
}

pub const AGENT_PLATFORM_SPECS: &[AgentPlatformSpec] = &[
    AgentPlatformSpec {
        id: "claude-code",
        display: "Claude Code",
        cli_names: &["claude"],
        config_subdirs: &[".claude"],
        app_bundles: &[],
        mcp_host_command: "claude mcp add ags -- ags mcp serve --transport stdio",
        native_skill_subdir: Some(".claude/skills"),
        loads_shared_agent_skills: false,
        loads_codex_plugin_skills: false,
        mcp_probe: Some(McpProbeSpec {
            protocol: McpProbeProtocol::DirectCommand,
            program: "claude",
            args: &["mcp", "list"],
            env: &[],
            format: McpListFormat::Claude,
            evidence_source: "`claude mcp list`",
            timeout_ms: 10_000,
        }),
        mcp_registrar: Some("claude"),
        memory_protocol: Some(MemoryProtocol::ClaudeCommandHooks),
        lifecycle_output: Some(LifecycleOutputProtocol::ClaudeCompatible),
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "codex",
        display: "Codex",
        cli_names: &["codex"],
        config_subdirs: &[".codex"],
        app_bundles: &[],
        mcp_host_command: "codex mcp add ags -- ags mcp serve --transport stdio",
        native_skill_subdir: Some(".codex/skills"),
        loads_shared_agent_skills: true,
        loads_codex_plugin_skills: true,
        mcp_probe: Some(McpProbeSpec {
            protocol: McpProbeProtocol::DirectCommand,
            program: "codex",
            args: &["mcp", "list"],
            env: &[],
            format: McpListFormat::Codex,
            evidence_source: "`codex mcp list`",
            timeout_ms: 10_000,
        }),
        mcp_registrar: Some("codex"),
        memory_protocol: Some(MemoryProtocol::CodexCommandHooks),
        lifecycle_output: Some(LifecycleOutputProtocol::ClaudeCompatible),
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "omp",
        display: "Oh My Pi (OMP)",
        cli_names: &["omp"],
        config_subdirs: &[".omp", ".omp/agent"],
        app_bundles: &[],
        mcp_host_command: "configure AGS in OMP MCP settings; verify through OMP's native JSONL RPC `/mcp list` protocol",
        native_skill_subdir: Some(".omp/agent/skills"),
        loads_shared_agent_skills: true,
        loads_codex_plugin_skills: false,
        mcp_probe: Some(McpProbeSpec {
            protocol: McpProbeProtocol::JsonlRpcCommand {
                command: "/mcp list",
            },
            program: "omp",
            args: &["--mode", "rpc", "--no-session"],
            env: &[],
            format: McpListFormat::Omp,
            evidence_source: "OMP native JSONL RPC `/mcp list`",
            timeout_ms: 10_000,
        }),
        mcp_registrar: None,
        memory_protocol: Some(MemoryProtocol::OmpExtension),
        lifecycle_output: Some(LifecycleOutputProtocol::ClaudeCompatible),
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "cursor",
        display: "Cursor",
        cli_names: &["cursor-agent", "cursor"],
        config_subdirs: &[".cursor"],
        app_bundles: &["Cursor.app"],
        mcp_host_command: "configure AGS in .cursor/mcp.json or ~/.cursor/mcp.json; verify with `cursor-agent mcp list`",
        native_skill_subdir: Some(".cursor/skills"),
        loads_shared_agent_skills: true,
        loads_codex_plugin_skills: false,
        mcp_probe: Some(McpProbeSpec {
            protocol: McpProbeProtocol::DirectCommand,
            program: "cursor-agent",
            args: &["mcp", "list"],
            env: &[("AGENT_CLI_CREDENTIAL_STORE", "file")],
            format: McpListFormat::Claude,
            evidence_source: "`cursor-agent mcp list`",
            timeout_ms: 10_000,
        }),
        mcp_registrar: None,
        memory_protocol: Some(MemoryProtocol::CursorCommandHooks),
        lifecycle_output: Some(LifecycleOutputProtocol::Cursor),
        verify_supported: true,
    },
    AgentPlatformSpec {
        id: "workbuddy",
        display: "Tencent Agent (WorkBuddy)",
        cli_names: &["workbuddy", "workbuddy-ide"],
        config_subdirs: &[".workbuddy"],
        app_bundles: &["WorkBuddy.app", "WorkBuddy IDE.app"],
        mcp_host_command: "register AGS MCP in WorkBuddy host config (exposes ags_preflight / ags_agent_instructions / ags_task_validate / ags_policy_resolve); AGS never runs the registrar",
        native_skill_subdir: None,
        loads_shared_agent_skills: false,
        loads_codex_plugin_skills: false,
        mcp_probe: None,
        mcp_registrar: None,
        memory_protocol: None,
        lifecycle_output: None,
        verify_supported: false,
    },
    AgentPlatformSpec {
        id: "codebuddy-code",
        display: "Tencent Agent (CodeBuddy-Code)",
        cli_names: &["codebuddy", "codebuddy-code"],
        config_subdirs: &[".codebuddy"],
        app_bundles: &["CodeBuddy CN.app", "CodeBuddy Code.app", "CodeBuddy.app"],
        mcp_host_command: "register AGS MCP in CodeBuddy-Code host config (exposes ags_preflight / ags_agent_instructions / ags_task_validate / ags_policy_resolve); AGS never runs the registrar",
        native_skill_subdir: Some(".codebuddy/skills"),
        loads_shared_agent_skills: false,
        loads_codex_plugin_skills: false,
        mcp_probe: None,
        mcp_registrar: None,
        memory_protocol: None,
        lifecycle_output: None,
        verify_supported: false,
    },
];

pub fn platform_spec(id: &str) -> Option<&'static AgentPlatformSpec> {
    AGENT_PLATFORM_SPECS.iter().find(|spec| spec.id == id)
}

pub fn supported_skill_hosts() -> impl Iterator<Item = &'static str> {
    AGENT_PLATFORM_SPECS
        .iter()
        .filter(|spec| spec.native_skill_subdir.is_some())
        .map(|spec| spec.id)
}

pub fn static_skill_roots(home: &Path, host: &str) -> Vec<PathBuf> {
    let Some(spec) = platform_spec(host) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if let Some(subdir) = spec.native_skill_subdir {
        roots.push(home.join(subdir));
    }
    if spec.loads_shared_agent_skills {
        roots.push(home.join(".agents/skills"));
    }
    roots
}

#[derive(Debug, Clone)]
pub struct AgentPlatformStatus {
    pub id: String,
    pub display: String,
    pub cli_present: bool,
    pub config_present: bool,
    pub app_present: bool,
    pub detected: bool,
    pub is_primary: bool,
    pub mcp_host_command: String,
    pub drift_check: String,
}

#[derive(Debug, Clone)]
pub struct CrossPlatformInitPlan {
    pub primary_agent: Option<String>,
    pub platforms: Vec<AgentPlatformStatus>,
}

pub fn cross_platform_init_plan(
    home: &Path,
    command_present: &dyn Fn(&str) -> bool,
) -> CrossPlatformInitPlan {
    cross_platform_init_plan_with_detectors(home, command_present, &|bundle| {
        application_bundle_present(home, bundle)
    })
}

pub fn cross_platform_init_plan_with_detectors(
    home: &Path,
    command_present: &dyn Fn(&str) -> bool,
    app_present: &dyn Fn(&str) -> bool,
) -> CrossPlatformInitPlan {
    let mut platforms = AGENT_PLATFORM_SPECS
        .iter()
        .map(|spec| {
            let cli_present = spec.cli_names.iter().any(|name| command_present(name));
            let config_present = spec
                .config_subdirs
                .iter()
                .any(|directory| home.join(directory).is_dir());
            let app_present = spec.app_bundles.iter().any(|bundle| app_present(bundle));
            let drift_check = if spec.verify_supported {
                format!("ags capability verify --host {}", spec.id)
            } else {
                format!("ags capability verify --host {} (reserved)", spec.id)
            };
            AgentPlatformStatus {
                id: spec.id.to_string(),
                display: spec.display.to_string(),
                cli_present,
                config_present,
                app_present,
                detected: cli_present || config_present || app_present,
                is_primary: false,
                mcp_host_command: spec.mcp_host_command.to_string(),
                drift_check,
            }
        })
        .collect::<Vec<_>>();

    let primary_index = platforms
        .iter()
        .position(|platform| platform.cli_present && platform.config_present)
        .or_else(|| {
            platforms
                .iter()
                .position(|platform| platform.config_present && platform.app_present)
        })
        .or_else(|| {
            platforms
                .iter()
                .position(|platform| platform.config_present)
        })
        .or_else(|| platforms.iter().position(|platform| platform.detected));
    if let Some(index) = primary_index {
        platforms[index].is_primary = true;
    }

    CrossPlatformInitPlan {
        primary_agent: primary_index.map(|index| platforms[index].id.clone()),
        platforms,
    }
}

fn application_bundle_present(home: &Path, bundle_name: &str) -> bool {
    [
        Path::new("/Applications").join(bundle_name),
        Path::new("/System/Applications").join(bundle_name),
        home.join("Applications").join(bundle_name),
    ]
    .iter()
    .any(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> std::path::PathBuf {
        let base =
            std::env::temp_dir().join(format!("ags-host-platform-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn primary_selection_prefers_cli_plus_config() {
        let home = temp_home("primary");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let plan =
            cross_platform_init_plan_with_detectors(&home, &|name| name == "claude", &|_| false);
        assert_eq!(plan.primary_agent.as_deref(), Some("claude-code"));
        assert!(
            plan.platforms
                .iter()
                .find(|host| host.id == "claude-code")
                .unwrap()
                .is_primary
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn omp_uses_its_own_live_runtime_protocol() {
        let omp = platform_spec("omp").unwrap();
        let probe = omp.mcp_probe.unwrap();
        assert_eq!(probe.program, "omp");
        assert_eq!(
            probe.protocol,
            McpProbeProtocol::JsonlRpcCommand {
                command: "/mcp list"
            }
        );
        assert!(omp.mcp_registrar.is_none());
        assert_eq!(omp.memory_protocol, Some(MemoryProtocol::OmpExtension));
    }

    #[test]
    fn cursor_and_omp_share_the_agent_skill_store_without_codex_plugins() {
        let home = Path::new("/tmp/ags-host-facts");
        for host in ["cursor", "omp"] {
            assert_eq!(
                static_skill_roots(home, host),
                vec![
                    home.join(platform_spec(host).unwrap().native_skill_subdir.unwrap()),
                    home.join(".agents/skills"),
                ]
            );
            assert!(!platform_spec(host).unwrap().loads_codex_plugin_skills);
        }
        assert!(platform_spec("codex").unwrap().loads_codex_plugin_skills);
    }

    #[test]
    fn cursor_uses_native_command_hooks_and_cursor_output_protocol() {
        let cursor = platform_spec("cursor").unwrap();
        assert_eq!(
            cursor.memory_protocol,
            Some(MemoryProtocol::CursorCommandHooks)
        );
        assert_eq!(
            cursor.lifecycle_output,
            Some(LifecycleOutputProtocol::Cursor)
        );
        assert!(cursor.verify_supported);
        let probe = cursor.mcp_probe.unwrap();
        assert_eq!(probe.program, "cursor-agent");
        assert_eq!(probe.args, ["mcp", "list"]);
        assert_eq!(probe.env, [("AGENT_CLI_CREDENTIAL_STORE", "file")]);
    }

    #[test]
    fn codebuddy_app_does_not_report_workbuddy() {
        let home = temp_home("codebuddy");
        let plan = cross_platform_init_plan_with_detectors(&home, &|_| false, &|bundle| {
            bundle == "CodeBuddy CN.app"
        });
        assert!(
            !plan
                .platforms
                .iter()
                .find(|host| host.id == "workbuddy")
                .unwrap()
                .detected
        );
        assert!(
            plan.platforms
                .iter()
                .find(|host| host.id == "codebuddy-code")
                .unwrap()
                .detected
        );
        let _ = std::fs::remove_dir_all(home);
    }
}
