use super::util::*;
use super::*;

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
/// No shell is used. Skill recommendations never enter this mutation path;
/// source changes belong to an explicit reviewed install/update workflow.
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
