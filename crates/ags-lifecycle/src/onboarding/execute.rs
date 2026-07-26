use super::util::*;
use super::*;

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
