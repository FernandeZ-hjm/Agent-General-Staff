//! Deterministic public onboarding assessment for AGS.
//!
//! This module owns the `assess -> plan -> apply -> verify` vocabulary. It is
//! assessment and planning never launch a process or write files. The separate
//! [`execute_action`] entry accepts only a closed [`OnboardingAction`] returned
//! by a plan, after the caller has enforced explicit confirmation or an MCP
//! DecisionLease.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod manifest;

use manifest::{
    resolve_third_party_manifest, CapabilityKind, ManifestResolution, ThirdPartyCapability,
};

pub const ONBOARDING_PLAN_SCHEMA_VERSION: &str = "0.3.0-onboarding-plan";
const EMBEDDED_PUBLIC_PROFILE: &str = include_str!("../../../manifests/onboarding-public.yaml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentState {
    Absent,
    InstalledNotVisible,
    VisibleNotReady,
    ActiveReady,
    UpdateAvailable,
    BlockedUntrustedSource,
    BlockedMissingIntegrity,
    UnsupportedHost,
}

impl ComponentState {
    pub fn is_ready(self) -> bool {
        self == Self::ActiveReady
    }

    pub fn is_blocked(self) -> bool {
        matches!(
            self,
            Self::BlockedUntrustedSource | Self::BlockedMissingIntegrity | Self::UnsupportedHost
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OnboardingAction {
    ProjectInit {
        target: String,
    },
    RegisterAgsMcp {
        registrar: String,
        executable: String,
    },
    AdoptSkill {
        source: String,
        host: String,
    },
    RegisterNpmMcp {
        registrar: String,
        server_name: String,
        package: String,
        integrity: String,
    },
    RegisterCommandMcp {
        registrar: String,
        server_name: String,
        command: String,
        args: Vec<String>,
    },
    InstallNpmCli {
        package: String,
        integrity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingItem {
    pub id: String,
    pub category: String,
    pub required: bool,
    pub state: ComponentState,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<OnboardingAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingReadiness {
    pub route_state: String,
    pub metadata_complete: bool,
    pub semantic_probe: String,
    pub boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookReadiness {
    pub host: String,
    pub events: Vec<String>,
    pub config_present: bool,
    pub health_probe: String,
    pub event_probe: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingPlan {
    pub schema_version: String,
    pub profile: String,
    pub host: String,
    pub target: String,
    pub manifest_source: String,
    pub manifest_hash: String,
    pub manifest_freshness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_fallback_reason: Option<String>,
    pub bootstrap_required: bool,
    pub ready: bool,
    pub plan_hash: String,
    pub items: Vec<OnboardingItem>,
    pub excluded_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionExecution {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackAdvice {
    pub affected_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inverse_command: Option<String>,
    pub detail: String,
}

/// Produce conservative, plan-only rollback advice for one closed onboarding
/// action. AGS never executes these inverse steps automatically.
pub fn rollback_advice(action: &OnboardingAction) -> Vec<RollbackAdvice> {
    let advice = match action {
        OnboardingAction::ProjectInit { target } => RollbackAdvice {
            affected_path: target.clone(),
            inverse_command: None,
            detail: "inspect the init receipt and remove only AGS-created project files; preserve pre-existing files"
                .to_string(),
        },
        OnboardingAction::RegisterAgsMcp { registrar, .. } => RollbackAdvice {
            affected_path: format!("{registrar}:mcp:ags"),
            inverse_command: Some(registrar_remove_command(registrar, "ags")),
            detail: "remove the AGS MCP registration after confirming it was created by this receipt"
                .to_string(),
        },
        OnboardingAction::AdoptSkill { source, host } => RollbackAdvice {
            affected_path: format!("{host}:skill:{source}"),
            inverse_command: None,
            detail: "inspect the nested `ags skill adopt` receipt and restore its recorded overlay/thin-index state"
                .to_string(),
        },
        OnboardingAction::RegisterNpmMcp {
            registrar,
            server_name,
            ..
        }
        | OnboardingAction::RegisterCommandMcp {
            registrar,
            server_name,
            ..
        } => RollbackAdvice {
            affected_path: format!("{registrar}:mcp:{server_name}"),
            inverse_command: Some(registrar_remove_command(registrar, server_name)),
            detail: "remove only the MCP registration created by this onboarding action".to_string(),
        },
        OnboardingAction::InstallNpmCli { package, .. } => RollbackAdvice {
            affected_path: format!("npm-global:{}", npm_package_name(package)),
            inverse_command: Some(format!(
                "npm uninstall --global {}",
                npm_package_name(package)
            )),
            detail: "uninstall only after confirming the package was not present before onboarding"
                .to_string(),
        },
    };
    vec![advice]
}

fn registrar_remove_command(registrar: &str, server_name: &str) -> String {
    if registrar == "claude" {
        format!("claude mcp remove -s user {server_name}")
    } else {
        format!("{registrar} mcp remove {server_name}")
    }
}

fn npm_package_name(spec: &str) -> &str {
    if let Some(scoped) = spec.strip_prefix('@') {
        scoped
            .rfind('@')
            .map(|index| &spec[..index + 1])
            .unwrap_or(spec)
    } else {
        spec.split_once('@').map(|(name, _)| name).unwrap_or(spec)
    }
}

#[derive(Debug, Clone)]
pub struct AssessContext<'a> {
    pub source_root: &'a Path,
    pub home: &'a Path,
    pub target: &'a Path,
    pub host: &'a str,
    pub ags_executable: &'a Path,
    /// True when assessment is being served through a live AGS MCP connection.
    pub mcp_connected: bool,
    /// Read-only official registrar probe for the AGS MCP entry.
    pub host_registered: Option<bool>,
    /// MCP server ids proven by an official host registrar probe.
    pub registered_mcp_ids: &'a [String],
    /// Exact skills proven Active+Ready by the current host capability
    /// snapshot. This is the routing truth source; filesystem visibility alone
    /// is never enough to claim a skill is ready.
    pub active_skill_ids: &'a [String],
}

#[derive(Debug, Default, Deserialize)]
struct PublicProfile {
    #[serde(default)]
    profile: String,
    #[serde(default)]
    excluded_capabilities: Vec<String>,
    #[serde(default)]
    developer_tools: Vec<DeveloperTool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DeveloperTool {
    id: String,
    command: String,
    #[serde(default)]
    purpose: String,
}

pub fn assess_public(ctx: &AssessContext<'_>) -> Result<OnboardingPlan, String> {
    let third_party = resolve_third_party_manifest(ctx.source_root)?;
    assess_public_with_resolution(ctx, &third_party)
}

/// Assess onboarding against one immutable third-party manifest resolution.
///
/// Callers that also build a capability snapshot should resolve once and pass
/// the same value to both paths so onboarding and natural-language routing are
/// bound to the exact same content hash.
pub fn assess_public_with_resolution(
    ctx: &AssessContext<'_>,
    third_party: &ManifestResolution,
) -> Result<OnboardingPlan, String> {
    let profile_path = ctx.source_root.join("manifests/onboarding-public.yaml");
    let profile_content = std::fs::read_to_string(&profile_path)
        .unwrap_or_else(|_| EMBEDDED_PUBLIC_PROFILE.to_string());
    let profile: PublicProfile = serde_yaml::from_str(&profile_content)
        .map_err(|error| format!("cannot parse {}: {error}", profile_path.display()))?;
    if profile.profile != "public" {
        return Err("onboarding profile must be public".to_string());
    }

    let mut items = Vec::new();
    items.push(kernel_item(ctx));
    items.push(project_item(ctx));
    items.push(host_item(ctx));

    for capability in third_party
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.applies_to("public"))
    {
        items.push(capability_item(ctx, capability));
    }

    for tool in &profile.developer_tools {
        let present = command_in_path(&tool.command).is_some();
        items.push(OnboardingItem {
            id: format!("developer-tool:{}", tool.id),
            category: "developer-tool".to_string(),
            required: false,
            state: if present {
                ComponentState::ActiveReady
            } else {
                ComponentState::Absent
            },
            reason: if present {
                format!("{} is available on PATH", tool.command)
            } else {
                format!(
                    "{} is optional and used only for {}; AGS does not install system toolchains",
                    tool.command, tool.purpose
                )
            },
            source: None,
            license: None,
            integrity: None,
            routing: None,
            hook: None,
            action: None,
        });
    }

    let bootstrap_required = items
        .iter()
        .any(|item| item.required && !item.state.is_ready());
    let ready = !bootstrap_required;
    let target = normalized_path(ctx.target);
    let mut plan = OnboardingPlan {
        schema_version: ONBOARDING_PLAN_SCHEMA_VERSION.to_string(),
        profile: profile.profile,
        host: ctx.host.to_string(),
        target,
        manifest_source: third_party.source.clone(),
        manifest_hash: third_party.content_hash.clone(),
        manifest_freshness: third_party.freshness.clone(),
        manifest_fallback_reason: third_party.fallback_reason.clone(),
        bootstrap_required,
        ready,
        plan_hash: String::new(),
        items,
        excluded_capabilities: profile.excluded_capabilities,
    };
    plan.plan_hash = plan_hash(&plan)?;
    Ok(plan)
}

pub fn find_action<'a>(
    plan: &'a OnboardingPlan,
    item_id: &str,
) -> Result<&'a OnboardingAction, String> {
    let item = plan
        .items
        .iter()
        .find(|item| item.id == item_id)
        .ok_or_else(|| format!("unknown onboarding item: {item_id}"))?;
    item.action
        .as_ref()
        .ok_or_else(|| format!("onboarding item is not applyable: {item_id}"))
}

pub fn action_hash(plan_hash: &str, item_id: &str, action: &OnboardingAction) -> String {
    let bytes = serde_json::to_vec(&(plan_hash, item_id, action)).unwrap_or_default();
    sha256(&bytes)
}

/// Execute one closed action already selected from an assessed plan.
///
/// No shell is used. Skill adoption deliberately performs the existing
/// review-plan call before the apply call so `ags skill adopt` retains its
/// saved-plan integrity and TOCTOU checks.
pub fn execute_action(
    action: &OnboardingAction,
    ags_executable: &Path,
) -> Result<ActionExecution, String> {
    let output = match action {
        OnboardingAction::ProjectInit { target } => Command::new(ags_executable)
            .args(["init", "--target", target, "--format", "json"])
            .output()
            .map_err(|error| format!("project init launch failed: {error}"))?,
        OnboardingAction::RegisterAgsMcp {
            registrar,
            executable,
        } => {
            let mut command = Command::new(registrar);
            command.args(["mcp", "add"]);
            if registrar == "claude" {
                command.args(["-s", "user"]);
            }
            command
                .args([
                    "ags",
                    "--",
                    executable,
                    "mcp",
                    "serve",
                    "--transport",
                    "stdio",
                ])
                .output()
                .map_err(|error| format!("{registrar} registrar launch failed: {error}"))?
        }
        OnboardingAction::AdoptSkill { source, host } => {
            let planned = Command::new(ags_executable)
                .args(["skill", "adopt", source, "--host", host, "--format", "json"])
                .output()
                .map_err(|error| format!("skill adoption plan failed: {error}"))?;
            if !planned.status.success() {
                planned
            } else {
                Command::new(ags_executable)
                    .args([
                        "skill", "adopt", source, "--host", host, "--apply", "--format", "json",
                    ])
                    .output()
                    .map_err(|error| format!("skill adoption apply failed: {error}"))?
            }
        }
        OnboardingAction::RegisterNpmMcp {
            registrar,
            server_name,
            package,
            integrity,
        } => {
            verify_npm_integrity(package, integrity)?;
            let mut command = Command::new(registrar);
            command.args(["mcp", "add"]);
            if registrar == "claude" {
                command.args(["-s", "user"]);
            }
            command
                .args([server_name, "--", "npx", "-y", package])
                .output()
                .map_err(|error| format!("{registrar} registrar launch failed: {error}"))?
        }
        OnboardingAction::RegisterCommandMcp {
            registrar,
            server_name,
            command: executable,
            args,
        } => {
            let mut command = Command::new(registrar);
            command.args(["mcp", "add"]);
            if registrar == "claude" {
                command.args(["-s", "user"]);
            }
            command
                .arg(server_name)
                .arg("--")
                .arg(executable)
                .args(args);
            command
                .output()
                .map_err(|error| format!("{registrar} registrar launch failed: {error}"))?
        }
        OnboardingAction::InstallNpmCli { package, integrity } => {
            verify_npm_integrity(package, integrity)?;
            Command::new("npm")
                .args(["install", "--global", package])
                .output()
                .map_err(|error| format!("npm CLI install failed: {error}"))?
        }
    };
    Ok(ActionExecution {
        success: output.status.success(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn verify_npm_integrity(package: &str, expected: &str) -> Result<(), String> {
    let output = Command::new("npm")
        .args(["view", package, "dist.integrity", "--json"])
        .output()
        .map_err(|error| format!("npm integrity lookup failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "npm integrity lookup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let actual = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_matches('"')
        .to_string();
    if actual != expected {
        return Err(format!(
            "npm integrity mismatch for {package}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn plan_hash(plan: &OnboardingPlan) -> Result<String, String> {
    let mut copy = plan.clone();
    copy.plan_hash.clear();
    serde_json::to_vec(&copy)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("cannot hash onboarding plan: {error}"))
}

fn kernel_item(ctx: &AssessContext<'_>) -> OnboardingItem {
    let ready = ctx.ags_executable.is_file();
    OnboardingItem {
        id: "ags-kernel".to_string(),
        category: "kernel".to_string(),
        required: true,
        state: if ready {
            ComponentState::ActiveReady
        } else {
            ComponentState::Absent
        },
        reason: if ready {
            format!(
                "AGS executable is available at {}",
                ctx.ags_executable.display()
            )
        } else {
            "AGS executable is unavailable; use the signed npm launcher bootstrap".to_string()
        },
        source: Some("@agent-governance-suite/mcp".to_string()),
        license: Some("MIT".to_string()),
        integrity: None,
        routing: None,
        hook: None,
        action: None,
    }
}

fn project_item(ctx: &AssessContext<'_>) -> OnboardingItem {
    let marker = ctx.target.join("config/agent-project-profile.yaml");
    let ready = marker.is_file() && ctx.target.join("AGENT_SUITE_PROTOCOL.md").is_file();
    OnboardingItem {
        id: "project-init".to_string(),
        category: "project".to_string(),
        required: true,
        state: if ready {
            ComponentState::ActiveReady
        } else {
            ComponentState::Absent
        },
        reason: if ready {
            "project AGS markers are present".to_string()
        } else {
            "project has not been initialized into AGS governance".to_string()
        },
        source: None,
        license: None,
        integrity: None,
        routing: None,
        hook: None,
        action: (!ready).then(|| OnboardingAction::ProjectInit {
            target: normalized_path(ctx.target),
        }),
    }
}

fn host_item(ctx: &AssessContext<'_>) -> OnboardingItem {
    let supported = matches!(ctx.host, "claude-code" | "codex" | "omp");
    if !supported {
        return OnboardingItem {
            id: format!("host:{}", ctx.host),
            category: "host".to_string(),
            required: true,
            state: ComponentState::UnsupportedHost,
            reason: "host has no stable official registrar; use manual configuration".to_string(),
            source: None,
            license: None,
            integrity: None,
            routing: None,
            hook: None,
            action: None,
        };
    }
    let registrar = if ctx.host == "claude-code" {
        "claude"
    } else {
        "codex"
    };
    let host_present = if ctx.host == "omp" {
        command_in_path("omp").is_some() || ctx.home.join(".omp").exists()
    } else {
        command_in_path(registrar).is_some() || ctx.home.join(format!(".{}", registrar)).exists()
    };
    let state = if ctx.mcp_connected || ctx.host_registered == Some(true) {
        ComponentState::ActiveReady
    } else if host_present {
        ComponentState::InstalledNotVisible
    } else {
        ComponentState::Absent
    };
    let action = if ctx.host == "omp"
        || !host_present
        || ctx.mcp_connected
        || ctx.host_registered == Some(true)
    {
        None
    } else {
        Some(OnboardingAction::RegisterAgsMcp {
            registrar: registrar.to_string(),
            executable: normalized_path(ctx.ags_executable),
        })
    };
    OnboardingItem {
        id: format!("host:{}", ctx.host),
        category: "host".to_string(),
        required: true,
        state,
        reason: match state {
            ComponentState::ActiveReady if ctx.mcp_connected => {
                "live AGS MCP connection proves host visibility".into()
            }
            ComponentState::ActiveReady => {
                "official host registrar probe confirms the AGS MCP entry".into()
            }
            ComponentState::InstalledNotVisible => {
                "host is installed but AGS MCP registration is not proven".into()
            }
            ComponentState::Absent => "host executable/config was not detected".into(),
            _ => "host readiness could not be established".into(),
        },
        source: None,
        license: None,
        integrity: None,
        routing: None,
        hook: None,
        action,
    }
}

fn capability_item(ctx: &AssessContext<'_>, capability: &ThirdPartyCapability) -> OnboardingItem {
    match capability.kind {
        CapabilityKind::Skill => skill_item(ctx, capability),
        CapabilityKind::Cli => cli_item(capability),
        CapabilityKind::Mcp => mcp_item(ctx, capability),
        CapabilityKind::Hook => hook_item(ctx, capability),
    }
}

fn cli_item(capability: &ThirdPartyCapability) -> OnboardingItem {
    let command = capability.install.command.as_deref();
    let present = command.and_then(command_in_path).is_some();
    let package = pinned_npm_package(capability);
    let action = if !present && capability.install.strategy == "npm-global" {
        package
            .clone()
            .zip(capability.source.integrity.clone())
            .map(|(package, integrity)| OnboardingAction::InstallNpmCli { package, integrity })
    } else {
        None
    };
    OnboardingItem {
        id: format!("cli:{}", capability.id),
        category: "cli".into(),
        required: capability.required,
        state: if present {
            ComponentState::ActiveReady
        } else {
            ComponentState::Absent
        },
        reason: if present {
            format!(
                "{} is available; version drift is checked by `ags skill update`",
                command.unwrap_or_default()
            )
        } else if action.is_some() {
            "pinned CLI is available for one explicitly confirmed npm-global action".into()
        } else {
            "CLI is not visible; its external manager remains authoritative".into()
        },
        source: capability.source.repository.clone(),
        license: capability.source.license.clone(),
        integrity: capability.source.integrity.clone(),
        routing: routing_readiness(capability),
        hook: None,
        action,
    }
}

fn mcp_item(ctx: &AssessContext<'_>, capability: &ThirdPartyCapability) -> OnboardingItem {
    let Some(mcp) = capability.mcp.as_ref() else {
        return blocked_capability_item(capability, "MCP contract is missing");
    };
    let registered = ctx
        .registered_mcp_ids
        .iter()
        .any(|server| server == &mcp.server_name);
    let dependency_ready =
        capability.install.depends_on.iter().all(|dependency| {
            dependency != "codegraph-cli" || command_in_path("codegraph").is_some()
        });
    let integrity_ready = capability
        .source
        .integrity
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        && capability
            .source
            .repository
            .as_deref()
            .is_some_and(|value| value.starts_with("https://"))
        && capability
            .source
            .license
            .as_deref()
            .is_some_and(|value| !value.is_empty());
    let registrar = match ctx.host {
        "claude-code" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    };
    let action = if registered || !dependency_ready || !integrity_ready {
        None
    } else if mcp.command == "npx" {
        registrar
            .zip(pinned_npm_package(capability))
            .zip(capability.source.integrity.clone())
            .map(
                |((registrar, package), integrity)| OnboardingAction::RegisterNpmMcp {
                    registrar: registrar.to_string(),
                    server_name: mcp.server_name.clone(),
                    package,
                    integrity,
                },
            )
    } else {
        registrar.map(|registrar| OnboardingAction::RegisterCommandMcp {
            registrar: registrar.to_string(),
            server_name: mcp.server_name.clone(),
            command: mcp.command.clone(),
            args: mcp.args.clone(),
        })
    };
    OnboardingItem {
        id: format!("mcp:{}", capability.id),
        category: "mcp".to_string(),
        required: capability.required,
        state: if !integrity_ready {
            ComponentState::BlockedMissingIntegrity
        } else if registered && dependency_ready {
            ComponentState::ActiveReady
        } else if !dependency_ready {
            ComponentState::VisibleNotReady
        } else {
            ComponentState::InstalledNotVisible
        },
        reason: if !integrity_ready {
            "registry source/version/license/integrity is incomplete; installation is blocked"
                .to_string()
        } else if registered && dependency_ready {
            "official host registrar probe confirms the MCP entry and its dependency is ready"
                .into()
        } else if !dependency_ready {
            "MCP registration waits for its pinned CLI dependency".into()
        } else {
            "MCP is not visible to the active host; an exact registrar action is available".into()
        },
        source: capability.source.repository.clone(),
        license: capability.source.license.clone(),
        integrity: capability.source.integrity.clone(),
        routing: routing_readiness(capability),
        hook: None,
        action,
    }
}

fn skill_item(ctx: &AssessContext<'_>, skill: &ThirdPartyCapability) -> OnboardingItem {
    let active = ctx
        .active_skill_ids
        .iter()
        .any(|skill_id| skill_id == &skill.id);
    let visible = host_skill_body_paths(ctx.home, ctx.host, &skill.id)
        .iter()
        .any(|body| body.join("SKILL.md").is_file());
    let trusted_source = skill.source.manager == "git"
        && skill
            .source
            .repository
            .as_deref()
            .is_some_and(|source| source.starts_with("https://github.com/"));
    let revision = skill
        .source
        .revision
        .as_deref()
        .filter(|value| is_git_revision(value));
    let integrity = revision.map(|value| format!("git-commit:{value}"));
    let license_ready = skill
        .source
        .license
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    let (state, reason, action) = if active {
        (
            ComponentState::ActiveReady,
            "current host capability snapshot confirms the skill is active and routable"
                .to_string(),
            None,
        )
    } else if visible {
        (
            ComponentState::VisibleNotReady,
            "a host-visible skill body exists but the current capability snapshot does not admit it"
                .to_string(),
            None,
        )
    } else if !trusted_source {
        (
            ComponentState::BlockedUntrustedSource,
            "no reviewed upstream source is recorded".to_string(),
            None,
        )
    } else if revision.is_none() || !license_ready {
        (
            ComponentState::BlockedMissingIntegrity,
            "source revision or license metadata is missing".to_string(),
            None,
        )
    } else {
        let source = pin_github_source(
            skill.source.repository.as_deref().unwrap_or_default(),
            revision.unwrap_or_default(),
            skill.source.subdir.as_deref(),
        );
        (
            ComponentState::Absent,
            "reviewed skill is available for explicit per-item adoption".to_string(),
            Some(OnboardingAction::AdoptSkill {
                source,
                host: ctx.host.to_string(),
            }),
        )
    };
    OnboardingItem {
        id: format!("skill:{}", skill.id),
        category: "skill".to_string(),
        required: skill.required,
        state,
        reason,
        source: skill.source.repository.clone(),
        license: skill.source.license.clone(),
        integrity,
        routing: routing_readiness(skill),
        hook: None,
        action,
    }
}

fn host_skill_body_paths(home: &Path, host: &str, skill_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let native = match host {
        "claude-code" => Some(".claude/skills"),
        "codex" => Some(".codex/skills"),
        "cursor" => Some(".cursor/skills"),
        "omp" => Some(".omp/agent/skills"),
        "codebuddy-code" => Some(".codebuddy/skills"),
        _ => None,
    };
    if let Some(native) = native {
        roots.push(home.join(native).join(skill_id));
    }
    if matches!(host, "codex" | "cursor" | "omp") {
        roots.push(home.join(".agents/skills").join(skill_id));
    }
    roots
}

fn hook_item(ctx: &AssessContext<'_>, capability: &ThirdPartyCapability) -> OnboardingItem {
    let Some(contract) = capability.hook.as_ref() else {
        return blocked_capability_item(capability, "hook contract is missing");
    };
    let config_present = hook_config_present(&contract.config_glob, ctx.home);
    OnboardingItem {
        id: format!("hook:{}", capability.id),
        category: "hook".into(),
        required: capability.required,
        state: if config_present {
            ComponentState::ActiveReady
        } else {
            ComponentState::Absent
        },
        reason: if config_present {
            "hook configuration exists; event semantics remain owned by the host/plugin manager"
                .into()
        } else {
            "hook configuration was not found; AGS will not install protected host wiring".into()
        },
        source: capability.source.repository.clone(),
        license: capability.source.license.clone(),
        integrity: capability.source.integrity.clone(),
        routing: None,
        hook: Some(HookReadiness {
            host: contract.host.clone(),
            events: contract.events.clone(),
            config_present,
            health_probe: contract.health_probe.clone(),
            event_probe: if config_present {
                "CONFIG_PRESENT_RUNTIME_NOT_RUN".into()
            } else {
                "NOT_RUN".into()
            },
        }),
        action: None,
    }
}

fn blocked_capability_item(capability: &ThirdPartyCapability, reason: &str) -> OnboardingItem {
    OnboardingItem {
        id: format!("{}:{}", capability.kind.as_str(), capability.id),
        category: capability.kind.as_str().into(),
        required: capability.required,
        state: ComponentState::BlockedMissingIntegrity,
        reason: reason.into(),
        source: capability.source.repository.clone(),
        license: capability.source.license.clone(),
        integrity: capability.source.integrity.clone(),
        routing: routing_readiness(capability),
        hook: None,
        action: None,
    }
}

fn routing_readiness(capability: &ThirdPartyCapability) -> Option<RoutingReadiness> {
    (capability.kind != CapabilityKind::Hook).then(|| RoutingReadiness {
        route_state: capability.routing.route_state.clone(),
        metadata_complete: capability.routing.route_state != "routable"
            || (capability.routing.invoke_hint.is_some()
                && !capability.routing.intent_tags.is_empty()
                && !capability.routing.positive_examples.is_empty()
                && !capability.routing.negative_examples.is_empty()),
        semantic_probe: "NOT_RUN".into(),
        boundary:
            "Natural-language interpretation belongs to the host; AGS validates only the exact target contract."
                .into(),
    })
}

fn pinned_npm_package(capability: &ThirdPartyCapability) -> Option<String> {
    capability.source.package.as_ref().and_then(|package| {
        capability
            .source
            .version
            .as_ref()
            .map(|version| format!("{package}@{version}"))
    })
}

fn hook_config_present(pattern: &str, home: &Path) -> bool {
    let expanded = pattern
        .strip_prefix("$HOME/")
        .map(|rest| home.join(rest))
        .unwrap_or_else(|| PathBuf::from(pattern));
    let Some(path) = expanded.to_str() else {
        return false;
    };
    let Some((prefix, suffix)) = path.split_once('*') else {
        return expanded.is_file();
    };
    let root = Path::new(prefix);
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let candidate = entry.path().join(suffix.trim_start_matches('/'));
        candidate.is_file()
    })
}

fn pin_github_source(source: &str, revision: &str, subdir: Option<&str>) -> String {
    let mut pinned = format!("{}/tree/{revision}", source.trim_end_matches('/'));
    if let Some(subdir) = subdir.filter(|value| !value.is_empty()) {
        pinned.push('/');
        pinned.push_str(subdir.trim_start_matches('/'));
    }
    pinned
}

fn is_git_revision(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn normalized_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn command_in_path(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ags-onboarding-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::write(
            root.join("manifests/onboarding-public.yaml"),
            "profile: public\nexcluded_capabilities: []\nrequired_mcps: []\noptional_mcps: []\ndeveloper_tools: []\n",
        )
        .unwrap();
        std::fs::write(
            root.join("manifests/skill-recommendations.yaml"),
            "skills: []\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn uninitialized_project_gets_closed_init_action() {
        let root = fixture_root("init");
        let home = root.join("home");
        let target = root.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let exe = root.join("ags");
        std::fs::write(&exe, "").unwrap();
        let plan = assess_public(&AssessContext {
            source_root: &root,
            home: &home,
            target: &target,
            host: "codex",
            ags_executable: &exe,
            mcp_connected: true,
            host_registered: Some(true),
            registered_mcp_ids: &[],
            active_skill_ids: &[],
        })
        .unwrap();
        assert!(plan.bootstrap_required);
        assert!(matches!(
            find_action(&plan, "project-init").unwrap(),
            OnboardingAction::ProjectInit { .. }
        ));
        assert!(plan.excluded_capabilities.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pins_github_tree_to_reviewed_commit() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            pin_github_source(
                "https://github.com/acme/skills",
                revision,
                Some("skills/review")
            ),
            format!("https://github.com/acme/skills/tree/{revision}/skills/review")
        );
    }

    #[test]
    fn onboarding_rollback_is_manual_and_action_specific() {
        let registration = rollback_advice(&OnboardingAction::RegisterAgsMcp {
            registrar: "claude".to_string(),
            executable: "/tmp/ags".to_string(),
        });
        assert_eq!(
            registration[0].inverse_command.as_deref(),
            Some("claude mcp remove -s user ags")
        );

        let npm = rollback_advice(&OnboardingAction::InstallNpmCli {
            package: "@scope/tool@1.2.3".to_string(),
            integrity: "sha512-test".to_string(),
        });
        assert_eq!(
            npm[0].inverse_command.as_deref(),
            Some("npm uninstall --global @scope/tool")
        );
    }

    #[test]
    fn active_snapshot_prevents_duplicate_skill_adoption() {
        let root = fixture_root("active-skill");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let skill = ThirdPartyCapability {
            id: "review".to_string(),
            kind: CapabilityKind::Skill,
            name: "Review".to_string(),
            profiles: vec!["public".to_string()],
            required: false,
            tier: "flow".to_string(),
            purpose: "review changes".to_string(),
            risk: "low".to_string(),
            requires_auth: false,
            source: manifest::CapabilitySource {
                manager: "git".to_string(),
                revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                repository: Some("https://github.com/acme/skills".to_string()),
                license: Some("MIT".to_string()),
                ..Default::default()
            },
            install: manifest::InstallContract {
                strategy: "ags-skill-adopt".to_string(),
                ..Default::default()
            },
            routing: manifest::RoutingContract {
                route_state: "routable".to_string(),
                invoke_hint: Some("[skill: review]".to_string()),
                ..Default::default()
            },
            mcp: None,
            hook: None,
        };
        let active = vec!["review".to_string()];
        let exe = root.join("ags");
        let target = root.join("project");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(&exe, "").unwrap();
        let context = AssessContext {
            source_root: &root,
            home: &home,
            target: &target,
            host: "codex",
            ags_executable: &exe,
            mcp_connected: true,
            host_registered: Some(true),
            registered_mcp_ids: &[],
            active_skill_ids: &active,
        };
        let item = skill_item(&context, &skill);
        assert_eq!(item.state, ComponentState::ActiveReady);
        assert!(item.action.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn visible_but_unadmitted_skill_is_not_reported_ready_or_reinstalled() {
        let root = fixture_root("visible-skill");
        let home = root.join("home");
        let body = home.join(".codex/skills/review");
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(body.join("SKILL.md"), "# review").unwrap();
        let skill = ThirdPartyCapability {
            id: "review".to_string(),
            kind: CapabilityKind::Skill,
            name: "Review".to_string(),
            profiles: vec!["public".to_string()],
            required: false,
            tier: "flow".to_string(),
            purpose: "review changes".to_string(),
            risk: "low".to_string(),
            requires_auth: false,
            source: Default::default(),
            install: Default::default(),
            routing: Default::default(),
            mcp: None,
            hook: None,
        };
        let exe = root.join("ags");
        let target = root.join("project");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(&exe, "").unwrap();
        let context = AssessContext {
            source_root: &root,
            home: &home,
            target: &target,
            host: "codex",
            ags_executable: &exe,
            mcp_connected: true,
            host_registered: Some(true),
            registered_mcp_ids: &[],
            active_skill_ids: &[],
        };
        let item = skill_item(&context, &skill);
        assert_eq!(item.state, ComponentState::VisibleNotReady);
        assert!(item.action.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
