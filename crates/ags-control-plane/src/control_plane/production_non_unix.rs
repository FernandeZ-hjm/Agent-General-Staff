use super::*;

#[derive(Clone)]
pub(crate) enum ProductionAction {}

pub(crate) struct ProductionEffectAdapter {
    _runtime_home: PathBuf,
    _host_home: PathBuf,
}

impl ProductionEffectAdapter {
    pub(crate) fn new(runtime_home: impl Into<PathBuf>) -> Self {
        Self {
            _runtime_home: runtime_home.into(),
            _host_home: ags_platform::home_dir_or_temp(),
        }
    }

    pub(crate) fn with_host_home(
        runtime_home: impl Into<PathBuf>,
        host_home: impl Into<PathBuf>,
    ) -> Self {
        Self {
            _runtime_home: runtime_home.into(),
            _host_home: host_home.into(),
        }
    }
}

fn unavailable() -> EffectError {
    EffectError {
        code: platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE.to_string(),
        detail:
            "this target has no backend that can prove the required retained-descriptor semantics"
                .to_string(),
        effect_started: false,
        output_digest: sha256("platform-descriptor-semantics-unavailable"),
        observed_write_set: Vec::new(),
    }
}

impl EffectAdapter for ProductionEffectAdapter {
    type Action = ProductionAction;

    fn validate_platform_support(&self, operation: &OperationRequest) -> Result<(), EffectError> {
        if matches!(operation, OperationRequest::Schema(_)) {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    fn plan(
        &self,
        _operation: &OperationRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<Self::Action>, EffectError> {
        Err(unavailable())
    }

    fn read_only_roots(
        &self,
        _operation: &OperationRequest,
        _binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        Vec::new()
    }

    fn read(
        &self,
        operation: &OperationRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<ReadObservation, EffectError> {
        let OperationRequest::Schema(request) = operation else {
            return Err(unavailable());
        };
        let result = schema_read_result(request).map_err(|error| EffectError {
            code: error.code.to_string(),
            detail: error.detail,
            effect_started: false,
            output_digest: sha256("schema-read-failed"),
            observed_write_set: Vec::new(),
        })?;
        let output_digest = sha256(serde_json::to_vec(&result).map_err(|error| EffectError {
            code: "read_output_encode_failed".to_string(),
            detail: error.to_string(),
            effect_started: false,
            output_digest: sha256("read-output-encode-failed"),
            observed_write_set: Vec::new(),
        })?);
        Ok(ReadObservation {
            result,
            output_digest,
            succeeded: true,
        })
    }

    fn apply(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _operation: Option<&OperationRequest>,
        _binding: &AuthenticatedBinding,
    ) -> Result<EffectObservation, EffectError> {
        Err(unavailable())
    }

    fn verify(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _observation: &EffectObservation,
    ) -> Result<VerificationObservation, EffectError> {
        Err(unavailable())
    }

    fn recover(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _observation: &EffectObservation,
    ) -> Result<RecoveryObservation, EffectError> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> AuthenticatedBinding {
        let workspace = PathBuf::from(r"C:\ags-workspace");
        AuthenticatedBinding::cli(
            "windows",
            workspace.clone(),
            sha256("workspace"),
            sha256("facts"),
            "registry",
            "workspace-service",
            vec![workspace],
        )
    }

    fn plane() -> (ControlPlane<ProductionEffectAdapter>, AuthenticatedBinding) {
        let binding = binding();
        (
            ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(r"C:\ags-runtime"),
                sha256("windows-test-seal"),
            ),
            binding,
        )
    }

    #[test]
    fn schema_uses_the_portable_builder_and_succeeds() {
        let (mut plane, binding) = plane();
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy"),
            })
            .unwrap();
        let request = SchemaRequest {
            context: OperationContext::default(),
            operation: None,
        };
        let expected = schema_read_result(&request).unwrap();
        let decision = plane
            .decide(&session, OperationRequest::Schema(request))
            .unwrap();
        assert_eq!(decision.result, Some(expected));
        assert!(decision.action_ref.is_none());
        assert!(decision.receipt.is_some());
    }

    #[test]
    fn filesystem_read_is_rejected_before_handler_with_exact_platform_code() {
        let (mut plane, binding) = plane();
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy"),
            })
            .unwrap();
        let error = plane
            .decide(
                &session,
                OperationRequest::Doctor(DoctorRequest {
                    context: OperationContext::default(),
                    scope: DoctorScope::All,
                }),
            )
            .unwrap_err();
        assert_eq!(error.code, platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE);
        assert!(plane.actions.is_empty());
    }

    #[test]
    fn effectful_operation_is_rejected_before_action_is_stored() {
        let (mut plane, binding) = plane();
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy"),
            })
            .unwrap();
        let error = plane
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: Vec::new(),
                }),
            )
            .unwrap_err();
        assert_eq!(error.code, platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE);
        assert!(plane.actions.is_empty());
    }
}
