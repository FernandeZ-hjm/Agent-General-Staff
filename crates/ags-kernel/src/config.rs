//! `ags.toml` — the single per-workspace policy file (contract v3 §7.1).
//!
//! One file owns the permission matrix, write boundaries, sealed-operation
//! list, verification commands, review escalation table, host registrations
//! and capability sources. `ags init` generates the scaffold, `ags check`
//! lints it, `ags doctor` runs health probes against it.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Operations that must remain sealed for a role-A workspace. `ags check`
/// lints a role-A config that drops one of them (D1 / §7.8).
pub const CANONICAL_SEALED_OPS: &[&str] = &[
    "govern.skill.install",
    "govern.skill.remove",
    "govern.host.register",
    "govern.host_projection",
    "govern.delegation.issue",
    "upgrade",
    "update",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub workspace: WorkspaceSection,
    #[serde(default)]
    pub boundaries: BoundariesSection,
    #[serde(default)]
    pub permissions: PermissionsSection,
    #[serde(default)]
    pub sealed: SealedSection,
    #[serde(default)]
    pub verify: VerifySection,
    #[serde(default)]
    pub review: ReviewSection,
    #[serde(default)]
    pub hosts: Vec<HostEntry>,
    #[serde(default)]
    pub capabilities: CapabilitiesSection,
    #[serde(default)]
    pub guardrails: GuardrailsSection,
}

fn default_ask() -> String {
    "ask".to_string()
}

fn default_deny() -> String {
    "deny".to_string()
}

fn default_sealed() -> String {
    "sealed".to_string()
}

fn default_depth_two() -> u32 {
    2
}

/// The workspace ceiling that no task authority can exceed. Guardrails are
/// not a tool whitelist; they bound effects (write targets, processes,
/// credentials, delegation depth, publishing).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailsSection {
    /// Paths (relative to the workspace root) that are never writable or
    /// deletable, regardless of task authority.
    #[serde(default)]
    pub protected_resources: Vec<String>,
    /// Decision for effects without a dedicated policy: ask by default.
    #[serde(default = "default_ask")]
    pub default_decision: String,
    /// credential.use policy: deny by default.
    #[serde(default = "default_deny")]
    pub credential_use: String,
    /// release.publish policy: always sealed.
    #[serde(default = "default_sealed")]
    pub release_publish: String,
    /// Hard cap on delegation depth (0 disables delegation).
    #[serde(default = "default_depth_two")]
    pub max_delegation_depth: u32,
    /// Substrings of process.execute commands that are always denied.
    #[serde(default)]
    pub process_deny: Vec<String>,
}

impl Default for GuardrailsSection {
    fn default() -> Self {
        GuardrailsSection {
            protected_resources: vec![],
            default_decision: default_ask(),
            credential_use: default_deny(),
            release_publish: default_sealed(),
            max_delegation_depth: default_depth_two(),
            process_deny: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSection {
    pub slug: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BoundariesSection {
    /// Paths (relative to the workspace root) that may be written. Anything
    /// outside escalates the decision (never below `ask`).
    #[serde(default)]
    pub allowed_write_paths: Vec<String>,
    /// Paths that are never writable regardless of the matrix.
    #[serde(default)]
    pub deny_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PermissionsSection {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SealedSection {
    #[serde(default)]
    pub ops: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VerifySection {
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default = "default_profile")]
    pub profile: String,
}

fn default_profile() -> String {
    "smoke".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ReviewSection {
    #[serde(default)]
    pub escalate_to_medium: Vec<String>,
    #[serde(default)]
    pub escalate_to_heavy: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEntry {
    pub id: String,
    pub surface: String,
    /// True when this host can spawn child agents for delegation.
    #[serde(default)]
    pub dispatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesSection {
    /// Directories (relative to the workspace root) scanned by `ags update`.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// A machine-readable lint finding for `ags check`.
#[derive(Debug, Clone, Serialize)]
pub struct LintFinding {
    pub code: &'static str,
    pub message: String,
}

impl Config {
    pub fn load(root: &Path) -> Result<Config> {
        let path = root.join(crate::workspace::AGS_TOML);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| crate::error::io("ags_toml_read_failed", &e))?;
        let config: Config = toml::from_str(&text).map_err(|e| {
            Error::new(
                "ags_toml_parse_failed",
                format!("{}: {}", path.display(), e),
            )
        })?;
        Ok(config)
    }

    /// Structural lint. Returns every finding; `ags check` fails on any.
    pub fn lint(&self) -> Vec<LintFinding> {
        let mut findings = Vec::new();
        if !valid_slug(&self.workspace.slug) {
            findings.push(LintFinding {
                code: "workspace_slug_invalid",
                message: "workspace.slug must use ASCII letters, numbers, dot, underscore or dash"
                    .to_string(),
            });
        }
        for host in &self.hosts {
            if !matches!(host.surface.as_str(), "cli" | "mcp") {
                findings.push(LintFinding {
                    code: "host_surface_invalid",
                    message: format!(
                        "host {} surface must be cli or mcp (got `{}`)",
                        host.id, host.surface
                    ),
                });
            }
        }
        let mut seen: Vec<(&str, String)> = Vec::new();

        for (list_name, list) in [
            ("allow", self.permissions.allow.as_slice()),
            ("ask", self.permissions.ask.as_slice()),
            ("deny", self.permissions.deny.as_slice()),
        ] {
            for entry in list {
                if crate::matrix::parse_pattern(entry).is_none() {
                    findings.push(LintFinding {
                        code: "matrix_pattern_invalid",
                        message: format!(
                            "permissions.{list_name}: invalid surface:action pattern `{entry}`"
                        ),
                    });
                    continue;
                }
                let lower = entry.trim().to_ascii_lowercase();
                if seen
                    .iter()
                    .any(|(n, e)| *n != list_name && e.eq_ignore_ascii_case(&lower))
                {
                    findings.push(LintFinding {
                        code: "matrix_pattern_conflict",
                        message: format!("permissions: pattern `{entry}` appears in more than one of allow/ask/deny"),
                    });
                }
                seen.push((list_name, lower));
            }
        }

        if !matches!(self.workspace.role.as_str(), "A" | "S" | "B") {
            findings.push(LintFinding {
                code: "workspace_role_invalid",
                message: format!(
                    "workspace.role must be A, S or B (got `{}`)",
                    self.workspace.role
                ),
            });
        }

        if self.workspace.role == "A" {
            for op in CANONICAL_SEALED_OPS {
                if !self.sealed.ops.iter().any(|e| e.eq_ignore_ascii_case(op)) {
                    findings.push(LintFinding {
                        code: "sealed_coverage_incomplete",
                        message: format!("role A must keep `{op}` in [sealed].ops"),
                    });
                }
            }
        }

        for entry in &self.boundaries.allowed_write_paths {
            if entry.starts_with('/') || entry.starts_with("..") {
                findings.push(LintFinding {
                    code: "boundary_path_absolute",
                    message: format!(
                        "boundaries.allowed_write_paths: `{entry}` must be a relative in-root path"
                    ),
                });
            }
        }
        for entry in &self.boundaries.deny_paths {
            if entry.starts_with('/') || entry.starts_with("..") {
                findings.push(LintFinding {
                    code: "boundary_path_absolute",
                    message: format!(
                        "boundaries.deny_paths: `{entry}` must be a relative in-root path"
                    ),
                });
            }
        }

        if !matches!(self.verify.profile.as_str(), "smoke" | "standard" | "full") {
            findings.push(LintFinding {
                code: "verify_profile_invalid",
                message: format!(
                    "verify.profile must be smoke/standard/full (got `{}`)",
                    self.verify.profile
                ),
            });
        }

        for host in &self.hosts {
            if crate::hosts::normalize_host_id(&host.id).is_err() {
                findings.push(LintFinding {
                    code: "host_id_invalid",
                    message: format!(
                        "hosts: `{}` does not normalize to a canonical lowercase-dash id",
                        host.id
                    ),
                });
            }
            if !matches!(host.surface.as_str(), "cli" | "mcp" | "hybrid") {
                findings.push(LintFinding {
                    code: "host_surface_invalid",
                    message: format!(
                        "hosts.{}: surface must be cli/mcp/hybrid (got `{}`)",
                        host.id, host.surface
                    ),
                });
            }
        }

        for (name, value) in [
            ("default_decision", &self.guardrails.default_decision),
            ("credential_use", &self.guardrails.credential_use),
            ("release_publish", &self.guardrails.release_publish),
        ] {
            if !matches!(value.as_str(), "allow" | "ask" | "deny" | "sealed") {
                findings.push(LintFinding {
                    code: "guardrail_policy_invalid",
                    message: format!("guardrails.{name}: `{value}` is not allow/ask/deny/sealed"),
                });
            }
        }

        findings
    }
}

pub fn read_identity(root: &Path) -> Result<(String, String)> {
    let config = Config::load(root)?;
    if config.workspace.slug.trim().is_empty() {
        return Err(Error::new(
            "workspace_slug_empty",
            "workspace.slug must not be empty",
        ));
    }
    Ok((config.workspace.slug.clone(), config.workspace.role.clone()))
}

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Scaffold generated by `ags init` for a role-A workspace.
pub fn scaffold(slug: &str) -> Config {
    Config {
        workspace: WorkspaceSection {
            slug: slug.to_string(),
            role: "A".to_string(),
        },
        boundaries: BoundariesSection {
            allowed_write_paths: vec![".".to_string()],
            deny_paths: vec![
                ".git".to_string(),
                ".ags".to_string(),
                "protocol".to_string(),
                "manifests".to_string(),
            ],
        },
        permissions: PermissionsSection {
            allow: vec![
                "read:file".to_string(),
                "edit:file".to_string(),
                "write:file-new".to_string(),
                "bash:readonly".to_string(),
                "git:status".to_string(),
                "git:diff".to_string(),
                "git:log".to_string(),
                "mcp:readonly".to_string(),
            ],
            ask: vec![
                "bash:mutate".to_string(),
                "git:add".to_string(),
                "git:commit".to_string(),
                "git:branch".to_string(),
                "git:checkout".to_string(),
                "git:merge".to_string(),
                "git:stash".to_string(),
                "git:restore".to_string(),
                "mcp:network".to_string(),
                "mcp:mutate".to_string(),
            ],
            deny: vec![
                "write:deny_paths".to_string(),
                "git:push".to_string(),
                "git:tag".to_string(),
                "remote:*".to_string(),
                "credential:*".to_string(),
            ],
        },
        sealed: SealedSection {
            ops: CANONICAL_SEALED_OPS.iter().map(|s| s.to_string()).collect(),
        },
        verify: VerifySection {
            commands: vec![
                "cargo fmt --check".to_string(),
                "cargo clippy --all-targets --all-features -- -D warnings".to_string(),
                "RUSTFLAGS=\"-D warnings\" cargo test --workspace".to_string(),
                "cargo build --release".to_string(),
            ],
            profile: "smoke".to_string(),
        },
        review: ReviewSection {
            escalate_to_medium: vec![
                "ask-hit".to_string(),
                "fanout".to_string(),
                "boundary-crossing".to_string(),
            ],
            escalate_to_heavy: vec![
                "sealed".to_string(),
                "promotion".to_string(),
                "release".to_string(),
            ],
        },
        hosts: vec![],
        capabilities: CapabilitiesSection {
            sources: vec!["ags-skills".to_string(), "skill-packs".to_string()],
        },
        guardrails: GuardrailsSection {
            protected_resources: vec![
                ".git".to_string(),
                ".ags".to_string(),
                "protocol".to_string(),
                "manifests".to_string(),
            ],
            default_decision: default_ask(),
            credential_use: default_deny(),
            release_publish: default_sealed(),
            max_delegation_depth: default_depth_two(),
            process_deny: vec![],
        },
    }
}
