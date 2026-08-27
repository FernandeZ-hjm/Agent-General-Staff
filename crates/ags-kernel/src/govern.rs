//! Effect-level governance (contract v3 §7.4 extension).
//!
//! AGS constrains what an agent can DO in a task, not which host tool it
//! calls: host adapters normalize native tool invocations into an
//! `ActionIntent { actor, task, effect, resource, capability }`, and the
//! guardrails section of ags.toml holds the workspace ceiling. The tool
//! permission matrix remains as a fallback for calls that carry no task
//! context; the stricter of the two decisions wins (fail closed).

use serde::Serialize;

use crate::config::Config;
use crate::matrix::Decision;

/// The standard effect vocabulary. Host tools are normalized onto these;
/// AGS never guesses semantics — each host adapter declares its mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    WorkspaceRead,
    WorkspaceWrite,
    WorkspaceDelete,
    ProcessExecute,
    VcsLocal,
    VcsPublish,
    NetworkRead,
    NetworkMutate,
    CredentialUse,
    CapabilityInstall,
    DelegationIssue,
    ReleasePublish,
}

pub const ALL_EFFECTS: [&str; 12] = [
    "workspace.read",
    "workspace.write",
    "workspace.delete",
    "process.execute",
    "vcs.local",
    "vcs.publish",
    "network.read",
    "network.mutate",
    "credential.use",
    "capability.install",
    "delegation.issue",
    "release.publish",
];

impl Effect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Effect::WorkspaceRead => "workspace.read",
            Effect::WorkspaceWrite => "workspace.write",
            Effect::WorkspaceDelete => "workspace.delete",
            Effect::ProcessExecute => "process.execute",
            Effect::VcsLocal => "vcs.local",
            Effect::VcsPublish => "vcs.publish",
            Effect::NetworkRead => "network.read",
            Effect::NetworkMutate => "network.mutate",
            Effect::CredentialUse => "credential.use",
            Effect::CapabilityInstall => "capability.install",
            Effect::DelegationIssue => "delegation.issue",
            Effect::ReleasePublish => "release.publish",
        }
    }
}

impl serde::Serialize for Effect {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// One normalized host tool invocation. `actor` and `task` are supplied by
/// the host adapter when available; a call without task context is governed
/// by guardrails plus the tool matrix, never by task authority.
#[derive(Debug, Clone, Serialize)]
pub struct ActionIntent {
    pub actor: Option<String>,
    pub task: Option<String>,
    pub effect: Effect,
    pub resource: Option<String>,
    pub capability: Option<String>,
    pub externality: Option<String>,
}

/// Workspace ceiling evaluation. Task-level authority (TaskAuthority /
/// DelegationGrant) intersects with this; the guardrails alone never grant
/// more than `default_decision`.
pub fn evaluate_guardrails(config: &Config, intent: &ActionIntent) -> Decision {
    use Effect::*;
    match intent.effect {
        // Reading inside the workspace has no externality; it is the safe
        // baseline and never needs an ask.
        WorkspaceRead => Decision::Allow,
        CredentialUse => decision_from(&config.guardrails.credential_use),
        ReleasePublish | CapabilityInstall | DelegationIssue => Decision::Sealed,
        WorkspaceWrite | WorkspaceDelete => {
            if let Some(resource) = &intent.resource {
                let rel = resource.trim_start_matches("./");
                if config
                    .guardrails
                    .protected_resources
                    .iter()
                    .any(|p| rel == p || rel.starts_with(&format!("{p}/")))
                {
                    return Decision::Deny;
                }
            }
            decision_from(&config.guardrails.default_decision)
        }
        ProcessExecute => {
            if let Some(resource) = &intent.resource {
                if config
                    .guardrails
                    .process_deny
                    .iter()
                    .any(|p| resource.contains(p))
                {
                    return Decision::Deny;
                }
            }
            decision_from(&config.guardrails.default_decision)
        }
        // Local VCS writes and network effects have externality; the matrix
        // fallback still applies its exact git:* / mcp:* rules on top.
        _ => decision_from(&config.guardrails.default_decision),
    }
}

/// Combine guardrails with the tool-matrix fallback: the stricter decision
/// wins, so an unknown effect or a denied guardrail can never be overruled
/// by a permissive tool pattern.
pub fn stricter(a: Decision, b: Decision) -> Decision {
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

fn rank(decision: Decision) -> u8 {
    match decision {
        Decision::Allow => 1,
        Decision::Ask => 2,
        Decision::Sealed => 3,
        Decision::Deny => 4,
    }
}

/// Parse a guardrail policy value; anything unknown fails closed to `ask`.
pub fn decision_from(value: &str) -> Decision {
    match value.trim().to_ascii_lowercase().as_str() {
        "allow" => Decision::Allow,
        "ask" => Decision::Ask,
        "deny" => Decision::Deny,
        "sealed" => Decision::Sealed,
        _ => Decision::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::scaffold;
    use serde_json::json;

    fn intent(effect: Effect, resource: Option<&str>) -> ActionIntent {
        ActionIntent {
            actor: None,
            task: None,
            effect,
            resource: resource.map(str::to_string),
            capability: None,
            externality: None,
        }
    }

    #[test]
    fn protected_resources_are_denied_even_when_default_allows() {
        let mut config = scaffold("t");
        config.guardrails.default_decision = "allow".to_string();
        let i = intent(Effect::WorkspaceWrite, Some(".ags/x"));
        assert_eq!(evaluate_guardrails(&config, &i), Decision::Deny);
        let i = intent(Effect::WorkspaceWrite, Some("protocol/design.md"));
        assert_eq!(evaluate_guardrails(&config, &i), Decision::Deny);
        let i = intent(Effect::WorkspaceWrite, Some("src/main.rs"));
        assert_eq!(evaluate_guardrails(&config, &i), Decision::Allow);
    }

    #[test]
    fn credential_and_release_effects_are_never_allow_by_guardrail() {
        let mut config = scaffold("t");
        config.guardrails.credential_use = "ask".to_string();
        assert_eq!(
            evaluate_guardrails(&config, &intent(Effect::CredentialUse, None)),
            Decision::Ask
        );
        assert_eq!(
            evaluate_guardrails(&config, &intent(Effect::ReleasePublish, None)),
            Decision::Sealed
        );
    }

    #[test]
    fn process_deny_list_blocks_matching_commands() {
        let mut config = scaffold("t");
        config.guardrails.process_deny = vec!["cargo publish".to_string()];
        let i = intent(Effect::ProcessExecute, Some("cargo publish --dry-run"));
        assert_eq!(evaluate_guardrails(&config, &i), Decision::Deny);
        let i = intent(Effect::ProcessExecute, Some("cargo test"));
        assert_eq!(evaluate_guardrails(&config, &i), Decision::Ask);
    }

    #[test]
    fn unknown_policy_value_fails_closed_to_ask() {
        assert_eq!(decision_from("banana"), Decision::Ask);
        assert_eq!(decision_from("ask"), Decision::Ask);
        assert_eq!(decision_from("deny"), Decision::Deny);
    }

    #[test]
    fn stricter_combines_guardrail_and_matrix() {
        assert_eq!(stricter(Decision::Allow, Decision::Ask), Decision::Ask);
        assert_eq!(stricter(Decision::Ask, Decision::Deny), Decision::Deny);
        assert_eq!(
            stricter(Decision::Sealed, Decision::Allow),
            Decision::Sealed
        );
    }

    #[test]
    fn effect_vocabulary_is_stable() {
        assert_eq!(ALL_EFFECTS.len(), 12);
        assert_eq!(Effect::WorkspaceWrite.as_str(), "workspace.write");
        assert_eq!(json!(Effect::DelegationIssue), json!("delegation.issue"));
    }
}
