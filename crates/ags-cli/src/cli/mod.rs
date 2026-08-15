use ags_control_plane::{
    AgentProbeRequest, AgentRegisterRequest, AgentSurface, CapabilityInventoryRequest,
    CapabilitySnapshotRequest, CheckRequest, CheckScope, DoctorRequest, DoctorScope,
    EvidenceArtifactKind, EvidenceRequest, GateRequest, HostProjectionRequest, InitRequest,
    McpAdviceRequest, MemoryCloseRequest, MigrationMode, OperationContext, OperationName,
    OperationRequest, PolicyRequest, ProjectionMode, SchemaRequest, SetupRequest,
    SkillInstallRequest, SkillRemoveRequest, SkillSourceKind, SkillSourceSpec, SkillUpdatePolicy,
    TaskCloseRequest, TaskPlanRequest, TaskValidateRequest, TestExecutor, TestProfile, TestRequest,
    UpdateRequest,
};
use ags_host_integration::HostId;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use std::ffi::OsString;
use std::path::PathBuf;

/// Typed adapter seam generated from the canonical Operation registry.
///
/// Clap owns product syntax only. Operation identity, CLI path metadata and the
/// request-to-control-plane constructor all come from the single core registry.
trait CliOperationAdapter: Sized {
    const OPERATION: OperationName;
    const CLI_PATH: &'static str;

    fn into_operation(self) -> OperationRequest;
}

macro_rules! define_cli_registry {
    ($( $variant:ident($request:ty) => $wire:literal, $cli:literal, $surface:ident, $resolver:path, [$primary:ident $(, $allowed:ident)*], $schema:literal, $summary:literal; )+) => {
        $(
            impl CliOperationAdapter for $request {
                const OPERATION: OperationName = OperationName::$variant;
                const CLI_PATH: &'static str = $cli;

                fn into_operation(self) -> OperationRequest {
                    let operation = OperationRequest::$variant(self);
                    debug_assert_eq!(operation.name(), Self::OPERATION);
                    debug_assert_eq!(operation.spec().cli_path, Self::CLI_PATH);
                    operation
                }
            }
        )+
    };
}

ags_control_plane::for_each_operation!(define_cli_registry);

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug)]
pub enum Invocation {
    Decide(OperationRequest),
    Apply {
        action_ref: String,
        outcome: Option<PathBuf>,
    },
    /// Ops-only A-to-B public projection (restored 0.4.16 mechanism, kept off
    /// the product Operation registry because it spans two checkouts).
    Release(ReleaseInvocation),
}

#[derive(Debug)]
pub struct ParsedInvocation {
    pub workspace: Option<PathBuf>,
    pub format: OutputFormat,
    pub invocation: Invocation,
}

#[derive(Debug, Parser)]
#[command(
    name = "ags",
    version,
    about = "Agent Governance Suite contract-v2 CLI",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

struct RegistryCommandNode {
    name: &'static str,
    leaf: Option<clap::Command>,
    children: Vec<RegistryCommandNode>,
}

impl RegistryCommandNode {
    fn branch(name: &'static str) -> Self {
        Self {
            name,
            leaf: None,
            children: Vec::new(),
        }
    }

    fn insert(&mut self, path: &[&'static str], leaf: clap::Command) {
        let Some((head, tail)) = path.split_first() else {
            panic!("registered CLI route is empty")
        };
        let index = self
            .children
            .iter()
            .position(|child| child.name == *head)
            .unwrap_or_else(|| {
                self.children.push(Self::branch(head));
                self.children.len() - 1
            });
        let child = &mut self.children[index];
        if tail.is_empty() {
            assert!(
                child.leaf.replace(leaf).is_none(),
                "duplicate registered CLI route"
            );
        } else {
            child.insert(tail, leaf);
        }
    }

    fn into_command(self) -> clap::Command {
        if let Some(leaf) = self.leaf {
            assert!(self.children.is_empty(), "CLI leaf is also a route prefix");
            return leaf;
        }
        clap::Command::new(self.name)
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommands(self.children.into_iter().map(Self::into_command))
    }
}

fn syntax_template(template: &clap::Command, path: &[&str]) -> clap::Command {
    let mut current = template;
    for segment in path {
        current = current.find_subcommand(segment).unwrap_or_else(|| {
            panic!(
                "registered CLI route `{}` has no typed syntax template",
                path.join(" ")
            )
        });
    }
    current.clone()
}

#[derive(Debug, Subcommand)]
enum Commands {
    Setup(WorkspaceArgs),
    Init(InitArgs),
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    Govern {
        #[command(subcommand)]
        action: GovernAction,
    },
    Update(UpdateArgs),
    Doctor(DoctorArgs),
    Check(CheckArgs),
    Test(TestArgs),
    Apply(ApplyArgs),
    Schema(SchemaArgs),
    /// Restored 0.4.16 ops surface: plan/apply the typed A-to-B public
    /// projection. Hidden from product help; used by the private-public
    /// promotion script.
    #[command(hide = true)]
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
}

#[derive(Debug, Args)]
struct WorkspaceArgs {
    #[arg(long)]
    workspace: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, value_enum, default_value_t = MigrationArg::None)]
    migration: MigrationArg,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum MigrationArg {
    None,
    ExactOwnedOnly,
}

#[derive(Debug, Subcommand)]
enum AgentAction {
    Register(AgentArgs),
    Probe(AgentArgs),
}

#[derive(Debug, Args)]
struct AgentArgs {
    #[arg(long)]
    host: String,
    #[arg(long, value_enum, default_value_t = SurfaceArg::Hybrid)]
    surface: SurfaceArg,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SurfaceArg {
    Cli,
    Mcp,
    Hybrid,
}

#[derive(Debug, Subcommand)]
enum GovernAction {
    HostProjection(ProjectionArgs),
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    Policy(PathArgs),
    Gate(PathArgs),
    Evidence(EvidenceArgs),
    Memory(MemoryArgs),
}

#[derive(Debug, Args)]
struct ProjectionArgs {
    #[arg(long)]
    host: String,
    #[arg(long, value_enum, default_value_t = ProjectionArg::Reconcile)]
    mode: ProjectionArg,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProjectionArg {
    Reconcile,
    RemoveOwned,
}

#[derive(Debug, Subcommand)]
enum CapabilityAction {
    Inventory(InventoryArgs),
    Snapshot(SnapshotArgs),
}

#[derive(Debug, Args)]
struct InventoryArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    include_inactive: bool,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct SnapshotArgs {
    #[arg(long)]
    host: String,
    #[arg(long)]
    replace_all: bool,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Subcommand)]
enum SkillAction {
    Install(SkillInstallArgs),
    Remove(SkillRemoveArgs),
}

#[derive(Debug, Args)]
struct SkillInstallArgs {
    skill_id: String,
    source: String,
    #[arg(long, value_enum, default_value_t = SkillSourceKindArg::Local)]
    source_kind: SkillSourceKindArg,
    #[arg(long)]
    requested_ref: Option<String>,
    #[arg(long)]
    tracking_ref: Option<String>,
    #[arg(long)]
    subdir: Option<String>,
    #[arg(long)]
    routing_metadata: Option<String>,
    #[arg(long = "target-host", required = true)]
    target_hosts: Vec<String>,
    #[arg(long, value_enum, default_value_t = SkillUpdatePolicyArg::Notify)]
    update_policy: SkillUpdatePolicyArg,
    #[arg(long = "acknowledge-risk")]
    risk_acknowledgements: Vec<String>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SkillSourceKindArg {
    Local,
    #[value(name = "github")]
    GitHub,
    Git,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SkillUpdatePolicyArg {
    Notify,
    Manual,
    Pinned,
}

#[derive(Debug, Args)]
struct SkillRemoveArgs {
    skill_id: String,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Subcommand)]
enum McpAction {
    Advice(McpAdviceArgs),
}

#[derive(Debug, Args)]
struct McpAdviceArgs {
    mcp_id: String,
    #[arg(long)]
    tool: Option<String>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Subcommand)]
enum TaskAction {
    Validate(TaskValidateArgs),
    Plan(TaskValidateArgs),
    Close(TaskCloseArgs),
}

#[derive(Debug, Args)]
struct TaskValidateArgs {
    #[arg(long)]
    task_card: PathBuf,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct TaskCloseArgs {
    #[arg(long)]
    task_card: PathBuf,
    #[arg(long)]
    launch_plan: PathBuf,
    #[arg(long)]
    delivery_report: PathBuf,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct PathArgs {
    #[arg(long)]
    task_card: PathBuf,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct EvidenceArgs {
    #[arg(value_enum)]
    kind: EvidenceKindArg,
    path: PathBuf,
    #[arg(long)]
    task_card: Option<PathBuf>,
    #[arg(long)]
    launch_plan: Option<PathBuf>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EvidenceKindArg {
    LaunchPlan,
    DeliveryReport,
    Receipt,
    TestReceipt,
}

#[derive(Debug, Args)]
struct MemoryArgs {
    receipt: PathBuf,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    #[arg(long, default_value = "stable")]
    channel: String,
    #[arg(long)]
    target_version: Option<String>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(value_enum, default_value_t = DoctorScopeArg::All)]
    scope: DoctorScopeArg,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DoctorScopeArg {
    Workspace,
    Runtime,
    Host,
    All,
}

#[derive(Debug, Args)]
struct CheckArgs {
    #[arg(value_enum, default_value_t = CheckScopeArg::Governance)]
    scope: CheckScopeArg,
    /// Explicit public worktree for promotion/release scopes.
    #[arg(long)]
    public_root: Option<PathBuf>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CheckScopeArg {
    Governance,
    Changes,
    Evidence,
    Release,
    Promotion,
}

#[derive(Debug, Args)]
struct TestArgs {
    #[arg(value_enum)]
    profile: TestProfileArg,
    #[arg(long, value_enum, default_value_t = TestExecutorArg::Host)]
    executor: TestExecutorArg,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TestProfileArg {
    Smoke,
    Standard,
    Full,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TestExecutorArg {
    Host,
    Local,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    action_ref: String,
    /// Typed delegated outcome JSON file, or `-` for stdin.
    #[arg(long)]
    outcome: Option<PathBuf>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Args)]
struct SchemaArgs {
    operation: Option<String>,
    #[command(flatten)]
    common: WorkspaceArgs,
}

#[derive(Debug, Clone, Subcommand)]
enum ReleaseAction {
    /// Plan or apply the complete transactional A-to-B public source projection.
    ProjectPublic {
        /// Private authority checkout A.
        #[arg(long)]
        source: PathBuf,
        /// Public checkout B.
        #[arg(long)]
        target: PathBuf,
        /// Apply the exact approved plan. Without this flag the command is read-only.
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// Plan hash printed by a preceding read-only invocation; required with --apply.
        #[arg(long, requires = "apply")]
        plan_hash: Option<String>,
    },
    /// Stage the release runtime asset payload from a frozen public release plan.
    StageRuntime {
        /// Release plan JSON produced by `ags check release --format json`.
        #[arg(long)]
        plan: PathBuf,
        /// Public source checkout root.
        #[arg(long)]
        source: PathBuf,
        /// Destination directory for the staged runtime payload.
        #[arg(long)]
        target: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub struct ReleaseInvocation {
    pub project_public: ReleaseProjectPublic,
    pub stage_runtime: ReleaseStageRuntime,
}

#[derive(Debug, Clone)]
pub struct ReleaseStageRuntime {
    pub plan: PathBuf,
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ReleaseProjectPublic {
    pub source: PathBuf,
    pub target: PathBuf,
    pub apply: bool,
    pub plan_hash: Option<String>,
}

impl Cli {
    /// Build the product parser from typed syntax templates, then make the
    /// canonical Operation registry authoritative for every public route's
    /// identity and help metadata. A template/registry drift is a build-time
    /// contract defect and fails before user input is interpreted.
    pub fn command_from_registry() -> clap::Command {
        let templates = <Self as CommandFactory>::command();
        let mut routes = RegistryCommandNode::branch("ags");
        let mut apply_inserted = false;
        for spec in ags_control_plane::operation_registry_for_surface(
            ags_control_plane::AdapterSurface::ProductCli,
        ) {
            if spec.name == OperationName::Schema && !apply_inserted {
                let apply_path = ["apply"];
                routes.insert(&apply_path, syntax_template(&templates, &apply_path));
                apply_inserted = true;
            }
            let path = spec.cli_path.split_whitespace().collect::<Vec<_>>();
            let leaf = syntax_template(&templates, &path).about(spec.summary);
            routes.insert(&path, leaf);
        }
        let release_path = ["release", "project-public"];
        routes.insert(&release_path, syntax_template(&templates, &release_path));
        let stage_path = ["release", "stage-runtime"];
        routes.insert(&stage_path, syntax_template(&templates, &stage_path));
        assert!(
            apply_inserted,
            "schema route must anchor the apply Interface"
        );
        clap::Command::new("ags")
            .version(env!("CARGO_PKG_VERSION"))
            .about("Agent Governance Suite contract-v2 CLI")
            .disable_help_subcommand(true)
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommands(
                routes
                    .children
                    .into_iter()
                    .map(RegistryCommandNode::into_command),
            )
    }

    pub fn try_parse_from<I, T>(input: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command_from_registry().try_get_matches_from(input)?;
        <Self as FromArgMatches>::from_arg_matches(&matches)
    }

    pub fn parse() -> Self {
        Self::try_parse_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    pub fn into_invocation(self) -> ParsedInvocation {
        let (common, mut invocation) = match self.command {
            Commands::Setup(common) => (
                common,
                SetupRequest {
                    context: ctx(),
                    approved_hosts: vec![],
                }
                .into_operation(),
            ),
            Commands::Init(args) => (
                args.common,
                InitRequest {
                    context: ctx(),
                    migration: match args.migration {
                        MigrationArg::None => MigrationMode::None,
                        MigrationArg::ExactOwnedOnly => MigrationMode::ExactOwnedOnly,
                    },
                }
                .into_operation(),
            ),
            Commands::Agent { action } => match action {
                AgentAction::Register(args) => {
                    let (host_id, surface) = agent(&args.host, args.surface);
                    (
                        args.common,
                        AgentRegisterRequest {
                            context: ctx(),
                            host_id,
                            surface,
                        }
                        .into_operation(),
                    )
                }
                AgentAction::Probe(args) => {
                    let (host_id, surface) = agent(&args.host, args.surface);
                    (
                        args.common,
                        AgentProbeRequest {
                            context: ctx(),
                            host_id,
                            surface,
                        }
                        .into_operation(),
                    )
                }
            },
            Commands::Govern { action } => govern(action),
            Commands::Update(args) => (
                args.common,
                UpdateRequest {
                    context: ctx(),
                    channel: args.channel,
                    target_version: args.target_version,
                }
                .into_operation(),
            ),
            Commands::Doctor(args) => (
                args.common,
                DoctorRequest {
                    context: ctx(),
                    scope: match args.scope {
                        DoctorScopeArg::Workspace => DoctorScope::Workspace,
                        DoctorScopeArg::Runtime => DoctorScope::Runtime,
                        DoctorScopeArg::Host => DoctorScope::Host,
                        DoctorScopeArg::All => DoctorScope::All,
                    },
                }
                .into_operation(),
            ),
            Commands::Check(args) => (
                args.common,
                CheckRequest {
                    context: ctx(),
                    scope: match args.scope {
                        CheckScopeArg::Governance => CheckScope::Governance,
                        CheckScopeArg::Changes => CheckScope::Changes,
                        CheckScopeArg::Evidence => CheckScope::Evidence,
                        CheckScopeArg::Release => CheckScope::Release,
                        CheckScopeArg::Promotion => CheckScope::Promotion,
                    },
                    public_root: args.public_root.as_ref().map(|p| p.to_string_lossy().into_owned()),
                }
                .into_operation(),
            ),
            Commands::Test(args) => (
                args.common,
                TestRequest {
                    context: ctx(),
                    profile: match args.profile {
                        TestProfileArg::Smoke => TestProfile::Smoke,
                        TestProfileArg::Standard => TestProfile::Standard,
                        TestProfileArg::Full => TestProfile::Full,
                    },
                    executor: match args.executor {
                        TestExecutorArg::Host => TestExecutor::Host,
                        TestExecutorArg::Local => TestExecutor::Local,
                    },
                }
                .into_operation(),
            ),
            Commands::Schema(args) => (
                args.common,
                SchemaRequest {
                    context: ctx(),
                    operation: args.operation,
                }
                .into_operation(),
            ),
            Commands::Apply(args) => {
                return ParsedInvocation {
                    workspace: args.common.workspace,
                    format: args.common.format,
                    invocation: Invocation::Apply {
                        action_ref: args.action_ref,
                        outcome: args.outcome,
                    },
                }
            }
            Commands::Release { action } => {
                return ParsedInvocation {
                    workspace: None,
                    format: OutputFormat::Text,
                    invocation: match action {
                        ReleaseAction::ProjectPublic {
                            source,
                            target,
                            apply,
                            plan_hash,
                        } => Invocation::Release(ReleaseInvocation {
                            project_public: ReleaseProjectPublic {
                                source,
                                target,
                                apply,
                                plan_hash,
                            },
                            stage_runtime: ReleaseStageRuntime {
                                plan: PathBuf::new(),
                                source: PathBuf::new(),
                                target: PathBuf::new(),
                            },
                        }),
                        ReleaseAction::StageRuntime { plan, source, target } => {
                            Invocation::Release(ReleaseInvocation {
                                project_public: ReleaseProjectPublic {
                                    source: PathBuf::new(),
                                    target: PathBuf::new(),
                                    apply: false,
                                    plan_hash: None,
                                },
                                stage_runtime: ReleaseStageRuntime { plan, source, target },
                            })
                        }
                    },
                }
            }
        };
        if let Some(workspace) = common.workspace.as_ref() {
            let normalized = workspace
                .canonicalize()
                .unwrap_or_else(|_| workspace.clone());
            invocation.context_mut().workspace = Some(path(normalized));
        }
        ParsedInvocation {
            workspace: common.workspace,
            format: common.format,
            invocation: Invocation::Decide(invocation),
        }
    }
}

fn govern(action: GovernAction) -> (WorkspaceArgs, OperationRequest) {
    match action {
        GovernAction::HostProjection(args) => (
            args.common,
            HostProjectionRequest {
                context: ctx(),
                host_id: host(&args.host),
                mode: match args.mode {
                    ProjectionArg::Reconcile => ProjectionMode::Reconcile,
                    ProjectionArg::RemoveOwned => ProjectionMode::RemoveOwned,
                },
            }
            .into_operation(),
        ),
        GovernAction::Capability { action } => match action {
            CapabilityAction::Inventory(args) => (
                args.common,
                CapabilityInventoryRequest {
                    context: ctx(),
                    host_id: args.host.map(|value| host(&value)),
                    include_inactive: args.include_inactive,
                }
                .into_operation(),
            ),
            CapabilityAction::Snapshot(args) => (
                args.common,
                CapabilitySnapshotRequest {
                    context: ctx(),
                    host_id: host(&args.host),
                    replace_all: args.replace_all,
                }
                .into_operation(),
            ),
        },
        GovernAction::Skill { action } => match action {
            SkillAction::Install(args) => (
                args.common,
                SkillInstallRequest {
                    context: ctx(),
                    skill_id: args.skill_id,
                    source: SkillSourceSpec {
                        kind: match args.source_kind {
                            SkillSourceKindArg::Local => SkillSourceKind::Local,
                            SkillSourceKindArg::GitHub => SkillSourceKind::GitHub,
                            SkillSourceKindArg::Git => SkillSourceKind::Git,
                        },
                        uri: args.source,
                        requested_ref: args.requested_ref,
                        tracking_ref: args.tracking_ref,
                        subdir: args.subdir,
                    },
                    routing_metadata: args.routing_metadata,
                    target_hosts: args
                        .target_hosts
                        .iter()
                        .map(|host_id| host(host_id))
                        .collect(),
                    update_policy: match args.update_policy {
                        SkillUpdatePolicyArg::Notify => SkillUpdatePolicy::Notify,
                        SkillUpdatePolicyArg::Manual => SkillUpdatePolicy::Manual,
                        SkillUpdatePolicyArg::Pinned => SkillUpdatePolicy::Pinned,
                    },
                    risk_acknowledgements: args.risk_acknowledgements,
                }
                .into_operation(),
            ),
            SkillAction::Remove(args) => (
                args.common,
                SkillRemoveRequest {
                    context: ctx(),
                    skill_id: args.skill_id,
                }
                .into_operation(),
            ),
        },
        GovernAction::Mcp {
            action: McpAction::Advice(args),
        } => (
            args.common,
            McpAdviceRequest {
                context: ctx(),
                mcp_id: args.mcp_id,
                tool: args.tool,
            }
            .into_operation(),
        ),
        GovernAction::Task { action } => match action {
            TaskAction::Validate(args) => (
                args.common,
                TaskValidateRequest {
                    context: ctx(),
                    task_card_path: path(args.task_card),
                }
                .into_operation(),
            ),
            TaskAction::Plan(args) => (
                args.common,
                TaskPlanRequest {
                    context: ctx(),
                    task_card_path: path(args.task_card),
                }
                .into_operation(),
            ),
            TaskAction::Close(args) => (
                args.common,
                TaskCloseRequest {
                    context: ctx(),
                    task_card_path: path(args.task_card),
                    launch_plan_path: path(args.launch_plan),
                    delivery_report_path: path(args.delivery_report),
                }
                .into_operation(),
            ),
        },
        GovernAction::Policy(args) => (
            args.common,
            PolicyRequest {
                context: ctx(),
                task_card_path: path(args.task_card),
            }
            .into_operation(),
        ),
        GovernAction::Gate(args) => (
            args.common,
            GateRequest {
                context: ctx(),
                task_card_path: path(args.task_card),
            }
            .into_operation(),
        ),
        GovernAction::Evidence(args) => (
            args.common,
            EvidenceRequest {
                context: ctx(),
                artifact_kind: match args.kind {
                    EvidenceKindArg::LaunchPlan => EvidenceArtifactKind::LaunchPlan,
                    EvidenceKindArg::DeliveryReport => EvidenceArtifactKind::DeliveryReport,
                    EvidenceKindArg::Receipt => EvidenceArtifactKind::Receipt,
                    EvidenceKindArg::TestReceipt => EvidenceArtifactKind::TestReceipt,
                },
                path: path(args.path),
                task_card_path: args.task_card.map(path),
                launch_plan_path: args.launch_plan.map(path),
            }
            .into_operation(),
        ),
        GovernAction::Memory(args) => (
            args.common,
            MemoryCloseRequest {
                context: ctx(),
                receipt_path: path(args.receipt),
            }
            .into_operation(),
        ),
    }
}

fn ctx() -> OperationContext {
    OperationContext::default()
}
fn path(value: PathBuf) -> String {
    value.to_string_lossy().into_owned()
}
fn host(value: &str) -> String {
    HostId::new(value)
        .unwrap_or_else(|error| {
            clap::Error::raw(clap::error::ErrorKind::InvalidValue, error).exit()
        })
        .to_string()
}
fn agent(value: &str, surface: SurfaceArg) -> (String, AgentSurface) {
    (
        host(value),
        match surface {
            SurfaceArg::Cli => AgentSurface::Cli,
            SurfaceArg::Mcp => AgentSurface::Mcp,
            SurfaceArg::Hybrid => AgentSurface::Hybrid,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_control_plane::{operation_registry_for_surface, AdapterSurface, OperationName};

    #[test]
    fn text_and_json_build_the_same_central_operation() {
        let text = Cli::try_parse_from(["ags", "test", "full", "--workspace", "."])
            .unwrap()
            .into_invocation();
        let json = Cli::try_parse_from([
            "ags",
            "test",
            "full",
            "--workspace",
            ".",
            "--format",
            "json",
        ])
        .unwrap()
        .into_invocation();
        let (Invocation::Decide(text), Invocation::Decide(json)) =
            (text.invocation, json.invocation)
        else {
            panic!()
        };
        assert_eq!(text, json);
    }

    #[test]
    fn generic_agent_is_normalized_without_an_allowlist() {
        let parsed = Cli::try_parse_from([
            "ags",
            "agent",
            "register",
            "--host",
            "Hermes Agent.v2",
            "--surface",
            "hybrid",
        ])
        .unwrap()
        .into_invocation();
        let Invocation::Decide(OperationRequest::AgentRegister(request)) = parsed.invocation else {
            panic!()
        };
        assert_eq!(request.host_id, "hermes-agent-v2");
        assert_eq!(request.surface, AgentSurface::Hybrid);
        assert_eq!(
            request.into_operation().kind(),
            ags_control_plane::OperationKind::Transaction
        );
    }

    #[test]
    fn skill_install_is_one_typed_transaction_request_for_generic_hosts() {
        let parsed = Cli::try_parse_from([
            "ags",
            "govern",
            "skill",
            "install",
            "hermes-fixture",
            "https://github.com/example/hermes-fixture",
            "--source-kind",
            "github",
            "--requested-ref",
            "v1.2.3",
            "--target-host",
            "Hermes Agent.v2",
            "--update-policy",
            "pinned",
            "--acknowledge-risk",
            "catalog_unreviewed",
        ])
        .unwrap()
        .into_invocation();
        let Invocation::Decide(OperationRequest::GovernSkillInstall(request)) = parsed.invocation
        else {
            panic!()
        };
        assert_eq!(request.skill_id, "hermes-fixture");
        assert_eq!(request.source.kind, SkillSourceKind::GitHub);
        assert_eq!(request.source.requested_ref.as_deref(), Some("v1.2.3"));
        assert_eq!(request.target_hosts, ["hermes-agent-v2"]);
        assert_eq!(request.update_policy, SkillUpdatePolicy::Pinned);
        assert_eq!(request.risk_acknowledgements, ["catalog_unreviewed"]);
        assert_eq!(
            request.into_operation().kind(),
            ags_control_plane::OperationKind::Transaction
        );
    }

    #[test]
    fn every_registered_product_operation_has_exactly_one_typed_cli_route() {
        let cases = vec![
            ("setup", vec!["ags", "setup"], OperationName::Setup),
            ("init", vec!["ags", "init"], OperationName::Init),
            (
                "agent register",
                vec!["ags", "agent", "register", "--host", "hermes"],
                OperationName::AgentRegister,
            ),
            (
                "agent probe",
                vec!["ags", "agent", "probe", "--host", "hermes"],
                OperationName::AgentProbe,
            ),
            (
                "govern host-projection",
                vec!["ags", "govern", "host-projection", "--host", "hermes"],
                OperationName::GovernHostProjection,
            ),
            (
                "govern capability inventory",
                vec!["ags", "govern", "capability", "inventory"],
                OperationName::GovernCapabilityInventory,
            ),
            (
                "govern skill install",
                vec![
                    "ags",
                    "govern",
                    "skill",
                    "install",
                    "example",
                    "/tmp/example",
                    "--target-host",
                    "hermes",
                ],
                OperationName::GovernSkillInstall,
            ),
            (
                "govern skill remove",
                vec!["ags", "govern", "skill", "remove", "example"],
                OperationName::GovernSkillRemove,
            ),
            (
                "govern capability snapshot",
                vec![
                    "ags",
                    "govern",
                    "capability",
                    "snapshot",
                    "--host",
                    "hermes",
                ],
                OperationName::GovernCapabilitySnapshot,
            ),
            (
                "govern mcp advice",
                vec!["ags", "govern", "mcp", "advice", "example"],
                OperationName::GovernMcpAdvice,
            ),
            (
                "govern task validate",
                vec![
                    "ags",
                    "govern",
                    "task",
                    "validate",
                    "--task-card",
                    "task.md",
                ],
                OperationName::GovernTaskValidate,
            ),
            (
                "govern task plan",
                vec!["ags", "govern", "task", "plan", "--task-card", "task.md"],
                OperationName::GovernTaskPlan,
            ),
            (
                "govern task close",
                vec![
                    "ags",
                    "govern",
                    "task",
                    "close",
                    "--task-card",
                    "task.md",
                    "--launch-plan",
                    "plan.md",
                    "--delivery-report",
                    "report.md",
                ],
                OperationName::GovernTaskClose,
            ),
            (
                "govern policy",
                vec!["ags", "govern", "policy", "--task-card", "task.md"],
                OperationName::GovernPolicy,
            ),
            (
                "govern gate",
                vec!["ags", "govern", "gate", "--task-card", "task.md"],
                OperationName::GovernGate,
            ),
            (
                "govern evidence",
                vec!["ags", "govern", "evidence", "receipt", "receipt.json"],
                OperationName::GovernEvidence,
            ),
            (
                "govern memory",
                vec!["ags", "govern", "memory", "receipt.json"],
                OperationName::GovernMemoryClose,
            ),
            ("update", vec!["ags", "update"], OperationName::Update),
            ("doctor", vec!["ags", "doctor"], OperationName::Doctor),
            ("check", vec!["ags", "check"], OperationName::Check),
            ("test", vec!["ags", "test", "smoke"], OperationName::Test),
            ("schema", vec!["ags", "schema"], OperationName::Schema),
        ];

        let mut observed = Vec::new();
        for (cli_path, argv, expected_name) in cases {
            let parsed = Cli::try_parse_from(argv).unwrap().into_invocation();
            let Invocation::Decide(operation) = parsed.invocation else {
                panic!("{cli_path} must normalize to a typed decide operation")
            };
            assert_eq!(operation.name(), expected_name, "{cli_path}");
            assert_eq!(operation.spec().cli_path, cli_path);
            let canonical = serde_json::to_vec(&operation).unwrap();
            let decoded: OperationRequest = serde_json::from_slice(&canonical).unwrap();
            assert_eq!(decoded, operation);
            assert_eq!(canonical, serde_json::to_vec(&decoded).unwrap());
            observed.push(operation.name().as_str());
        }
        observed.sort_unstable();
        let mut registered = operation_registry_for_surface(AdapterSurface::ProductCli)
            .into_iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>();
        registered.sort_unstable();
        assert_eq!(observed, registered);
    }

    #[test]
    fn registry_summaries_drive_real_clap_help() {
        let doctor = Cli::try_parse_from(["ags", "doctor", "--help"])
            .unwrap_err()
            .to_string();
        assert!(doctor.contains("Inspect runtime and workspace health"));
        let skill = Cli::try_parse_from(["ags", "govern", "skill", "install", "--help"])
            .unwrap_err()
            .to_string();
        assert!(skill.contains("Install an integrity-bound Skill"));
    }
}
