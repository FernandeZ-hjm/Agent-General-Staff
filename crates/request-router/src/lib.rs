//! The single AGS natural-language routing node.
//!
//! This module is deliberately pure and stateless. The host supplies the
//! current request plus structured conversation evidence; AGS returns one
//! [`RequestDecision`]. No compiler, policy, gate, CLI adapter, or skill
//! resolver is allowed to parse natural language again.

use serde::{Deserialize, Serialize};

pub const REQUEST_DECISION_SCHEMA_VERSION: &str = "0.2.8-request-decision";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext<'a> {
    pub request: &'a str,
    pub approved_contract: bool,
    pub confirmed_handoff_contract: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDecision {
    pub schema_version: String,
    pub status: DecisionStatus,
    pub targets: Vec<RouteTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionStatus {
    Ready,
    InsufficientContext { missing: Vec<RequiredInput> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredInput {
    UserRequest,
    ApprovedContract,
    ConfirmedHandoffContract,
    ContentKind,
    DivinationMethod,
    TaskCard,
    Receipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteTarget {
    DirectResponse,
    Skill {
        demand: SkillDemand,
    },
    MachineCli {
        capability: CliCapabilityId,
        input: TypedCliInput,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliCapabilityId {
    TaskCompile,
    TaskExecute,
    TaskValidate,
    PolicyResolve,
    ProjectVerify,
    SkillTagsVerify,
    ReceiptVerify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedCliInput {
    RequestText { text: String },
    TaskCard { content: String },
    Target { path: String },
    Empty,
}

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

pub fn route_request(context: RequestContext<'_>) -> RequestDecision {
    let request = context.request.trim();
    if request.is_empty() {
        return insufficient(vec![RequiredInput::UserRequest]);
    }

    if is_canonical_task_card(request) {
        return ready(vec![RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskExecute,
            input: TypedCliInput::TaskCard {
                content: request.to_string(),
            },
        }]);
    }

    // Output-only transformations terminate here. They cannot be widened by a
    // skill keyword or a machine-action phrase later in the same sentence.
    if is_bounded_content_transform(request) {
        return ready(vec![RouteTarget::DirectResponse]);
    }

    if is_task_card_handoff(request) {
        let mut missing = Vec::new();
        if !context.approved_contract {
            missing.push(RequiredInput::ApprovedContract);
        }
        if !context.confirmed_handoff_contract {
            missing.push(RequiredInput::ConfirmedHandoffContract);
        }
        if !missing.is_empty() {
            return insufficient(missing);
        }
        return ready(vec![RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskCompile,
            input: TypedCliInput::RequestText {
                text: request.to_string(),
            },
        }]);
    }

    if is_ambiguous_content_request(request) {
        return insufficient(vec![RequiredInput::ContentKind]);
    }
    if is_ambiguous_divination_request(request) {
        return insufficient(vec![RequiredInput::DivinationMethod]);
    }

    let mut targets = Vec::with_capacity(2);
    if let Some(demand) = classify_skill_demand(request) {
        targets.push(RouteTarget::Skill { demand });
    }
    match classify_cli_target(request) {
        Ok(Some(target)) => targets.push(target),
        Ok(None) => {}
        Err(missing) => return insufficient(vec![missing]),
    }

    if targets.is_empty() {
        targets.push(RouteTarget::DirectResponse);
    }
    ready(targets)
}

fn ready(targets: Vec<RouteTarget>) -> RequestDecision {
    debug_assert!(!targets.is_empty());
    debug_assert!(targets.len() <= 2);
    debug_assert!(
        !targets.contains(&RouteTarget::DirectResponse) || targets.len() == 1,
        "DirectResponse is exclusive"
    );
    RequestDecision {
        schema_version: REQUEST_DECISION_SCHEMA_VERSION.to_string(),
        status: DecisionStatus::Ready,
        targets,
    }
}

fn insufficient(missing: Vec<RequiredInput>) -> RequestDecision {
    RequestDecision {
        schema_version: REQUEST_DECISION_SCHEMA_VERSION.to_string(),
        status: DecisionStatus::InsufficientContext { missing },
        targets: Vec::new(),
    }
}

fn normalized(request: &str) -> String {
    request
        .to_lowercase()
        .split_whitespace()
        .collect::<String>()
}

fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn contains_pair(text: &str, left: &[&str], right: &[&str]) -> bool {
    contains_any(text, left) && contains_any(text, right)
}

fn is_canonical_task_card(request: &str) -> bool {
    request
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        == Some("## 任务卡")
}

fn is_bounded_content_transform(request: &str) -> bool {
    let text = normalized(request);
    let actions = [
        "压缩",
        "摘要",
        "总结",
        "翻译",
        "改写",
        "重排",
        "格式转换",
        "转换格式",
        "统一字段",
        "字段统一",
        "字段名",
        "normalize",
        "summarize",
        "translate",
        "reformat",
        "rewrite",
    ];
    let bounded = [
        "已确认",
        "已批准",
        "确定结构",
        "确认结构",
        "不改语义",
        "按这个结构",
        "按以下结构",
        "approved",
        "confirmed",
        "givenformat",
    ];
    contains_pair(&text, &actions, &bounded)
}

fn is_task_card_handoff(request: &str) -> bool {
    let text = normalized(request);
    contains_any(
        &text,
        &[
            "生成任务卡",
            "出任务卡",
            "交给claudecode",
            "给claudecode执行",
            "taskcard",
            "handoffcontract",
        ],
    )
}

fn is_ambiguous_content_request(request: &str) -> bool {
    matches!(
        normalized(request).as_str(),
        "写一篇文章" | "写文章" | "writeanarticle"
    )
}

fn is_ambiguous_divination_request(request: &str) -> bool {
    matches!(
        normalized(request).as_str(),
        "占卜" | "算一卦" | "divination"
    )
}

fn classify_skill_demand(request: &str) -> Option<SkillDemand> {
    let text = normalized(request);

    if contains_pair(
        &text,
        &["设计", "新架构", "架构设计", "define", "design"],
        &[
            "系统架构",
            "跨模块架构",
            "架构边界",
            "跨mcp",
            "cross-module",
            "systemarchitecture",
        ],
    ) {
        return engineering(EngineeringDemand::SystemArchitecture);
    }
    if contains_any(&text, &["brainstorm", "头脑风暴"]) {
        return engineering(EngineeringDemand::Brainstorming);
    }
    if contains_pair(
        &text,
        &["诊断", "排查", "定位", "debug", "diagnose"],
        &["失败", "报错", "异常", "bug", "error", "性能回归"],
    ) {
        return engineering(EngineeringDemand::Debugging);
    }
    if contains_any(
        &text,
        &["执行已批准计划", "按已确认计划执行", "executeapprovedplan"],
    ) {
        return engineering(EngineeringDemand::ApprovedPlanExecution);
    }
    if contains_any(&text, &["实现计划", "implementationplan", "编写实施计划"]) {
        return engineering(EngineeringDemand::ImplementationPlanning);
    }
    if contains_any(&text, &["测试驱动", "tdd", "test-drivendevelopment"]) {
        return engineering(EngineeringDemand::TestDrivenDevelopment);
    }
    if contains_any(&text, &["完成前验证", "verificationbeforecompletion"]) {
        return engineering(EngineeringDemand::CompletionVerification);
    }
    if contains_any(&text, &["代码审查", "审查代码", "codereview"]) {
        return engineering(EngineeringDemand::CodeReview);
    }
    if contains_any(&text, &["请求代码审查", "requestreview"]) {
        return engineering(EngineeringDemand::ReviewRequest);
    }
    if contains_pair(
        &text,
        &["设计", "重构", "改进"],
        &["模块接口", "模块边界", "可测试性", "deepmodule"],
    ) {
        return engineering(EngineeringDemand::ModuleDesign);
    }
    if contains_any(&text, &["领域建模", "领域模型", "统一术语", "domainmodel"]) {
        return engineering(EngineeringDemand::DomainModeling);
    }
    if contains_any(&text, &["架构技术债", "改进代码库架构", "architecturedebt"]) {
        return engineering(EngineeringDemand::ArchitectureImprovement);
    }
    if contains_any(&text, &["可丢弃原型", "throwawayprototype", "做个原型验证"]) {
        return engineering(EngineeringDemand::Prototype);
    }
    if contains_any(&text, &["压力测试方案", "grill这个计划", "plangrill"]) {
        return engineering(EngineeringDemand::PlanGrilling);
    }
    if contains_any(&text, &["结合adr追问", "结合术语表追问", "grillwithdocs"]) {
        return engineering(EngineeringDemand::DocsGroundedGrilling);
    }
    if contains_any(
        &text,
        &["创建技能", "优化技能", "写skill", "skillauthoring"],
    ) {
        return engineering(EngineeringDemand::SkillAuthoring);
    }
    if contains_any(&text, &["写prd", "生成prd", "prd authoring"]) {
        return engineering(EngineeringDemand::PrdAuthoring);
    }
    if contains_any(&text, &["拆成issues", "拆issue", "issueslicing"]) {
        return engineering(EngineeringDemand::IssueSlicing);
    }
    if contains_any(&text, &["issue分诊", "issuetriage"]) {
        return engineering(EngineeringDemand::IssueTriage);
    }
    if contains_any(&text, &["决策地图", "decisionmapping"]) {
        return engineering(EngineeringDemand::DecisionMapping);
    }
    if contains_any(&text, &["解决合并冲突", "mergeconflict"]) {
        return engineering(EngineeringDemand::MergeConflictResolution);
    }
    if contains_any(&text, &["上下文交接", "context handoff", "handoff文档"]) {
        return engineering(EngineeringDemand::ContextHandoff);
    }
    if contains_any(&text, &["审查分支", "branchreview"]) {
        return engineering(EngineeringDemand::BranchReview);
    }
    if contains_any(&text, &["交付报告", "deliveryreport"]) {
        return engineering(EngineeringDemand::DeliveryReporting);
    }

    classify_non_engineering(&text)
}

fn engineering(demand: EngineeringDemand) -> Option<SkillDemand> {
    Some(SkillDemand::Engineering(demand))
}

fn classify_non_engineering(text: &str) -> Option<SkillDemand> {
    let exact = [
        (
            "回忆对话",
            SkillDemand::Knowledge(KnowledgeDemand::ConversationMemoryRecall),
        ),
        (
            "搜索vault",
            SkillDemand::Knowledge(KnowledgeDemand::VaultKnowledge),
        ),
        ("飞书邮件", SkillDemand::Lark(LarkDemand::Mail)),
        ("飞书日历", SkillDemand::Lark(LarkDemand::Calendar)),
        ("飞书文档", SkillDemand::Lark(LarkDemand::Document)),
        ("飞书表格", SkillDemand::Lark(LarkDemand::Spreadsheet)),
        ("多维表格", SkillDemand::Lark(LarkDemand::Base)),
        ("飞书消息", SkillDemand::Lark(LarkDemand::Messaging)),
        ("飞书审批", SkillDemand::Lark(LarkDemand::Approval)),
        ("飞书任务", SkillDemand::Lark(LarkDemand::Task)),
        ("飞书知识库", SkillDemand::Lark(LarkDemand::Wiki)),
        ("飞书妙记", SkillDemand::Lark(LarkDemand::Minutes)),
        (
            "会议周报",
            SkillDemand::Lark(LarkDemand::MeetingSummaryWorkflow),
        ),
        (
            "站会日报",
            SkillDemand::Lark(LarkDemand::StandupReportWorkflow),
        ),
        (
            "产经文章",
            SkillDemand::Content(ContentDemand::IndustryArticleWriting),
        ),
        (
            "产经选题",
            SkillDemand::Content(ContentDemand::IndustryTopicPlanning),
        ),
        (
            "科技评论",
            SkillDemand::Content(ContentDemand::TechnologyCommentary),
        ),
        (
            "六爻",
            SkillDemand::Personal(PersonalDemand::LiuYaoDivination),
        ),
        (
            "塔罗",
            SkillDemand::Personal(PersonalDemand::TarotDivination),
        ),
    ];
    exact
        .iter()
        .find_map(|(trigger, demand)| text.contains(trigger).then_some(*demand))
}

fn classify_cli_target(request: &str) -> Result<Option<RouteTarget>, RequiredInput> {
    let text = normalized(request);
    let task_card = request
        .find("## 任务卡")
        .map(|start| request[start..].to_string());
    let target = if contains_any(&text, &["验证任务卡", "validatetaskcard"]) {
        RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskValidate,
            input: TypedCliInput::TaskCard {
                content: task_card.ok_or(RequiredInput::TaskCard)?,
            },
        }
    } else if contains_any(&text, &["解析执行策略", "resolvepolicy"]) {
        RouteTarget::MachineCli {
            capability: CliCapabilityId::PolicyResolve,
            input: TypedCliInput::TaskCard {
                content: task_card.ok_or(RequiredInput::TaskCard)?,
            },
        }
    } else if contains_any(
        &text,
        &[
            "运行项目验证",
            "验证项目",
            "runprojectverification",
            "runverify",
        ],
    ) {
        RouteTarget::MachineCli {
            capability: CliCapabilityId::ProjectVerify,
            input: TypedCliInput::Target {
                path: ".".to_string(),
            },
        }
    } else if contains_any(&text, &["验证技能标签", "verifyskilltags"]) {
        RouteTarget::MachineCli {
            capability: CliCapabilityId::SkillTagsVerify,
            input: TypedCliInput::TaskCard {
                content: task_card.ok_or(RequiredInput::TaskCard)?,
            },
        }
    } else if contains_any(&text, &["验证receipt", "验证回执", "verifyreceipt"]) {
        let receipt = request
            .split_once(':')
            .or_else(|| request.split_once('：'))
            .map(|(_, content)| content.trim())
            .filter(|content| !content.is_empty())
            .ok_or(RequiredInput::Receipt)?;
        RouteTarget::MachineCli {
            capability: CliCapabilityId::ReceiptVerify,
            input: TypedCliInput::RequestText {
                text: receipt.to_string(),
            },
        }
    } else {
        return Ok(None);
    };
    Ok(Some(target))
}
