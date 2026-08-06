use super::{PreflightBinding, RoutingSession};
use ags_capability_governance::skill_adoption::{AdoptionContext, SnapshotDiscovery};
use ags_lifecycle::maintenance::{
    MaintenanceBackendRouter, MaintenanceIntent, MaintenanceService, ServiceClock, ServiceContext,
    SkillMaintenanceBackend, SuiteSkillMaintenanceBackend,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

const PLAN_TTL_SECONDS: u64 = 30 * 60;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanArguments {
    intent: MaintenanceIntent,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanHashArguments {
    plan_hash: String,
    #[serde(default)]
    acknowledgements: BTreeSet<String>,
}

pub(super) fn tool_maintenance_status(
    arguments: &serde_json::Value,
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<String, String> {
    let arguments: PlanHashArguments = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid maintenance status arguments: {error}"))?;
    if is_core_plan(&arguments.plan_hash) {
        return core_launcher(&["update", "status", "--plan-hash", &arguments.plan_hash]);
    }
    let service = service(binding, routing_session)?;
    serde_json::to_string_pretty(&service.status(&arguments.plan_hash)?)
        .map_err(|error| error.to_string())
}

pub(super) fn tool_maintenance_plan(
    arguments: &serde_json::Value,
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<String, String> {
    let arguments: PlanArguments = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid maintenance plan arguments: {error}"))?;
    if arguments.intent.subject == ags_lifecycle::maintenance::MaintenanceSubject::Ags {
        if arguments.intent.operation != ags_lifecycle::maintenance::MaintenanceOperation::Update
            || arguments.intent.target != "core"
        {
            return Err("AGS maintenance accepts operation=update target=core".to_string());
        }
        return core_launcher(&["update", "plan"]);
    }
    let service = service(binding, routing_session)?;
    serde_json::to_string_pretty(&service.plan(arguments.intent)?)
        .map_err(|error| error.to_string())
}

pub(super) fn tool_maintenance_apply(
    arguments: &serde_json::Value,
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<String, String> {
    let arguments: PlanHashArguments = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid maintenance apply arguments: {error}"))?;
    if is_core_plan(&arguments.plan_hash) {
        if !arguments.acknowledgements.is_empty() {
            return Err("core update plan has no risk acknowledgement ids".to_string());
        }
        return core_launcher(&["update", "apply", "--plan-hash", &arguments.plan_hash]);
    }
    let service = service(binding, routing_session)?;
    serde_json::to_string_pretty(&service.apply(&arguments.plan_hash, &arguments.acknowledgements)?)
        .map_err(|error| error.to_string())
}

pub(super) fn tool_maintenance_verify(
    arguments: &serde_json::Value,
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<String, String> {
    let arguments: PlanHashArguments = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid maintenance verify arguments: {error}"))?;
    if is_core_plan(&arguments.plan_hash) {
        return core_launcher(&["update", "verify", "--plan-hash", &arguments.plan_hash]);
    }
    let service = service(binding, routing_session)?;
    serde_json::to_string_pretty(&service.verify(&arguments.plan_hash)?)
        .map_err(|error| error.to_string())
}

pub(super) fn tool_maintenance_recover(
    arguments: &serde_json::Value,
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<String, String> {
    let arguments: PlanHashArguments = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid maintenance recover arguments: {error}"))?;
    if is_core_plan(&arguments.plan_hash) {
        return core_launcher(&["update", "recover", "--plan-hash", &arguments.plan_hash]);
    }
    let service = service(binding, routing_session)?;
    serde_json::to_string_pretty(&service.recover(&arguments.plan_hash)?)
        .map_err(|error| error.to_string())
}

fn core_plan_path(plan_hash: &str) -> PathBuf {
    let cache_root = std::env::var_os("AGS_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| ags_platform::home_dir_or_temp().join(".ags"));
    cache_root
        .join("launcher-state")
        .join("update-plans")
        .join(format!("{plan_hash}.json"))
}

fn is_core_plan(plan_hash: &str) -> bool {
    plan_hash.len() == 64
        && plan_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && core_plan_path(plan_hash).is_file()
}

fn core_launcher(args: &[&str]) -> Result<String, String> {
    let launcher = std::env::var_os("AGS_MAINTENANCE_LAUNCHER")
        .map(PathBuf::from)
        .or_else(|| ags_platform::find_in_path("ags-mcp"))
        .ok_or_else(|| {
            "MCP core update requires the @agent-governance-suite/mcp launcher".to_string()
        })?;
    let output = Command::new(&launcher)
        .args(args)
        .output()
        .map_err(|error| {
            format!(
                "cannot run signed MCP launcher {}: {error}",
                launcher.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "signed MCP launcher failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("signed MCP launcher returned invalid JSON: {error}"))?;
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

fn service(
    binding: &PreflightBinding,
    routing_session: &RoutingSession,
) -> Result<MaintenanceService<MaintenanceBackendRouter>, String> {
    let runtime_home = ags_platform::runtime_home();
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from);
    let authority_root = ags_capability_governance::resolve_capability_authority_root(
        &binding.target,
        &runtime_home,
        explicit,
    )
    .map_err(|error| error.to_string())?;
    let binding_id = routing_session.stable_id("maintenance", "binding");
    MaintenanceService::new(
        ServiceContext {
            runtime_home: runtime_home.clone(),
            binding_id,
            clock: ServiceClock::System,
            plan_ttl_seconds: PLAN_TTL_SECONDS,
        },
        MaintenanceBackendRouter {
            skill: SkillMaintenanceBackend {
                adoption: AdoptionContext {
                    authority_root: authority_root.clone(),
                    runtime_home: runtime_home.clone(),
                    host_home: binding.host_home.clone(),
                    snapshot_discovery: SnapshotDiscovery::Live,
                },
                preflight_target: binding.target.clone(),
            },
            suite_skills: SuiteSkillMaintenanceBackend {
                source_root: authority_root,
                policy: ags_lifecycle::suite_skill_projection::SuiteSkillProjectionPolicy {
                    required_authority_root: None,
                    target_hosts: ags_lifecycle::setup::approved_lifecycle_hosts(&runtime_home)
                        .unwrap_or_default(),
                },
                runtime_home,
                host_home: binding.host_home.clone(),
                preflight_target: binding.target.clone(),
                prepared_change: None,
            },
        },
    )
}

pub(super) fn plan_hash_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "plan_hash": { "type": "string", "minLength": 64, "maxLength": 71 },
            "acknowledgements": {
                "type": "array",
                "items": { "type": "string" },
                "uniqueItems": true,
                "default": []
            }
        },
        "required": ["plan_hash"],
        "additionalProperties": false
    })
}

pub(super) fn maintenance_plan_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "intent": {
                "type": "object",
                "properties": {
                    "schema_version": { "const": "0.4.13-maintenance-intent" },
                    "request_id": { "type": "string", "minLength": 1 },
                    "subject": { "type": "string", "enum": ["ags", "skill", "runtime"] },
                    "operation": { "type": "string", "enum": ["check", "install", "update", "remove", "rollback", "repair"] },
                    "target": { "type": "string", "minLength": 1 },
                    "target_hosts": { "type": "array", "items": { "type": "string" }, "uniqueItems": true },
                    "requested_channel": { "type": "string", "enum": ["notify", "manual", "pinned"] },
                    "source": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "enum": ["github", "local"] },
                            "locator": { "type": "string", "minLength": 1 },
                            "requested_ref": { "type": "string" },
                            "tracking_ref": { "type": "string" },
                            "resolved_revision": { "type": "string" },
                            "subdirectory": { "type": "string" },
                            "content_hash": { "type": "string" },
                            "observed_license": { "type": "string" },
                            "catalog_review_status": { "type": "string" }
                        },
                        "required": ["kind", "locator"],
                        "additionalProperties": false
                    },
                    "options": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["schema_version", "request_id", "subject", "operation", "target"],
                "additionalProperties": false
            }
        },
        "required": ["intent"],
        "additionalProperties": false
    })
}
