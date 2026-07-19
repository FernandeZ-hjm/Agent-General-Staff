//! Typed request-governance contracts for AGS.
//!
//! Natural-language interpretation belongs to the host. This crate accepts a
//! host-produced [`HostRouteProposal`], validates its closed fields, and
//! derives stable hashes and route contracts. It never reads natural language,
//! launches processes, or writes files.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const HOST_ROUTE_PROPOSAL_SCHEMA_VERSION: &str = "0.3.0-host-route-proposal";
pub const ROUTE_RESOLUTION_SCHEMA_VERSION: &str = "0.3.0-route-resolution";

/// Shared foreground status used by preflight, routing, apply, runner and
/// receipts. It is deliberately separate from task level and permission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GovernanceStatus {
    Ok,
    NeedsUserDecision,
    BlockedByPolicy,
    RiskEscalated,
    DoneWithReceipt,
    AdvisoryNoMutation,
    HostExecutionRequired,
}

impl GovernanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::NeedsUserDecision => "NEEDS_USER_DECISION",
            Self::BlockedByPolicy => "BLOCKED_BY_POLICY",
            Self::RiskEscalated => "RISK_ESCALATED",
            Self::DoneWithReceipt => "DONE_WITH_RECEIPT",
            Self::AdvisoryNoMutation => "ADVISORY_NO_MUTATION",
            Self::HostExecutionRequired => "HOST_EXECUTION_REQUIRED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalPhase {
    DirectResponse,
    SolutionFormation,
    Execution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolutionState {
    NotRequired,
    Open,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAuthority {
    None,
    DirectEdit,
    TaskCardHandoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliCapabilityId {
    TaskCompile,
    #[serde(alias = "task_execute")]
    TaskPrepareExecution,
    TaskValidate,
    PolicyResolve,
    ProjectVerify,
    SkillTagsVerify,
    ReceiptVerify,
}

impl CliCapabilityId {
    pub fn is_handoff_capability(self) -> bool {
        matches!(self, Self::TaskCompile | Self::TaskPrepareExecution)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedCliInput {
    ConfirmedHandoffContract { content: String },
    TaskCard { content: String },
    Receipt { content: String },
    Empty,
}

/// Legacy closed intent taxonomy retained for registry/catalog migration.
/// New route proposals select an exact `skill_id`; this enum is never a route
/// authority by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "category", content = "demand", rename_all = "snake_case")]
pub enum SkillDemand {
    Engineering(EngineeringDemand),
    Knowledge(KnowledgeDemand),
    Lark(LarkDemand),
    Content(ContentDemand),
    Personal(PersonalDemand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringDemand {
    Debugging,
    ModuleDesign,
    DomainModeling,
    ArchitectureImprovement,
    SystemArchitecture,
    Brainstorming,
    Prototype,
    PlanGrilling,
    DocsGroundedGrilling,
    ImplementationPlanning,
    ApprovedPlanExecution,
    TestDrivenDevelopment,
    CompletionVerification,
    CodeReview,
    ReviewRequest,
    SkillAuthoring,
    PrdAuthoring,
    IssueSlicing,
    IssueTriage,
    DecisionMapping,
    MergeConflictResolution,
    ContextHandoff,
    BranchReview,
    DeliveryReporting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeDemand {
    ConversationMemoryRecall,
    VaultKnowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LarkDemand {
    Mail,
    Calendar,
    Document,
    Spreadsheet,
    Base,
    Messaging,
    Approval,
    Task,
    Wiki,
    Minutes,
    MeetingHistory,
    MeetingAgent,
    Drive,
    Contact,
    Attendance,
    Okr,
    Event,
    Slides,
    Whiteboard,
    Markdown,
    AppDevelopment,
    OpenApiExploration,
    MeetingSummaryWorkflow,
    StandupReportWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentDemand {
    IndustryArticleWriting,
    IndustryTopicPlanning,
    TechnologyCommentary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalDemand {
    LiuYaoDivination,
    TarotDivination,
}

impl SkillDemand {
    pub fn all() -> &'static [SkillDemand] {
        use ContentDemand::*;
        use EngineeringDemand::*;
        use KnowledgeDemand::*;
        use LarkDemand::*;
        use PersonalDemand::*;

        const ALL: &[SkillDemand] = &[
            SkillDemand::Engineering(Debugging),
            SkillDemand::Engineering(ModuleDesign),
            SkillDemand::Engineering(DomainModeling),
            SkillDemand::Engineering(ArchitectureImprovement),
            SkillDemand::Engineering(SystemArchitecture),
            SkillDemand::Engineering(Brainstorming),
            SkillDemand::Engineering(Prototype),
            SkillDemand::Engineering(PlanGrilling),
            SkillDemand::Engineering(DocsGroundedGrilling),
            SkillDemand::Engineering(ImplementationPlanning),
            SkillDemand::Engineering(ApprovedPlanExecution),
            SkillDemand::Engineering(TestDrivenDevelopment),
            SkillDemand::Engineering(CompletionVerification),
            SkillDemand::Engineering(CodeReview),
            SkillDemand::Engineering(ReviewRequest),
            SkillDemand::Engineering(SkillAuthoring),
            SkillDemand::Engineering(PrdAuthoring),
            SkillDemand::Engineering(IssueSlicing),
            SkillDemand::Engineering(IssueTriage),
            SkillDemand::Engineering(DecisionMapping),
            SkillDemand::Engineering(MergeConflictResolution),
            SkillDemand::Engineering(ContextHandoff),
            SkillDemand::Engineering(BranchReview),
            SkillDemand::Engineering(DeliveryReporting),
            SkillDemand::Knowledge(ConversationMemoryRecall),
            SkillDemand::Knowledge(VaultKnowledge),
            SkillDemand::Lark(Mail),
            SkillDemand::Lark(Calendar),
            SkillDemand::Lark(Document),
            SkillDemand::Lark(Spreadsheet),
            SkillDemand::Lark(Base),
            SkillDemand::Lark(Messaging),
            SkillDemand::Lark(Approval),
            SkillDemand::Lark(Task),
            SkillDemand::Lark(Wiki),
            SkillDemand::Lark(Minutes),
            SkillDemand::Lark(MeetingHistory),
            SkillDemand::Lark(MeetingAgent),
            SkillDemand::Lark(Drive),
            SkillDemand::Lark(Contact),
            SkillDemand::Lark(Attendance),
            SkillDemand::Lark(Okr),
            SkillDemand::Lark(Event),
            SkillDemand::Lark(Slides),
            SkillDemand::Lark(Whiteboard),
            SkillDemand::Lark(Markdown),
            SkillDemand::Lark(AppDevelopment),
            SkillDemand::Lark(OpenApiExploration),
            SkillDemand::Lark(MeetingSummaryWorkflow),
            SkillDemand::Lark(StandupReportWorkflow),
            SkillDemand::Content(IndustryArticleWriting),
            SkillDemand::Content(IndustryTopicPlanning),
            SkillDemand::Content(TechnologyCommentary),
            SkillDemand::Personal(LiuYaoDivination),
            SkillDemand::Personal(TarotDivination),
        ];
        ALL
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTarget {
    pub skill_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineCliTarget {
    pub capability: CliCapabilityId,
    pub input: TypedCliInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalTarget {
    DirectResponse {},
    Skill(SkillTarget),
    MachineCli(MachineCliTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRouteProposal {
    pub schema_version: String,
    pub request_fingerprint: String,
    pub phase: ProposalPhase,
    pub solution_state: SolutionState,
    pub execution_authority: ExecutionAuthority,
    pub scope_hash: String,
    pub targets: Vec<ProposalTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalError {
    pub code: String,
    pub field: String,
    pub message: String,
}

impl ProposalError {
    pub fn new(code: &'static str, field: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            field: field.to_string(),
            message: message.into(),
        }
    }
}

/// Validate the exact typed input accepted by one closed CLI capability.
///
/// This is deliberately pure so the read-only route can reject malformed
/// capability/input pairs before it creates a held action. The apply side calls
/// it again as a defense-in-depth invariant before constructing fixed argv.
pub fn validate_machine_input(
    capability: CliCapabilityId,
    input: &TypedCliInput,
) -> Result<(), ProposalError> {
    let kind_matches = matches!(
        (capability, input),
        (
            CliCapabilityId::TaskCompile,
            TypedCliInput::ConfirmedHandoffContract { .. }
        ) | (
            CliCapabilityId::TaskPrepareExecution
                | CliCapabilityId::TaskValidate
                | CliCapabilityId::PolicyResolve
                | CliCapabilityId::SkillTagsVerify,
            TypedCliInput::TaskCard { .. }
        ) | (CliCapabilityId::ProjectVerify, TypedCliInput::Empty)
            | (
                CliCapabilityId::ReceiptVerify,
                TypedCliInput::Receipt { .. }
            )
    );
    if !kind_matches {
        return Err(ProposalError::new(
            "machine_input_kind_mismatch",
            "targets.input.kind",
            format!("{capability:?} does not accept this typed input kind"),
        ));
    }

    let content = match input {
        TypedCliInput::ConfirmedHandoffContract { content }
        | TypedCliInput::TaskCard { content }
        | TypedCliInput::Receipt { content } => Some(content),
        TypedCliInput::Empty => None,
    };
    if content.is_some_and(|value| value.trim().is_empty()) {
        return Err(ProposalError::new(
            "machine_input_empty",
            "targets.input.content",
            "typed machine input content must not be empty",
        ));
    }
    Ok(())
}

/// Validate the closed host proposal without interpreting its originating text.
pub fn validate_proposal(proposal: &HostRouteProposal) -> Result<(), Vec<ProposalError>> {
    let mut errors = Vec::new();
    if proposal.schema_version != HOST_ROUTE_PROPOSAL_SCHEMA_VERSION {
        errors.push(ProposalError::new(
            "unsupported_schema_version",
            "schema_version",
            format!(
                "expected {HOST_ROUTE_PROPOSAL_SCHEMA_VERSION}, got {}",
                proposal.schema_version
            ),
        ));
    }
    if !is_hash_like(&proposal.request_fingerprint) {
        errors.push(ProposalError::new(
            "invalid_request_fingerprint",
            "request_fingerprint",
            "request_fingerprint must be a non-empty digest-like value",
        ));
    }
    if !is_hash_like(&proposal.scope_hash) {
        errors.push(ProposalError::new(
            "invalid_scope_hash",
            "scope_hash",
            "scope_hash must be a non-empty digest-like value",
        ));
    }
    if proposal.targets.len() > 2 {
        errors.push(ProposalError::new(
            "invalid_target_count",
            "targets",
            "targets may contain at most one skill and at most one machine action",
        ));
    }

    let direct_count = proposal
        .targets
        .iter()
        .filter(|target| matches!(target, ProposalTarget::DirectResponse {}))
        .count();
    let skill_count = proposal
        .targets
        .iter()
        .filter(|target| matches!(target, ProposalTarget::Skill(_)))
        .count();
    let machine_count = proposal
        .targets
        .iter()
        .filter(|target| matches!(target, ProposalTarget::MachineCli(_)))
        .count();
    if direct_count > 0 && proposal.targets.len() != 1 {
        errors.push(ProposalError::new(
            "direct_response_not_exclusive",
            "targets",
            "direct_response must be the only target",
        ));
    }
    if direct_count > 1 || skill_count > 1 || machine_count > 1 {
        errors.push(ProposalError::new(
            "duplicate_target_kind",
            "targets",
            "at most one direct response, one skill and one machine target are allowed",
        ));
    }

    for target in &proposal.targets {
        match target {
            ProposalTarget::Skill(skill) => {
                if !is_stable_identifier(&skill.skill_id) {
                    errors.push(ProposalError::new(
                        "invalid_skill_id",
                        "targets.skill_id",
                        "skill_id must be a stable canonical identifier",
                    ));
                }
                if !is_hash_like(&skill.snapshot_hash) {
                    errors.push(ProposalError::new(
                        "invalid_snapshot_hash",
                        "targets.snapshot_hash",
                        "skill targets must bind the host snapshot hash",
                    ));
                }
                if skill
                    .entrypoint
                    .as_deref()
                    .is_some_and(|entrypoint| !is_stable_identifier(entrypoint))
                {
                    errors.push(ProposalError::new(
                        "invalid_skill_entrypoint",
                        "targets.entrypoint",
                        "entrypoint must be a stable identifier",
                    ));
                }
            }
            ProposalTarget::MachineCli(machine) => {
                if let Err(error) = validate_machine_input(machine.capability, &machine.input) {
                    errors.push(error);
                }
                if machine.capability.is_handoff_capability()
                    && proposal.execution_authority != ExecutionAuthority::TaskCardHandoff
                {
                    errors.push(ProposalError::new(
                        "handoff_authority_required",
                        "execution_authority",
                        "task compilation/preparation requires task_card_handoff authority",
                    ));
                }
            }
            ProposalTarget::DirectResponse {} => {}
        }
    }

    match proposal.phase {
        ProposalPhase::DirectResponse => {
            if direct_count != 1
                || proposal.solution_state != SolutionState::NotRequired
                || proposal.execution_authority != ExecutionAuthority::None
            {
                errors.push(ProposalError::new(
                    "direct_response_phase_mismatch",
                    "phase",
                    "direct_response phase requires one direct response, not_required solution and no execution authority",
                ));
            }
        }
        ProposalPhase::SolutionFormation => {
            if direct_count > 0 {
                errors.push(ProposalError::new(
                    "direct_response_phase_only",
                    "targets",
                    "DirectResponse is only valid in the direct_response phase",
                ));
            }
            if proposal.solution_state != SolutionState::Open
                || proposal.execution_authority != ExecutionAuthority::None
                || machine_count > 0
            {
                errors.push(ProposalError::new(
                    "solution_phase_mismatch",
                    "phase",
                    "solution_formation requires an open solution, no authority and no machine action",
                ));
            }
        }
        ProposalPhase::Execution => {
            if direct_count > 0 {
                errors.push(ProposalError::new(
                    "direct_response_phase_only",
                    "targets",
                    "DirectResponse is only valid in the direct_response phase",
                ));
            }
            if proposal.execution_authority != ExecutionAuthority::None
                && proposal.solution_state != SolutionState::Confirmed
            {
                errors.push(ProposalError::new(
                    "confirmed_solution_required",
                    "solution_state",
                    "authorized execution requires a confirmed solution",
                ));
            }
        }
    }

    if proposal.execution_authority == ExecutionAuthority::DirectEdit && machine_count > 0 {
        errors.push(ProposalError::new(
            "direct_edit_is_host_native",
            "targets",
            "direct_edit cannot carry a MachineCli target",
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn proposal_hash(proposal: &HostRouteProposal) -> String {
    let bytes = serde_json::to_vec(proposal).unwrap_or_default();
    sha256(&bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedTarget {
    DirectResponse,
    Skill {
        skill_id: String,
        invoke_hint: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        entrypoint: Option<String>,
    },
    HostNativeDirectEdit {
        action_id: String,
    },
    ServerHeldAction {
        action_id: String,
        action_kind: ServerHeldActionKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        capability: Option<CliCapabilityId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerHeldActionKind {
    MachineCli,
    SkillOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionLeaseEvidence {
    pub lease_id: String,
    pub decision_id: String,
    pub proposal_hash: String,
    pub scope_hash: String,
    pub host: String,
    pub target: String,
    pub registry_hash: String,
    pub snapshot_hash: String,
    pub policy_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteResolution {
    pub schema_version: String,
    pub governance_status: GovernanceStatus,
    pub decision_id: String,
    pub proposal_hash: String,
    pub host: String,
    pub target: String,
    pub resolved_targets: Vec<ResolvedTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<DecisionLeaseEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ProposalError>,
}

fn is_hash_like(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_')
        })
}

fn is_stable_identifier(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct() -> HostRouteProposal {
        HostRouteProposal {
            schema_version: HOST_ROUTE_PROPOSAL_SCHEMA_VERSION.to_string(),
            request_fingerprint: "sha256:req".to_string(),
            phase: ProposalPhase::DirectResponse,
            solution_state: SolutionState::NotRequired,
            execution_authority: ExecutionAuthority::None,
            scope_hash: "sha256:scope".to_string(),
            targets: vec![ProposalTarget::DirectResponse {}],
        }
    }

    #[test]
    fn direct_response_is_exclusive() {
        let mut proposal = direct();
        proposal.targets.push(ProposalTarget::Skill(SkillTarget {
            skill_id: "codebase-design".to_string(),
            entrypoint: None,
            snapshot_hash: "sha256:snapshot".to_string(),
        }));
        let errors = validate_proposal(&proposal).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "direct_response_not_exclusive"));
    }

    #[test]
    fn direct_response_target_is_rejected_outside_direct_response_phase() {
        for phase in [ProposalPhase::SolutionFormation, ProposalPhase::Execution] {
            let mut proposal = direct();
            proposal.phase = phase;
            proposal.solution_state = match phase {
                ProposalPhase::SolutionFormation => SolutionState::Open,
                ProposalPhase::Execution => SolutionState::Confirmed,
                ProposalPhase::DirectResponse => unreachable!(),
            };
            let errors = validate_proposal(&proposal).unwrap_err();
            assert!(errors
                .iter()
                .any(|error| error.code == "direct_response_phase_only"));
        }
    }

    #[test]
    fn direct_edit_is_host_native() {
        let proposal = HostRouteProposal {
            schema_version: HOST_ROUTE_PROPOSAL_SCHEMA_VERSION.to_string(),
            request_fingerprint: "sha256:req".to_string(),
            phase: ProposalPhase::Execution,
            solution_state: SolutionState::Confirmed,
            execution_authority: ExecutionAuthority::DirectEdit,
            scope_hash: "sha256:scope".to_string(),
            targets: vec![ProposalTarget::MachineCli(MachineCliTarget {
                capability: CliCapabilityId::ProjectVerify,
                input: TypedCliInput::Empty,
            })],
        };
        assert!(validate_proposal(&proposal)
            .unwrap_err()
            .iter()
            .any(|error| error.code == "direct_edit_is_host_native"));
    }

    #[test]
    fn direct_edit_may_use_only_the_host_native_action() {
        let proposal = HostRouteProposal {
            schema_version: HOST_ROUTE_PROPOSAL_SCHEMA_VERSION.to_string(),
            request_fingerprint: "sha256:req".to_string(),
            phase: ProposalPhase::Execution,
            solution_state: SolutionState::Confirmed,
            execution_authority: ExecutionAuthority::DirectEdit,
            scope_hash: "sha256:scope".to_string(),
            targets: Vec::new(),
        };
        assert!(validate_proposal(&proposal).is_ok());
    }

    #[test]
    fn task_execute_is_deserialize_only_alias() {
        let legacy: CliCapabilityId = serde_json::from_str("\"task_execute\"").unwrap();
        assert_eq!(legacy, CliCapabilityId::TaskPrepareExecution);
        assert_eq!(
            serde_json::to_string(&legacy).unwrap(),
            "\"task_prepare_execution\""
        );
    }

    #[test]
    fn machine_capability_requires_its_exact_input_kind() {
        let mut proposal = direct();
        proposal.phase = ProposalPhase::Execution;
        proposal.solution_state = SolutionState::Confirmed;
        proposal.execution_authority = ExecutionAuthority::TaskCardHandoff;
        proposal.targets = vec![ProposalTarget::MachineCli(MachineCliTarget {
            capability: CliCapabilityId::TaskCompile,
            input: TypedCliInput::TaskCard {
                content: "## 任务卡\n".to_string(),
            },
        })];
        let errors = validate_proposal(&proposal).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "machine_input_kind_mismatch"));
    }

    #[test]
    fn typed_machine_content_must_not_be_empty() {
        let error = validate_machine_input(
            CliCapabilityId::ReceiptVerify,
            &TypedCliInput::Receipt {
                content: "  \n".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "machine_input_empty");
    }

    #[test]
    fn proposal_hash_is_stable() {
        let proposal = direct();
        assert_eq!(proposal_hash(&proposal), proposal_hash(&proposal));
        assert!(proposal_hash(&proposal).starts_with("sha256:"));
    }
}
