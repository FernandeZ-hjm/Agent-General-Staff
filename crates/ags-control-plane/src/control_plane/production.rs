use super::*;
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Seek, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_STAGE_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_QUARANTINE_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_JOURNAL_APPLIED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_RECOVERY_FINALIZE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_TRANSACTION_VERIFY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_RISK_JOURNAL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PANIC_RECOVERY_AFTER_RESTORE_BEFORE_PROGRESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RUNTIME_AUTHORITY_UID_OVERRIDE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    static PRODUCTION_DOMAIN_APPLY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FAIL_PROJECTION_AFTER_FIRST_MUTATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PANIC_PROJECTION_AFTER_FIRST_MUTATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DRIFT_PROJECTION_AFTER_FIRST_MUTATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PANIC_PROJECTION_BEFORE_IDENTITY_KIND: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static PANIC_PROJECTION_AFTER_OPERATION: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static FAIL_SKILL_AFTER_MUTATION_KIND: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static PANIC_SKILL_AFTER_MUTATION_KIND: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static DRIFT_SKILL_AFTER_MUTATION_KIND: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static SUBSTITUTE_NEXT_STAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SUBSTITUTE_NEXT_STAGE_DIFFERENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RELEASE_AFTER_READ_REWRITE: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(all(test, unix))]
fn run_release_after_read_rewrite_test_hook(path: &Path) {
    RELEASE_AFTER_READ_REWRITE.with(|slot| {
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

const MANAGED_ENTRY: &str = "\n<!-- ags-managed:contract-v2 -->\n## Agent Governance Suite\n\nThis workspace uses the AGS contract v2 control plane. Runtime capability discovery is available at `ags://capabilities/current-host`.\n";
const CLOSURE_AUTHORITY_KEY_FILE: &str = "closure-authority-v1.key";
const PROFILE: &str = r#"schema_version: ags://schema/contract/v2/project-profile
verification:
  project_tests:
    smoke: { program: cargo, argv: ["+stable", test, --workspace], cwd: ., env: {}, timeout_ms: 180000, allowed_write_paths: [target] }
    standard: { program: cargo, argv: ["+stable", test, --workspace], cwd: ., env: {}, timeout_ms: 900000, allowed_write_paths: [target] }
    full: { program: cargo, argv: ["+stable", test, --workspace, --all-features], cwd: ., env: {}, timeout_ms: 1200000, allowed_write_paths: [target] }
workflow:
  memory_uri: ags-memory://project/current
"#;
const TRANSACTION_JOURNAL_SCHEMA: &str = "ags://schema/contract/v2/transaction-journal";
const MAX_TRANSACTION_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROFILE_BYTES: usize = 256 * 1024;
const MAX_CLOSURE_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RELEASE_MEMBER_BYTES: usize = 16 * 1024 * 1024;
const MAX_RELEASE_NAME_BYTES: usize = 4096;
const MAX_TRANSACTION_JOURNAL_ENTRIES: usize = 256;
const MAX_TRANSACTION_JOURNAL_NAME_BYTES: usize = 64 * 1024;
const RELEASE_PAYLOAD_NAMES: [&str; 5] = [
    "ags",
    "ags-mcp",
    "ags-host",
    "ags-launcher.js",
    "release-metadata.json",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JournalPhase {
    Prepared,
    Applying,
    Applied,
    Verified,
    Recovered,
    RiskEscalated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JournalImage {
    RegularFile {
        sha256: String,
        data_hex: String,
        mode: u32,
    },
    Directory {
        mode: u32,
    },
    Symlink {
        target_hex: String,
    },
    Absent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JournalPostimage {
    RegularFile { sha256: String, mode: u32 },
    Directory { mode: u32 },
    Symlink { target_hex: String },
    Absent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JournalParentIdentity {
    relative_path: String,
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JournalApplyAnchor {
    Pending,
    Applied {
        parent_chain: Vec<JournalParentIdentity>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JournalWriteRecoveryProgress {
    Applied,
    Restored { preimage_proof: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JournalWrite {
    order: usize,
    path: String,
    root_path: String,
    root_device: u64,
    root_inode: u64,
    operation: String,
    preimage: JournalImage,
    postimage: JournalPostimage,
    apply_anchor: JournalApplyAnchor,
    recovery_progress: JournalWriteRecoveryProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    post_identity: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TransactionJournal {
    schema_version: String,
    action_ref: String,
    binding_hash: String,
    canonical_workspace: String,
    workspace_identity: String,
    registry_key: String,
    plan_hash: String,
    policy_hash: String,
    payload_hash: String,
    operation: String,
    ordered_writes: Vec<JournalWrite>,
    identity_digest: String,
    phase: JournalPhase,
    recovery_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_recovery: Option<TerminalRecoveryRecord>,
    integrity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TerminalRecoveryRecord {
    original_action_ref: String,
    original_operation: String,
    original_binding_hash: String,
    original_policy_hash: String,
    original_journal_digest: String,
    journal_identity_digest: String,
    journal_state_digest: String,
    recovery_action_ref: String,
    recovery_plan_hash: String,
    recovery_payload_hash: String,
    recovery_binding_hash: String,
    recovery_policy_hash: String,
    receipt: OperationReceipt,
}

impl TransactionJournal {
    fn recompute_identity_digest(&self) -> Result<String, EffectError> {
        let writes = self
            .ordered_writes
            .iter()
            .map(|write| {
                serde_json::json!({
                    "order": write.order,
                    "path": write.path,
                    "root_path": write.root_path,
                    "root_device": write.root_device,
                    "root_inode": write.root_inode,
                    "operation": write.operation,
                    "preimage": write.preimage,
                    "postimage": write.postimage,
                    "apply_anchor": write.apply_anchor,
                    "post_identity": write.post_identity,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&serde_json::json!({
            "schema_version": self.schema_version,
            "action_ref": self.action_ref,
            "binding_hash": self.binding_hash,
            "canonical_workspace": self.canonical_workspace,
            "workspace_identity": self.workspace_identity,
            "registry_key": self.registry_key,
            "plan_hash": self.plan_hash,
            "policy_hash": self.policy_hash,
            "payload_hash": self.payload_hash,
            "operation": self.operation,
            "ordered_writes": writes,
        }))
        .map(sha256)
        .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))
    }

    fn state_digest(&self) -> Result<String, EffectError> {
        serde_json::to_vec(&serde_json::json!({
            "identity_digest": self.identity_digest,
            "phase": self.phase,
            "recovery_generation": self.recovery_generation,
            "write_progress": self.ordered_writes.iter().map(|write| &write.recovery_progress).collect::<Vec<_>>(),
        }))
        .map(sha256)
        .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))
    }

    fn reseal(&mut self) -> Result<(), EffectError> {
        self.integrity.clear();
        self.integrity =
            sha256(serde_json::to_vec(self).map_err(|error| {
                blocked("transaction_journal_encode_failed", error.to_string())
            })?);
        Ok(())
    }

    fn verify_integrity(&self) -> Result<(), EffectError> {
        let mut unsealed = self.clone();
        let declared = std::mem::take(&mut unsealed.integrity);
        let actual =
            sha256(serde_json::to_vec(&unsealed).map_err(|error| {
                blocked("transaction_journal_encode_failed", error.to_string())
            })?);
        if declared != actual {
            return Err(blocked(
                "transaction_journal_integrity_failed",
                &self.action_ref,
            ));
        }
        if self.identity_digest != self.recompute_identity_digest()? {
            return Err(blocked(
                "transaction_journal_identity_digest_mismatch",
                &self.action_ref,
            ));
        }
        Ok(())
    }
}

/// Private runtime state is a same-credential authority boundary: journal and
/// closure-key bytes are trusted only after the already-open directory FD is
/// proven to be owned by the effective uid and not writable by group/other.
/// Workspace roots intentionally do not inherit this runtime-only policy.
fn validate_runtime_authority_root(stat: &rustix::fs::Stat) -> Result<(), EffectError> {
    let effective_uid = {
        #[cfg(test)]
        {
            RUNTIME_AUTHORITY_UID_OVERRIDE
                .with(|override_uid| override_uid.get())
                .unwrap_or_else(|| rustix::process::geteuid().as_raw())
        }
        #[cfg(not(test))]
        {
            rustix::process::geteuid().as_raw()
        }
    };
    let mode = stat.st_mode as u32;
    if stat.st_uid != effective_uid || mode & 0o022 != 0 {
        return Err(blocked(
            "runtime_authority_permissions_invalid",
            format!(
                "runtime root fd owner/mode rejected: owner={}, effective_uid={effective_uid}, mode={:#o}",
                stat.st_uid,
                mode & 0o7777
            ),
        ));
    }
    Ok(())
}

fn validate_terminal_recovery_record(
    journal: &TransactionJournal,
    record: &TerminalRecoveryRecord,
) -> Result<(), EffectError> {
    let hashes = [
        &record.original_binding_hash,
        &record.original_policy_hash,
        &record.original_journal_digest,
        &record.journal_identity_digest,
        &record.journal_state_digest,
        &record.recovery_plan_hash,
        &record.recovery_payload_hash,
        &record.recovery_binding_hash,
        &record.recovery_policy_hash,
        &record.receipt.output_digest,
    ];
    if hashes.iter().any(|hash| !ags_platform::is_sha256(hash)) {
        return Err(blocked(
            "terminal_recovery_receipt_invalid",
            &journal.action_ref,
        ));
    }
    let operation = operation_registry()
        .iter()
        .find(|spec| spec.name.as_str() == journal.operation)
        .map(|spec| spec.name)
        .ok_or_else(|| blocked("transaction_journal_operation_invalid", &journal.operation))?;
    let recomputed = receipt_with_evidence(
        operation,
        ReceiptStatus::Recovered,
        &record.receipt.plan_hash,
        &record.receipt.payload_hash,
        &record.receipt.binding_hash,
        &record.receipt.output_digest,
        record.receipt.observed_write_set.clone(),
        true,
        record.receipt.evidence.clone(),
    );
    let evidence = record
        .receipt
        .evidence
        .as_ref()
        .and_then(serde_json::Value::as_object);
    let evidence_matches = evidence.is_some_and(|evidence| {
        evidence
            .get("recovery_action_ref")
            .and_then(serde_json::Value::as_str)
            == Some(record.recovery_action_ref.as_str())
            && evidence
                .get("recovery_policy_hash")
                .and_then(serde_json::Value::as_str)
                == Some(record.recovery_policy_hash.as_str())
            && evidence
                .get("original_journal_digest")
                .and_then(serde_json::Value::as_str)
                == Some(record.original_journal_digest.as_str())
            && evidence
                .get("journal_identity_digest")
                .and_then(serde_json::Value::as_str)
                == Some(record.journal_identity_digest.as_str())
            && evidence
                .get("journal_state_digest")
                .and_then(serde_json::Value::as_str)
                == Some(record.journal_state_digest.as_str())
    });
    let valid = journal.phase == JournalPhase::Recovered
        && record.original_action_ref == journal.action_ref
        && record.original_operation == journal.operation
        && record.original_binding_hash == journal.binding_hash
        && record.original_policy_hash == journal.policy_hash
        && record.journal_identity_digest == journal.identity_digest
        && record.receipt.schema_version == CONTRACT_SCHEMA_VERSION
        && record.receipt.operation == operation
        && record.receipt.status == ReceiptStatus::Recovered
        && record.receipt.recovered
        && record.receipt.plan_hash == record.recovery_plan_hash
        && record.receipt.payload_hash == record.recovery_payload_hash
        && record.receipt.binding_hash == record.recovery_binding_hash
        && journal.output_digest.as_deref() == Some(record.receipt.output_digest.as_str())
        && !record.recovery_action_ref.is_empty()
        && evidence_matches
        && record.receipt == recomputed;
    if !valid {
        return Err(blocked(
            "terminal_recovery_receipt_invalid",
            &journal.action_ref,
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct RootCapability {
    label: &'static str,
    canonical: PathBuf,
    descriptor: Arc<OwnedFd>,
    identity: (u64, u64),
}

impl std::fmt::Debug for RootCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RootCapability")
            .field("label", &self.label)
            .field("canonical", &self.canonical)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct AnchoredPath {
    root: RootCapability,
    relative: PathBuf,
    sealed: String,
    parent_anchor: Arc<Mutex<ParentAnchor>>,
    target_identity: Arc<Mutex<Option<(u64, u64)>>>,
}

struct ParentAnchor {
    descriptor: Option<Arc<OwnedFd>>,
    identity: Option<(u64, u64)>,
}

impl std::fmt::Debug for AnchoredPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("AnchoredPath")
            .field(&self.sealed)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct PlannedWrite {
    target: AnchoredPath,
    content: Vec<u8>,
    previous: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct PlannedDelete {
    target: AnchoredPath,
    previous: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PlannedProjection {
    workspace: PathBuf,
    planned_directory_paths: Vec<String>,
    mutations: Vec<PlannedObjectMutation>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedObjectMutation {
    target: AnchoredPath,
    operation: String,
    preimage: JournalImage,
    postimage: JournalPostimage,
    apply: ObjectMutationApply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum ObjectMutationApply {
    CreateDirectory,
    WriteFile {
        previous: Option<Vec<u8>>,
        next: Vec<u8>,
    },
    DeleteFile {
        previous: Vec<u8>,
    },
    SetSymlink {
        previous: Option<Vec<u8>>,
        next: Option<Vec<u8>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseMember {
    name: String,
    sha256: String,
    size: u64,
    mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedReleaseManifest {
    schema_version: String,
    version: String,
    tree_digest: String,
    members: Vec<ReleaseMember>,
}

#[derive(Debug, Clone)]
pub enum ProductionAction {
    Files(Vec<PlannedWrite>),
    Deletes(Vec<PlannedDelete>),
    Projections(Vec<PlannedProjection>),
    ProjectTest {
        workspace: PathBuf,
        profile: ags_verification::TestProfile,
        spec: ags_verification::CommandSpec,
    },
    SkillChange {
        context: ags_capability_governance::skill_adoption::AdoptionContext,
        materialized: Box<ags_capability_governance::skill_adoption::MaterializedSkillChange>,
        mutations: Vec<PlannedObjectMutation>,
    },
    PendingRecovery {
        original_action_ref: String,
        journal_identity_digest: String,
        journal_state_digest: String,
        expected_write_paths: Vec<String>,
    },
    LifecycleSessionEnd {
        receipt_ids: Vec<String>,
        pointer_paths: Vec<String>,
    },
    Update {
        candidate_directory: PathBuf,
        release_directory: PathBuf,
        manifest: ReleaseMember,
        tree_digest: String,
        members: Vec<ReleaseMember>,
    },
    None,
}

/// Production adapter with concrete filesystem transactions and local command
/// execution. Unsupported domains fail closed during planning; they never
/// produce a successful no-op receipt.
pub struct ProductionEffectAdapter {
    runtime_home: PathBuf,
    host_home: PathBuf,
}

struct NoopHostProbeRunner;

impl ags_host_integration::HostProbeRunner for NoopHostProbeRunner {
    fn run(
        &self,
        _spec: &ags_host_integration::McpProbeSpec,
    ) -> ags_host_integration::HostProbeExecution {
        ags_host_integration::HostProbeExecution::Unavailable
    }
}

impl ProductionEffectAdapter {
    fn closure_authority_key(
        &self,
        _binding: &AuthenticatedBinding,
    ) -> Result<[u8; 32], EffectError> {
        let root = self.root_capability("runtime", &self.runtime_home)?;
        let target = self.path_in_root(root, Path::new(CLOSURE_AUTHORITY_KEY_FILE))?;
        let bytes = anchored_read(&target, false)?.ok_or_else(|| {
            blocked(
                "closure_authority_unavailable",
                "setup has not created the machine closure authority",
            )
        })?;
        bytes.try_into().map_err(|bytes: Vec<u8>| {
            blocked(
                "closure_authority_invalid",
                format!(
                    "machine closure authority must be exactly 32 bytes, got {}",
                    bytes.len()
                ),
            )
        })
    }

    fn inspect_pending_transaction(
        &self,
        binding: &AuthenticatedBinding,
    ) -> Result<PendingInspection<ProductionAction>, EffectError> {
        match fs::symlink_metadata(&self.runtime_home) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(blocked(
                    "transaction_root_invalid",
                    self.runtime_home.display().to_string(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PendingInspection::default());
            }
            Err(error) => {
                return Err(effect_error("transaction_root_stat_failed", error, false));
            }
        }
        let root = self.root_capability("runtime", &self.runtime_home)?;
        let directory = rustix::io::dup(root.descriptor.as_ref())
            .map_err(|error| effect_error("transaction_root_dup_failed", error, false))?;
        let directory_identity = parent_identity(&directory)?;
        let mut names = Vec::new();
        let mut pending = None;
        let mut terminal_receipts = Vec::new();
        let mut name_bytes = 0usize;
        for entry in rustix::fs::Dir::read_from(&directory).map_err(|error| {
            effect_error("transaction_journal_directory_read_failed", error, false)
        })? {
            let entry = entry.map_err(|error| {
                effect_error("transaction_journal_directory_read_failed", error, false)
            })?;
            let name = entry
                .file_name()
                .to_str()
                .map(str::to_string)
                .map_err(|_| {
                    blocked(
                        "transaction_journal_name_invalid",
                        "journal name is not UTF-8",
                    )
                })?;
            if name == "." || name == ".." {
                continue;
            }
            if names.len() >= MAX_TRANSACTION_JOURNAL_ENTRIES {
                return Err(blocked(
                    "transaction_journal_entry_budget_exceeded",
                    "runtime directory exceeds the journal enumeration entry budget",
                ));
            }
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > MAX_TRANSACTION_JOURNAL_NAME_BYTES {
                return Err(blocked(
                    "transaction_journal_entry_budget_exceeded",
                    "runtime directory exceeds the journal enumeration name-byte budget",
                ));
            }
            names.push(name);
        }
        names.sort();
        for name in names {
            let Some(action_ref) = name
                .strip_prefix(".ags-transaction-")
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if action_ref.is_empty()
                || !action_ref
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(blocked("transaction_journal_name_invalid", name));
            }
            let target = AnchoredPath {
                root: root.clone(),
                relative: PathBuf::from(&name),
                sealed: self.runtime_home.join(&name).display().to_string(),
                parent_anchor: Arc::new(Mutex::new(ParentAnchor {
                    descriptor: Some(Arc::new(rustix::io::dup(&directory).map_err(|error| {
                        effect_error("transaction_parent_dup_failed", error, false)
                    })?)),
                    identity: Some(directory_identity),
                })),
                target_identity: Arc::new(Mutex::new(identity_at(&directory, OsStr::new(&name))?)),
            };
            let raw = anchored_read(&target, false)?
                .ok_or_else(|| blocked("transaction_journal_missing", action_ref))?;
            let journal: TransactionJournal = serde_json::from_slice(&raw)
                .map_err(|error| blocked("transaction_journal_invalid", error.to_string()))?;
            journal.verify_integrity()?;
            if journal.schema_version != TRANSACTION_JOURNAL_SCHEMA
                || journal.action_ref != action_ref
            {
                return Err(blocked("transaction_journal_identity_mismatch", action_ref));
            }
            if journal.canonical_workspace != binding.canonical_workspace.display().to_string()
                || journal.workspace_identity != binding.workspace_identity
            {
                continue;
            }
            if journal.registry_key != binding.registry_key {
                return Err(blocked(
                    "transaction_journal_registry_mismatch",
                    &journal.action_ref,
                ));
            }
            if journal.phase == JournalPhase::Recovered {
                let record = journal.terminal_recovery.as_ref().ok_or_else(|| {
                    blocked("terminal_recovery_receipt_missing", &journal.action_ref)
                })?;
                validate_terminal_recovery_record(&journal, record)?;
                terminal_receipts.push(record.receipt.clone());
                continue;
            }
            if journal.phase == JournalPhase::Verified {
                continue;
            }
            if journal.phase == JournalPhase::RiskEscalated {
                return Err(blocked(
                    "transaction_recovery_risk_requires_operator",
                    &journal.action_ref,
                ));
            }
            let marker = sibling_target(
                &target,
                &format!(".ags-transaction-{}.commit", journal.action_ref),
            )?;
            if let Some(marker_bytes) = anchored_read(&marker, false)? {
                verify_commit_marker(&marker_bytes, &journal)?;
                let mut committed = true;
                for write in &journal.ordered_writes {
                    let anchored = self.journal_write_target(binding, write)?;
                    validate_journal_root(write, &anchored)?;
                    committed &= journal_postimage_matches(write, &anchored)?;
                }
                if !committed {
                    return Err(blocked(
                        "transaction_committed_postimage_drift",
                        &journal.action_ref,
                    ));
                }
                continue;
            }
            let operation = operation_registry()
                .iter()
                .find(|spec| spec.name.as_str() == journal.operation)
                .map(|spec| spec.name)
                .ok_or_else(|| {
                    blocked("transaction_journal_operation_invalid", &journal.operation)
                })?;
            let mut expected_write_paths = journal
                .ordered_writes
                .iter()
                .map(|write| write.path.clone())
                .collect::<Vec<_>>();
            expected_write_paths.push(target.sealed.clone());
            expected_write_paths.sort();
            expected_write_paths.dedup();
            let journal_identity_digest = journal.identity_digest.clone();
            let journal_state_digest = journal.state_digest()?;
            let candidate = PendingRecovery {
                operation,
                journal_identity_digest: journal_identity_digest.clone(),
                journal_state_digest: journal_state_digest.clone(),
                expected_write_paths: expected_write_paths.clone(),
                action: ProductionAction::PendingRecovery {
                    original_action_ref: journal.action_ref,
                    journal_identity_digest,
                    journal_state_digest,
                    expected_write_paths,
                },
            };
            if pending.is_some() {
                return Err(blocked(
                    "multiple_pending_transactions",
                    "more than one active journal exists for this workspace; causal order is unproven",
                ));
            }
            pending = Some(candidate);
        }
        Ok(PendingInspection {
            active: pending,
            terminal_receipts,
        })
    }

    pub fn new(runtime_home: impl Into<PathBuf>) -> Self {
        Self {
            runtime_home: ags_platform::normalize_path(&runtime_home.into()),
            host_home: ags_platform::home_dir_or_temp(),
        }
    }

    pub fn with_host_home(runtime_home: impl Into<PathBuf>, host_home: impl Into<PathBuf>) -> Self {
        Self {
            runtime_home: ags_platform::normalize_path(&runtime_home.into()),
            host_home: ags_platform::normalize_path(&host_home.into()),
        }
    }

    fn root_capability(
        &self,
        label: &'static str,
        root_path: &Path,
    ) -> Result<RootCapability, EffectError> {
        let descriptor = rustix::fs::open(
            root_path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| effect_error("transaction_root_open_failed", error, false))?;
        let root_stat = rustix::fs::fstat(&descriptor)
            .map_err(|error| effect_error("transaction_root_stat_failed", error, false))?;
        if label == "runtime" && root_path == self.runtime_home {
            validate_runtime_authority_root(&root_stat)?;
        }
        Ok(RootCapability {
            label,
            canonical: root_path.to_path_buf(),
            descriptor: Arc::new(descriptor),
            identity: (root_stat.st_dev as u64, root_stat.st_ino as u64),
        })
    }

    fn path_in_root(
        &self,
        root: RootCapability,
        relative: &Path,
    ) -> Result<AnchoredPath, EffectError> {
        self.path_in_root_with_leaf_policy(root, relative, false)
    }

    /// Symlink leaves are accepted only for a typed MaterializedSymlink
    /// mutation. Parent traversal remains descriptor-relative and NOFOLLOW.
    fn path_in_root_symlink(
        &self,
        root: RootCapability,
        relative: &Path,
    ) -> Result<AnchoredPath, EffectError> {
        self.path_in_root_with_leaf_policy(root, relative, true)
    }

    fn path_in_root_with_leaf_policy(
        &self,
        root: RootCapability,
        relative: &Path,
        allow_symlink_leaf: bool,
    ) -> Result<AnchoredPath, EffectError> {
        validate_relative_path(relative)?;
        let (held_parent, held_parent_identity) = match open_parent(&root, relative, false) {
            Ok(parent) => {
                let stat = rustix::fs::fstat(&parent).map_err(|error| {
                    effect_error("transaction_parent_stat_failed", error, false)
                })?;
                (
                    Some(Arc::new(parent)),
                    Some((stat.st_dev as u64, stat.st_ino as u64)),
                )
            }
            Err(error) if error.code == "transaction_parent_missing" => (None, None),
            Err(error) => return Err(error),
        };
        let target_identity = match &held_parent {
            Some(parent) => {
                let name = relative.file_name().ok_or_else(|| {
                    blocked(
                        "transaction_target_name_invalid",
                        relative.display().to_string(),
                    )
                })?;
                let identity = identity_at(parent, name)?;
                if identity.is_some() {
                    let stat = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|error| {
                            effect_error("transaction_target_stat_failed", error, false)
                        })?;
                    let kind = FileType::from_raw_mode(stat.st_mode);
                    if !(kind.is_file() || kind.is_dir() || allow_symlink_leaf && kind.is_symlink())
                    {
                        return Err(blocked(
                            "transaction_target_not_regular",
                            relative.display().to_string(),
                        ));
                    }
                }
                identity
            }
            None => None,
        };
        Ok(AnchoredPath {
            sealed: root.canonical.join(relative).display().to_string(),
            root,
            relative: relative.to_path_buf(),
            parent_anchor: Arc::new(Mutex::new(ParentAnchor {
                descriptor: held_parent,
                identity: held_parent_identity,
            })),
            target_identity: Arc::new(Mutex::new(target_identity)),
        })
    }

    fn anchored_symlink_target(
        &self,
        binding: &AuthenticatedBinding,
        absolute: &Path,
    ) -> Result<AnchoredPath, EffectError> {
        let (label, root_path, relative) =
            if let Ok(relative) = absolute.strip_prefix(binding.canonical_workspace()) {
                ("workspace", binding.canonical_workspace(), relative)
            } else if let Ok(relative) = absolute.strip_prefix(&self.runtime_home) {
                ("runtime", self.runtime_home.as_path(), relative)
            } else if let Some((root, relative)) = binding
                .authorized_write_roots()
                .iter()
                .filter_map(|root| {
                    absolute
                        .strip_prefix(root)
                        .ok()
                        .map(|relative| (root, relative))
                })
                .max_by_key(|(root, _)| root.components().count())
            {
                ("authorized", root.as_path(), relative)
            } else {
                return Err(blocked(
                    "transaction_target_outside_capability",
                    absolute.display().to_string(),
                ));
            };
        let root = self.root_capability(label, root_path)?;
        self.path_in_root_symlink(root, relative)
    }

    fn journal_write_target(
        &self,
        binding: &AuthenticatedBinding,
        write: &JournalWrite,
    ) -> Result<AnchoredPath, EffectError> {
        if matches!(write.preimage, JournalImage::Symlink { .. })
            || matches!(write.postimage, JournalPostimage::Symlink { .. })
        {
            self.anchored_symlink_target(binding, Path::new(&write.path))
        } else {
            self.anchored_target(binding, Path::new(&write.path))
        }
    }

    fn anchored_target(
        &self,
        binding: &AuthenticatedBinding,
        absolute: &Path,
    ) -> Result<AnchoredPath, EffectError> {
        let (label, root_path, relative) =
            if let Ok(relative) = absolute.strip_prefix(binding.canonical_workspace()) {
                ("workspace", binding.canonical_workspace(), relative)
            } else if let Ok(relative) = absolute.strip_prefix(&self.runtime_home) {
                ("runtime", self.runtime_home.as_path(), relative)
            } else if let Some((root, relative)) = binding
                .authorized_write_roots()
                .iter()
                .filter_map(|root| {
                    absolute
                        .strip_prefix(root)
                        .ok()
                        .map(|relative| (root, relative))
                })
                .max_by_key(|(root, _)| root.components().count())
            {
                ("authorized", root.as_path(), relative)
            } else {
                return Err(blocked(
                    "transaction_target_outside_capability",
                    absolute.display().to_string(),
                ));
            };
        let root = self.root_capability(label, root_path)?;
        self.path_in_root(root, relative)
    }

    fn journal_target(
        &self,
        action_ref: &str,
        extension: &str,
    ) -> Result<AnchoredPath, EffectError> {
        if action_ref.is_empty()
            || !action_ref
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(blocked("transaction_action_ref_invalid", action_ref));
        }
        let root = self.root_capability("runtime", &self.runtime_home)?;
        self.path_in_root(
            root,
            &PathBuf::from(format!(".ags-transaction-{action_ref}.{extension}")),
        )
    }

    fn prepare_journal(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &ProductionAction,
        binding: &AuthenticatedBinding,
    ) -> Result<(), EffectError> {
        let ordered_writes = journal_writes(action)?;
        if ordered_writes.is_empty() {
            return Err(blocked(
                "transaction_journal_empty",
                "transaction action has no journalable writes",
            ));
        }
        if ordered_writes
            .iter()
            .map(|write| write.path.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != ordered_writes.len()
        {
            return Err(blocked("transaction_journal_duplicate_path", action_ref));
        }
        let mut journal = TransactionJournal {
            schema_version: TRANSACTION_JOURNAL_SCHEMA.to_string(),
            action_ref: action_ref.to_string(),
            binding_hash: plan.binding_hash.clone(),
            canonical_workspace: binding.canonical_workspace.display().to_string(),
            workspace_identity: binding.workspace_identity.clone(),
            registry_key: binding.registry_key.clone(),
            plan_hash: plan.plan_hash.clone(),
            policy_hash: plan.policy_hash.clone(),
            payload_hash: plan.payload_hash.clone(),
            operation: plan.operation.as_str().to_string(),
            ordered_writes,
            identity_digest: String::new(),
            phase: JournalPhase::Prepared,
            recovery_generation: 0,
            output_digest: None,
            terminal_recovery: None,
            integrity: String::new(),
        };
        journal.identity_digest = journal.recompute_identity_digest()?;
        journal.reseal()?;
        let target = self.journal_target(action_ref, "json")?;
        if anchored_read(&target, false)?.is_some() {
            return Err(blocked("transaction_journal_already_exists", action_ref));
        }
        let bytes = serde_json::to_vec_pretty(&journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        anchored_write(&target, None, &bytes)
    }

    fn load_journal(
        &self,
        action_ref: &str,
    ) -> Result<(AnchoredPath, Vec<u8>, TransactionJournal), EffectError> {
        let target = self.journal_target(action_ref, "json")?;
        let bytes = anchored_read(&target, false)?
            .ok_or_else(|| blocked("transaction_journal_missing", action_ref))?;
        let journal: TransactionJournal = serde_json::from_slice(&bytes)
            .map_err(|error| blocked("transaction_journal_invalid", error.to_string()))?;
        journal.verify_integrity()?;
        if journal.schema_version != TRANSACTION_JOURNAL_SCHEMA || journal.action_ref != action_ref
        {
            return Err(blocked("transaction_journal_identity_mismatch", action_ref));
        }
        Ok((target, bytes, journal))
    }

    fn transition_journal(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        phase: JournalPhase,
        output_digest: Option<&str>,
    ) -> Result<TransactionJournal, EffectError> {
        let (target, previous, mut journal) = self.load_journal(action_ref)?;
        if journal.binding_hash != plan.binding_hash
            || journal.plan_hash != plan.plan_hash
            || journal.policy_hash != plan.policy_hash
            || journal.payload_hash != plan.payload_hash
            || journal.operation != plan.operation.as_str()
        {
            return Err(blocked("transaction_journal_identity_mismatch", action_ref));
        }
        journal.phase = phase;
        journal.output_digest = output_digest.map(str::to_string);
        journal.reseal()?;
        let bytes = serde_json::to_vec_pretty(&journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        anchored_write(&target, Some(&previous), &bytes)?;
        Ok(journal)
    }

    fn transition_journal_applied(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &ProductionAction,
        output_digest: &str,
    ) -> Result<TransactionJournal, EffectError> {
        #[cfg(test)]
        if FAIL_NEXT_JOURNAL_APPLIED.with(|flag| flag.replace(false)) {
            return Err(effect_error(
                "transaction_journal_applied_failed",
                "injected applied-journal transition failure",
                true,
            ));
        }
        let (target, previous, mut journal) = self.load_journal(action_ref)?;
        if journal.binding_hash != plan.binding_hash
            || journal.plan_hash != plan.plan_hash
            || journal.policy_hash != plan.policy_hash
            || journal.payload_hash != plan.payload_hash
            || journal.operation != plan.operation.as_str()
        {
            return Err(blocked("transaction_journal_identity_mismatch", action_ref));
        }
        let targets = action_targets(action);
        for write in &mut journal.ordered_writes {
            let action_target = targets
                .get(&write.path)
                .ok_or_else(|| blocked("transaction_journal_write_set_mismatch", action_ref))?;
            if !postimage_matches(&write.postimage, action_target)? {
                return Err(blocked("transaction_applied_postimage_drift", &write.path));
            }
            write.post_identity = target_identity_now(action_target)?;
            if !matches!(write.postimage, JournalPostimage::Absent) && write.post_identity.is_none()
            {
                return Err(blocked(
                    "transaction_applied_postimage_missing",
                    &write.path,
                ));
            }
            let parent_chain = target_parent_chain(action_target)?;
            match &write.apply_anchor {
                JournalApplyAnchor::Pending => {
                    write.apply_anchor = JournalApplyAnchor::Applied { parent_chain };
                }
                JournalApplyAnchor::Applied {
                    parent_chain: sealed,
                } if *sealed == parent_chain => {}
                JournalApplyAnchor::Applied { .. } => {
                    return Err(blocked(
                        "transaction_applied_parent_chain_changed",
                        &write.path,
                    ));
                }
            }
        }
        if journal.ordered_writes.iter().any(|write| {
            !matches!(write.apply_anchor, JournalApplyAnchor::Applied { .. })
                || !matches!(
                    write.recovery_progress,
                    JournalWriteRecoveryProgress::Applied
                )
        }) {
            return Err(blocked("transaction_applied_anchor_incomplete", action_ref));
        }
        journal.phase = JournalPhase::Applied;
        journal.output_digest = Some(output_digest.to_string());
        journal.identity_digest = journal.recompute_identity_digest()?;
        journal.reseal()?;
        let bytes = serde_json::to_vec_pretty(&journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        anchored_write(&target, Some(&previous), &bytes)?;
        Ok(journal)
    }

    fn record_journal_post_identity(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        target_path: &str,
        action_target: &AnchoredPath,
    ) -> Result<(), EffectError> {
        let (journal_target, previous, mut journal) = self.load_journal(action_ref)?;
        if journal.phase != JournalPhase::Applying
            || journal.binding_hash != plan.binding_hash
            || journal.plan_hash != plan.plan_hash
            || journal.policy_hash != plan.policy_hash
            || journal.payload_hash != plan.payload_hash
        {
            return Err(blocked("transaction_journal_identity_mismatch", action_ref));
        }
        let write = journal
            .ordered_writes
            .iter_mut()
            .find(|write| write.path == target_path)
            .ok_or_else(|| blocked("transaction_journal_write_set_mismatch", target_path))?;
        validate_journal_root(write, action_target)?;
        if !postimage_matches(&write.postimage, action_target)? {
            return Err(blocked("transaction_applied_postimage_drift", target_path));
        }
        let identity = target_identity_now(action_target)?;
        if !matches!(write.postimage, JournalPostimage::Absent) && identity.is_none() {
            return Err(blocked(
                "transaction_applied_postimage_missing",
                target_path,
            ));
        }
        if write.post_identity.is_some() && write.post_identity != identity {
            return Err(blocked("transaction_applied_identity_changed", target_path));
        }
        write.post_identity = identity;
        write.apply_anchor = JournalApplyAnchor::Applied {
            parent_chain: target_parent_chain(action_target)?,
        };
        journal.identity_digest = journal.recompute_identity_digest()?;
        journal.reseal()?;
        let bytes = serde_json::to_vec_pretty(&journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        anchored_write(&journal_target, Some(&previous), &bytes)
    }

    fn commit_journal(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        output_digest: &str,
    ) -> Result<(), EffectError> {
        let journal = self.transition_journal(
            action_ref,
            plan,
            JournalPhase::Verified,
            Some(output_digest),
        )?;
        let marker = self.journal_target(action_ref, "commit")?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": "ags://schema/contract/v2/transaction-commit",
            "action_ref": action_ref,
            "binding_hash": plan.binding_hash,
            "plan_hash": plan.plan_hash,
            "journal_integrity": journal.integrity,
            "output_digest": output_digest,
        }))
        .map_err(|error| blocked("transaction_commit_encode_failed", error.to_string()))?;
        match anchored_read(&marker, false)? {
            Some(existing) if existing == bytes => Ok(()),
            Some(_) => Err(blocked("transaction_commit_marker_conflict", action_ref)),
            None => anchored_write(&marker, None, &bytes),
        }
    }

    fn recover_journal(
        &self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &ProductionAction,
    ) -> Result<RecoveryObservation, EffectError> {
        let (journal_target, mut journal_bytes, mut journal) = self.load_journal(action_ref)?;
        let journal_state_digest_at_open = journal.state_digest()?;
        if journal.binding_hash != plan.binding_hash
            || journal.plan_hash != plan.plan_hash
            || journal.policy_hash != plan.policy_hash
            || journal.payload_hash != plan.payload_hash
        {
            return Err(blocked("transaction_journal_identity_mismatch", action_ref));
        }
        let targets = action_targets(action);
        if journal
            .ordered_writes
            .iter()
            .any(|write| !targets.contains_key(&write.path))
        {
            return Err(blocked(
                "transaction_journal_write_set_mismatch",
                action_ref,
            ));
        }
        for write in &journal.ordered_writes {
            validate_journal_root(write, &targets[&write.path])?;
        }
        let marker = self.journal_target(action_ref, "commit")?;
        if let Some(marker_bytes) = anchored_read(&marker, false)? {
            let marker: serde_json::Value = serde_json::from_slice(&marker_bytes)
                .map_err(|error| blocked("transaction_commit_marker_invalid", error.to_string()))?;
            let marker_valid = marker.get("action_ref").and_then(serde_json::Value::as_str)
                == Some(action_ref)
                && marker
                    .get("binding_hash")
                    .and_then(serde_json::Value::as_str)
                    == Some(plan.binding_hash.as_str())
                && marker.get("plan_hash").and_then(serde_json::Value::as_str)
                    == Some(plan.plan_hash.as_str())
                && marker
                    .get("journal_integrity")
                    .and_then(serde_json::Value::as_str)
                    == Some(journal.integrity.as_str());
            if !marker_valid {
                return Err(blocked("transaction_commit_marker_invalid", action_ref));
            }
            for write in &journal.ordered_writes {
                let target = &targets[&write.path];
                if !journal_postimage_matches(write, target)? {
                    return Err(blocked(
                        "transaction_committed_postimage_drift",
                        &write.path,
                    ));
                }
            }
            return Err(blocked("transaction_already_committed", action_ref));
        }

        let mut actual_writes = match execute_exact_recovery(
            &journal_target,
            &mut journal_bytes,
            &mut journal,
            &targets,
        ) {
            Ok(writes) => writes,
            Err(mut error) => {
                let digest = sha256(format!(
                    "transaction-recovery-risk\n{}\n{}",
                    error.code, error.detail
                ));
                if let Err(mut persist_error) = self.persist_recovered_journal(
                    &journal_target,
                    &journal_bytes,
                    &mut journal,
                    JournalPhase::RiskEscalated,
                    &digest,
                ) {
                    persist_error.effect_started = true;
                    persist_error
                        .observed_write_set
                        .append(&mut error.observed_write_set);
                    persist_error.observed_write_set.sort();
                    persist_error.observed_write_set.dedup();
                    return Err(persist_error);
                }
                error.effect_started = true;
                error.output_digest = digest;
                error.observed_write_set.push(journal_target.sealed.clone());
                error.observed_write_set.sort();
                error.observed_write_set.dedup();
                return Err(error);
            }
        };
        actual_writes.push(journal_target.sealed.clone());
        actual_writes.sort();
        actual_writes.dedup();
        Ok(RecoveryObservation {
            succeeded: true,
            output_digest: sha256("transaction-recovered"),
            observed_write_set: actual_writes,
            evidence: Some(serde_json::json!({
                "recovery_action": action_ref,
                "journal_phase": "awaiting_durable_terminal",
                "journal_identity_digest": journal.identity_digest,
                "journal_state_digest": journal.state_digest()?,
            })),
            original_journal_digest: Some(journal_state_digest_at_open),
        })
    }

    fn persist_recovered_journal(
        &self,
        target: &AnchoredPath,
        previous: &[u8],
        journal: &mut TransactionJournal,
        phase: JournalPhase,
        output_digest: &str,
    ) -> Result<(), EffectError> {
        journal.phase = phase;
        journal.output_digest = Some(output_digest.to_string());
        journal.reseal()?;
        let bytes = serde_json::to_vec_pretty(journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        #[cfg(test)]
        if phase == JournalPhase::RiskEscalated
            && FAIL_NEXT_RISK_JOURNAL_WRITE.with(|flag| flag.replace(false))
        {
            return Err(effect_error(
                "transaction_risk_journal_write_failed",
                "injected risk-journal persistence failure",
                false,
            ));
        }
        anchored_write(target, Some(previous), &bytes)
    }

    fn finalize_exact_recovery(
        &self,
        recovery_action_ref: &str,
        plan: &SealedPlan,
        action: &ProductionAction,
        binding: &AuthenticatedBinding,
        receipt: &OperationReceipt,
    ) -> Result<(), EffectError> {
        let (
            original_action_ref,
            sealed_identity_digest,
            sealed_state_digest,
            pending_expected_paths,
        ) = match action {
            ProductionAction::PendingRecovery {
                original_action_ref,
                journal_identity_digest,
                journal_state_digest,
                expected_write_paths,
            } => (
                original_action_ref.as_str(),
                Some(journal_identity_digest.as_str()),
                Some(journal_state_digest.as_str()),
                Some(expected_write_paths),
            ),
            _ => (recovery_action_ref, None, None, None),
        };
        let (target, previous, mut journal) = self.load_journal(original_action_ref)?;
        let current_state_digest = journal.state_digest()?;
        let evidence = receipt
            .evidence
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| blocked("recovery_finalizer_receipt_mismatch", original_action_ref))?;
        let receipt_original_state = evidence
            .get("original_journal_digest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| blocked("recovery_finalizer_receipt_mismatch", original_action_ref))?;
        let receipt_identity = evidence
            .get("journal_identity_digest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| blocked("recovery_finalizer_receipt_mismatch", original_action_ref))?;
        let receipt_final_state = evidence
            .get("journal_state_digest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| blocked("recovery_finalizer_receipt_mismatch", original_action_ref))?;
        if sealed_identity_digest.is_some_and(|digest| digest != journal.identity_digest)
            || sealed_state_digest.is_some_and(|digest| digest != receipt_original_state)
            || receipt_identity != journal.identity_digest
            || receipt_final_state != current_state_digest
            || journal.phase == JournalPhase::Verified
            || journal.phase == JournalPhase::Recovered
            || journal.phase == JournalPhase::RiskEscalated
            || journal.canonical_workspace != binding.canonical_workspace.display().to_string()
            || journal.workspace_identity != binding.workspace_identity
            || journal.registry_key != binding.registry_key
            || journal.operation != plan.operation.as_str()
            || plan.binding_hash != sha256(binding.canonical_bytes())
            || (pending_expected_paths.is_none()
                && (journal.binding_hash != plan.binding_hash
                    || journal.plan_hash != plan.plan_hash
                    || journal.policy_hash != plan.policy_hash
                    || journal.payload_hash != plan.payload_hash))
        {
            return Err(blocked(
                "recovery_finalizer_identity_mismatch",
                original_action_ref,
            ));
        }
        let mut journal_write_paths = journal
            .ordered_writes
            .iter()
            .map(|write| write.path.clone())
            .collect::<Vec<_>>();
        journal_write_paths.sort();
        journal_write_paths.dedup();
        let mut sealed_paths = journal_write_paths.clone();
        sealed_paths.push(target.sealed.clone());
        sealed_paths.sort();
        sealed_paths.dedup();
        let expected_paths = if let Some(expected_write_paths) = pending_expected_paths {
            let mut paths = expected_write_paths.clone();
            paths.sort();
            paths.dedup();
            paths
        } else {
            let mut plan_paths = plan.expected_write_paths.clone();
            plan_paths.sort();
            plan_paths.dedup();
            if plan_paths != journal_write_paths {
                return Err(blocked(
                    "recovery_finalizer_receipt_mismatch",
                    original_action_ref,
                ));
            }
            sealed_paths.clone()
        };
        let observed = receipt
            .observed_write_set
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if sealed_paths != expected_paths
            || !observed.contains(&target.sealed)
            || observed
                .iter()
                .any(|path| expected_paths.binary_search(path).is_err())
            || receipt.operation.as_str() != journal.operation
            || receipt.status != ReceiptStatus::Recovered
            || !receipt.recovered
            || receipt.plan_hash != plan.plan_hash
            || receipt.payload_hash != plan.payload_hash
            || receipt.binding_hash != plan.binding_hash
        {
            return Err(blocked(
                "recovery_finalizer_receipt_mismatch",
                original_action_ref,
            ));
        }
        for write in &journal.ordered_writes {
            let restored = self.journal_write_target(binding, write)?;
            validate_journal_root(write, &restored)?;
            if !preimage_matches(&write.preimage, &restored)? {
                return Err(blocked(
                    "transaction_recovery_postcondition_failed",
                    &write.path,
                ));
            }
        }
        journal.phase = JournalPhase::Recovered;
        journal.output_digest = Some(receipt.output_digest.clone());
        journal.terminal_recovery = Some(TerminalRecoveryRecord {
            original_action_ref: journal.action_ref.clone(),
            original_operation: journal.operation.clone(),
            original_binding_hash: journal.binding_hash.clone(),
            original_policy_hash: journal.policy_hash.clone(),
            original_journal_digest: receipt_original_state.to_string(),
            journal_identity_digest: journal.identity_digest.clone(),
            journal_state_digest: current_state_digest,
            recovery_action_ref: recovery_action_ref.to_string(),
            recovery_plan_hash: plan.plan_hash.clone(),
            recovery_payload_hash: plan.payload_hash.clone(),
            recovery_binding_hash: plan.binding_hash.clone(),
            recovery_policy_hash: plan.policy_hash.clone(),
            receipt: receipt.clone(),
        });
        validate_terminal_recovery_record(
            &journal,
            journal
                .terminal_recovery
                .as_ref()
                .expect("terminal recovery was just constructed"),
        )?;
        journal.reseal()?;
        let bytes = serde_json::to_vec_pretty(&journal)
            .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
        #[cfg(test)]
        if FAIL_NEXT_RECOVERY_FINALIZE.with(|flag| flag.replace(false)) {
            return Err(effect_error(
                "transaction_recovery_finalize_failed",
                "injected durable recovery-terminal failure",
                false,
            ));
        }
        anchored_write(&target, Some(&previous), &bytes)?;
        let durable = anchored_read(&target, false)?
            .ok_or_else(|| blocked("transaction_journal_missing", original_action_ref))?;
        let durable_journal: TransactionJournal = serde_json::from_slice(&durable)
            .map_err(|error| blocked("transaction_journal_invalid", error.to_string()))?;
        durable_journal.verify_integrity()?;
        let terminal = durable_journal
            .terminal_recovery
            .as_ref()
            .ok_or_else(|| blocked("terminal_recovery_receipt_missing", original_action_ref))?;
        validate_terminal_recovery_record(&durable_journal, terminal)
    }

    fn recover_exact_pending_transaction(
        &self,
        binding: &AuthenticatedBinding,
        expected_action_ref: &str,
        expected_identity_digest: &str,
        expected_state_digest: &str,
    ) -> Result<EffectObservation, EffectError> {
        let root = self.root_capability("runtime", &self.runtime_home)?;
        let directory = rustix::io::dup(root.descriptor.as_ref())
            .map_err(|error| effect_error("transaction_root_dup_failed", error, false))?;
        let directory_identity = parent_identity(&directory)?;
        let mut names = Vec::new();
        let mut name_bytes = 0usize;
        for entry in rustix::fs::Dir::read_from(&directory).map_err(|error| {
            effect_error("transaction_journal_directory_read_failed", error, false)
        })? {
            let entry = entry.map_err(|error| {
                effect_error("transaction_journal_directory_read_failed", error, false)
            })?;
            let name = entry
                .file_name()
                .to_str()
                .map(str::to_string)
                .map_err(|_| {
                    blocked(
                        "transaction_journal_name_invalid",
                        "journal name is not UTF-8",
                    )
                })?;
            if name == "." || name == ".." {
                continue;
            }
            if names.len() >= MAX_TRANSACTION_JOURNAL_ENTRIES {
                return Err(blocked(
                    "transaction_journal_entry_budget_exceeded",
                    "runtime directory exceeds the journal enumeration entry budget",
                ));
            }
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > MAX_TRANSACTION_JOURNAL_NAME_BYTES {
                return Err(blocked(
                    "transaction_journal_entry_budget_exceeded",
                    "runtime directory exceeds the journal enumeration name-byte budget",
                ));
            }
            names.push(name);
        }
        names.sort();
        for name in names {
            let Some(action_ref) = name
                .strip_prefix(".ags-transaction-")
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if action_ref.is_empty()
                || !action_ref
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(blocked("transaction_journal_name_invalid", name));
            }
            let journal_target = AnchoredPath {
                root: root.clone(),
                relative: PathBuf::from(&name),
                sealed: self.runtime_home.join(&name).display().to_string(),
                parent_anchor: Arc::new(Mutex::new(ParentAnchor {
                    descriptor: Some(Arc::new(rustix::io::dup(&directory).map_err(|error| {
                        effect_error("transaction_parent_dup_failed", error, false)
                    })?)),
                    identity: Some(directory_identity),
                })),
                target_identity: Arc::new(Mutex::new(identity_at(&directory, OsStr::new(&name))?)),
            };
            let raw = anchored_read(&journal_target, false)?
                .ok_or_else(|| blocked("transaction_journal_missing", action_ref))?;
            let preview: TransactionJournal = serde_json::from_slice(&raw)
                .map_err(|error| blocked("transaction_journal_invalid", error.to_string()))?;
            preview.verify_integrity()?;
            if preview.schema_version != TRANSACTION_JOURNAL_SCHEMA
                || preview.action_ref != action_ref
            {
                return Err(blocked("transaction_journal_identity_mismatch", action_ref));
            }
            if preview.canonical_workspace != binding.canonical_workspace.display().to_string()
                || preview.workspace_identity != binding.workspace_identity
            {
                continue;
            }
            if preview.registry_key != binding.registry_key {
                return Err(blocked(
                    "transaction_journal_registry_mismatch",
                    &preview.action_ref,
                ));
            }
            let mut previous = raw;
            let mut journal = preview;
            if journal.phase == JournalPhase::Recovered {
                let terminal = journal.terminal_recovery.as_ref().ok_or_else(|| {
                    blocked("terminal_recovery_receipt_missing", &journal.action_ref)
                })?;
                validate_terminal_recovery_record(&journal, terminal)?;
                continue;
            }
            if journal.phase == JournalPhase::Verified {
                continue;
            }
            if journal.phase == JournalPhase::RiskEscalated {
                return Err(blocked(
                    "transaction_recovery_risk_requires_operator",
                    &journal.action_ref,
                ));
            }
            let mut targets = std::collections::BTreeMap::new();
            for write in &journal.ordered_writes {
                let target = self.journal_write_target(binding, write)?;
                validate_journal_root(write, &target)?;
                targets.insert(write.path.clone(), target);
            }
            let marker = sibling_target(
                &journal_target,
                &format!(".ags-transaction-{}.commit", journal.action_ref),
            )?;
            if let Some(marker_bytes) = anchored_read(&marker, false)? {
                verify_commit_marker(&marker_bytes, &journal)?;
                for write in &journal.ordered_writes {
                    if !journal_postimage_matches(write, &targets[&write.path])? {
                        return Err(blocked(
                            "transaction_committed_postimage_drift",
                            &write.path,
                        ));
                    }
                }
                continue;
            }
            if journal.action_ref != expected_action_ref {
                return Err(blocked(
                    "pending_transaction_order_mismatch",
                    format!(
                        "sealed recovery `{expected_action_ref}` cannot consume active `{}`",
                        journal.action_ref
                    ),
                ));
            }
            if journal.identity_digest != expected_identity_digest
                || journal.state_digest()? != expected_state_digest
            {
                return Err(blocked(
                    "pending_transaction_identity_mismatch",
                    &journal.action_ref,
                ));
            }

            let mut actual_writes = match execute_exact_recovery(
                &journal_target,
                &mut previous,
                &mut journal,
                &targets,
            ) {
                Ok(writes) => writes,
                Err(mut error) => {
                    let digest = sha256(format!(
                        "transaction-recovery-risk\n{}\n{}",
                        error.code, error.detail
                    ));
                    if let Err(mut persist_error) = self.persist_recovered_journal(
                        &journal_target,
                        &previous,
                        &mut journal,
                        JournalPhase::RiskEscalated,
                        &digest,
                    ) {
                        persist_error.effect_started = true;
                        persist_error
                            .observed_write_set
                            .append(&mut error.observed_write_set);
                        persist_error.observed_write_set.sort();
                        persist_error.observed_write_set.dedup();
                        return Err(persist_error);
                    }
                    error.effect_started = true;
                    error.output_digest = digest;
                    error.observed_write_set.push(journal_target.sealed.clone());
                    error.observed_write_set.sort();
                    error.observed_write_set.dedup();
                    return Err(error);
                }
            };
            actual_writes.push(journal_target.sealed.clone());
            actual_writes.sort();
            actual_writes.dedup();
            return EffectObservation::bounded(
                true,
                true,
                sha256("transaction-recovered"),
                actual_writes,
                Some(serde_json::json!({
                    "recovery_action": expected_action_ref,
                    "journal_phase": "awaiting_durable_terminal",
                    "journal_identity_digest": journal.identity_digest,
                    "journal_state_digest": journal.state_digest()?,
                })),
            );
        }
        Err(blocked("pending_transaction_missing", expected_action_ref))
    }

    #[cfg(test)]
    fn recover_pending_transactions(
        &self,
        binding: &AuthenticatedBinding,
    ) -> Result<(), EffectError> {
        let inspection = self.inspect_pending_transaction(binding)?;
        let Some(pending) = inspection.active else {
            return Ok(());
        };
        let ProductionAction::PendingRecovery {
            original_action_ref,
            journal_identity_digest,
            journal_state_digest,
            ..
        } = pending.action
        else {
            unreachable!("pending inspection only returns recovery actions")
        };
        self.recover_exact_pending_transaction(
            binding,
            &original_action_ref,
            &journal_identity_digest,
            &journal_state_digest,
        )
        .map(|_| ())
    }

    fn file_plan(
        &self,
        binding: &AuthenticatedBinding,
        writes: Vec<(PathBuf, Vec<u8>)>,
        description: &str,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let mut planned = Vec::new();
        for (path, content) in writes {
            let target = self.anchored_target(binding, &path)?;
            let previous = anchored_read(&target, false)?;
            if previous.as_deref() == Some(content.as_slice()) {
                continue;
            }
            planned.push(PlannedWrite {
                target,
                content,
                previous,
            });
        }
        if planned.is_empty() {
            return Ok(PlanDisposition::NoChange {
                output_digest: sha256(description),
            });
        }
        let expected_write_paths = planned
            .iter()
            .map(|write| write.target.sealed.clone())
            .collect();
        let action = ProductionAction::Files(planned);
        let action_digest = canonical_production_action_digest(&action)?;
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest,
                steps: vec![PlanStep {
                    step_id: "write-owned-projection".to_string(),
                    description: description.to_string(),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec!["content-hash-equals-sealed-plan".to_string()],
                },
                recoverability: Recoverability::Transactional,
                execution: None,
            },
            action,
        })))
    }

    fn delete_plan(
        &self,
        binding: &AuthenticatedBinding,
        paths: Vec<PathBuf>,
        description: &str,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let mut deletes = Vec::new();
        for path in paths {
            let target = self.anchored_target(binding, &path)?;
            if let Some(previous) = anchored_read(&target, false)? {
                deletes.push(PlannedDelete { target, previous });
            }
        }
        if deletes.is_empty() {
            return Ok(PlanDisposition::NoChange {
                output_digest: sha256(description),
            });
        }
        let expected_write_paths = deletes
            .iter()
            .map(|delete| delete.target.sealed.clone())
            .collect();
        let action = ProductionAction::Deletes(deletes);
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest: canonical_production_action_digest(&action)?,
                steps: vec![PlanStep {
                    step_id: "remove-exact-owned-state".to_string(),
                    description: description.to_string(),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec!["sealed-file-is-absent".to_string()],
                },
                recoverability: Recoverability::Transactional,
                execution: None,
            },
            action,
        })))
    }

    fn init_plan(
        &self,
        request: &InitRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let workspace = binding.canonical_workspace();
        let agents = MANAGED_ENTRY.trim_start().as_bytes().to_vec();
        let profile_path = workspace.join("config/agent-project-profile.yaml");
        if profile_path.exists()
            && read_binding_text(
                binding,
                &profile_path,
                MAX_PROFILE_BYTES,
                "init_profile_read_failed",
            )
            .is_ok_and(|body| !body.contains("ags://schema/contract/v2/project-profile"))
        {
            return Err(blocked(
                "init_profile_user_owned",
                format!(
                    "refusing to replace non-contract-v2 profile {}",
                    profile_path.display()
                ),
            ));
        }
        let desired = vec![
            ags_verification::ProjectProjectionFile::write("AGENTS.md", agents),
            ags_verification::ProjectProjectionFile::write(
                "config/agent-project-profile.yaml",
                PROFILE.as_bytes(),
            ),
            ags_verification::ProjectProjectionFile::write(".ags/evidence/.keep", []),
            ags_verification::ProjectProjectionFile::write(".ags/state/closure-pointers/.keep", []),
        ];
        let plan = ags_verification::plan_project_projection(workspace, &desired)
            .map_err(|detail| blocked("init_projection_plan_failed", detail))?;
        if request.migration == MigrationMode::None && !plan.conflicts().is_empty() {
            return Err(blocked(
                "init_projection_conflict",
                "existing unowned projection paths require explicit exact-owned migration",
            ));
        }
        let planned_directory_paths = plan
            .planned_directories()
            .iter()
            .map(|relative| workspace.join(relative).display().to_string())
            .collect::<Vec<_>>();
        let mutations = plan
            .materialized_mutations()
            .into_iter()
            .map(|mutation| {
                use ags_verification::ProjectProjectionMutation;
                match mutation {
                    ProjectProjectionMutation::CreateDirectory {
                        relative_path,
                        mode,
                    } => {
                        let target =
                            self.anchored_target(binding, &workspace.join(relative_path))?;
                        let preimage = target_journal_image(&target)?;
                        if preimage != JournalImage::Absent {
                            return Err(blocked(
                                "projection_materialized_preimage_drift",
                                &target.sealed,
                            ));
                        }
                        Ok(PlannedObjectMutation {
                            target,
                            operation: "create_directory".to_string(),
                            preimage,
                            postimage: JournalPostimage::Directory { mode },
                            apply: ObjectMutationApply::CreateDirectory,
                        })
                    }
                    ProjectProjectionMutation::WriteFile {
                        relative_path,
                        previous_bytes,
                        next_bytes,
                        mode,
                    } => {
                        let target =
                            self.anchored_target(binding, &workspace.join(relative_path))?;
                        let preimage = target_journal_image(&target)?;
                        let expected_preimage_matches = match (&preimage, previous_bytes.as_deref())
                        {
                            (JournalImage::Absent, None) => true,
                            (JournalImage::RegularFile { data_hex, .. }, Some(previous)) => {
                                decode_hex(data_hex)? == previous
                            }
                            _ => false,
                        };
                        if !expected_preimage_matches {
                            return Err(blocked(
                                "projection_materialized_preimage_drift",
                                &target.sealed,
                            ));
                        }
                        Ok(PlannedObjectMutation {
                            target,
                            operation: if previous_bytes.is_some() {
                                "replace_file".to_string()
                            } else {
                                "create_file".to_string()
                            },
                            preimage,
                            postimage: JournalPostimage::RegularFile {
                                sha256: sha256(&next_bytes),
                                mode,
                            },
                            apply: ObjectMutationApply::WriteFile {
                                previous: previous_bytes,
                                next: next_bytes,
                            },
                        })
                    }
                    ProjectProjectionMutation::DeleteFile {
                        relative_path,
                        previous_bytes,
                    } => {
                        let target =
                            self.anchored_target(binding, &workspace.join(relative_path))?;
                        let preimage = target_journal_image(&target)?;
                        if !matches!(
                            &preimage,
                            JournalImage::RegularFile { data_hex, .. }
                                if decode_hex(data_hex)? == previous_bytes
                        ) {
                            return Err(blocked(
                                "projection_materialized_preimage_drift",
                                &target.sealed,
                            ));
                        }
                        Ok(PlannedObjectMutation {
                            target,
                            operation: "delete_file".to_string(),
                            preimage,
                            postimage: JournalPostimage::Absent,
                            apply: ObjectMutationApply::DeleteFile {
                                previous: previous_bytes,
                            },
                        })
                    }
                }
            })
            .collect::<Result<Vec<_>, EffectError>>()?;
        let mut expected_write_paths = mutations
            .iter()
            .map(|mutation| mutation.target.sealed.clone())
            .collect::<Vec<_>>();
        expected_write_paths.sort();
        expected_write_paths.dedup();
        let projection = PlannedProjection {
            workspace: workspace.to_path_buf(),
            planned_directory_paths,
            mutations,
        };
        let action = ProductionAction::Projections(vec![projection]);
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest: canonical_production_action_digest(&action)?,
                steps: vec![PlanStep {
                    step_id: "project-lightweight-projection".to_string(),
                    description:
                        "create or reclaim only exact AGS-owned contract-v2 projection files"
                            .to_string(),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec!["fd-relative-content-hash-equals-plan".to_string()],
                },
                recoverability: Recoverability::Transactional,
                execution: None,
            },
            action,
        })))
    }

    fn setup_plan(
        &self,
        request: &SetupRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let path = self.runtime_home.join("install-manifest.json");
        if path.exists()
            && read_binding_text(
                binding,
                &path,
                MAX_PROFILE_BYTES,
                "setup_manifest_read_failed",
            )
            .is_ok_and(|body| !body.contains("ags://schema/contract/v2/runtime-install"))
        {
            return Err(blocked(
                "setup_legacy_install_requires_migration",
                format!(
                    "refusing to overwrite non-contract-v2 manifest {}",
                    path.display()
                ),
            ));
        }
        let body = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "ags://schema/contract/v2/runtime-install",
            "contract_version": CONTRACT_SCHEMA_VERSION,
            "canonical_workspace": binding.canonical_workspace(),
            "approved_hosts": request.approved_hosts,
            "third_party_mcp_policy": "advice-only"
        }))
        .map_err(|error| blocked("setup_manifest_encode_failed", error.to_string()))?;
        let key_path = self.runtime_home.join(CLOSURE_AUTHORITY_KEY_FILE);
        let runtime_root = self.root_capability("runtime", &self.runtime_home)?;
        let key_target = self.path_in_root(runtime_root, Path::new(CLOSURE_AUTHORITY_KEY_FILE))?;
        let key = anchored_read(&key_target, false)?;
        let mut writes = vec![(path, body)];
        match key {
            Some(bytes) if bytes.len() == 32 => {}
            Some(bytes) => {
                return Err(blocked(
                    "closure_authority_invalid",
                    format!(
                        "machine closure authority must be exactly 32 bytes, got {}",
                        bytes.len()
                    ),
                ));
            }
            None => {
                let mut bytes = vec![0_u8; 32];
                getrandom::fill(&mut bytes).map_err(|error| {
                    blocked("closure_authority_entropy_failed", error.to_string())
                })?;
                writes.push((key_path, bytes));
            }
        }
        self.file_plan(binding, writes, "install the contract-v2 runtime manifest")
    }

    fn agent_register_plan(
        &self,
        request: &AgentRegisterRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let surface = match request.surface {
            AgentSurface::Cli => ags_host_integration::AgentSurface::Cli,
            AgentSurface::Mcp => ags_host_integration::AgentSurface::Mcp,
            AgentSurface::Hybrid => ags_host_integration::AgentSurface::Hybrid,
        };
        let agent = ags_host_integration::GenericAgent::new(&request.host_id, surface)
            .map_err(|detail| blocked("agent_host_invalid", detail))?;
        let official_adapter = agent
            .official_adapter()
            .map(|adapter| adapter.id.to_string());
        let registration = ags_host_integration::HostRegistration::new(
            agent.host_id.clone(),
            surface,
            official_adapter,
        );
        let body = serde_json::to_vec_pretty(&registration)
            .map_err(|error| blocked("agent_registration_encode_failed", error.to_string()))?;
        let registration_path = self
            .runtime_home
            .join("hosts")
            .join(agent.host_id.as_str())
            .join("registration.json");
        self.file_plan(
            binding,
            vec![(registration_path, body)],
            "write the canonical AGS-owned host registration",
        )
    }

    fn update_plan(
        &self,
        request: &UpdateRequest,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let channel = safe_id(&request.channel, "update_channel_invalid")?;
        if request
            .target_version
            .as_deref()
            .is_some_and(|version| version.trim().is_empty() || version.len() > 64)
        {
            return Err(blocked(
                "update_target_version_invalid",
                request.target_version.as_deref().unwrap_or_default(),
            ));
        }
        let release_id = request.target_version.as_deref().unwrap_or(channel);
        if !release_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(blocked("update_target_version_invalid", release_id));
        }
        let releases_directory = self.runtime_home.join("releases");
        let release_directory = releases_directory.join(release_id);
        let candidate_directory = self.runtime_home.join("update-candidates").join(release_id);
        let members = scan_release_directory(&candidate_directory, &RELEASE_PAYLOAD_NAMES)
            .map_err(|error| {
                blocked(
                    "update_candidate_invalid",
                    format!("{}: {}", error.code, error.detail),
                )
            })?;
        let tree_digest = release_tree_digest(&members)?;
        let manifest = SealedReleaseManifest {
            schema_version: "ags://schema/contract/v2/sealed-release-manifest".to_string(),
            version: release_id.to_string(),
            tree_digest: tree_digest.clone(),
            members: members.clone(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| blocked("update_manifest_encode_failed", error.to_string()))?;
        let manifest_member = ReleaseMember {
            name: "release-manifest.json".to_string(),
            sha256: sha256(&manifest_bytes),
            size: manifest_bytes.len() as u64,
            mode: 0o644,
        };
        let mut expected_write_paths = vec![
            releases_directory.display().to_string(),
            release_directory.display().to_string(),
        ];
        expected_write_paths.extend(
            RELEASE_PAYLOAD_NAMES
                .iter()
                .map(|name| release_directory.join(name).display().to_string()),
        );
        expected_write_paths.extend([
            release_directory
                .join("release-manifest.json")
                .display()
                .to_string(),
            self.runtime_home
                .join("current-release.json")
                .display()
                .to_string(),
            self.runtime_home
                .join("update-state.json")
                .display()
                .to_string(),
        ]);
        let action = ProductionAction::Update {
            candidate_directory,
            release_directory,
            manifest: manifest_member,
            tree_digest,
            members,
        };
        let action_digest = canonical_production_action_digest(&action)?;
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest,
                steps: vec![PlanStep {
                    step_id: "host-runtime-update".to_string(),
                    description: "host executes and attests one exact runtime update".to_string(),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec![
                        "authenticated-update-receipt".to_string(),
                        "host-outcome-write-set-and-postimages".to_string(),
                    ],
                },
                recoverability: Recoverability::NotApplicable,
                execution: None,
            },
            action,
        })))
    }

    fn host_projection_plan(
        &self,
        request: &HostProjectionRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let host = ags_host_integration::HostId::new(&request.host_id)
            .map_err(|detail| blocked("host_id_invalid", detail))?;
        let path = self
            .runtime_home
            .join("hosts")
            .join(host.as_str())
            .join("projection.json");
        match request.mode {
            ProjectionMode::RemoveOwned => self.delete_plan(
                binding,
                vec![path],
                "remove the exact AGS-owned host projection",
            ),
            ProjectionMode::Reconcile => {
                let body = serde_json::to_vec_pretty(&serde_json::json!({
                    "schema_version": "ags://schema/contract/v2/host-projection",
                    "host_id": host.as_str(),
                    "workspace": binding.canonical_workspace(),
                    "third_party_mcp_policy": "advice-only"
                }))
                .map_err(|error| blocked("host_projection_encode_failed", error.to_string()))?;
                self.file_plan(
                    binding,
                    vec![(path, body)],
                    "reconcile the AGS-owned host projection",
                )
            }
        }
    }

    fn skill_install_plan(
        &self,
        request: &SkillInstallRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let requested_skill_id = safe_id(&request.skill_id, "skill_id_invalid")?;
        let context = self.skill_adoption_context(binding);
        let source = canonical_skill_source(&request.source);
        let scratch = if matches!(
            &source,
            ags_capability_governance::skill_adoption::SourceSpec::GitHub { .. }
                | ags_capability_governance::skill_adoption::SourceSpec::Git { .. }
        ) {
            Some(
                tempfile::Builder::new()
                    .prefix("ags-skill-plan-")
                    .tempdir()
                    .map_err(|error| blocked("skill_plan_scratch_failed", error.to_string()))?,
            )
        } else {
            None
        };
        let mut planning_context = context.clone();
        if let Some(scratch) = &scratch {
            planning_context.candidate_home = scratch.path().to_path_buf();
        }
        let routing_metadata = request.routing_metadata.as_deref().map(Path::new);
        let update_policy = match request.update_policy {
            SkillUpdatePolicy::Notify => {
                ags_capability_governance::skill_adoption::UpdatePolicy::Notify
            }
            SkillUpdatePolicy::Manual => {
                ags_capability_governance::skill_adoption::UpdatePolicy::Manual
            }
            SkillUpdatePolicy::Pinned => {
                ags_capability_governance::skill_adoption::UpdatePolicy::Pinned
            }
        };
        let plan = ags_capability_governance::skill_adoption::plan_install(
            &planning_context,
            &source,
            routing_metadata,
            &request.target_hosts,
            update_policy,
        )
        .map_err(|detail| blocked("skill_install_plan_failed", detail))?;
        if plan.skill_id != requested_skill_id {
            return Err(blocked(
                "skill_id_source_mismatch",
                format!(
                    "requested {}, audited source declares {}",
                    requested_skill_id, plan.skill_id
                ),
            ));
        }
        let acknowledgements = request.risk_acknowledgements.iter().cloned().collect();
        let materialized = ags_capability_governance::skill_adoption::materialize_skill_change(
            &planning_context,
            &plan,
            &acknowledgements,
        )
        .map_err(|detail| blocked("skill_change_materialization_failed", detail))?;
        materialized_skill_change_domain_plan(self, binding, context, plan, materialized)
    }

    fn skill_remove_plan(
        &self,
        request: &SkillRemoveRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let skill_id = safe_id(&request.skill_id, "skill_id_invalid")?;
        let context = self.skill_adoption_context(binding);
        let plan = ags_capability_governance::skill_adoption::plan_removal(&context, skill_id)
            .map_err(|detail| blocked("skill_remove_plan_failed", detail))?;
        skill_change_domain_plan(self, binding, context, plan, Default::default())
    }

    fn skill_adoption_context(
        &self,
        binding: &AuthenticatedBinding,
    ) -> ags_capability_governance::skill_adoption::AdoptionContext {
        ags_capability_governance::skill_adoption::AdoptionContext {
            authority_root: binding.canonical_workspace().to_path_buf(),
            runtime_home: self.runtime_home.clone(),
            candidate_home: self.runtime_home.clone(),
            host_home: self
                .runtime_home
                .parent()
                .unwrap_or(self.runtime_home.as_path())
                .to_path_buf(),
            snapshot_discovery:
                ags_capability_governance::skill_adoption::SnapshotDiscovery::Offline,
        }
    }

    fn capability_snapshot_plan(
        &self,
        request: &CapabilitySnapshotRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let host = ags_host_integration::HostId::new(&request.host_id)
            .map_err(|detail| blocked("host_id_invalid", detail))?;
        let snapshot = ags_capability_governance::build_capability_snapshot_with_roots(
            binding.canonical_workspace(),
            host.as_str(),
            &self.runtime_home,
            &self.host_home,
        )
        .map_err(|error| blocked("capability_snapshot_build_failed", format!("{error:?}")))?;
        let mut body = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| blocked("capability_snapshot_encode_failed", error.to_string()))?;
        body.push(b'\n');
        self.file_plan(
            binding,
            vec![(
                ags_capability_governance::snapshot_path(&self.runtime_home, host.as_str()),
                body,
            )],
            "publish the exact host capability snapshot",
        )
    }

    fn task_close_plan(
        &self,
        request: &TaskCloseRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let task_path = normalized_binding_path(binding, &request.task_card_path)?;
        let plan_path = normalized_binding_path(binding, &request.launch_plan_path)?;
        let report_path = normalized_binding_path(binding, &request.delivery_report_path)?;
        let task = read_binding_text(
            binding,
            &task_path,
            MAX_CLOSURE_ARTIFACT_BYTES,
            "task_close_read_failed",
        )?;
        let plan = read_binding_text(
            binding,
            &plan_path,
            MAX_CLOSURE_ARTIFACT_BYTES,
            "task_close_read_failed",
        )?;
        let report = read_binding_text(
            binding,
            &report_path,
            MAX_CLOSURE_ARTIFACT_BYTES,
            "task_close_read_failed",
        )?;
        let closure = ags_evidence::delivery_report::validate(&task, &plan, &report);
        if !closure.valid {
            return Err(blocked(
                "task_close_validation_failed",
                closure
                    .checks
                    .iter()
                    .filter(|check| !check.passed)
                    .map(|check| format!("{}: {}", check.name, check.detail))
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let receipt = ags_evidence::generate_closed_receipt(
            &task_path,
            &plan_path,
            &report_path,
            &closure,
            Vec::new(),
            None,
        );
        let body = serde_json::to_vec_pretty(&receipt)
            .map_err(|error| blocked("task_receipt_encode_failed", error.to_string()))?;
        ags_evidence::VerifiedClosure::from_bounded_bytes(
            &body,
            task.as_bytes(),
            plan.as_bytes(),
            report.as_bytes(),
        )
        .map_err(|detail| blocked("task_close_pure_verification_failed", detail))?;
        let receipt_path = binding
            .canonical_workspace()
            .join(".ags/evidence")
            .join(format!("{}.json", receipt.receipt_id));
        let pointer_path = binding
            .canonical_workspace()
            .join(".ags/state/closure-pointers")
            .join(format!("{}.json", receipt.receipt_id));
        let machine_key = self.closure_authority_key(binding)?;
        let mut pointer = crate::workspace_lifecycle::ClosurePointer {
            schema_version: crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: Some(binding.canonical_workspace().display().to_string()),
            workspace_identity: Some(crate::workspace_lifecycle::workspace_identity(
                binding.canonical_workspace(),
            )),
            receipt_id: receipt.receipt_id.clone(),
            receipt_path: receipt_path.display().to_string(),
            receipt_sha256: sha256(&body),
            task_card_hash: receipt.task_card_hash.clone(),
            launch_plan_hash: receipt.launch_plan_hash.clone(),
            delivery_report_hash: receipt.delivery_report_hash.clone(),
            authority_key_id: String::new(),
            authority_seal: String::new(),
        };
        crate::workspace_lifecycle::seal_closure_pointer(&machine_key, &mut pointer)
            .map_err(|detail| blocked("closure_pointer_seal_failed", detail))?;
        let pointer = serde_json::to_vec_pretty(&pointer)
            .map_err(|error| blocked("closure_pointer_encode_failed", error.to_string()))?;
        self.file_plan(
            binding,
            vec![(receipt_path, body), (pointer_path, pointer)],
            "write the verified task receipt and closure pointer",
        )
    }

    fn memory_close_plan(
        &self,
        request: &MemoryCloseRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        let receipt_path = normalized_binding_path(binding, &request.receipt_path)?;
        let bytes = read_binding_bytes(
            binding,
            &receipt_path,
            MAX_CLOSURE_ARTIFACT_BYTES,
            "memory_receipt_read_failed",
        )?;
        let receipt: ags_evidence::Receipt = serde_json::from_slice(&bytes)
            .map_err(|error| blocked("memory_receipt_invalid", error.to_string()))?;
        let machine_key = self.closure_authority_key(binding)?;
        let (pointer_path, pointer_bytes) = verify_canonical_closure_seal(
            binding,
            &machine_key,
            &receipt_path,
            &bytes,
            &receipt,
            "memory_receipt_unverified",
        )?;
        Ok(PlanDisposition::NoChange {
            output_digest: sha256(format!(
                "{}\n{}\n{}",
                sha256(bytes),
                sha256(pointer_bytes),
                pointer_path.display()
            )),
        })
    }

    fn test_plan(
        &self,
        request: &TestRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        if request.executor == TestExecutor::Local {
            if let ags_verification::LocalExecutionPlatformSupport::Blocked { error_code, reason } =
                ags_verification::local_execution_platform_support()
            {
                return Err(blocked(
                    format!("local_execution_{}", error_code.as_str()),
                    reason,
                ));
            }
        }
        let profile = match request.profile {
            TestProfile::Smoke => ags_verification::TestProfile::Smoke,
            TestProfile::Standard => ags_verification::TestProfile::Standard,
            TestProfile::Full => ags_verification::TestProfile::Full,
        };
        let profile_path = Path::new("config/agent-project-profile.yaml");
        let profiles = ags_verification::load_project_test_profiles(
            binding.canonical_workspace(),
            profile_path,
        )
        .map_err(|detail| blocked("project_test_profile_invalid", detail))?;
        let spec = profiles.get(profile).clone();
        let cwd = if spec.cwd.is_absolute() {
            spec.cwd.clone()
        } else {
            binding.canonical_workspace().join(&spec.cwd)
        };
        let allowed_write_paths = spec
            .allowed_write_paths
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    binding.canonical_workspace().join(path)
                }
                .to_path_buf()
            })
            .collect();
        let mut command = CommandSpec {
            cwd,
            allowed_write_paths,
            ..spec.clone()
        };
        if !command.env.contains_key("PATH") {
            let path = std::env::var("PATH").map_err(|error| {
                blocked(
                    "project_test_path_unavailable",
                    format!("host PATH cannot be sealed into the test plan: {error}"),
                )
            })?;
            if path.is_empty() || path.len() > MAX_EFFECT_PATH_BYTES {
                return Err(blocked(
                    "project_test_path_invalid",
                    "host PATH is empty or exceeds the structured environment budget",
                ));
            }
            command.env.insert("PATH".to_string(), path);
        }
        let host_temp = command
            .env
            .get("TMPDIR")
            .cloned()
            .or_else(|| std::env::var("TMPDIR").ok())
            .unwrap_or_else(|| "/tmp".to_string());
        for key in ["TMPDIR", "TMP", "TEMP"] {
            command
                .env
                .entry(key.to_string())
                .or_insert_with(|| host_temp.clone());
            let value = &command.env[key];
            let path = Path::new(value);
            if value.is_empty()
                || value.len() > MAX_EFFECT_PATH_BYTES
                || !path.is_absolute()
                || path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                || !path.is_dir()
            {
                return Err(blocked(
                    "project_test_temp_invalid",
                    format!("{key} must name an existing absolute host directory"),
                ));
            }
        }
        if let Some(runtime_root) = command.allowed_write_paths.first() {
            command
                .env
                .entry("AGS_RUNTIME_HOME".to_string())
                .or_insert_with(|| runtime_root.join(".ags-test-runtime").display().to_string());
            let test_runtime = Path::new(&command.env["AGS_RUNTIME_HOME"]);
            if !command
                .allowed_write_paths
                .iter()
                .any(|root| test_runtime.starts_with(root))
            {
                return Err(blocked(
                    "project_test_runtime_outside_write_set",
                    "AGS_RUNTIME_HOME must stay inside the sealed test write set",
                ));
            }
        }
        let action = if request.executor == TestExecutor::Host {
            ProductionAction::None
        } else {
            ProductionAction::ProjectTest {
                workspace: binding.canonical_workspace().to_path_buf(),
                profile,
                spec: command.clone(),
            }
        };
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest: canonical_production_action_digest(&action)?,
                steps: vec![PlanStep {
                    step_id: "execute-derived-test".to_string(),
                    description: "execute the profile-derived project test command".to_string(),
                }],
                expected_write_paths: command
                    .allowed_write_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                verification: VerificationSpec {
                    checks: if request.executor == TestExecutor::Host {
                        vec![
                            "authenticated-test-receipt".to_string(),
                            "host-outcome-write-set-and-postimages".to_string(),
                        ]
                    } else {
                        vec!["process-exit-success".to_string()]
                    },
                },
                recoverability: if request.executor == TestExecutor::Host {
                    Recoverability::NotApplicable
                } else {
                    Recoverability::SourcePreserving
                },
                execution: Some(command.clone()),
            },
            action,
        })))
    }

    fn lifecycle_session_end_plan(
        &self,
        request: &LifecycleSessionEndRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<ProductionAction>, EffectError> {
        ensure_bound_host(&request.host_id, binding, "lifecycle_host_binding_mismatch")?;
        let machine_key = self.closure_authority_key(binding)?;
        let planned = crate::workspace_lifecycle::plan_session_end(
            binding.canonical_workspace(),
            &self.host_home,
            request,
            &machine_key,
        )
        .map_err(|detail| blocked("lifecycle_session_end_plan_failed", detail))?;
        if planned.receipt_ids.is_empty() {
            return Ok(PlanDisposition::NoChange {
                output_digest: planned.action_digest,
            });
        }
        let receipt_count = planned.receipt_ids.len();
        let expected_write_paths = planned.expected_write_paths;
        let action = ProductionAction::LifecycleSessionEnd {
            receipt_ids: planned.receipt_ids,
            pointer_paths: planned.pointer_paths,
        };
        Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
            plan: DomainPlan {
                action_digest: canonical_production_action_digest(&action)?,
                steps: vec![PlanStep {
                    step_id: "archive-verified-session-closures".to_string(),
                    description: format!(
                        "archive {} verified closure receipts through the bound host",
                        receipt_count
                    ),
                }],
                expected_write_paths,
                verification: VerificationSpec {
                    checks: vec![
                        "host-outcome-artifact-binding".to_string(),
                        "host-outcome-write-set-and-postimages".to_string(),
                    ],
                },
                recoverability: Recoverability::NotApplicable,
                execution: None,
            },
            action,
        })))
    }

    fn capability_inventory(
        &self,
        request: &CapabilityInventoryRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let requested_host = request
            .host_id
            .as_deref()
            .unwrap_or_else(|| binding.host_id());
        let host = ags_host_integration::HostId::new(requested_host)
            .map_err(|detail| blocked("capability_inventory_host_invalid", detail))?;
        let context =
            ags_capability_governance::skill_body::console::ConsoleContext::new_with_runtime_home(
                binding.canonical_workspace(),
                self.host_home.clone(),
                &self.runtime_home,
                Box::new(NoopHostProbeRunner),
            );
        let inventory = ags_capability_governance::skill_body::console::build_inventory(
            &context,
            &[host.as_str()],
        );
        let inventory_hash =
            ags_capability_governance::skill_body::console::inventory_snapshot_hash(&inventory);
        let snapshot =
            ags_capability_governance::load_static_snapshot(&self.runtime_home, host.as_str());
        let (snapshot_status, snapshot_hash) = match snapshot {
            Ok((snapshot, _tables)) => ("ready", Some(snapshot.snapshot_hash)),
            Err(_) => ("missing_or_stale", None),
        };
        Ok(serde_json::json!({
            "schema_version": "ags://schema/contract/v2/capability-inventory-result",
            "host_id": host.as_str(),
            "include_inactive": request.include_inactive,
            "summary": inventory.summary,
            "routing_parse_failures": inventory.routing_parse_failures,
            "inventory_hash": inventory_hash,
            "snapshot_status": snapshot_status,
            "snapshot_hash": snapshot_hash,
            "core_operations_blocked": false
        }))
    }

    fn empty_read_roots<R>(&self, _request: &R, _binding: &AuthenticatedBinding) -> Vec<PathBuf> {
        Vec::new()
    }

    fn doctor_roots<R>(&self, _request: &R, binding: &AuthenticatedBinding) -> Vec<PathBuf> {
        vec![
            binding.canonical_workspace().join("Cargo.toml"),
            binding
                .canonical_workspace()
                .join("config/agent-project-profile.yaml"),
            binding.canonical_workspace().join(".git/index"),
            self.runtime_home.join("installed-skills.json"),
        ]
    }

    fn task_validate_roots(
        &self,
        request: &TaskValidateRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        vec![workspace_path(binding, &request.task_card_path)]
    }

    fn task_plan_roots(
        &self,
        request: &TaskPlanRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        vec![workspace_path(binding, &request.task_card_path)]
    }

    fn policy_roots(
        &self,
        request: &PolicyRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        vec![workspace_path(binding, &request.task_card_path)]
    }

    fn gate_roots(&self, request: &GateRequest, binding: &AuthenticatedBinding) -> Vec<PathBuf> {
        vec![workspace_path(binding, &request.task_card_path)]
    }

    fn evidence_roots(
        &self,
        request: &EvidenceRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        let mut paths = vec![workspace_path(binding, &request.path)];
        paths.extend(
            request
                .task_card_path
                .as_deref()
                .map(|path| workspace_path(binding, path)),
        );
        paths.extend(
            request
                .launch_plan_path
                .as_deref()
                .map(|path| workspace_path(binding, path)),
        );
        paths
    }

    fn lifecycle_start_roots(
        &self,
        _request: &LifecycleSessionStartRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        let Ok(memory) = ags_host_integration::project_memory_dir_at(
            binding.canonical_workspace(),
            &self.host_home,
        ) else {
            return Vec::new();
        };
        vec![
            memory.join("context-capsule.md"),
            memory.join("task-memory.md"),
        ]
    }

    fn capability_inventory_roots(
        &self,
        request: &CapabilityInventoryRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        let host = request
            .host_id
            .as_deref()
            .unwrap_or_else(|| binding.host_id());
        vec![
            self.runtime_home.join("hosts").join(host),
            ags_capability_governance::snapshot_path(&self.runtime_home, host),
        ]
    }

    fn read_schema(
        &self,
        request: &SchemaRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        schema_read_result(request).map_err(|error| blocked(error.code, error.detail))
    }

    fn read_doctor(
        &self,
        request: &DoctorRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let report =
            ags_verification::doctor::inspect(binding.canonical_workspace(), &self.runtime_home);
        let mut value = serde_json::to_value(report)
            .map_err(|error| blocked("doctor_report_encode_failed", error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "requested_scope".to_string(),
                serde_json::to_value(request.scope.clone())
                    .map_err(|error| blocked("doctor_scope_encode_failed", error.to_string()))?,
            );
        }
        Ok(value)
    }

    fn read_check(
        &self,
        request: &CheckRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let scope = match request.scope {
            CheckScope::Governance => ags_verification::Scope::Governance,
            CheckScope::Changes => ags_verification::Scope::Changes,
            CheckScope::Evidence => ags_verification::Scope::Evidence,
            CheckScope::Release => ags_verification::Scope::Release,
            CheckScope::Promotion => ags_verification::Scope::Promotion,
        };
        let report = match &request.public_root {
            Some(public_root) => {
                let options = ags_verification::VerificationOptions {
                    public_root: Some(std::path::PathBuf::from(public_root)),
                };
                ags_verification::run_verify_with_options(
                    scope,
                    binding.canonical_workspace(),
                    &options,
                )
            }
            None => ags_verification::run_verify(scope, binding.canonical_workspace()),
        };
        let passed = report.passed();
        let exit_code = report.exit_code();
        let mut value = serde_json::to_value(report)
            .map_err(|error| blocked("check_report_encode_failed", error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "requested_scope".to_string(),
                serde_json::to_value(request.scope.clone())
                    .map_err(|error| blocked("check_scope_encode_failed", error.to_string()))?,
            );
            object.insert("project_tests_run".to_string(), serde_json::json!(false));
            object.insert("passed".to_string(), serde_json::json!(passed));
            object.insert("exit_code".to_string(), serde_json::json!(exit_code));
        }
        Ok(value)
    }

    fn read_task_validate(
        &self,
        request: &TaskValidateRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let (path, body) =
            read_workspace_text(binding, &request.task_card_path, "task_card_read_failed")?;
        Ok(task_validation_result(&path, &body))
    }

    fn read_task_plan(
        &self,
        request: &TaskPlanRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let path = workspace_artifact(binding, &request.task_card_path)?;
        serde_json::to_value(ags_task_contract::runner::run_task_card_inner(
            &path.display().to_string(),
            false,
            false,
            false,
            false,
            &self.runtime_home,
        ))
        .map_err(|error| blocked("task_plan_encode_failed", error.to_string()))
    }

    fn read_policy(
        &self,
        request: &PolicyRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let (path, body) =
            read_workspace_text(binding, &request.task_card_path, "task_card_read_failed")?;
        task_policy_result(&path, &body)
    }

    fn read_gate(
        &self,
        request: &GateRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        let (path, body) =
            read_workspace_text(binding, &request.task_card_path, "task_card_read_failed")?;
        task_gate_result(&path, &body)
    }

    fn read_evidence(
        &self,
        request: &EvidenceRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        verify_evidence(self, binding, request)
    }

    fn read_mcp_advice(
        &self,
        request: &McpAdviceRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        Ok(
            serde_json::json!({"mcp_id": request.mcp_id, "policy": "advice-only", "mutation_authorized": false}),
        )
    }

    fn read_agent_probe(
        &self,
        request: &AgentProbeRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        generic_agent_probe(request)
    }

    fn read_capability_inventory(
        &self,
        request: &CapabilityInventoryRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        self.capability_inventory(request, binding)
    }

    fn read_lifecycle_start(
        &self,
        request: &LifecycleSessionStartRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        ensure_bound_host(&request.host_id, binding, "lifecycle_host_binding_mismatch")?;
        serde_json::to_value(
            crate::workspace_lifecycle::session_start(
                binding.canonical_workspace(),
                &self.host_home,
                request,
            )
            .map_err(|detail| blocked("lifecycle_session_start_failed", detail))?,
        )
        .map_err(|error| blocked("lifecycle_session_start_encode_failed", error.to_string()))
    }

    fn read_lifecycle_stop_guard(
        &self,
        request: &LifecycleStopGuardRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        ensure_bound_host(&request.host_id, binding, "lifecycle_host_binding_mismatch")?;
        serde_json::to_value(
            crate::workspace_lifecycle::stop_guard(binding.canonical_workspace(), request)
                .map_err(|detail| blocked("lifecycle_stop_guard_failed", detail))?,
        )
        .map_err(|error| blocked("lifecycle_stop_guard_encode_failed", error.to_string()))
    }

    fn read_details_mismatch(
        &self,
        _request: &DetailsReadRequest,
        _binding: &AuthenticatedBinding,
    ) -> Result<serde_json::Value, EffectError> {
        Err(blocked(
            "details_read_owned_by_control_plane",
            OperationName::DetailsRead.as_str(),
        ))
    }
}

fn workspace_path(binding: &AuthenticatedBinding, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else {
        binding.canonical_workspace().join(candidate)
    }
}

enum ProductionStage<'a> {
    Plan {
        binding: &'a AuthenticatedBinding,
    },
    VerifyHost {
        plan: &'a SealedPlan,
        action: &'a ProductionAction,
        binding: &'a AuthenticatedBinding,
        receipt: &'a HostOutcomeReceipt,
        evidence: Option<&'a VerifiedHostEvidence>,
    },
    ReadRoots {
        binding: &'a AuthenticatedBinding,
    },
    Read {
        binding: &'a AuthenticatedBinding,
    },
    ReadSucceeded {
        result: &'a serde_json::Value,
    },
}

enum ProductionStageResult {
    Plan(Result<PlanDisposition<ProductionAction>, EffectError>),
    VerifyHost(Result<(), EffectError>),
    ReadRoots(Vec<PathBuf>),
    Read(Result<serde_json::Value, EffectError>),
    ReadSucceeded(bool),
}

trait ProductionOperation {
    fn dispatch(
        &self,
        adapter: &ProductionEffectAdapter,
        stage: ProductionStage<'_>,
    ) -> ProductionStageResult;
}

macro_rules! production_dispatcher {
    ($( $variant:ident($request:ty) => $wire:literal, $cli:literal, $surface:ident, $resolver:path, [$primary:ident $(, $allowed:ident)*], $schema:literal, $summary:literal; )+) => {
        impl OperationRequest {
            fn dispatch_production(
                &self,
                adapter: &ProductionEffectAdapter,
                stage: ProductionStage<'_>,
            ) -> ProductionStageResult {
                match self {
                    $(Self::$variant(request) => <$request as ProductionOperation>::dispatch(request, adapter, stage),)+
                }
            }
        }
    };
}

for_each_operation!(production_dispatcher);

fn production_plan_mismatch(
    name: OperationName,
) -> Result<PlanDisposition<ProductionAction>, EffectError> {
    Err(blocked("operation_kind_dispatch_mismatch", name.as_str()))
}

fn production_verify_mismatch(name: OperationName) -> Result<(), EffectError> {
    Err(blocked("operation_kind_dispatch_mismatch", name.as_str()))
}

fn production_read_mismatch(name: OperationName) -> Result<serde_json::Value, EffectError> {
    Err(blocked("operation_kind_dispatch_mismatch", name.as_str()))
}

fn effectful_read_roots(binding: &AuthenticatedBinding) -> Vec<PathBuf> {
    vec![binding.canonical_workspace().join(".ags")]
}

macro_rules! impl_effectful_operation {
    ($request:ty, $name:ident, $plan:ident) => {
        impl ProductionOperation for $request {
            fn dispatch(
                &self,
                adapter: &ProductionEffectAdapter,
                stage: ProductionStage<'_>,
            ) -> ProductionStageResult {
                match stage {
                    ProductionStage::Plan { binding } => {
                        ProductionStageResult::Plan(adapter.$plan(self, binding))
                    }
                    ProductionStage::VerifyHost { .. } => ProductionStageResult::VerifyHost(
                        production_verify_mismatch(OperationName::$name),
                    ),
                    ProductionStage::ReadRoots { binding } => {
                        ProductionStageResult::ReadRoots(effectful_read_roots(binding))
                    }
                    ProductionStage::Read { .. } => {
                        ProductionStageResult::Read(production_read_mismatch(OperationName::$name))
                    }
                    ProductionStage::ReadSucceeded { .. } => {
                        ProductionStageResult::ReadSucceeded(true)
                    }
                }
            }
        }
    };
}

macro_rules! impl_read_operation {
    ($request:ty, $name:ident, $roots:ident, $read:ident, $succeeded:expr) => {
        impl ProductionOperation for $request {
            fn dispatch(
                &self,
                adapter: &ProductionEffectAdapter,
                stage: ProductionStage<'_>,
            ) -> ProductionStageResult {
                match stage {
                    ProductionStage::Plan { .. } => {
                        ProductionStageResult::Plan(production_plan_mismatch(OperationName::$name))
                    }
                    ProductionStage::VerifyHost { .. } => ProductionStageResult::VerifyHost(
                        production_verify_mismatch(OperationName::$name),
                    ),
                    ProductionStage::ReadRoots { binding } => {
                        ProductionStageResult::ReadRoots(adapter.$roots(self, binding))
                    }
                    ProductionStage::Read { binding } => {
                        ProductionStageResult::Read(adapter.$read(self, binding))
                    }
                    ProductionStage::ReadSucceeded { result } => {
                        ProductionStageResult::ReadSucceeded(($succeeded)(result))
                    }
                }
            }
        }
    };
}

impl_effectful_operation!(SetupRequest, Setup, setup_plan);
impl_effectful_operation!(InitRequest, Init, init_plan);
impl_effectful_operation!(AgentRegisterRequest, AgentRegister, agent_register_plan);
impl_effectful_operation!(
    HostProjectionRequest,
    GovernHostProjection,
    host_projection_plan
);
impl_effectful_operation!(SkillInstallRequest, GovernSkillInstall, skill_install_plan);
impl_effectful_operation!(SkillRemoveRequest, GovernSkillRemove, skill_remove_plan);
impl_effectful_operation!(
    CapabilitySnapshotRequest,
    GovernCapabilitySnapshot,
    capability_snapshot_plan
);
impl_effectful_operation!(TaskCloseRequest, GovernTaskClose, task_close_plan);
impl_effectful_operation!(MemoryCloseRequest, GovernMemoryClose, memory_close_plan);

impl_read_operation!(
    AgentProbeRequest,
    AgentProbe,
    empty_read_roots,
    read_agent_probe,
    |_| true
);
impl_read_operation!(
    CapabilityInventoryRequest,
    GovernCapabilityInventory,
    capability_inventory_roots,
    read_capability_inventory,
    |_| true
);
impl_read_operation!(
    McpAdviceRequest,
    GovernMcpAdvice,
    empty_read_roots,
    read_mcp_advice,
    |_| true
);
impl_read_operation!(
    TaskValidateRequest,
    GovernTaskValidate,
    task_validate_roots,
    read_task_validate,
    |result: &serde_json::Value| result
        .get("valid")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
);
impl_read_operation!(
    TaskPlanRequest,
    GovernTaskPlan,
    task_plan_roots,
    read_task_plan,
    |result: &serde_json::Value| result
        .get("gate_decision")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|decision| decision == "allow")
);
impl_read_operation!(
    PolicyRequest,
    GovernPolicy,
    policy_roots,
    read_policy,
    |result: &serde_json::Value| result
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|decision| decision != "stop")
);
impl_read_operation!(
    GateRequest,
    GovernGate,
    gate_roots,
    read_gate,
    |result: &serde_json::Value| result
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|decision| decision == "allow")
);
impl_read_operation!(
    EvidenceRequest,
    GovernEvidence,
    evidence_roots,
    read_evidence,
    |result: &serde_json::Value| result
        .get("valid")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
);
impl_read_operation!(DoctorRequest, Doctor, doctor_roots, read_doctor, |_| true);
impl_read_operation!(
    CheckRequest,
    Check,
    doctor_roots,
    read_check,
    |result: &serde_json::Value| result
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
);
impl_read_operation!(SchemaRequest, Schema, empty_read_roots, read_schema, |_| {
    true
});
impl_read_operation!(
    LifecycleSessionStartRequest,
    HostLifecycleSessionStart,
    lifecycle_start_roots,
    read_lifecycle_start,
    |_| true
);
impl_read_operation!(
    LifecycleStopGuardRequest,
    HostLifecycleStopGuard,
    empty_read_roots,
    read_lifecycle_stop_guard,
    |_| true
);
impl_read_operation!(
    DetailsReadRequest,
    DetailsRead,
    empty_read_roots,
    read_details_mismatch,
    |_| true
);

impl ProductionOperation for UpdateRequest {
    fn dispatch(
        &self,
        adapter: &ProductionEffectAdapter,
        stage: ProductionStage<'_>,
    ) -> ProductionStageResult {
        match stage {
            ProductionStage::Plan { .. } => ProductionStageResult::Plan(adapter.update_plan(self)),
            ProductionStage::VerifyHost {
                plan,
                action,
                receipt,
                evidence,
                ..
            } => ProductionStageResult::VerifyHost(
                evidence
                    .ok_or_else(|| {
                        blocked("update_evidence_missing", "update requires typed evidence")
                    })
                    .and_then(|evidence| {
                        verify_update_host_outcome(self, plan, action, receipt, evidence)
                    }),
            ),
            ProductionStage::ReadRoots { binding } => {
                ProductionStageResult::ReadRoots(effectful_read_roots(binding))
            }
            ProductionStage::Read { .. } => {
                ProductionStageResult::Read(production_read_mismatch(OperationName::Update))
            }
            ProductionStage::ReadSucceeded { .. } => ProductionStageResult::ReadSucceeded(true),
        }
    }
}

impl ProductionOperation for TestRequest {
    fn dispatch(
        &self,
        adapter: &ProductionEffectAdapter,
        stage: ProductionStage<'_>,
    ) -> ProductionStageResult {
        match stage {
            ProductionStage::Plan { binding } => {
                ProductionStageResult::Plan(adapter.test_plan(self, binding))
            }
            ProductionStage::VerifyHost {
                plan,
                binding,
                receipt,
                evidence,
                ..
            } => ProductionStageResult::VerifyHost(
                evidence
                    .ok_or_else(|| {
                        blocked("host_test_evidence_missing", "Test requires typed evidence")
                    })
                    .and_then(|evidence| {
                        verify_host_test_outcome(self, plan, binding, receipt, evidence)
                    }),
            ),
            ProductionStage::ReadRoots { binding } => {
                ProductionStageResult::ReadRoots(effectful_read_roots(binding))
            }
            ProductionStage::Read { .. } => {
                ProductionStageResult::Read(production_read_mismatch(OperationName::Test))
            }
            ProductionStage::ReadSucceeded { .. } => ProductionStageResult::ReadSucceeded(true),
        }
    }
}

impl ProductionOperation for LifecycleSessionEndRequest {
    fn dispatch(
        &self,
        adapter: &ProductionEffectAdapter,
        stage: ProductionStage<'_>,
    ) -> ProductionStageResult {
        match stage {
            ProductionStage::Plan { binding } => {
                ProductionStageResult::Plan(adapter.lifecycle_session_end_plan(self, binding))
            }
            ProductionStage::VerifyHost {
                plan,
                action,
                receipt,
                evidence,
                ..
            } => ProductionStageResult::VerifyHost(
                evidence
                    .ok_or_else(|| {
                        blocked(
                            "lifecycle_host_evidence_missing",
                            "session end requires typed evidence",
                        )
                    })
                    .and_then(|evidence| {
                        verify_lifecycle_host_outcome(self, plan, action, receipt, evidence)
                    }),
            ),
            ProductionStage::ReadRoots { binding } => {
                ProductionStageResult::ReadRoots(effectful_read_roots(binding))
            }
            ProductionStage::Read { .. } => ProductionStageResult::Read(production_read_mismatch(
                OperationName::HostLifecycleSessionEnd,
            )),
            ProductionStage::ReadSucceeded { .. } => ProductionStageResult::ReadSucceeded(true),
        }
    }
}

impl EffectAdapter for ProductionEffectAdapter {
    type Action = ProductionAction;

    fn semantic_action_digest(&self, action: &Self::Action) -> Result<Option<String>, EffectError> {
        Ok(Some(canonical_production_action_digest(action)?))
    }

    fn validate_platform_support(&self, _operation: &OperationRequest) -> Result<(), EffectError> {
        Ok(())
    }

    fn validate_sealed_action(
        &self,
        plan: &SealedPlan,
        action: &Self::Action,
        binding: &AuthenticatedBinding,
    ) -> Result<(), EffectError> {
        if plan.kind != OperationKind::Transaction {
            return Ok(());
        }
        let actual = journal_writes(action)?
            .into_iter()
            .map(|write| write.path)
            .collect::<std::collections::BTreeSet<_>>();
        if let ProductionAction::PendingRecovery {
            expected_write_paths,
            ..
        } = action
        {
            let actual = expected_write_paths
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let expected = plan
                .expected_write_paths
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            return if actual == expected {
                Ok(())
            } else {
                Err(blocked(
                    "transaction_journal_write_set_mismatch",
                    "pending recovery footprint differs from sealed plan",
                ))
            };
        }
        if let ProductionAction::SkillChange {
            materialized,
            mutations,
            ..
        } = action
        {
            if materialized_hash(materialized)? != materialized.materialization_hash {
                return Err(blocked(
                    "skill_materialized_action_identity_mismatch",
                    &materialized.skill_id,
                ));
            }
            let derived = materialized_skill_mutations(self, binding, materialized)?;
            if canonical_mutations(&derived)? != canonical_mutations(mutations)? {
                return Err(blocked(
                    "skill_materialized_mutation_mismatch",
                    &materialized.skill_id,
                ));
            }
            let materialized_paths = materialized
                .write_paths()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            if materialized_paths != actual {
                return Err(blocked(
                    "skill_materialized_write_set_mismatch",
                    &materialized.skill_id,
                ));
            }
        }
        let expected = plan
            .expected_write_paths
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if actual != expected {
            let code = if matches!(action, ProductionAction::Projections(_)) {
                "transaction_journal_write_set_mismatch"
            } else {
                "sealed_action_write_set_mismatch"
            };
            return Err(blocked(
                code,
                format!("sealed={expected:?}; action={actual:?}"),
            ));
        }
        Ok(())
    }

    fn inspect_pending(
        &self,
        binding: &AuthenticatedBinding,
    ) -> Result<PendingInspection<Self::Action>, EffectError> {
        self.inspect_pending_transaction(binding)
    }

    fn seals_host_physical_state(
        &self,
        operation: &OperationRequest,
        action: &Self::Action,
    ) -> bool {
        matches!(operation, OperationRequest::Test(request) if request.executor == TestExecutor::Host)
            || matches!(
                action,
                ProductionAction::Update { .. } | ProductionAction::LifecycleSessionEnd { .. }
            )
    }

    fn is_recovery_action(&self, action: &Self::Action) -> bool {
        matches!(action, ProductionAction::PendingRecovery { .. })
    }

    fn host_execution_action(
        &self,
        operation: &OperationRequest,
        plan: &SealedPlan,
        action: &Self::Action,
        _binding: &AuthenticatedBinding,
    ) -> Result<HostExecutionAction, EffectError> {
        let expected_write_paths = plan
            .expected_write_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        match (operation, action) {
            (
                OperationRequest::Update(request),
                ProductionAction::Update {
                    candidate_directory,
                    release_directory,
                    manifest,
                    tree_digest,
                    members,
                },
            ) => {
                let candidate_root = self.runtime_home.join("update-candidates");
                if candidate_directory.parent() != Some(candidate_root.as_path())
                    || candidate_directory == release_directory
                {
                    return Err(blocked(
                        "host_execution_instruction_source_invalid",
                        "runtime update candidate is outside the sealed update-candidates authority",
                    ));
                }
                Ok(HostExecutionAction::RuntimeUpdate {
                    channel: request.channel.clone(),
                    target_version: request.target_version.clone(),
                    candidate_directory: candidate_directory.clone(),
                    release_directory: release_directory.clone(),
                    manifest: host_release_member(manifest),
                    tree_digest: tree_digest.clone(),
                    members: members.iter().map(host_release_member).collect(),
                    expected_write_paths,
                })
            }
            (
                OperationRequest::HostLifecycleSessionEnd(request),
                ProductionAction::LifecycleSessionEnd {
                    receipt_ids,
                    pointer_paths,
                },
            ) => Ok(HostExecutionAction::ArchiveClosures {
                event_id: request.event_id.clone(),
                receipt_ids: receipt_ids.clone(),
                pointer_paths: pointer_paths.iter().map(PathBuf::from).collect(),
                expected_write_paths,
            }),
            _ => Err(blocked(
                "host_execution_instruction_operation_mismatch",
                "typed operation does not match its sealed HostDelegated domain action",
            )),
        }
    }

    fn finalize_recovery(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        binding: &AuthenticatedBinding,
        receipt: &OperationReceipt,
    ) -> Result<(), EffectError> {
        self.finalize_exact_recovery(action_ref, plan, action, binding, receipt)
    }

    fn verify_host_outcome(
        &self,
        operation: &OperationRequest,
        plan: &SealedPlan,
        action: &Self::Action,
        binding: &AuthenticatedBinding,
        receipt: &HostOutcomeReceipt,
        evidence: Option<&VerifiedHostEvidence>,
    ) -> Result<(), EffectError> {
        match operation.dispatch_production(
            self,
            ProductionStage::VerifyHost {
                plan,
                action,
                binding,
                receipt,
                evidence,
            },
        ) {
            ProductionStageResult::VerifyHost(result) => result,
            _ => Err(blocked(
                "production_dispatch_contract_violation",
                operation.name().as_str(),
            )),
        }
    }

    fn plan(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<Self::Action>, EffectError> {
        match operation.dispatch_production(self, ProductionStage::Plan { binding }) {
            ProductionStageResult::Plan(result) => result,
            _ => Err(blocked(
                "production_dispatch_contract_violation",
                operation.name().as_str(),
            )),
        }
    }

    fn read_only_roots(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf> {
        let mut protected =
            match operation.dispatch_production(self, ProductionStage::ReadRoots { binding }) {
                ProductionStageResult::ReadRoots(paths) => paths,
                _ => Vec::new(),
            };
        protected.sort();
        protected.dedup();
        protected
    }

    fn read(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<ReadObservation, EffectError> {
        let digest = match operation.dispatch_production(self, ProductionStage::Read { binding }) {
            ProductionStageResult::Read(result) => result?,
            _ => {
                return Err(blocked(
                    "production_dispatch_contract_violation",
                    operation.name().as_str(),
                ));
            }
        };
        let output_digest = sha256(
            serde_json::to_vec(&digest)
                .map_err(|error| blocked("read_output_encode_failed", error.to_string()))?,
        );
        Ok(ReadObservation {
            succeeded: match operation
                .dispatch_production(self, ProductionStage::ReadSucceeded { result: &digest })
            {
                ProductionStageResult::ReadSucceeded(succeeded) => succeeded,
                _ => {
                    return Err(blocked(
                        "production_dispatch_contract_violation",
                        operation.name().as_str(),
                    ));
                }
            },
            result: digest,
            output_digest,
        })
    }

    fn apply(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        _operation: Option<&OperationRequest>,
        binding: &AuthenticatedBinding,
    ) -> Result<EffectObservation, EffectError> {
        if plan.kind == OperationKind::Transaction
            && !matches!(action, ProductionAction::PendingRecovery { .. })
        {
            self.prepare_journal(action_ref, plan, action, binding)?;
            self.transition_journal(action_ref, plan, JournalPhase::Applying, None)?;
        }
        #[cfg(test)]
        PRODUCTION_DOMAIN_APPLY_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
        let result = match action {
            ProductionAction::Files(writes) => {
                apply_files_journaled(self, action_ref, plan, writes)
            }
            ProductionAction::Deletes(deletes) => {
                apply_deletes_journaled(self, action_ref, plan, deletes)
            }
            ProductionAction::Projections(projections) => {
                apply_projections(self, action_ref, plan, projections)
            }
            ProductionAction::ProjectTest {
                workspace,
                profile,
                spec,
            } => {
                let receipt = ags_verification::run_project_test(workspace, *profile, spec)
                    .map_err(|error| {
                        blocked(
                            format!("local_execution_{:?}", error.code).to_lowercase(),
                            error.to_string(),
                        )
                    })?;
                let succeeded = receipt.status == ags_verification::TestExecutionStatus::Succeeded;
                let output_digest = receipt.output_digest.clone();
                let observed_write_set = receipt
                    .observed_write_set
                    .iter()
                    .map(|path| {
                        Path::new(&receipt.canonical_workspace)
                            .join(path)
                            .display()
                            .to_string()
                    })
                    .collect();
                let evidence = serde_json::to_value(receipt)
                    .map_err(|error| blocked("test_receipt_encode_failed", error.to_string()))?;
                EffectObservation::bounded(
                    succeeded,
                    true,
                    output_digest,
                    observed_write_set,
                    Some(evidence),
                )
            }
            ProductionAction::SkillChange {
                context,
                materialized,
                mutations,
            } => apply_materialized_skill_change(
                self,
                action_ref,
                plan,
                context,
                materialized,
                mutations,
            ),
            ProductionAction::PendingRecovery {
                original_action_ref,
                journal_identity_digest,
                journal_state_digest,
                expected_write_paths,
            } => {
                let current = self
                    .inspect_pending_transaction(binding)?
                    .active
                    .ok_or_else(|| blocked("pending_transaction_missing", original_action_ref))?;
                if current.journal_identity_digest != *journal_identity_digest
                    || current.journal_state_digest != *journal_state_digest
                    || current.expected_write_paths != *expected_write_paths
                {
                    return Err(blocked(
                        "pending_transaction_identity_mismatch",
                        original_action_ref,
                    ));
                }
                Ok(self.recover_exact_pending_transaction(
                    binding,
                    original_action_ref,
                    journal_identity_digest,
                    journal_state_digest,
                )?)
            }
            ProductionAction::LifecycleSessionEnd { .. }
            | ProductionAction::Update { .. }
            | ProductionAction::None => Err(blocked(
                "invalid_local_apply",
                "host/read action reached local apply",
            )),
        };
        match result {
            Ok(observation)
                if plan.kind == OperationKind::Transaction
                    && !matches!(action, ProductionAction::PendingRecovery { .. }) =>
            {
                self.transition_journal_applied(
                    action_ref,
                    plan,
                    action,
                    &observation.output_digest,
                )
                .map(|_| observation.clone())
                .map_err(|mut error| {
                    error.effect_started = true;
                    error.output_digest = observation.output_digest.clone();
                    error
                        .observed_write_set
                        .extend(observation.observed_write_set.iter().cloned());
                    error.observed_write_set.sort();
                    error.observed_write_set.dedup();
                    error
                })
            }
            other => other,
        }
    }

    fn verify(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        observation: &EffectObservation,
    ) -> Result<VerificationObservation, EffectError> {
        let passed = match action {
            ProductionAction::Files(writes) => {
                let mut passed = true;
                for write in writes {
                    passed &= anchored_read(&write.target, false)?.as_deref()
                        == Some(write.content.as_slice());
                }
                passed
            }
            ProductionAction::Deletes(deletes) => {
                let mut passed = true;
                for delete in deletes {
                    passed &= anchored_read(&delete.target, false)?.is_none();
                }
                passed
            }
            ProductionAction::Projections(_) => observation.succeeded,
            ProductionAction::ProjectTest { .. } => observation.succeeded,
            ProductionAction::SkillChange {
                materialized,
                mutations,
                ..
            } => {
                mutations.iter().try_fold(true, |passed, mutation| {
                    postimage_matches(&mutation.postimage, &mutation.target)
                        .map(|matches| passed && matches)
                })? && materialized_route_verification(materialized)
                    .get("passed")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            }
            ProductionAction::PendingRecovery { .. } => observation.succeeded,
            ProductionAction::LifecycleSessionEnd { .. }
            | ProductionAction::Update { .. }
            | ProductionAction::None => false,
        };
        #[cfg(test)]
        let passed = passed
            && !(plan.kind == OperationKind::Transaction
                && !matches!(action, ProductionAction::PendingRecovery { .. })
                && FAIL_NEXT_TRANSACTION_VERIFY.with(|flag| flag.replace(false)));
        let output_digest = sha256(format!("verified:{passed}"));
        if passed
            && plan.kind == OperationKind::Transaction
            && !matches!(action, ProductionAction::PendingRecovery { .. })
        {
            self.commit_journal(action_ref, plan, &output_digest)?;
        }
        Ok(VerificationObservation {
            passed,
            output_digest,
        })
    }

    fn recover(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        observation: &EffectObservation,
    ) -> Result<RecoveryObservation, EffectError> {
        if plan.kind == OperationKind::Transaction {
            return self.recover_journal(action_ref, plan, action);
        }
        match action {
            ProductionAction::Files(writes) => {
                for write in writes.iter().rev() {
                    if !observation
                        .observed_write_set
                        .iter()
                        .any(|path| path == &write.target.sealed)
                    {
                        continue;
                    }
                    let current = anchored_read(&write.target, false)?;
                    if current == write.previous {
                        continue;
                    }
                    if current.as_deref() != Some(write.content.as_slice()) {
                        return Err(blocked(
                            "transaction_recovery_drift",
                            format!(
                                "applied target changed before recovery: {}",
                                write.target.sealed
                            ),
                        ));
                    }
                    match &write.previous {
                        Some(previous) => {
                            anchored_write(&write.target, Some(write.content.as_slice()), previous)?
                        }
                        None => anchored_delete(&write.target, &write.content)?,
                    }
                }
            }
            ProductionAction::Deletes(deletes) => {
                for delete in deletes.iter().rev() {
                    if !observation
                        .observed_write_set
                        .iter()
                        .any(|path| path == &delete.target.sealed)
                    {
                        continue;
                    }
                    let current = anchored_read(&delete.target, false)?;
                    if current.as_deref() == Some(delete.previous.as_slice()) {
                        continue;
                    }
                    if current.is_some() {
                        return Err(blocked(
                            "transaction_recovery_drift",
                            format!(
                                "deleted target was recreated before recovery: {}",
                                delete.target.sealed
                            ),
                        ));
                    }
                    anchored_write(&delete.target, None, &delete.previous)?;
                }
            }
            ProductionAction::Projections(_)
            | ProductionAction::SkillChange { .. }
            | ProductionAction::PendingRecovery { .. }
            | ProductionAction::LifecycleSessionEnd { .. }
            | ProductionAction::Update { .. }
            | ProductionAction::None
            | ProductionAction::ProjectTest { .. } => {
                return Ok(RecoveryObservation {
                    succeeded: false,
                    output_digest: sha256("not-recoverable"),
                    observed_write_set: Vec::new(),
                    evidence: None,
                    original_journal_digest: None,
                });
            }
        }
        Ok(RecoveryObservation {
            succeeded: true,
            output_digest: sha256("transaction-recovered"),
            observed_write_set: observation.observed_write_set.clone(),
            evidence: None,
            original_journal_digest: None,
        })
    }
}

fn read_workspace_text(
    binding: &AuthenticatedBinding,
    requested: &str,
    code: &str,
) -> Result<(PathBuf, String), EffectError> {
    let path = workspace_artifact(binding, requested)?;
    let body = read_binding_text(binding, &path, MAX_CLOSURE_ARTIFACT_BYTES, code)?;
    Ok((path, body))
}

fn read_binding_bytes(
    binding: &AuthenticatedBinding,
    path: &Path,
    limit: usize,
    code: &str,
) -> Result<Vec<u8>, EffectError> {
    let bytes = descriptor_read_host_artifact(binding, path, limit as u64)
        .map_err(|error| blocked(code, format!("{}: {}", error.code, error.detail)))?
        .ok_or_else(|| blocked(code, format!("{} does not exist", path.display())))?;
    Ok(bytes)
}

fn read_binding_text(
    binding: &AuthenticatedBinding,
    path: &Path,
    limit: usize,
    code: &str,
) -> Result<String, EffectError> {
    String::from_utf8(read_binding_bytes(binding, path, limit, code)?)
        .map_err(|error| blocked(code, format!("{} is not UTF-8: {error}", path.display())))
}

fn task_validation_result(path: &Path, body: &str) -> serde_json::Value {
    match ags_task_contract::validator::parse_validated(body) {
        Ok(card) => {
            let closure = ags_task_contract::validator::closure_contract(&card);
            serde_json::json!({
                "schema_version": "ags://schema/contract/v2/task-validation-result",
                "valid": true,
                "path": path,
                "digest": sha256(body),
                "fields": card.fields,
                "closure": {
                    "contract_id": closure.contract_id,
                    "handoff_source": closure.handoff_source,
                    "goal_ids": closure.goal_ids,
                    "acceptance_criteria_ids": closure.acceptance_criteria_ids,
                    "verification_ids": closure.verification_ids,
                    "evidence_ids": closure.evidence_ids
                },
                "errors": []
            })
        }
        Err(errors) => serde_json::json!({
            "schema_version": "ags://schema/contract/v2/task-validation-result",
            "valid": false,
            "path": path,
            "digest": sha256(body),
            "errors": errors
        }),
    }
}

fn task_policy_result(path: &Path, body: &str) -> Result<serde_json::Value, EffectError> {
    let card = match ags_task_contract::validator::parse_validated(body) {
        Ok(card) => card,
        Err(errors) => {
            return serde_json::to_value(ags_governance_decision::policy::gate_check_failed(
                "task_card_invalid",
                errors,
            ))
            .map_err(|error| blocked("policy_result_encode_failed", error.to_string()));
        }
    };
    let input = ags_governance_decision::policy::TaskPolicyInput::from_fields(&card.fields);
    let mut value = serde_json::to_value(ags_governance_decision::policy::explain_policy(&input))
        .map_err(|error| blocked("policy_result_encode_failed", error.to_string()))?;
    if let Some(object) = value.as_object_mut() {
        object.insert("task_card_path".to_string(), serde_json::json!(path));
        object.insert(
            "task_card_hash".to_string(),
            serde_json::json!(sha256(body)),
        );
    }
    Ok(value)
}

fn task_gate_result(path: &Path, body: &str) -> Result<serde_json::Value, EffectError> {
    let value = match ags_task_contract::validator::parse_validated(body) {
        Ok(card) => {
            let input = ags_governance_decision::policy::TaskPolicyInput::from_fields(&card.fields);
            serde_json::to_value(ags_governance_decision::policy::gate_check(&input))
        }
        Err(errors) => serde_json::to_value(ags_governance_decision::policy::gate_check_failed(
            "task_card_invalid",
            errors,
        )),
    }
    .map_err(|error| blocked("gate_result_encode_failed", error.to_string()))?;
    let mut value = value;
    if let Some(object) = value.as_object_mut() {
        object.insert("task_card_path".to_string(), serde_json::json!(path));
        object.insert(
            "task_card_hash".to_string(),
            serde_json::json!(sha256(body)),
        );
    }
    Ok(value)
}

fn verify_evidence(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    request: &EvidenceRequest,
) -> Result<serde_json::Value, EffectError> {
    let path = if request.artifact_kind == EvidenceArtifactKind::Receipt {
        normalized_binding_path(binding, &request.path)?
    } else {
        workspace_artifact(binding, &request.path)?
    };
    let bytes = read_binding_bytes(
        binding,
        &path,
        MAX_CLOSURE_ARTIFACT_BYTES,
        "evidence_read_failed",
    )?;
    let digest = sha256(&bytes);
    match &request.artifact_kind {
        EvidenceArtifactKind::LaunchPlan => {
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| blocked("launch_plan_invalid", error.to_string()))?;
            let schema_ok = value
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                == Some(ags_task_contract::runner::SCHEMA_VERSION);
            let declared = value
                .get("launch_plan_hash")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let actual = ags_task_contract::runner::canonical_launch_plan_hash(&value)
                .map_err(|detail| blocked("launch_plan_invalid", detail))?;
            Ok(serde_json::json!({
                "artifact_kind": "launch_plan",
                "path": path,
                "digest": digest,
                "valid": schema_ok && declared == actual,
                "schema_valid": schema_ok,
                "declared_hash": declared,
                "actual_hash": actual
            }))
        }
        EvidenceArtifactKind::DeliveryReport => {
            let task_path = request.task_card_path.as_deref().ok_or_else(|| {
                blocked(
                    "delivery_report_context_required",
                    "task_card_path and launch_plan_path are required",
                )
            })?;
            let plan_path = request.launch_plan_path.as_deref().ok_or_else(|| {
                blocked(
                    "delivery_report_context_required",
                    "task_card_path and launch_plan_path are required",
                )
            })?;
            let (_, task) = read_workspace_text(binding, task_path, "task_card_read_failed")?;
            let (_, plan) = read_workspace_text(binding, plan_path, "launch_plan_read_failed")?;
            let report = String::from_utf8(bytes)
                .map_err(|error| blocked("delivery_report_invalid_utf8", error.to_string()))?;
            let result = ags_evidence::delivery_report::validate(&task, &plan, &report);
            serde_json::to_value(result)
                .map_err(|error| blocked("delivery_report_result_encode_failed", error.to_string()))
        }
        EvidenceArtifactKind::Receipt => {
            let receipt: ags_evidence::Receipt = serde_json::from_slice(&bytes)
                .map_err(|error| blocked("receipt_invalid", error.to_string()))?;
            let seal = adapter
                .closure_authority_key(binding)
                .and_then(|machine_key| {
                    verify_canonical_closure_seal(
                        binding,
                        &machine_key,
                        &path,
                        &bytes,
                        &receipt,
                        "receipt_unproven",
                    )
                });
            Ok(serde_json::json!({
                "artifact_kind": "receipt",
                "path": path,
                "digest": digest,
                "valid": seal.is_ok(),
                "proof": if seal.is_ok() { "canonical_task_close_seal" } else { "structural_unproven" },
                "proof_error": seal.err().map(|error| format!("{}: {}", error.code, error.detail))
            }))
        }
        EvidenceArtifactKind::TestReceipt => {
            let receipt: ags_verification::TestReceipt = serde_json::from_slice(&bytes)
                .map_err(|error| blocked("test_receipt_invalid", error.to_string()))?;
            let workspace_ok = Path::new(&receipt.canonical_workspace)
                .canonicalize()
                .is_ok_and(|path| path == binding.canonical_workspace());
            let schema_ok = receipt.schema_version == "ags://schema/contract/v2/test-receipt";
            let hashes_ok = [
                receipt.commit_hash.as_str(),
                receipt.tree_hash.as_str(),
                receipt.workspace_tree_hash.as_str(),
                receipt.argv_hash.as_str(),
                receipt.output_digest.as_str(),
            ]
            .iter()
            .all(|hash| *hash == "unborn" || hash.starts_with("sha256:") || hash.len() == 40);
            Ok(serde_json::json!({
                "artifact_kind": "test_receipt",
                "path": path,
                "digest": digest,
                "valid": schema_ok && workspace_ok && hashes_ok && receipt.closed,
                "schema_valid": schema_ok,
                "workspace_binding_valid": workspace_ok,
                "hash_fields_valid": hashes_ok,
                "receipt": receipt
            }))
        }
    }
}

fn verify_canonical_closure_seal(
    binding: &AuthenticatedBinding,
    machine_key: &[u8; 32],
    receipt_path: &Path,
    receipt_bytes: &[u8],
    receipt: &ags_evidence::Receipt,
    code: &str,
) -> Result<(PathBuf, Vec<u8>), EffectError> {
    let expected_receipt_id =
        ags_evidence::receipt_id(&receipt.task_card_hash, &receipt.launch_plan_hash);
    let hashes_valid = [
        receipt.task_card_hash.as_str(),
        receipt.launch_plan_hash.as_str(),
        receipt.delivery_report_hash.as_str(),
    ]
    .iter()
    .all(|hash| hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit()));
    if receipt.receipt_id != expected_receipt_id || !hashes_valid {
        return Err(blocked(
            code,
            "receipt identity or artifact hashes are invalid",
        ));
    }
    let canonical_receipt_path = binding
        .canonical_workspace()
        .join(".ags/evidence")
        .join(format!("{}.json", receipt.receipt_id));
    if receipt_path != canonical_receipt_path {
        return Err(blocked(
            code,
            "receipt is not the canonical task-close receipt",
        ));
    }
    let pointer_path = binding
        .canonical_workspace()
        .join(".ags/state/closure-pointers")
        .join(format!("{}.json", receipt.receipt_id));
    let pointer_bytes =
        read_binding_bytes(binding, &pointer_path, MAX_CLOSURE_ARTIFACT_BYTES, code)?;
    let pointer: crate::workspace_lifecycle::ClosurePointer =
        serde_json::from_slice(&pointer_bytes)
            .map_err(|error| blocked(code, format!("invalid canonical pointer: {error}")))?;
    crate::workspace_lifecycle::verify_closure_pointer_authority(machine_key, &pointer)
        .map_err(|detail| blocked(code, detail))?;
    let workspace_identity =
        crate::workspace_lifecycle::workspace_identity(binding.canonical_workspace());
    let valid = pointer.schema_version
        == crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION
        && pointer.canonical_workspace.as_deref()
            == Some(binding.canonical_workspace().to_string_lossy().as_ref())
        && pointer.workspace_identity.as_deref() == Some(workspace_identity.as_str())
        && pointer.receipt_id == receipt.receipt_id
        && pointer.receipt_path == canonical_receipt_path.display().to_string()
        && pointer.receipt_sha256 == sha256(receipt_bytes)
        && pointer.task_card_hash == receipt.task_card_hash
        && pointer.launch_plan_hash == receipt.launch_plan_hash
        && pointer.delivery_report_hash == receipt.delivery_report_hash;
    if !valid {
        return Err(blocked(code, "canonical closure pointer mismatch"));
    }
    Ok((pointer_path, pointer_bytes))
}

fn generic_agent_probe(request: &AgentProbeRequest) -> Result<serde_json::Value, EffectError> {
    let surface = match request.surface {
        AgentSurface::Cli => ags_host_integration::AgentSurface::Cli,
        AgentSurface::Mcp => ags_host_integration::AgentSurface::Mcp,
        AgentSurface::Hybrid => ags_host_integration::AgentSurface::Hybrid,
    };
    let agent = ags_host_integration::GenericAgent::new(&request.host_id, surface)
        .map_err(|detail| blocked("agent_host_invalid", detail))?;
    let official = agent.official_adapter();
    Ok(serde_json::json!({
        "schema_version": "ags://schema/contract/v2/agent-probe-result",
        "host_id": agent.host_id.as_str(),
        "surface": request.surface,
        "governable": true,
        "official_adapter": official.map(|spec| spec.id),
        "transport": {
            "cli_supported": matches!(request.surface, AgentSurface::Cli | AgentSurface::Hybrid),
            "mcp_supported": matches!(request.surface, AgentSurface::Mcp | AgentSurface::Hybrid),
            "mcp_tools": ["ags_decide", "ags_apply"]
        },
        "registration_mutated": false,
        "probe_status": if official.is_some() { "adapter_metadata_available" } else { "generic_contract_ready" }
    }))
}

fn canonical_skill_source(
    source: &SkillSourceSpec,
) -> ags_capability_governance::skill_adoption::SourceSpec {
    use ags_capability_governance::skill_adoption::SourceSpec;
    match source.kind {
        SkillSourceKind::Local => SourceSpec::Local {
            path: source.uri.clone(),
        },
        SkillSourceKind::GitHub => SourceSpec::GitHub {
            url: source.uri.clone(),
            requested_ref: source.requested_ref.clone(),
            tracking_ref: source.tracking_ref.clone(),
            subdir: source.subdir.clone(),
        },
        SkillSourceKind::Git => SourceSpec::Git {
            url: source.uri.clone(),
            requested_ref: source.requested_ref.clone(),
            tracking_ref: source.tracking_ref.clone(),
            subdir: source.subdir.clone(),
        },
    }
}

fn skill_change_domain_plan(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    context: ags_capability_governance::skill_adoption::AdoptionContext,
    plan: ags_capability_governance::skill_adoption::PreparedSkillChange,
    acknowledgements: ags_capability_governance::skill_adoption::RiskAcknowledgements,
) -> Result<PlanDisposition<ProductionAction>, EffectError> {
    let materialized = ags_capability_governance::skill_adoption::materialize_skill_change(
        &context,
        &plan,
        &acknowledgements,
    )
    .map_err(|detail| blocked("skill_change_materialization_failed", detail))?;
    materialized_skill_change_domain_plan(adapter, binding, context, plan, materialized)
}

fn materialized_skill_change_domain_plan(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    context: ags_capability_governance::skill_adoption::AdoptionContext,
    plan: ags_capability_governance::skill_adoption::PreparedSkillChange,
    materialized: ags_capability_governance::skill_adoption::MaterializedSkillChange,
) -> Result<PlanDisposition<ProductionAction>, EffectError> {
    let mutations = materialized_skill_mutations(adapter, binding, &materialized)?;
    let mut expected_write_paths = materialized.write_paths();
    expected_write_paths.sort();
    expected_write_paths.dedup();
    let mutation_paths = mutations
        .iter()
        .map(|mutation| mutation.target.sealed.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if mutation_paths != expected_write_paths.iter().cloned().collect() {
        return Err(blocked(
            "skill_materialized_write_set_mismatch",
            materialized.materialization_hash.clone(),
        ));
    }
    let action = ProductionAction::SkillChange {
        context,
        materialized: Box::new(materialized),
        mutations,
    };
    let action_digest = canonical_production_action_digest(&action)?;
    Ok(PlanDisposition::Planned(Box::new(PlannedDomain {
        plan: DomainPlan {
            action_digest,
            steps: vec![PlanStep {
                step_id: format!("skill-{}", plan.operation),
                description: format!(
                    "apply canonical Skill {} transaction for {}",
                    plan.operation, plan.skill_id
                ),
            }],
            expected_write_paths,
            verification: VerificationSpec {
                checks: vec![
                    "installed-index-cas".to_string(),
                    "immutable-body-hash".to_string(),
                    "host-index-and-snapshot-route".to_string(),
                ],
            },
            recoverability: Recoverability::Transactional,
            execution: None,
        },
        action,
    })))
}

fn canonical_target(target: &AnchoredPath) -> Result<serde_json::Value, EffectError> {
    let parent_identity = target
        .parent_anchor
        .lock()
        .map_err(|_| blocked("transaction_parent_anchor_poisoned", &target.sealed))?
        .identity;
    let target_identity = *target
        .target_identity
        .lock()
        .map_err(|_| blocked("transaction_target_anchor_poisoned", &target.sealed))?;
    Ok(serde_json::json!({
        "root_label": target.root.label,
        "root_path": target.root.canonical,
        "root_identity": target.root.identity,
        "relative": target.relative,
        "sealed": target.sealed,
        "parent_identity": parent_identity,
        "target_identity": target_identity,
    }))
}

fn canonical_mutations(
    mutations: &[PlannedObjectMutation],
) -> Result<Vec<serde_json::Value>, EffectError> {
    mutations
        .iter()
        .map(|mutation| {
            Ok(serde_json::json!({
                "target": canonical_target(&mutation.target)?,
                "operation": mutation.operation,
                "preimage": mutation.preimage,
                "postimage": mutation.postimage,
                "apply": mutation.apply,
            }))
        })
        .collect()
}

fn canonical_production_action(
    action: &ProductionAction,
) -> Result<serde_json::Value, EffectError> {
    Ok(match action {
        ProductionAction::Files(files) => serde_json::json!({
            "kind": "files",
            "files": files.iter().map(|file| Ok(serde_json::json!({
                "target": canonical_target(&file.target)?,
                "content": file.content,
                "previous": file.previous,
            }))).collect::<Result<Vec<_>, EffectError>>()?,
        }),
        ProductionAction::Deletes(deletes) => serde_json::json!({
            "kind": "deletes",
            "deletes": deletes.iter().map(|delete| Ok(serde_json::json!({
                "target": canonical_target(&delete.target)?,
                "previous": delete.previous,
            }))).collect::<Result<Vec<_>, EffectError>>()?,
        }),
        ProductionAction::Projections(projections) => serde_json::json!({
            "kind": "projections",
            "projections": projections.iter().map(|projection| Ok(serde_json::json!({
                "workspace": projection.workspace,
                "planned_directory_paths": projection.planned_directory_paths,
                "mutations": canonical_mutations(&projection.mutations)?,
            }))).collect::<Result<Vec<_>, EffectError>>()?,
        }),
        ProductionAction::ProjectTest {
            workspace,
            profile,
            spec,
        } => serde_json::json!({
            "kind": "project_test",
            "workspace": workspace,
            "profile": format!("{profile:?}"),
            "spec": spec,
        }),
        ProductionAction::SkillChange {
            context,
            materialized,
            mutations,
        } => serde_json::json!({
            "kind": "skill_change",
            "context": {
                "authority_root": context.authority_root,
                "runtime_home": context.runtime_home,
                "candidate_home": context.candidate_home,
                "host_home": context.host_home,
                "snapshot_discovery": format!("{:?}", context.snapshot_discovery),
            },
            "materialized": materialized,
            "mutations": canonical_mutations(mutations)?,
        }),
        ProductionAction::PendingRecovery {
            original_action_ref,
            journal_identity_digest,
            journal_state_digest,
            expected_write_paths,
        } => serde_json::json!({
            "kind": "pending_recovery",
            "original_action_ref": original_action_ref,
            "journal_identity_digest": journal_identity_digest,
            "journal_state_digest": journal_state_digest,
            "expected_write_paths": expected_write_paths,
        }),
        ProductionAction::LifecycleSessionEnd {
            receipt_ids,
            pointer_paths,
        } => serde_json::json!({
            "kind": "lifecycle_session_end",
            "receipt_ids": receipt_ids,
            "pointer_paths": pointer_paths,
        }),
        ProductionAction::Update {
            candidate_directory,
            release_directory,
            manifest,
            tree_digest,
            members,
        } => serde_json::json!({
            "kind": "update",
            "candidate_directory": candidate_directory,
            "release_directory": release_directory,
            "manifest": manifest,
            "tree_digest": tree_digest,
            "members": members,
        }),
        ProductionAction::None => serde_json::json!({"kind": "none"}),
    })
}

fn host_release_member(member: &ReleaseMember) -> HostReleaseMember {
    HostReleaseMember {
        name: member.name.clone(),
        sha256: member.sha256.clone(),
        size: member.size,
        mode: member.mode,
    }
}

fn canonical_production_action_digest(action: &ProductionAction) -> Result<String, EffectError> {
    if let ProductionAction::PendingRecovery {
        journal_identity_digest,
        journal_state_digest,
        expected_write_paths,
        ..
    } = action
    {
        return Ok(sha256(format!(
            "ags-control-plane/recovery-action/v2\n{}\n{}\n{}",
            journal_identity_digest,
            journal_state_digest,
            expected_write_paths.join("\n")
        )));
    }
    Ok(sha256(
        serde_json::to_vec(&canonical_production_action(action)?)
            .map_err(|error| blocked("production_action_encode_failed", error.to_string()))?,
    ))
}

fn materialized_hash(
    materialized: &ags_capability_governance::skill_adoption::MaterializedSkillChange,
) -> Result<String, EffectError> {
    Ok(sha256(
        serde_json::to_vec(&(
            &materialized.operation,
            &materialized.skill_id,
            materialized.registry_revision,
            &materialized.registry,
            &materialized.parent_directories,
            &materialized.body,
            &materialized.links,
            &materialized.snapshots,
            &materialized.read_inputs,
        ))
        .map_err(|error| {
            blocked(
                "skill_materialization_hash_encode_failed",
                error.to_string(),
            )
        })?,
    ))
}

fn materialized_skill_mutations(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    materialized: &ags_capability_governance::skill_adoption::MaterializedSkillChange,
) -> Result<Vec<PlannedObjectMutation>, EffectError> {
    use ags_capability_governance::skill_adoption::{
        MaterializedBodyDisposition, MaterializedBodyNode, MaterializedDirectory,
        MaterializedRegularFile,
    };

    let mut mutations = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut directories = materialized.parent_directories.clone();
    directories.sort_by(|left, right| {
        Path::new(&left.path)
            .components()
            .count()
            .cmp(&Path::new(&right.path).components().count())
            .then(left.path.cmp(&right.path))
    });
    for directory in &directories {
        push_materialized_directory(
            adapter,
            binding,
            directory,
            "skill_parent",
            &mut seen,
            &mut mutations,
        )?;
    }

    if let MaterializedBodyDisposition::CreateExact(body) = &materialized.body {
        push_materialized_directory(
            adapter,
            binding,
            &MaterializedDirectory {
                path: body.root.clone(),
                mode: body.root_mode,
            },
            "skill_body",
            &mut seen,
            &mut mutations,
        )?;
        let mut nodes = body.nodes.clone();
        nodes.sort_by(|left, right| {
            let left_path = match left {
                MaterializedBodyNode::Directory { relative_path, .. }
                | MaterializedBodyNode::RegularFile { relative_path, .. } => relative_path,
            };
            let right_path = match right {
                MaterializedBodyNode::Directory { relative_path, .. }
                | MaterializedBodyNode::RegularFile { relative_path, .. } => relative_path,
            };
            Path::new(left_path)
                .components()
                .count()
                .cmp(&Path::new(right_path).components().count())
                .then(left_path.cmp(right_path))
        });
        for node in nodes {
            match node {
                MaterializedBodyNode::Directory {
                    relative_path,
                    mode,
                } => push_materialized_directory(
                    adapter,
                    binding,
                    &MaterializedDirectory {
                        path: Path::new(&body.root)
                            .join(relative_path)
                            .to_string_lossy()
                            .into_owned(),
                        mode,
                    },
                    "skill_body",
                    &mut seen,
                    &mut mutations,
                )?,
                MaterializedBodyNode::RegularFile {
                    relative_path,
                    bytes,
                    mode,
                } => {
                    let file = MaterializedRegularFile {
                        path: Path::new(&body.root)
                            .join(relative_path)
                            .to_string_lossy()
                            .into_owned(),
                        pre_bytes: None,
                        post_bytes: bytes,
                        pre_mode: None,
                        post_mode: mode,
                    };
                    push_materialized_file(
                        adapter,
                        binding,
                        &file,
                        "skill_body",
                        &mut seen,
                        &mut mutations,
                    )?;
                }
            }
        }
    }

    push_materialized_file(
        adapter,
        binding,
        &materialized.registry,
        "skill_index",
        &mut seen,
        &mut mutations,
    )?;
    for link in &materialized.links {
        if link.previous_target == link.post_target {
            continue;
        }
        if !seen.insert(link.path.clone()) {
            return Err(blocked("skill_materialized_duplicate_path", &link.path));
        }
        let target = adapter.anchored_symlink_target(binding, Path::new(&link.path))?;
        let preimage = match &link.previous_target {
            Some(previous) => JournalImage::Symlink {
                target_hex: encode_hex(previous),
            },
            None => JournalImage::Absent,
        };
        if !preimage_matches(&preimage, &target)? {
            return Err(blocked("skill_materialized_preimage_drift", &link.path));
        }
        let postimage = match &link.post_target {
            Some(next) => JournalPostimage::Symlink {
                target_hex: encode_hex(next),
            },
            None => JournalPostimage::Absent,
        };
        mutations.push(PlannedObjectMutation {
            target,
            operation: "skill_link".to_string(),
            preimage,
            postimage,
            apply: ObjectMutationApply::SetSymlink {
                previous: link.previous_target.clone(),
                next: link.post_target.clone(),
            },
        });
    }
    for snapshot in &materialized.snapshots {
        push_materialized_file(
            adapter,
            binding,
            &snapshot.file,
            "skill_snapshot",
            &mut seen,
            &mut mutations,
        )?;
    }
    Ok(mutations)
}

fn push_materialized_file(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    file: &ags_capability_governance::skill_adoption::MaterializedRegularFile,
    operation: &str,
    seen: &mut std::collections::BTreeSet<String>,
    mutations: &mut Vec<PlannedObjectMutation>,
) -> Result<(), EffectError> {
    if !seen.insert(file.path.clone()) {
        return Err(blocked("skill_materialized_duplicate_path", &file.path));
    }
    let target = adapter.anchored_target(binding, Path::new(&file.path))?;
    let preimage = match (&file.pre_bytes, file.pre_mode) {
        (None, None) => JournalImage::Absent,
        (Some(bytes), Some(mode)) => JournalImage::RegularFile {
            sha256: sha256(bytes),
            data_hex: encode_hex(bytes),
            mode,
        },
        _ => {
            return Err(blocked(
                "skill_materialized_file_preimage_invalid",
                &file.path,
            ))
        }
    };
    if !preimage_matches(&preimage, &target)? {
        return Err(blocked("skill_materialized_preimage_drift", &file.path));
    }
    mutations.push(PlannedObjectMutation {
        target,
        operation: operation.to_string(),
        preimage,
        postimage: JournalPostimage::RegularFile {
            sha256: sha256(&file.post_bytes),
            mode: file.post_mode,
        },
        apply: ObjectMutationApply::WriteFile {
            previous: file.pre_bytes.clone(),
            next: file.post_bytes.clone(),
        },
    });
    Ok(())
}

fn push_materialized_directory(
    adapter: &ProductionEffectAdapter,
    binding: &AuthenticatedBinding,
    directory: &ags_capability_governance::skill_adoption::MaterializedDirectory,
    operation: &str,
    seen: &mut std::collections::BTreeSet<String>,
    mutations: &mut Vec<PlannedObjectMutation>,
) -> Result<(), EffectError> {
    if !seen.insert(directory.path.clone()) {
        return Ok(());
    }
    let target = adapter.anchored_target(binding, Path::new(&directory.path))?;
    let preimage = target_journal_image(&target)?;
    if preimage != JournalImage::Absent {
        return Err(blocked(
            "skill_materialized_preimage_drift",
            &directory.path,
        ));
    }
    mutations.push(PlannedObjectMutation {
        target,
        operation: operation.to_string(),
        preimage,
        postimage: JournalPostimage::Directory {
            mode: directory.mode,
        },
        apply: ObjectMutationApply::CreateDirectory,
    });
    Ok(())
}

fn verify_update_host_outcome(
    request: &UpdateRequest,
    plan: &SealedPlan,
    action: &ProductionAction,
    outcome: &HostOutcomeReceipt,
    evidence: &VerifiedHostEvidence,
) -> Result<(), EffectError> {
    if evidence.kind() != HostEvidenceKind::UpdateReceipt {
        return Err(blocked(
            "update_evidence_kind_invalid",
            evidence.artifact().uri.clone(),
        ));
    }
    let receipt: UpdateReceipt = serde_json::from_slice(evidence.bytes())
        .map_err(|error| blocked("update_receipt_invalid", error.to_string()))?;
    let ProductionAction::Update {
        candidate_directory: _,
        release_directory,
        manifest,
        tree_digest,
        members,
    } = action
    else {
        return Err(blocked(
            "update_action_invalid",
            "update action did not seal release authority",
        ));
    };
    let common = receipt.schema_version == "ags://schema/contract/v2/update-receipt"
        && receipt.channel == request.channel
        && receipt.target_version == request.target_version
        && receipt.action_ref == outcome.action_ref
        && receipt.binding_hash == outcome.binding_hash
        && receipt.plan_hash == outcome.plan_hash
        && receipt.observed_write_set == outcome.observed_write_set
        && is_verified_plan_subset(plan, &outcome.observed_write_set)
        && receipt.release_manifest_sha256 == manifest.sha256
        && receipt.release_tree_digest == *tree_digest
        && receipt.output_digest == outcome.output_digest;
    let valid = common
        && match outcome.status {
            HostOutcomeStatus::Succeeded => {
                let mut expected_names = RELEASE_PAYLOAD_NAMES.to_vec();
                expected_names.push("release-manifest.json");
                let scanned = scan_release_directory(release_directory, &expected_names)?;
                let scanned_payload = scanned
                    .iter()
                    .filter(|member| member.name != "release-manifest.json")
                    .cloned()
                    .collect::<Vec<_>>();
                let manifest_member = scanned
                    .iter()
                    .find(|member| member.name == "release-manifest.json");
                outcome.observed_write_set == plan.expected_write_paths
                    && scanned_payload == *members
                    && manifest_member == Some(manifest)
                    && receipt.completed
            }
            HostOutcomeStatus::Failed | HostOutcomeStatus::Abandoned => !receipt.completed,
        };
    if valid {
        Ok(())
    } else {
        Err(blocked(
            "update_receipt_binding_mismatch",
            evidence.artifact().uri.clone(),
        ))
    }
}

fn verify_host_test_outcome(
    request: &TestRequest,
    plan: &SealedPlan,
    binding: &AuthenticatedBinding,
    outcome: &HostOutcomeReceipt,
    evidence: &VerifiedHostEvidence,
) -> Result<(), EffectError> {
    if evidence.kind() != HostEvidenceKind::TestReceipt {
        return Err(blocked(
            "host_test_evidence_kind_invalid",
            evidence.artifact().uri.clone(),
        ));
    }
    let receipt: ags_verification::TestReceipt = serde_json::from_slice(evidence.bytes())
        .map_err(|error| blocked("host_test_receipt_invalid", error.to_string()))?;
    let expected_profile = match request.profile {
        TestProfile::Smoke => ags_verification::TestProfile::Smoke,
        TestProfile::Standard => ags_verification::TestProfile::Standard,
        TestProfile::Full => ags_verification::TestProfile::Full,
    };
    let command = plan.execution.as_ref().ok_or_else(|| {
        blocked(
            "host_test_command_missing",
            "HostDelegated Test plan must seal one CommandSpec",
        )
    })?;
    let expected_argv_hash = sha256(
        serde_json::to_vec(command)
            .map_err(|error| blocked("host_test_command_encode_failed", error.to_string()))?,
    );
    let expected_writes = receipt
        .observed_write_set
        .iter()
        .map(|path| {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                binding.canonical_workspace().join(path)
            }
            .display()
            .to_string()
        })
        .collect::<Vec<_>>();
    let git_hashes_valid = |value: &str| value == "unborn" || ags_platform::is_git_commit(value);
    let status_valid = match outcome.status {
        HostOutcomeStatus::Succeeded => {
            receipt.status == ags_verification::TestExecutionStatus::Succeeded
                && receipt.exit_code == 0
        }
        HostOutcomeStatus::Failed | HostOutcomeStatus::Abandoned => {
            receipt.status != ags_verification::TestExecutionStatus::Succeeded
        }
    };
    let valid = receipt.schema_version == "ags://schema/contract/v2/test-receipt"
        && receipt.profile == expected_profile
        && receipt.canonical_workspace == binding.canonical_workspace().display().to_string()
        && git_hashes_valid(&receipt.commit_hash)
        && git_hashes_valid(&receipt.tree_hash)
        && ags_platform::is_sha256(&receipt.workspace_tree_hash)
        && receipt.argv_hash == expected_argv_hash
        && receipt.duration_ms <= command.timeout_ms
        && receipt.output_digest == outcome.output_digest
        && expected_writes == outcome.observed_write_set
        && receipt.unexpected_write_set.is_empty()
        && status_valid
        && receipt.closed
        && !receipt.source_rollback_performed;
    if valid {
        Ok(())
    } else {
        Err(blocked(
            "host_test_receipt_binding_mismatch",
            evidence.artifact().uri.clone(),
        ))
    }
}

fn verify_lifecycle_host_outcome(
    request: &LifecycleSessionEndRequest,
    plan: &SealedPlan,
    action: &ProductionAction,
    outcome: &HostOutcomeReceipt,
    evidence: &VerifiedHostEvidence,
) -> Result<(), EffectError> {
    if evidence.kind() != HostEvidenceKind::LifecycleReceipt {
        return Err(blocked(
            "lifecycle_host_evidence_kind_invalid",
            evidence.artifact().uri.clone(),
        ));
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LifecycleReceipt {
        schema_version: String,
        event_id: String,
        receipt_ids: Vec<String>,
        observed_write_set: Vec<String>,
        consumed_pointer_paths: Vec<String>,
        output_digest: String,
        completed: bool,
    }
    let verified: LifecycleReceipt = serde_json::from_slice(evidence.bytes())
        .map_err(|error| blocked("lifecycle_host_receipt_invalid", error.to_string()))?;
    let ProductionAction::LifecycleSessionEnd {
        receipt_ids,
        pointer_paths,
    } = action
    else {
        return Err(blocked(
            "lifecycle_host_action_invalid",
            "session-end action did not seal lifecycle authority",
        ));
    };
    let common = !receipt_ids.is_empty()
        && receipt_ids.len() == pointer_paths.len()
        && verified.schema_version == "ags://schema/contract/v2/lifecycle-host-outcome"
        && verified.event_id == request.event_id
        && verified.output_digest == outcome.output_digest
        && verified.receipt_ids == *receipt_ids
        && !verified.receipt_ids.iter().any(String::is_empty)
        && verified.observed_write_set == outcome.observed_write_set
        && is_verified_plan_subset(plan, &outcome.observed_write_set);
    let valid = common
        && match outcome.status {
            HostOutcomeStatus::Succeeded => {
                outcome.observed_write_set == plan.expected_write_paths
                    && outcome.artifacts.len() == plan.expected_write_paths.len()
                    && verified.consumed_pointer_paths == *pointer_paths
                    && pointer_paths.iter().all(|pointer| {
                        outcome.artifacts.iter().any(|artifact| {
                            artifact.path == *pointer && artifact.state == HostArtifactState::Absent
                        })
                    })
                    && verified.completed
            }
            HostOutcomeStatus::Failed | HostOutcomeStatus::Abandoned => {
                !verified.completed
                    && verified.consumed_pointer_paths.iter().all(|pointer| {
                        pointer_paths.contains(pointer)
                            && outcome.observed_write_set.contains(pointer)
                    })
            }
        };
    if !valid {
        return Err(blocked(
            "lifecycle_host_receipt_binding_mismatch",
            evidence.artifact().uri.clone(),
        ));
    }
    Ok(())
}

fn is_verified_plan_subset(plan: &SealedPlan, observed: &[String]) -> bool {
    let unique = observed.iter().collect::<std::collections::BTreeSet<_>>();
    unique.len() == observed.len()
        && observed
            .iter()
            .all(|path| plan.expected_write_paths.contains(path))
}

fn safe_id<'a>(value: &'a str, code: &str) -> Result<&'a str, EffectError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(blocked(code.to_string(), value.to_string()));
    }
    Ok(value)
}

fn scan_release_directory(
    directory_path: &Path,
    expected_names: &[&str],
) -> Result<Vec<ReleaseMember>, EffectError> {
    let directory = rustix::fs::open(
        directory_path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| effect_error("release_directory_open_failed", error, false))?;
    let mut names = Vec::new();
    let mut name_bytes = 0usize;
    for entry in rustix::fs::Dir::read_from(&directory)
        .map_err(|error| effect_error("release_directory_read_failed", error, false))?
    {
        let entry =
            entry.map_err(|error| effect_error("release_directory_read_failed", error, false))?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .map_err(|error| effect_error("release_member_name_invalid", error, false))?;
        if name == "." || name == ".." {
            continue;
        }
        if names.len() >= expected_names.len() {
            return Err(blocked(
                "release_directory_entry_budget_exceeded",
                "release directory exceeds the sealed member-count budget",
            ));
        }
        name_bytes = name_bytes.saturating_add(name.len());
        if name_bytes > MAX_RELEASE_NAME_BYTES {
            return Err(blocked(
                "release_directory_entry_budget_exceeded",
                "release directory exceeds the member name-byte budget",
            ));
        }
        names.push(name);
    }
    names.sort();
    let mut expected = expected_names
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    expected.sort();
    if names != expected {
        return Err(blocked(
            "release_member_set_mismatch",
            format!("expected {expected:?}, actual {names:?}"),
        ));
    }
    let mut members = Vec::with_capacity(names.len());
    for name in names {
        let descriptor = rustix::fs::openat(
            &directory,
            name.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| effect_error("release_member_open_failed", error, false))?;
        let path = directory_path.join(&name);
        let stable = read_regular_fd(&descriptor, MAX_RELEASE_MEMBER_BYTES as u64, || {
            #[cfg(all(test, unix))]
            run_release_after_read_rewrite_test_hook(&path);
        })
        .map_err(|error| match error {
            StableReadError::TooLarge => blocked("release_member_too_large", name.clone()),
            StableReadError::Changed => blocked(
                "release_member_changed_during_read",
                path.display().to_string(),
            ),
            StableReadError::NotRegular => blocked("release_member_not_regular", name.clone()),
            StableReadError::Io(error) => effect_error("release_member_read_failed", error, false),
        })?;
        members.push(ReleaseMember {
            name,
            sha256: sha256(&stable.bytes),
            size: stable.stable_stat.size,
            mode: stable.stable_stat.mode & 0o777,
        });
    }
    members.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(members)
}

fn release_tree_digest(members: &[ReleaseMember]) -> Result<String, EffectError> {
    serde_json::to_vec(members)
        .map(sha256)
        .map_err(|error| blocked("release_tree_encode_failed", error.to_string()))
}

fn ensure_bound_host(
    requested: &str,
    binding: &AuthenticatedBinding,
    code: &str,
) -> Result<String, EffectError> {
    let requested = ags_host_integration::HostId::new(requested)
        .map_err(|detail| blocked(code.to_string(), detail))?;
    let authenticated = ags_host_integration::HostId::new(binding.host_id())
        .map_err(|detail| blocked("binding_host_invalid", detail))?;
    if requested != authenticated {
        return Err(blocked(
            code.to_string(),
            format!(
                "requested host {} does not match authenticated host {}",
                requested, authenticated
            ),
        ));
    }
    Ok(requested.to_string())
}

fn workspace_artifact(binding: &AuthenticatedBinding, value: &str) -> Result<PathBuf, EffectError> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        binding.canonical_workspace().join(path)
    };
    let canonical = path
        .canonicalize()
        .map_err(|error| effect_error("workspace_artifact_invalid", error, false))?;
    if !canonical.starts_with(binding.canonical_workspace()) {
        return Err(blocked(
            "workspace_artifact_outside_binding",
            canonical.display().to_string(),
        ));
    }
    Ok(canonical)
}

fn normalized_binding_path(
    binding: &AuthenticatedBinding,
    value: &str,
) -> Result<PathBuf, EffectError> {
    let supplied = Path::new(value);
    let relative = if supplied.is_absolute() {
        supplied
            .strip_prefix(binding.canonical_workspace())
            .map_err(|_| blocked("workspace_artifact_outside_binding", value.to_string()))?
    } else {
        supplied
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(blocked(
            "workspace_artifact_invalid",
            "artifact path must be a normalized workspace-relative path",
        ));
    }
    Ok(binding.canonical_workspace().join(relative))
}

fn journal_image(bytes: Option<&[u8]>) -> JournalImage {
    match bytes {
        Some(bytes) => JournalImage::RegularFile {
            sha256: sha256(bytes),
            data_hex: encode_hex(bytes),
            mode: 0o600,
        },
        None => JournalImage::Absent,
    }
}

fn journal_writes(action: &ProductionAction) -> Result<Vec<JournalWrite>, EffectError> {
    let mut writes = Vec::new();
    let mut push = |target: &AnchoredPath,
                    operation: &str,
                    preimage: JournalImage,
                    postimage: JournalPostimage| {
        let order = writes.len();
        writes.push(JournalWrite {
            order,
            path: target.sealed.clone(),
            root_path: target.root.canonical.display().to_string(),
            root_device: target.root.identity.0,
            root_inode: target.root.identity.1,
            operation: operation.to_string(),
            preimage,
            postimage,
            apply_anchor: JournalApplyAnchor::Pending,
            recovery_progress: JournalWriteRecoveryProgress::Applied,
            post_identity: None,
        });
    };
    match action {
        ProductionAction::Files(files) => {
            for file in files {
                push(
                    &file.target,
                    if file.previous.is_some() {
                        "replace"
                    } else {
                        "create"
                    },
                    journal_image(file.previous.as_deref()),
                    JournalPostimage::RegularFile {
                        sha256: sha256(&file.content),
                        mode: 0o600,
                    },
                );
            }
        }
        ProductionAction::Deletes(deletes) => {
            for delete in deletes {
                push(
                    &delete.target,
                    "delete",
                    journal_image(Some(&delete.previous)),
                    JournalPostimage::Absent,
                );
            }
        }
        ProductionAction::Projections(projections) => {
            for projection in projections {
                for mutation in &projection.mutations {
                    push(
                        &mutation.target,
                        &mutation.operation,
                        mutation.preimage.clone(),
                        mutation.postimage.clone(),
                    );
                }
            }
        }
        ProductionAction::SkillChange { mutations, .. } => {
            for mutation in mutations {
                push(
                    &mutation.target,
                    &mutation.operation,
                    mutation.preimage.clone(),
                    mutation.postimage.clone(),
                );
            }
        }
        ProductionAction::ProjectTest { .. }
        | ProductionAction::PendingRecovery { .. }
        | ProductionAction::LifecycleSessionEnd { .. }
        | ProductionAction::Update { .. }
        | ProductionAction::None => {}
    }
    Ok(writes)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, EffectError> {
    if !value.len().is_multiple_of(2) {
        return Err(blocked(
            "transaction_journal_preimage_invalid",
            "hex preimage has odd length",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            let high = digit(pair[0]).ok_or_else(|| {
                blocked("transaction_journal_preimage_invalid", "invalid hex digit")
            })?;
            let low = digit(pair[1]).ok_or_else(|| {
                blocked("transaction_journal_preimage_invalid", "invalid hex digit")
            })?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn validate_journal_root(write: &JournalWrite, target: &AnchoredPath) -> Result<(), EffectError> {
    if write.root_path != target.root.canonical.display().to_string()
        || (write.root_device, write.root_inode) != target.root.identity
    {
        return Err(blocked(
            "transaction_journal_root_binding_mismatch",
            &write.path,
        ));
    }
    Ok(())
}

fn action_targets(action: &ProductionAction) -> std::collections::BTreeMap<String, AnchoredPath> {
    match action {
        ProductionAction::Files(files) => files
            .iter()
            .map(|file| (file.target.sealed.clone(), file.target.clone()))
            .collect(),
        ProductionAction::Deletes(deletes) => deletes
            .iter()
            .map(|delete| (delete.target.sealed.clone(), delete.target.clone()))
            .collect(),
        ProductionAction::Projections(projections) => projections
            .iter()
            .flat_map(|projection| projection.mutations.iter())
            .map(|mutation| (mutation.target.sealed.clone(), mutation.target.clone()))
            .collect(),
        ProductionAction::SkillChange { mutations, .. } => mutations
            .iter()
            .map(|mutation| (mutation.target.sealed.clone(), mutation.target.clone()))
            .collect(),
        ProductionAction::ProjectTest { .. }
        | ProductionAction::PendingRecovery { .. }
        | ProductionAction::LifecycleSessionEnd { .. }
        | ProductionAction::Update { .. }
        | ProductionAction::None => std::collections::BTreeMap::new(),
    }
}

fn target_journal_image(target: &AnchoredPath) -> Result<JournalImage, EffectError> {
    let parent = match anchored_parent(target, false) {
        Ok(parent) => parent,
        Err(error) if error.code == "transaction_parent_missing" => {
            return Ok(JournalImage::Absent);
        }
        Err(error) if error.code == "transaction_parent_binding_changed" => {
            match open_parent(&target.root, &target.relative, false) {
                Err(missing) if missing.code == "transaction_parent_missing" => {
                    return Ok(JournalImage::Absent);
                }
                _ => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    let name = target_name(target)?;
    let before = match rustix::fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(JournalImage::Absent),
        Err(error) => {
            return Err(effect_error("transaction_target_stat_failed", error, false));
        }
    };
    let file_type = FileType::from_raw_mode(before.st_mode);
    let mode = before.st_mode as u32 & 0o7777;
    let image = if file_type.is_file() {
        let bytes = read_at(&parent, name, "transaction_target_read_failed")?
            .ok_or_else(|| blocked("transaction_target_binding_changed", &target.sealed))?;
        JournalImage::RegularFile {
            sha256: sha256(&bytes),
            data_hex: encode_hex(&bytes),
            mode,
        }
    } else if file_type.is_dir() {
        let descriptor = rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| effect_error("transaction_target_open_failed", error, false))?;
        let opened = rustix::fs::fstat(&descriptor)
            .map_err(|error| effect_error("transaction_target_stat_failed", error, false))?;
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino)
            || !FileType::from_raw_mode(opened.st_mode).is_dir()
        {
            return Err(blocked(
                "transaction_target_binding_changed",
                &target.sealed,
            ));
        }
        JournalImage::Directory { mode }
    } else if file_type.is_symlink() {
        let target_bytes = rustix::fs::readlinkat(&parent, name, Vec::new())
            .map_err(|error| effect_error("transaction_symlink_read_failed", error, false))?;
        JournalImage::Symlink {
            target_hex: encode_hex(target_bytes.as_bytes()),
        }
    } else {
        return Err(blocked(
            "transaction_target_type_unsupported",
            &target.sealed,
        ));
    };
    let after = rustix::fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| effect_error("transaction_target_restat_failed", error, false))?;
    if (after.st_dev, after.st_ino, after.st_mode) != (before.st_dev, before.st_ino, before.st_mode)
    {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    Ok(image)
}

fn validate_journal_image(image: &JournalImage) -> Result<(), EffectError> {
    if let JournalImage::RegularFile {
        sha256: digest,
        data_hex,
        ..
    } = image
    {
        if sha256(decode_hex(data_hex)?) != *digest {
            return Err(blocked(
                "transaction_journal_preimage_digest_mismatch",
                digest,
            ));
        }
    }
    Ok(())
}

fn preimage_matches(expected: &JournalImage, target: &AnchoredPath) -> Result<bool, EffectError> {
    validate_journal_image(expected)?;
    Ok(target_journal_image(target)? == *expected)
}

fn postimage_matches(
    expected: &JournalPostimage,
    target: &AnchoredPath,
) -> Result<bool, EffectError> {
    let current = target_journal_image(target)?;
    Ok(match (expected, current) {
        (JournalPostimage::Absent, JournalImage::Absent) => true,
        (
            JournalPostimage::RegularFile {
                sha256: expected_sha,
                mode: expected_mode,
            },
            JournalImage::RegularFile { sha256, mode, .. },
        ) => sha256 == *expected_sha && mode == *expected_mode,
        (
            JournalPostimage::Directory {
                mode: expected_mode,
            },
            JournalImage::Directory { mode },
        ) => mode == *expected_mode,
        (
            JournalPostimage::Symlink {
                target_hex: expected_target,
            },
            JournalImage::Symlink { target_hex },
        ) => target_hex == *expected_target,
        _ => false,
    })
}

fn journal_postimage_matches(
    write: &JournalWrite,
    target: &AnchoredPath,
) -> Result<bool, EffectError> {
    if !postimage_matches(&write.postimage, target)? {
        return Ok(false);
    }
    match (&write.postimage, write.post_identity) {
        (
            JournalPostimage::RegularFile { .. }
            | JournalPostimage::Directory { .. }
            | JournalPostimage::Symlink { .. },
            Some(expected),
        ) => Ok(target_identity_now(target)? == Some(expected)),
        (
            JournalPostimage::RegularFile { .. }
            | JournalPostimage::Directory { .. }
            | JournalPostimage::Symlink { .. },
            None,
        ) => Ok(false),
        (JournalPostimage::Absent, _) => Ok(target_identity_now(target)?.is_none()),
    }
}

fn verify_commit_marker(
    marker_bytes: &[u8],
    journal: &TransactionJournal,
) -> Result<(), EffectError> {
    let marker: serde_json::Value = serde_json::from_slice(marker_bytes)
        .map_err(|error| blocked("transaction_commit_marker_invalid", error.to_string()))?;
    let valid = marker.get("action_ref").and_then(serde_json::Value::as_str)
        == Some(journal.action_ref.as_str())
        && marker
            .get("binding_hash")
            .and_then(serde_json::Value::as_str)
            == Some(journal.binding_hash.as_str())
        && marker.get("plan_hash").and_then(serde_json::Value::as_str)
            == Some(journal.plan_hash.as_str())
        && marker
            .get("journal_integrity")
            .and_then(serde_json::Value::as_str)
            == Some(journal.integrity.as_str());
    if valid {
        Ok(())
    } else {
        Err(blocked(
            "transaction_commit_marker_invalid",
            &journal.action_ref,
        ))
    }
}

fn restore_journal_write(target: &AnchoredPath, write: &JournalWrite) -> Result<(), EffectError> {
    match (&write.preimage, &write.postimage) {
        (JournalImage::Absent, JournalPostimage::RegularFile { .. }) => {
            let JournalImage::RegularFile { data_hex, .. } = target_journal_image(target)? else {
                return Err(blocked(
                    "transaction_recovery_postimage_changed",
                    &write.path,
                ));
            };
            anchored_delete(target, &decode_hex(&data_hex)?)
        }
        (JournalImage::Absent, JournalPostimage::Directory { .. }) => {
            remove_exact_directory(target, write.post_identity)
        }
        (JournalImage::Absent, JournalPostimage::Symlink { target_hex }) => {
            remove_exact_symlink(target, write.post_identity, &decode_hex(target_hex)?)
        }
        (JournalImage::RegularFile { data_hex, mode, .. }, JournalPostimage::Absent) => {
            anchored_write(target, None, &decode_hex(data_hex)?)?;
            set_regular_mode(target, *mode)
        }
        (
            JournalImage::RegularFile { data_hex, mode, .. },
            JournalPostimage::RegularFile { .. },
        ) => {
            let JournalImage::RegularFile {
                data_hex: current_hex,
                ..
            } = target_journal_image(target)?
            else {
                return Err(blocked(
                    "transaction_recovery_postimage_changed",
                    &write.path,
                ));
            };
            let current = decode_hex(&current_hex)?;
            anchored_write(target, Some(&current), &decode_hex(data_hex)?)?;
            set_regular_mode(target, *mode)
        }
        (JournalImage::Directory { mode }, JournalPostimage::Absent) => {
            create_exact_directory(target, *mode)
        }
        (JournalImage::Directory { mode }, JournalPostimage::Directory { .. }) => {
            set_directory_mode(target, *mode)
        }
        (JournalImage::Symlink { target_hex }, JournalPostimage::Absent) => {
            create_exact_symlink(target, &decode_hex(target_hex)?)
        }
        (
            JournalImage::Symlink {
                target_hex: previous_target,
            },
            JournalPostimage::Symlink {
                target_hex: current_target,
            },
        ) => {
            remove_exact_symlink(target, write.post_identity, &decode_hex(current_target)?)?;
            create_exact_symlink(target, &decode_hex(previous_target)?)
        }
        (JournalImage::Absent, JournalPostimage::Absent) => Err(blocked(
            "transaction_journal_transition_invalid",
            &write.path,
        )),
        (JournalImage::RegularFile { .. }, JournalPostimage::Directory { .. })
        | (JournalImage::RegularFile { .. }, JournalPostimage::Symlink { .. })
        | (JournalImage::Directory { .. }, JournalPostimage::RegularFile { .. })
        | (JournalImage::Directory { .. }, JournalPostimage::Symlink { .. })
        | (JournalImage::Symlink { .. }, JournalPostimage::RegularFile { .. })
        | (JournalImage::Symlink { .. }, JournalPostimage::Directory { .. }) => Err(blocked(
            "transaction_journal_type_transition_unsupported",
            &write.path,
        )),
    }
}

fn set_regular_mode(target: &AnchoredPath, mode: u32) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    let descriptor = rustix::fs::openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| effect_error("transaction_file_mode_open_failed", error, true))?;
    let stat = rustix::fs::fstat(&descriptor)
        .map_err(|error| effect_error("transaction_file_mode_stat_failed", error, true))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || Some((stat.st_dev as u64, stat.st_ino as u64)) != sealed_target_identity(target)?
    {
        return Err(effect_with_target(
            blocked("transaction_target_binding_changed", &target.sealed),
            target,
        ));
    }
    rustix::fs::fchmod(&descriptor, exact_mode(mode)?).map_err(|error| {
        effect_with_target(
            effect_error("transaction_file_mode_failed", error, true),
            target,
        )
    })?;
    rustix::fs::fsync(&descriptor).map_err(|error| {
        effect_with_target(
            effect_error("transaction_file_sync_failed", error, true),
            target,
        )
    })?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn set_directory_mode(target: &AnchoredPath, mode: u32) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let descriptor = rustix::fs::openat(
        &parent,
        target_name(target)?,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| effect_error("transaction_directory_mode_open_failed", error, true))?;
    let stat = rustix::fs::fstat(&descriptor)
        .map_err(|error| effect_error("transaction_directory_mode_stat_failed", error, true))?;
    if Some((stat.st_dev as u64, stat.st_ino as u64)) != sealed_target_identity(target)? {
        return Err(effect_with_target(
            blocked("transaction_target_binding_changed", &target.sealed),
            target,
        ));
    }
    rustix::fs::fchmod(&descriptor, exact_mode(mode)?).map_err(|error| {
        effect_with_target(
            effect_error("transaction_directory_mode_failed", error, true),
            target,
        )
    })?;
    rustix::fs::fsync(&descriptor).map_err(|error| {
        effect_with_target(
            effect_error("transaction_directory_sync_failed", error, true),
            target,
        )
    })?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn create_exact_directory(target: &AnchoredPath, mode: u32) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    if sealed_target_identity(target)?.is_some() || identity_at(&parent, name)?.is_some() {
        return Err(blocked(
            "transaction_create_directory_drift",
            &target.sealed,
        ));
    }
    rustix::fs::mkdirat(&parent, name, exact_mode(mode)?).map_err(|error| {
        effect_with_target(
            effect_error("transaction_create_directory_failed", error, true),
            target,
        )
    })?;
    update_target_identity(target, identity_at(&parent, name)?)?;
    set_directory_mode(target, mode)
}

fn exact_mode(mode: u32) -> Result<Mode, EffectError> {
    if mode & !0o7777 != 0 {
        return Err(blocked("transaction_mode_invalid", mode.to_string()));
    }
    Ok(Mode::from_raw_mode(mode as u16).into())
}

fn remove_exact_directory(
    target: &AnchoredPath,
    expected_identity: Option<(u64, u64)>,
) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    let descriptor = rustix::fs::openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| effect_error("transaction_remove_directory_open_failed", error, true))?;
    let stat = rustix::fs::fstat(&descriptor)
        .map_err(|error| effect_error("transaction_remove_directory_stat_failed", error, true))?;
    let identity = Some((stat.st_dev as u64, stat.st_ino as u64));
    if identity != expected_identity || identity != sealed_target_identity(target)? {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    for entry in rustix::fs::Dir::read_from(&descriptor)
        .map_err(|error| effect_error("transaction_remove_directory_read_failed", error, true))?
    {
        let entry = entry.map_err(|error| {
            effect_error("transaction_remove_directory_read_failed", error, true)
        })?;
        let name = entry.file_name().to_str().map_err(|error| {
            effect_error("transaction_remove_directory_name_failed", error, true)
        })?;
        if name != "." && name != ".." {
            return Err(blocked(
                "transaction_remove_directory_not_empty",
                &target.sealed,
            ));
        }
    }
    if identity_at(&parent, name)? != expected_identity {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    rustix::fs::unlinkat(&parent, name, AtFlags::REMOVEDIR).map_err(|error| {
        effect_with_target(
            effect_error("transaction_remove_directory_failed", error, true),
            target,
        )
    })?;
    update_target_identity(target, None)?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn create_exact_symlink(target: &AnchoredPath, link_target: &[u8]) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    if sealed_target_identity(target)?.is_some() || identity_at(&parent, name)?.is_some() {
        return Err(blocked("transaction_create_symlink_drift", &target.sealed));
    }
    let link_target = OsString::from_vec(link_target.to_vec());
    rustix::fs::symlinkat(&link_target, &parent, name).map_err(|error| {
        effect_with_target(
            effect_error("transaction_create_symlink_failed", error, true),
            target,
        )
    })?;
    update_target_identity(target, identity_at(&parent, name)?)?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn remove_exact_symlink(
    target: &AnchoredPath,
    expected_identity: Option<(u64, u64)>,
    expected_target: &[u8],
) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    if identity_at(&parent, name)? != expected_identity
        || expected_identity != sealed_target_identity(target)?
    {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    let current = rustix::fs::readlinkat(&parent, name, Vec::new())
        .map_err(|error| effect_error("transaction_symlink_read_failed", error, true))?;
    if current.as_bytes() != expected_target {
        return Err(blocked("transaction_symlink_target_drift", &target.sealed));
    }
    rustix::fs::unlinkat(&parent, name, AtFlags::empty()).map_err(|error| {
        effect_with_target(
            effect_error("transaction_remove_symlink_failed", error, true),
            target,
        )
    })?;
    update_target_identity(target, None)?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn apply_materialized_symlink(
    target: &AnchoredPath,
    previous: Option<&[u8]>,
    next: Option<&[u8]>,
) -> Result<(), EffectError> {
    match (previous, next) {
        (None, Some(next)) => create_exact_symlink(target, next),
        (Some(previous), None) => {
            remove_exact_symlink(target, sealed_target_identity(target)?, previous)
        }
        (Some(previous), Some(next)) if previous == next => {
            if !preimage_matches(
                &JournalImage::Symlink {
                    target_hex: encode_hex(previous),
                },
                target,
            )? {
                return Err(blocked("transaction_symlink_target_drift", &target.sealed));
            }
            Ok(())
        }
        (Some(previous), Some(next)) => {
            let parent = anchored_parent(target, false)?;
            let name = target_name(target)?;
            let identity = sealed_target_identity(target)?;
            if identity_at(&parent, name)? != identity {
                return Err(blocked(
                    "transaction_target_binding_changed",
                    &target.sealed,
                ));
            }
            let current = rustix::fs::readlinkat(&parent, name, Vec::new())
                .map_err(|error| effect_error("transaction_symlink_read_failed", error, true))?;
            if current.as_bytes() != previous {
                return Err(blocked("transaction_symlink_target_drift", &target.sealed));
            }
            let stage_name = format!(
                ".ags-symlink-stage-{}",
                &sha256(format!("{}\n{}", target.sealed, encode_hex(next)))[..24]
            );
            let stage = Path::new(&stage_name);
            let next = OsString::from_vec(next.to_vec());
            rustix::fs::symlinkat(&next, &parent, stage).map_err(|error| {
                effect_with_target(
                    effect_error("transaction_stage_symlink_failed", error, true),
                    target,
                )
            })?;
            if let Err(error) = rustix::fs::renameat(&parent, stage, &parent, name) {
                let _ = rustix::fs::unlinkat(&parent, stage, AtFlags::empty());
                return Err(effect_with_target(
                    effect_error("transaction_replace_symlink_failed", error, true),
                    target,
                ));
            }
            update_target_identity(target, identity_at(&parent, name)?)?;
            sync_parent(&parent).map_err(|error| effect_with_target(error, target))
        }
        (None, None) => Err(blocked(
            "skill_materialized_symlink_transition_invalid",
            &target.sealed,
        )),
    }
}

fn validate_relative_path(path: &Path) -> Result<(), EffectError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(blocked(
            "transaction_relative_path_invalid",
            path.display().to_string(),
        ));
    }
    Ok(())
}

fn open_parent(
    root: &RootCapability,
    relative: &Path,
    create: bool,
) -> Result<OwnedFd, EffectError> {
    validate_relative_path(relative)?;
    let mut current = rustix::io::dup(root.descriptor.as_ref())
        .map_err(|error| effect_error("transaction_root_dup_failed", error, false))?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(blocked(
                "transaction_relative_path_invalid",
                relative.display().to_string(),
            ));
        };
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let next = match rustix::fs::openat(&current, name, flags, Mode::empty()) {
            Ok(next) => next,
            Err(error) if error == rustix::io::Errno::NOENT && create => {
                match rustix::fs::mkdirat(&current, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                    Ok(()) => sync_parent(&current)?,
                    Err(error) if error == rustix::io::Errno::EXIST => {}
                    Err(error) => {
                        return Err(effect_error(
                            "transaction_parent_create_failed",
                            error,
                            true,
                        ));
                    }
                }
                rustix::fs::openat(&current, name, flags, Mode::empty())
                    .map_err(|error| effect_error("transaction_parent_open_failed", error, true))?
            }
            Err(error) if error == rustix::io::Errno::NOENT => {
                return Err(blocked(
                    "transaction_parent_missing",
                    relative.display().to_string(),
                ));
            }
            Err(error) => return Err(effect_error("transaction_parent_open_failed", error, false)),
        };
        current = next;
    }
    Ok(current)
}

fn target_parent_chain(target: &AnchoredPath) -> Result<Vec<JournalParentIdentity>, EffectError> {
    validate_relative_path(&target.relative)?;
    let mut current = rustix::io::dup(target.root.descriptor.as_ref())
        .map_err(|error| effect_error("transaction_root_dup_failed", error, false))?;
    let mut relative = PathBuf::new();
    let mut chain = Vec::new();
    for component in target
        .relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            return Err(blocked(
                "transaction_relative_path_invalid",
                target.relative.display().to_string(),
            ));
        };
        let next = rustix::fs::openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| effect_error("transaction_parent_chain_open_failed", error, false))?;
        let stat = rustix::fs::fstat(&next)
            .map_err(|error| effect_error("transaction_parent_chain_stat_failed", error, false))?;
        relative.push(name);
        chain.push(JournalParentIdentity {
            relative_path: relative.display().to_string(),
            device: stat.st_dev as u64,
            inode: stat.st_ino as u64,
        });
        current = next;
    }
    Ok(chain)
}

fn journal_apply_anchor_matches(
    write: &JournalWrite,
    target: &AnchoredPath,
) -> Result<bool, EffectError> {
    let JournalApplyAnchor::Applied { parent_chain } = &write.apply_anchor else {
        return Ok(false);
    };
    Ok(target_parent_chain(target)? == *parent_chain)
}

fn recovery_preimage_proof(write: &JournalWrite) -> Result<String, EffectError> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ags-transaction-recovery-preimage-v1",
        "path": write.path,
        "preimage": write.preimage,
    }))
    .map(sha256)
    .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))
}

fn restored_absent_directory_explains(
    journal: &TransactionJournal,
    write: &JournalWrite,
    relative_parent: &str,
) -> Result<bool, EffectError> {
    let expected_path = Path::new(&write.root_path).join(relative_parent);
    let Some(parent_write) = journal
        .ordered_writes
        .iter()
        .find(|candidate| Path::new(&candidate.path) == expected_path)
    else {
        return Ok(false);
    };
    let JournalWriteRecoveryProgress::Restored { preimage_proof } = &parent_write.recovery_progress
    else {
        return Ok(false);
    };
    Ok(matches!(parent_write.preimage, JournalImage::Absent)
        && matches!(parent_write.postimage, JournalPostimage::Directory { .. })
        && *preimage_proof == recovery_preimage_proof(parent_write)?)
}

fn restored_parent_chain_matches(
    journal: &TransactionJournal,
    write: &JournalWrite,
    target: &AnchoredPath,
) -> Result<bool, EffectError> {
    let JournalApplyAnchor::Applied { parent_chain } = &write.apply_anchor else {
        return Ok(false);
    };
    let components = target
        .relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .collect::<Vec<_>>();
    if components.len() != parent_chain.len() {
        return Ok(false);
    }
    let mut current = rustix::io::dup(target.root.descriptor.as_ref())
        .map_err(|error| effect_error("transaction_root_dup_failed", error, false))?;
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Ok(false);
        };
        let expected = &parent_chain[index];
        match rustix::fs::openat(
            &current,
            *name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(next) => {
                let stat = rustix::fs::fstat(&next).map_err(|error| {
                    effect_error("transaction_parent_chain_stat_failed", error, false)
                })?;
                if (stat.st_dev as u64, stat.st_ino as u64) != (expected.device, expected.inode) {
                    return Ok(false);
                }
                current = next;
            }
            Err(error) if error == rustix::io::Errno::NOENT => {
                for missing in &parent_chain[index..] {
                    if !restored_absent_directory_explains(journal, write, &missing.relative_path)?
                    {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum RecoveryStep {
    Restore,
    MarkRestored,
}

fn recovery_preflight(
    journal: &TransactionJournal,
    targets: &std::collections::BTreeMap<String, AnchoredPath>,
) -> Result<std::collections::BTreeMap<String, RecoveryStep>, EffectError> {
    if journal.identity_digest != journal.recompute_identity_digest()? {
        return Err(blocked(
            "transaction_journal_identity_digest_mismatch",
            &journal.action_ref,
        ));
    }
    if journal.phase == JournalPhase::Applied
        && journal
            .ordered_writes
            .iter()
            .any(|write| matches!(write.apply_anchor, JournalApplyAnchor::Pending))
    {
        return Err(blocked(
            "transaction_applied_anchor_incomplete",
            &journal.action_ref,
        ));
    }
    let has_restored = journal.ordered_writes.iter().any(|write| {
        matches!(
            write.recovery_progress,
            JournalWriteRecoveryProgress::Restored { .. }
        )
    });
    if has_restored != (journal.recovery_generation > 0) {
        return Err(blocked(
            "transaction_recovery_progress_invalid",
            &journal.action_ref,
        ));
    }
    let mut steps = std::collections::BTreeMap::new();
    for write in &journal.ordered_writes {
        let target = targets
            .get(&write.path)
            .ok_or_else(|| blocked("transaction_journal_write_set_mismatch", &write.path))?;
        validate_journal_root(write, target)?;
        match &write.recovery_progress {
            JournalWriteRecoveryProgress::Restored { preimage_proof } => {
                if *preimage_proof != recovery_preimage_proof(write)?
                    || !preimage_matches(&write.preimage, target)?
                    || !restored_parent_chain_matches(journal, write, target)?
                {
                    return Err(blocked("transaction_recovery_drift", &write.path));
                }
            }
            JournalWriteRecoveryProgress::Applied => {
                if preimage_matches(&write.preimage, target)? {
                    if matches!(write.apply_anchor, JournalApplyAnchor::Pending) {
                        continue;
                    }
                    if !journal_apply_anchor_matches(write, target)? {
                        return Err(blocked("transaction_recovery_drift", &write.path));
                    }
                    steps.insert(write.path.clone(), RecoveryStep::MarkRestored);
                } else {
                    if !journal_postimage_matches(write, target)?
                        || !journal_apply_anchor_matches(write, target)?
                    {
                        return Err(blocked("transaction_recovery_drift", &write.path));
                    }
                    steps.insert(write.path.clone(), RecoveryStep::Restore);
                }
            }
        }
    }
    Ok(steps)
}

fn persist_recovery_progress(
    journal_target: &AnchoredPath,
    previous: &mut Vec<u8>,
    journal: &mut TransactionJournal,
    index: usize,
) -> Result<(), EffectError> {
    let before = journal.clone();
    let frozen_identity = journal.identity_digest.clone();
    let proof = recovery_preimage_proof(&journal.ordered_writes[index])?;
    journal.ordered_writes[index].recovery_progress = JournalWriteRecoveryProgress::Restored {
        preimage_proof: proof,
    };
    journal.recovery_generation = journal.recovery_generation.checked_add(1).ok_or_else(|| {
        blocked(
            "transaction_recovery_generation_exhausted",
            &journal.action_ref,
        )
    })?;
    if journal.recompute_identity_digest()? != frozen_identity {
        *journal = before;
        return Err(blocked(
            "transaction_recovery_identity_changed",
            &journal.action_ref,
        ));
    }
    journal.reseal()?;
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| blocked("transaction_journal_encode_failed", error.to_string()))?;
    if let Err(error) = anchored_write(journal_target, Some(previous), &bytes) {
        *journal = before;
        return Err(error);
    }
    *previous = bytes;
    Ok(())
}

fn execute_exact_recovery(
    journal_target: &AnchoredPath,
    previous: &mut Vec<u8>,
    journal: &mut TransactionJournal,
    targets: &std::collections::BTreeMap<String, AnchoredPath>,
) -> Result<Vec<String>, EffectError> {
    let steps = recovery_preflight(journal, targets)?;
    let mut actual_writes = Vec::new();
    for index in (0..journal.ordered_writes.len()).rev() {
        let path = journal.ordered_writes[index].path.clone();
        let Some(step) = steps.get(&path).copied() else {
            continue;
        };
        let target = &targets[&path];
        if matches!(step, RecoveryStep::Restore) {
            let write = &journal.ordered_writes[index];
            if !journal_postimage_matches(write, target)?
                || !journal_apply_anchor_matches(write, target)?
            {
                let mut error = blocked("transaction_recovery_drift", &write.path);
                error.effect_started = !actual_writes.is_empty();
                error.observed_write_set = actual_writes;
                return Err(error);
            }
            if let Err(mut error) = restore_journal_write(target, write) {
                actual_writes.append(&mut error.observed_write_set);
                actual_writes.sort();
                actual_writes.dedup();
                error.effect_started |= !actual_writes.is_empty();
                error.observed_write_set = actual_writes;
                return Err(error);
            }
            actual_writes.push(path.clone());
        }
        if !preimage_matches(&journal.ordered_writes[index].preimage, target)? {
            let mut error = blocked(
                "transaction_recovery_postcondition_failed",
                &journal.ordered_writes[index].path,
            );
            error.effect_started = !actual_writes.is_empty();
            error.observed_write_set = actual_writes;
            return Err(error);
        }
        #[cfg(test)]
        if matches!(step, RecoveryStep::Restore)
            && PANIC_RECOVERY_AFTER_RESTORE_BEFORE_PROGRESS.with(|flag| flag.replace(false))
        {
            panic!("injected recovery crash after business restore before progress persistence");
        }
        if let Err(mut error) = persist_recovery_progress(journal_target, previous, journal, index)
        {
            error.effect_started = true;
            error.observed_write_set.extend(actual_writes);
            error.observed_write_set.sort();
            error.observed_write_set.dedup();
            return Err(error);
        }
    }
    for write in &journal.ordered_writes {
        if !preimage_matches(&write.preimage, &targets[&write.path])? {
            let mut error = blocked("transaction_recovery_postcondition_failed", &write.path);
            error.effect_started = !actual_writes.is_empty();
            error.observed_write_set = actual_writes;
            return Err(error);
        }
    }
    Ok(actual_writes)
}

fn parent_identity(parent: &OwnedFd) -> Result<(u64, u64), EffectError> {
    let stat = rustix::fs::fstat(parent)
        .map_err(|error| effect_error("transaction_parent_stat_failed", error, false))?;
    Ok((stat.st_dev as u64, stat.st_ino as u64))
}

fn anchored_parent(target: &AnchoredPath, create: bool) -> Result<OwnedFd, EffectError> {
    let current_root = rustix::fs::open(
        &target.root.canonical,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| effect_error("transaction_root_binding_changed", error, false))?;
    if parent_identity(&current_root)? != target.root.identity {
        return Err(blocked(
            "transaction_root_binding_changed",
            target.root.canonical.display().to_string(),
        ));
    }
    let mut anchor = target
        .parent_anchor
        .lock()
        .map_err(|_| blocked("transaction_parent_anchor_poisoned", &target.sealed))?;
    if let (Some(held), Some(identity)) = (&anchor.descriptor, anchor.identity) {
        let current = open_parent(&target.root, &target.relative, false).map_err(|error| {
            blocked(
                "transaction_parent_binding_changed",
                format!("{}: {}", target.sealed, error.detail),
            )
        })?;
        if parent_identity(&current)? != identity {
            return Err(blocked(
                "transaction_parent_binding_changed",
                &target.sealed,
            ));
        }
        return rustix::io::dup(held.as_ref())
            .map_err(|error| effect_error("transaction_parent_dup_failed", error, false));
    }

    let parent = open_parent(&target.root, &target.relative, create)?;
    let identity = parent_identity(&parent)?;
    anchor.descriptor =
        Some(Arc::new(rustix::io::dup(&parent).map_err(|error| {
            effect_error("transaction_parent_dup_failed", error, false)
        })?));
    anchor.identity = Some(identity);
    Ok(parent)
}

fn target_name(target: &AnchoredPath) -> Result<&OsStr, EffectError> {
    target
        .relative
        .file_name()
        .ok_or_else(|| blocked("transaction_target_name_invalid", &target.sealed))
}

fn sibling_target(target: &AnchoredPath, name: &str) -> Result<AnchoredPath, EffectError> {
    let name = Path::new(name);
    validate_relative_path(name)?;
    if name.components().count() != 1 {
        return Err(blocked(
            "transaction_target_name_invalid",
            name.display().to_string(),
        ));
    }
    let parent = target.relative.parent().unwrap_or_else(|| Path::new(""));
    let relative = parent.join(name);
    let descriptor = anchored_parent(target, false)?;
    let target_identity = identity_at(
        &descriptor,
        relative.file_name().ok_or_else(|| {
            blocked(
                "transaction_target_name_invalid",
                name.display().to_string(),
            )
        })?,
    )?;
    Ok(AnchoredPath {
        root: target.root.clone(),
        sealed: target.root.canonical.join(&relative).display().to_string(),
        relative,
        parent_anchor: Arc::clone(&target.parent_anchor),
        target_identity: Arc::new(Mutex::new(target_identity)),
    })
}

fn read_at(parent: &OwnedFd, name: &OsStr, code: &str) -> Result<Option<Vec<u8>>, EffectError> {
    let fd = match rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(effect_error(code, error, false)),
    };
    let stat = rustix::fs::fstat(&fd).map_err(|error| effect_error(code, error, false))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(blocked(
            "transaction_target_not_regular",
            name.to_string_lossy(),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::from(fd)
        .take(MAX_TRANSACTION_FILE_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| effect_error(code, error, false))?;
    if bytes.len() > MAX_TRANSACTION_FILE_BYTES {
        return Err(blocked(
            "transaction_target_too_large",
            name.to_string_lossy(),
        ));
    }
    Ok(Some(bytes))
}

fn identity_at(parent: &OwnedFd, name: &OsStr) -> Result<Option<(u64, u64)>, EffectError> {
    let stat = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => {
            return Err(effect_error(
                "transaction_target_identity_failed",
                error,
                false,
            ));
        }
    };
    Ok(Some((stat.st_dev as u64, stat.st_ino as u64)))
}

fn sealed_target_identity(target: &AnchoredPath) -> Result<Option<(u64, u64)>, EffectError> {
    target
        .target_identity
        .lock()
        .map(|identity| *identity)
        .map_err(|_| blocked("transaction_target_identity_poisoned", &target.sealed))
}

fn update_target_identity(
    target: &AnchoredPath,
    identity: Option<(u64, u64)>,
) -> Result<(), EffectError> {
    *target
        .target_identity
        .lock()
        .map_err(|_| blocked("transaction_target_identity_poisoned", &target.sealed))? = identity;
    Ok(())
}

fn target_identity_now(target: &AnchoredPath) -> Result<Option<(u64, u64)>, EffectError> {
    let parent = match anchored_parent(target, false) {
        Ok(parent) => parent,
        Err(error) if error.code == "transaction_parent_missing" => return Ok(None),
        Err(error) => return Err(error),
    };
    identity_at(&parent, target_name(target)?)
}

fn anchored_read(
    target: &AnchoredPath,
    create_parent: bool,
) -> Result<Option<Vec<u8>>, EffectError> {
    let parent = match anchored_parent(target, create_parent) {
        Ok(parent) => parent,
        Err(error) if error.code == "transaction_parent_missing" && !create_parent => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    read_at(
        &parent,
        target_name(target)?,
        "transaction_target_read_failed",
    )
}

fn create_staged_file(
    parent: &OwnedFd,
    content: &[u8],
    sealed_parent: &Path,
) -> Result<(OsString, OwnedFd), EffectError> {
    for attempt in 0_u8..16 {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|error| effect_error("transaction_temp_entropy_failed", error, false))?;
        let name = OsString::from(format!(
            ".ags-txn-{}-{attempt}.tmp",
            ags_platform::sha256(random)
                .strip_prefix("sha256:")
                .unwrap_or("random")
        ));
        match rustix::fs::openat(
            parent,
            &name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => {
                let mut file = fs::File::from(fd);
                let injected = {
                    #[cfg(test)]
                    {
                        FAIL_NEXT_STAGE_WRITE.with(|flag| flag.replace(false))
                    }
                    #[cfg(not(test))]
                    {
                        false
                    }
                };
                let write_result = if injected {
                    Err(std::io::Error::other("injected stage write failure"))
                } else {
                    file.write_all(content).and_then(|_| file.sync_all())
                };
                if let Err(error) = write_result {
                    let mut error = effect_error("transaction_stage_write_failed", error, true);
                    error.observed_write_set =
                        vec![sealed_parent.join(&name).display().to_string()];
                    return Err(error);
                }
                return Ok((name, file.into()));
            }
            Err(error) if error == rustix::io::Errno::EXIST => continue,
            Err(error) => return Err(effect_error("transaction_stage_create_failed", error, true)),
        }
    }
    Err(blocked(
        "transaction_stage_collision",
        "could not allocate an exclusive same-directory staging file",
    ))
}

fn file_identity(fd: &OwnedFd, code: &str) -> Result<(u64, u64), EffectError> {
    let stat = rustix::fs::fstat(fd).map_err(|error| effect_error(code, error, false))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(blocked(code, "staged descriptor is not a regular file"));
    }
    Ok((stat.st_dev as u64, stat.st_ino as u64))
}

fn file_digest(fd: &OwnedFd, code: &str) -> Result<String, EffectError> {
    let duplicate = rustix::io::dup(fd).map_err(|error| effect_error(code, error, false))?;
    let mut file = fs::File::from(duplicate);
    file.rewind()
        .map_err(|error| effect_error(code, error, false))?;
    let mut bytes = Vec::new();
    file.take(MAX_TRANSACTION_FILE_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| effect_error(code, error, false))?;
    if bytes.len() > MAX_TRANSACTION_FILE_BYTES {
        return Err(blocked(code, "staged file exceeds transaction byte budget"));
    }
    Ok(sha256(bytes))
}

fn effect_with_residue(mut error: EffectError, sealed_parent: &Path, name: &OsStr) -> EffectError {
    error.effect_started = true;
    error
        .observed_write_set
        .push(sealed_parent.join(name).display().to_string());
    error.observed_write_set.sort();
    error.observed_write_set.dedup();
    error
}

fn effect_with_target(mut error: EffectError, target: &AnchoredPath) -> EffectError {
    error.effect_started = true;
    error.observed_write_set.push(target.sealed.clone());
    error.observed_write_set.sort();
    error.observed_write_set.dedup();
    error
}

fn named_file_matches(
    parent: &OwnedFd,
    name: &OsStr,
    expected_identity: (u64, u64),
    expected_content: &[u8],
) -> Result<bool, EffectError> {
    if identity_at(parent, name)? != Some(expected_identity) {
        return Ok(false);
    }
    Ok(
        read_at(parent, name, "transaction_stage_verify_failed")?.as_deref()
            == Some(expected_content),
    )
}

fn unlink_owned_named_file(
    parent: &OwnedFd,
    name: &OsStr,
    expected_identity: (u64, u64),
    expected_content: &[u8],
    sealed_parent: &Path,
    code: &str,
) -> Result<(), EffectError> {
    let initial_match = named_file_matches(parent, name, expected_identity, expected_content)
        .map_err(|error| effect_with_residue(error, sealed_parent, name))?;
    if !initial_match {
        return Err(effect_with_residue(
            blocked(
                "transaction_cleanup_binding_changed",
                name.to_string_lossy(),
            ),
            sealed_parent,
            name,
        ));
    }
    let mut selected = None;
    for attempt in 0_u8..16 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| {
            effect_with_residue(
                effect_error("transaction_quarantine_entropy_failed", error, true),
                sealed_parent,
                name,
            )
        })?;
        let quarantine = OsString::from(format!(
            ".ags-quarantine-{}-{attempt}",
            ags_platform::sha256(random)
                .strip_prefix("sha256:")
                .unwrap_or("random")
        ));
        match rustix::fs::renameat_with(
            parent,
            name,
            parent,
            &quarantine,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                selected = Some(quarantine);
                break;
            }
            Err(error) if error == rustix::io::Errno::EXIST => continue,
            Err(error) => {
                return Err(effect_with_residue(
                    effect_error(code, error, true),
                    sealed_parent,
                    name,
                ));
            }
        }
    }
    let quarantine = selected.ok_or_else(|| {
        effect_with_residue(
            blocked("transaction_quarantine_collision", name.to_string_lossy()),
            sealed_parent,
            name,
        )
    })?;
    #[cfg(test)]
    if FAIL_NEXT_QUARANTINE_OPEN.with(|flag| flag.replace(false)) {
        return Err(effect_with_residue(
            effect_error(
                "transaction_quarantine_open_failed",
                "injected quarantine open failure",
                true,
            ),
            sealed_parent,
            &quarantine,
        ));
    }
    let quarantine_match =
        named_file_matches(parent, &quarantine, expected_identity, expected_content)
            .map_err(|error| effect_with_residue(error, sealed_parent, &quarantine))?;
    if !quarantine_match {
        rustix::fs::renameat_with(
            parent,
            &quarantine,
            parent,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            effect_with_residue(
                effect_error("transaction_quarantine_rollback_failed", error, true),
                sealed_parent,
                &quarantine,
            )
        })?;
        return Err(effect_with_residue(
            blocked(
                "transaction_cleanup_binding_changed",
                name.to_string_lossy(),
            ),
            sealed_parent,
            name,
        ));
    }
    // The unpredictable quarantine basename minimizes the final name-based
    // unlink window on platforms without unlink-by-fd. Revalidate immediately
    // before removal and restore rather than deleting an unknown replacement.
    let quarantine_match =
        named_file_matches(parent, &quarantine, expected_identity, expected_content)
            .map_err(|error| effect_with_residue(error, sealed_parent, &quarantine))?;
    if !quarantine_match {
        rustix::fs::renameat_with(
            parent,
            &quarantine,
            parent,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            effect_with_residue(
                effect_error("transaction_quarantine_rollback_failed", error, true),
                sealed_parent,
                &quarantine,
            )
        })?;
        return Err(effect_with_residue(
            blocked(
                "transaction_cleanup_binding_changed",
                name.to_string_lossy(),
            ),
            sealed_parent,
            name,
        ));
    }
    rustix::fs::unlinkat(parent, &quarantine, AtFlags::empty()).map_err(|error| {
        effect_with_residue(effect_error(code, error, true), sealed_parent, &quarantine)
    })
}

#[cfg(test)]
fn inject_stage_substitution(parent: &OwnedFd, stage: &OsStr, content: &[u8]) {
    let same = SUBSTITUTE_NEXT_STAGE.with(|flag| flag.replace(false));
    let different = SUBSTITUTE_NEXT_STAGE_DIFFERENT.with(|flag| flag.replace(false));
    if !same && !different {
        return;
    }
    let original = OsStr::new(".ags-test-original-stage");
    rustix::fs::renameat_with(
        parent,
        stage,
        parent,
        original,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .unwrap();
    let replacement = rustix::fs::openat(
        parent,
        stage,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .unwrap();
    let mut replacement = fs::File::from(replacement);
    replacement
        .write_all(if different {
            b"attacker-different-stage"
        } else {
            content
        })
        .unwrap();
    replacement.sync_all().unwrap();
}

fn anchored_write(
    target: &AnchoredPath,
    expected: Option<&[u8]>,
    content: &[u8],
) -> Result<(), EffectError> {
    let parent = anchored_parent(target, true)?;
    let name = target_name(target)?;
    let sealed_identity = sealed_target_identity(target)?;
    if identity_at(&parent, name)? != sealed_identity {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    let current = read_at(&parent, name, "transaction_write_read_failed")?;
    if current.as_deref() != expected {
        return Err(blocked("transaction_write_drift", &target.sealed));
    }
    let sealed_parent = Path::new(&target.sealed)
        .parent()
        .ok_or_else(|| blocked("transaction_target_parent_invalid", &target.sealed))?;
    let (stage, stage_fd) = create_staged_file(&parent, content, sealed_parent)?;
    let stage_identity = file_identity(&stage_fd, "transaction_stage_stat_failed")?;
    if file_digest(&stage_fd, "transaction_stage_digest_failed")? != sha256(content) {
        return Err(effect_with_residue(
            blocked("transaction_stage_fd_digest_changed", &target.sealed),
            sealed_parent,
            &stage,
        ));
    }
    #[cfg(test)]
    inject_stage_substitution(&parent, &stage, content);
    let stage_matches = named_file_matches(&parent, &stage, stage_identity, content)
        .map_err(|error| effect_with_residue(error, sealed_parent, &stage))?;
    if !stage_matches {
        return Err(effect_with_residue(
            blocked("transaction_stage_binding_changed", &target.sealed),
            sealed_parent,
            &stage,
        ));
    }
    let commit = if expected.is_some() {
        rustix::fs::renameat_with(
            &parent,
            &stage,
            &parent,
            name,
            rustix::fs::RenameFlags::EXCHANGE,
        )
    } else {
        rustix::fs::renameat_with(
            &parent,
            &stage,
            &parent,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
    };
    if let Err(error) = commit {
        unlink_owned_named_file(
            &parent,
            &stage,
            stage_identity,
            content,
            sealed_parent,
            "transaction_stage_cleanup_failed",
        )?;
        return Err(effect_error("transaction_write_commit_failed", error, true));
    }
    if let Some(expected) = expected {
        let exchanged = read_at(&parent, &stage, "transaction_exchange_read_failed")?;
        let replacement = read_at(&parent, name, "transaction_exchange_read_failed")?;
        let exchanged_identity = identity_at(&parent, &stage)?;
        let replacement_identity = identity_at(&parent, name)?;
        if exchanged.as_deref() != Some(expected)
            || exchanged_identity != sealed_identity
            || replacement.as_deref() != Some(content)
            || replacement_identity != Some(stage_identity)
        {
            let rollback = rustix::fs::renameat_with(
                &parent,
                &stage,
                &parent,
                name,
                rustix::fs::RenameFlags::EXCHANGE,
            );
            return match rollback {
                Ok(()) => match unlink_owned_named_file(
                    &parent,
                    &stage,
                    stage_identity,
                    content,
                    sealed_parent,
                    "transaction_stage_cleanup_failed",
                ) {
                    Ok(()) => Err(blocked("transaction_write_substitution", &target.sealed)),
                    Err(error) => Err(error),
                },
                Err(error) => {
                    let error = effect_with_residue(
                        effect_error("transaction_write_rollback_failed", error, true),
                        sealed_parent,
                        &stage,
                    );
                    Err(effect_with_residue(error, sealed_parent, name))
                }
            };
        }
        unlink_owned_named_file(
            &parent,
            &stage,
            sealed_identity.expect("replace target identity was sealed as present"),
            expected,
            sealed_parent,
            "transaction_exchange_cleanup_failed",
        )
        .map_err(|error| effect_with_target(error, target))?;
    } else if identity_at(&parent, name)? != Some(stage_identity)
        || read_at(&parent, name, "transaction_create_verify_failed")?.as_deref() != Some(content)
    {
        rustix::fs::renameat_with(
            &parent,
            name,
            &parent,
            &stage,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            let error = effect_with_residue(
                effect_error("transaction_write_rollback_failed", error, true),
                sealed_parent,
                &stage,
            );
            effect_with_residue(error, sealed_parent, name)
        })?;
        return Err(effect_with_residue(
            blocked("transaction_write_substitution", &target.sealed),
            sealed_parent,
            &stage,
        ));
    }
    update_target_identity(target, identity_at(&parent, name)?)?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn anchored_delete(target: &AnchoredPath, expected: &[u8]) -> Result<(), EffectError> {
    let parent = anchored_parent(target, false)?;
    let name = target_name(target)?;
    let sealed_identity = sealed_target_identity(target)?;
    if identity_at(&parent, name)? != sealed_identity {
        return Err(blocked(
            "transaction_target_binding_changed",
            &target.sealed,
        ));
    }
    if read_at(&parent, name, "transaction_delete_read_failed")?.as_deref() != Some(expected) {
        return Err(blocked("transaction_delete_drift", &target.sealed));
    }
    let sealed_parent = Path::new(&target.sealed)
        .parent()
        .ok_or_else(|| blocked("transaction_target_parent_invalid", &target.sealed))?;
    let (stage, stage_fd) = create_staged_file(&parent, &[], sealed_parent)?;
    let stage_identity = file_identity(&stage_fd, "transaction_stage_stat_failed")?;
    if file_digest(&stage_fd, "transaction_stage_digest_failed")? != sha256([]) {
        return Err(effect_with_residue(
            blocked("transaction_stage_fd_digest_changed", &target.sealed),
            sealed_parent,
            &stage,
        ));
    }
    #[cfg(test)]
    inject_stage_substitution(&parent, &stage, &[]);
    let stage_matches = named_file_matches(&parent, &stage, stage_identity, &[])
        .map_err(|error| effect_with_residue(error, sealed_parent, &stage))?;
    if !stage_matches {
        return Err(effect_with_residue(
            blocked("transaction_stage_binding_changed", &target.sealed),
            sealed_parent,
            &stage,
        ));
    }
    if let Err(error) = rustix::fs::renameat_with(
        &parent,
        &stage,
        &parent,
        name,
        rustix::fs::RenameFlags::EXCHANGE,
    ) {
        unlink_owned_named_file(
            &parent,
            &stage,
            stage_identity,
            &[],
            sealed_parent,
            "transaction_stage_cleanup_failed",
        )?;
        return Err(effect_error(
            "transaction_delete_commit_failed",
            error,
            true,
        ));
    }
    let exchanged = read_at(&parent, &stage, "transaction_delete_exchange_read_failed")?;
    let exchanged_identity = identity_at(&parent, &stage)?;
    let replacement = read_at(&parent, name, "transaction_delete_exchange_read_failed")?;
    let replacement_identity = identity_at(&parent, name)?;
    if exchanged.as_deref() != Some(expected)
        || exchanged_identity != sealed_identity
        || replacement.as_deref() != Some(&[])
        || replacement_identity != Some(stage_identity)
    {
        let rollback = rustix::fs::renameat_with(
            &parent,
            &stage,
            &parent,
            name,
            rustix::fs::RenameFlags::EXCHANGE,
        );
        return match rollback {
            Ok(()) => match unlink_owned_named_file(
                &parent,
                &stage,
                stage_identity,
                &[],
                sealed_parent,
                "transaction_stage_cleanup_failed",
            ) {
                Ok(()) => Err(blocked("transaction_delete_substitution", &target.sealed)),
                Err(error) => Err(error),
            },
            Err(error) => {
                let error = effect_with_residue(
                    effect_error("transaction_delete_rollback_failed", error, true),
                    sealed_parent,
                    &stage,
                );
                Err(effect_with_residue(error, sealed_parent, name))
            }
        };
    }
    unlink_owned_named_file(
        &parent,
        name,
        stage_identity,
        &[],
        sealed_parent,
        "transaction_delete_cleanup_failed",
    )
    .map_err(|error| effect_with_target(error, target))?;
    update_target_identity(target, None)?;
    unlink_owned_named_file(
        &parent,
        &stage,
        sealed_identity.expect("delete target identity was sealed as present"),
        expected,
        sealed_parent,
        "transaction_delete_cleanup_failed",
    )
    .map_err(|error| effect_with_target(error, target))?;
    sync_parent(&parent).map_err(|error| effect_with_target(error, target))
}

fn sync_parent(parent: &OwnedFd) -> Result<(), EffectError> {
    #[cfg(test)]
    if FAIL_NEXT_DIRECTORY_SYNC.with(|flag| flag.replace(false)) {
        return Err(EffectError {
            code: "transaction_parent_sync_failed".to_string(),
            detail: "injected directory fsync failure".to_string(),
            effect_started: true,
            output_digest: sha256("injected directory fsync failure"),
            // The generic directory-sync primitive does not own a sealed
            // pathname, so it cannot truthfully attribute the effect. The
            // caller that holds the anchored target adds the exact path.
            observed_write_set: Vec::new(),
        });
    }
    rustix::fs::fsync(parent)
        .map_err(|error| effect_error("transaction_parent_sync_failed", error, true))
}

#[cfg(test)]
fn apply_files(writes: &[PlannedWrite]) -> Result<EffectObservation, EffectError> {
    let mut observed = Vec::new();
    for write in writes {
        if let Err(mut error) =
            anchored_write(&write.target, write.previous.as_deref(), &write.content)
        {
            let mut effects = observed;
            effects.append(&mut error.observed_write_set);
            effects.sort();
            effects.dedup();
            error.observed_write_set = effects;
            error.effect_started |= !error.observed_write_set.is_empty();
            return Err(error);
        }
        observed.push(write.target.sealed.clone());
    }
    EffectObservation::bounded(
        true,
        !observed.is_empty(),
        sha256(observed.join("\n")),
        observed,
        None,
    )
}

fn apply_files_journaled(
    adapter: &ProductionEffectAdapter,
    action_ref: &str,
    plan: &SealedPlan,
    writes: &[PlannedWrite],
) -> Result<EffectObservation, EffectError> {
    let mut observed = Vec::new();
    for write in writes {
        if let Err(mut error) =
            anchored_write(&write.target, write.previous.as_deref(), &write.content)
        {
            let mut effects = observed;
            effects.append(&mut error.observed_write_set);
            effects.sort();
            effects.dedup();
            error.observed_write_set = effects;
            error.effect_started |= !error.observed_write_set.is_empty();
            return Err(error);
        }
        observed.push(write.target.sealed.clone());
        if let Err(mut error) = adapter.record_journal_post_identity(
            action_ref,
            plan,
            &write.target.sealed,
            &write.target,
        ) {
            error.effect_started = true;
            error.observed_write_set.extend(observed.iter().cloned());
            error.observed_write_set.sort();
            error.observed_write_set.dedup();
            return Err(error);
        }
    }
    EffectObservation::bounded(
        true,
        !observed.is_empty(),
        sha256(observed.join("\n")),
        observed,
        None,
    )
}

fn apply_deletes_journaled(
    adapter: &ProductionEffectAdapter,
    action_ref: &str,
    plan: &SealedPlan,
    deletes: &[PlannedDelete],
) -> Result<EffectObservation, EffectError> {
    let mut observed = Vec::new();
    for delete in deletes {
        if let Err(mut error) = anchored_delete(&delete.target, &delete.previous) {
            let mut effects = observed;
            effects.append(&mut error.observed_write_set);
            effects.sort();
            effects.dedup();
            error.observed_write_set = effects;
            error.effect_started |= !error.observed_write_set.is_empty();
            return Err(error);
        }
        observed.push(delete.target.sealed.clone());
        if let Err(mut error) = adapter.record_journal_post_identity(
            action_ref,
            plan,
            &delete.target.sealed,
            &delete.target,
        ) {
            error.effect_started = true;
            error.observed_write_set.extend(observed.iter().cloned());
            error.observed_write_set.sort();
            error.observed_write_set.dedup();
            return Err(error);
        }
    }
    EffectObservation::bounded(
        true,
        !observed.is_empty(),
        sha256(observed.join("\n")),
        observed,
        None,
    )
}

fn apply_projections(
    adapter: &ProductionEffectAdapter,
    action_ref: &str,
    plan: &SealedPlan,
    projections: &[PlannedProjection],
) -> Result<EffectObservation, EffectError> {
    let mut observed = Vec::new();
    let mut evidence = Vec::new();
    for projection in projections {
        let mut applied = Vec::new();
        for mutation in &projection.mutations {
            if !preimage_matches(&mutation.preimage, &mutation.target)? {
                let mut error = blocked("projection_apply_preimage_drift", &mutation.target.sealed);
                error.effect_started = !observed.is_empty();
                error.observed_write_set = observed;
                return Err(error);
            }
            let result = match &mutation.apply {
                ObjectMutationApply::CreateDirectory => {
                    let JournalPostimage::Directory { mode } = mutation.postimage else {
                        return Err(blocked(
                            "projection_materialized_action_invalid",
                            &mutation.target.sealed,
                        ));
                    };
                    create_exact_directory(&mutation.target, mode)
                }
                ObjectMutationApply::WriteFile { previous, next } => {
                    let JournalPostimage::RegularFile { mode, .. } = mutation.postimage else {
                        return Err(blocked(
                            "projection_materialized_action_invalid",
                            &mutation.target.sealed,
                        ));
                    };
                    anchored_write(&mutation.target, previous.as_deref(), next)
                        .and_then(|()| set_regular_mode(&mutation.target, mode))
                }
                ObjectMutationApply::DeleteFile { previous } => {
                    anchored_delete(&mutation.target, previous)
                }
                ObjectMutationApply::SetSymlink { previous, next } => apply_materialized_symlink(
                    &mutation.target,
                    previous.as_deref(),
                    next.as_deref(),
                ),
            };
            if let Err(mut error) = result {
                observed.append(&mut error.observed_write_set);
                observed.sort();
                observed.dedup();
                error.effect_started |= !observed.is_empty();
                error.observed_write_set = observed;
                return Err(error);
            }
            observed.push(mutation.target.sealed.clone());
            #[cfg(test)]
            {
                let kind = match &mutation.apply {
                    ObjectMutationApply::CreateDirectory => "directory",
                    ObjectMutationApply::WriteFile { .. } => "regular_file",
                    ObjectMutationApply::DeleteFile { .. } => "delete_file",
                    ObjectMutationApply::SetSymlink { .. } => "symlink",
                };
                let should_panic = PANIC_PROJECTION_BEFORE_IDENTITY_KIND.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .is_some_and(|expected| *expected == kind)
                });
                if should_panic {
                    PANIC_PROJECTION_BEFORE_IDENTITY_KIND.with(|slot| {
                        slot.borrow_mut().take();
                    });
                    panic!("injected projection crash before durable post identity for {kind}");
                }
            }
            if let Err(mut error) = adapter.record_journal_post_identity(
                action_ref,
                plan,
                &mutation.target.sealed,
                &mutation.target,
            ) {
                error.effect_started = true;
                error.observed_write_set.extend(observed.iter().cloned());
                error.observed_write_set.sort();
                error.observed_write_set.dedup();
                return Err(error);
            }
            #[cfg(test)]
            {
                let should_panic = PANIC_PROJECTION_AFTER_OPERATION.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .is_some_and(|expected| *expected == mutation.operation)
                });
                if should_panic {
                    PANIC_PROJECTION_AFTER_OPERATION.with(|slot| {
                        slot.borrow_mut().take();
                    });
                    panic!(
                        "injected projection crash after durable {} mutation",
                        mutation.operation
                    );
                }
            }
            applied.push(serde_json::json!({
                "path": mutation.target.sealed,
                "operation": mutation.operation,
                "postimage": mutation.postimage,
            }));
            #[cfg(test)]
            if observed.len() == 1
                && DRIFT_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.replace(false))
            {
                assert!(matches!(
                    mutation.postimage,
                    JournalPostimage::Directory { .. }
                ));
                fs::write(
                    Path::new(&mutation.target.sealed).join("third-party.txt"),
                    b"third-party-drift",
                )
                .unwrap();
                panic!("injected projection crash after third-party drift");
            }
            #[cfg(test)]
            if observed.len() == 1
                && PANIC_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.replace(false))
            {
                panic!("injected projection crash after first durable object mutation");
            }
            #[cfg(test)]
            if observed.len() == 1
                && FAIL_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.replace(false))
            {
                return Err(EffectError {
                    code: "projection_injected_partial_failure".to_string(),
                    detail: "injected after first real projection mutation".to_string(),
                    effect_started: false,
                    output_digest: sha256("projection-injected-partial-failure"),
                    observed_write_set: observed,
                });
            }
        }
        evidence.push(serde_json::json!({
            "schema_version": "ags://schema/contract/v2/project-projection-evidence",
            "workspace": projection.workspace,
            "planned_directories": projection.planned_directory_paths,
            "mutations": applied,
        }));
    }
    EffectObservation::bounded(
        true,
        !observed.is_empty(),
        sha256(
            serde_json::to_vec(&evidence)
                .map_err(|error| blocked("projection_receipt_encode_failed", error.to_string()))?,
        ),
        observed,
        Some(serde_json::Value::Array(evidence)),
    )
}

fn apply_materialized_skill_change(
    adapter: &ProductionEffectAdapter,
    action_ref: &str,
    plan: &SealedPlan,
    _context: &ags_capability_governance::skill_adoption::AdoptionContext,
    materialized: &ags_capability_governance::skill_adoption::MaterializedSkillChange,
    mutations: &[PlannedObjectMutation],
) -> Result<EffectObservation, EffectError> {
    let mut observed = Vec::new();
    for (_phase, operation) in [
        ("parent", "skill_parent"),
        ("body", "skill_body"),
        ("index", "skill_index"),
        ("link", "skill_link"),
        ("snapshot", "skill_snapshot"),
    ] {
        for mutation in mutations
            .iter()
            .filter(|mutation| mutation.operation == operation)
        {
            if !preimage_matches(&mutation.preimage, &mutation.target)? {
                let mut error =
                    blocked("skill_materialized_preimage_drift", &mutation.target.sealed);
                error.effect_started = !observed.is_empty();
                error.observed_write_set = observed;
                return Err(error);
            }
            let result = match &mutation.apply {
                ObjectMutationApply::CreateDirectory => {
                    let JournalPostimage::Directory { mode } = mutation.postimage else {
                        return Err(blocked(
                            "skill_materialized_action_invalid",
                            &mutation.target.sealed,
                        ));
                    };
                    create_exact_directory(&mutation.target, mode)
                }
                ObjectMutationApply::WriteFile { previous, next } => {
                    let JournalPostimage::RegularFile { mode, .. } = mutation.postimage else {
                        return Err(blocked(
                            "skill_materialized_action_invalid",
                            &mutation.target.sealed,
                        ));
                    };
                    anchored_write(&mutation.target, previous.as_deref(), next)
                        .and_then(|()| set_regular_mode(&mutation.target, mode))
                }
                ObjectMutationApply::DeleteFile { previous } => {
                    anchored_delete(&mutation.target, previous)
                }
                ObjectMutationApply::SetSymlink { previous, next } => apply_materialized_symlink(
                    &mutation.target,
                    previous.as_deref(),
                    next.as_deref(),
                ),
            };
            if let Err(mut error) = result {
                observed.append(&mut error.observed_write_set);
                observed.sort();
                observed.dedup();
                error.effect_started |= !observed.is_empty();
                error.observed_write_set = observed;
                return Err(error);
            }
            observed.push(mutation.target.sealed.clone());
            if let Err(mut error) = adapter.record_journal_post_identity(
                action_ref,
                plan,
                &mutation.target.sealed,
                &mutation.target,
            ) {
                error.effect_started = true;
                error.observed_write_set.extend(observed.iter().cloned());
                error.observed_write_set.sort();
                error.observed_write_set.dedup();
                return Err(error);
            }
        }
        #[cfg(test)]
        {
            if DRIFT_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|expected| *expected == _phase)
            }) {
                DRIFT_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                    slot.borrow_mut().take();
                });
                let target = mutations
                    .iter()
                    .find(|mutation| mutation.operation == operation)
                    .expect("drift hook requires a mutation in its phase");
                fs::write(&target.target.sealed, b"third-party-after-skill-mutation").unwrap();
                let mut error = blocked("skill_injected_third_party_drift", _phase);
                error.effect_started = true;
                error.observed_write_set = observed;
                return Err(error);
            }
            if PANIC_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|expected| *expected == _phase)
            }) {
                PANIC_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                    slot.borrow_mut().take();
                });
                panic!("injected Skill crash after {_phase} phase");
            }
            if FAIL_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|expected| *expected == _phase)
            }) {
                FAIL_SKILL_AFTER_MUTATION_KIND.with(|slot| {
                    slot.borrow_mut().take();
                });
                let mut error = blocked("skill_injected_partial_failure", _phase);
                error.effect_started = !observed.is_empty();
                error.observed_write_set = observed;
                return Err(error);
            }
        }
    }
    observed.sort();
    observed.dedup();
    let route_verification = materialized_route_verification(materialized);
    let evidence = serde_json::json!({
        "schema_version": "ags://schema/contract/v2/materialized-skill-change-evidence",
        "materialization_hash": materialized.materialization_hash,
        "operation": materialized.operation,
        "skill_id": materialized.skill_id,
        "registry_revision": materialized.registry_revision,
        "route_verification": route_verification,
    });
    EffectObservation::bounded(
        route_verification
            .get("passed")
            .and_then(serde_json::Value::as_bool)
            == Some(true),
        !observed.is_empty(),
        sha256(
            serde_json::to_vec(&evidence).map_err(|error| {
                blocked("skill_change_receipt_encode_failed", error.to_string())
            })?,
        ),
        observed,
        Some(evidence),
    )
}

fn materialized_route_verification(
    materialized: &ags_capability_governance::skill_adoption::MaterializedSkillChange,
) -> serde_json::Value {
    let registry = serde_json::from_slice::<serde_json::Value>(&materialized.registry.post_bytes);
    let registered = registry.as_ref().is_ok_and(|registry| {
        registry
            .get("skills")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|skills| skills.contains_key(&materialized.skill_id))
    });
    let snapshot_routes = materialized
        .snapshots
        .iter()
        .map(|snapshot| {
            let parsed = serde_json::from_slice::<serde_json::Value>(&snapshot.file.post_bytes);
            let active = parsed.as_ref().is_ok_and(|value| {
                value
                    .get("active_skills")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|skills| {
                        skills.iter().any(|skill| {
                            skill.get("skill_id").and_then(serde_json::Value::as_str)
                                == Some(materialized.skill_id.as_str())
                        })
                    })
            });
            serde_json::json!({
                "host": snapshot.host,
                "snapshot_hash": snapshot.snapshot_hash,
                "active": active,
            })
        })
        .collect::<Vec<_>>();
    let snapshots_match = snapshot_routes.iter().all(|route| {
        route.get("active").and_then(serde_json::Value::as_bool)
            == Some(materialized.operation != "remove")
    });
    let links_match = materialized
        .links
        .iter()
        .all(|link| link.post_target.is_some() == (materialized.operation != "remove"));
    let passed =
        registered == (materialized.operation != "remove") && snapshots_match && links_match;
    serde_json::json!({
        "passed": passed,
        "registered": registered,
        "links_match": links_match,
        "snapshots": &snapshot_routes,
        "status": {
            "registered": registered,
            "installation": {
                "skill_id": materialized.skill_id,
                "registered": registered,
            },
            "activations": snapshot_routes,
        },
    })
}

fn blocked(code: impl Into<String>, detail: impl Into<String>) -> EffectError {
    EffectError {
        code: code.into(),
        detail: detail.into(),
        effect_started: false,
        output_digest: String::new(),
        observed_write_set: Vec::new(),
    }
}

fn effect_error(
    code: impl Into<String>,
    error: impl std::fmt::Display,
    started: bool,
) -> EffectError {
    let detail = error.to_string();
    EffectError {
        code: code.into(),
        output_digest: sha256(&detail),
        detail,
        effect_started: started,
        observed_write_set: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn flat_directory_bytes(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                let bytes = if entry.file_type().unwrap().is_file() {
                    fs::read(entry.path()).unwrap()
                } else {
                    Vec::new()
                };
                (name, bytes)
            })
            .collect()
    }

    fn exact_tree(root: &Path) -> std::collections::BTreeMap<String, (String, u32, Vec<u8>)> {
        fn visit(
            root: &Path,
            path: &Path,
            output: &mut std::collections::BTreeMap<String, (String, u32, Vec<u8>)>,
        ) {
            let metadata = fs::symlink_metadata(path).unwrap();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mode = metadata.permissions().mode() & 0o7777;
            if metadata.file_type().is_symlink() {
                output.insert(
                    relative,
                    (
                        "symlink".to_string(),
                        mode,
                        fs::read_link(path)
                            .unwrap()
                            .as_os_str()
                            .as_encoded_bytes()
                            .to_vec(),
                    ),
                );
            } else if metadata.is_dir() {
                if !relative.is_empty() {
                    output.insert(relative, ("directory".to_string(), mode, Vec::new()));
                }
                let mut children = fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                for child in children {
                    visit(root, &child, output);
                }
            } else {
                output.insert(
                    relative,
                    ("file".to_string(), mode, fs::read(path).unwrap()),
                );
            }
        }
        let mut output = std::collections::BTreeMap::new();
        if root.exists() {
            visit(root, root, &mut output);
        }
        output
    }

    fn managed_tree_without_control_journals(
        root: &Path,
    ) -> std::collections::BTreeMap<String, (String, u32, Vec<u8>)> {
        exact_tree(root)
            .into_iter()
            .filter(|(path, _)| !path.contains(".ags-transaction-"))
            .collect()
    }

    fn prepare_exact_owned_projection_workspace(root: &Path) -> (Vec<u8>, Vec<u8>) {
        let old_agents = b"old exact-owned AGENTS\n".to_vec();
        let obsolete = b"exact-owned obsolete bytes\n".to_vec();
        fs::create_dir_all(root.join(".ags/evidence")).unwrap();
        fs::create_dir_all(root.join(".ags/state/closure-pointers")).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("AGENTS.md"), &old_agents).unwrap();
        fs::write(root.join("obsolete.txt"), &obsolete).unwrap();
        fs::write(root.join(".ags/evidence/.keep"), []).unwrap();
        fs::write(root.join(".ags/state/closure-pointers/.keep"), []).unwrap();
        fs::write(root.join("config/agent-project-profile.yaml"), PROFILE).unwrap();
        fs::set_permissions(root.join("AGENTS.md"), fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(root.join("obsolete.txt"), fs::Permissions::from_mode(0o750)).unwrap();
        let ownership = serde_json::json!({
            "schema_version": "ags://schema/contract/v2/project-ownership",
            "entries": {
                ".ags/evidence/.keep": {"last_applied_sha256": sha256([])},
                ".ags/state/closure-pointers/.keep": {"last_applied_sha256": sha256([])},
                "AGENTS.md": {"last_applied_sha256": sha256(&old_agents)},
                "config/agent-project-profile.yaml": {"last_applied_sha256": sha256(PROFILE.as_bytes())},
                "obsolete.txt": {"last_applied_sha256": sha256(&obsolete)}
            }
        });
        let mut ownership_bytes = serde_json::to_vec_pretty(&ownership).unwrap();
        ownership_bytes.push(b'\n');
        fs::write(root.join(".ags/ownership-v2.json"), ownership_bytes).unwrap();
        fs::set_permissions(
            root.join(".ags/ownership-v2.json"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        (old_agents, obsolete)
    }

    fn skill_install_fixture() -> (
        tempfile::TempDir,
        PathBuf,
        AuthenticatedBinding,
        ProductionEffectAdapter,
        SkillInstallRequest,
    ) {
        let authority = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        let repository = temp.path().join("fixture-source");
        let source = repository.join("skill");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: control-plane-fixture\ndescription: Control-plane route fixture.\n---\n\n# Fixture\n",
        )
        .unwrap();
        let routing = temp.path().join("routing.yaml");
        fs::write(
            &routing,
            "summary: Verify the control-plane canonical Skill route.\nintent_tags: [control-plane-fixture]\npositive_examples: [Use the control-plane fixture]\nnegative_examples: [Do unrelated work]\n",
        )
        .unwrap();
        let binding = AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &authority,
            "workspace-a",
            sha256("skill-midpoint-facts"),
            "registry-a",
            "session-a",
            vec![authority.clone(), temp.path().canonicalize().unwrap()],
        );
        let adapter = ProductionEffectAdapter::new(&runtime);
        seed_canonical_skill_host(&authority, &runtime, temp.path());
        let mut request = SkillInstallRequest {
            context: OperationContext::default(),
            skill_id: "control-plane-fixture".to_string(),
            source: SkillSourceSpec {
                kind: SkillSourceKind::Local,
                uri: source.display().to_string(),
                requested_ref: None,
                tracking_ref: None,
                subdir: None,
            },
            routing_metadata: Some(routing.display().to_string()),
            target_hosts: vec!["codex".to_string()],
            update_policy: SkillUpdatePolicy::Notify,
            risk_acknowledgements: Vec::new(),
        };
        let prepared = ags_capability_governance::skill_adoption::plan_install(
            &adapter.skill_adoption_context(&binding),
            &canonical_skill_source(&request.source),
            request.routing_metadata.as_deref().map(Path::new),
            &request.target_hosts,
            ags_capability_governance::skill_adoption::UpdatePolicy::Notify,
        )
        .unwrap();
        request.risk_acknowledgements = prepared
            .risk_findings
            .iter()
            .map(|finding| finding.acknowledgement_id())
            .collect();
        (temp, runtime, binding, adapter, request)
    }

    fn seed_canonical_skill_host(authority: &Path, runtime: &Path, host_home: &Path) {
        let host = ags_host_integration::HostId::new("codex").unwrap();
        let registration = ags_host_integration::HostRegistration::new(
            host,
            ags_host_integration::AgentSurface::Hybrid,
            Some("codex".to_string()),
        );
        let registration_path = runtime.join("hosts/codex/registration.json");
        fs::create_dir_all(registration_path.parent().unwrap()).unwrap();
        fs::write(
            &registration_path,
            serde_json::to_vec_pretty(&registration).unwrap(),
        )
        .unwrap();
        ags_capability_governance::write_capability_snapshot_with_roots(
            authority, "codex", runtime, host_home,
        )
        .unwrap();
    }

    #[test]
    fn production_skill_path_has_no_legacy_apply_or_autonomous_recovery_calls() {
        let source = [
            include_str!("production.rs"),
            include_str!("../../../ags-capability-governance/src/skill_adoption/mod.rs"),
            include_str!("../../../ags-capability-governance/src/skill_adoption/model.rs"),
            include_str!("../../../ags-capability-governance/src/skill_adoption/transaction.rs"),
        ]
        .concat();
        for forbidden in [
            ["skill_adoption::", "apply_install("].concat(),
            ["skill_adoption::", "apply_removal("].concat(),
            ["skill_adoption::", "recover_pending_transactions("].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "production must not call legacy Skill mutation authority: {forbidden}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn materialized_skill_action_rejects_tamper_before_outer_journal() {
        let (temp, _runtime, binding, adapter, request) = skill_install_fixture();
        let baseline = managed_tree_without_control_journals(temp.path());
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-tamper-daemon"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-tamper-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap()
            .action_ref
            .unwrap();
        let action = &mut plane.actions.get_mut(&action_ref).unwrap().domain_action;
        let ProductionAction::SkillChange { materialized, .. } = action else {
            panic!("Skill install must retain one materialized action")
        };
        materialized.registry.post_bytes.push(b' ');
        let error = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "sealed_action_mismatch");
        assert!(error.detail.contains("stored domain action"));
        assert_eq!(managed_tree_without_control_journals(temp.path()), baseline);
    }

    #[cfg(unix)]
    #[test]
    fn materialized_skill_derived_mutation_tamper_is_rejected_before_outer_journal() {
        for tamper in [
            "regular_bytes",
            "regular_mode",
            "symlink_target",
            "apply_variant",
        ] {
            let (temp, _runtime, binding, adapter, request) = skill_install_fixture();
            let baseline = managed_tree_without_control_journals(temp.path());
            let mut plane = ControlPlane::with_sealing_key(
                adapter,
                sha256(format!("skill-derived-tamper-{tamper}")),
            );
            let session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("skill-derived-tamper-policy-{tamper}")),
                })
                .unwrap();
            let action_ref = plane
                .decide(&session, OperationRequest::GovernSkillInstall(request))
                .unwrap()
                .action_ref
                .unwrap();
            let ProductionAction::SkillChange { mutations, .. } =
                &mut plane.actions.get_mut(&action_ref).unwrap().domain_action
            else {
                panic!("Skill install must retain derived mutations")
            };
            match tamper {
                "regular_bytes" => {
                    let mutation = mutations
                        .iter_mut()
                        .find(|mutation| {
                            matches!(mutation.apply, ObjectMutationApply::WriteFile { .. })
                        })
                        .unwrap();
                    let ObjectMutationApply::WriteFile { next, .. } = &mut mutation.apply else {
                        unreachable!()
                    };
                    next.push(b'!');
                    let JournalPostimage::RegularFile { sha256: digest, .. } =
                        &mut mutation.postimage
                    else {
                        unreachable!()
                    };
                    *digest = sha256(next);
                }
                "regular_mode" => {
                    let mutation = mutations
                        .iter_mut()
                        .find(|mutation| {
                            matches!(mutation.apply, ObjectMutationApply::WriteFile { .. })
                        })
                        .unwrap();
                    let JournalPostimage::RegularFile { mode, .. } = &mut mutation.postimage else {
                        unreachable!()
                    };
                    *mode ^= 0o111;
                }
                "symlink_target" => {
                    let mutation = mutations
                        .iter_mut()
                        .find(|mutation| {
                            matches!(mutation.apply, ObjectMutationApply::SetSymlink { .. })
                        })
                        .unwrap();
                    let next = b"../tampered-skill-body".to_vec();
                    mutation.postimage = JournalPostimage::Symlink {
                        target_hex: encode_hex(&next),
                    };
                    let ObjectMutationApply::SetSymlink {
                        next: apply_next, ..
                    } = &mut mutation.apply
                    else {
                        unreachable!()
                    };
                    *apply_next = Some(next);
                }
                "apply_variant" => {
                    let mutation = mutations
                        .iter_mut()
                        .find(|mutation| {
                            mutation.preimage == JournalImage::Absent
                                && matches!(mutation.apply, ObjectMutationApply::WriteFile { .. })
                        })
                        .unwrap();
                    let next = b"variant-tamper".to_vec();
                    mutation.postimage = JournalPostimage::Symlink {
                        target_hex: encode_hex(&next),
                    };
                    mutation.apply = ObjectMutationApply::SetSymlink {
                        previous: None,
                        next: Some(next),
                    };
                }
                _ => unreachable!(),
            }
            let error = plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref,
                        outcome: None,
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "sealed_action_mismatch", "tamper={tamper}");
            assert!(
                error.detail.contains("stored domain action"),
                "tamper={tamper}: {error:?}"
            );
            assert_eq!(managed_tree_without_control_journals(temp.path()), baseline);
        }
    }

    #[test]
    fn transaction_semantic_tamper_is_rejected_before_outer_journal() {
        let cases = ["files", "projection_bytes", "projection_variant"];
        for case in cases {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path().canonicalize().unwrap();
            let runtime = tempfile::tempdir().unwrap();
            let runtime_root = runtime.path().canonicalize().unwrap();
            let binding =
                binding_with_runtime(&root, &runtime_root, sha256(format!("{case}-facts")));
            let mut plane = ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(&runtime_root),
                sha256(format!("{case}-daemon")),
            );
            let session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("{case}-policy")),
                })
                .unwrap();
            let operation = if case == "files" {
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                })
            } else {
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                })
            };
            let action_ref = plane
                .decide(&session, operation)
                .unwrap()
                .action_ref
                .unwrap();
            let before_workspace = exact_tree(&root);
            let before_runtime = exact_tree(&runtime_root);
            match &mut plane.actions.get_mut(&action_ref).unwrap().domain_action {
                ProductionAction::Files(files) if case == "files" => files[0].content.push(b'!'),
                ProductionAction::Projections(projections) if case == "projection_bytes" => {
                    let mutation = projections[0]
                        .mutations
                        .iter_mut()
                        .find(|mutation| {
                            matches!(mutation.apply, ObjectMutationApply::WriteFile { .. })
                        })
                        .unwrap();
                    let ObjectMutationApply::WriteFile { next, .. } = &mut mutation.apply else {
                        unreachable!()
                    };
                    next.push(b'!');
                    let JournalPostimage::RegularFile { sha256: digest, .. } =
                        &mut mutation.postimage
                    else {
                        unreachable!()
                    };
                    *digest = sha256(next);
                }
                ProductionAction::Projections(projections) if case == "projection_variant" => {
                    let mutation = projections[0]
                        .mutations
                        .iter_mut()
                        .find(|mutation| mutation.preimage == JournalImage::Absent)
                        .unwrap();
                    let next = b"projection-variant".to_vec();
                    mutation.postimage = JournalPostimage::Symlink {
                        target_hex: encode_hex(&next),
                    };
                    mutation.apply = ObjectMutationApply::SetSymlink {
                        previous: None,
                        next: Some(next),
                    };
                }
                action => panic!("unexpected action for {case}: {action:?}"),
            }
            let error = plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref,
                        outcome: None,
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "sealed_action_mismatch", "case={case}");
            assert_eq!(exact_tree(&root), before_workspace, "case={case}");
            assert_eq!(exact_tree(&runtime_root), before_runtime, "case={case}");
        }
    }

    #[test]
    fn stored_plan_and_project_test_action_tamper_are_rejected_pre_effect() {
        for tamper in ["execution", "project_test_action"] {
            let workspace = tempfile::tempdir().unwrap();
            fs::create_dir_all(workspace.path().join("config")).unwrap();
            fs::write(
                workspace.path().join("config/agent-project-profile.yaml"),
                PROFILE,
            )
            .unwrap();
            let root = workspace.path().canonicalize().unwrap();
            let runtime = tempfile::tempdir().unwrap();
            let binding = binding(&root);
            let mut plane = ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(runtime.path()),
                sha256(format!("{tamper}-daemon")),
            );
            let session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("{tamper}-policy")),
                })
                .unwrap();
            let action_ref = plane
                .decide(
                    &session,
                    OperationRequest::Test(TestRequest {
                        context: OperationContext::default(),
                        profile: TestProfile::Smoke,
                        executor: TestExecutor::Host,
                    }),
                )
                .unwrap()
                .action_ref
                .unwrap();
            let record = plane.actions.get_mut(&action_ref).unwrap();
            if tamper == "execution" {
                record
                    .plan
                    .execution
                    .as_mut()
                    .unwrap()
                    .program
                    .push_str("-tampered");
            } else {
                let mut spec = record.plan.execution.clone().unwrap();
                spec.program.push_str("-tampered");
                record.domain_action = ProductionAction::ProjectTest {
                    workspace: root.clone(),
                    profile: ags_verification::TestProfile::Smoke,
                    spec,
                };
            }
            let error = plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref,
                        outcome: None,
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "sealed_action_mismatch", "tamper={tamper}");
        }
    }

    #[test]
    fn sealed_plan_payload_and_host_actions_are_revalidated_before_grant_or_effect() {
        for tamper in [
            "verification",
            "payload",
            "update",
            "update_source",
            "lifecycle_variant",
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path().canonicalize().unwrap();
            let runtime = tempfile::tempdir().unwrap();
            let runtime_root = runtime.path().canonicalize().unwrap();
            let binding =
                binding_with_runtime(&root, &runtime_root, sha256(format!("{tamper}-facts")));
            let operation = if matches!(tamper, "update" | "update_source" | "lifecycle_variant") {
                let candidate = runtime_root.join("update-candidates/0.4.20");
                fs::create_dir_all(&candidate).unwrap();
                for name in RELEASE_PAYLOAD_NAMES {
                    fs::write(candidate.join(name), name.as_bytes()).unwrap();
                }
                OperationRequest::Update(UpdateRequest {
                    context: OperationContext::default(),
                    channel: "stable".to_string(),
                    target_version: Some("0.4.20".to_string()),
                })
            } else {
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                })
            };
            let mut plane = ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(&runtime_root),
                sha256(format!("{tamper}-daemon")),
            );
            let session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("{tamper}-policy")),
                })
                .unwrap();
            let action_ref = plane
                .decide(&session, operation)
                .unwrap()
                .action_ref
                .unwrap();
            let before_workspace = exact_tree(&root);
            let before_runtime = exact_tree(&runtime_root);
            let record = plane.actions.get_mut(&action_ref).unwrap();
            match tamper {
                "verification" => record
                    .plan
                    .verification
                    .checks
                    .push("tampered-verification".to_string()),
                "payload" => {
                    let ActionOperation::External(operation) = &mut record.operation else {
                        unreachable!()
                    };
                    let OperationRequest::Setup(request) = operation.as_mut() else {
                        unreachable!()
                    };
                    request.approved_hosts.push("tampered".to_string());
                }
                "update" => {
                    let ProductionAction::Update { tree_digest, .. } = &mut record.domain_action
                    else {
                        unreachable!()
                    };
                    tree_digest.push_str("-tampered");
                }
                "update_source" => {
                    let ProductionAction::Update {
                        candidate_directory,
                        ..
                    } = &mut record.domain_action
                    else {
                        unreachable!()
                    };
                    *candidate_directory = runtime_root.join("update-candidates/other");
                }
                "lifecycle_variant" => {
                    record.domain_action = ProductionAction::LifecycleSessionEnd {
                        receipt_ids: vec!["tampered-receipt".to_string()],
                        pointer_paths: vec![root
                            .join("tampered-pointer.json")
                            .display()
                            .to_string()],
                    };
                }
                _ => unreachable!(),
            }
            let error = plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref,
                        outcome: None,
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "sealed_action_mismatch", "tamper={tamper}");
            assert_eq!(exact_tree(&root), before_workspace, "tamper={tamper}");
            assert_eq!(exact_tree(&runtime_root), before_runtime, "tamper={tamper}");
        }
    }

    #[test]
    fn pending_recovery_domain_identity_tamper_is_rejected_prewrite() {
        let (_workspace, runtime, root, binding, _original_action_ref) = crashed_init();
        let runtime_before = exact_tree(runtime.path());
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("pending-domain-tamper-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("pending-domain-tamper-policy"),
            })
            .unwrap();
        let action_ref = session.pending_recovery_action_ref().unwrap().to_string();
        let ProductionAction::PendingRecovery {
            journal_state_digest,
            ..
        } = &mut plane.actions.get_mut(&action_ref).unwrap().domain_action
        else {
            unreachable!()
        };
        journal_state_digest.push_str("-tampered");
        let error = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "sealed_action_mismatch");
        assert_eq!(exact_tree(runtime.path()), runtime_before);
        assert!(root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn materialized_skill_action_is_cross_binding_and_replay_safe() {
        let (temp, _runtime, binding, adapter, request) = skill_install_fixture();
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-replay-daemon"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-replay-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap()
            .action_ref
            .unwrap();
        let cross_binding = AuthenticatedBinding::mcp(
            "connection-b",
            "hermes",
            binding.canonical_workspace().to_path_buf(),
            "workspace-a",
            sha256("skill-midpoint-facts"),
            "registry-a",
            "session-b",
            vec![
                binding.canonical_workspace().to_path_buf(),
                temp.path().canonicalize().unwrap(),
            ],
        );
        plane
            .open(OpenRequest {
                binding: cross_binding.clone(),
                policy_hash: sha256("skill-cross-policy"),
            })
            .unwrap();
        assert_eq!(
            plane
                .apply(
                    &cross_binding,
                    ApplyRequest {
                        action_ref: action_ref.clone(),
                        outcome: None,
                    },
                )
                .unwrap_err()
                .code,
            "action_ref_cross_binding"
        );
        assert_eq!(
            plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: action_ref.clone(),
                        outcome: None,
                    },
                )
                .unwrap()
                .receipt
                .unwrap()
                .status,
            ReceiptStatus::Succeeded
        );
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

    #[cfg(unix)]
    #[test]
    fn materialized_skill_preimage_bytes_drift_is_preserved_fail_closed() {
        let (_temp, _runtime, binding, adapter, request) = skill_install_fixture();
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-byte-drift-daemon"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-byte-drift-policy"),
            })
            .unwrap();
        let decision = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap();
        let registry = decision
            .plan
            .as_ref()
            .unwrap()
            .expected_write_paths
            .iter()
            .find(|path| path.ends_with("installed-skills.json"))
            .cloned()
            .unwrap();
        fs::create_dir_all(Path::new(&registry).parent().unwrap()).unwrap();
        fs::write(&registry, b"third-party-between-materialize-and-apply").unwrap();
        let error = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: decision.action_ref.unwrap(),
                    outcome: None,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "sealed_action_mismatch");
        assert!(error.detail.contains("skill_materialized_preimage_drift"));
        assert_eq!(
            fs::read(registry).unwrap(),
            b"third-party-between-materialize-and-apply"
        );
    }

    fn binding(root: &Path) -> AuthenticatedBinding {
        binding_with_facts(root, sha256("facts-a"))
    }

    fn binding_with_facts(root: &Path, project_facts_hash: String) -> AuthenticatedBinding {
        AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            root,
            "workspace-a",
            project_facts_hash,
            "registry-a",
            "session-a",
            vec![root.to_path_buf()],
        )
    }

    fn binding_with_runtime(
        root: &Path,
        runtime: &Path,
        project_facts_hash: String,
    ) -> AuthenticatedBinding {
        AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            root,
            "workspace-a",
            project_facts_hash,
            "registry-a",
            "session-a",
            vec![root.to_path_buf(), runtime.to_path_buf()],
        )
    }

    #[test]
    fn runtime_authority_root_permissions_are_fd_validated_without_mutation() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        fs::write(runtime.path().join("sentinel"), b"unchanged").unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("runtime-mode-facts"));

        for mode in [0o770, 0o707, 0o777] {
            fs::set_permissions(runtime.path(), fs::Permissions::from_mode(mode)).unwrap();
            let before = flat_directory_bytes(runtime.path());
            let mut plane = ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(runtime.path()),
                sha256(format!("runtime-mode-{mode:o}")),
            );
            let error = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256("runtime-mode-policy"),
                })
                .unwrap_err();
            assert_eq!(error.code, "pending_transaction_inspection_failed");
            assert!(error
                .detail
                .contains("runtime_authority_permissions_invalid"));
            assert_eq!(flat_directory_bytes(runtime.path()), before);
            assert_eq!(
                fs::metadata(runtime.path()).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        for mode in [0o755, 0o700] {
            fs::set_permissions(runtime.path(), fs::Permissions::from_mode(mode)).unwrap();
            let mut plane = ControlPlane::with_sealing_key(
                ProductionEffectAdapter::new(runtime.path()),
                sha256(format!("runtime-safe-mode-{mode:o}")),
            );
            plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256("runtime-safe-mode-policy"),
                })
                .unwrap();
        }
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn runtime_authority_rejects_foreign_owner_and_setup_or_key_reads_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        fs::write(runtime.path().join(CLOSURE_AUTHORITY_KEY_FILE), [7_u8; 32]).unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("runtime-owner-facts"));
        let before = flat_directory_bytes(runtime.path());
        let foreign_uid = rustix::process::geteuid().as_raw().wrapping_add(1);
        RUNTIME_AUTHORITY_UID_OVERRIDE.with(|override_uid| override_uid.set(Some(foreign_uid)));
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let key_error = adapter.closure_authority_key(&binding).unwrap_err();
        assert_eq!(key_error.code, "runtime_authority_permissions_invalid");
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("runtime-owner-daemon"));
        let open_error = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("runtime-owner-policy"),
            })
            .unwrap_err();
        RUNTIME_AUTHORITY_UID_OVERRIDE.with(|override_uid| override_uid.set(None));
        assert_eq!(open_error.code, "pending_transaction_inspection_failed");
        assert!(open_error
            .detail
            .contains("runtime_authority_permissions_invalid"));
        assert_eq!(flat_directory_bytes(runtime.path()), before);

        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("runtime-owner-policy-2"),
            })
            .unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let before_plan = flat_directory_bytes(runtime.path());
        let error = plane
            .decide(
                &opened,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap_err();
        assert!(error
            .detail
            .contains("runtime_authority_permissions_invalid"));
        assert_eq!(flat_directory_bytes(runtime.path()), before_plan);
        assert_eq!(
            fs::metadata(runtime.path()).unwrap().permissions().mode() & 0o777,
            0o777
        );
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn workspace_journal_lookalike_cannot_enter_runtime_authority() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join(".ags-transaction-forged.json"),
            br#"{"self_consistent":"workspace-only"}"#,
        )
        .unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("workspace-lookalike-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("workspace-lookalike-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("workspace-lookalike-policy"),
            })
            .unwrap();
        assert!(opened.pending_recovery_action_ref().is_none());
        assert_eq!(opened.terminal_recovery_count, 0);
    }

    fn apply_without_commit(
        plane: &mut ControlPlane<ProductionEffectAdapter>,
        binding: &AuthenticatedBinding,
    ) -> String {
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
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
        let record = plane.actions.get(&action_ref).unwrap().clone();
        plane
            .adapter
            .prepare_journal(&action_ref, &record.plan, &record.domain_action, binding)
            .unwrap();
        plane
            .adapter
            .transition_journal(&action_ref, &record.plan, JournalPhase::Applying, None)
            .unwrap();
        let observation = match &record.domain_action {
            ProductionAction::Files(writes) => apply_files(writes).unwrap(),
            other => panic!("expected setup files, got {other:?}"),
        };
        plane
            .adapter
            .transition_journal_applied(
                &action_ref,
                &record.plan,
                &record.domain_action,
                &observation.output_digest,
            )
            .unwrap();
        action_ref
    }

    fn crashed_init() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        PathBuf,
        AuthenticatedBinding,
        String,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let before_binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-before-profile"),
        );
        let mut first = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("first-daemon"),
        );
        let action_ref = apply_without_commit(&mut first, &before_binding);
        drop(first);
        (workspace, runtime, root, before_binding, action_ref)
    }

    #[test]
    fn red_open_only_inspects_pending_journal_and_preserves_every_byte() {
        let (_workspace, runtime, root, _before_binding, action_ref) = crashed_init();
        let target = runtime.path().join("install-manifest.json");
        let journal = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        let target_before = fs::read(&target).unwrap();
        let journal_before = fs::read(&journal).unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-after-crash"),
        );
        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("restarted-daemon"),
        );

        let opened = restarted
            .open(OpenRequest {
                binding,
                policy_hash: sha256("restarted-policy"),
            })
            .unwrap();

        assert_eq!(fs::read(&target).unwrap(), target_before);
        assert_eq!(fs::read(&journal).unwrap(), journal_before);
        let opened = serde_json::to_value(opened).unwrap();
        assert_eq!(opened["pending_recovery_required"], true);
        assert!(opened["pending_recovery_action_ref"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn pending_recovery_ref_is_binding_bound_replay_safe_and_restart_rotated() {
        let (_workspace, runtime, root, _before_binding, _original_action_ref) = crashed_init();
        let target = runtime.path().join("install-manifest.json");
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-after-crash"),
        );
        let mut first = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("recovery-daemon-a"),
        );
        let opened = first
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("recovery-policy-a"),
            })
            .unwrap();
        let old_ref = opened.pending_recovery_action_ref().unwrap().to_string();
        let wrong = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("different-binding-facts"),
        );
        assert_eq!(
            first
                .apply(
                    &wrong,
                    ApplyRequest {
                        action_ref: old_ref.clone(),
                        outcome: None,
                    },
                )
                .unwrap_err()
                .code,
            "action_ref_cross_binding"
        );

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("recovery-daemon-b"),
        );
        let reopened = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("recovery-policy-b"),
            })
            .unwrap();
        let new_ref = reopened.pending_recovery_action_ref().unwrap().to_string();
        assert_ne!(old_ref, new_ref);
        assert_eq!(
            restarted
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: old_ref,
                        outcome: None,
                    },
                )
                .unwrap_err()
                .code,
            "action_ref_invalid"
        );
        let result = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: new_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert!(!target.exists());
        assert_eq!(
            restarted
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: new_ref,
                        outcome: None,
                    },
                )
                .unwrap_err()
                .code,
            "action_ref_invalid"
        );
    }

    #[test]
    fn durable_recovery_terminal_is_retryable_exact_and_requires_reopen() {
        let (_workspace, runtime, root, _before_binding, action_ref) = crashed_init();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let manifest = runtime.path().join("install-manifest.json");
        let authority = runtime.path().join(CLOSURE_AUTHORITY_KEY_FILE);
        let journal_path = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        fs::remove_file(&authority).unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("recovery-retry-facts"));
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("recovery-retry-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("recovery-retry-policy"),
            })
            .unwrap();
        let recovery_ref = opened.pending_recovery_action_ref().unwrap().to_string();
        FAIL_NEXT_RECOVERY_FINALIZE.with(|flag| flag.set(true));
        let failed = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_ref,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(failed.state, OperationState::RiskEscalated);
        assert_eq!(
            failed.reason_code.as_deref(),
            Some("recovery_finalize_failed")
        );
        assert!(!manifest.exists());
        assert!(!authority.exists());
        let active: TransactionJournal =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        active.verify_integrity().unwrap();
        assert_eq!(active.phase, JournalPhase::Applied);
        assert!(active.terminal_recovery.is_none());
        let schema = || {
            OperationRequest::Schema(SchemaRequest {
                context: OperationContext::default(),
                operation: None,
            })
        };
        assert_eq!(
            plane.decide(&opened, schema()).unwrap_err().code,
            "session_unknown"
        );

        let reopened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("recovery-retry-policy-2"),
            })
            .unwrap();
        let retried = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: reopened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        let receipt = retried.receipt.unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Recovered);
        assert_eq!(
            receipt.observed_write_set,
            vec![runtime_root
                .join(format!(".ags-transaction-{action_ref}.json"))
                .display()
                .to_string()]
        );
        assert_eq!(
            plane.decide(&reopened, schema()).unwrap_err().code,
            "session_unknown"
        );

        let terminal_open = plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("recovery-retry-policy-3"),
            })
            .unwrap();
        assert!(terminal_open.pending_recovery_action_ref().is_none());
        assert_eq!(terminal_open.terminal_recovery_count, 1);
        assert_eq!(terminal_open.recovery_receipt(), Some(&receipt));
        assert!(terminal_open.terminal_recovery_digest.is_some());
        let terminal: TransactionJournal =
            serde_json::from_slice(&fs::read(journal_path).unwrap()).unwrap();
        terminal.verify_integrity().unwrap();
        assert_eq!(terminal.phase, JournalPhase::Recovered);
        validate_terminal_recovery_record(&terminal, terminal.terminal_recovery.as_ref().unwrap())
            .unwrap();
    }

    #[test]
    fn restart_recovery_is_idempotent_after_business_restore_before_terminal_finalize() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let baseline = exact_tree(&root);
        let binding =
            binding_with_runtime(&root, &runtime_root, sha256("idempotent-recovery-facts"));
        let sealing_key = sha256("idempotent-recovery-daemon");
        let mut first = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = first
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("idempotent-recovery-policy"),
            })
            .unwrap();
        let original_action_ref = first
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        let record = first.actions.get(&original_action_ref).unwrap().clone();
        first
            .adapter
            .prepare_journal(
                &original_action_ref,
                &record.plan,
                &record.domain_action,
                &binding,
            )
            .unwrap();
        first
            .adapter
            .transition_journal(
                &original_action_ref,
                &record.plan,
                JournalPhase::Applying,
                None,
            )
            .unwrap();
        let observation = match &record.domain_action {
            ProductionAction::Projections(projections) => apply_projections(
                &first.adapter,
                &original_action_ref,
                &record.plan,
                projections,
            )
            .unwrap(),
            other => panic!("expected pristine init projections, got {other:?}"),
        };
        first
            .adapter
            .transition_journal_applied(
                &original_action_ref,
                &record.plan,
                &record.domain_action,
                &observation.output_digest,
            )
            .unwrap();
        let (_, _, frozen_journal) = first.adapter.load_journal(&original_action_ref).unwrap();
        let frozen_identity_digest = frozen_journal.identity_digest.clone();
        assert!(frozen_journal
            .ordered_writes
            .iter()
            .all(|write| matches!(write.apply_anchor, JournalApplyAnchor::Applied { .. })));
        assert_ne!(exact_tree(&root), baseline);
        drop(first);

        let mut first_recovery = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let recovery_session = first_recovery
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("idempotent-recovery-policy-2"),
            })
            .unwrap();
        FAIL_NEXT_RECOVERY_FINALIZE.with(|flag| flag.set(true));
        let interrupted = first_recovery
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(interrupted.state, OperationState::RiskEscalated);
        assert_eq!(
            interrupted.reason_code.as_deref(),
            Some("recovery_finalize_failed")
        );
        assert_eq!(exact_tree(&root), baseline);
        let (_, _, interrupted_journal) = first_recovery
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(interrupted_journal.phase, JournalPhase::Applied);
        assert!(interrupted_journal.terminal_recovery.is_none());
        assert_eq!(interrupted_journal.identity_digest, frozen_identity_digest);
        assert!(interrupted_journal
            .ordered_writes
            .iter()
            .all(|write| matches!(
                write.recovery_progress,
                JournalWriteRecoveryProgress::Restored { .. }
            )));
        drop(first_recovery);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let restarted_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("idempotent-recovery-policy-3"),
            })
            .unwrap();
        let retried = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: restarted_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(retried.state, OperationState::Receipted, "{retried:?}");
        assert_eq!(retried.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(exact_tree(&root), baseline);
        let (_, _, terminal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(terminal.phase, JournalPhase::Recovered);
        assert!(terminal.terminal_recovery.is_some());
        assert_eq!(terminal.identity_digest, frozen_identity_digest);
    }

    #[test]
    fn restart_marks_restored_child_after_crash_before_progress_cas() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let baseline = exact_tree(&root);
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("restore-before-progress-facts"),
        );
        let sealing_key = sha256("restore-before-progress-daemon");
        let mut first = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = first
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("restore-before-progress-policy"),
            })
            .unwrap();
        let original_action_ref = first
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        let record = first.actions.get(&original_action_ref).unwrap().clone();
        first
            .adapter
            .prepare_journal(
                &original_action_ref,
                &record.plan,
                &record.domain_action,
                &binding,
            )
            .unwrap();
        first
            .adapter
            .transition_journal(
                &original_action_ref,
                &record.plan,
                JournalPhase::Applying,
                None,
            )
            .unwrap();
        let observation = match &record.domain_action {
            ProductionAction::Projections(projections) => apply_projections(
                &first.adapter,
                &original_action_ref,
                &record.plan,
                projections,
            )
            .unwrap(),
            other => panic!("expected pristine init projections, got {other:?}"),
        };
        first
            .adapter
            .transition_journal_applied(
                &original_action_ref,
                &record.plan,
                &record.domain_action,
                &observation.output_digest,
            )
            .unwrap();
        let (_, _, frozen) = first.adapter.load_journal(&original_action_ref).unwrap();
        let frozen_identity_digest = frozen.identity_digest.clone();
        let first_reverse_write = frozen.ordered_writes.last().unwrap().clone();
        drop(first);

        let mut interrupted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let recovery_session = interrupted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("restore-before-progress-policy-2"),
            })
            .unwrap();
        PANIC_RECOVERY_AFTER_RESTORE_BEFORE_PROGRESS.with(|flag| flag.set(true));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = interrupted.apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(!PANIC_RECOVERY_AFTER_RESTORE_BEFORE_PROGRESS.with(std::cell::Cell::get));
        drop(interrupted);

        let inspection_adapter = ProductionEffectAdapter::new(&runtime_root);
        let (_, _, interrupted_journal) = inspection_adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(interrupted_journal.phase, JournalPhase::Applied);
        assert_eq!(interrupted_journal.recovery_generation, 0);
        assert_eq!(interrupted_journal.identity_digest, frozen_identity_digest);
        assert!(interrupted_journal
            .ordered_writes
            .iter()
            .all(|write| matches!(
                write.recovery_progress,
                JournalWriteRecoveryProgress::Applied
            )));
        let restored_child = inspection_adapter
            .journal_write_target(&binding, &first_reverse_write)
            .unwrap();
        assert!(preimage_matches(&first_reverse_write.preimage, &restored_child).unwrap());
        assert!(journal_apply_anchor_matches(&first_reverse_write, &restored_child).unwrap());
        assert!(Path::new(&first_reverse_write.path)
            .parent()
            .unwrap()
            .is_dir());
        assert_ne!(exact_tree(&root), baseline);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let restarted_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("restore-before-progress-policy-3"),
            })
            .unwrap();
        let recovered = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: restarted_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.state, OperationState::Receipted, "{recovered:?}");
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(exact_tree(&root), baseline);
        let (_, _, terminal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(terminal.phase, JournalPhase::Recovered);
        assert_eq!(terminal.identity_digest, frozen_identity_digest);
    }

    #[test]
    fn applied_journal_with_pending_anchor_is_rejected_before_business_restore() {
        let (_workspace, runtime, root, _before_binding, original_action_ref) = crashed_init();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let adapter = ProductionEffectAdapter::new(&runtime_root);
        let (journal_target, previous, mut journal) =
            adapter.load_journal(&original_action_ref).unwrap();
        let write = journal.ordered_writes.first_mut().unwrap();
        write.apply_anchor = JournalApplyAnchor::Pending;
        write.post_identity = None;
        journal.identity_digest = journal.recompute_identity_digest().unwrap();
        journal.reseal().unwrap();
        let forged = serde_json::to_vec_pretty(&journal).unwrap();
        anchored_write(&journal_target, Some(&previous), &forged).unwrap();
        let business_before = journal
            .ordered_writes
            .iter()
            .map(|write| (write.path.clone(), fs::read(&write.path).unwrap()))
            .collect::<Vec<_>>();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("pending-anchor-recovery-facts"),
        );
        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sha256("pending-anchor-recovery-daemon"),
        );
        let opened = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("pending-anchor-recovery-policy"),
            })
            .unwrap();
        let result = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: opened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::RiskEscalated, "{result:?}");
        assert_eq!(
            result.reason_code.as_deref(),
            Some("transaction_recovery_failed")
        );
        let business_after = business_before
            .iter()
            .map(|(path, _)| (path.clone(), fs::read(path).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(business_after, business_before);
    }

    #[test]
    fn ordinary_rollback_persists_terminal_receipt_and_reopen_is_read_only() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("ordinary-recovery-facts"));
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("ordinary-recovery-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("ordinary-recovery-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &opened,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        FAIL_NEXT_TRANSACTION_VERIFY.with(|flag| flag.set(true));
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        let receipt = result.receipt.unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Recovered);
        assert_eq!(result.reason_code.as_deref(), Some("transaction_recovered"));
        assert_eq!(
            plane
                .decide(
                    &opened,
                    OperationRequest::Schema(SchemaRequest {
                        context: OperationContext::default(),
                        operation: None,
                    }),
                )
                .unwrap_err()
                .code,
            "session_unknown"
        );
        let journal_path = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        let before_open = fs::read(&journal_path).unwrap();
        let journal: TransactionJournal = serde_json::from_slice(&before_open).unwrap();
        journal.verify_integrity().unwrap();
        assert_eq!(journal.phase, JournalPhase::Recovered);
        validate_terminal_recovery_record(&journal, journal.terminal_recovery.as_ref().unwrap())
            .unwrap();
        let reopened = plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("ordinary-recovery-policy-2"),
            })
            .unwrap();
        assert!(reopened.pending_recovery_action_ref().is_none());
        assert_eq!(reopened.recovery_receipt(), Some(&receipt));
        assert_eq!(fs::read(journal_path).unwrap(), before_open);
    }

    #[test]
    fn ordinary_finalize_failure_leaves_active_journal_for_explicit_recovery() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("ordinary-finalize-facts"));
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("ordinary-finalize-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("ordinary-finalize-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &opened,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        FAIL_NEXT_TRANSACTION_VERIFY.with(|flag| flag.set(true));
        FAIL_NEXT_RECOVERY_FINALIZE.with(|flag| flag.set(true));
        let failed = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(failed.state, OperationState::RiskEscalated);
        assert_eq!(
            failed.reason_code.as_deref(),
            Some("recovery_finalize_failed")
        );
        let (_, _, active) = plane.adapter.load_journal(&action_ref).unwrap();
        assert_eq!(active.phase, JournalPhase::Applied);
        assert!(active.terminal_recovery.is_none());
        let reopened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("ordinary-finalize-policy-2"),
            })
            .unwrap();
        let recovered = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: reopened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(
            recovered.receipt.as_ref().unwrap().status,
            ReceiptStatus::Recovered,
            "{recovered:?}"
        );
    }

    #[test]
    fn risk_journal_write_failure_is_not_swallowed_and_preserves_actual_write_set() {
        let (_workspace, runtime, _root, binding, action_ref) = crashed_init();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let (_, _, journal) = adapter.load_journal(&action_ref).unwrap();
        assert!(journal.ordered_writes.len() >= 2);
        let drift = journal.ordered_writes.first().unwrap();
        let restored = journal.ordered_writes.last().unwrap();
        fs::write(&drift.path, b"third-party-drift").unwrap();
        FAIL_NEXT_RISK_JOURNAL_WRITE.with(|flag| flag.set(true));
        let action = ProductionAction::Files(
            journal
                .ordered_writes
                .iter()
                .map(|write| {
                    let target = adapter
                        .anchored_target(&binding, Path::new(&write.path))
                        .unwrap();
                    let content = match &write.postimage {
                        JournalPostimage::RegularFile { .. } => {
                            anchored_read(&target, false).unwrap().unwrap_or_default()
                        }
                        JournalPostimage::Directory { .. } => {
                            panic!("regular-file recovery fixture received directory postimage")
                        }
                        JournalPostimage::Symlink { .. } => {
                            panic!("regular-file recovery fixture received symlink postimage")
                        }
                        JournalPostimage::Absent => Vec::new(),
                    };
                    PlannedWrite {
                        target,
                        previous: match &write.preimage {
                            JournalImage::RegularFile { data_hex, .. } => {
                                Some(decode_hex(data_hex).unwrap())
                            }
                            JournalImage::Directory { .. } => {
                                panic!("regular-file recovery fixture received directory preimage")
                            }
                            JournalImage::Symlink { .. } => {
                                panic!("regular-file recovery fixture received symlink preimage")
                            }
                            JournalImage::Absent => None,
                        },
                        content,
                    }
                })
                .collect(),
        );
        let plan = SealedPlan {
            schema_version: CONTRACT_SCHEMA_VERSION.to_string(),
            plan_hash: journal.plan_hash.clone(),
            operation: OperationName::Setup,
            kind: OperationKind::Transaction,
            binding_hash: journal.binding_hash.clone(),
            policy_hash: journal.policy_hash.clone(),
            payload_hash: journal.payload_hash.clone(),
            action_digest: sha256("test-action"),
            steps: Vec::new(),
            expected_write_paths: journal
                .ordered_writes
                .iter()
                .map(|write| write.path.clone())
                .collect(),
            verification: VerificationSpec { checks: Vec::new() },
            recoverability: Recoverability::Transactional,
            execution: None,
        };
        let error = adapter
            .recover_journal(&action_ref, &plan, &action)
            .unwrap_err();
        assert_eq!(error.code, "transaction_risk_journal_write_failed");
        assert!(error.effect_started);
        assert!(
            error.observed_write_set.is_empty(),
            "Phase A must not restore `{}` before the risk journal write",
            restored.path
        );
        let (_, _, still_active) = adapter.load_journal(&action_ref).unwrap();
        assert_eq!(still_active.phase, JournalPhase::Applied);
    }

    #[test]
    fn open_fails_closed_without_writes_when_two_active_journals_exist() {
        let (_workspace, runtime, root, _before_binding, action_ref) = crashed_init();
        let first_path = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        let first_raw = fs::read(&first_path).unwrap();
        let mut second: TransactionJournal = serde_json::from_slice(&first_raw).unwrap();
        second.action_ref = "second-active".to_string();
        second.identity_digest = second.recompute_identity_digest().unwrap();
        second.reseal().unwrap();
        let second_path = runtime.path().join(".ags-transaction-second-active.json");
        fs::write(&second_path, serde_json::to_vec_pretty(&second).unwrap()).unwrap();
        let manifest = runtime.path().join("install-manifest.json");
        let manifest_before = fs::read(&manifest).unwrap();
        let first_before = fs::read(&first_path).unwrap();
        let second_before = fs::read(&second_path).unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("two-active-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("two-active-daemon"),
        );
        let error = plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("two-active-policy"),
            })
            .unwrap_err();
        assert_eq!(error.code, "pending_transaction_inspection_failed");
        assert!(error.detail.contains("multiple_pending_transactions"));
        assert_eq!(fs::read(manifest).unwrap(), manifest_before);
        assert_eq!(fs::read(first_path).unwrap(), first_before);
        assert_eq!(fs::read(second_path).unwrap(), second_before);
    }

    #[test]
    fn terminal_receipt_does_not_mask_a_distinct_active_journal() {
        let (_workspace, runtime, root, _before_binding, action_ref) = crashed_init();
        let original_path = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        let original_active = fs::read(&original_path).unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("terminal-plus-active-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("terminal-plus-active-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("terminal-plus-active-policy"),
            })
            .unwrap();
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: opened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();

        let mut second: TransactionJournal = serde_json::from_slice(&original_active).unwrap();
        second.action_ref = "second-active".to_string();
        second.identity_digest = second.recompute_identity_digest().unwrap();
        second.reseal().unwrap();
        fs::write(
            runtime.path().join(".ags-transaction-second-active.json"),
            serde_json::to_vec_pretty(&second).unwrap(),
        )
        .unwrap();
        let mixed = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("terminal-plus-active-policy-2"),
            })
            .unwrap();
        assert!(mixed.pending_recovery_action_ref().is_some());
        assert_eq!(mixed.terminal_recovery_count, 1);
        assert_eq!(
            mixed.recovery_receipt().unwrap().status,
            ReceiptStatus::Recovered
        );
        let recovered = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: mixed.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
    }

    #[test]
    fn resealed_but_tampered_terminal_receipt_blocks_open() {
        let (_workspace, runtime, root, _before_binding, action_ref) = crashed_init();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("tampered-terminal-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("tampered-terminal-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("tampered-terminal-policy"),
            })
            .unwrap();
        plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: opened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        let journal_path = runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.json"));
        let mut terminal: TransactionJournal =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        terminal
            .terminal_recovery
            .as_mut()
            .unwrap()
            .receipt
            .receipt_id = "receipt-v2-tampered".to_string();
        terminal.reseal().unwrap();
        fs::write(journal_path, serde_json::to_vec_pretty(&terminal).unwrap()).unwrap();
        let error = plane
            .open(OpenRequest {
                binding,
                policy_hash: sha256("tampered-terminal-policy-2"),
            })
            .unwrap_err();
        assert_eq!(error.code, "pending_transaction_inspection_failed");
        assert!(error.detail.contains("terminal_recovery_receipt_invalid"));
    }

    #[test]
    fn pending_inspection_of_missing_runtime_is_read_only_and_allows_bootstrap() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime_parent = tempfile::tempdir().unwrap();
        let runtime = runtime_parent.path().join("not-created");
        let binding = AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &root,
            "workspace-a",
            sha256("facts-a"),
            "registry-a",
            "session-a",
            vec![root.clone(), runtime.clone()],
        );
        let adapter = ProductionEffectAdapter::new(&runtime);
        assert!(adapter
            .inspect_pending_transaction(&binding)
            .unwrap()
            .active
            .is_none());
        assert!(!runtime.exists());
    }

    #[test]
    fn red_sealed_write_set_mismatch_is_rejected_before_adapter_mutation() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-before-mismatch"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("mismatch-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("mismatch-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        plane
            .actions
            .get_mut(&action_ref)
            .unwrap()
            .plan
            .expected_write_paths
            .clear();

        let error = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err();

        assert_eq!(error.code, "sealed_action_mismatch");
        assert!(
            !runtime.path().join("install-manifest.json").exists(),
            "sealed/action write-set mismatch must stop before invoking the adapter"
        );
    }

    #[test]
    fn generic_agent_registration_is_transactional_and_update_is_host_delegated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let candidate = runtime.path().join("update-candidates/0.4.20");
        fs::create_dir_all(&candidate).unwrap();
        for name in RELEASE_PAYLOAD_NAMES {
            fs::write(candidate.join(name), name.as_bytes()).unwrap();
        }
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let agent = OperationRequest::AgentRegister(AgentRegisterRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            surface: AgentSurface::Hybrid,
        });
        let update = OperationRequest::Update(UpdateRequest {
            context: OperationContext::default(),
            channel: "stable".to_string(),
            target_version: Some("0.4.20".to_string()),
        });

        let planned = adapter.plan(&agent, &binding(&root)).unwrap();
        let PlanDisposition::Planned(planned) = planned else {
            panic!("new registration must produce a host plan")
        };
        assert_eq!(planned.plan.recoverability, Recoverability::Transactional);
        assert!(planned
            .plan
            .expected_write_paths
            .iter()
            .all(|path| path.starts_with(adapter.runtime_home.to_string_lossy().as_ref())));
        let PlanDisposition::Planned(update) = adapter.plan(&update, &binding(&root)).unwrap()
        else {
            panic!("update must produce a host plan")
        };
        assert_eq!(update.plan.recoverability, Recoverability::NotApplicable);
    }

    #[test]
    fn lifecycle_plan_action_seals_exact_receipt_ids_and_empty_ids_are_rejected() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-a"),
        );
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let evidence_dir = root.join(".ags/evidence");
        let machine_key = [7_u8; 32];
        std::fs::write(runtime.path().join(CLOSURE_AUTHORITY_KEY_FILE), machine_key).unwrap();
        let pointer_dir = root.join(".ags/state/closure-pointers");
        std::fs::create_dir_all(&evidence_dir).unwrap();
        std::fs::create_dir_all(&pointer_dir).unwrap();
        let task_path = root.join("task.md");
        let plan_path = root.join("launch-plan.json");
        let report_path = root.join("delivery-report.md");
        std::fs::write(&task_path, b"canonical task").unwrap();
        std::fs::write(&report_path, b"delivery report").unwrap();
        let task_card_hash = ags_evidence::sha256_hex(b"canonical task");
        let mut launch_plan = serde_json::json!({
            "schema_version": ags_task_contract::runner::SCHEMA_VERSION,
            "task_card_hash": task_card_hash,
        });
        let launch_plan_hash =
            ags_task_contract::runner::canonical_launch_plan_hash(&launch_plan).unwrap();
        launch_plan["launch_plan_hash"] = serde_json::Value::String(launch_plan_hash.clone());
        std::fs::write(&plan_path, serde_json::to_vec_pretty(&launch_plan).unwrap()).unwrap();
        let delivery_report_hash = ags_evidence::sha256_hex(b"delivery report");
        let receipt_id = ags_evidence::receipt_id(&task_card_hash, &launch_plan_hash);
        let receipt = ags_evidence::Receipt {
            schema_version: ags_evidence::RECEIPT_SCHEMA_VERSION.to_string(),
            receipt_id: receipt_id.clone(),
            timestamp: "unix-0".to_string(),
            task_card_hash: task_card_hash.clone(),
            launch_plan_hash: launch_plan_hash.clone(),
            task_card_path: Some(task_path.display().to_string()),
            launch_plan_path: plan_path.display().to_string(),
            delivery_report_path: report_path.display().to_string(),
            gate_result: ags_evidence::GateResult {
                decision: "allow".to_string(),
                reason: None,
            },
            verification_results: Vec::new(),
            delivery_report_hash: delivery_report_hash.clone(),
            execution_footprint: ags_evidence::ExecutionFootprint {
                execution_mode_used: "single-writer".to_string(),
                execution_topology_used: "single".to_string(),
                delegation_used: "none".to_string(),
            },
            closure_status: "completed".to_string(),
            exit_code: Some(0),
            governance_status: Some(ags_governance_decision::GovernanceStatus::DoneWithReceipt),
            governance_evidence: None,
        };
        let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
        let receipt_path = evidence_dir.join(format!("{receipt_id}.json"));
        std::fs::write(&receipt_path, &receipt_bytes).unwrap();
        let pointer_path = pointer_dir.join(format!("{receipt_id}.json"));
        let mut pointer = crate::workspace_lifecycle::ClosurePointer {
            schema_version: crate::workspace_lifecycle::CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: Some(root.display().to_string()),
            workspace_identity: Some(crate::workspace_lifecycle::workspace_identity(&root)),
            receipt_id,
            receipt_path: receipt_path.display().to_string(),
            receipt_sha256: sha256(&receipt_bytes),
            task_card_hash,
            launch_plan_hash,
            delivery_report_hash,
            authority_key_id: String::new(),
            authority_seal: String::new(),
        };
        crate::workspace_lifecycle::seal_closure_pointer(&machine_key, &mut pointer).unwrap();
        std::fs::write(&pointer_path, serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();
        let request = LifecycleSessionEndRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "session-a".to_string(),
            event_id: "event-a".to_string(),
        };
        let PlanDisposition::Planned(planned) = adapter
            .lifecycle_session_end_plan(&request, &binding)
            .unwrap()
        else {
            panic!("session end must plan")
        };
        assert!(
            format!("{:?}", planned.action).contains("LifecycleSessionEnd"),
            "the in-memory sealed action must retain the exact receipt id set"
        );
        let sealed_plan = SealedPlan {
            schema_version: CONTRACT_SCHEMA_VERSION.to_string(),
            plan_hash: sha256("plan"),
            operation: OperationName::HostLifecycleSessionEnd,
            kind: OperationKind::HostDelegated,
            binding_hash: sha256("binding"),
            policy_hash: sha256("policy"),
            payload_hash: sha256("payload"),
            action_digest: planned.plan.action_digest.clone(),
            steps: planned.plan.steps.clone(),
            expected_write_paths: planned.plan.expected_write_paths.clone(),
            verification: planned.plan.verification.clone(),
            recoverability: planned.plan.recoverability,
            execution: planned.plan.execution.clone(),
        };

        let outcome = HostOutcomeReceipt {
            schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
            action_ref: "action".to_string(),
            binding_hash: sealed_plan.binding_hash.clone(),
            plan_hash: sealed_plan.plan_hash.clone(),
            policy_hash: sealed_plan.policy_hash.clone(),
            instruction_digest: sha256("instruction"),
            outcome_token: "token".to_string(),
            generation: 1,
            status: HostOutcomeStatus::Failed,
            output_digest: sha256("output"),
            observed_write_set: vec![root.join("target").display().to_string()],
            artifacts: Vec::new(),
            evidence: None,
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": "ags://schema/contract/v2/lifecycle-host-outcome",
            "event_id": request.event_id,
            "receipt_ids": [],
            "observed_write_set": [],
            "consumed_pointer_paths": [],
            "output_digest": outcome.output_digest,
            "completed": false,
        }))
        .unwrap();
        let evidence = VerifiedHostEvidence {
            kind: HostEvidenceKind::LifecycleReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: "memory://lifecycle".to_string(),
                sha256: sha256(&bytes),
            },
            bytes,
        };
        assert!(verify_lifecycle_host_outcome(
            &request,
            &sealed_plan,
            &planned.action,
            &outcome,
            &evidence,
        )
        .is_err());
    }

    #[test]
    fn production_dispatch_contains_no_silent_operation_disconnect_wildcards() {
        let source = include_str!("production.rs");
        assert!(!source.contains(concat!("operation_not_", "connected")));
        assert!(!source.contains(concat!("read_operation_not_", "connected")));
        assert!(!source.contains(concat!("Operation", "Handler")));
        assert!(!source.contains(concat!("typed_", "request!")));
        assert!(!source.contains(concat!("operation.", "handler()")));
        assert!(source.contains("for_each_operation!(production_dispatcher)"));
    }

    #[test]
    fn production_dispatch_wrong_stage_is_a_structured_contract_error() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let adapter = ProductionEffectAdapter::new(workspace.path());
        let binding = binding(&root);
        let operation = OperationRequest::Schema(SchemaRequest {
            context: OperationContext::default(),
            operation: None,
        });

        let result =
            operation.dispatch_production(&adapter, ProductionStage::Plan { binding: &binding });
        let ProductionStageResult::Plan(result) = result else {
            panic!("wrong-stage dispatch must preserve the requested result shape");
        };
        let error = result.unwrap_err();
        assert_eq!(error.code, "operation_kind_dispatch_mismatch");
        assert_eq!(error.detail, OperationName::Schema.as_str());
    }

    #[test]
    fn pristine_init_seals_and_observes_every_created_projection_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("pristine-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("pristine-init-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let decision = plane
            .decide(
                &session,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap();
        let plan = decision.plan.unwrap();
        let expected_directories = [
            root.join("config"),
            root.join(".ags"),
            root.join(".ags/evidence"),
            root.join(".ags/state"),
            root.join(".ags/state/closure-pointers"),
        ];
        for directory in &expected_directories {
            assert!(plan
                .expected_write_paths
                .contains(&directory.display().to_string()));
        }
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: decision.action_ref.unwrap(),
                    outcome: None,
                },
            )
            .unwrap();
        let receipt = result.receipt.unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Succeeded);
        for directory in expected_directories {
            assert!(directory.is_dir());
            assert!(receipt
                .observed_write_set
                .contains(&directory.display().to_string()));
        }
    }

    #[test]
    fn projection_partial_effect_false_flag_recovers_exact_tree_without_fake_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("projection-partial-facts"),
        );
        let before = exact_tree(&root);
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("projection-partial-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-partial-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        PRODUCTION_DOMAIN_APPLY_CALLS.with(|calls| calls.set(0));
        FAIL_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.set(true));
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::Receipted, "{result:?}");
        assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(PRODUCTION_DOMAIN_APPLY_CALLS.with(std::cell::Cell::get), 1);
        assert!(!FAIL_PROJECTION_AFTER_FIRST_MUTATION.with(std::cell::Cell::get));
        let (_, _, journal) = plane.adapter.load_journal(&action_ref).unwrap();
        assert_eq!(journal.phase, JournalPhase::Recovered);
        let terminal = journal
            .terminal_recovery
            .expect("outer journal must durably bind the terminal recovery receipt");
        assert_eq!(terminal.original_action_ref, action_ref);
        assert_eq!(terminal.receipt.status, ReceiptStatus::Recovered);
        assert!(terminal.receipt.recovered);
        assert_eq!(exact_tree(&root), before);
    }

    #[test]
    fn projection_midpoint_crash_reopens_as_pending_outer_recovery() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("projection-crash-facts"));
        let before = exact_tree(&root);
        let sealing_key = sha256("projection-crash-daemon");
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-crash-policy"),
            })
            .unwrap();
        let original_action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        PANIC_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.set(true));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: original_action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(!PANIC_PROJECTION_AFTER_FIRST_MUTATION.with(std::cell::Cell::get));
        assert_ne!(exact_tree(&root), before);

        let before_open_workspace = exact_tree(&root);
        let before_open_runtime = exact_tree(&runtime_root);
        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let recovery_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-crash-policy"),
            })
            .unwrap();
        assert_eq!(exact_tree(&root), before_open_workspace);
        assert_eq!(exact_tree(&runtime_root), before_open_runtime);
        let recovery_action_ref = recovery_session
            .pending_recovery_action_ref()
            .expect("restart must expose one typed outer recovery action")
            .to_string();
        let recovered = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_action_ref,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.state, OperationState::Receipted, "{recovered:?}");
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(exact_tree(&root), before);
        let (_, _, journal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(journal.phase, JournalPhase::Recovered);
        let terminal = journal
            .terminal_recovery
            .expect("restart recovery must durably close the original outer journal");
        assert_eq!(terminal.original_action_ref, original_action_ref);
        assert_eq!(terminal.receipt.status, ReceiptStatus::Recovered);
    }

    #[test]
    fn projection_third_party_drift_is_preserved_and_outer_receipt_is_risk() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(&root, &runtime_root, sha256("projection-drift-facts"));
        let sealing_key = sha256("projection-drift-daemon");
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-drift-policy"),
            })
            .unwrap();
        let original_action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        DRIFT_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.set(true));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: original_action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(!DRIFT_PROJECTION_AFTER_FIRST_MUTATION.with(std::cell::Cell::get));
        let drifted_workspace = exact_tree(&root);
        assert_eq!(
            fs::read(root.join(".ags/third-party.txt")).unwrap(),
            b"third-party-drift"
        );

        let before_open_runtime = exact_tree(&runtime_root);
        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let recovery_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-drift-policy"),
            })
            .unwrap();
        assert_eq!(exact_tree(&root), drifted_workspace);
        assert_eq!(exact_tree(&runtime_root), before_open_runtime);
        let recovery_action_ref = recovery_session
            .pending_recovery_action_ref()
            .expect("drifted outer journal must remain the only pending recovery")
            .to_string();
        let risk = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(risk.state, OperationState::RiskEscalated, "{risk:?}");
        assert_eq!(risk.receipt.unwrap().status, ReceiptStatus::RiskEscalated);
        assert_eq!(exact_tree(&root), drifted_workspace);
        assert_eq!(
            fs::read(root.join(".ags/third-party.txt")).unwrap(),
            b"third-party-drift"
        );
        let (_, _, risk_journal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(risk_journal.phase, JournalPhase::RiskEscalated);
        assert!(risk_journal.terminal_recovery.is_none());
        restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_action_ref,
                    outcome: None,
                },
            )
            .expect_err("risk escalation must invalidate the recovery session");
    }

    #[test]
    fn projection_directory_without_durable_post_identity_never_claims_same_shape_replacement() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("projection-directory-identity-gap-facts"),
        );
        let sealing_key = sha256("projection-directory-identity-gap-daemon");
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-directory-identity-gap-policy"),
            })
            .unwrap();
        let original_action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        PANIC_PROJECTION_BEFORE_IDENTITY_KIND.with(|slot| *slot.borrow_mut() = Some("directory"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: original_action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(PANIC_PROJECTION_BEFORE_IDENTITY_KIND.with(|slot| slot.borrow().is_none()));
        let created = root.join(".ags");
        let created_metadata = fs::symlink_metadata(&created).unwrap();
        let created_identity = (created_metadata.dev(), created_metadata.ino());
        let (_, _, applying) = plane.adapter.load_journal(&original_action_ref).unwrap();
        let write = applying
            .ordered_writes
            .iter()
            .find(|write| write.path == created.display().to_string())
            .unwrap();
        assert_eq!(applying.phase, JournalPhase::Applying);
        assert!(write.post_identity.is_none());

        fs::remove_dir(&created).unwrap();
        fs::create_dir(&created).unwrap();
        fs::set_permissions(&created, fs::Permissions::from_mode(0o755)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&created).unwrap();
        let replacement_identity = (replacement_metadata.dev(), replacement_metadata.ino());
        assert_ne!(replacement_identity, created_identity);
        let replacement_tree = exact_tree(&root);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let recovery_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-directory-identity-gap-policy"),
            })
            .unwrap();
        let recovery_action_ref = recovery_session
            .pending_recovery_action_ref()
            .unwrap()
            .to_string();
        let risk = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_action_ref,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(risk.state, OperationState::RiskEscalated, "{risk:?}");
        assert_eq!(risk.receipt.unwrap().status, ReceiptStatus::RiskEscalated);
        assert_eq!(exact_tree(&root), replacement_tree);
        let preserved = fs::symlink_metadata(&created).unwrap();
        assert_eq!((preserved.dev(), preserved.ino()), replacement_identity);
        let (_, _, journal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(journal.phase, JournalPhase::RiskEscalated);
    }

    #[test]
    fn projection_file_without_durable_post_identity_never_claims_same_bytes_mode_replacement() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        prepare_exact_owned_projection_workspace(&root);
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("projection-file-identity-gap-facts"),
        );
        let sealing_key = sha256("projection-file-identity-gap-daemon");
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-file-identity-gap-policy"),
            })
            .unwrap();
        let original_action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        PANIC_PROJECTION_BEFORE_IDENTITY_KIND
            .with(|slot| *slot.borrow_mut() = Some("regular_file"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: original_action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(PANIC_PROJECTION_BEFORE_IDENTITY_KIND.with(|slot| slot.borrow().is_none()));
        let agents = root.join("AGENTS.md");
        let desired_bytes = fs::read(&agents).unwrap();
        let created_metadata = fs::symlink_metadata(&agents).unwrap();
        assert_eq!(created_metadata.permissions().mode() & 0o7777, 0o600);
        let created_identity = (created_metadata.dev(), created_metadata.ino());
        let (_, _, applying) = plane.adapter.load_journal(&original_action_ref).unwrap();
        let write = applying
            .ordered_writes
            .iter()
            .find(|write| write.path == agents.display().to_string())
            .unwrap();
        assert!(write.post_identity.is_none());

        let replacement = root.join("AGENTS.replacement");
        fs::write(&replacement, &desired_bytes).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&replacement, &agents).unwrap();
        let replacement_metadata = fs::symlink_metadata(&agents).unwrap();
        let replacement_identity = (replacement_metadata.dev(), replacement_metadata.ino());
        assert_ne!(replacement_identity, created_identity);
        let replacement_tree = exact_tree(&root);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let recovery_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-file-identity-gap-policy"),
            })
            .unwrap();
        let risk = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(risk.state, OperationState::RiskEscalated, "{risk:?}");
        assert_eq!(risk.receipt.unwrap().status, ReceiptStatus::RiskEscalated);
        assert_eq!(exact_tree(&root), replacement_tree);
        assert_eq!(fs::read(&agents).unwrap(), desired_bytes);
        let preserved = fs::symlink_metadata(&agents).unwrap();
        assert_eq!((preserved.dev(), preserved.ino()), replacement_identity);
        let (_, _, journal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(journal.phase, JournalPhase::RiskEscalated);
    }

    #[test]
    fn projection_exact_owned_update_delete_materializer_binds_modes_preimages_and_footprint() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let (old_agents, obsolete) = prepare_exact_owned_projection_workspace(&root);
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("projection-exact-owned-materializer-facts"),
        );
        let adapter = ProductionEffectAdapter::new(&runtime_root);
        let PlanDisposition::Planned(planned) = adapter
            .init_plan(
                &InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                },
                &binding,
            )
            .unwrap()
        else {
            panic!("exact-owned update/delete fixture must be effectful")
        };
        let ProductionAction::Projections(projections) = &planned.action else {
            panic!("exact-owned fixture must materialize projection objects")
        };
        assert_eq!(projections.len(), 1);
        let mutations = &projections[0].mutations;
        assert_eq!(mutations.len(), 3);
        let agents = mutations
            .iter()
            .find(|mutation| mutation.target.sealed == root.join("AGENTS.md").display().to_string())
            .unwrap();
        assert!(matches!(
            &agents.preimage,
            JournalImage::RegularFile { data_hex, mode: 0o644, .. }
                if decode_hex(data_hex).unwrap() == old_agents
        ));
        assert!(matches!(
            agents.postimage,
            JournalPostimage::RegularFile { mode: 0o600, .. }
        ));
        let deleted = mutations
            .iter()
            .find(|mutation| {
                mutation.target.sealed == root.join("obsolete.txt").display().to_string()
            })
            .unwrap();
        assert!(matches!(
            &deleted.preimage,
            JournalImage::RegularFile { data_hex, mode: 0o750, .. }
                if decode_hex(data_hex).unwrap() == obsolete
        ));
        assert_eq!(deleted.postimage, JournalPostimage::Absent);
        let ownership = mutations
            .iter()
            .find(|mutation| {
                mutation.target.sealed == root.join(".ags/ownership-v2.json").display().to_string()
            })
            .unwrap();
        assert!(matches!(
            ownership.preimage,
            JournalImage::RegularFile { mode: 0o640, .. }
        ));
        assert!(matches!(
            ownership.postimage,
            JournalPostimage::RegularFile { mode: 0o600, .. }
        ));
        let actual = mutations
            .iter()
            .map(|mutation| mutation.target.sealed.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let expected = planned
            .plan
            .expected_write_paths
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn projection_exact_owned_update_partial_false_flag_restores_bytes_and_modes() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        prepare_exact_owned_projection_workspace(&root);
        let before = exact_tree(&root);
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("projection-exact-owned-partial-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sha256("projection-exact-owned-partial-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-exact-owned-partial-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        FAIL_PROJECTION_AFTER_FIRST_MUTATION.with(|flag| flag.set(true));
        let recovered = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.state, OperationState::Receipted, "{recovered:?}");
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(exact_tree(&root), before);
        let (_, _, journal) = plane.adapter.load_journal(&action_ref).unwrap();
        assert_eq!(journal.phase, JournalPhase::Recovered);
    }

    #[test]
    fn projection_exact_owned_delete_restart_recovery_restores_update_delete_and_modes() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        prepare_exact_owned_projection_workspace(&root);
        let before = exact_tree(&root);
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime_root,
            sha256("projection-exact-owned-delete-restart-facts"),
        );
        let sealing_key = sha256("projection-exact-owned-delete-restart-daemon");
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key.clone(),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-exact-owned-delete-restart-policy"),
            })
            .unwrap();
        let original_action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        PANIC_PROJECTION_AFTER_OPERATION.with(|slot| *slot.borrow_mut() = Some("delete_file"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: original_action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        assert!(PANIC_PROJECTION_AFTER_OPERATION.with(|slot| slot.borrow().is_none()));
        assert!(!root.join("obsolete.txt").exists());
        assert_ne!(exact_tree(&root), before);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sealing_key,
        );
        let recovery_session = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-exact-owned-delete-restart-policy"),
            })
            .unwrap();
        let recovered = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: recovery_session
                        .pending_recovery_action_ref()
                        .unwrap()
                        .to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.state, OperationState::Receipted, "{recovered:?}");
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(exact_tree(&root), before);
        let (_, _, journal) = restarted
            .adapter
            .load_journal(&original_action_ref)
            .unwrap();
        assert_eq!(journal.phase, JournalPhase::Recovered);
    }

    #[test]
    fn projection_sealed_expected_set_mismatch_is_pre_adapter_and_zero_write() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("projection-sealed-set-facts"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("projection-sealed-set-daemon"),
        );
        let opened = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("projection-sealed-set-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &opened,
                OperationRequest::Init(InitRequest {
                    context: OperationContext::default(),
                    migration: MigrationMode::ExactOwnedOnly,
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        let original = match &plane.actions[&action_ref].domain_action {
            ProductionAction::Projections(projections) => projections[0].mutations.clone(),
            other => panic!("expected projection action, got {other:?}"),
        };
        let mut extra_mutation = original[0].clone();
        extra_mutation.target = plane
            .adapter
            .anchored_target(&binding, &root.join("not-in-action"))
            .unwrap();
        let before_workspace = exact_tree(&root);
        let before_runtime = exact_tree(runtime.path());
        PRODUCTION_DOMAIN_APPLY_CALLS.with(|calls| calls.set(0));
        let ProductionAction::Projections(projections) =
            &mut plane.actions.get_mut(&action_ref).unwrap().domain_action
        else {
            unreachable!()
        };
        projections[0].mutations.pop();
        let missing = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap_err();
        assert_eq!(missing.code, "sealed_action_mismatch");
        assert_eq!(PRODUCTION_DOMAIN_APPLY_CALLS.with(std::cell::Cell::get), 0);
        assert_eq!(exact_tree(&root), before_workspace);
        assert_eq!(exact_tree(runtime.path()), before_runtime);

        let ProductionAction::Projections(projections) =
            &mut plane.actions.get_mut(&action_ref).unwrap().domain_action
        else {
            unreachable!()
        };
        projections[0].mutations = original;
        projections[0].mutations.push(extra_mutation);
        let extra = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            )
            .unwrap_err();
        assert_eq!(extra.code, "sealed_action_mismatch");
        assert_eq!(PRODUCTION_DOMAIN_APPLY_CALLS.with(std::cell::Cell::get), 0);
        assert_eq!(exact_tree(&root), before_workspace);
        assert_eq!(exact_tree(runtime.path()), before_runtime);
    }

    #[test]
    fn read_only_roots_are_operation_specific_and_never_hash_the_whole_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let adapter = ProductionEffectAdapter::new(&runtime_root);
        let binding = binding_with_runtime(&root, &runtime_root, sha256("facts-a"));
        assert!(adapter
            .read_only_roots(
                &OperationRequest::Schema(SchemaRequest {
                    context: OperationContext::default(),
                    operation: None,
                }),
                &binding,
            )
            .is_empty());
        let doctor = adapter.read_only_roots(
            &OperationRequest::Doctor(DoctorRequest {
                context: OperationContext::default(),
                scope: DoctorScope::All,
            }),
            &binding,
        );
        assert!(!doctor.contains(&root));
        assert!(!doctor.contains(&runtime_root));
        assert!(doctor.contains(&root.join("Cargo.toml")));
    }

    #[cfg(unix)]
    #[test]
    fn governed_reads_reject_symlink_fifo_and_oversize_inputs_without_blocking() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let binding = binding(&root);

        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"outside").unwrap();
        let link = root.join("link.md");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(read_binding_text(&binding, &link, 64, "read_failed").is_err());

        let fifo = root.join("input.fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        assert!(read_binding_text(&binding, &fifo, 64, "read_failed").is_err());

        let oversized = root.join("oversized.md");
        fs::write(&oversized, vec![b'x'; MAX_PROFILE_BYTES + 1]).unwrap();
        assert!(
            read_binding_text(&binding, &oversized, MAX_PROFILE_BYTES, "read_failed")
                .unwrap_err()
                .detail
                .contains("host_outcome_artifact_too_large")
        );
    }

    #[test]
    fn stage_write_failure_preserves_residue_and_risk_escalates() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &root,
            "workspace-a",
            sha256("facts-a"),
            "registry-a",
            "session-a",
            vec![root.clone(), runtime_root.clone()],
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sha256("stage-write-failure-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        FAIL_NEXT_STAGE_WRITE.with(|flag| flag.set(true));
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
        let receipt = result.receipt.unwrap();
        assert_eq!(receipt.status, ReceiptStatus::RiskEscalated);
        let residue = receipt
            .observed_write_set
            .iter()
            .find(|path| path.contains("/.ags-txn-"))
            .map(PathBuf::from)
            .expect("preserved stage residue is receipt-visible");
        assert!(residue.is_file());
        assert!(fs::read(residue).unwrap().is_empty());
        assert!(!runtime_root.join("install-manifest.json").exists());
    }

    #[test]
    fn applied_journal_transition_failure_preserves_effect_and_recovers() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        let binding = AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &root,
            "workspace-a",
            sha256("facts-a"),
            "registry-a",
            "session-a",
            vec![root.clone(), runtime_root.clone()],
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime_root),
            sha256("journal-applied-failure-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let decision = plane
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap();
        let target = runtime_root.join("install-manifest.json");
        FAIL_NEXT_JOURNAL_APPLIED.with(|flag| flag.set(true));
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
        assert_eq!(result.reason_code.as_deref(), Some("transaction_recovered"));
        let receipt = result.receipt.unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Recovered);
        assert!(receipt
            .observed_write_set
            .contains(&target.display().to_string()));
        assert!(receipt.observed_write_set.contains(
            &runtime_root
                .join(CLOSURE_AUTHORITY_KEY_FILE)
                .display()
                .to_string()
        ));
        assert!(!target.exists());
    }

    #[test]
    fn staged_file_digest_is_computed_from_the_original_readable_fd() {
        let directory = tempfile::tempdir().unwrap();
        let parent = rustix::fs::open(
            directory.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let content = b"fd-bound-stage-content";
        let (_name, fd) = create_staged_file(&parent, content, directory.path()).unwrap();
        assert_eq!(
            file_digest(&fd, "transaction_stage_digest_failed").unwrap(),
            sha256(content)
        );
    }

    #[cfg(unix)]
    #[test]
    fn skill_index_body_link_snapshot_midpoint_failures_restore_exact_outer_tree() {
        let (temp, _runtime, binding, adapter, request) = skill_install_fixture();
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-midpoint-daemon"));
        let mut session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-midpoint-policy"),
            })
            .unwrap();
        let baseline = managed_tree_without_control_journals(temp.path());
        for kind in ["index", "body", "link", "snapshot"] {
            let action_ref = plane
                .decide(
                    &session,
                    OperationRequest::GovernSkillInstall(request.clone()),
                )
                .unwrap()
                .action_ref
                .unwrap();
            FAIL_SKILL_AFTER_MUTATION_KIND.with(|slot| *slot.borrow_mut() = Some(kind));
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
            assert_eq!(managed_tree_without_control_journals(temp.path()), baseline);
            session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("skill-midpoint-policy-{kind}")),
                })
                .unwrap();
        }

        let install_ref = plane
            .decide(
                &session,
                OperationRequest::GovernSkillInstall(request.clone()),
            )
            .unwrap()
            .action_ref
            .unwrap();
        let installed = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: install_ref,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(installed.receipt.unwrap().status, ReceiptStatus::Succeeded);
        let installed_tree = managed_tree_without_control_journals(temp.path());
        for kind in ["index", "body", "link", "snapshot"] {
            let remove_ref = plane
                .decide(
                    &session,
                    OperationRequest::GovernSkillRemove(SkillRemoveRequest {
                        context: OperationContext::default(),
                        skill_id: "control-plane-fixture".to_string(),
                    }),
                )
                .unwrap()
                .action_ref
                .unwrap();
            FAIL_SKILL_AFTER_MUTATION_KIND.with(|slot| *slot.borrow_mut() = Some(kind));
            let result = plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: remove_ref,
                        outcome: None,
                    },
                )
                .unwrap();
            assert_eq!(result.receipt.unwrap().status, ReceiptStatus::Recovered);
            assert_eq!(
                managed_tree_without_control_journals(temp.path()),
                installed_tree
            );
            session = plane
                .open(OpenRequest {
                    binding: binding.clone(),
                    policy_hash: sha256(format!("skill-remove-midpoint-policy-{kind}")),
                })
                .unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn skill_midpoint_crash_reopens_as_pending_outer_recovery() {
        let (temp, runtime, binding, adapter, request) = skill_install_fixture();
        let baseline = managed_tree_without_control_journals(temp.path());
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-crash-daemon-a"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-crash-policy-a"),
            })
            .unwrap();
        let action_ref = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap()
            .action_ref
            .unwrap();
        PANIC_SKILL_AFTER_MUTATION_KIND.with(|slot| *slot.borrow_mut() = Some("body"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome: None,
                },
            );
        }));
        assert!(
            crashed.is_err(),
            "test seam must stop after a real partial mutation"
        );
        drop(plane);

        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(&runtime),
            sha256("skill-crash-daemon-b"),
        );
        let reopened = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-crash-policy-b"),
            })
            .unwrap();
        let pending = reopened
            .pending_recovery_action_ref()
            .expect("crash must leave an outer pending recovery")
            .to_string();
        let recovered = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: pending,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(recovered.receipt.unwrap().status, ReceiptStatus::Recovered);
        assert_eq!(managed_tree_without_control_journals(temp.path()), baseline);
    }

    #[cfg(unix)]
    #[test]
    fn restart_recovery_rejects_replaced_parent_with_same_inode_hardlinked_children_prewrite() {
        let (temp, runtime, binding, adapter, request) = skill_install_fixture();
        let sealing_key = sha256("skill-parent-chain-daemon");
        let mut plane = ControlPlane::with_sealing_key(adapter, sealing_key.clone());
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-parent-chain-policy"),
            })
            .unwrap();
        let action_ref = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap()
            .action_ref
            .unwrap();
        let body_root = match &plane.actions.get(&action_ref).unwrap().domain_action {
            ProductionAction::SkillChange { materialized, .. } => match &materialized.body {
                ags_capability_governance::skill_adoption::MaterializedBodyDisposition::CreateExact(
                    body,
                ) => PathBuf::from(&body.root),
                other => panic!("expected exact body creation, got {other:?}"),
            },
            other => panic!("expected Skill action, got {other:?}"),
        };
        PANIC_SKILL_AFTER_MUTATION_KIND.with(|slot| *slot.borrow_mut() = Some("body"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = plane.apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            );
        }));
        assert!(crashed.is_err());
        drop(plane);

        let displaced = body_root.with_extension("displaced-parent");
        fs::rename(&body_root, &displaced).unwrap();
        fs::create_dir(&body_root).unwrap();
        for entry in fs::read_dir(&displaced).unwrap() {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file());
            fs::hard_link(entry.path(), body_root.join(entry.file_name())).unwrap();
        }
        let replacement_root_identity = {
            let metadata = fs::metadata(&body_root).unwrap();
            (metadata.dev(), metadata.ino())
        };
        let replacement_before = exact_tree(&body_root);
        for name in replacement_before.keys() {
            if name.is_empty() {
                continue;
            }
            let original = fs::metadata(displaced.join(name)).unwrap();
            let replacement = fs::metadata(body_root.join(name)).unwrap();
            assert_eq!(
                (original.dev(), original.ino()),
                (replacement.dev(), replacement.ino())
            );
        }

        let mut restarted =
            ControlPlane::with_sealing_key(ProductionEffectAdapter::new(&runtime), sealing_key);
        let reopened = restarted
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-parent-chain-restart-policy"),
            })
            .unwrap();
        let pending = reopened.pending_recovery_action_ref().unwrap().to_string();
        let result = restarted
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: pending,
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::RiskEscalated);
        assert_eq!(exact_tree(&body_root), replacement_before);
        let after = fs::metadata(&body_root).unwrap();
        assert_eq!((after.dev(), after.ino()), replacement_root_identity);
        assert!(temp.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn skill_third_party_drift_is_preserved_and_outer_receipt_is_risk() {
        let (temp, _runtime, binding, adapter, request) = skill_install_fixture();
        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-drift-daemon"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("skill-drift-policy"),
            })
            .unwrap();
        let decision = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
            .unwrap();
        let index_path = decision
            .plan
            .as_ref()
            .unwrap()
            .expected_write_paths
            .iter()
            .find(|path| path.ends_with("installed-skills.json"))
            .cloned()
            .unwrap();
        DRIFT_SKILL_AFTER_MUTATION_KIND.with(|slot| *slot.borrow_mut() = Some("index"));
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: decision.action_ref.unwrap(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::RiskEscalated);
        assert_eq!(
            fs::read(index_path).unwrap(),
            b"third-party-after-skill-mutation"
        );
        assert!(managed_tree_without_control_journals(temp.path())
            .values()
            .any(|(_, _, bytes)| bytes == b"third-party-after-skill-mutation"));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_skill_install_and_remove_close_with_route_verified_receipts() {
        let authority = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        let repository = temp.path().join("fixture-source");
        let source = repository.join("skill");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(repository.join("LICENSE"), "MIT fixture license\n").unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: control-plane-fixture\ndescription: Control-plane route fixture.\n---\n\n# Fixture\n",
        )
        .unwrap();
        let routing = temp.path().join("routing.yaml");
        fs::write(
            &routing,
            "summary: Verify the control-plane canonical Skill route.\nintent_tags: [control-plane-fixture]\npositive_examples: [Use the control-plane fixture]\nnegative_examples: [Do unrelated work]\n",
        )
        .unwrap();
        let binding = AuthenticatedBinding::mcp(
            "connection-a",
            "hermes",
            &authority,
            "workspace-a",
            sha256("facts-a"),
            "registry-a",
            "session-a",
            vec![authority.clone(), temp.path().canonicalize().unwrap()],
        );
        let adapter = ProductionEffectAdapter::new(&runtime);
        seed_canonical_skill_host(&authority, &runtime, temp.path());
        let mut request = SkillInstallRequest {
            context: OperationContext::default(),
            skill_id: "control-plane-fixture".to_string(),
            source: SkillSourceSpec {
                kind: SkillSourceKind::Local,
                uri: source.display().to_string(),
                requested_ref: None,
                tracking_ref: None,
                subdir: None,
            },
            routing_metadata: Some(routing.display().to_string()),
            target_hosts: vec!["codex".to_string()],
            update_policy: SkillUpdatePolicy::Notify,
            risk_acknowledgements: Vec::new(),
        };
        let prepared = ags_capability_governance::skill_adoption::plan_install(
            &adapter.skill_adoption_context(&binding),
            &canonical_skill_source(&request.source),
            request.routing_metadata.as_deref().map(Path::new),
            &request.target_hosts,
            ags_capability_governance::skill_adoption::UpdatePolicy::Notify,
        )
        .unwrap();
        request.risk_acknowledgements = prepared
            .risk_findings
            .iter()
            .map(|finding| finding.acknowledgement_id())
            .collect();

        let mut plane = ControlPlane::with_sealing_key(adapter, sha256("skill-route-daemon"));
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let action_ref = plane
            .decide(&session, OperationRequest::GovernSkillInstall(request))
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
        let receipt = result.receipt.as_ref().unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Succeeded);
        assert_eq!(
            receipt
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.pointer("/route_verification/passed"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(receipt
            .evidence
            .as_ref()
            .and_then(|evidence| { evidence.pointer("/route_verification/status/activations") })
            .and_then(serde_json::Value::as_array)
            .is_some_and(|activations| !activations.is_empty()));
        let routes = ags_capability_governance::skill_adoption::verify_adoption_routes(
            &runtime,
            temp.path(),
            "control-plane-fixture",
        )
        .unwrap();
        assert!(routes.verified_on_all_targets());

        let action_ref = plane
            .decide(
                &session,
                OperationRequest::GovernSkillRemove(SkillRemoveRequest {
                    context: OperationContext::default(),
                    skill_id: "control-plane-fixture".to_string(),
                }),
            )
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
        let receipt = result.receipt.as_ref().unwrap();
        assert_eq!(receipt.status, ReceiptStatus::Succeeded);
        assert_eq!(
            receipt
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.pointer("/route_verification/passed"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            receipt
                .evidence
                .as_ref()
                .and_then(|evidence| { evidence.pointer("/route_verification/status/registered") })
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn host_test_closes_only_with_command_bound_test_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        fs::write(
            workspace.path().join("config/agent-project-profile.yaml"),
            PROFILE,
        )
        .unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding(&root);
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("host-test-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("host-test-policy"),
            })
            .unwrap();
        let request = OperationRequest::Test(TestRequest {
            context: OperationContext::default(),
            profile: TestProfile::Smoke,
            executor: TestExecutor::Host,
        });
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
        let command = plan.execution.as_ref().unwrap();
        assert!(command.env.get("PATH").is_some_and(|path| !path.is_empty()));
        for key in ["TMPDIR", "TMP", "TEMP"] {
            assert!(command
                .env
                .get(key)
                .is_some_and(|path| Path::new(path).is_absolute()));
        }
        assert!(command.env["AGS_RUNTIME_HOME"].ends_with("target/.ags-test-runtime"));
        let output_digest = sha256("host-test-output");
        let test_receipt = ags_verification::TestReceipt {
            schema_version: "ags://schema/contract/v2/test-receipt".to_string(),
            profile: ags_verification::TestProfile::Smoke,
            canonical_workspace: root.display().to_string(),
            commit_hash: "unborn".to_string(),
            tree_hash: "unborn".to_string(),
            workspace_tree_hash: sha256("workspace-tree"),
            argv_hash: sha256(serde_json::to_vec(command).unwrap()),
            exit_code: 0,
            duration_ms: 1,
            output_digest: output_digest.clone(),
            output_bytes: 0,
            output_truncated: false,
            sandbox_backend: "host-adapter".to_string(),
            timeout_descendants_terminated: false,
            observed_write_set: Vec::new(),
            unexpected_write_set: Vec::new(),
            status: ags_verification::TestExecutionStatus::Succeeded,
            closed: true,
            source_rollback_performed: false,
        };
        let evidence_bytes = serde_json::to_vec(&test_receipt).unwrap();
        let receipt = HostOutcomeReceipt {
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
            observed_write_set: Vec::new(),
            artifacts: Vec::new(),
            evidence: Some(HostOutcomeEvidence {
                kind: HostEvidenceKind::TestReceipt,
                artifact: ContentAddressedArtifactRef {
                    uri: format!("memory://test-receipt/{action_ref}"),
                    sha256: sha256(&evidence_bytes),
                },
                content_hex: encode_hex(&evidence_bytes),
            }),
        };
        let mut tampered_test_receipt = test_receipt.clone();
        tampered_test_receipt.argv_hash = sha256("different-command");
        let tampered_evidence_bytes = serde_json::to_vec(&tampered_test_receipt).unwrap();
        let mut tampered_receipt = receipt.clone();
        tampered_receipt.evidence = Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::TestReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: format!("memory://test-receipt/tampered-{action_ref}"),
                sha256: sha256(&tampered_evidence_bytes),
            },
            content_hex: encode_hex(&tampered_evidence_bytes),
        });
        let tampered_bytes = serde_json::to_vec(&tampered_receipt).unwrap();
        assert_eq!(
            plane
                .apply(
                    &binding,
                    ApplyRequest {
                        action_ref: action_ref.clone(),
                        outcome: Some(AuthenticatedHostOutcome::from_artifact(
                            binding.clone(),
                            ContentAddressedArtifactRef {
                                uri: format!("memory://host-outcome/tampered-{action_ref}"),
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
        let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
        let result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: format!("memory://host-outcome/{action_ref}"),
                            sha256: sha256(&receipt_bytes),
                        },
                        receipt_bytes,
                    )),
                },
            )
            .unwrap();
        assert_eq!(
            result.receipt.as_ref().map(|receipt| receipt.status),
            Some(ReceiptStatus::Succeeded)
        );

        let failed_request = OperationRequest::Test(TestRequest {
            context: OperationContext::default(),
            profile: TestProfile::Smoke,
            executor: TestExecutor::Host,
        });
        let failed_decision = plane.decide(&session, failed_request).unwrap();
        let failed_action_ref = failed_decision.action_ref.unwrap();
        let failed_plan = failed_decision.plan.unwrap();
        let failed_grant = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: failed_action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        let failed_output_digest = sha256("host-test-failed-output");
        let failed_test_receipt = ags_verification::TestReceipt {
            schema_version: "ags://schema/contract/v2/test-receipt".to_string(),
            profile: ags_verification::TestProfile::Smoke,
            canonical_workspace: root.display().to_string(),
            commit_hash: "unborn".to_string(),
            tree_hash: "unborn".to_string(),
            workspace_tree_hash: sha256("workspace-tree-failed"),
            argv_hash: sha256(serde_json::to_vec(failed_plan.execution.as_ref().unwrap()).unwrap()),
            exit_code: 1,
            duration_ms: 1,
            output_digest: failed_output_digest.clone(),
            output_bytes: 0,
            output_truncated: false,
            sandbox_backend: "host-adapter".to_string(),
            timeout_descendants_terminated: false,
            observed_write_set: Vec::new(),
            unexpected_write_set: Vec::new(),
            status: ags_verification::TestExecutionStatus::Failed,
            closed: true,
            source_rollback_performed: false,
        };
        let failed_evidence = serde_json::to_vec(&failed_test_receipt).unwrap();
        let failed_host_receipt = HostOutcomeReceipt {
            schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
            action_ref: failed_action_ref.clone(),
            binding_hash: failed_plan.binding_hash.clone(),
            plan_hash: failed_plan.plan_hash.clone(),
            policy_hash: failed_plan.policy_hash.clone(),
            instruction_digest: plane.outcome_grants[&failed_action_ref]
                .instruction_digest
                .clone(),
            outcome_token: failed_grant.outcome_token.unwrap(),
            generation: failed_grant.outcome_generation.unwrap(),
            status: HostOutcomeStatus::Failed,
            output_digest: failed_output_digest,
            observed_write_set: Vec::new(),
            artifacts: Vec::new(),
            evidence: Some(HostOutcomeEvidence {
                kind: HostEvidenceKind::TestReceipt,
                artifact: ContentAddressedArtifactRef {
                    uri: format!("memory://test-receipt/{failed_action_ref}"),
                    sha256: sha256(&failed_evidence),
                },
                content_hex: encode_hex(&failed_evidence),
            }),
        };
        let failed_bytes = serde_json::to_vec(&failed_host_receipt).unwrap();
        let failed_result = plane
            .apply(
                &binding,
                ApplyRequest {
                    action_ref: failed_action_ref,
                    outcome: Some(AuthenticatedHostOutcome::from_artifact(
                        binding.clone(),
                        ContentAddressedArtifactRef {
                            uri: "memory://host-outcome/failed-test".to_string(),
                            sha256: sha256(&failed_bytes),
                        },
                        failed_bytes,
                    )),
                },
            )
            .unwrap();
        assert_eq!(failed_result.state, OperationState::Receipted);
        assert_eq!(
            failed_result.receipt.as_ref().map(|receipt| receipt.status),
            Some(ReceiptStatus::Failed)
        );
    }

    #[test]
    fn restart_recovers_init_even_when_profile_changes_project_facts_hash() {
        let (_workspace, runtime, root, _before_binding, _action_ref) = crashed_init();
        assert!(runtime.path().join("install-manifest.json").exists());

        let after_binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-after-profile"),
        );
        let mut restarted = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("restarted-daemon"),
        );
        let opened = restarted
            .open(OpenRequest {
                binding: after_binding.clone(),
                policy_hash: sha256("policy-after-restart"),
            })
            .unwrap();

        assert!(runtime.path().join("install-manifest.json").exists());
        restarted
            .apply(
                &after_binding,
                ApplyRequest {
                    action_ref: opened.pending_recovery_action_ref().unwrap().to_string(),
                    outcome: None,
                },
            )
            .unwrap();
        assert!(!runtime.path().join("install-manifest.json").exists());
    }

    #[test]
    fn journal_tamper_and_truncation_fail_closed_without_touching_postimages() {
        for truncate in [false, true] {
            let (_workspace, runtime, root, _binding, action_ref) = crashed_init();
            let journal_path = runtime
                .path()
                .join(format!(".ags-transaction-{action_ref}.json"));
            if truncate {
                fs::write(&journal_path, b"{").unwrap();
            } else {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
                value["phase"] = serde_json::Value::String("verified".to_string());
                fs::write(&journal_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
            }
            let after_binding = binding_with_runtime(
                &root,
                &runtime.path().canonicalize().unwrap(),
                sha256("facts-after-profile"),
            );
            let error = ProductionEffectAdapter::new(runtime.path())
                .recover_pending_transactions(&after_binding)
                .unwrap_err();
            assert!(
                matches!(
                    error.code.as_str(),
                    "transaction_journal_integrity_failed" | "transaction_journal_invalid"
                ),
                "unexpected code: {}",
                error.code
            );
            assert!(runtime.path().join("install-manifest.json").exists());
        }
    }

    #[test]
    fn recovery_escalates_drift_and_preserves_third_party_bytes() {
        let (_workspace, runtime, root, _binding, action_ref) = crashed_init();
        fs::write(runtime.path().join("install-manifest.json"), b"third-party").unwrap();
        let after_binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-after-drift"),
        );
        let error = ProductionEffectAdapter::new(runtime.path())
            .recover_pending_transactions(&after_binding)
            .unwrap_err();
        assert_eq!(error.code, "transaction_recovery_drift");
        assert_eq!(
            fs::read(runtime.path().join("install-manifest.json")).unwrap(),
            b"third-party"
        );

        let journal: TransactionJournal = serde_json::from_slice(
            &fs::read(
                runtime
                    .path()
                    .join(format!(".ags-transaction-{action_ref}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        journal.verify_integrity().unwrap();
        assert_eq!(journal.phase, JournalPhase::RiskEscalated);

        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("risk-blocks-open-daemon"),
        );
        let error = plane
            .open(OpenRequest {
                binding: after_binding,
                policy_hash: sha256("risk-blocks-open-policy"),
            })
            .unwrap_err();
        assert_eq!(error.code, "pending_transaction_inspection_failed");
        assert!(error
            .detail
            .contains("transaction_recovery_risk_requires_operator"));
    }

    #[test]
    fn recovery_preflight_rejects_invalid_write_before_restoring_any_valid_write() {
        let (_workspace, runtime, _root, binding, action_ref) = crashed_init();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let (_, _, journal) = adapter.load_journal(&action_ref).unwrap();
        assert!(journal.ordered_writes.len() >= 2);
        let invalid = PathBuf::from(&journal.ordered_writes[0].path);
        let valid = PathBuf::from(&journal.ordered_writes.last().unwrap().path);
        let valid_bytes = fs::read(&valid).unwrap();
        let valid_metadata = fs::metadata(&valid).unwrap();
        fs::write(&invalid, b"third-party-invalid-first-write").unwrap();

        let error = adapter.recover_pending_transactions(&binding).unwrap_err();
        assert_eq!(error.code, "transaction_recovery_drift");
        assert_eq!(fs::read(&valid).unwrap(), valid_bytes);
        let after = fs::metadata(&valid).unwrap();
        assert_eq!(
            (after.dev(), after.ino()),
            (valid_metadata.dev(), valid_metadata.ino()),
            "Phase A must classify every write before Phase B mutates any business path"
        );
        assert_eq!(
            fs::read(&invalid).unwrap(),
            b"third-party-invalid-first-write"
        );
    }

    #[test]
    fn restart_recovery_rejects_identical_postimage_with_replaced_inode() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-a"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("post-inode-daemon"),
        );
        let action_ref = apply_without_commit(&mut plane, &binding);
        let (_, _, journal) = plane.adapter.load_journal(&action_ref).unwrap();
        let target = PathBuf::from(&journal.ordered_writes.last().unwrap().path);
        let original = target.with_extension("ags-original-inode");
        let postimage = fs::read(&target).unwrap();
        fs::rename(&target, &original).unwrap();
        fs::write(&target, &postimage).unwrap();
        drop(plane);

        let error = ProductionEffectAdapter::new(runtime.path())
            .recover_pending_transactions(&binding)
            .unwrap_err();
        assert_eq!(error.code, "transaction_recovery_drift");
        assert_eq!(fs::read(&target).unwrap(), postimage);
        assert_eq!(fs::read(&original).unwrap(), postimage);
        let (_, _, journal) = ProductionEffectAdapter::new(runtime.path())
            .load_journal(&action_ref)
            .unwrap();
        assert_eq!(journal.phase, JournalPhase::RiskEscalated);
    }

    #[test]
    fn durable_commit_marker_is_authoritative_after_restart() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let before_binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-before-profile"),
        );
        let mut first = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("first-daemon"),
        );
        let session = first
            .open(OpenRequest {
                binding: before_binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let action_ref = first
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        let result = first
            .apply(
                &before_binding,
                ApplyRequest {
                    action_ref: action_ref.clone(),
                    outcome: None,
                },
            )
            .unwrap();
        assert_eq!(result.state, OperationState::Receipted);
        assert!(runtime
            .path()
            .join(format!(".ags-transaction-{action_ref}.commit"))
            .exists());
        drop(first);

        let after_binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-after-profile"),
        );
        let restarted = ProductionEffectAdapter::new(runtime.path());
        restarted
            .recover_pending_transactions(&after_binding)
            .unwrap();
        restarted
            .recover_pending_transactions(&after_binding)
            .unwrap();
        assert!(runtime.path().join("install-manifest.json").exists());
    }

    #[test]
    fn journal_directory_fsync_failure_happens_before_user_mutation() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let binding = binding_with_runtime(
            &root,
            &runtime.path().canonicalize().unwrap(),
            sha256("facts-a"),
        );
        let mut plane = ControlPlane::with_sealing_key(
            ProductionEffectAdapter::new(runtime.path()),
            sha256("fsync-failure-daemon"),
        );
        let session = plane
            .open(OpenRequest {
                binding: binding.clone(),
                policy_hash: sha256("policy-a"),
            })
            .unwrap();
        let action_ref = plane
            .decide(
                &session,
                OperationRequest::Setup(SetupRequest {
                    context: OperationContext::default(),
                    approved_hosts: vec!["hermes".to_string()],
                }),
            )
            .unwrap()
            .action_ref
            .unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.with(|flag| flag.set(true));
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
            result.receipt.as_ref().map(|receipt| receipt.status),
            Some(ReceiptStatus::RiskEscalated)
        );
        assert_eq!(result.reason_code.as_deref(), Some("unexpected_write_set"));
        let receipt = result.receipt.unwrap();
        assert_eq!(receipt.observed_write_set.len(), 1);
        assert!(receipt.observed_write_set[0].contains("/.ags-transaction-"));
        assert!(Path::new(&receipt.observed_write_set[0]).is_file());
        assert!(!runtime.path().join("install-manifest.json").exists());
    }

    #[test]
    fn workspace_root_substitution_is_detected_before_create() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let binding = binding(&root);
        let target = adapter
            .anchored_target(&binding, &root.join("created.txt"))
            .unwrap();
        let moved = outer.path().join("workspace-moved");
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();

        let error = anchored_write(&target, None, b"content").unwrap_err();
        assert_eq!(error.code, "transaction_root_binding_changed");
        assert!(!root.join("created.txt").exists());
        assert!(!moved.join("created.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn basename_symlink_substitution_is_rejected_for_create_replace_and_delete() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let binding = binding(&root);
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"outside").unwrap();

        let create = adapter
            .anchored_target(&binding, &root.join("create.txt"))
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("create.txt")).unwrap();
        assert!(anchored_write(&create, None, b"new").is_err());

        fs::write(root.join("replace.txt"), b"before").unwrap();
        let replace = adapter
            .anchored_target(&binding, &root.join("replace.txt"))
            .unwrap();
        fs::remove_file(root.join("replace.txt")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("replace.txt")).unwrap();
        assert!(anchored_write(&replace, Some(b"before"), b"after").is_err());

        fs::write(root.join("delete.txt"), b"before").unwrap();
        let delete = adapter
            .anchored_target(&binding, &root.join("delete.txt"))
            .unwrap();
        fs::remove_file(root.join("delete.txt")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("delete.txt")).unwrap();
        assert!(anchored_delete(&delete, b"before").is_err());
        assert_eq!(fs::read(outside.path()).unwrap(), b"outside");
    }

    #[test]
    fn basename_regular_file_substitution_is_rejected_even_with_identical_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let binding = binding(&root);

        fs::write(root.join("replace.txt"), b"before").unwrap();
        let replace = adapter
            .anchored_target(&binding, &root.join("replace.txt"))
            .unwrap();
        fs::rename(root.join("replace.txt"), root.join("replace-original.txt")).unwrap();
        fs::write(root.join("replace.txt"), b"before").unwrap();
        let error = anchored_write(&replace, Some(b"before"), b"after").unwrap_err();
        assert_eq!(error.code, "transaction_target_binding_changed");
        assert_eq!(fs::read(root.join("replace.txt")).unwrap(), b"before");
        assert_eq!(
            fs::read(root.join("replace-original.txt")).unwrap(),
            b"before"
        );

        fs::write(root.join("delete.txt"), b"before").unwrap();
        let delete = adapter
            .anchored_target(&binding, &root.join("delete.txt"))
            .unwrap();
        fs::rename(root.join("delete.txt"), root.join("delete-original.txt")).unwrap();
        fs::write(root.join("delete.txt"), b"before").unwrap();
        let error = anchored_delete(&delete, b"before").unwrap_err();
        assert_eq!(error.code, "transaction_target_binding_changed");
        assert_eq!(fs::read(root.join("delete.txt")).unwrap(), b"before");
        assert_eq!(
            fs::read(root.join("delete-original.txt")).unwrap(),
            b"before"
        );
    }

    #[test]
    fn staged_basename_substitution_preserves_unknown_inode_for_create_replace_and_delete() {
        let run = |initial: Option<&[u8]>, replacement: Option<&[u8]>, different: bool| {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path().canonicalize().unwrap();
            let runtime = tempfile::tempdir().unwrap();
            let adapter = ProductionEffectAdapter::new(runtime.path());
            let binding = binding(&root);
            let path = root.join("target.txt");
            if let Some(initial) = initial {
                fs::write(&path, initial).unwrap();
            }
            let target = adapter.anchored_target(&binding, &path).unwrap();
            if different {
                SUBSTITUTE_NEXT_STAGE_DIFFERENT.with(|flag| flag.set(true));
            } else {
                SUBSTITUTE_NEXT_STAGE.with(|flag| flag.set(true));
            }
            let error = match replacement {
                Some(replacement) => anchored_write(&target, initial, replacement).unwrap_err(),
                None => anchored_delete(&target, initial.unwrap()).unwrap_err(),
            };
            assert_eq!(error.code, "transaction_stage_binding_changed");
            assert_eq!(fs::read(&path).ok().as_deref(), initial);

            let staged_content = replacement.unwrap_or(&[]);
            assert_eq!(
                fs::read(root.join(".ags-test-original-stage")).unwrap(),
                staged_content
            );
            let substitutes = fs::read_dir(&root)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|name| name.starts_with(".ags-txn-"))
                })
                .collect::<Vec<_>>();
            assert_eq!(substitutes.len(), 1);
            assert_eq!(
                error.observed_write_set,
                vec![substitutes[0].display().to_string()]
            );
            assert_eq!(
                fs::read(&substitutes[0]).unwrap(),
                if different {
                    b"attacker-different-stage".as_slice()
                } else {
                    staged_content
                }
            );
        };

        for different in [false, true] {
            run(None, Some(b"created"), different);
            run(Some(b"before"), Some(b"after"), different);
            run(Some(b"before"), None, different);
        }
    }

    #[test]
    fn quarantine_open_failure_preserves_validated_entry_and_reports_residue() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let binding = binding(&root);
        let path = root.join("target.txt");
        fs::write(&path, b"before").unwrap();
        let target = adapter.anchored_target(&binding, &path).unwrap();
        FAIL_NEXT_QUARANTINE_OPEN.with(|flag| flag.set(true));
        let error = anchored_write(&target, Some(b"before"), b"after").unwrap_err();
        assert_eq!(error.code, "transaction_quarantine_open_failed");
        assert_eq!(fs::read(&path).unwrap(), b"after");
        assert_eq!(error.observed_write_set.len(), 2);
        assert!(error
            .observed_write_set
            .iter()
            .any(|observed| observed == &path.display().to_string()));
        let residue = error
            .observed_write_set
            .iter()
            .map(PathBuf::from)
            .find(|observed| {
                observed
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with(".ags-quarantine-"))
            })
            .unwrap();
        assert!(residue
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(".ags-quarantine-")));
        assert_eq!(fs::read(residue).unwrap(), b"before");
    }

    #[cfg(unix)]
    #[test]
    fn fifo_target_is_rejected_without_blocking_during_plan_sealing() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let fifo = root.join("target.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let runtime = tempfile::tempdir().unwrap();
        let adapter = ProductionEffectAdapter::new(runtime.path());
        let binding = binding(&root);
        let started = std::time::Instant::now();
        let error = adapter.anchored_target(&binding, &fifo).unwrap_err();
        assert_eq!(error.code, "transaction_target_not_regular");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn release_scanner_rejects_expected_plus_one_during_enumeration() {
        let directory = tempfile::tempdir().unwrap();
        for name in RELEASE_PAYLOAD_NAMES {
            fs::write(directory.path().join(name), name.as_bytes()).unwrap();
        }
        fs::write(directory.path().join("unsealed-extra"), b"extra").unwrap();
        let error = scan_release_directory(directory.path(), &RELEASE_PAYLOAD_NAMES).unwrap_err();
        assert_eq!(error.code, "release_directory_entry_budget_exceeded");
    }

    #[cfg(unix)]
    #[test]
    fn release_scanner_rejects_same_fd_same_inode_same_size_rewrite() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempfile::tempdir().unwrap();
        for name in RELEASE_PAYLOAD_NAMES {
            fs::write(directory.path().join(name), b"original").unwrap();
        }
        let target = directory.path().join("ags");
        let before = fs::metadata(&target).unwrap();
        RELEASE_AFTER_READ_REWRITE.with(|slot| {
            *slot.borrow_mut() = Some((target.clone(), b"tampered".to_vec()));
        });
        let error = scan_release_directory(directory.path(), &RELEASE_PAYLOAD_NAMES).unwrap_err();
        RELEASE_AFTER_READ_REWRITE.with(|slot| {
            slot.borrow_mut().take();
        });
        assert_eq!(error.code, "release_member_changed_during_read");
        let after = fs::metadata(target).unwrap();
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.len(), after.len());
    }

    #[test]
    fn journal_recovery_rejects_entry_budget_during_enumeration() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_root = runtime.path().canonicalize().unwrap();
        for index in 0..300 {
            fs::write(runtime_root.join(format!("unrelated-{index:03}")), b"x").unwrap();
        }
        let adapter = ProductionEffectAdapter::with_host_home(&runtime_root, &root);
        let error = adapter
            .recover_pending_transactions(&binding_with_runtime(
                &root,
                &runtime_root,
                sha256("facts-a"),
            ))
            .unwrap_err();
        assert_eq!(error.code, "transaction_journal_entry_budget_exceeded");
    }
}
