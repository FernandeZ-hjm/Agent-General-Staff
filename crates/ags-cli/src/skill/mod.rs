//! Skill catalog and machine-local adoption facade.

use crate::cli::SkillAction;
use ags_capability_governance::skill_adoption::{
    load_installed_skills, parse_github_source, verify_adoption_routes, AdoptionContext,
    SourceSpec, UpdatePolicy,
};
use ags_lifecycle::maintenance::{
    maintenance_source_from_spec, MaintenanceIntent, MaintenanceOperation, MaintenancePlan,
    MaintenanceReceipt, MaintenanceService, MaintenanceSource, MaintenanceSubject, ServiceClock,
    ServiceContext, SkillMaintenanceBackend,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn authority(command: &str) -> std::path::PathBuf {
    crate::context::capability_authority_root_or_exit(command)
}

fn verify(skill_id: Option<&str>, plan_hash: Option<&str>, strict: bool, format: &str) {
    if let Some(plan_hash) = plan_hash {
        let receipt = maintenance_service("ags skill verify")
            .verify(plan_hash)
            .unwrap_or_else(|error| {
                eprintln!("ags skill verify: refused — {error}");
                std::process::exit(1);
            });
        let passed = receipt.status == ags_lifecycle::maintenance::MaintenanceStatus::Verified;
        emit_maintenance_receipt(&receipt, format);
        if strict && !passed {
            std::process::exit(1);
        }
        return;
    }

    if let Some(skill_id) = skill_id {
        let context = adoption_context("ags skill verify");
        let status = verify_adoption_routes(&context.runtime_home, &context.host_home, skill_id)
            .unwrap_or_else(|error| {
                eprintln!("ags skill verify: {error}");
                std::process::exit(1);
            });
        let passed = status.verified_on_all_targets();
        crate::output::emit(format, &status, || {
            format!(
                "Skill route verification\nSkill: {}\nVerified on all target hosts: {}\nVerified hosts: {}",
                status.installation.skill_id,
                passed,
                status
                    .activations
                    .iter()
                    .filter(|item| item.route_verified)
                    .map(|item| item.host.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        });
        if strict && !passed {
            std::process::exit(1);
        }
        return;
    }

    eprintln!("ags skill verify: provide <skill-id> or --plan-hash <hash>");
    std::process::exit(2);
}

fn adoption_context(command: &str) -> AdoptionContext {
    AdoptionContext {
        authority_root: authority(command),
        runtime_home: ags_platform::runtime_home(),
        host_home: ags_platform::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
        snapshot_discovery: ags_capability_governance::skill_adoption::SnapshotDiscovery::Live,
    }
}

fn maintenance_service(command: &str) -> MaintenanceService<SkillMaintenanceBackend> {
    let adoption = adoption_context(command);
    // Catalog authority and the workspace that requested the operation are
    // separate facts. A public release resolves its catalog from the bundled
    // runtime, which is intentionally not an AGS-managed project; repreflight
    // must therefore validate the current workspace just like the MCP path.
    let preflight_target =
        std::env::current_dir().unwrap_or_else(|_| adoption.authority_root.clone());
    let binding_material = format!(
        "cli\n{}\n{}",
        adoption.runtime_home.display(),
        adoption.host_home.display()
    );
    MaintenanceService::new(
        ServiceContext {
            runtime_home: adoption.runtime_home.clone(),
            binding_id: format!(
                "cli:{}",
                ags_platform::sha256_hex(binding_material.as_bytes())
            ),
            clock: ServiceClock::System,
            plan_ttl_seconds: 30 * 60,
        },
        SkillMaintenanceBackend {
            adoption,
            preflight_target,
            activation: ags_mcp::workspace_capability_runtime_activator(),
        },
    )
    .unwrap_or_else(|error| {
        eprintln!("{command}: cannot initialize maintenance service — {error}");
        std::process::exit(1);
    })
}

fn request_id(operation: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("cli-{operation}-{nonce}")
}

fn install_intent(
    source: &SourceSpec,
    metadata: Option<&Path>,
    hosts: &[String],
    policy: &str,
) -> MaintenanceIntent {
    let mut intent = MaintenanceIntent::new(
        request_id("skill-install"),
        MaintenanceSubject::Skill,
        MaintenanceOperation::Install,
        source.repository_url().unwrap_or_else(|| match source {
            SourceSpec::Local { path } => path,
            _ => "skill",
        }),
    );
    intent.target_hosts = hosts.to_vec();
    intent.requested_channel = Some(policy.to_string());
    intent.source = Some(maintenance_source_from_spec(source));
    if let Some(metadata) = metadata {
        intent.options.insert(
            "routing_metadata_path".to_string(),
            metadata.to_string_lossy().into_owned(),
        );
    }
    intent
}

fn observed_git_origin(path: &Path) -> Option<String> {
    let directory = if path.is_dir() { path } else { path.parent()? };
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn resolve_remote_source(command: &str, value: &str, requested_ref: Option<&str>) -> SourceSpec {
    let path = PathBuf::from(value);
    if path.exists() {
        eprintln!(
            "{command}: local paths must use `ags skill adopt`; install accepts a catalog id or GitHub HTTPS URL"
        );
        std::process::exit(2);
    }
    if value.starts_with("https://github.com/") {
        return parse_github_source(value, requested_ref).unwrap_or_else(|error| {
            eprintln!("{command}: refused — {error}");
            std::process::exit(1);
        });
    }
    let recommendations =
        ags_capability_governance::skill_body::recommendations::read_recommendations(&authority(
            command,
        ));
    let recommendation = recommendations
        .skills
        .iter()
        .find(|candidate| candidate.id == value)
        .unwrap_or_else(|| {
            eprintln!(
                "{command}: unknown catalog id or source `{value}`; catalog membership is not required, but arbitrary remote sources must be GitHub HTTPS URLs"
            );
            std::process::exit(1);
        });
    if recommendation.source_kind == "bundled" {
        let bundled = recommendation.bundled_path.as_deref().unwrap_or_else(|| {
            eprintln!("{command}: recommendation `{value}` has no bundled source path");
            std::process::exit(1);
        });
        let root = authority(command);
        let root = std::fs::canonicalize(&root).unwrap_or_else(|error| {
            eprintln!("{command}: cannot resolve capability authority: {error}");
            std::process::exit(1);
        });
        let candidate = root.join(bundled);
        let candidate = std::fs::canonicalize(&candidate).unwrap_or_else(|error| {
            eprintln!("{command}: bundled recommendation `{value}` is unavailable: {error}");
            std::process::exit(1);
        });
        if !candidate.starts_with(&root) {
            eprintln!("{command}: bundled recommendation `{value}` escapes capability authority");
            std::process::exit(1);
        }
        return SourceSpec::local(candidate.to_string_lossy());
    }
    let source = recommendation
        .source
        .as_deref()
        .or(recommendation.upstream.as_deref());
    let source = source.unwrap_or_else(|| {
        eprintln!("{command}: recommendation `{value}` has no installable source identity");
        std::process::exit(1);
    });
    parse_github_source(source, requested_ref.or(recommendation.revision.as_deref()))
        .map(|source| source.with_tracking_ref(recommendation.tracking_ref.clone()))
        .unwrap_or_else(|error| {
            eprintln!("{command}: invalid recommendation source — {error}");
            std::process::exit(1);
        })
}

fn recommend(format: &str) {
    let root = authority("ags skill recommend");
    let recommendations =
        ags_capability_governance::skill_body::recommendations::read_recommendations(&root);
    crate::output::emit(format, &recommendations.skills, || {
        let mut lines = vec![
            "AGS third-party Skill recommendations (discovery only; not an allowlist)".to_string(),
        ];
        lines.extend(recommendations.skills.iter().map(|skill| {
            format!(
                "  {} — {} [{}]",
                skill.id,
                skill.purpose,
                skill.license.as_deref().unwrap_or("license unknown")
            )
        }));
        lines.join("\n")
    });
}

fn inspect(
    source: &str,
    requested_ref: Option<&str>,
    metadata: Option<&Path>,
    hosts: &[String],
    policy: &str,
    format: &str,
) {
    let path = PathBuf::from(source);
    if path.exists() {
        let source_spec = SourceSpec::local(path.to_string_lossy());
        let plan = maintenance_service("ags skill inspect")
            .plan(install_intent(&source_spec, metadata, hosts, policy))
            .unwrap_or_else(|error| {
                crate::output::error_exit(
                    "ags skill inspect",
                    format!("refused — {error}"),
                    format,
                    1,
                );
            });
        let observed_repository = observed_git_origin(&path);
        let output = serde_json::json!({
            "plan": plan,
            "observed_repository": observed_repository,
            "source_binding": "local_observation_only"
        });
        crate::output::emit(format, &output, || {
            format!(
                "{}\nObserved Git origin: {}",
                render_maintenance_plan(&plan),
                observed_repository.as_deref().unwrap_or("none")
            )
        });
        return;
    }
    let source = resolve_remote_source("ags skill inspect", source, requested_ref);
    let plan = maintenance_service("ags skill inspect")
        .plan(install_intent(&source, metadata, hosts, policy))
        .unwrap_or_else(|error| {
            crate::output::error_exit("ags skill inspect", format!("refused — {error}"), format, 1);
        });
    emit_maintenance_plan(&plan, format);
}

struct InstallCommand<'a> {
    source: &'a str,
    requested_ref: Option<&'a str>,
    metadata: Option<&'a Path>,
    hosts: &'a [String],
    policy: &'a str,
    acknowledged_risks: &'a [String],
    plan_hash: Option<&'a str>,
    yes: bool,
    format: &'a str,
}

fn install(command: InstallCommand<'_>) {
    let service = maintenance_service("ags skill install");
    if command.yes {
        let reviewed = required_plan_hash("ags skill install", command.plan_hash);
        let acknowledgements = command
            .acknowledged_risks
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let receipt = service
            .apply(reviewed, &acknowledgements)
            .unwrap_or_else(|error| {
                eprintln!("ags skill install: refused — {error}");
                std::process::exit(1);
            });
        emit_maintenance_receipt(&receipt, command.format);
    } else {
        let source =
            resolve_remote_source("ags skill install", command.source, command.requested_ref);
        let plan = service
            .plan(install_intent(
                &source,
                command.metadata,
                command.hosts,
                command.policy,
            ))
            .unwrap_or_else(|error| {
                crate::output::error_exit(
                    "ags skill install",
                    format!("refused — {error}"),
                    command.format,
                    1,
                );
            });
        emit_maintenance_plan(&plan, command.format);
    }
}

fn required_plan_hash<'a>(command: &str, plan_hash: Option<&'a str>) -> &'a str {
    plan_hash.unwrap_or_else(|| {
        eprintln!("{command}: refused — --plan-hash is required with --yes");
        std::process::exit(2);
    })
}

fn check(skill_id: Option<&str>, format: &str) {
    let context = adoption_context("ags skill check");
    let service = maintenance_service("ags skill check");
    let registry = load_installed_skills(&context.runtime_home).unwrap_or_else(|error| {
        eprintln!("ags skill check: {error}");
        std::process::exit(1);
    });
    let ids = match skill_id {
        Some(skill_id) => vec![skill_id.to_string()],
        None => registry.skills.keys().cloned().collect(),
    };
    let mut results = Vec::new();
    for id in ids {
        let Some(record) = registry.skills.get(&id) else {
            eprintln!("ags skill check: skill is not installed: {id}");
            std::process::exit(1);
        };
        let mut candidates = Vec::new();
        let mut status = "current".to_string();
        if record.update_policy == UpdatePolicy::Pinned {
            status = "pinned".to_string();
        } else {
            let intent = MaintenanceIntent::new(
                request_id("skill-check"),
                MaintenanceSubject::Skill,
                MaintenanceOperation::Check,
                &id,
            );
            match service.plan(intent) {
                Ok(plan) => {
                    status = "update_available".to_string();
                    candidates.push(plan);
                }
                Err(error) if error == "no_update_available" => {}
                Err(error) if error == "local_source_has_no_upstream_update_candidate" => {
                    status = "local_source_reinstall_required".to_string();
                }
                Err(error) => {
                    eprintln!("ags skill check: {error}");
                    std::process::exit(1);
                }
            }
        }
        results.push(serde_json::json!({
            "skill_id": id,
            "update_policy": record.update_policy,
            "status": status,
            "update_candidates": candidates,
        }));
    }
    let output = if skill_id.is_some() {
        results
            .into_iter()
            .next()
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({"skills": results})
    };
    crate::output::emit(format, &output, || {
        serde_json::to_string_pretty(&output).unwrap_or_default()
    });
}

fn update(
    skill_id: &str,
    acknowledged_risks: &[String],
    plan_hash: Option<&str>,
    yes: bool,
    format: &str,
) {
    let service = maintenance_service("ags skill update");
    if yes {
        let reviewed = required_plan_hash("ags skill update", plan_hash);
        let acknowledgements = acknowledged_risks.iter().cloned().collect::<BTreeSet<_>>();
        let receipt = service
            .apply(reviewed, &acknowledgements)
            .unwrap_or_else(|error| {
                eprintln!("ags skill update: refused — {error}");
                std::process::exit(1);
            });
        emit_maintenance_receipt(&receipt, format);
    } else {
        let intent = MaintenanceIntent::new(
            request_id("skill-update"),
            MaintenanceSubject::Skill,
            MaintenanceOperation::Update,
            skill_id,
        );
        let plan = service.plan(intent).unwrap_or_else(|error| {
            eprintln!("ags skill update: refused — {error}");
            std::process::exit(1);
        });
        emit_maintenance_plan(&plan, format);
    }
}

fn rollback(
    skill_id: &str,
    revision: Option<&str>,
    plan_hash: Option<&str>,
    yes: bool,
    format: &str,
) {
    let context = adoption_context("ags skill rollback");
    let service = maintenance_service("ags skill rollback");
    if yes {
        let reviewed = required_plan_hash("ags skill rollback", plan_hash);
        let receipt = service
            .apply(reviewed, &BTreeSet::new())
            .unwrap_or_else(|error| {
                eprintln!("ags skill rollback: refused — {error}");
                std::process::exit(1);
            });
        emit_maintenance_receipt(&receipt, format);
        return;
    }
    let selected = revision.map(str::to_string).unwrap_or_else(|| {
        let registry = load_installed_skills(&context.runtime_home).unwrap_or_else(|error| {
            eprintln!("ags skill rollback: {error}");
            std::process::exit(1);
        });
        let record = registry.skills.get(skill_id).unwrap_or_else(|| {
            eprintln!("ags skill rollback: skill is not installed: {skill_id}");
            std::process::exit(1);
        });
        record
            .body_revisions
            .iter()
            .rev()
            .find(|candidate| candidate.revision != record.body_revision)
            .map(|candidate| candidate.revision.clone())
            .unwrap_or_else(|| {
                eprintln!("ags skill rollback: no previous immutable body revision is retained");
                std::process::exit(1);
            })
    });
    let mut intent = MaintenanceIntent::new(
        request_id("skill-rollback"),
        MaintenanceSubject::Skill,
        MaintenanceOperation::Rollback,
        skill_id,
    );
    intent.source = Some(MaintenanceSource {
        kind: "installed".to_string(),
        locator: skill_id.to_string(),
        requested_ref: None,
        tracking_ref: None,
        resolved_revision: Some(selected),
        subdirectory: None,
        content_hash: None,
        observed_license: None,
        catalog_review_status: None,
    });
    let plan = service.plan(intent).unwrap_or_else(|error| {
        eprintln!("ags skill rollback: refused — {error}");
        std::process::exit(1);
    });
    emit_maintenance_plan(&plan, format);
}

struct AdoptCommand<'a> {
    source: &'a Path,
    metadata: Option<&'a Path>,
    hosts: &'a [String],
    update_policy: &'a str,
    plan_hash: Option<&'a str>,
    acknowledged_risks: &'a [String],
    yes: bool,
    format: &'a str,
}

fn adopt(command: AdoptCommand<'_>) {
    let service = maintenance_service("ags skill adopt");
    if command.yes {
        let reviewed = command.plan_hash.unwrap_or_else(|| {
            eprintln!("ags skill adopt: refused — --plan-hash is required with --yes");
            std::process::exit(2);
        });
        let acknowledgements = command
            .acknowledged_risks
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let receipt = service
            .apply(reviewed, &acknowledgements)
            .unwrap_or_else(|error| {
                eprintln!("ags skill adopt: refused — {error}");
                std::process::exit(1);
            });
        emit_maintenance_receipt(&receipt, command.format);
    } else {
        let source_spec = SourceSpec::local(command.source.to_string_lossy());
        let plan = service
            .plan(install_intent(
                &source_spec,
                command.metadata,
                command.hosts,
                command.update_policy,
            ))
            .unwrap_or_else(|error| {
                crate::output::error_exit(
                    "ags skill adopt",
                    format!("refused — {error}"),
                    command.format,
                    1,
                );
            });
        emit_maintenance_plan(&plan, command.format);
    }
}

fn remove(skill_id: &str, plan_hash: Option<&str>, yes: bool, format: &str) {
    let service = maintenance_service("ags skill remove");
    if yes {
        let reviewed = plan_hash.unwrap_or_else(|| {
            eprintln!("ags skill remove: refused — --plan-hash is required with --yes");
            std::process::exit(2);
        });
        let receipt = service
            .apply(reviewed, &BTreeSet::new())
            .unwrap_or_else(|error| {
                eprintln!("ags skill remove: refused — {error}");
                std::process::exit(1);
            });
        emit_maintenance_receipt(&receipt, format);
    } else {
        let intent = MaintenanceIntent::new(
            request_id("skill-remove"),
            MaintenanceSubject::Skill,
            MaintenanceOperation::Remove,
            skill_id,
        );
        let plan = service.plan(intent).unwrap_or_else(|error| {
            eprintln!("ags skill remove: refused — {error}");
            std::process::exit(1);
        });
        emit_maintenance_plan(&plan, format);
    }
}

fn status(skill_id: Option<&str>, format: &str) {
    let context = adoption_context("ags skill status");
    let root = authority("ags skill status");
    if skill_id.is_none() {
        let registry = load_installed_skills(&context.runtime_home).unwrap_or_else(|error| {
            eprintln!("ags skill status: {error}");
            std::process::exit(1);
        });
        let mut ids = registry.skills.keys().cloned().collect::<BTreeSet<_>>();
        ids.extend(
            ags_capability_governance::skill_body::recommendations::read_recommendations(&root)
                .skills
                .into_iter()
                .map(|entry| entry.id),
        );
        let ids = ids.into_iter().collect::<Vec<_>>();
        let statuses =
            ags_capability_governance::skill_body::recommendations::skill_status_projections(
                &root,
                &context.runtime_home,
                &context.host_home,
                &ids,
            )
            .unwrap_or_else(|error| {
                eprintln!("ags skill status: {error}");
                std::process::exit(1);
            });
        crate::output::emit(format, &statuses, || {
            serde_json::to_string_pretty(&statuses).unwrap_or_default()
        });
        return;
    }
    let skill_id = skill_id.expect("checked above");
    let status = ags_capability_governance::skill_body::recommendations::skill_status_projection(
        &root,
        &context.runtime_home,
        &context.host_home,
        skill_id,
    )
    .unwrap_or_else(|error| {
        eprintln!("ags skill status: {error}");
        std::process::exit(1);
    });
    crate::output::emit(format, &status, || {
        format!(
            "Skill status\nSkill: {}\nCatalog: {:?}\nInstallation: {:?}\nActivation: {:?}\nUpdate: {:?}\nNext: {}",
            status.skill_id,
            status.catalog.state,
            status.installation.state,
            status.activation.state,
            status.update.state,
            status.next_action,
        )
    });
}

fn emit_maintenance_plan(plan: &MaintenancePlan, format: &str) {
    crate::output::emit(format, plan, || render_maintenance_plan(plan));
}

fn render_maintenance_plan(plan: &MaintenancePlan) -> String {
    format!(
        "Skill maintenance {:?} plan\nTarget: {}\nSource: {}\nTarget version: {}\nWrites: {}\nRisks requiring acknowledgement: {}\nPlan hash: {}\nDry-run only — review, then pass --yes --plan-hash <hash> and one --ack-risk per accepted finding.",
        plan.intent.operation,
        plan.intent.target,
        plan.source
            .as_ref()
            .map(|source| source.locator.as_str())
            .unwrap_or("installed record"),
        plan.target_version.as_deref().unwrap_or("none"),
        plan.planned_writes.len(),
        plan.required_acknowledgements
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
        plan.plan_hash
    )
}

fn emit_maintenance_receipt(receipt: &MaintenanceReceipt, format: &str) {
    crate::output::emit(format, receipt, || {
        format!(
            "Skill maintenance {:?}\nReceipt: {}\nPlan: {}\nActivation results: {}\nRun `ags_maintenance_verify` or the matching CLI verify step to close route evidence.",
            receipt.status,
            receipt.receipt_id,
            receipt.plan_hash,
            receipt.activation_results.len()
        )
    });
}

pub(crate) fn run(action: Option<SkillAction>, format: &str) {
    match action {
        Some(SkillAction::Recommend { format }) => recommend(&format),
        Some(SkillAction::Inspect {
            source,
            requested_ref,
            metadata,
            host,
            update_policy,
            format,
        }) => inspect(
            &source,
            requested_ref.as_deref(),
            metadata.as_deref(),
            &host,
            &update_policy,
            &format,
        ),
        Some(SkillAction::Install {
            source,
            requested_ref,
            metadata,
            host,
            update_policy,
            acknowledged_risks,
            plan_hash,
            yes,
            format,
        }) => install(InstallCommand {
            source: &source,
            requested_ref: requested_ref.as_deref(),
            metadata: metadata.as_deref(),
            hosts: &host,
            policy: &update_policy,
            acknowledged_risks: &acknowledged_risks,
            plan_hash: plan_hash.as_deref(),
            yes,
            format: &format,
        }),
        Some(SkillAction::Check { skill_id, format }) => check(skill_id.as_deref(), &format),
        Some(SkillAction::Update {
            skill_id,
            acknowledged_risks,
            plan_hash,
            yes,
            format,
        }) => update(
            &skill_id,
            &acknowledged_risks,
            plan_hash.as_deref(),
            yes,
            &format,
        ),
        Some(SkillAction::Rollback {
            skill_id,
            revision,
            plan_hash,
            yes,
            format,
        }) => rollback(
            &skill_id,
            revision.as_deref(),
            plan_hash.as_deref(),
            yes,
            &format,
        ),
        Some(SkillAction::Adopt {
            source,
            metadata,
            host,
            update_policy,
            plan_hash,
            acknowledged_risks,
            yes,
            format,
        }) => adopt(AdoptCommand {
            source: &source,
            metadata: metadata.as_deref(),
            hosts: &host,
            update_policy: &update_policy,
            plan_hash: plan_hash.as_deref(),
            acknowledged_risks: &acknowledged_risks,
            yes,
            format: &format,
        }),
        Some(SkillAction::Remove {
            skill_id,
            plan_hash,
            yes,
            format,
        }) => remove(&skill_id, plan_hash.as_deref(), yes, &format),
        Some(SkillAction::Status { skill_id, format }) => status(skill_id.as_deref(), &format),
        Some(SkillAction::Verify {
            skill_id,
            plan_hash,
            strict,
            format,
        }) => verify(skill_id.as_deref(), plan_hash.as_deref(), strict, &format),
        None => status(None, format),
    }
}
