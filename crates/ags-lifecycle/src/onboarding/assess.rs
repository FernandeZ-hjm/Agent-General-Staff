use super::util::*;
use super::*;

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
        bootstrap_required,
        ready,
        plan_hash: String::new(),
        items,
        excluded_capabilities: profile.excluded_capabilities,
    };
    plan.plan_hash = plan_hash(&plan)?;
    Ok(plan)
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
        license: Some("GPL-3.0-only".to_string()),
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
    let Some(spec) = ags_host_integration::platform_spec(ctx.host)
        .filter(|spec| spec.native_memory_adapter.is_some())
    else {
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
    };
    let host_present = spec
        .cli_names
        .iter()
        .any(|command| command_in_path(command).is_some())
        || spec
            .config_subdirs
            .iter()
            .any(|subdir| ctx.home.join(subdir).exists());
    let state = if ctx.mcp_connected || ctx.host_registered == Some(true) {
        ComponentState::ActiveReady
    } else if host_present {
        ComponentState::InstalledNotVisible
    } else {
        ComponentState::Absent
    };
    let action = if !host_present || ctx.mcp_connected || ctx.host_registered == Some(true) {
        None
    } else {
        spec.mcp_registrar
            .map(|registrar| OnboardingAction::RegisterAgsMcp {
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
                "{} is available; version changes require an explicit upstream refresh",
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
    let registrar =
        ags_host_integration::platform_spec(ctx.host).and_then(|spec| spec.mcp_registrar);
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

pub(super) fn skill_item(ctx: &AssessContext<'_>, skill: &ThirdPartyCapability) -> OnboardingItem {
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
        (
            ComponentState::Absent,
            "reviewed skill is available through its external manager; install or update it explicitly, then refresh the static host snapshot once".to_string(),
            None,
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
    ags_host_integration::static_skill_roots(home, host)
        .into_iter()
        .map(|root| root.join(skill_id))
        .collect()
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
