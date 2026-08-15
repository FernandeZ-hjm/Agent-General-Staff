use super::*;
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(unix)]
thread_local! {
    static SNAPSHOT_AFTER_STAT_SUBSTITUTION: std::cell::RefCell<Option<(PathBuf, PathBuf)>> = const {
        std::cell::RefCell::new(None)
    };
    static ROOT_AFTER_SCAN_SUBSTITUTION: std::cell::RefCell<Option<(PathBuf, PathBuf)>> = const {
        std::cell::RefCell::new(None)
    };
    static STABLE_READ_SAME_INODE_REWRITE: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> = const {
        std::cell::RefCell::new(None)
    };
    static SNAPSHOT_AFTER_READ_REWRITE: std::cell::RefCell<Option<(PathBuf, PathBuf, Vec<u8>)>> = const {
        std::cell::RefCell::new(None)
    };
    static PHYSICAL_DIRECT_MEMBER_SCANS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(unix)]
pub(super) fn note_physical_direct_member_scan() {
    PHYSICAL_DIRECT_MEMBER_SCANS.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(unix)]
pub(super) fn run_snapshot_after_stat_test_hook(relative: &Path) {
    SNAPSHOT_AFTER_STAT_SUBSTITUTION.with(|slot| {
        let should_replace = slot
            .borrow()
            .as_ref()
            .is_some_and(|(_, expected)| expected == relative);
        if !should_replace {
            return;
        }
        let Some((absolute, _)) = slot.borrow_mut().take() else {
            return;
        };
        std::fs::remove_file(&absolute).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(&absolute)
            .status()
            .unwrap()
            .success());
    });
}

#[cfg(unix)]
pub(super) fn run_root_after_scan_test_hook() {
    ROOT_AFTER_SCAN_SUBSTITUTION.with(|slot| {
        let Some((root, displaced)) = slot.borrow_mut().take() else {
            return;
        };
        std::fs::rename(&root, displaced).unwrap();
        std::fs::create_dir(&root).unwrap();
    });
}

#[cfg(unix)]
pub(super) fn run_stable_read_same_inode_rewrite_test_hook(path: &Path) {
    STABLE_READ_SAME_INODE_REWRITE.with(|slot| {
        let rewrite = slot
            .borrow()
            .as_ref()
            .is_some_and(|(expected, _)| expected == path);
        if !rewrite {
            return;
        }
        let Some((path, replacement)) = slot.borrow_mut().take() else {
            return;
        };
        std::fs::write(path, replacement).unwrap();
    });
}

#[cfg(unix)]
pub(super) fn run_snapshot_after_read_rewrite_test_hook(relative: &Path) {
    SNAPSHOT_AFTER_READ_REWRITE.with(|slot| {
        let rewrite = slot
            .borrow()
            .as_ref()
            .is_some_and(|(_, expected, _)| expected == relative);
        if !rewrite {
            return;
        }
        let Some((absolute, _, replacement)) = slot.borrow_mut().take() else {
            return;
        };
        std::fs::write(absolute, replacement).unwrap();
    });
}

#[derive(Debug, Default)]
struct FakeState {
    apply_succeeds: bool,
    verify_succeeds: bool,
    recover_succeeds: bool,
    effect_started: bool,
    observed_writes: Vec<String>,
    recover_calls: usize,
    read_mutates: bool,
    read_result: Option<serde_json::Value>,
    effect_evidence: Option<serde_json::Value>,
    apply_error: bool,
    apply_calls: usize,
    host_verify_calls: usize,
    recovery_action: bool,
    pending_recovery: bool,
    recovery_finalize_calls: usize,
}

#[derive(Clone)]
struct FakeAdapter {
    state: Arc<Mutex<FakeState>>,
    read_root: PathBuf,
}

impl EffectAdapter for FakeAdapter {
    type Action = ();

    fn validate_platform_support(&self, _operation: &OperationRequest) -> Result<(), EffectError> {
        Ok(())
    }

    fn plan(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<Self::Action>, EffectError> {
        let expected_write_paths = vec![binding
            .canonical_workspace()
            .join("allowed")
            .display()
            .to_string()];
        let recoverability = match operation.kind() {
            OperationKind::Transaction => Recoverability::Transactional,
            OperationKind::LocalExecution => Recoverability::SourcePreserving,
            OperationKind::HostDelegated => Recoverability::NotApplicable,
            OperationKind::ReadOnly => unreachable!(),
        };
        let execution = matches!(
            operation.kind(),
            OperationKind::LocalExecution | OperationKind::HostDelegated
        )
        .then(|| CommandSpec {
            program: "true".to_string(),
            argv: Vec::new(),
            cwd: binding.canonical_workspace().to_path_buf(),
            env: Default::default(),
            timeout_ms: 1000,
            allowed_write_paths: expected_write_paths.iter().map(PathBuf::from).collect(),
        });
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest: sha256("fake-action"),
                steps: vec![PlanStep {
                    step_id: "step-1".to_string(),
                    description: "test step".to_string(),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec!["verify-test".to_string()],
                },
                recoverability,
                execution,
            },
            action: (),
        })))
    }

    fn read_only_roots(
        &self,
        _operation: &OperationRequest,
        _binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        vec![self.read_root.clone()]
    }

    fn read(
        &self,
        _operation: &OperationRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<ReadObservation, EffectError> {
        let state = self.state.lock().unwrap();
        if state.read_mutates {
            std::fs::write(self.read_root.join("mutation"), b"bad").unwrap();
        }
        Ok(ReadObservation {
            result: state
                .read_result
                .clone()
                .unwrap_or_else(|| serde_json::json!({"ok": true})),
            output_digest: sha256("read"),
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
        let mut state = self.state.lock().unwrap();
        state.apply_calls += 1;
        if state.apply_error {
            return Err(EffectError {
                code: "synthetic_apply_error".to_string(),
                detail: "synthetic failure".to_string(),
                effect_started: state.effect_started,
                output_digest: sha256("apply-error"),
                observed_write_set: state.observed_writes.clone(),
            });
        }
        EffectObservation::bounded(
            state.apply_succeeds,
            state.effect_started,
            sha256("apply"),
            state.observed_writes.clone(),
            state.effect_evidence.clone(),
        )
    }

    fn verify_host_outcome(
        &self,
        _operation: &OperationRequest,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
        _receipt: &HostOutcomeReceipt,
        _evidence: Option<&VerifiedHostEvidence>,
    ) -> Result<(), EffectError> {
        self.state.lock().unwrap().host_verify_calls += 1;
        Ok(())
    }

    fn verify(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _observation: &EffectObservation,
    ) -> Result<VerificationObservation, EffectError> {
        Ok(VerificationObservation {
            passed: self.state.lock().unwrap().verify_succeeds,
            output_digest: sha256("verify"),
        })
    }

    fn recover(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _observation: &EffectObservation,
    ) -> Result<RecoveryObservation, EffectError> {
        let mut state = self.state.lock().unwrap();
        state.recover_calls += 1;
        Ok(RecoveryObservation {
            succeeded: state.recover_succeeds,
            output_digest: sha256("recover"),
            observed_write_set: state.observed_writes.clone(),
            evidence: None,
            original_journal_digest: None,
        })
    }

    fn is_recovery_action(&self, _action: &Self::Action) -> bool {
        self.state.lock().unwrap().recovery_action
    }

    fn inspect_pending(
        &self,
        binding: &AuthenticatedBinding,
    ) -> Result<PendingInspection<Self::Action>, EffectError> {
        if !self.state.lock().unwrap().pending_recovery {
            return Ok(PendingInspection::default());
        }
        let expected_write_paths = vec![binding
            .canonical_workspace()
            .join("allowed")
            .display()
            .to_string()];
        Ok(PendingInspection {
            active: Some(PendingRecovery {
                operation: OperationName::Setup,
                journal_identity_digest: sha256("fake-pending-journal-identity"),
                journal_state_digest: sha256("fake-pending-journal-state"),
                expected_write_paths,
                action: (),
            }),
            terminal_receipts: Vec::new(),
        })
    }

    fn finalize_recovery(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
        _receipt: &OperationReceipt,
    ) -> Result<(), EffectError> {
        self.state.lock().unwrap().recovery_finalize_calls += 1;
        Ok(())
    }
}

fn fixture() -> (
    tempfile::TempDir,
    ControlPlane<FakeAdapter>,
    Arc<Mutex<FakeState>>,
    AuthenticatedBinding,
    OpenedSession,
) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("allowed")).unwrap();
    let root = temp.path().canonicalize().unwrap();
    let state = Arc::new(Mutex::new(FakeState {
        apply_succeeds: true,
        verify_succeeds: true,
        recover_succeeds: true,
        effect_started: true,
        ..FakeState::default()
    }));
    let adapter = FakeAdapter {
        state: Arc::clone(&state),
        read_root: root.clone(),
    };
    let mut plane = ControlPlane::with_sealing_key(adapter, sha256("test-key"));
    let binding = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        root.clone(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    let session = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("policy"),
        })
        .unwrap();
    (temp, plane, state, binding, session)
}

fn transaction_request() -> OperationRequest {
    OperationRequest::Setup(SetupRequest {
        context: OperationContext::default(),
        approved_hosts: Vec::new(),
    })
}

fn host_request() -> OperationRequest {
    OperationRequest::Test(TestRequest {
        context: OperationContext::default(),
        profile: TestProfile::Smoke,
        executor: TestExecutor::Host,
    })
}

fn production_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    ControlPlane<ProductionEffectAdapter>,
    AuthenticatedBinding,
    OpenedSession,
) {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("config")).unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let runtime_root = runtime.path().canonicalize().unwrap();
    let mut plane = ControlPlane::with_sealing_key(
        ProductionEffectAdapter::with_host_home(&runtime_root, &root),
        sha256("production-test-key"),
    );
    let binding = AuthenticatedBinding::mcp(
        "connection-production",
        "hermes",
        &root,
        "workspace-production",
        sha256("production-facts"),
        "registry-production",
        "session-production",
        vec![root.clone(), runtime_root],
    );
    let session = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("production-policy"),
        })
        .unwrap();
    (workspace, runtime, plane, binding, session)
}

fn initialize_closure_authority(
    plane: &mut ControlPlane<ProductionEffectAdapter>,
    binding: &AuthenticatedBinding,
    session: &OpenedSession,
) {
    let action_ref = plane
        .decide(
            session,
            OperationRequest::Setup(SetupRequest {
                context: OperationContext::default(),
                approved_hosts: vec![binding.host_id().to_string()],
            }),
        )
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);
}

fn production_update_failure_case(
    status: HostOutcomeStatus,
    unprovable_extra: bool,
    unreported_hidden_child: bool,
) -> ApplyResult {
    let (_workspace, runtime, mut plane, binding, session) = production_fixture();
    let runtime_root = runtime.path().canonicalize().unwrap();
    let payload = prepare_update_candidate(&runtime_root, "0.4.20");
    let request = UpdateRequest {
        context: OperationContext::default(),
        channel: "stable".to_string(),
        target_version: Some("0.4.20".to_string()),
    };
    let decision = plane
        .decide(&session, OperationRequest::Update(request.clone()))
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let mut observed_write_set = Vec::new();
    let mut artifacts = Vec::new();
    if unreported_hidden_child {
        let release_directory = runtime_root.join("releases/0.4.20");
        std::fs::create_dir_all(&release_directory).unwrap();
        std::fs::write(release_directory.join("unreported-hidden-child"), b"hidden").unwrap();
    }
    if unprovable_extra {
        let extra = runtime_root.join("unsealed-update-write");
        observed_write_set.push(extra.display().to_string());
        artifacts.push(HostWriteArtifact {
            path: extra.display().to_string(),
            state: HostArtifactState::Present {
                sha256: sha256("unprovable"),
            },
        });
    }
    let output_digest = sha256("update-terminal-failure");
    let evidence = UpdateReceipt {
        schema_version: "ags://schema/contract/v2/update-receipt".to_string(),
        channel: request.channel,
        target_version: request.target_version,
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        observed_write_set: observed_write_set.clone(),
        release_manifest_sha256: payload.manifest_sha256,
        release_tree_digest: payload.tree_digest,
        output_digest: output_digest.clone(),
        completed: false,
    };
    let evidence_bytes = serde_json::to_vec(&evidence).unwrap();
    let receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: plane.outcome_grants[&action_ref].instruction_digest.clone(),
        outcome_token: issued.outcome_token.unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status,
        output_digest,
        observed_write_set,
        artifacts,
        evidence: Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::UpdateReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: "memory://update-terminal-failure".to_string(),
                sha256: sha256(&evidence_bytes),
            },
            content_hex: hex_bytes(&evidence_bytes),
        }),
    };
    let bytes = serde_json::to_vec(&receipt).unwrap();
    plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://update-terminal-outcome".to_string(),
                        sha256: sha256(&bytes),
                    },
                    bytes,
                )),
            },
        )
        .unwrap()
}

fn write_valid_closure_artifacts(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let task = "## 任务卡\n\
读取并遵守：\n- 本任务卡\n\
Contract ID: tc-0123456789abcdef\n\
Handoff source: host-plan-mode\n\
Confirmed handoff contract: yes\n\
Executor: Claude Code\n\
Runtime adapter: claude-code\n\
Execution surface: cli\n\
Execution mode: single-writer\n\
Execution topology: single\n\
Execution effort: normal\n\
Delegation planning: no\n\
任务级别：Medium\n\
Review gate:\n- 按协议执行\n\
任务：实现并验证闭环\n\
背景：测试闭环\n\
项目画像：无\n记忆胶囊：无\n任务存档：无\n\
目标文件夹路径：\n- .\n相关路径：\n- .\n本次任务相关文件：\n- .\n\
目标：\n- G-01: 完成闭环\n\
验收标准：\n- AC-01 -> G-01: 闭环校验通过\n\
非目标：\n- 不发布\n\
验证：\n- 运行闭环校验\n\
Verification gate:\n- commands:\n  - V-01 -> AC-01: cargo test\n- expected evidence:\n  - EV-01 -> AC-01: test pass\n- stop condition:\n  - 失败时停止\n\
交付：\n- 输出交付报告\n";
    let mut plan = serde_json::json!({
        "schema_version": "ags://schema/contract/v2/launch-plan",
        "task_card_hash": ags_evidence::sha256_hex(task.as_bytes()),
        "launch_plan_hash": "",
        "effective_execution_mode": "single-writer",
        "effective_execution_topology": "single",
        "delegation_planning": false
    });
    plan["launch_plan_hash"] = serde_json::Value::String(
        ags_task_contract::runner::canonical_launch_plan_hash(&plan).unwrap(),
    );
    let plan_text = serde_json::to_string_pretty(&plan).unwrap();
    let task_hash = ags_evidence::sha256_hex(task.as_bytes());
    let plan_hash = plan["launch_plan_hash"].as_str().unwrap();
    let report = format!(
        "# 任务交付报告\n\
Closure schema: 1.1\n\
Contract ID: tc-0123456789abcdef\n\
task-card-hash: {task_hash}\n\
launch-plan-hash: {plan_hash}\n\
execution-mode-used: single-writer\n\
execution-topology-used: single\n\
delegation-used: none\n\
状态: completed\n\
review-gate: passed\n\
## 目标闭环\n- G-01: done — 已完成\n\
## 验收闭环\n- AC-01: pass — evidence: closure validator passed\n\
## 验证闭环\n- V-01: pass — cargo test exit 0\n\
## 改动与边界\n- changed: test\n\
## 未闭环项\n- none\n"
    );
    let closure = ags_evidence::delivery_report::validate(task, &plan_text, &report);
    assert!(closure.valid, "{:#?}", closure.checks);
    let task_path = root.join("task.md");
    let plan_path = root.join("launch-plan.json");
    let report_path = root.join("delivery-report.md");
    std::fs::write(&task_path, task).unwrap();
    std::fs::write(&plan_path, plan_text).unwrap();
    std::fs::write(&report_path, report).unwrap();
    (task_path, plan_path, report_path)
}

#[cfg(unix)]
fn write_valid_closure_receipt(root: &Path) -> (ags_evidence::Receipt, PathBuf) {
    let (task_path, plan_path, report_path) = write_valid_closure_artifacts(root);
    let task = std::fs::read_to_string(&task_path).unwrap();
    let plan = std::fs::read_to_string(&plan_path).unwrap();
    let report = std::fs::read_to_string(&report_path).unwrap();
    let closure = ags_evidence::delivery_report::validate(&task, &plan, &report);
    let receipt = ags_evidence::generate_closed_receipt(
        &task_path,
        &plan_path,
        &report_path,
        &closure,
        Vec::new(),
        None,
    );
    let receipt_path = root
        .join(".ags/evidence")
        .join(format!("{}.json", receipt.receipt_id));
    std::fs::create_dir_all(receipt_path.parent().unwrap()).unwrap();
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    (receipt, receipt_path)
}

#[cfg(unix)]
fn write_canonical_closure_pointer(
    root: &Path,
    receipt: &ags_evidence::Receipt,
    receipt_path: &Path,
) -> PathBuf {
    let receipt_bytes = std::fs::read(receipt_path).unwrap();
    let pointer_path = root
        .join(".ags/state/closure-pointers")
        .join(format!("{}.json", receipt.receipt_id));
    std::fs::create_dir_all(pointer_path.parent().unwrap()).unwrap();
    std::fs::write(
        &pointer_path,
        serde_json::to_vec_pretty(&crate::workspace_lifecycle::ClosurePointer {
            schema_version: crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: Some(root.display().to_string()),
            workspace_identity: Some(crate::workspace_lifecycle::workspace_identity(root)),
            receipt_id: receipt.receipt_id.clone(),
            receipt_path: receipt_path.display().to_string(),
            receipt_sha256: sha256(&receipt_bytes),
            task_card_hash: receipt.task_card_hash.clone(),
            launch_plan_hash: receipt.launch_plan_hash.clone(),
            delivery_report_hash: receipt.delivery_report_hash.clone(),
            authority_key_id: String::new(),
            authority_seal: String::new(),
        })
        .unwrap(),
    )
    .unwrap();
    pointer_path
}

#[cfg(unix)]
fn write_signed_closure_pointer(
    root: &Path,
    receipt: &ags_evidence::Receipt,
    receipt_path: &Path,
    machine_key: &[u8; 32],
) -> PathBuf {
    let receipt_bytes = std::fs::read(receipt_path).unwrap();
    let pointer_path = root
        .join(".ags/state/closure-pointers")
        .join(format!("{}.json", receipt.receipt_id));
    std::fs::create_dir_all(pointer_path.parent().unwrap()).unwrap();
    let mut pointer = crate::workspace_lifecycle::ClosurePointer {
        schema_version: crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
        canonical_workspace: Some(root.display().to_string()),
        workspace_identity: Some(crate::workspace_lifecycle::workspace_identity(root)),
        receipt_id: receipt.receipt_id.clone(),
        receipt_path: receipt_path.display().to_string(),
        receipt_sha256: sha256(&receipt_bytes),
        task_card_hash: receipt.task_card_hash.clone(),
        launch_plan_hash: receipt.launch_plan_hash.clone(),
        delivery_report_hash: receipt.delivery_report_hash.clone(),
        authority_key_id: String::new(),
        authority_seal: String::new(),
    };
    crate::workspace_lifecycle::seal_closure_pointer(machine_key, &mut pointer).unwrap();
    std::fs::write(&pointer_path, serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();
    pointer_path
}

fn production_lifecycle_failure_case(
    status: HostOutcomeStatus,
    unprovable_extra: bool,
    unreported_pointer_delete: bool,
) -> ApplyResult {
    let (workspace, runtime, mut plane, binding, session) = production_fixture();
    initialize_closure_authority(&mut plane, &binding, &session);
    let root = workspace.path().canonicalize().unwrap();
    let (task_path, plan_path, report_path) = write_valid_closure_artifacts(&root);
    let task = std::fs::read_to_string(&task_path).unwrap();
    let launch_plan = std::fs::read_to_string(&plan_path).unwrap();
    let report = std::fs::read_to_string(&report_path).unwrap();
    let closure = ags_evidence::delivery_report::validate(&task, &launch_plan, &report);
    let receipt = ags_evidence::generate_closed_receipt(
        &task_path,
        &plan_path,
        &report_path,
        &closure,
        Vec::new(),
        None,
    );
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
    let receipt_path = root
        .join(".ags/evidence")
        .join(format!("{}.json", receipt.receipt_id));
    let pointer_path = root
        .join(".ags/state/closure-pointers")
        .join(format!("{}.json", receipt.receipt_id));
    std::fs::create_dir_all(receipt_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(pointer_path.parent().unwrap()).unwrap();
    std::fs::write(&receipt_path, &receipt_bytes).unwrap();
    let mut pointer = crate::workspace_lifecycle::ClosurePointer {
        schema_version: crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
        canonical_workspace: Some(root.display().to_string()),
        workspace_identity: Some(crate::workspace_lifecycle::workspace_identity(&root)),
        receipt_id: receipt.receipt_id.clone(),
        receipt_path: receipt_path.display().to_string(),
        receipt_sha256: sha256(&receipt_bytes),
        task_card_hash: receipt.task_card_hash.clone(),
        launch_plan_hash: receipt.launch_plan_hash.clone(),
        delivery_report_hash: receipt.delivery_report_hash.clone(),
        authority_key_id: String::new(),
        authority_seal: String::new(),
    };
    let machine_key: [u8; 32] = std::fs::read(runtime.path().join("closure-authority-v1.key"))
        .unwrap()
        .try_into()
        .unwrap();
    crate::workspace_lifecycle::seal_closure_pointer(&machine_key, &mut pointer).unwrap();
    std::fs::write(&pointer_path, serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();
    let request = LifecycleSessionEndRequest {
        context: OperationContext::default(),
        host_id: "hermes".to_string(),
        host_session_id: "session-terminal".to_string(),
        event_id: "event-terminal".to_string(),
    };
    let decision = plane
        .decide(
            &session,
            OperationRequest::HostLifecycleSessionEnd(request.clone()),
        )
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let mut observed_write_set = Vec::new();
    let mut artifacts = Vec::new();
    if unreported_pointer_delete {
        std::fs::remove_file(&pointer_path).unwrap();
    }
    if unprovable_extra {
        let extra = root.join("unsealed-lifecycle-write");
        observed_write_set.push(extra.display().to_string());
        artifacts.push(HostWriteArtifact {
            path: extra.display().to_string(),
            state: HostArtifactState::Present {
                sha256: sha256("unprovable"),
            },
        });
    }
    let output_digest = sha256("lifecycle-terminal-failure");
    let evidence = serde_json::json!({
        "schema_version": "ags://schema/contract/v2/lifecycle-host-outcome",
        "event_id": request.event_id,
        "receipt_ids": [receipt.receipt_id],
        "observed_write_set": observed_write_set,
        "consumed_pointer_paths": [],
        "output_digest": output_digest,
        "completed": false,
    });
    let evidence_bytes = serde_json::to_vec(&evidence).unwrap();
    let host_receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: plane.outcome_grants[&action_ref].instruction_digest.clone(),
        outcome_token: issued.outcome_token.unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status,
        output_digest,
        observed_write_set,
        artifacts,
        evidence: Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::LifecycleReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: "memory://lifecycle-terminal-failure".to_string(),
                sha256: sha256(&evidence_bytes),
            },
            content_hex: hex_bytes(&evidence_bytes),
        }),
    };
    let bytes = serde_json::to_vec(&host_receipt).unwrap();
    plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://lifecycle-terminal-outcome".to_string(),
                        sha256: sha256(&bytes),
                    },
                    bytes,
                )),
            },
        )
        .unwrap()
}

#[derive(Clone)]
struct UpdatePayloadFixture {
    files: Vec<(String, Vec<u8>, u32)>,
    manifest_bytes: Vec<u8>,
    manifest_sha256: String,
    tree_digest: String,
}

fn prepare_update_candidate(runtime: &Path, version: &str) -> UpdatePayloadFixture {
    #[derive(Serialize)]
    struct Member {
        name: String,
        sha256: String,
        size: u64,
        mode: u32,
    }
    #[derive(Serialize)]
    struct Manifest<'a> {
        schema_version: &'a str,
        version: &'a str,
        tree_digest: &'a str,
        members: &'a [Member],
    }
    let files = vec![
        ("ags".to_string(), b"ags-0.4.20".to_vec(), 0o755),
        ("ags-mcp".to_string(), b"ags-mcp-0.4.20".to_vec(), 0o755),
        ("ags-host".to_string(), b"ags-host-0.4.20".to_vec(), 0o755),
        (
            "ags-launcher.js".to_string(),
            b"ags-launcher-0.4.20".to_vec(),
            0o644,
        ),
        (
            "release-metadata.json".to_string(),
            br#"{"version":"0.4.20","channel":"stable"}"#.to_vec(),
            0o644,
        ),
    ];
    let candidate = runtime.join("update-candidates").join(version);
    std::fs::create_dir_all(&candidate).unwrap();
    for (name, bytes, _mode) in &files {
        let path = candidate.join(name);
        std::fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(*_mode)).unwrap();
        }
    }
    let mut members = files
        .iter()
        .map(|(name, bytes, mode)| Member {
            name: name.clone(),
            sha256: sha256(bytes),
            size: bytes.len() as u64,
            mode: *mode,
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| left.name.cmp(&right.name));
    let tree_digest = sha256(serde_json::to_vec(&members).unwrap());
    let manifest_bytes = serde_json::to_vec_pretty(&Manifest {
        schema_version: "ags://schema/contract/v2/sealed-release-manifest",
        version,
        tree_digest: &tree_digest,
        members: &members,
    })
    .unwrap();
    UpdatePayloadFixture {
        files,
        manifest_sha256: sha256(&manifest_bytes),
        manifest_bytes,
        tree_digest,
    }
}

fn init_request() -> OperationRequest {
    OperationRequest::Init(InitRequest {
        context: OperationContext::default(),
        migration: MigrationMode::ExactOwnedOnly,
    })
}

fn json_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            pending.extend(
                std::fs::read_dir(path)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path()),
            );
        } else if metadata.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            files.push(path);
        }
    }
    files
}

#[test]
fn cli_and_mcp_share_semantic_plan_and_receipt_hashes_but_not_action_seals() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("allowed")).unwrap();
    let root = temp.path().canonicalize().unwrap();
    let state = Arc::new(Mutex::new(FakeState {
        apply_succeeds: true,
        verify_succeeds: true,
        recover_succeeds: true,
        effect_started: true,
        ..FakeState::default()
    }));
    let adapter = || FakeAdapter {
        state: Arc::clone(&state),
        read_root: root.clone(),
    };
    let mcp = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    let cli = AuthenticatedBinding::cli(
        "ags-cli",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "workspace-service-a",
        vec![root.clone()],
    );
    let mut mcp_plane = ControlPlane::with_sealing_key(adapter(), sha256("parity-key"));
    let mut cli_plane = ControlPlane::with_sealing_key(adapter(), sha256("parity-key"));
    let open = |plane: &mut ControlPlane<FakeAdapter>, binding| {
        plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("policy"),
            })
            .unwrap()
    };
    let mcp_session = open(&mut mcp_plane, mcp);
    let cli_session = open(&mut cli_plane, cli);
    let mcp_decision = mcp_plane
        .decide(&mcp_session, transaction_request())
        .unwrap();
    let cli_decision = cli_plane
        .decide(&cli_session, transaction_request())
        .unwrap();
    assert_eq!(
        mcp_decision.plan.as_ref().unwrap().plan_hash,
        cli_decision.plan.as_ref().unwrap().plan_hash
    );
    assert_eq!(
        mcp_decision.plan.as_ref().unwrap().payload_hash,
        cli_decision.plan.as_ref().unwrap().payload_hash
    );
    assert_eq!(
        mcp_decision.plan.as_ref().unwrap().policy_hash,
        cli_decision.plan.as_ref().unwrap().policy_hash
    );
    assert_eq!(
        mcp_decision.plan.as_ref().unwrap().operation,
        cli_decision.plan.as_ref().unwrap().operation
    );
    assert_ne!(
        mcp_decision.plan.as_ref().unwrap().binding_hash,
        cli_decision.plan.as_ref().unwrap().binding_hash
    );
    assert_ne!(mcp_decision.action_ref, cli_decision.action_ref);

    let read = OperationRequest::Schema(SchemaRequest {
        context: OperationContext::default(),
        operation: Some("setup".to_string()),
    });
    let mcp_receipt = mcp_plane
        .decide(&mcp_session, read.clone())
        .unwrap()
        .receipt
        .unwrap();
    let cli_receipt = cli_plane
        .decide(&cli_session, read)
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(mcp_receipt.plan_hash, cli_receipt.plan_hash);
    assert_eq!(mcp_receipt.receipt_id, cli_receipt.receipt_id);
}

#[test]
fn receipt_identity_binds_nested_evidence() {
    let first = receipt_with_evidence(
        OperationName::Test,
        ReceiptStatus::Succeeded,
        &sha256("plan"),
        &sha256("payload"),
        &sha256("binding"),
        &sha256("output"),
        Vec::new(),
        false,
        Some(serde_json::json!({"commit": "one", "argv_hash": "a"})),
    );
    let second = receipt_with_evidence(
        OperationName::Test,
        ReceiptStatus::Succeeded,
        &sha256("plan"),
        &sha256("payload"),
        &sha256("binding"),
        &sha256("output"),
        Vec::new(),
        false,
        Some(serde_json::json!({"commit": "two", "argv_hash": "a"})),
    );
    assert_ne!(first.receipt_id, second.receipt_id);
}

fn test_request(_root: &Path) -> OperationRequest {
    OperationRequest::Test(TestRequest {
        context: OperationContext::default(),
        profile: TestProfile::Smoke,
        executor: TestExecutor::Local,
    })
}

fn authenticated_host_outcome(
    binding: AuthenticatedBinding,
    action_ref: &str,
    plan: &SealedPlan,
    issued: &ApplyResult,
    instruction_digest: &str,
    status: HostOutcomeStatus,
) -> AuthenticatedHostOutcome {
    let receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.to_string(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: instruction_digest.to_string(),
        outcome_token: issued.outcome_token.clone().unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status,
        output_digest: sha256("host-outcome"),
        observed_write_set: Vec::new(),
        artifacts: Vec::new(),
        evidence: None,
    };
    let bytes = serde_json::to_vec(&receipt).unwrap();
    AuthenticatedHostOutcome::from_artifact(
        binding,
        ContentAddressedArtifactRef {
            uri: format!("memory://host-outcome/{action_ref}"),
            sha256: sha256(&bytes),
        },
        bytes,
    )
}

fn read_details_bytes<A: EffectAdapter>(
    plane: &mut ControlPlane<A>,
    session: &OpenedSession,
    details: &DetailsReference,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut offset = 0;
    loop {
        let decision = plane
            .decide(
                session,
                OperationRequest::DetailsRead(DetailsReadRequest {
                    context: OperationContext::default(),
                    artifact: ContentAddressedArtifactRef {
                        uri: details.details_uri.clone(),
                        sha256: details.sha256.clone(),
                    },
                    offset,
                    max_bytes: DETAILS_CHUNK_LIMIT,
                }),
            )
            .unwrap();
        let chunk: DetailsChunk = serde_json::from_value(decision.result.unwrap()).unwrap();
        bytes.extend(decode_hex_evidence(&chunk.data).unwrap());
        offset = chunk.next_offset;
        if chunk.eof {
            break;
        }
    }
    assert_eq!(bytes.len() as u64, details.byte_length);
    assert_eq!(sha256(&bytes), details.sha256);
    bytes
}

#[test]
fn host_delegated_first_apply_issues_one_bound_canonical_instruction() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let decision = plane.decide(&session, host_request()).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();

    assert_eq!(issued.state, OperationState::AwaitingOutcome);
    assert!(
        issued.outcome_deadline_unix_ms.unwrap() >= now_unix_ms().saturating_add(10 * 60 * 1000)
    );
    let retried = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(retried, issued, "an unconsumed sealed grant is idempotent");
    let details = issued.details.as_ref().expect("typed instruction details");
    let bytes = read_details_bytes(&mut plane, &session, details);
    let instruction: HostExecutionInstruction = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        instruction.schema_version,
        HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION
    );
    assert_eq!(instruction.action_ref, action_ref);
    assert_eq!(instruction.binding_hash, plan.binding_hash);
    assert_eq!(instruction.plan_hash, plan.plan_hash);
    assert_eq!(instruction.policy_hash, plan.policy_hash);
    assert_eq!(
        instruction.instruction_digest,
        canonical_host_execution_instruction_digest(&instruction, &plan.action_digest).unwrap()
    );
    assert!(ags_platform::is_sha256(&instruction.instruction_digest));
    assert_eq!(
        instruction.action,
        HostExecutionAction::Command {
            profile: TestProfile::Smoke,
            program: "true".to_string(),
            argv: Vec::new(),
            cwd: binding.canonical_workspace.clone(),
            env: Default::default(),
            timeout_ms: 1000,
            allowed_write_paths: plan
                .expected_write_paths
                .iter()
                .map(PathBuf::from)
                .collect(),
        }
    );

    let wrong_binding = AuthenticatedBinding::mcp(
        "instruction-cross-binding",
        "hermes",
        binding.canonical_workspace.clone(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "instruction-cross-binding-session",
        binding.authorized_write_roots.clone(),
    );
    let wrong_session = plane
        .open(OpenRequest {
            binding: wrong_binding,
            policy_hash: sha256("policy"),
        })
        .unwrap();
    let error = plane
        .decide(
            &wrong_session,
            OperationRequest::DetailsRead(DetailsReadRequest {
                context: OperationContext::default(),
                artifact: ContentAddressedArtifactRef {
                    uri: details.details_uri.clone(),
                    sha256: details.sha256.clone(),
                },
                offset: 0,
                max_bytes: DETAILS_CHUNK_LIMIT,
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, "details_artifact_cross_binding");
}

#[test]
fn cli_binding_reuses_session_and_can_read_issued_instruction() {
    let (temp, mut plane, _state, _binding, _session) = fixture();
    let canonical = temp.path().canonicalize().unwrap();
    let binding = AuthenticatedBinding::cli(
        "codex-local",
        canonical.clone(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "service-a",
        vec![canonical],
    );
    let first = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("policy"),
        })
        .unwrap();
    let action_ref = plane
        .decide(&first, host_request())
        .unwrap()
        .action_ref
        .unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();

    let reopened = plane
        .open(OpenRequest {
            binding,
            policy_hash: sha256("policy"),
        })
        .unwrap();
    assert_eq!(reopened.session_ref, first.session_ref);
    let bytes = read_details_bytes(&mut plane, &reopened, issued.details.as_ref().unwrap());
    assert!(serde_json::from_slice::<HostExecutionInstruction>(&bytes).is_ok());
}

#[test]
fn host_instruction_contract_is_closed_bounded_and_has_no_prose_or_shell_escape_hatches() {
    let schema = serde_json::to_vec(&schemars::schema_for!(HostExecutionInstruction)).unwrap();
    assert!(
        schema.len() <= 16 * 1024,
        "schema is {} bytes",
        schema.len()
    );
    let schema_text = String::from_utf8(schema).unwrap();
    for forbidden in ["natural_language", "prompt", "shell_command"] {
        assert!(
            !schema_text.contains(forbidden),
            "forbidden field: {forbidden}"
        );
    }

    let unknown_instruction = serde_json::json!({
        "schema_version": HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION,
        "action_ref": "action-v2:test",
        "binding_hash": sha256("binding"),
        "plan_hash": sha256("plan"),
        "policy_hash": sha256("policy"),
        "instruction_digest": sha256("instruction"),
        "action": {
            "kind": "command",
            "profile": "smoke",
            "program": "true",
            "argv": [],
            "cwd": "/tmp/workspace",
            "env": {},
            "timeout_ms": 1000,
            "allowed_write_paths": [],
        },
        "surprise": true,
    });
    assert!(serde_json::from_value::<HostExecutionInstruction>(unknown_instruction).is_err());

    let unknown_action = serde_json::json!({
        "kind": "command",
        "profile": "smoke",
        "program": "true",
        "argv": [],
        "cwd": "/tmp/workspace",
        "env": {},
        "timeout_ms": 1000,
        "allowed_write_paths": [],
        "shell_command": "true && touch escaped",
    });
    assert!(serde_json::from_value::<HostExecutionAction>(unknown_action).is_err());
}

#[test]
fn wrong_instruction_digest_rejects_without_consuming_the_outcome_grant() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let decision = plane.decide(&session, host_request()).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let correct_digest = plane.outcome_grants[&action_ref].instruction_digest.clone();
    let wrong_receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: sha256("wrong-instruction"),
        outcome_token: issued.outcome_token.clone().unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status: HostOutcomeStatus::Succeeded,
        output_digest: sha256("host-outcome"),
        observed_write_set: Vec::new(),
        artifacts: Vec::new(),
        evidence: None,
    };
    let wrong_bytes = serde_json::to_vec(&wrong_receipt).unwrap();
    let error = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://wrong-instruction-outcome".to_string(),
                        sha256: sha256(&wrong_bytes),
                    },
                    wrong_bytes,
                )),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "host_outcome_instruction_digest_mismatch");
    assert!(!plane.outcome_grants[&action_ref].consumed);
    assert!(plane.actions.contains_key(&action_ref));

    let correct = authenticated_host_outcome(
        binding.clone(),
        &action_ref,
        &plan,
        &issued,
        &correct_digest,
        HostOutcomeStatus::Succeeded,
    );
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: Some(correct),
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::Receipted);
    assert!(!plane.actions.contains_key(&action_ref));
}

#[test]
fn registry_is_single_typed_contract_and_unknown_fields_fail() {
    assert_eq!(operation_registry().len(), 26);
    let names = operation_registry()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(names.len(), operation_registry().len());
    let union = operation_request_schema();
    assert!(
        union.get("$defs").is_some(),
        "schema must be self-contained"
    );
    let encoded = serde_json::to_vec(&union).unwrap();
    assert!(
        encoded.len() <= 8192,
        "MCP operation schema is {} bytes",
        encoded.len()
    );
    assert!(
        encoded
            .windows(b"\"additionalProperties\":false".len())
            .any(|window| window == b"\"additionalProperties\":false"),
        "strict object schemas must retain additionalProperties=false"
    );
    assert!(operation_registry().iter().all(|spec| {
        spec.request_schema_id
            .starts_with("ags://schema/contract/v2/")
    }));
    let install = operation_registry()
        .iter()
        .find(|spec| spec.name == OperationName::GovernSkillInstall)
        .expect("govern skill install is canonical");
    assert_eq!(install.name.as_str(), "govern.skill.install");
    assert_eq!(install.cli_path, "govern skill install");
    assert_eq!(
        install.request_schema_id,
        "ags://schema/contract/v2/skill-install-request"
    );
    for retired in [
        concat!("GovernSkill", "Adopt"),
        concat!("Skill", "AdoptRequest"),
        concat!("govern.skill.", "adopt"),
        concat!("govern skill ", "adopt"),
        concat!("skill-", "adopt-request"),
    ] {
        assert!(
            !include_str!("../control_plane.rs").contains(retired)
                && !include_str!("production.rs").contains(retired),
            "retired Skill adoption token remains in production source: {retired}"
        );
    }
    let unknown = serde_json::json!({
        "operation": "update",
        "request": { "context": {}, "channel": "stable", "surprise": true }
    });
    assert!(serde_json::from_value::<OperationRequest>(unknown).is_err());
}

#[test]
fn details_read_requires_original_authenticated_session_and_digest() {
    let (temp, mut plane, state, _binding, session) = fixture();
    state.lock().unwrap().read_result = Some(serde_json::json!({
        "evidence": "x".repeat(DETAILS_INLINE_LIMIT + 1024)
    }));
    let decision = plane
        .decide(
            &session,
            OperationRequest::Schema(SchemaRequest {
                context: OperationContext::default(),
                operation: None,
            }),
        )
        .unwrap();
    let reference = decision.result.unwrap();
    let artifact = ContentAddressedArtifactRef {
        uri: reference["details_uri"].as_str().unwrap().to_string(),
        sha256: reference["sha256"].as_str().unwrap().to_string(),
    };
    let request = OperationRequest::DetailsRead(DetailsReadRequest {
        context: OperationContext::default(),
        artifact: artifact.clone(),
        offset: 0,
        max_bytes: 128,
    });
    let chunk = plane.decide(&session, request.clone()).unwrap();
    let chunk: DetailsChunk = serde_json::from_value(chunk.result.unwrap()).unwrap();
    assert_eq!(chunk.artifact, artifact);
    assert_eq!(chunk.offset, 0);
    assert_eq!(chunk.next_offset, 128);
    assert!(!chunk.eof);
    assert_eq!(chunk.data.len(), 256);

    let wrong_binding = AuthenticatedBinding::mcp(
        "connection-b",
        "hermes",
        temp.path().canonicalize().unwrap(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-b",
        vec![temp.path().canonicalize().unwrap()],
    );
    let wrong_session = plane
        .open(OpenRequest {
            binding: wrong_binding,
            policy_hash: sha256("policy"),
        })
        .unwrap();
    assert_eq!(
        plane
            .decide(&wrong_session, request.clone())
            .unwrap_err()
            .code,
        "details_artifact_cross_binding"
    );

    let mut tampered = request;
    let OperationRequest::DetailsRead(details) = &mut tampered else {
        unreachable!()
    };
    details.artifact.sha256 = sha256("wrong");
    assert_eq!(
        plane.decide(&session, tampered).unwrap_err().code,
        "details_artifact_digest_mismatch"
    );
}

#[test]
fn oversized_apply_receipt_is_externalized_and_action_bound() {
    let (temp, mut plane, state, binding, session) = fixture();
    state.lock().unwrap().effect_evidence = Some(serde_json::json!({
        "trace": "y".repeat(DETAILS_INLINE_LIMIT + 1024)
    }));
    let action_ref = plane
        .decide(&session, test_request(temp.path()))
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    assert!(result.receipt.is_none());
    assert!(!plane.actions.contains_key(&action_ref));
    assert!(!plane.outcome_grants.contains_key(&action_ref));
    let details = result.details.expect("bounded apply details reference");
    let chunk = plane
        .decide(
            &session,
            OperationRequest::DetailsRead(DetailsReadRequest {
                context: OperationContext::default(),
                artifact: ContentAddressedArtifactRef {
                    uri: details.details_uri,
                    sha256: details.sha256,
                },
                offset: 0,
                max_bytes: 64,
            }),
        )
        .unwrap();
    assert_eq!(chunk.state, OperationState::NoChange);
}

#[test]
fn full_details_cache_still_externalizes_maximum_legal_effect_receipt() {
    let (temp, mut plane, state, binding, session) = fixture();
    let binding_hash = plane
        .sessions
        .get(session.session_ref())
        .unwrap()
        .binding_hash
        .clone();
    let mut first_uri = None;
    for index in 0..(MAX_DETAILS_TOTAL_BYTES / MAX_DETAILS_ARTIFACT_BYTES) {
        let reference = plane
            .store_details(
                session.session_ref(),
                &binding_hash,
                None,
                vec![index as u8; MAX_DETAILS_ARTIFACT_BYTES],
            )
            .unwrap();
        first_uri.get_or_insert(reference.details_uri);
    }
    assert_eq!(plane.details_total_bytes, MAX_DETAILS_TOTAL_BYTES);
    state.lock().unwrap().effect_evidence = Some(serde_json::json!({
        "trace": "z".repeat(900 * 1024)
    }));
    let action_ref = plane
        .decide(&session, test_request(temp.path()))
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    let details = result.details.expect("terminal result must externalize");
    assert!(plane.details.contains_key(&details.details_uri));
    assert!(!plane.details.contains_key(first_uri.as_ref().unwrap()));
    assert!(plane.details_total_bytes <= MAX_DETAILS_TOTAL_BYTES);
}

#[test]
fn oversized_host_outcome_is_rejected_before_typed_verifier_or_consumption() {
    let (_temp, mut plane, state, binding, session) = fixture();
    let action_ref = plane
        .decide(&session, host_request())
        .unwrap()
        .action_ref
        .unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(issued.state, OperationState::AwaitingOutcome);
    let bytes = vec![b'x'; MAX_DETAILS_ARTIFACT_BYTES + 1];
    let error = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://oversized".to_string(),
                        sha256: sha256(&bytes),
                    },
                    bytes,
                )),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "host_outcome_artifact_too_large");
    assert_eq!(state.lock().unwrap().host_verify_calls, 0);
    assert!(plane.actions.contains_key(&action_ref));
    assert!(plane.outcome_grants.contains_key(&action_ref));
}

#[test]
fn lifecycle_events_are_canonical_typed_operations() {
    let names = operation_registry()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    for expected in [
        "host.lifecycle.session_start",
        "host.lifecycle.session_end",
        "host.lifecycle.stop_guard",
    ] {
        assert!(
            names.contains(expected),
            "missing canonical lifecycle Operation `{expected}`"
        );
    }
}

#[test]
fn production_session_end_without_closure_authority_fails_closed() {
    let (_workspace, _runtime, mut plane, _binding, session) = production_fixture();
    let error = plane
        .decide(
            &session,
            OperationRequest::HostLifecycleSessionEnd(LifecycleSessionEndRequest {
                context: OperationContext::default(),
                host_id: "hermes".to_string(),
                host_session_id: "no-pointer-session".to_string(),
                event_id: "no-pointer-event".to_string(),
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, "operation_plan_failed");
    assert!(error.detail.contains("closure_authority_unavailable"));
}

#[test]
fn registry_generates_adapter_surface_without_parallel_handler_metadata() {
    let encoded = serde_json::to_value(operation_registry()).unwrap();
    for spec in encoded.as_array().unwrap() {
        assert!(
            spec.get("adapter_surface").is_some(),
            "registry entry lacks generated adapter surface: {spec}"
        );
        assert!(
            spec.get("handler").is_none(),
            "registry entry must not carry a parallel production handler enum: {spec}"
        );
    }
}

#[test]
fn every_registry_fixture_reaches_a_real_production_dispatch_arm() {
    let (workspace, runtime, _plane, binding, _session) = production_fixture();
    let root = workspace.path().canonicalize().unwrap();
    let fixtures = vec![
        serde_json::json!({"operation":"setup","request":{"approved_hosts":[]}}),
        serde_json::json!({"operation":"init","request":{"migration":"none"}}),
        serde_json::json!({"operation":"agent.register","request":{"host_id":"hermes","surface":"hybrid"}}),
        serde_json::json!({"operation":"agent.probe","request":{"host_id":"hermes","surface":"hybrid"}}),
        serde_json::json!({"operation":"govern.host_projection","request":{"host_id":"hermes","mode":"reconcile"}}),
        serde_json::json!({"operation":"govern.capability.inventory","request":{"host_id":"hermes","include_inactive":true}}),
        serde_json::json!({"operation":"govern.skill.install","request":{"skill_id":"fixture-skill","source":{"kind":"local","uri":root.join("fixture-skill").display().to_string()},"target_hosts":["hermes"],"update_policy":"manual","risk_acknowledgements":[]}}),
        serde_json::json!({"operation":"govern.skill.remove","request":{"skill_id":"fixture-skill"}}),
        serde_json::json!({"operation":"govern.capability.snapshot","request":{"host_id":"hermes","replace_all":false}}),
        serde_json::json!({"operation":"govern.mcp.advice","request":{"mcp_id":"fixture-mcp"}}),
        serde_json::json!({"operation":"govern.task.validate","request":{"task_card_path":"task.md"}}),
        serde_json::json!({"operation":"govern.task.plan","request":{"task_card_path":"task.md"}}),
        serde_json::json!({"operation":"govern.task.close","request":{"task_card_path":"task.md","launch_plan_path":"launch-plan.json","delivery_report_path":"delivery-report.md"}}),
        serde_json::json!({"operation":"govern.policy","request":{"task_card_path":"task.md"}}),
        serde_json::json!({"operation":"govern.gate","request":{"task_card_path":"task.md"}}),
        serde_json::json!({"operation":"govern.evidence","request":{"artifact_kind":"receipt","path":"receipt.json"}}),
        serde_json::json!({"operation":"govern.memory.close","request":{"receipt_path":"receipt.json"}}),
        serde_json::json!({"operation":"update","request":{"channel":"stable","target_version":"0.4.20"}}),
        serde_json::json!({"operation":"doctor","request":{"scope":"all"}}),
        serde_json::json!({"operation":"check","request":{"scope":"governance"}}),
        serde_json::json!({"operation":"test","request":{"profile":"smoke","executor":"host"}}),
        serde_json::json!({"operation":"schema","request":{}}),
        serde_json::json!({"operation":"host.lifecycle.session_start","request":{"host_id":"hermes","host_session_id":"fixture-session","event_id":"fixture-event"}}),
        serde_json::json!({"operation":"host.lifecycle.session_end","request":{"host_id":"hermes","host_session_id":"fixture-session","event_id":"fixture-event"}}),
        serde_json::json!({"operation":"host.lifecycle.stop_guard","request":{"host_id":"hermes","host_session_id":"fixture-session","event_id":"fixture-event","last_assistant_message":"clear"}}),
        serde_json::json!({"operation":"details.read","request":{"artifact":{"uri":"ags://details/missing","sha256":sha256("missing")},"offset":0,"max_bytes":64}}),
    ];
    assert_eq!(fixtures.len(), operation_registry().len());
    let adapter = ProductionEffectAdapter::with_host_home(runtime.path(), &root);
    let mut dispatched = std::collections::BTreeSet::new();
    for fixture in fixtures {
        let operation: OperationRequest = serde_json::from_value(fixture).unwrap();
        assert_eq!(operation.name(), operation.spec().name);
        dispatched.insert(operation.name().as_str().to_string());
        let _ = adapter.read_only_roots(&operation, &binding);
        let result = if operation.kind() == OperationKind::ReadOnly {
            adapter.read(&operation, &binding).map(|_| ())
        } else {
            adapter.plan(&operation, &binding).map(|_| ())
        };
        if let Err(error) = result {
            assert_ne!(error.code, "operation_kind_dispatch_mismatch");
        }
    }
    assert_eq!(dispatched.len(), operation_registry().len());
}

#[test]
fn sealed_transaction_paths_preserve_absolute_authorization_semantics() {
    let (_workspace, _runtime, mut plane, binding, session) = production_fixture();
    let plan = plane
        .decide(&session, init_request())
        .unwrap()
        .plan
        .unwrap();
    assert!(!plan.expected_write_paths.is_empty());
    for path in plan.expected_write_paths {
        let path = Path::new(&path);
        assert!(path.is_absolute(), "public write-set path must be absolute");
        assert!(path.starts_with(binding.canonical_workspace()));
        assert!(!path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)));
    }
}

#[cfg(unix)]
#[test]
fn parent_symlink_substitution_during_create_fails_closed_without_escape() {
    let (workspace, _runtime, mut plane, binding, session) = production_fixture();
    let action_ref = plane
        .decide(&session, init_request())
        .unwrap()
        .action_ref
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::rename(
        workspace.path().join("config"),
        workspace.path().join("config-original"),
    )
    .unwrap();
    symlink(outside.path(), workspace.path().join("config")).unwrap();

    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();

    assert_ne!(
        result.receipt.as_ref().map(|receipt| receipt.status),
        Some(ReceiptStatus::Succeeded)
    );
    assert!(result.transitions.contains(&OperationState::Recovering));
    assert!(!outside.path().join("agent-project-profile.yaml").exists());
}

#[test]
fn transaction_persists_identity_bound_integrity_checked_journal() {
    let (_workspace, runtime, mut plane, binding, session) = production_fixture();
    let decision = plane
        .decide(
            &session,
            OperationRequest::Setup(SetupRequest {
                context: OperationContext::default(),
                approved_hosts: vec!["hermes".to_string()],
            }),
        )
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let _result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();

    let journals = json_files(runtime.path())
        .into_iter()
        .filter_map(|path| {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            (value.get("action_ref").and_then(serde_json::Value::as_str)
                == Some(action_ref.as_str()))
            .then_some(value)
        })
        .collect::<Vec<_>>();
    assert_eq!(journals.len(), 1, "exactly one durable action journal");
    let journal = &journals[0];
    assert_eq!(journal["plan_hash"], plan.plan_hash);
    assert_eq!(journal["binding_hash"], plan.binding_hash);
    assert_eq!(journal["policy_hash"], plan.policy_hash);
    assert!(journal["ordered_writes"]
        .as_array()
        .is_some_and(|writes| !writes.is_empty()));
    assert!(journal["integrity"]
        .as_str()
        .is_some_and(ags_platform::is_sha256));
}

#[test]
fn host_delegated_first_apply_returns_awaiting_state_and_token() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let operation = host_request();
    let action_ref = plane
        .decide(&session, operation)
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["state"], "awaiting-outcome");
    assert!(value["outcome_token"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
}

#[test]
fn lifecycle_module_contains_no_direct_writer_or_archiver() {
    let source = include_str!("../workspace_lifecycle.rs");
    for forbidden in [
        "ags_platform::atomic_write(",
        "std::fs::remove_file(",
        "ags_evidence::memory::archive(",
        "struct LifecycleKernel",
        "impl LifecycleKernel",
        "fn process(",
    ] {
        assert!(
            !source.contains(forbidden),
            "lifecycle bypasses sealed apply through `{forbidden}`"
        );
    }
}

#[test]
fn public_seam_hides_adapter_mechanics() {
    let library = include_str!("../lib.rs");
    assert!(!library.contains("pub mod control_plane;"));
    assert!(!library.contains("pub mod workspace_lifecycle;"));
    assert!(library.contains("pub struct ProductionControlPlane"));
}

#[test]
fn stable_read_callers_use_only_the_returned_stable_stat_for_metadata() {
    let core = include_str!("../control_plane.rs");
    let production = include_str!("production.rs");
    for required in [
        "device: stable.stable_stat.device",
        "inode: stable.stable_stat.inode",
        "mode: stable.stable_stat.mode",
        "size: stable.stable_stat.size",
    ] {
        assert!(core.contains(required), "host scanner omits `{required}`");
    }
    assert!(
        production.contains("mode: stable.stable_stat.mode & 0o777"),
        "release metadata must come from the stable read result"
    );
}

#[test]
fn details_read_is_registry_dispatched_and_narrowly_declared() {
    let source = include_str!("../control_plane.rs");
    assert!(source.contains("dispatch_control_plane"));
    assert!(
        !source.contains("if let OperationRequest::DetailsRead(request) = &operation"),
        "DetailsRead must be selected by the generated registry dispatcher"
    );
    let details = operation_registry()
        .iter()
        .find(|spec| spec.name == OperationName::DetailsRead)
        .unwrap();
    assert_eq!(
        details.adapter_surface,
        AdapterSurface::ControlPlaneInternal
    );
    assert!(details.cli_path.is_empty());
    assert_eq!(details.allowed_kinds, &[OperationKind::ReadOnly]);
}

#[test]
fn red_open_is_read_only_and_all_transactions_use_control_plane_recovery() {
    let core = include_str!("../control_plane.rs");
    let production = include_str!("production.rs");

    assert!(
        core.contains("inspect_pending"),
        "open must only inspect pending recovery state"
    );
    assert!(
        !core.contains(".recover_pending(&request.binding)"),
        "opening a session must never execute recovery writes"
    );
    assert!(
        !core.contains("AdapterManaged"),
        "every Transaction operation must use the control-plane journal"
    );
    assert!(
        !production.contains("canonical-projection-transaction-recovered"),
        "projection recovery cannot report success without checking the durable journal"
    );
    assert!(
        !production.contains("recover_applied_change_in_maintenance_transaction"),
        "skill recovery authority must not bypass the control-plane journal"
    );
}

#[test]
fn red_closure_pointer_requires_machine_authority_seal() {
    let lifecycle = include_str!("../workspace_lifecycle.rs");
    let production = include_str!("production.rs");

    assert!(
        lifecycle.contains("authority_key_id"),
        "closure pointer must identify the machine authority without exposing its key"
    );
    assert!(
        lifecycle.contains("authority_seal"),
        "closure pointer must carry a keyed, domain-separated authority seal"
    );
    assert!(
        production.contains("closure-authority-v1.key"),
        "production setup/apply must own the descriptor-confined machine key"
    );
}

#[test]
fn red_legacy_public_authorities_are_physically_removed() {
    let evidence = include_str!("../../../ags-evidence/src/lib.rs");
    let host = include_str!("../../../ags-host-integration/src/lib.rs");
    let workspace_facts = include_str!("../../../ags-workspace-facts/src/workspace_facts.rs");

    assert!(!evidence.contains("mod action;"));
    assert!(!evidence.contains("mod action_model;"));
    assert!(!evidence.contains("pub use action::*;"));
    assert!(!evidence.contains("pub use action_model::*;"));
    assert!(!host.contains("extract_profile_slug"));
    assert!(!workspace_facts.contains("use ags_host_integration::extract_profile_slug"));
}

#[test]
fn red_product_operation_schema_excludes_internal_surfaces() {
    let schema = serde_json::to_string(&operation_request_schema()).unwrap();
    for forbidden in [
        "host.lifecycle.session_start",
        "host.lifecycle.session_end",
        "host.lifecycle.stop_guard",
        "details.read",
    ] {
        assert!(
            !schema.contains(forbidden),
            "product ags_decide schema leaked internal operation `{forbidden}`"
        );
    }
}

#[cfg(unix)]
#[test]
fn red_self_consistent_forged_closure_is_rejected_by_all_consumers() {
    let (workspace, _runtime, mut plane, _binding, session) = production_fixture();
    let root = workspace.path().canonicalize().unwrap();
    let (receipt, receipt_path) = write_valid_closure_receipt(&root);
    write_canonical_closure_pointer(&root, &receipt, &receipt_path);

    let memory = plane.decide(
        &session,
        OperationRequest::GovernMemoryClose(MemoryCloseRequest {
            context: OperationContext::default(),
            receipt_path: receipt_path.display().to_string(),
        }),
    );
    assert!(
        memory.is_err(),
        "MemoryClose accepted an unkeyed forged closure"
    );

    let evidence = plane.decide(
        &session,
        OperationRequest::GovernEvidence(EvidenceRequest {
            context: OperationContext::default(),
            artifact_kind: EvidenceArtifactKind::Receipt,
            path: receipt_path.display().to_string(),
            task_card_path: None,
            launch_plan_path: None,
        }),
    );
    assert!(
        evidence.is_err()
            || evidence
                .as_ref()
                .ok()
                .and_then(|decision| decision.result.as_ref())
                .and_then(|result| result.get("valid"))
                .and_then(serde_json::Value::as_bool)
                == Some(false),
        "GovernEvidence accepted an unkeyed forged closure"
    );

    let lifecycle = plane.decide(
        &session,
        OperationRequest::HostLifecycleSessionEnd(LifecycleSessionEndRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "forged-session".to_string(),
            event_id: "forged-event".to_string(),
        }),
    );
    assert!(
        lifecycle.is_err(),
        "SessionEnd accepted an unkeyed forged closure"
    );
}

#[test]
fn read_only_is_no_change_and_tree_mutation_is_detected() {
    let (_temp, mut plane, state, _binding, session) = fixture();
    let request = OperationRequest::Schema(SchemaRequest {
        context: OperationContext::default(),
        operation: None,
    });
    let decision = plane.decide(&session, request.clone()).unwrap();
    assert_eq!(decision.state, OperationState::NoChange);
    assert!(decision.action_ref.is_none());

    state.lock().unwrap().read_mutates = true;
    assert_eq!(
        plane.decide(&session, request).unwrap_err().code,
        "read_only_write_detected"
    );
}

#[test]
fn mutation_snapshot_fails_closed_when_file_budget_is_exceeded() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("large.bin"), vec![0_u8; 33 * 1024 * 1024]).unwrap();
    let error = tree_digest(&[root.path().canonicalize().unwrap()]).unwrap_err();
    assert_eq!(
        error.code, "read_only_snapshot_budget_exceeded",
        "{}",
        error.detail
    );
}

#[test]
fn mutation_snapshot_treats_missing_ancestor_as_missing_root() {
    let root = tempfile::tempdir().unwrap();
    let missing = root
        .path()
        .canonicalize()
        .unwrap()
        .join("not-created")
        .join("profile.yaml");
    let digest = tree_digest(&[missing]).unwrap();
    assert!(digest.starts_with("sha256:"), "{digest}");
}

#[cfg(unix)]
#[test]
fn mutation_snapshot_digest_binds_symlink_target_bytes() {
    let root = tempfile::tempdir().unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink("target-a", &link).unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let first = tree_digest(std::slice::from_ref(&canonical_root)).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("target-b", &link).unwrap();
    let second = tree_digest(&[canonical_root]).unwrap();
    assert_ne!(
        first, second,
        "symlink retargeting must change the snapshot"
    );
}

#[test]
fn mutation_snapshot_enforces_name_byte_budget_during_enumeration() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..32 {
        let name = format!("{index:02}-{}", "x".repeat(200));
        std::fs::write(root.path().join(name), b"small").unwrap();
    }
    let error = tree_digest(&[root.path().canonicalize().unwrap()]).unwrap_err();
    assert_eq!(error.code, "read_only_snapshot_budget_exceeded");
    assert!(error.detail.contains("name bytes"), "{}", error.detail);
}

#[cfg(unix)]
#[test]
fn mutation_snapshot_uses_post_open_fstat_as_type_authority() {
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"regular-before-stat").unwrap();
    SNAPSHOT_AFTER_STAT_SUBSTITUTION.with(|slot| {
        *slot.borrow_mut() = Some((victim.clone(), PathBuf::from("victim")));
    });
    let error = tree_digest(&[root.path().canonicalize().unwrap()]).unwrap_err();
    assert_eq!(error.code, "read_only_snapshot_failed");
    assert!(error.detail.contains("post-open type"), "{}", error.detail);
}

#[cfg(unix)]
#[test]
fn production_capability_inventory_snapshot_rejects_oversize_and_special_files() {
    let (_workspace, runtime, mut plane, _binding, session) = production_fixture();
    let host_directory = runtime.path().join("hosts/hermes");
    std::fs::create_dir_all(&host_directory).unwrap();
    let oversized = host_directory.join("oversized.bin");
    std::fs::write(&oversized, vec![0_u8; 33 * 1024 * 1024]).unwrap();
    let request = || {
        OperationRequest::GovernCapabilityInventory(CapabilityInventoryRequest {
            context: OperationContext::default(),
            host_id: Some("hermes".to_string()),
            include_inactive: true,
        })
    };
    assert_eq!(
        plane.decide(&session, request()).unwrap_err().code,
        "read_only_snapshot_budget_exceeded"
    );
    std::fs::remove_file(oversized).unwrap();

    let fifo = host_directory.join("special.fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        plane.decide(&session, request()).unwrap_err().code,
        "read_only_snapshot_failed"
    );
    std::fs::remove_file(fifo).unwrap();
}

#[test]
fn transaction_apply_verify_receipt_and_replay_fail_closed() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let decision = plane.decide(&session, transaction_request()).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(
        result.transitions,
        vec![
            OperationState::Applying,
            OperationState::Verifying,
            OperationState::Receipted
        ]
    );
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None
                }
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );
}

#[test]
fn consumed_action_ref_is_rejected_before_effects() {
    let (_temp, mut plane, state, binding, session) = fixture();
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    plane.actions.get_mut(&action_ref).unwrap().consumed = true;

    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_replayed"
    );
    assert_eq!(state.lock().unwrap().apply_calls, 0);
}

#[test]
fn transaction_recovery_requires_reopen_and_local_failure_never_rolls_back_source() {
    let (temp, mut plane, state, binding, session) = fixture();
    state.lock().unwrap().verify_succeeds = false;
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
    assert_eq!(state.lock().unwrap().recover_calls, 1);

    state.lock().unwrap().apply_succeeds = false;
    assert_eq!(
        plane
            .decide(&session, test_request(temp.path()))
            .unwrap_err()
            .code,
        "session_unknown"
    );
    let session = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("policy"),
        })
        .unwrap();
    let action_ref = plane
        .decide(&session, test_request(temp.path()))
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Failed);
    assert_eq!(
        state.lock().unwrap().recover_calls,
        1,
        "LocalExecution must not roll back source"
    );
}

#[test]
fn verified_recovery_calls_the_adapter_durable_finalizer() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("allowed")).unwrap();
    let root = temp.path().canonicalize().unwrap();
    let state = Arc::new(Mutex::new(FakeState {
        apply_succeeds: true,
        verify_succeeds: true,
        effect_started: true,
        recovery_action: true,
        pending_recovery: true,
        ..FakeState::default()
    }));
    let adapter = FakeAdapter {
        state: Arc::clone(&state),
        read_root: root.clone(),
    };
    let mut plane = ControlPlane::with_sealing_key(adapter, sha256("fake-recovery-key"));
    let binding = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    let session = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("fake-recovery-policy"),
        })
        .unwrap();
    let action_ref = session.pending_recovery_action_ref().unwrap().to_string();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
    assert_eq!(state.lock().unwrap().recovery_finalize_calls, 1);
}

#[test]
fn red_transaction_adapter_error_is_after_effect_once_adapter_was_invoked() {
    let (_temp, mut plane, state, binding, session) = fixture();
    {
        let mut state = state.lock().unwrap();
        state.apply_error = true;
        state.effect_started = false;
        state.recover_succeeds = true;
    }
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();

    assert_eq!(
        state.lock().unwrap().recover_calls,
        1,
        "an adapter Err cannot suppress recovery by claiming effect_started=false"
    );
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
}

#[test]
fn unexpected_writes_escalate_transaction_and_local_execution() {
    let (temp, mut plane, state, binding, session) = fixture();
    state.lock().unwrap().observed_writes = vec![temp.path().join("outside").display().to_string()];
    for request in [transaction_request(), test_request(temp.path())] {
        let action_ref = plane.decide(&session, request).unwrap().action_ref.unwrap();
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::RiskEscalated);
        assert_eq!(result.reason_code.as_deref(), Some("unexpected_write_set"));
    }
}

#[test]
fn oversized_local_effect_observation_is_explicitly_risk_escalated() {
    let (temp, mut plane, state, binding, session) = fixture();
    state.lock().unwrap().observed_writes = vec!["x".to_string(); 513];
    let action_ref = plane
        .decide(&session, test_request(temp.path()))
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert_eq!(
        result.reason_code.as_deref(),
        Some("effect_observation_contract_violation")
    );
    let receipt = result.receipt.unwrap();
    assert_eq!(receipt.status, ReceiptStatus::RiskEscalated);
    assert_eq!(receipt.output_digest, sha256("apply"));
    assert!(receipt
        .observed_write_set
        .iter()
        .any(|entry| entry.starts_with("ags://contract-violation/")));
}

#[test]
fn after_effect_error_preserves_observed_writes_and_escalates() {
    let (temp, mut plane, state, binding, session) = fixture();
    {
        let mut state = state.lock().unwrap();
        state.apply_error = true;
        state.observed_writes = vec![temp.path().join("outside").display().to_string()];
    }
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::RiskEscalated);
    let receipt = result.receipt.unwrap();
    assert_eq!(receipt.output_digest, sha256("apply-error"));
    assert_eq!(receipt.observed_write_set.len(), 1);
}

#[test]
fn transaction_recovery_failure_is_risk_escalated() {
    let (_temp, mut plane, state, binding, session) = fixture();
    {
        let mut state = state.lock().unwrap();
        state.verify_succeeds = false;
        state.recover_succeeds = false;
    }
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert_eq!(
        result.reason_code.as_deref(),
        Some("transaction_recovery_failed")
    );
}

#[test]
fn daemon_restart_invalidates_in_memory_action_ref() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let action_ref = plane
        .decide(&session, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    let replacement = FakeAdapter {
        state: Arc::new(Mutex::new(FakeState::default())),
        read_root: binding.canonical_workspace().to_path_buf(),
    };
    let mut restarted = ControlPlane::with_sealing_key(replacement, sha256("new-daemon-key"));
    assert_eq!(
        restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );
}

#[test]
fn action_ref_rejects_tamper_cross_connection_host_and_workspace() {
    let (temp, mut plane, _state, binding_a, session_a) = fixture();
    let action_ref = plane
        .decide(&session_a, transaction_request())
        .unwrap()
        .action_ref
        .unwrap();
    assert_eq!(
        plane
            .apply(
                &binding_a,
                ApplyRequest {
                    action_ref: format!("{action_ref}x"),
                    outcome: None
                }
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );

    for (connection, host, workspace_identity, session_id) in [
        ("connection-b", "hermes", "workspace-a", "session-b"),
        ("connection-a", "codex", "workspace-a", "session-c"),
        ("connection-a", "hermes", "workspace-b", "session-d"),
    ] {
        let binding = AuthenticatedBinding::mcp(
            connection,
            host,
            temp.path().canonicalize().unwrap(),
            workspace_identity,
            sha256("facts-a"),
            "registry-a",
            session_id,
            vec![temp.path().canonicalize().unwrap()],
        );
        let _other = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy"),
            })
            .unwrap();
        assert_eq!(
            plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: action_ref.clone(),
                        outcome: None
                    }
                )
                .unwrap_err()
                .code,
            "action_ref_cross_binding"
        );
    }
}

#[test]
fn host_delegated_requires_authenticated_and_verifiable_artifacts() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let request = host_request();
    let decision = plane.decide(&session, request).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(issued.state, OperationState::AwaitingOutcome);

    let wrong = AuthenticatedBinding::mcp(
        "other",
        "hermes",
        binding.canonical_workspace.clone(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "other-session",
        binding.authorized_write_roots.clone(),
    );
    let outcome = authenticated_host_outcome(
        wrong,
        &action_ref,
        &plan,
        &issued,
        &plane.outcome_grants[&action_ref].instruction_digest,
        HostOutcomeStatus::Succeeded,
    );
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(outcome)
                }
            )
            .unwrap_err()
            .code,
        "host_outcome_cross_binding"
    );

    let outcome = authenticated_host_outcome(
        binding.clone(),
        &action_ref,
        &plan,
        &issued,
        &plane.outcome_grants[&action_ref].instruction_digest,
        HostOutcomeStatus::Succeeded,
    );
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: Some(outcome),
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::Receipted);
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );
}

#[test]
fn host_delegated_failed_outcome_without_physical_seal_risk_escalates() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let decision = plane.decide(&session, host_request()).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let outcome = authenticated_host_outcome(
        binding.clone(),
        &action_ref,
        &plan,
        &issued,
        &plane.outcome_grants[&action_ref].instruction_digest,
        HostOutcomeStatus::Failed,
    );
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(outcome),
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::RiskEscalated);
    let receipt = result.receipt.unwrap();
    assert_eq!(receipt.status, ReceiptStatus::RiskEscalated);
    assert_eq!(
        receipt.observed_write_set,
        vec!["ags://unprovable-host-delta/no-physical-seal"]
    );
}

#[test]
fn host_outcome_expiry_and_stale_generation_fail_closed() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let request = host_request();

    let decision = plane.decide(&session, request.clone()).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    plane
        .outcome_grants
        .get_mut(&action_ref)
        .unwrap()
        .deadline_unix_ms = 0;
    let expired = authenticated_host_outcome(
        binding.clone(),
        &action_ref,
        &plan,
        &issued,
        &plane.outcome_grants[&action_ref].instruction_digest,
        HostOutcomeStatus::Succeeded,
    );
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: Some(expired),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_token_expired"
    );

    let decision = plane.decide(&session, request).unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let newer_binding = AuthenticatedBinding::mcp(
        "connection-new",
        "hermes",
        binding.canonical_workspace.clone(),
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-new",
        binding.authorized_write_roots.clone(),
    );
    plane
        .open(OpenRequest {
            binding: newer_binding,
            policy_hash: sha256("policy"),
        })
        .unwrap();
    let stale = authenticated_host_outcome(
        binding.clone(),
        &action_ref,
        &plan,
        &issued,
        &plane.outcome_grants[&action_ref].instruction_digest,
        HostOutcomeStatus::Succeeded,
    );
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: Some(stale),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_generation_stale"
    );
}

#[test]
fn write_set_containment_rejects_relative_and_parent_components() {
    let root = std::env::temp_dir().join("ags-allowed");
    let allowed = vec![root.display().to_string()];
    assert!(!has_unexpected_writes(
        &allowed,
        &[root.join("child").display().to_string()]
    ));
    assert!(has_unexpected_writes(
        &allowed,
        &[root.join("../outside").display().to_string()]
    ));
    assert!(has_unexpected_writes(
        &allowed,
        &["relative/path".to_string()]
    ));
}

#[test]
fn host_outcome_boundary_accepts_only_one_content_addressed_receipt() {
    let typed = serde_json::json!({
        "receipt": {
            "uri": "file:///tmp/host-outcome.json",
            "sha256": sha256("host-outcome")
        }
    });
    assert!(serde_json::from_value::<HostOutcomeInput>(typed).is_ok());
    let loose = serde_json::json!({
        "status": "succeeded",
        "output_digest": sha256("outcome"),
        "observed_write_set": []
    });
    assert!(serde_json::from_value::<HostOutcomeInput>(loose).is_err());
}

#[cfg(unix)]
#[test]
fn host_postimage_verification_is_descriptor_relative_and_rejects_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("owned")).unwrap();
    std::fs::write(temp.path().join("owned/result.json"), b"verified").unwrap();
    let root = temp.path().canonicalize().unwrap();
    let binding = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    assert_eq!(
        descriptor_read_host_artifact(&binding, &root.join("owned/result.json"), 1024).unwrap(),
        Some(b"verified".to_vec())
    );

    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), b"outside").unwrap();
    std::fs::remove_file(root.join("owned/result.json")).unwrap();
    symlink(outside.path(), root.join("owned/result.json")).unwrap();
    assert!(
        descriptor_read_host_artifact(&binding, &root.join("owned/result.json"), 1024).is_err()
    );

    std::fs::remove_file(root.join("owned/result.json")).unwrap();
    std::fs::remove_dir(root.join("owned")).unwrap();
    symlink(outside.path().parent().unwrap(), root.join("owned")).unwrap();
    assert!(
        descriptor_read_host_artifact(&binding, &root.join("owned/result.json"), 1024).is_err()
    );

    std::fs::remove_file(root.join("owned")).unwrap();
    std::fs::create_dir(root.join("owned")).unwrap();
    let fifo = root.join("owned/result.fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let started = std::time::Instant::now();
    assert!(descriptor_read_host_artifact(&binding, &fifo, 1024).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn host_postimage_reader_rejects_same_inode_same_size_rewrite_during_read() {
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("owned")).unwrap();
    let path = root.join("owned/result.json");
    std::fs::write(&path, b"original").unwrap();
    let before = std::fs::metadata(&path).unwrap();
    let binding = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    STABLE_READ_SAME_INODE_REWRITE.with(|slot| {
        *slot.borrow_mut() = Some((path.clone(), b"tampered".to_vec()));
    });
    let result = descriptor_read_host_artifact(&binding, &path, 1024);
    STABLE_READ_SAME_INODE_REWRITE.with(|slot| {
        slot.borrow_mut().take();
    });
    assert_eq!(
        result.unwrap_err().code,
        "host_outcome_artifact_changed_during_read"
    );
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(
        before.ino(),
        after.ino(),
        "test must preserve inode identity"
    );
    assert_eq!(before.len(), after.len(), "test must preserve file size");
}

#[test]
fn test_executor_defaults_to_host_and_declares_both_allowed_kinds() {
    let host: OperationRequest = serde_json::from_value(serde_json::json!({
        "operation": "test",
        "request": {"profile": "smoke"}
    }))
    .unwrap();
    assert_eq!(host.kind(), OperationKind::HostDelegated);
    assert_eq!(
        host.spec().allowed_kinds,
        &[OperationKind::HostDelegated, OperationKind::LocalExecution]
    );
    let local = OperationRequest::Test(TestRequest {
        context: OperationContext::default(),
        profile: TestProfile::Smoke,
        executor: TestExecutor::Local,
    });
    assert_eq!(local.kind(), OperationKind::LocalExecution);
}

#[test]
fn agent_register_is_ags_owned_transaction_for_a_distinct_subject() {
    let spec = operation_registry()
        .iter()
        .find(|spec| spec.name == OperationName::AgentRegister)
        .unwrap();
    assert_eq!(spec.kind, OperationKind::Transaction);
    assert_eq!(spec.allowed_kinds, &[OperationKind::Transaction]);

    let (_workspace, runtime, mut plane, binding, session) = production_fixture();
    let decision = plane
        .decide(
            &session,
            OperationRequest::AgentRegister(AgentRegisterRequest {
                context: OperationContext::default(),
                host_id: "atlas-administered".to_string(),
                surface: AgentSurface::Hybrid,
            }),
        )
        .unwrap();
    let plan = decision.plan.unwrap();
    assert_eq!(plan.kind, OperationKind::Transaction);
    assert_eq!(plan.expected_write_paths.len(), 1);
    assert!(plan.expected_write_paths[0].ends_with("/hosts/atlas-administered/registration.json"));
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: decision.action_ref.unwrap(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, OperationState::Receipted);
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);

    let registration_path = runtime
        .path()
        .join("hosts/atlas-administered/registration.json");
    let registration: ags_host_integration::HostRegistration =
        serde_json::from_slice(&std::fs::read(&registration_path).unwrap()).unwrap();
    assert_eq!(registration.host_id.as_str(), "atlas-administered");
    assert!(registration.validate().is_ok());
}

#[test]
fn agent_register_rejects_a_legacy_host_outcome_generically() {
    let (_workspace, _runtime, mut plane, binding, session) = production_fixture();
    let decision = plane
        .decide(
            &session,
            OperationRequest::AgentRegister(AgentRegisterRequest {
                context: OperationContext::default(),
                host_id: "atlas-outcome".to_string(),
                surface: AgentSurface::Cli,
            }),
        )
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let legacy = br#"{"schema_version":"ags://schema/contract/v2/host-outcome"}"#.to_vec();
    let error = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://legacy-agent-host-outcome".to_string(),
                        sha256: sha256(&legacy),
                    },
                    legacy,
                )),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "unexpected_host_outcome");
}

#[test]
fn update_host_plan_closes_only_with_exact_typed_receipt() {
    let (_workspace, runtime, mut plane, binding, session) = production_fixture();
    let runtime_root = runtime.path().canonicalize().unwrap();
    let payload = prepare_update_candidate(&runtime_root, "0.4.20");
    let request = UpdateRequest {
        context: OperationContext::default(),
        channel: "stable".to_string(),
        target_version: Some("0.4.20".to_string()),
    };
    let decision = plane
        .decide(&session, OperationRequest::Update(request.clone()))
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    assert_eq!(plan.kind, OperationKind::HostDelegated);
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let instruction: HostExecutionInstruction = serde_json::from_slice(&read_details_bytes(
        &mut plane,
        &session,
        issued.details.as_ref().unwrap(),
    ))
    .unwrap();
    assert!(matches!(
        instruction.action,
        HostExecutionAction::RuntimeUpdate {
            ref channel,
            ref target_version,
            ref candidate_directory,
            ..
        } if channel == "stable" && target_version.as_deref() == Some("0.4.20")
            && candidate_directory == &runtime_root.join("update-candidates/0.4.20")
    ));
    let releases_directory = runtime_root.join("releases");
    let release_directory = releases_directory.join("0.4.20");
    std::fs::create_dir_all(&release_directory).unwrap();
    let mut file_targets = payload
        .files
        .iter()
        .map(|(name, bytes, mode)| (release_directory.join(name), bytes.clone(), *mode))
        .collect::<Vec<_>>();
    file_targets.extend([
        (
            release_directory.join("release-manifest.json"),
            payload.manifest_bytes.clone(),
            0o644,
        ),
        (
            runtime_root.join("current-release.json"),
            br#"{"version":"0.4.20"}"#.to_vec(),
            0o644,
        ),
        (
            runtime_root.join("update-state.json"),
            br#"{"version":"0.4.20","status":"installed"}"#.to_vec(),
            0o644,
        ),
    ]);
    for (path, bytes, _mode) in &file_targets {
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(*_mode)).unwrap();
        }
    }
    assert_eq!(
        plan.expected_write_paths,
        std::iter::once(releases_directory.display().to_string())
            .chain(std::iter::once(release_directory.display().to_string()))
            .chain(
                file_targets
                    .iter()
                    .map(|(path, _, _)| path.display().to_string())
            )
            .collect::<Vec<_>>()
    );
    let output_digest = sha256("update-output");
    let evidence = UpdateReceipt {
        schema_version: "ags://schema/contract/v2/update-receipt".to_string(),
        channel: request.channel,
        target_version: request.target_version,
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        observed_write_set: plan.expected_write_paths.clone(),
        release_manifest_sha256: payload.manifest_sha256.clone(),
        release_tree_digest: payload.tree_digest.clone(),
        output_digest: output_digest.clone(),
        completed: true,
    };
    let evidence_bytes = serde_json::to_vec(&evidence).unwrap();
    let host_receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: plane.outcome_grants[&action_ref].instruction_digest.clone(),
        outcome_token: issued.outcome_token.unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status: HostOutcomeStatus::Succeeded,
        output_digest,
        observed_write_set: plan.expected_write_paths.clone(),
        artifacts: std::iter::once(HostWriteArtifact {
            path: releases_directory.display().to_string(),
            state: HostArtifactState::Directory,
        })
        .chain(std::iter::once(HostWriteArtifact {
            path: release_directory.display().to_string(),
            state: HostArtifactState::Directory,
        }))
        .chain(
            file_targets
                .iter()
                .map(|(path, bytes, _)| HostWriteArtifact {
                    path: path.display().to_string(),
                    state: HostArtifactState::Present {
                        sha256: sha256(bytes),
                    },
                }),
        )
        .collect(),
        evidence: Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::UpdateReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: "memory://update-receipt".to_string(),
                sha256: sha256(&evidence_bytes),
            },
            content_hex: hex_bytes(&evidence_bytes),
        }),
    };
    let unexpected_path = release_directory.join("unsealed-helper");
    std::fs::write(&unexpected_path, b"unexpected").unwrap();
    let mut unexpected = host_receipt.clone();
    unexpected
        .observed_write_set
        .push(unexpected_path.display().to_string());
    unexpected.artifacts.push(HostWriteArtifact {
        path: unexpected_path.display().to_string(),
        state: HostArtifactState::Present {
            sha256: sha256(b"unexpected"),
        },
    });
    let unexpected_bytes = serde_json::to_vec(&unexpected).unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: "memory://unexpected-update-outcome".to_string(),
                            sha256: sha256(&unexpected_bytes),
                        },
                        unexpected_bytes,
                    )),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    std::fs::remove_file(unexpected_path).unwrap();

    let mut omitted = host_receipt.clone();
    omitted.observed_write_set.pop();
    omitted.artifacts.pop();
    let omitted_bytes = serde_json::to_vec(&omitted).unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: "memory://omitted-update-outcome".to_string(),
                            sha256: sha256(&omitted_bytes),
                        },
                        omitted_bytes,
                    )),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );

    let binary = release_directory.join("ags-host");
    std::fs::write(&binary, b"tampered").unwrap();
    let tampered_bytes = serde_json::to_vec(&host_receipt).unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: "memory://tampered-update-outcome".to_string(),
                            sha256: sha256(&tampered_bytes),
                        },
                        tampered_bytes,
                    )),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    std::fs::write(&binary, b"ags-host-0.4.20").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let manifest_path = release_directory.join("release-manifest.json");
        std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o777)).unwrap();
        let mode_bytes = serde_json::to_vec(&host_receipt).unwrap();
        assert_eq!(
            plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: action_ref.clone(),
                        outcome: Some(AuthenticatedHostOutcome::from_artifact(
                            binding.clone(),
                            ContentAddressedArtifactRef {
                                uri: "memory://manifest-mode-update-outcome".to_string(),
                                sha256: sha256(&mode_bytes),
                            },
                            mode_bytes,
                        )),
                    },
                )
                .unwrap_err()
                .code,
            "host_outcome_verification_failed"
        );
        std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let hidden_member = release_directory.join("physically-present-but-unreported");
    std::fs::write(&hidden_member, b"hidden").unwrap();
    let hidden_bytes = serde_json::to_vec(&host_receipt).unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: "memory://hidden-member-update-outcome".to_string(),
                            sha256: sha256(&hidden_bytes),
                        },
                        hidden_bytes,
                    )),
                },
            )
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    std::fs::remove_file(hidden_member).unwrap();

    let host_bytes = serde_json::to_vec(&host_receipt).unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://update-outcome".to_string(),
                        sha256: sha256(&host_bytes),
                    },
                    host_bytes,
                )),
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);
}

#[test]
fn production_failed_update_with_verified_empty_subset_closes_failed() {
    let (_workspace, runtime, mut plane, binding, session) = production_fixture();
    let payload = prepare_update_candidate(&runtime.path().canonicalize().unwrap(), "0.4.20");
    let request = UpdateRequest {
        context: OperationContext::default(),
        channel: "stable".to_string(),
        target_version: Some("0.4.20".to_string()),
    };
    let decision = plane
        .decide(&session, OperationRequest::Update(request.clone()))
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let output_digest = sha256("verified-update-failure");
    let evidence = UpdateReceipt {
        schema_version: "ags://schema/contract/v2/update-receipt".to_string(),
        channel: request.channel,
        target_version: request.target_version,
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        observed_write_set: Vec::new(),
        release_manifest_sha256: payload.manifest_sha256,
        release_tree_digest: payload.tree_digest,
        output_digest: output_digest.clone(),
        completed: false,
    };
    let evidence_bytes = serde_json::to_vec(&evidence).unwrap();
    let host_receipt = HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: action_ref.clone(),
        binding_hash: plan.binding_hash.clone(),
        plan_hash: plan.plan_hash.clone(),
        policy_hash: plan.policy_hash.clone(),
        instruction_digest: plane.outcome_grants[&action_ref].instruction_digest.clone(),
        outcome_token: issued.outcome_token.unwrap(),
        generation: issued.outcome_generation.unwrap(),
        status: HostOutcomeStatus::Failed,
        output_digest,
        observed_write_set: Vec::new(),
        artifacts: Vec::new(),
        evidence: Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::UpdateReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: "memory://failed-update-receipt".to_string(),
                sha256: sha256(&evidence_bytes),
            },
            content_hex: hex_bytes(&evidence_bytes),
        }),
    };
    let bytes = serde_json::to_vec(&host_receipt).unwrap();
    let result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref,
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://failed-update-outcome".to_string(),
                        sha256: sha256(&bytes),
                    },
                    bytes,
                )),
            },
        )
        .unwrap();
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Failed);
}

#[test]
fn production_update_abandoned_subset_closes_and_unprovable_extra_risks() {
    let result = production_update_failure_case(HostOutcomeStatus::Abandoned, false, false);
    assert_eq!(result.state, OperationState::Receipted);
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Failed);

    let result = production_update_failure_case(HostOutcomeStatus::Failed, true, false);
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert_eq!(
        result.reason_code.as_deref(),
        Some("host_outcome_unprovable_failure")
    );
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::RiskEscalated);
}

#[test]
fn production_failed_update_with_unreported_hidden_release_child_risks() {
    let result = production_update_failure_case(HostOutcomeStatus::Failed, false, true);
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert_eq!(
        result.reason_code.as_deref(),
        Some("host_outcome_unprovable_failure")
    );
    assert!(result
        .receipt
        .unwrap()
        .observed_write_set
        .iter()
        .any(|path| path.ends_with("/releases/0.4.20/unreported-hidden-child")));
}

#[test]
fn task_close_memory_close_and_session_end_form_one_exact_production_chain() {
    let (workspace, _runtime, mut plane, binding, session) = production_fixture();
    initialize_closure_authority(&mut plane, &binding, &session);
    let root = workspace.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".ags/evidence")).unwrap();
    std::fs::create_dir_all(root.join(".ags/state/closure-pointers")).unwrap();
    let (task_path, launch_plan_path, delivery_report_path) = write_valid_closure_artifacts(&root);

    let task_close = plane
        .decide(
            &session,
            OperationRequest::GovernTaskClose(TaskCloseRequest {
                context: OperationContext::default(),
                task_card_path: task_path.display().to_string(),
                launch_plan_path: launch_plan_path.display().to_string(),
                delivery_report_path: delivery_report_path.display().to_string(),
            }),
        )
        .unwrap();
    let task_close_result = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: task_close.action_ref.unwrap(),
                outcome: None,
            },
        )
        .unwrap();
    assert_eq!(
        task_close_result.receipt.as_ref().unwrap().status,
        ReceiptStatus::Succeeded,
        "{task_close_result:#?}"
    );
    let receipt_path = std::fs::read_dir(root.join(".ags/evidence"))
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .unwrap();
    let receipt: ags_evidence::Receipt =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    let pointer_path = root
        .join(".ags/state/closure-pointers")
        .join(format!("{}.json", receipt.receipt_id));
    assert!(pointer_path.is_file());

    let memory_close = plane
        .decide(
            &session,
            OperationRequest::GovernMemoryClose(MemoryCloseRequest {
                context: OperationContext::default(),
                receipt_path: receipt_path.display().to_string(),
            }),
        )
        .unwrap();
    assert_eq!(memory_close.state, OperationState::NoChange);
    assert!(memory_close.action_ref.is_none());
    assert_eq!(
        memory_close.receipt.unwrap().status,
        ReceiptStatus::Succeeded
    );
    assert!(pointer_path.is_file());

    let request = LifecycleSessionEndRequest {
        context: OperationContext::default(),
        host_id: "hermes".to_string(),
        host_session_id: "session-production".to_string(),
        event_id: "event-production".to_string(),
    };
    let decision = plane
        .decide(
            &session,
            OperationRequest::HostLifecycleSessionEnd(request.clone()),
        )
        .unwrap();
    let action_ref = decision.action_ref.unwrap();
    let plan = decision.plan.unwrap();
    assert!(plan
        .expected_write_paths
        .contains(&pointer_path.display().to_string()));
    let issued = plane
        .apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: None,
            },
        )
        .unwrap();
    let instruction: HostExecutionInstruction = serde_json::from_slice(&read_details_bytes(
        &mut plane,
        &session,
        issued.details.as_ref().unwrap(),
    ))
    .unwrap();
    assert!(matches!(
        instruction.action,
        HostExecutionAction::ArchiveClosures { ref event_id, .. }
            if event_id == "event-production"
    ));

    let mut artifacts = Vec::new();
    let expected_paths = plan
        .expected_write_paths
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for expected in &plan.expected_write_paths {
        let path = PathBuf::from(expected);
        if path == pointer_path {
            std::fs::remove_file(&path).unwrap();
            artifacts.push(HostWriteArtifact {
                path: expected.clone(),
                state: HostArtifactState::Absent,
            });
        } else if expected_paths
            .iter()
            .any(|child| child != &path && child.starts_with(&path))
        {
            std::fs::create_dir_all(&path).unwrap();
            artifacts.push(HostWriteArtifact {
                path: expected.clone(),
                state: HostArtifactState::Directory,
            });
        } else {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = format!("archived:{}", path.display()).into_bytes();
            std::fs::write(&path, &bytes).unwrap();
            artifacts.push(HostWriteArtifact {
                path: expected.clone(),
                state: HostArtifactState::Present {
                    sha256: sha256(bytes),
                },
            });
        }
    }
    let output_digest = sha256("lifecycle-production-output");
    let exact_evidence = serde_json::json!({
        "schema_version": "ags://schema/contract/v2/lifecycle-host-outcome",
        "event_id": request.event_id,
        "receipt_ids": [receipt.receipt_id],
        "observed_write_set": plan.expected_write_paths,
        "consumed_pointer_paths": [pointer_path.display().to_string()],
        "output_digest": output_digest,
        "completed": true,
    });
    let instruction_digest = plane.outcome_grants[&action_ref].instruction_digest.clone();
    let make_receipt = |value: serde_json::Value| {
        let evidence_bytes = serde_json::to_vec(&value).unwrap();
        HostOutcomeReceipt {
            schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
            action_ref: action_ref.clone(),
            binding_hash: plan.binding_hash.clone(),
            plan_hash: plan.plan_hash.clone(),
            policy_hash: plan.policy_hash.clone(),
            instruction_digest: instruction_digest.clone(),
            outcome_token: issued.outcome_token.clone().unwrap(),
            generation: issued.outcome_generation.unwrap(),
            status: HostOutcomeStatus::Succeeded,
            output_digest: output_digest.clone(),
            observed_write_set: plan.expected_write_paths.clone(),
            artifacts: artifacts.clone(),
            evidence: Some(HostOutcomeEvidence {
                kind: HostEvidenceKind::LifecycleReceipt,
                artifact: ContentAddressedArtifactRef {
                    uri: "memory://lifecycle-production-evidence".to_string(),
                    sha256: sha256(&evidence_bytes),
                },
                content_hex: hex_bytes(&evidence_bytes),
            }),
        }
    };
    let apply_receipt = |plane: &mut ControlPlane<ProductionEffectAdapter>, receipt| {
        let bytes = serde_json::to_vec(&receipt).unwrap();
        plane.apply(
            &binding,
            ApplyRequest {
                action_ref: action_ref.clone(),
                outcome: Some(AuthenticatedHostOutcome::from_artifact(
                    binding.clone(),
                    ContentAddressedArtifactRef {
                        uri: "memory://lifecycle-production-outcome".to_string(),
                        sha256: sha256(&bytes),
                    },
                    bytes,
                )),
            },
        )
    };

    let mut empty = exact_evidence.clone();
    empty["receipt_ids"] = serde_json::json!([]);
    assert_eq!(
        apply_receipt(&mut plane, make_receipt(empty))
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    let mut omitted = exact_evidence.clone();
    omitted.as_object_mut().unwrap().remove("receipt_ids");
    assert_eq!(
        apply_receipt(&mut plane, make_receipt(omitted))
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    let mut extra = exact_evidence.clone();
    extra["receipt_ids"] = serde_json::json!([receipt.receipt_id, "receipt-extra"]);
    assert_eq!(
        apply_receipt(&mut plane, make_receipt(extra))
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );
    let mut tampered = exact_evidence.clone();
    tampered["receipt_ids"] = serde_json::json!(["receipt-tampered"]);
    assert_eq!(
        apply_receipt(&mut plane, make_receipt(tampered))
            .unwrap_err()
            .code,
        "host_outcome_verification_failed"
    );

    let result = apply_receipt(&mut plane, make_receipt(exact_evidence)).unwrap();
    assert_eq!(result.state, OperationState::Receipted);
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Succeeded);
}

#[cfg(unix)]
#[test]
fn memory_close_rejects_every_unsafe_artifact_path_shape_for_each_artifact() {
    use std::os::unix::fs::symlink;

    #[derive(Clone, Copy, Debug)]
    enum UnsafeShape {
        External,
        AncestorSymlink,
        FinalSymlink,
        Fifo,
        Oversized,
    }

    for artifact_index in 0..3 {
        for shape in [
            UnsafeShape::External,
            UnsafeShape::AncestorSymlink,
            UnsafeShape::FinalSymlink,
            UnsafeShape::Fifo,
            UnsafeShape::Oversized,
        ] {
            let (workspace, _runtime, mut plane, binding, session) = production_fixture();
            initialize_closure_authority(&mut plane, &binding, &session);
            let root = workspace.path().canonicalize().unwrap();
            let (mut receipt, receipt_path) = write_valid_closure_receipt(&root);
            let original_path = match artifact_index {
                0 => PathBuf::from(receipt.task_card_path.as_deref().unwrap()),
                1 => PathBuf::from(&receipt.launch_plan_path),
                _ => PathBuf::from(&receipt.delivery_report_path),
            };
            let original_bytes = std::fs::read(&original_path).unwrap();
            let outside = tempfile::tempdir().unwrap();
            let unsafe_path = match shape {
                UnsafeShape::External => {
                    let path = outside.path().join("artifact");
                    std::fs::write(&path, &original_bytes).unwrap();
                    path
                }
                UnsafeShape::AncestorSymlink => {
                    std::fs::write(outside.path().join("artifact"), &original_bytes).unwrap();
                    let ancestor = root.join(format!("linked-{artifact_index}"));
                    symlink(outside.path(), &ancestor).unwrap();
                    ancestor.join("artifact")
                }
                UnsafeShape::FinalSymlink => {
                    let outside_file = outside.path().join("artifact");
                    std::fs::write(&outside_file, &original_bytes).unwrap();
                    let path = root.join(format!("final-link-{artifact_index}"));
                    symlink(outside_file, &path).unwrap();
                    path
                }
                UnsafeShape::Fifo => {
                    let path = root.join(format!("artifact-{artifact_index}.fifo"));
                    assert!(std::process::Command::new("mkfifo")
                        .arg(&path)
                        .status()
                        .unwrap()
                        .success());
                    path
                }
                UnsafeShape::Oversized => {
                    let path = root.join(format!("oversized-{artifact_index}"));
                    std::fs::write(&path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
                    path
                }
            };
            match artifact_index {
                0 => receipt.task_card_path = Some(unsafe_path.display().to_string()),
                1 => receipt.launch_plan_path = unsafe_path.display().to_string(),
                _ => receipt.delivery_report_path = unsafe_path.display().to_string(),
            }
            std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();

            let error = plane
                .decide(
                    &session,
                    OperationRequest::GovernMemoryClose(MemoryCloseRequest {
                        context: OperationContext::default(),
                        receipt_path: receipt_path.display().to_string(),
                    }),
                )
                .unwrap_err();
            assert_eq!(error.code, "operation_plan_failed", "{shape:?}");
            assert!(
                error.detail.contains("memory_receipt_unverified"),
                "{shape:?}: {}",
                error.detail
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn task_close_rejects_symlink_fifo_and_oversize_for_each_source_artifact() {
    use std::os::unix::fs::symlink;

    for artifact_index in 0..3 {
        for shape in ["final-symlink", "ancestor-symlink", "fifo", "oversized"] {
            let (workspace, _runtime, mut plane, _binding, session) = production_fixture();
            let root = workspace.path().canonicalize().unwrap();
            let (task, plan, report) = write_valid_closure_artifacts(&root);
            let mut paths = [task, plan, report];
            let original = std::fs::read(&paths[artifact_index]).unwrap();
            let outside = tempfile::tempdir().unwrap();
            paths[artifact_index] = match shape {
                "final-symlink" => {
                    let target = outside.path().join("artifact");
                    std::fs::write(&target, &original).unwrap();
                    let path = root.join(format!("source-{artifact_index}.link"));
                    symlink(target, &path).unwrap();
                    path
                }
                "ancestor-symlink" => {
                    std::fs::write(outside.path().join("artifact"), &original).unwrap();
                    let ancestor = root.join(format!("source-parent-{artifact_index}"));
                    symlink(outside.path(), &ancestor).unwrap();
                    ancestor.join("artifact")
                }
                "fifo" => {
                    let path = root.join(format!("source-{artifact_index}.fifo"));
                    assert!(std::process::Command::new("mkfifo")
                        .arg(&path)
                        .status()
                        .unwrap()
                        .success());
                    path
                }
                _ => {
                    let path = root.join(format!("source-{artifact_index}.oversized"));
                    std::fs::write(&path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
                    path
                }
            };
            let error = plane
                .decide(
                    &session,
                    OperationRequest::GovernTaskClose(TaskCloseRequest {
                        context: OperationContext::default(),
                        task_card_path: paths[0].display().to_string(),
                        launch_plan_path: paths[1].display().to_string(),
                        delivery_report_path: paths[2].display().to_string(),
                    }),
                )
                .unwrap_err();
            assert_eq!(
                error.code, "operation_plan_failed",
                "{artifact_index}:{shape}"
            );
            assert!(
                error.detail.contains("task_close_read_failed"),
                "{artifact_index}:{shape}: {}",
                error.detail
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn sealed_closure_consumers_never_open_receipt_source_paths() {
    use std::os::unix::fs::symlink;

    for shape in ["fifo", "symlink", "oversized", "missing"] {
        let (workspace, runtime, mut plane, binding, session) = production_fixture();
        initialize_closure_authority(&mut plane, &binding, &session);
        let root = workspace.path().canonicalize().unwrap();
        let (mut receipt, receipt_path) = write_valid_closure_receipt(&root);
        let outside = tempfile::tempdir().unwrap();
        let hostile_dir = root.join("untrusted-receipt-sources");
        std::fs::create_dir(&hostile_dir).unwrap();
        let hostile = match shape {
            "fifo" => {
                let path = hostile_dir.join("must-not-open.fifo");
                assert!(std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success());
                path
            }
            "symlink" => {
                let target = outside.path().join("must-not-open");
                std::fs::write(&target, b"outside").unwrap();
                let path = hostile_dir.join("must-not-open.link");
                symlink(target, &path).unwrap();
                path
            }
            "oversized" => {
                let path = hostile_dir.join("must-not-open.oversized");
                std::fs::write(&path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
                path
            }
            _ => hostile_dir.join("must-not-open.missing"),
        };
        receipt.task_card_path = Some(hostile.display().to_string());
        receipt.launch_plan_path = hostile.display().to_string();
        receipt.delivery_report_path = hostile.display().to_string();
        std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        let machine_key: [u8; 32] = std::fs::read(runtime.path().join("closure-authority-v1.key"))
            .unwrap()
            .try_into()
            .unwrap();
        let pointer_path =
            write_signed_closure_pointer(&root, &receipt, &receipt_path, &machine_key);

        let memory_close = plane
            .decide(
                &session,
                OperationRequest::GovernMemoryClose(MemoryCloseRequest {
                    context: OperationContext::default(),
                    receipt_path: receipt_path.display().to_string(),
                }),
            )
            .unwrap();
        assert_eq!(memory_close.state, OperationState::NoChange, "{shape}");

        let evidence = plane
            .decide(
                &session,
                OperationRequest::GovernEvidence(EvidenceRequest {
                    context: OperationContext::default(),
                    artifact_kind: EvidenceArtifactKind::Receipt,
                    path: receipt_path.display().to_string(),
                    task_card_path: None,
                    launch_plan_path: None,
                }),
            )
            .unwrap();
        assert_eq!(
            evidence
                .result
                .as_ref()
                .and_then(|value| value.get("valid"))
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "{shape}"
        );

        let lifecycle = plane
            .decide(
                &session,
                OperationRequest::HostLifecycleSessionEnd(LifecycleSessionEndRequest {
                    context: OperationContext::default(),
                    host_id: "hermes".to_string(),
                    host_session_id: format!("session-{shape}"),
                    event_id: format!("event-{shape}"),
                }),
            )
            .unwrap();
        assert!(lifecycle.action_ref.is_some(), "{shape}");
        assert!(lifecycle
            .plan
            .unwrap()
            .expected_write_paths
            .contains(&pointer_path.display().to_string()));
    }
}

#[test]
fn production_lifecycle_failed_and_abandoned_subsets_close_or_risk_escalate() {
    for status in [HostOutcomeStatus::Failed, HostOutcomeStatus::Abandoned] {
        let result = production_lifecycle_failure_case(status, false, false);
        assert_eq!(result.state, OperationState::Receipted);
        assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Failed);
    }
    let result = production_lifecycle_failure_case(HostOutcomeStatus::Failed, true, false);
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert_eq!(
        result.reason_code.as_deref(),
        Some("host_outcome_unprovable_failure")
    );
    assert_eq!(result.receipt.unwrap().status, ReceiptStatus::RiskEscalated);
}

#[test]
fn production_failed_lifecycle_with_unreported_pointer_delete_risks() {
    let result = production_lifecycle_failure_case(HostOutcomeStatus::Failed, false, true);
    assert_eq!(result.state, OperationState::RiskEscalated);
    assert!(result
        .receipt
        .unwrap()
        .observed_write_set
        .iter()
        .any(|path| path.contains("/.ags/state/closure-pointers/")));
}

fn physical_delta_receipt(observed_write_set: Vec<String>) -> HostOutcomeReceipt {
    HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: "physical-action".to_string(),
        binding_hash: sha256("binding"),
        plan_hash: sha256("plan"),
        policy_hash: sha256("policy"),
        instruction_digest: sha256("instruction"),
        outcome_token: "token".to_string(),
        generation: 1,
        status: HostOutcomeStatus::Failed,
        output_digest: sha256("output"),
        observed_write_set,
        artifacts: Vec::new(),
        evidence: None,
    }
}

fn physical_test_binding(root: &Path) -> AuthenticatedBinding {
    AuthenticatedBinding::mcp(
        "physical-connection",
        "hermes",
        root,
        "physical-workspace",
        sha256("physical-facts"),
        "physical-registry",
        "physical-session",
        vec![root.to_path_buf()],
    )
}

#[test]
fn physical_delta_attributes_each_missing_ancestor_level_exactly() {
    for created_levels in 1..=3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let hosts = root.join("hosts");
        let host = hosts.join("hermes");
        let registration = host.join("registration.json");
        let expected = vec![
            hosts.display().to_string(),
            host.display().to_string(),
            registration.display().to_string(),
        ];
        let seal = seal_host_physical_state(&expected, &physical_test_binding(&root)).unwrap();
        std::fs::create_dir(&hosts).unwrap();
        if created_levels >= 2 {
            std::fs::create_dir(&host).unwrap();
        }
        if created_levels == 3 {
            std::fs::write(&registration, b"registration").unwrap();
        }
        let reported = expected[..created_levels].to_vec();
        let HostTerminalDelta::Exact { changed, .. } =
            verify_host_physical_delta(&seal, &physical_delta_receipt(reported.clone()))
        else {
            panic!("partial ancestor creation must remain an exact residual delta")
        };
        assert_eq!(changed, reported);
    }
}

#[test]
fn physical_delta_does_not_attribute_planned_child_to_preexisting_parent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let hosts = root.join("hosts");
    std::fs::create_dir(&hosts).unwrap();
    let host = hosts.join("hermes");
    let registration = host.join("registration.json");
    let expected = vec![
        hosts.display().to_string(),
        host.display().to_string(),
        registration.display().to_string(),
    ];
    let seal = seal_host_physical_state(&expected, &physical_test_binding(&root)).unwrap();
    std::fs::create_dir(&host).unwrap();
    let reported = vec![host.display().to_string()];
    let HostTerminalDelta::Exact { changed, .. } =
        verify_host_physical_delta(&seal, &physical_delta_receipt(reported.clone()))
    else {
        panic!("planned child addition under a preexisting parent must be exact")
    };
    assert_eq!(changed, reported);
}

#[test]
fn physical_seal_rejects_an_unplanned_missing_ancestor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let leaf = root.join("missing-parent/leaf.txt");
    let Err(error) =
        seal_host_physical_state(&[leaf.display().to_string()], &physical_test_binding(&root))
    else {
        panic!("unplanned missing ancestor must fail closed")
    };
    assert_eq!(error.code, "host_before_state_unplanned_missing_ancestor");
}

#[test]
#[cfg(unix)]
fn terminal_physical_scanner_stops_at_shared_member_budget_before_next_scan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let directory_count = MAX_HOST_PHYSICAL_DIRECTORY_GUARDS;
    let members_per_directory = MAX_HOST_PHYSICAL_TOTAL_MEMBERS / directory_count;
    assert_eq!(
        directory_count * members_per_directory,
        MAX_HOST_PHYSICAL_TOTAL_MEMBERS
    );
    let mut expected = Vec::with_capacity(directory_count);
    for directory_index in 0..directory_count {
        let directory = root.join(format!("d{directory_index:03}"));
        std::fs::create_dir(&directory).unwrap();
        for member_index in 0..members_per_directory {
            std::fs::write(directory.join(format!("m{member_index:02}")), b"x").unwrap();
        }
        expected.push(directory.join("planned").display().to_string());
    }
    let seal = seal_host_physical_state(&expected, &physical_test_binding(&root)).unwrap();
    std::fs::write(
        root.join(format!("d{:03}/zz-extra", directory_count - 1)),
        b"x",
    )
    .unwrap();
    PHYSICAL_DIRECT_MEMBER_SCANS.with(|count| count.set(0));
    let HostTerminalDelta::Risk { proof_error, .. } =
        verify_host_physical_delta(&seal, &physical_delta_receipt(Vec::new()))
    else {
        panic!("aggregate member overflow must fail closed")
    };
    assert_eq!(proof_error.code, "host_outcome_physical_budget_exceeded");
    let scans = PHYSICAL_DIRECT_MEMBER_SCANS.with(std::cell::Cell::get);
    assert_eq!(
        scans, MAX_HOST_PHYSICAL_TOTAL_MEMBERS,
        "the scanner must reject the next member before stat/open/read"
    );
}

#[cfg(unix)]
#[test]
fn physical_delta_binds_file_content_mode_and_type() {
    use std::os::unix::fs::PermissionsExt;

    for mutation in ["content", "mode", "type"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let file = root.join("owned");
        std::fs::write(&file, b"before").unwrap();
        let expected = vec![file.display().to_string()];
        let seal = seal_host_physical_state(&expected, &physical_test_binding(&root)).unwrap();
        match mutation {
            "content" => std::fs::write(&file, b"after").unwrap(),
            "mode" => {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap()
            }
            "type" => {
                std::fs::remove_file(&file).unwrap();
                std::fs::create_dir(&file).unwrap();
            }
            _ => unreachable!(),
        }
        let reported = vec![file.display().to_string()];
        let HostTerminalDelta::Exact { changed, .. } =
            verify_host_physical_delta(&seal, &physical_delta_receipt(reported.clone()))
        else {
            panic!("{mutation} replacement must produce an exact target delta")
        };
        assert_eq!(changed, reported);
    }
}

#[cfg(unix)]
#[test]
fn physical_delta_rejects_authorized_root_path_substitution() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("bound-root");
    std::fs::create_dir(&root_path).unwrap();
    let root = root_path.canonicalize().unwrap();
    let owned = root.join("owned");
    let seal = seal_host_physical_state(
        &[owned.display().to_string()],
        &physical_test_binding(&root),
    )
    .unwrap();
    std::fs::rename(&root, temp.path().join("old-bound-root")).unwrap();
    std::fs::create_dir(&root).unwrap();
    let HostTerminalDelta::Risk { proof_error, .. } =
        verify_host_physical_delta(&seal, &physical_delta_receipt(Vec::new()))
    else {
        panic!("root path substitution must make the terminal delta unprovable")
    };
    assert_eq!(proof_error.code, "host_outcome_root_binding_changed");
}

#[cfg(unix)]
#[test]
fn physical_delta_rechecks_root_path_identity_after_scanning() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bound-root");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let owned = root.join("owned");
    let seal = seal_host_physical_state(
        &[owned.display().to_string()],
        &physical_test_binding(&root),
    )
    .unwrap();
    let displaced = temp.path().join("displaced-bound-root");
    ROOT_AFTER_SCAN_SUBSTITUTION.with(|slot| {
        *slot.borrow_mut() = Some((root.clone(), displaced));
    });
    let result = verify_host_physical_delta(&seal, &physical_delta_receipt(Vec::new()));
    ROOT_AFTER_SCAN_SUBSTITUTION.with(|slot| {
        slot.borrow_mut().take();
    });
    let HostTerminalDelta::Risk { proof_error, .. } = result else {
        panic!("root substitution after scanning must invalidate the physical proof")
    };
    assert_eq!(proof_error.code, "host_outcome_root_binding_changed");
}

#[cfg(unix)]
#[test]
fn tree_digest_rejects_same_inode_same_size_rewrite_during_snapshot_read() {
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("stable.txt");
    std::fs::write(&file, b"original").unwrap();
    let before = std::fs::metadata(&file).unwrap();
    SNAPSHOT_AFTER_READ_REWRITE.with(|slot| {
        *slot.borrow_mut() = Some((
            file.clone(),
            PathBuf::from("stable.txt"),
            b"tampered".to_vec(),
        ));
    });
    let result = tree_digest(&[temp.path().canonicalize().unwrap()]);
    SNAPSHOT_AFTER_READ_REWRITE.with(|slot| {
        slot.borrow_mut().take();
    });
    assert!(
        result.is_err(),
        "snapshot must reject an unstable file read"
    );
    let after = std::fs::metadata(file).unwrap();
    assert_eq!(before.ino(), after.ino());
    assert_eq!(before.len(), after.len());
}

#[test]
fn authorized_root_order_is_semantic_and_duplicates_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let workspace = first.canonicalize().unwrap();
    let second = second.canonicalize().unwrap();
    let binding = |roots| {
        AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &workspace,
            "workspace-a",
            sha256("facts-a"),
            "registry-a",
            "session-a",
            roots,
        )
    };
    let ordered = binding(vec![workspace.clone(), second.clone()]);
    let reversed = binding(vec![second, workspace.clone()]);
    assert_eq!(ordered.semantic_bytes(), reversed.semantic_bytes());
    assert_eq!(ordered.canonical_bytes(), reversed.canonical_bytes());
    assert_eq!(
        validate_binding(&binding(vec![workspace.clone(), workspace.clone()]))
            .unwrap_err()
            .code,
        "binding_invalid"
    );
}

#[test]
fn host_apply_rejects_authorized_root_inode_substitution() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let root = workspace.canonicalize().unwrap();
    let state = Arc::new(Mutex::new(FakeState {
        apply_succeeds: true,
        verify_succeeds: true,
        recover_succeeds: true,
        effect_started: true,
        ..FakeState::default()
    }));
    let mut plane = ControlPlane::with_sealing_key(
        FakeAdapter {
            state,
            read_root: root.clone(),
        },
        sha256("root-substitution-key"),
    );
    let binding = AuthenticatedBinding::mcp(
        "connection-a",
        "hermes",
        &root,
        "workspace-a",
        sha256("facts-a"),
        "registry-a",
        "session-a",
        vec![root.clone()],
    );
    let session = plane
        .open(OpenRequest {
            binding: binding.clone(),
            policy_hash: sha256("policy"),
        })
        .unwrap();
    let action_ref = plane
        .decide(&session, host_request())
        .unwrap()
        .action_ref
        .unwrap();
    let moved = outer.path().join("workspace-moved");
    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_cross_binding"
    );
}

#[test]
fn session_lru_is_bounded_and_eviction_cascades_owned_state() {
    let (temp, mut plane, state, binding, first_session) = fixture();
    state.lock().unwrap().read_result = Some(serde_json::json!({
        "evidence": "x".repeat(DETAILS_INLINE_LIMIT + 1024)
    }));
    let details = plane
        .decide(
            &first_session,
            OperationRequest::Schema(SchemaRequest {
                context: OperationContext::default(),
                operation: None,
            }),
        )
        .unwrap()
        .result
        .unwrap();
    let details_uri = details["details_uri"].as_str().unwrap().to_string();
    let host_action = plane
        .decide(&first_session, host_request())
        .unwrap()
        .action_ref
        .unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: host_action.clone(),
                    outcome: None,
                },
            )
            .unwrap()
            .state,
        OperationState::AwaitingOutcome
    );

    for index in 0..MAX_ACTIVE_SESSIONS {
        let next = AuthenticatedBinding::mcp(
            format!("connection-{index}"),
            "hermes",
            temp.path().canonicalize().unwrap(),
            format!("workspace-{index}"),
            sha256(format!("facts-{index}")),
            "registry-a",
            format!("session-{index}"),
            vec![temp.path().canonicalize().unwrap()],
        );
        plane
            .open(OpenRequest {
                binding: next,
                policy_hash: sha256("policy"),
            })
            .unwrap();
    }

    assert_eq!(plane.sessions.len(), MAX_ACTIVE_SESSIONS);
    assert!(!plane.sessions.contains_key(first_session.session_ref()));
    assert!(!plane.actions.contains_key(&host_action));
    assert!(!plane.outcome_grants.contains_key(&host_action));
    assert!(!plane.details.contains_key(&details_uri));
    assert_eq!(plane.details_total_bytes, 0);
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: host_action,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );
}

#[test]
fn action_lru_bounds_planned_and_awaiting_operations_without_false_success() {
    let (_temp, mut plane, _state, binding, session) = fixture();
    let first = plane
        .decide(&session, host_request())
        .unwrap()
        .action_ref
        .unwrap();
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: first.clone(),
                    outcome: None,
                },
            )
            .unwrap()
            .state,
        OperationState::AwaitingOutcome
    );
    for _ in 0..MAX_ACTIVE_ACTIONS {
        let decision = plane.decide(&session, transaction_request()).unwrap();
        assert_eq!(decision.state, OperationState::Planned);
        assert!(decision.receipt.is_none());
    }
    assert_eq!(plane.actions.len(), MAX_ACTIVE_ACTIONS);
    assert!(!plane.actions.contains_key(&first));
    assert!(!plane.outcome_grants.contains_key(&first));
    assert_eq!(
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: first,
                    outcome: None,
                },
            )
            .unwrap_err()
            .code,
        "action_ref_invalid"
    );
}

#[test]
fn details_lru_enforces_record_and_byte_limits() {
    let (_temp, mut plane, state, _binding, session) = fixture();
    state.lock().unwrap().read_result = Some(serde_json::json!({
        "evidence": "z".repeat(DETAILS_INLINE_LIMIT + 1024)
    }));
    let mut first_uri = None;
    for _ in 0..=MAX_DETAILS_RECORDS {
        let result = plane
            .decide(
                &session,
                OperationRequest::Schema(SchemaRequest {
                    context: OperationContext::default(),
                    operation: None,
                }),
            )
            .unwrap()
            .result
            .unwrap();
        first_uri.get_or_insert_with(|| result["details_uri"].as_str().unwrap().to_string());
    }
    assert_eq!(plane.details.len(), MAX_DETAILS_RECORDS);
    assert!(plane.details_total_bytes <= MAX_DETAILS_TOTAL_BYTES);
    assert!(!plane.details.contains_key(first_uri.as_ref().unwrap()));
}
