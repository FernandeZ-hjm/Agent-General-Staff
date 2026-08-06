use super::*;
use ags_capability_governance::skill_adoption::{
    apply_install, apply_removal, apply_rollback, apply_update, load_installed_skills,
    parse_github_source, plan_install, plan_removal, plan_rollback, recover_applied_change,
    verify_adoption_routes, AdoptionContext, PreparedSkillChange, RiskAcknowledgements, SourceSpec,
    UpdatePolicy,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Shared Skill maintenance adapter used by both the CLI and MCP surfaces.
/// It is intentionally outside either transport crate so planning, risk
/// acknowledgement, apply, verification and recovery cannot drift.
pub struct SkillMaintenanceBackend {
    pub adoption: AdoptionContext,
    pub preflight_target: PathBuf,
    pub activation: std::sync::Arc<dyn CapabilityRuntimeActivator>,
}

impl SkillMaintenanceBackend {
    fn skill_change<'a>(
        &self,
        plan: &'a MaintenancePlan,
    ) -> Result<&'a PreparedSkillChange, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::Skill(change)) => Ok(change),
            _ => Err("maintenance plan has no typed Skill change".to_string()),
        }
    }

    fn prepare_skill(&self, intent: &MaintenanceIntent) -> Result<PreparedSkillChange, String> {
        match intent.operation {
            MaintenanceOperation::Install => {
                let source = source_spec(intent)?;
                let metadata = intent.options.get("routing_metadata_path").map(Path::new);
                let target_hosts = if intent.target_hosts.is_empty() {
                    crate::setup::approved_lifecycle_hosts(&self.adoption.runtime_home)?
                } else {
                    intent.target_hosts.clone()
                };
                if target_hosts.is_empty() {
                    return Err(
                        "skill_install_requires_target_host: run `ags setup --yes` to approve detected Hosts or pass `--host <id>`"
                            .to_string(),
                    );
                }
                plan_install(
                    &self.adoption,
                    &source,
                    metadata,
                    &target_hosts,
                    update_policy(intent),
                )
            }
            MaintenanceOperation::Update | MaintenanceOperation::Check => {
                ags_capability_governance::skill_adoption::plan_update(
                    &self.adoption,
                    &intent.target,
                )
            }
            MaintenanceOperation::Rollback => {
                let revision = intent
                    .source
                    .as_ref()
                    .and_then(|source| source.resolved_revision.as_deref())
                    .map(str::to_string)
                    .or_else(|| previous_revision(&self.adoption, &intent.target).ok())
                    .ok_or_else(|| {
                        "rollback requires source.resolved_revision or a retained previous revision"
                            .to_string()
                    })?;
                plan_rollback(&self.adoption, &intent.target, &revision)
            }
            MaintenanceOperation::Remove => plan_removal(&self.adoption, &intent.target),
            _ => Err(format!(
                "unsupported Skill maintenance operation: {:?}",
                intent.operation
            )),
        }
    }
}

impl MaintenanceBackend for SkillMaintenanceBackend {
    fn prepare(&self, intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String> {
        if intent.subject != MaintenanceSubject::Skill {
            return Err("Skill maintenance backend accepts only subject=skill".to_string());
        }
        let adoption = self.prepare_skill(intent)?;
        let risks = adoption
            .risk_findings
            .iter()
            .map(|risk| RiskFinding {
                id: risk.acknowledgement_id(),
                class: if risk.acknowledgement_required {
                    RiskClass::AcknowledgementRequired
                } else {
                    RiskClass::Advisory
                },
                summary: risk.detail.clone(),
                evidence_hash: None,
            })
            .collect::<Vec<_>>();
        let planned_writes = adoption
            .planned_writes
            .iter()
            .map(|path| PlannedWrite {
                operation: adoption.operation.clone(),
                path: path.clone(),
                before_hash: adoption.previous_body_hash.clone(),
                after_hash: Some(adoption.body_hash.clone()),
            })
            .collect::<Vec<_>>();
        let activation = adoption
            .target_hosts
            .iter()
            .map(|host| ActivationRequirement {
                host: host.clone(),
                requires_restart: true,
                requires_repreflight: true,
                expected_snapshot_hash: None,
                exact_route_target: Some(adoption.skill_id.clone()),
            })
            .collect::<Vec<_>>();
        let recovery_point =
            adoption
                .previous_body_revision
                .as_ref()
                .map(|revision| RecoveryPoint {
                    id: revision.clone(),
                    state_hash: ags_platform::sha256(revision.as_bytes()),
                });
        let mut metadata = BTreeMap::new();
        metadata.insert("adoption_operation".to_string(), adoption.operation.clone());
        metadata.insert("skill_id".to_string(), adoption.skill_id.clone());
        metadata.insert(
            "update_policy".to_string(),
            serde_json::to_value(adoption.update_policy)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "notify".to_string()),
        );
        if let Ok(catalog) =
            ags_capability_governance::third_party_manifest::resolve_third_party_manifest(
                &self.adoption.authority_root,
            )
        {
            if adoption.catalog_review
                == ags_capability_governance::skill_adoption::CatalogReviewStatus::Reviewed
            {
                let mut distributions = catalog.manifest.capabilities.iter().filter(|capability| {
                    capability.kind
                        == ags_capability_governance::third_party_manifest::CapabilityKind::Skill
                        && capability
                            .source
                            .integrity
                            .as_deref()
                            .is_some_and(|hash| hash == adoption.source_hash)
                        && capability
                            .compatibility_parent
                            .as_deref()
                            .unwrap_or(&capability.id)
                            == adoption.skill_id
                });
                if let Some(distribution) = distributions.next() {
                    if distributions.next().is_none() {
                        metadata.insert(
                            "catalog_distribution_id".to_string(),
                            distribution.id.clone(),
                        );
                        if !distribution.name.is_empty() {
                            metadata.insert(
                                "catalog_display_name".to_string(),
                                distribution.name.clone(),
                            );
                        }
                    }
                }
            }
            metadata.insert("catalog_source".to_string(), catalog.source);
            metadata.insert("catalog_hash".to_string(), catalog.content_hash);
            if let Some(release) = catalog.release {
                metadata.insert("catalog_release".to_string(), release);
            }
        }
        Ok(PreparedMaintenance {
            current_version: adoption.previous_body_revision.clone(),
            target_version: Some(adoption.body_hash.clone()),
            source: Some(maintenance_source(&adoption)),
            planned_writes,
            risks,
            verification_steps: vec![VerificationStep {
                id: "skill-body-host-snapshot-route".to_string(),
                description:
                    "verify immutable body, Host indexes, repreflight and exact Skill route"
                        .to_string(),
            }],
            activation,
            recovery_point,
            metadata,
            payload: Some(MaintenancePayload::Skill(Box::new(adoption))),
        })
    }

    fn apply(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.skill_change(plan)?;
        let acknowledgements = plan
            .required_acknowledgements
            .iter()
            .cloned()
            .collect::<RiskAcknowledgements>();
        let receipt = match plan.intent.operation {
            MaintenanceOperation::Install => {
                apply_install(&self.adoption, change, &plan.plan_hash, &acknowledgements)?
            }
            MaintenanceOperation::Update => {
                apply_update(&self.adoption, change, &plan.plan_hash, &acknowledgements)?
            }
            MaintenanceOperation::Rollback => {
                apply_rollback(&self.adoption, change, &plan.plan_hash)?
            }
            MaintenanceOperation::Remove => apply_removal(&self.adoption, change, &plan.plan_hash)?,
            operation => {
                return Err(format!(
                    "maintenance apply does not accept operation {operation:?}"
                ))
            }
        };
        Ok(MaintenanceExecution {
            status: MaintenanceStatus::Applied,
            applied_writes: plan.planned_writes.clone(),
            verification_results: vec![VerificationResult {
                id: "adoption-receipt".to_string(),
                passed: true,
                evidence: receipt.transaction_id,
            }],
            activation_results: receipt
                .snapshot_hashes
                .into_iter()
                .map(|(host, hash)| ActivationResult {
                    host,
                    activated: true,
                    repreflight_passed: false,
                    route_verified: false,
                    evidence: hash,
                })
                .collect(),
            recovery_status: "available".to_string(),
            error: None,
        })
    }

    fn verify(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let skill_id = plan
            .metadata
            .get("skill_id")
            .map(String::as_str)
            .unwrap_or(&plan.intent.target);
        let route_status = verify_adoption_routes(
            &self.adoption.runtime_home,
            &self.adoption.host_home,
            skill_id,
        )?;
        let expected_hosts = plan
            .activation
            .iter()
            .map(|activation| activation.host.clone())
            .collect::<BTreeSet<_>>();
        let affected_hosts = expected_hosts.iter().cloned().collect::<Vec<_>>();
        let activation_request = CapabilityRuntimeActivationRequest::from_runtime(
            &self.preflight_target,
            &self.adoption.runtime_home,
            &affected_hosts,
            false,
        )?;
        let runtime_activation = self.activation.activate(&activation_request)?;
        if runtime_activation.activated_snapshot_hashes != activation_request.active_snapshot_hashes
        {
            return Err(format!(
                "Skill runtime activation hashes differ: expected {:?}, observed {:?}",
                activation_request.active_snapshot_hashes,
                runtime_activation.activated_snapshot_hashes
            ));
        }
        if let Some(loaded) = &runtime_activation.loaded_snapshot_hashes {
            for (host, hash) in &activation_request.active_snapshot_hashes {
                if loaded.get(host) != Some(hash) {
                    return Err(format!(
                        "Skill runtime loaded stale `{host}` snapshot: expected {hash}, observed {:?}",
                        loaded.get(host)
                    ));
                }
            }
            for host in &activation_request.retired_hosts {
                if loaded.contains_key(host) {
                    return Err(format!("Skill runtime retained retired `{host}` snapshot"));
                }
            }
        }
        let repreflight = expected_hosts
            .iter()
            .map(|host| {
                let agent = ags_workspace_facts::AgentType::from_str(host)
                    .map_err(|error| format!("cannot map maintenance Host `{host}`: {error}"))?;
                let result =
                    ags_workspace_facts::run_session_preflight(&self.preflight_target, &agent);
                Ok((host.clone(), result.exit_code == 0, result))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let passed = route_status.verified_on_all_targets()
            && expected_hosts
                .iter()
                .all(|host| repreflight.iter().any(|(item, ok, _)| item == host && *ok));
        let mut verification_results = vec![VerificationResult {
            id: "skill-body-host-snapshot-route".to_string(),
            passed,
            evidence: serde_json::to_string(&route_status).unwrap_or_default(),
        }];
        if let Some(distribution_id) = plan.metadata.get("catalog_distribution_id") {
            verification_results.push(VerificationResult {
                id: "catalog-distribution-identity".to_string(),
                passed,
                evidence: serde_json::json!({
                    "distribution_id": distribution_id,
                    "display_name": plan.metadata.get("catalog_display_name"),
                    "compatibility_parent": skill_id,
                    "catalog_hash": plan.metadata.get("catalog_hash"),
                    "catalog_release": plan.metadata.get("catalog_release"),
                })
                .to_string(),
            });
        }
        Ok(MaintenanceExecution {
            status: if passed {
                MaintenanceStatus::Verified
            } else {
                MaintenanceStatus::Failed
            },
            applied_writes: Vec::new(),
            verification_results,
            activation_results: expected_hosts
                .into_iter()
                .map(|host| {
                    let activation = route_status
                        .activations
                        .iter()
                        .find(|activation| activation.host == host);
                    let preflight = repreflight
                        .iter()
                        .find(|(candidate, _, _)| candidate == &host);
                    ActivationResult {
                        activated: activation.is_some_and(|item| item.snapshot_loaded),
                        repreflight_passed: preflight.is_some_and(|(_, passed, _)| *passed),
                        route_verified: activation.is_some_and(|item| item.route_verified),
                        evidence: serde_json::json!({
                            "activation": activation,
                            "preflight": preflight.map(|(_, _, result)| result),
                            "runtime_identity": runtime_activation.runtime_identity,
                        })
                        .to_string(),
                        host,
                    }
                })
                .collect(),
            recovery_status: "not-required".to_string(),
            error: (!passed).then(|| {
                "installed Skill did not pass body, Host, repreflight and exact route verification"
                    .to_string()
            }),
        })
    }

    fn recover(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.skill_change(plan)?;
        let evidence =
            recover_applied_change(&self.adoption, change, &plan.plan_hash)?.transaction_id;
        let activation_request = CapabilityRuntimeActivationRequest::from_runtime(
            &self.preflight_target,
            &self.adoption.runtime_home,
            &change.target_hosts,
            false,
        )?;
        self.activation.activate(&activation_request)?;
        Ok(MaintenanceExecution {
            status: MaintenanceStatus::Recovered,
            applied_writes: Vec::new(),
            verification_results: vec![VerificationResult {
                id: "recovery".to_string(),
                passed: true,
                evidence,
            }],
            activation_results: Vec::new(),
            recovery_status: "recovered".to_string(),
            error: None,
        })
    }
}

fn source_spec(intent: &MaintenanceIntent) -> Result<SourceSpec, String> {
    let source = intent
        .source
        .as_ref()
        .ok_or_else(|| "Skill install intent requires source".to_string())?;
    match source.kind.as_str() {
        "github" => {
            let parsed = parse_github_source(&source.locator, source.requested_ref.as_deref())?;
            let SourceSpec::GitHub {
                url,
                requested_ref,
                tracking_ref: _,
                subdir: parsed_subdir,
            } = parsed
            else {
                return Err("GitHub parser returned a non-GitHub source".to_string());
            };
            if parsed_subdir.is_some()
                && source.subdirectory.is_some()
                && parsed_subdir != source.subdirectory
            {
                return Err(
                    "maintenance source subdirectory conflicts with the GitHub URL".to_string(),
                );
            }
            Ok(SourceSpec::github(
                url,
                requested_ref,
                source.subdirectory.clone().or(parsed_subdir),
            )
            .with_tracking_ref(source.tracking_ref.clone()))
        }
        "local" => Ok(SourceSpec::local(&source.locator)),
        "git" => Ok(SourceSpec::Git {
            url: source.locator.clone(),
            requested_ref: source.requested_ref.clone(),
            tracking_ref: source.tracking_ref.clone(),
            subdir: source.subdirectory.clone(),
        }),
        other => Err(format!("unsupported Skill source kind `{other}`")),
    }
}

pub fn maintenance_source_from_spec(source: &SourceSpec) -> MaintenanceSource {
    let (kind, locator, requested_ref, tracking_ref, subdirectory) = match source {
        SourceSpec::Local { path } => ("local", path.clone(), None, None, None),
        SourceSpec::GitHub {
            url,
            requested_ref,
            tracking_ref,
            subdir,
        } => (
            "github",
            url.clone(),
            requested_ref.clone(),
            tracking_ref.clone(),
            subdir.clone(),
        ),
        SourceSpec::Git {
            url,
            requested_ref,
            tracking_ref,
            subdir,
        } => (
            "git",
            url.clone(),
            requested_ref.clone(),
            tracking_ref.clone(),
            subdir.clone(),
        ),
    };
    MaintenanceSource {
        kind: kind.to_string(),
        locator,
        tracking_ref,
        requested_ref,
        resolved_revision: None,
        subdirectory,
        content_hash: None,
        observed_license: None,
        catalog_review_status: None,
    }
}

fn maintenance_source(plan: &PreparedSkillChange) -> MaintenanceSource {
    let source_spec = plan
        .resolved_source
        .as_ref()
        .map(|resolved| &resolved.source_spec)
        .unwrap_or(&plan.source_spec);
    let mut source = maintenance_source_from_spec(source_spec);
    source.resolved_revision = plan
        .resolved_source
        .as_ref()
        .map(|resolved| resolved.resolved_commit.clone());
    source.content_hash = Some(plan.body_hash.clone());
    source.observed_license = (!plan.license_path.is_empty())
        .then(|| format!("{}#{}", plan.license_path, plan.license_hash));
    source.catalog_review_status = serde_json::to_value(plan.catalog_review)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string));
    source
}

fn update_policy(intent: &MaintenanceIntent) -> UpdatePolicy {
    match intent.requested_channel.as_deref() {
        Some("manual") => UpdatePolicy::Manual,
        Some("pinned") => UpdatePolicy::Pinned,
        _ => UpdatePolicy::Notify,
    }
}

fn previous_revision(context: &AdoptionContext, skill_id: &str) -> Result<String, String> {
    let registry = load_installed_skills(&context.runtime_home)?;
    let record = registry
        .skills
        .get(skill_id)
        .ok_or_else(|| format!("skill is not installed: {skill_id}"))?;
    record
        .body_revisions
        .iter()
        .rev()
        .find(|revision| revision.revision != record.body_revision)
        .map(|revision| revision.revision.clone())
        .ok_or_else(|| "no previous immutable body revision is retained".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_intent(locator: &str, subdirectory: Option<&str>) -> MaintenanceIntent {
        let mut intent = MaintenanceIntent::new(
            "source-contract",
            MaintenanceSubject::Skill,
            MaintenanceOperation::Install,
            "source-contract",
        );
        intent.source = Some(MaintenanceSource {
            kind: "github".to_string(),
            locator: locator.to_string(),
            requested_ref: Some("main".to_string()),
            tracking_ref: Some("main".to_string()),
            resolved_revision: None,
            subdirectory: subdirectory.map(str::to_string),
            content_hash: None,
            observed_license: None,
            catalog_review_status: None,
        });
        intent
    }

    #[test]
    fn github_subdirectory_survives_the_transport_boundary() {
        let source = source_spec(&github_intent(
            "https://github.com/example/repository",
            Some("skills/example"),
        ))
        .unwrap();
        assert_eq!(source.subdir(), Some("skills/example"));
        assert_eq!(source.requested_ref(), Some("main"));
    }

    #[test]
    fn conflicting_github_subdirectories_fail_closed() {
        let error = source_spec(&github_intent(
            "https://github.com/example/repository/tree/main/skills/one",
            Some("skills/two"),
        ))
        .unwrap_err();
        assert!(error.contains("subdirectory conflicts"));
    }
}
