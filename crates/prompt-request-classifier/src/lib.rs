//! Prompt Request Classifier — deterministic, rule-based recognition of
//! "give me a prompt / generate a task card / hand off to Claude Code" intent.
//!
//! # Why this exists
//!
//! AGS has strong *back-end* gates (task-card validator, compiler
//! `task_card_requested` suppression, execution-policy resolver, MCP phase
//! prompts), but they are all **pull-based**: they only engage once some
//! upstream actor *decides* to route a request into the task-card pipeline.
//! That decision used to be pure model judgment, so a request like
//! "给我提示词" could be misread as ordinary prose and bypass every back-end
//! gate at the door. This classifier closes that gap with a **deterministic**,
//! testable signal — no model judgment, no free-form interpretation.
//!
//! # Design
//!
//! - **Deterministic** — substring matching over a locked trigger table.
//!   The same input always yields the same classification.
//! - **Spacing-insensitive** — every pattern is checked against the lowercased
//!   input *and* a despaced variant, so "交给 Claude Code" and "交给ClaudeCode"
//!   match the same trigger.
//! - **Action-bound** — task-card and prompt requests require an explicit
//!   generation or handoff verb. Bare artifact names are discussion topics, not
//!   execution instructions.

use serde::Serialize;

// ── Public types ───────────────────────────────────────────────────────────

/// The kind of prompt/task-card request detected in a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRequestKind {
    /// Explicit task-card request ("任务卡", "生成任务卡", "task card").
    TaskCardRequest,
    /// Hand off to an executor ("交给 Claude Code", "给 CC 执行", "handoff").
    Handoff,
    /// Prompt generation ("给我提示词", "写个 prompt", "give me a prompt").
    PromptRequest,
    /// Advisory/consultation intent ("评估一下", "你觉得", "should we").
    /// Mutation must be blocked unless an explicit execution override is present.
    AdvisoryIntent,
    /// No prompt/task-card request intent detected.
    NotARequest,
}

impl PromptRequestKind {
    /// Stable snake_case identifier, also used in JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            PromptRequestKind::TaskCardRequest => "task_card_request",
            PromptRequestKind::Handoff => "handoff",
            PromptRequestKind::PromptRequest => "prompt_request",
            PromptRequestKind::AdvisoryIntent => "advisory_intent",
            PromptRequestKind::NotARequest => "not_a_request",
        }
    }
}

/// The result of classifying a single user request.
#[derive(Debug, Clone, Serialize)]
pub struct Classification {
    /// The highest-precedence kind matched
    /// (TaskCardRequest > Handoff > PromptRequest > AdvisoryIntent > NotARequest).
    pub kind: PromptRequestKind,
    /// `true` for any positive kind — the AGS task-card pipeline must engage.
    pub is_task_card_request: bool,
    /// Every trigger phrase that matched, in their authored form.
    pub matched_triggers: Vec<String>,
    /// Distinct languages of the matched triggers (`"zh"` / `"en"`).
    pub trigger_lang: Vec<String>,
    /// `true` when advisory/consultation intent was detected.
    pub detected_advisory_intent: bool,
    /// `false` when advisory intent is active and no execution override clears it.
    /// Always `true` for non-advisory classifications.
    pub mutation_allowed: bool,
    /// Execution override triggers that were found (clears advisory no-mutation).
    pub advisory_override_triggers: Vec<String>,
}

/// A structured `governance_miss` event: a task-card request was about to leave
/// the foreground as non-canonical output (the exact pre-entry bypass this
/// component prevents). Emitted by the `ags gate output` check; AGS itself
/// writes no file — the host persists it if it wants the sample.
#[derive(Debug, Clone, Serialize)]
pub struct GovernanceMiss {
    /// Always `"governance_miss"`.
    pub event: &'static str,
    /// Detected request kind when a request string was supplied, else
    /// `"unclassified"`.
    pub detected_kind: String,
    /// Triggers from the supplied request (empty when no request was supplied).
    pub matched_triggers: Vec<String>,
    /// Why the output was blocked: `"bad_output_shape"` / `"validation_failed"`.
    pub blocked_reason: String,
    /// The gate stage that caught it: `"output_shape"` / `"validate"`.
    pub stage: String,
    /// Redacted head of the offending output (bounded, no secrets dumped).
    pub sample_redacted: String,
}

impl GovernanceMiss {
    /// Build a `governance_miss` event. `request` is the original user request
    /// when available (enables correlation of the bypass with its trigger).
    pub fn new(
        blocked_reason: &str,
        stage: &str,
        offending_output: &str,
        request: Option<&str>,
    ) -> Self {
        let (detected_kind, matched_triggers) = match request {
            Some(req) => {
                let c = classify(req);
                (c.kind.as_str().to_string(), c.matched_triggers)
            }
            None => ("unclassified".to_string(), Vec::new()),
        };
        GovernanceMiss {
            event: "governance_miss",
            detected_kind,
            matched_triggers,
            blocked_reason: blocked_reason.to_string(),
            stage: stage.to_string(),
            sample_redacted: redact_sample(offending_output, 160),
        }
    }
}

/// Truncate to `max` characters (char-safe) and strip newlines so the sample
/// stays a single bounded line. Never dumps an unbounded payload.
pub fn redact_sample(text: &str, max: usize) -> String {
    let flattened: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let trimmed = flattened.trim();
    let truncated: String = trimmed.chars().take(max).collect();
    if trimmed.chars().count() > max {
        format!("{truncated}…")
    } else {
        truncated
    }
}

// ── Trigger table (locked; covered by tests) ─────────────────────────────────

/// `(pattern, kind, lang)`. Patterns are authored lowercased. Chinese patterns
/// are space-free; English patterns keep natural spaces. Each is matched against
/// both the lowercased input and its despaced variant, so spacing never breaks a
/// match. Bare `提示词` / `prompt` are intentionally absent — only verb-bound
/// phrases trigger, to avoid matching incidental prose.
const TRIGGERS: &[(&str, PromptRequestKind, &str)] = &[
    // ── TaskCardRequest ──
    ("生成任务卡", PromptRequestKind::TaskCardRequest, "zh"),
    ("出任务卡", PromptRequestKind::TaskCardRequest, "zh"),
    (
        "generate a task card",
        PromptRequestKind::TaskCardRequest,
        "en",
    ),
    // ── Handoff ──
    ("交给 claude", PromptRequestKind::Handoff, "zh"),
    ("交给 claude code", PromptRequestKind::Handoff, "zh"),
    ("交给 cc", PromptRequestKind::Handoff, "zh"),
    ("给 cc 执行", PromptRequestKind::Handoff, "zh"),
    ("给 claude 执行", PromptRequestKind::Handoff, "zh"),
    ("给 claude code 执行", PromptRequestKind::Handoff, "zh"),
    ("给 claude code 提示词", PromptRequestKind::Handoff, "zh"),
    ("发给 cc", PromptRequestKind::Handoff, "zh"),
    ("拉去执行", PromptRequestKind::Handoff, "zh"),
    ("让 claude 做", PromptRequestKind::Handoff, "zh"),
    ("让 claude code 做", PromptRequestKind::Handoff, "zh"),
    ("让 claude 执行", PromptRequestKind::Handoff, "zh"),
    ("hand off", PromptRequestKind::Handoff, "en"),
    ("hand to claude", PromptRequestKind::Handoff, "en"),
    ("hand it to claude", PromptRequestKind::Handoff, "en"),
    // ── PromptRequest ──
    ("给我提示词", PromptRequestKind::PromptRequest, "zh"),
    ("生成提示词", PromptRequestKind::PromptRequest, "zh"),
    ("写个提示词", PromptRequestKind::PromptRequest, "zh"),
    ("写一个提示词", PromptRequestKind::PromptRequest, "zh"),
    ("要个提示词", PromptRequestKind::PromptRequest, "zh"),
    ("出提示词", PromptRequestKind::PromptRequest, "zh"),
    ("写个 prompt", PromptRequestKind::PromptRequest, "zh"),
    ("写一个 prompt", PromptRequestKind::PromptRequest, "zh"),
    ("生成 prompt", PromptRequestKind::PromptRequest, "zh"),
    ("给我 prompt", PromptRequestKind::PromptRequest, "zh"),
    ("give me a prompt", PromptRequestKind::PromptRequest, "en"),
    ("make a prompt", PromptRequestKind::PromptRequest, "en"),
    ("write a prompt", PromptRequestKind::PromptRequest, "en"),
    ("generate a prompt", PromptRequestKind::PromptRequest, "en"),
    ("create a prompt", PromptRequestKind::PromptRequest, "en"),
    // ── AdvisoryIntent ──
    ("你看看是否", PromptRequestKind::AdvisoryIntent, "zh"),
    ("是否需要", PromptRequestKind::AdvisoryIntent, "zh"),
    ("要不要", PromptRequestKind::AdvisoryIntent, "zh"),
    ("建议怎么做", PromptRequestKind::AdvisoryIntent, "zh"),
    ("评估一下", PromptRequestKind::AdvisoryIntent, "zh"),
    ("帮我评估", PromptRequestKind::AdvisoryIntent, "zh"),
    ("你觉得", PromptRequestKind::AdvisoryIntent, "zh"),
    ("分析一下", PromptRequestKind::AdvisoryIntent, "zh"),
    ("看看怎么样", PromptRequestKind::AdvisoryIntent, "zh"),
    ("给个建议", PromptRequestKind::AdvisoryIntent, "zh"),
    ("咨询一下", PromptRequestKind::AdvisoryIntent, "zh"),
    ("你怎么看", PromptRequestKind::AdvisoryIntent, "zh"),
    ("有没有必要", PromptRequestKind::AdvisoryIntent, "zh"),
    ("should we", PromptRequestKind::AdvisoryIntent, "en"),
    ("do you think", PromptRequestKind::AdvisoryIntent, "en"),
    (
        "would you recommend",
        PromptRequestKind::AdvisoryIntent,
        "en",
    ),
    ("assess whether", PromptRequestKind::AdvisoryIntent, "en"),
    ("give me advice", PromptRequestKind::AdvisoryIntent, "en"),
    ("what do you think", PromptRequestKind::AdvisoryIntent, "en"),
];

/// Execution override triggers that clear the advisory no-mutation block.
/// Only checked when advisory intent is detected AND the request is not phrased
/// as a question (see `is_question` in `classify`). These must be explicit
/// imperative authorizations — bare "执行" is excluded (false-positives on
/// "执行策略/执行器/执行门禁"), and "go ahead" is excluded because it reads as
/// part of a question ("should we go ahead with this refactor?").
const EXECUTION_OVERRIDES: &[(&str, &str)] = &[
    ("开改", "zh"),
    ("按这个改", "zh"),
    ("开始实现", "zh"),
    ("落地这个方案", "zh"),
    ("开始做", "zh"),
    ("去改", "zh"),
    ("动手", "zh"),
    ("一口气做完", "zh"),
    ("做完核验", "zh"),
    ("做完验证", "zh"),
    ("实现这个", "zh"),
    ("修复这个", "zh"),
    ("把它修复", "zh"),
    ("把它做完", "zh"),
    ("implement this", "en"),
    ("execute this", "en"),
    ("start implementing", "en"),
    ("fix this", "en"),
    ("get it done", "en"),
];

/// Deterministic detection of a STRUCTURED current-task execution approval on the
/// live user request (an explicit imperative instruction to implement / fix /
/// finish — see [`EXECUTION_OVERRIDES`]). This is the signal the gate / compiler
/// maps to `execution_policy::ApprovalSource::CurrentTaskInstruction`, an
/// audit/hint signal (task level does not downgrade the permission mode, so it is
/// no longer a Heavy execution unlock). It is NEVER derived from task-card text —
/// only from the live request. Bare "执行" is intentionally excluded (it reads as
/// a topic word, not an authorization).
pub fn detect_current_task_approval(request: &str) -> bool {
    if is_question(request) {
        return false;
    }
    let lower = request.to_lowercase();
    let lower_collapsed = collapse_ws(&lower);
    let compact = despace(&lower);
    EXECUTION_OVERRIDES.iter().any(|(pat, _lang)| {
        let pat_compact = despace(pat);
        contains_bounded(&lower_collapsed, pat) || contains_bounded(&compact, &pat_compact)
    })
}

// ── Classifier ───────────────────────────────────────────────────────────────

/// Remove ASCII whitespace from a string (used to build the despaced haystack
/// and despaced patterns so spacing never breaks a match).
fn despace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Collapse every run of whitespace to a single space. Spaced patterns are
/// authored with single spaces, so "交给  CLAUDE  code" must normalize to
/// "交给 claude code" before matching.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_question(s: &str) -> bool {
    let trimmed = s.trim_end();
    trimmed.ends_with('?') || trimmed.ends_with('？')
}

/// Substring match with a word boundary at ASCII-alphanumeric pattern edges.
///
/// A pattern that starts/ends with an ASCII alphanumeric must not be glued to
/// another ASCII alphanumeric in the haystack — so "写个prompt" matches
/// "写个prompt给我" but NOT "promptbook", and "task card" does not match inside
/// "task cardiac". CJK characters are treated as boundaries (they are not ASCII
/// alphanumeric), so Chinese patterns keep matching freely.
fn contains_bounded(haystack: &str, pat: &str) -> bool {
    if pat.is_empty() {
        return false;
    }
    let starts_alnum = pat
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let ends_alnum = pat
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric());

    for (idx, _) in haystack.match_indices(pat) {
        if starts_alnum {
            if let Some(prev) = haystack[..idx].chars().next_back() {
                if prev.is_ascii_alphanumeric() {
                    continue;
                }
            }
        }
        if ends_alnum {
            if let Some(next) = haystack[idx + pat.len()..].chars().next() {
                if next.is_ascii_alphanumeric() {
                    continue;
                }
            }
        }
        return true;
    }
    false
}

/// Classify a user request for prompt/task-card intent. Deterministic.
pub fn classify(request: &str) -> Classification {
    let lower = request.to_lowercase();
    let lower_collapsed = collapse_ws(&lower);
    let compact = despace(&lower);

    let mut matched_triggers: Vec<String> = Vec::new();
    let mut langs: Vec<String> = Vec::new();
    let mut has_task_card = false;
    let mut has_handoff = false;
    let mut has_prompt = false;
    let mut has_advisory = false;

    for (pattern, kind, lang) in TRIGGERS {
        let pat_compact = despace(pattern);
        let hit =
            contains_bounded(&lower_collapsed, pattern) || contains_bounded(&compact, &pat_compact);
        if !hit {
            continue;
        }
        matched_triggers.push((*pattern).to_string());
        if !langs.iter().any(|l| l == lang) {
            langs.push((*lang).to_string());
        }
        match kind {
            PromptRequestKind::TaskCardRequest => has_task_card = true,
            PromptRequestKind::Handoff => has_handoff = true,
            PromptRequestKind::PromptRequest => has_prompt = true,
            PromptRequestKind::AdvisoryIntent => has_advisory = true,
            PromptRequestKind::NotARequest => {}
        }
    }

    // Precedence: TaskCardRequest > Handoff > PromptRequest > AdvisoryIntent > NotARequest.
    let kind = if has_task_card {
        PromptRequestKind::TaskCardRequest
    } else if has_handoff {
        PromptRequestKind::Handoff
    } else if has_prompt {
        PromptRequestKind::PromptRequest
    } else if has_advisory {
        PromptRequestKind::AdvisoryIntent
    } else {
        PromptRequestKind::NotARequest
    };

    let is_task_card_request = !matches!(
        kind,
        PromptRequestKind::NotARequest | PromptRequestKind::AdvisoryIntent
    );

    let detected_advisory_intent = has_advisory;

    // A request phrased as a question never clears the advisory no-mutation guard,
    // even if it contains an imperative-looking phrase: "should we start
    // implementing this?" is still a question, not an authorization. We only scan
    // execution overrides for non-question requests.
    let mut advisory_override_triggers: Vec<String> = Vec::new();
    if detected_advisory_intent && !is_question(request) {
        for (pattern, _lang) in EXECUTION_OVERRIDES {
            let pat_compact = despace(pattern);
            let hit = contains_bounded(&lower_collapsed, pattern)
                || contains_bounded(&compact, &pat_compact);
            if hit {
                advisory_override_triggers.push((*pattern).to_string());
            }
        }
    }

    // Advisory no-mutation is cleared only by an explicit execution override or an
    // actual task-card request — never by a question, and never by a bare
    // ambiguous substring.
    let mutation_allowed = if detected_advisory_intent {
        !advisory_override_triggers.is_empty() || is_task_card_request
    } else {
        true
    };

    Classification {
        kind,
        is_task_card_request,
        matched_triggers,
        trigger_lang: langs,
        detected_advisory_intent,
        mutation_allowed,
        advisory_override_triggers,
    }
}

// ── Demand classification (Capability Route input) ───────────────────────────
//
// `classify_demand` is the deterministic text → demand-kind signal that feeds
// AGS Capability Route. It is the sibling of `classify` (which feeds Value
// Route / the task-card gate): same locked-trigger-table approach, same
// normalization (`despace` / `collapse_ws` / `contains_bounded`), no model
// judgment. Where `classify` answers "is this a prompt/task-card request?",
// `classify_demand` answers "what kind of development work is being asked
// for?" so AGS can route to the capability best suited to it.
//
// Like `classify`, this is deliberately **recall-biased**: the downstream
// Capability Route is advisory-only and never blocks, so a false positive only
// over-suggests a skill wakeup (a safe default) and a missed inflection only
// under-suggests one. Triggers are kept reasonably verb-bound to avoid the most
// obvious topic-vs-request confusions, but exhaustive precision is a non-goal.

/// The kind of development demand detected in a request, used to route to a
/// managed capability. Precedence (highest first — development demands win over
/// external mutations per governance policy):
/// `TaskCardHandoff > Debug > Verify > CodeReview > Commit > SkillAuthoring >
/// MattSuperpowers > Architecture > Brainstorm > MailSend > Messaging > Approval >
/// CalendarQuery > LarkDoc > SheetOp > LarkCollab > DocsLookup > None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DemandKind {
    /// Generate a prompt / task card / hand off to an executor. Delegated to
    /// `classify` so the handoff trigger table is never duplicated.
    TaskCardHandoff,
    /// An error, failing test, crash, or unexpected runtime behavior.
    Debug,
    /// Confirming work is complete / ready to commit / passing.
    Verify,
    /// Code review of existing code / a PR / a diff.
    CodeReview,
    /// Generate a commit message.
    Commit,
    /// Authoring / creating / installing a SKILL or plugin (e.g. `skill-creator`,
    /// `skill-installer`, `plugin-creator`). Routes only to an EXPLICITLY adopted
    /// (registry-routable) authoring capability — a merely discovered host-system
    /// skill stays fail-closed not-routable until adopted.
    SkillAuthoring,
    /// Matt Pocock / Superpowers workflow asks: PRD synthesis, issue slicing,
    /// decision maps, issue triage, branch/spec review, merge conflicts, or
    /// context handoff. Routed through AGS first, then subrouted to upstream
    /// named skills.
    MattSuperpowers,
    /// Refactoring, technical debt, boundary/coupling cleanup.
    Architecture,
    /// Open-ended feature, design, or recommendation work.
    Brainstorm,
    /// Compose / send an email (e.g. lark-mail). External write — confirm.
    MailSend,
    /// Send an IM / chat message (e.g. lark-im / Feishu group). External write.
    Messaging,
    /// Submit / act on an approval (e.g. lark-approval). External write.
    Approval,
    /// Query a calendar / schedule (e.g. lark-calendar). Read-mostly.
    CalendarQuery,
    /// Read / edit a Feishu (Lark) document (e.g. lark-doc). Platform-bound.
    LarkDoc,
    /// Read / edit a Feishu (Lark) sheet / base (e.g. lark-sheets). Platform-bound.
    SheetOp,
    /// Broad Feishu/Lark collaboration task when no narrower Lark demand fires.
    LarkCollab,
    /// Looking up library / API documentation (e.g. context7).
    DocsLookup,
    /// No development demand detected (ordinary prose).
    None,
}

impl DemandKind {
    /// Stable kebab-case identifier, identical to the serialized form.
    pub fn as_str(&self) -> &'static str {
        match self {
            DemandKind::TaskCardHandoff => "task-card-handoff",
            DemandKind::Debug => "debug",
            DemandKind::Verify => "verify",
            DemandKind::CodeReview => "code-review",
            DemandKind::Commit => "commit",
            DemandKind::SkillAuthoring => "skill-authoring",
            DemandKind::MattSuperpowers => "matt-superpowers",
            DemandKind::Architecture => "architecture",
            DemandKind::Brainstorm => "brainstorm",
            DemandKind::MailSend => "mail-send",
            DemandKind::Messaging => "messaging",
            DemandKind::Approval => "approval",
            DemandKind::CalendarQuery => "calendar-query",
            DemandKind::LarkDoc => "lark-doc",
            DemandKind::SheetOp => "sheet-op",
            DemandKind::LarkCollab => "lark-collab",
            DemandKind::DocsLookup => "docs-lookup",
            DemandKind::None => "none",
        }
    }
}

/// The result of classifying a request's development demand. Deterministic.
#[derive(Debug, Clone, Serialize)]
pub struct DemandClassification {
    /// The highest-precedence demand kind matched.
    pub kind: DemandKind,
    /// Every demand trigger phrase that matched, in its authored form. For a
    /// `TaskCardHandoff` verdict these are the `classify` handoff triggers.
    pub matched_triggers: Vec<String>,
}

/// `(pattern, kind, lang)` for demand detection. Patterns are authored
/// lowercased; Chinese patterns are space-free, English patterns keep natural
/// spaces — each is matched against both the collapsed and despaced input, like
/// `TRIGGERS`. `TaskCardHandoff` and `None` are intentionally absent here:
/// handoff is delegated to `classify` (no duplicate triggers) and `None` is the
/// fallback when nothing matches.
const DEMAND_TRIGGERS: &[(&str, DemandKind, &str)] = &[
    // ── Debug ──
    ("报错", DemandKind::Debug, "zh"),
    ("出bug", DemandKind::Debug, "zh"),
    ("有bug", DemandKind::Debug, "zh"),
    ("测试挂了", DemandKind::Debug, "zh"),
    ("测试失败", DemandKind::Debug, "zh"),
    ("不工作了", DemandKind::Debug, "zh"),
    ("跑不起来", DemandKind::Debug, "zh"),
    ("崩溃了", DemandKind::Debug, "zh"),
    ("error", DemandKind::Debug, "en"),
    ("errors", DemandKind::Debug, "en"),
    ("failing test", DemandKind::Debug, "en"),
    ("test failure", DemandKind::Debug, "en"),
    ("stack trace", DemandKind::Debug, "en"),
    ("stacktrace", DemandKind::Debug, "en"),
    ("traceback", DemandKind::Debug, "en"),
    ("crash", DemandKind::Debug, "en"),
    ("crashed", DemandKind::Debug, "en"),
    ("crashing", DemandKind::Debug, "en"),
    // ── Verify ──
    ("做完了", DemandKind::Verify, "zh"),
    ("搞定了", DemandKind::Verify, "zh"),
    ("验证一下", DemandKind::Verify, "zh"),
    ("帮我验证", DemandKind::Verify, "zh"),
    ("准备提交", DemandKind::Verify, "zh"),
    ("done", DemandKind::Verify, "en"),
    ("ready to commit", DemandKind::Verify, "en"),
    ("verify", DemandKind::Verify, "en"),
    // ── CodeReview ──
    ("代码审查", DemandKind::CodeReview, "zh"),
    ("审查代码", DemandKind::CodeReview, "zh"),
    ("评审代码", DemandKind::CodeReview, "zh"),
    ("帮我review", DemandKind::CodeReview, "zh"),
    ("code review", DemandKind::CodeReview, "en"),
    ("review my code", DemandKind::CodeReview, "en"),
    ("review this code", DemandKind::CodeReview, "en"),
    ("review the diff", DemandKind::CodeReview, "en"),
    // ── Commit ──
    ("提交信息", DemandKind::Commit, "zh"),
    ("提交说明", DemandKind::Commit, "zh"),
    ("写个commit", DemandKind::Commit, "zh"),
    ("生成commit", DemandKind::Commit, "zh"),
    ("commit message", DemandKind::Commit, "en"),
    ("commit msg", DemandKind::Commit, "en"),
    ("conventional commit", DemandKind::Commit, "en"),
    // ── SkillAuthoring (create / author / install a skill or plugin) ──
    // Verb-bound so a benign mention of "skill" / "技能" does not route here.
    ("创建技能", DemandKind::SkillAuthoring, "zh"),
    ("新建技能", DemandKind::SkillAuthoring, "zh"),
    ("新增技能", DemandKind::SkillAuthoring, "zh"),
    ("新技能", DemandKind::SkillAuthoring, "zh"),
    ("做一个技能", DemandKind::SkillAuthoring, "zh"),
    ("做个技能", DemandKind::SkillAuthoring, "zh"),
    ("写一个技能", DemandKind::SkillAuthoring, "zh"),
    ("写个技能", DemandKind::SkillAuthoring, "zh"),
    ("开发技能", DemandKind::SkillAuthoring, "zh"),
    ("技能开发", DemandKind::SkillAuthoring, "zh"),
    ("封装技能", DemandKind::SkillAuthoring, "zh"),
    ("封装成技能", DemandKind::SkillAuthoring, "zh"),
    ("创建一个技能", DemandKind::SkillAuthoring, "zh"),
    ("做一个插件", DemandKind::SkillAuthoring, "zh"),
    ("创建插件", DemandKind::SkillAuthoring, "zh"),
    ("安装技能", DemandKind::SkillAuthoring, "zh"),
    ("create a skill", DemandKind::SkillAuthoring, "en"),
    ("create a new skill", DemandKind::SkillAuthoring, "en"),
    ("new skill", DemandKind::SkillAuthoring, "en"),
    ("author a skill", DemandKind::SkillAuthoring, "en"),
    ("authoring a skill", DemandKind::SkillAuthoring, "en"),
    ("write a skill", DemandKind::SkillAuthoring, "en"),
    ("make a skill", DemandKind::SkillAuthoring, "en"),
    ("build a skill", DemandKind::SkillAuthoring, "en"),
    ("skill creator", DemandKind::SkillAuthoring, "en"),
    ("skill-creator", DemandKind::SkillAuthoring, "en"),
    ("skill installer", DemandKind::SkillAuthoring, "en"),
    ("skill-installer", DemandKind::SkillAuthoring, "en"),
    ("create a plugin", DemandKind::SkillAuthoring, "en"),
    ("new plugin", DemandKind::SkillAuthoring, "en"),
    ("plugin creator", DemandKind::SkillAuthoring, "en"),
    // ── Matt/Superpowers flow ──
    // These are specific workflow verbs/nouns. Generic `review` and generic
    // `handoff` stay with AGS root governance unless paired with Matt-style
    // artifacts (PRD/issues/decision map/etc.).
    ("prd", DemandKind::MattSuperpowers, "en"),
    ("product requirements", DemandKind::MattSuperpowers, "en"),
    (
        "break this prd into issues",
        DemandKind::MattSuperpowers,
        "en",
    ),
    (
        "break this plan into issues",
        DemandKind::MattSuperpowers,
        "en",
    ),
    ("decision map", DemandKind::MattSuperpowers, "en"),
    ("decision mapping", DemandKind::MattSuperpowers, "en"),
    ("investigation tickets", DemandKind::MattSuperpowers, "en"),
    ("triage issue", DemandKind::MattSuperpowers, "en"),
    ("ready-for-agent", DemandKind::MattSuperpowers, "en"),
    ("merge conflict", DemandKind::MattSuperpowers, "en"),
    ("merge conflicts", DemandKind::MattSuperpowers, "en"),
    ("rebase conflict", DemandKind::MattSuperpowers, "en"),
    ("handoff document", DemandKind::MattSuperpowers, "en"),
    ("handoff doc", DemandKind::MattSuperpowers, "en"),
    ("handoff note", DemandKind::MattSuperpowers, "en"),
    ("two-axis review", DemandKind::MattSuperpowers, "en"),
    ("review since", DemandKind::MattSuperpowers, "en"),
    ("standards and spec", DemandKind::MattSuperpowers, "en"),
    ("grill me", DemandKind::MattSuperpowers, "en"),
    ("grill-me", DemandKind::MattSuperpowers, "en"),
    ("obsidian vault", DemandKind::MattSuperpowers, "en"),
    ("obsidian", DemandKind::MattSuperpowers, "en"),
    ("需求文档", DemandKind::MattSuperpowers, "zh"),
    ("产品需求", DemandKind::MattSuperpowers, "zh"),
    ("拆成issues", DemandKind::MattSuperpowers, "zh"),
    ("拆成issue", DemandKind::MattSuperpowers, "zh"),
    ("拆成工单", DemandKind::MattSuperpowers, "zh"),
    ("任务拆分", DemandKind::MattSuperpowers, "zh"),
    ("决策地图", DemandKind::MattSuperpowers, "zh"),
    ("决策映射", DemandKind::MattSuperpowers, "zh"),
    ("松散想法", DemandKind::MattSuperpowers, "zh"),
    ("工单分诊", DemandKind::MattSuperpowers, "zh"),
    ("问题分诊", DemandKind::MattSuperpowers, "zh"),
    ("合并冲突", DemandKind::MattSuperpowers, "zh"),
    ("解决冲突", DemandKind::MattSuperpowers, "zh"),
    ("上下文交接", DemandKind::MattSuperpowers, "zh"),
    ("交接文档", DemandKind::MattSuperpowers, "zh"),
    ("交接说明", DemandKind::MattSuperpowers, "zh"),
    ("两轴审查", DemandKind::MattSuperpowers, "zh"),
    ("评审分支", DemandKind::MattSuperpowers, "zh"),
    ("方案拷问", DemandKind::MattSuperpowers, "zh"),
    ("黑曜石", DemandKind::MattSuperpowers, "zh"),
    // ── Architecture ──
    ("重构", DemandKind::Architecture, "zh"),
    ("技术债", DemandKind::Architecture, "zh"),
    ("边界混乱", DemandKind::Architecture, "zh"),
    ("难维护", DemandKind::Architecture, "zh"),
    ("refactor", DemandKind::Architecture, "en"),
    ("tech debt", DemandKind::Architecture, "en"),
    ("technical debt", DemandKind::Architecture, "en"),
    ("simplify architecture", DemandKind::Architecture, "en"),
    // ── Brainstorm ──
    ("设计一个", DemandKind::Brainstorm, "zh"),
    ("怎么实现", DemandKind::Brainstorm, "zh"),
    ("如何实现", DemandKind::Brainstorm, "zh"),
    ("做一个", DemandKind::Brainstorm, "zh"),
    ("加一个功能", DemandKind::Brainstorm, "zh"),
    // `推荐` kept verb-bound to avoid matching topics like `推荐系统` / `推荐算法`.
    ("推荐一下", DemandKind::Brainstorm, "zh"),
    ("推荐个", DemandKind::Brainstorm, "zh"),
    ("求推荐", DemandKind::Brainstorm, "zh"),
    ("有什么推荐", DemandKind::Brainstorm, "zh"),
    ("brainstorm", DemandKind::Brainstorm, "en"),
    ("design a", DemandKind::Brainstorm, "en"),
    ("how should i build", DemandKind::Brainstorm, "en"),
    // ── DocsLookup ──
    ("查文档", DemandKind::DocsLookup, "zh"),
    ("看文档", DemandKind::DocsLookup, "zh"),
    ("文档", DemandKind::DocsLookup, "zh"),
    ("怎么用这个api", DemandKind::DocsLookup, "zh"),
    ("api 怎么用", DemandKind::DocsLookup, "zh"),
    ("library docs", DemandKind::DocsLookup, "en"),
    ("api docs", DemandKind::DocsLookup, "en"),
    ("docs", DemandKind::DocsLookup, "en"),
    ("how do i use", DemandKind::DocsLookup, "en"),
    // ── MailSend ── (external mutation; outranks DocsLookup so "把文档发邮件"
    //   routes to mail, not docs)
    ("发邮件", DemandKind::MailSend, "zh"),
    ("发个邮件", DemandKind::MailSend, "zh"),
    ("发送邮件", DemandKind::MailSend, "zh"),
    ("写封邮件", DemandKind::MailSend, "zh"),
    ("发邮件给", DemandKind::MailSend, "zh"),
    ("email to", DemandKind::MailSend, "en"),
    ("send an email", DemandKind::MailSend, "en"),
    ("send email", DemandKind::MailSend, "en"),
    // ── Messaging (IM / Feishu group) ──
    ("发消息", DemandKind::Messaging, "zh"),
    ("发个消息", DemandKind::Messaging, "zh"),
    ("发到群里", DemandKind::Messaging, "zh"),
    ("群发", DemandKind::Messaging, "zh"),
    ("飞书群", DemandKind::Messaging, "zh"),
    ("send a message", DemandKind::Messaging, "en"),
    ("send to lark", DemandKind::Messaging, "en"),
    ("post to the channel", DemandKind::Messaging, "en"),
    // ── Approval ──
    ("发起审批", DemandKind::Approval, "zh"),
    ("提交审批", DemandKind::Approval, "zh"),
    ("审批一下", DemandKind::Approval, "zh"),
    ("approval request", DemandKind::Approval, "en"),
    ("submit for approval", DemandKind::Approval, "en"),
    // ── CalendarQuery ── (first-person / possessive bound to avoid stealing
    //   "重构日程模块")
    ("我的日程", DemandKind::CalendarQuery, "zh"),
    ("查日程", DemandKind::CalendarQuery, "zh"),
    ("看日程", DemandKind::CalendarQuery, "zh"),
    ("飞书日历", DemandKind::CalendarQuery, "zh"),
    ("日历安排", DemandKind::CalendarQuery, "zh"),
    ("今天的会", DemandKind::CalendarQuery, "zh"),
    ("my calendar", DemandKind::CalendarQuery, "en"),
    ("my schedule", DemandKind::CalendarQuery, "en"),
    ("agenda for", DemandKind::CalendarQuery, "en"),
    // ── LarkDoc ── (platform-bound so generic "文档" stays DocsLookup→context7)
    ("飞书文档", DemandKind::LarkDoc, "zh"),
    ("飞书云文档", DemandKind::LarkDoc, "zh"),
    ("飞书知识库", DemandKind::LarkDoc, "zh"),
    ("lark doc", DemandKind::LarkDoc, "en"),
    ("feishu doc", DemandKind::LarkDoc, "en"),
    // ── SheetOp ── (platform-bound)
    ("飞书表格", DemandKind::SheetOp, "zh"),
    ("飞书多维表格", DemandKind::SheetOp, "zh"),
    ("飞书电子表格", DemandKind::SheetOp, "zh"),
    ("lark sheet", DemandKind::SheetOp, "en"),
    ("feishu sheet", DemandKind::SheetOp, "en"),
    // ── LarkCollab ── (broad catch-all after narrower Lark demands)
    ("飞书", DemandKind::LarkCollab, "zh"),
    ("会议纪要", DemandKind::LarkCollab, "zh"),
    ("妙记", DemandKind::LarkCollab, "zh"),
    ("云盘", DemandKind::LarkCollab, "zh"),
    ("通讯录", DemandKind::LarkCollab, "zh"),
    ("联系人", DemandKind::LarkCollab, "zh"),
    ("okr", DemandKind::LarkCollab, "zh"),
    ("白板", DemandKind::LarkCollab, "zh"),
    ("飞书应用", DemandKind::LarkCollab, "zh"),
    ("lark", DemandKind::LarkCollab, "en"),
    ("feishu", DemandKind::LarkCollab, "en"),
];

/// Verb-bound creation/authoring lead-ins for the skill-authoring co-occurrence
/// rule. Locked, lowercased, space-free (Chinese) / natural-space (English) like
/// [`DEMAND_TRIGGERS`].
const SKILL_AUTHORING_VERBS: &[&str] = &[
    "创建",
    "新建",
    "新增",
    "做一个",
    "做个",
    "写一个",
    "写个",
    "搭一个",
    "搞一个",
    "搞个",
    "开发",
    "封装",
    "create",
    "build",
    "make",
    "author",
    "authoring",
    "write",
    "develop",
    "scaffold",
];

/// Skill / plugin nouns for the skill-authoring co-occurrence rule.
const SKILL_AUTHORING_NOUNS: &[&str] = &["技能", "skill", "插件", "plugin"];

/// Detect a skill-authoring request where a creation/authoring verb co-occurs
/// with a skill/plugin noun even though a skill NAME sits between them — e.g.
/// "创建一个新的 Hermes skill" (创建 … skill) or "build the Foo skill". A single
/// locked substring cannot span the intervening name, so the bounded
/// [`DEMAND_TRIGGERS`] table misses these; this co-occurrence pass closes the gap
/// without a free-form rule. Returns the matched `(verb, noun)` pair for trigger
/// transparency, or `None`. Recall-biased and advisory-only — higher-precedence
/// demands (Debug / Verify / …) still win in the precedence chain, so a bug
/// report that merely names a skill is never stolen by this rule.
fn skill_authoring_cooccurrence(
    collapsed: &str,
    compact: &str,
) -> Option<(&'static str, &'static str)> {
    let hit =
        |pat: &str| contains_bounded(collapsed, pat) || contains_bounded(compact, &despace(pat));
    let verb = SKILL_AUTHORING_VERBS.iter().copied().find(|v| hit(v))?;
    let noun = SKILL_AUTHORING_NOUNS.iter().copied().find(|n| hit(n))?;
    Some((verb, noun))
}

/// Classify a request's development demand for Capability Route. Deterministic:
/// same input → same demand. Task-card / handoff demand is delegated to
/// [`classify`] (so the handoff trigger table is not duplicated); the remaining
/// kinds come from [`DEMAND_TRIGGERS`]. Precedence:
/// `TaskCardHandoff > Debug > Verify > CodeReview > Commit > SkillAuthoring >
/// MattSuperpowers > Architecture > Brainstorm > MailSend > Messaging > Approval >
/// CalendarQuery > LarkDoc > SheetOp > LarkCollab > DocsLookup > None`.
pub fn classify_demand(request: &str) -> DemandClassification {
    let base = classify(request);

    let lower = request.to_lowercase();
    let lower_collapsed = collapse_ws(&lower);
    let compact = despace(&lower);

    let mut matched_triggers: Vec<String> = Vec::new();
    let (
        mut has_debug,
        mut has_verify,
        mut has_review,
        mut has_commit,
        mut has_matt,
        mut has_arch,
        mut has_brain,
        mut has_docs,
        mut has_skill_authoring,
    ) = (
        false, false, false, false, false, false, false, false, false,
    );
    let (mut has_mail, mut has_msg, mut has_approval, mut has_calendar) =
        (false, false, false, false);
    let (mut has_lark_doc, mut has_sheet, mut has_lark) = (false, false, false);

    for (pattern, kind, _lang) in DEMAND_TRIGGERS {
        let pat_compact = despace(pattern);
        let hit =
            contains_bounded(&lower_collapsed, pattern) || contains_bounded(&compact, &pat_compact);
        if !hit {
            continue;
        }
        matched_triggers.push((*pattern).to_string());
        match kind {
            DemandKind::Debug => has_debug = true,
            DemandKind::Verify => has_verify = true,
            DemandKind::CodeReview => has_review = true,
            DemandKind::Commit => has_commit = true,
            DemandKind::SkillAuthoring => has_skill_authoring = true,
            DemandKind::MattSuperpowers => has_matt = true,
            DemandKind::Architecture => has_arch = true,
            DemandKind::Brainstorm => has_brain = true,
            DemandKind::MailSend => has_mail = true,
            DemandKind::Messaging => has_msg = true,
            DemandKind::Approval => has_approval = true,
            DemandKind::CalendarQuery => has_calendar = true,
            DemandKind::LarkDoc => has_lark_doc = true,
            DemandKind::SheetOp => has_sheet = true,
            DemandKind::LarkCollab => has_lark = true,
            DemandKind::DocsLookup => has_docs = true,
            // TaskCardHandoff / None never appear in DEMAND_TRIGGERS.
            DemandKind::TaskCardHandoff | DemandKind::None => {}
        }
    }

    // Skill-authoring co-occurrence: catch "创建一个新的 Hermes skill" /
    // "build the Foo skill" where a creation verb and a skill noun are split by a
    // skill name. Only when no bounded trigger already fired (no double count),
    // and it does not change precedence — the chain below still lets higher
    // demands win.
    if !has_skill_authoring {
        if let Some((verb, noun)) = skill_authoring_cooccurrence(&lower_collapsed, &compact) {
            has_skill_authoring = true;
            matched_triggers.push(format!("{verb} … {noun} (skill-authoring co-occurrence)"));
        }
    }

    // Precedence (development demands win over external mutations per policy;
    // platform-bound Lark doc/sheet outrank generic docs-lookup):
    // TaskCardHandoff > Debug > Verify > CodeReview > Commit > MattSuperpowers >
    // Architecture > Brainstorm > MailSend > Messaging > Approval > CalendarQuery >
    // LarkDoc > SheetOp > LarkCollab > DocsLookup > None.
    // Handoff reuses the `classify` signal, with one narrow exception: a bare
    // English `handoff` trigger paired with Matt artifact words such as
    // "handoff document" means the upstream `handoff` skill, not executor
    // delegation. Explicit executor / prompt triggers still win.
    let bare_handoff_artifact = has_matt
        && base.kind == PromptRequestKind::Handoff
        && base
            .matched_triggers
            .iter()
            .all(|t| matches!(t.as_str(), "handoff" | "hand off"));
    if base.is_task_card_request && !bare_handoff_artifact {
        return DemandClassification {
            kind: DemandKind::TaskCardHandoff,
            matched_triggers: base.matched_triggers,
        };
    }
    let kind = if has_debug {
        DemandKind::Debug
    } else if has_verify {
        DemandKind::Verify
    } else if has_review {
        DemandKind::CodeReview
    } else if has_commit {
        DemandKind::Commit
    } else if has_skill_authoring {
        DemandKind::SkillAuthoring
    } else if has_matt {
        DemandKind::MattSuperpowers
    } else if has_arch {
        DemandKind::Architecture
    } else if has_brain {
        DemandKind::Brainstorm
    } else if has_mail {
        DemandKind::MailSend
    } else if has_msg {
        DemandKind::Messaging
    } else if has_approval {
        DemandKind::Approval
    } else if has_calendar {
        DemandKind::CalendarQuery
    } else if has_lark_doc {
        DemandKind::LarkDoc
    } else if has_sheet {
        DemandKind::SheetOp
    } else if has_lark {
        DemandKind::LarkCollab
    } else if has_docs {
        DemandKind::DocsLookup
    } else {
        DemandKind::None
    };

    DemandClassification {
        kind,
        matched_triggers,
    }
}

// ── Value Route (效价比路由) ─────────────────────────────────────────────────
//
// Value Route is the cost-effectiveness routing signal: after solution formation
// and before Light / Medium / Heavy routing, pick the *minimal execution-path
// form* that still covers the task's risk. It is deterministic — derived from the
// same locked classification signals as the entry gate — and advisory only. It
// NEVER sets or changes the task level, permission mode, Review gate, or
// Verification gate. The planner owns the final path; AGS only surfaces the
// recommendation and its rejected lighter / heavier alternatives so the choice is
// auditable.

/// The canonical Value Route execution-path forms. Orthogonal to Light / Medium /
/// Heavy: a Heavy task may still be `plan-first` then `claude-code-route`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValuePath {
    /// Diagnose / answer only; no mutation.
    ReadOnlyAdvisory,
    /// Bounded, local edit applied in-session, then verify.
    DirectEdit,
    /// Return root cause / design / plan; await confirmation before editing.
    PlanFirst,
    /// Frame a bounded task card and hand off to Claude Code CLI.
    ClaudeCodeRoute,
    /// Scope / authority / risk unclear — stop and report instead of proceeding.
    StopForScope,
}

impl ValuePath {
    /// Stable kebab-case identifier, identical to the serialized form.
    pub fn as_str(&self) -> &'static str {
        match self {
            ValuePath::ReadOnlyAdvisory => "read-only-advisory",
            ValuePath::DirectEdit => "direct-edit",
            ValuePath::PlanFirst => "plan-first",
            ValuePath::ClaudeCodeRoute => "claude-code-route",
            ValuePath::StopForScope => "stop-for-scope",
        }
    }
}

/// A rejected alternative path and why it was not chosen.
#[derive(Debug, Clone, Serialize)]
pub struct RejectedPath {
    pub path: ValuePath,
    pub reason: String,
}

/// The Value Route recommendation. Advisory and deterministic; shapes the
/// execution-path form only — never the task level, permission mode, or gates.
#[derive(Debug, Clone, Serialize)]
pub struct ValueRoute {
    /// Recommended minimal execution-path form.
    pub recommended_path: ValuePath,
    /// Why this path is the minimal form that still covers the risk.
    pub rationale: String,
    /// A lighter path that was rejected because it would under-cover the risk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_lighter: Option<RejectedPath>,
    /// A heavier path that was rejected because it would over-spend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_heavier: Option<RejectedPath>,
    /// Whether acting on the recommendation needs explicit user confirmation
    /// (e.g. a plan must be approved before mutation).
    pub requires_user_confirmation: bool,
    /// `true` when AGS lacks a strong signal and the planner must finalize.
    pub needs_planner_judgment: bool,
    /// Always `true` — Value Route is advisory, never an authority.
    pub advisory: bool,
    /// Fixed boundary statement: Value Route never changes level/permission/gates.
    pub authority_note: String,
}

/// Fixed boundary statement carried on every Value Route output.
pub const VALUE_ROUTE_AUTHORITY_NOTE: &str = "Value Route shapes the execution-path form only. It does NOT set or change the Light/Medium/Heavy task level, permission mode, Review gate, or Verification gate. It is advisory — the planner owns the final path. AGS protocol, the task-card validator, the execution-policy resolver, and the gates remain authoritative.";

/// Derive the Value Route recommendation from deterministic classification
/// signals plus solution-phase context. `task_card_requested` is whether the user
/// has issued an explicit task-card instruction; `direct_execution_authorized`
/// is whether the current live instruction explicitly authorizes same-session
/// mutation. Deterministic: same inputs → same route.
///
/// `stop-for-scope` is part of the taxonomy but is never auto-recommended here —
/// it is a planner/host-selected escape hatch when scope, authority, or risk is
/// unclear, which cannot be inferred reliably from request text alone.
pub fn derive_value_route(
    classification: &Classification,
    task_card_requested: bool,
    direct_execution_authorized: bool,
) -> ValueRoute {
    let note = VALUE_ROUTE_AUTHORITY_NOTE.to_string();

    if classification.detected_advisory_intent && !classification.mutation_allowed {
        ValueRoute {
            recommended_path: ValuePath::ReadOnlyAdvisory,
            rationale: "Consultation intent detected with no execution authorization; answer and assess only. Any mutation requires an explicit execution authorization first.".to_string(),
            rejected_lighter: None,
            rejected_heavier: Some(RejectedPath {
                path: ValuePath::DirectEdit,
                reason: "Editing now would exceed the consultation intent — the user asked for assessment, not a change.".to_string(),
            }),
            requires_user_confirmation: true,
            needs_planner_judgment: false,
            advisory: true,
            authority_note: note,
        }
    } else if classification.is_task_card_request {
        if task_card_requested {
            ValueRoute {
                recommended_path: ValuePath::ClaudeCodeRoute,
                rationale: "Explicit task-card instruction received for a prompt/handoff request; frame a bounded task card and hand off to Claude Code.".to_string(),
                rejected_lighter: Some(RejectedPath {
                    path: ValuePath::PlanFirst,
                    reason: "Stopping at a plan would not deliver the requested executable handoff.".to_string(),
                }),
                rejected_heavier: None,
                requires_user_confirmation: false,
                needs_planner_judgment: false,
                advisory: true,
                authority_note: note,
            }
        } else {
            ValueRoute {
                recommended_path: ValuePath::PlanFirst,
                rationale: "Prompt/handoff intent detected but no explicit task-card instruction yet; form the solution/contract and await the instruction before routing.".to_string(),
                rejected_lighter: Some(RejectedPath {
                    path: ValuePath::DirectEdit,
                    reason: "Editing now would skip the requested prompt/handoff and the task-card instruction gate.".to_string(),
                }),
                rejected_heavier: Some(RejectedPath {
                    path: ValuePath::ClaudeCodeRoute,
                    reason: "The host has not supplied the explicit task-card/handoff authorization required to generate the artifact.".to_string(),
                }),
                requires_user_confirmation: true,
                needs_planner_judgment: false,
                advisory: true,
                authority_note: note,
            }
        }
    } else if direct_execution_authorized {
        ValueRoute {
            recommended_path: ValuePath::DirectEdit,
            rationale: "The current live instruction explicitly authorizes same-session mutation; edit in-session and verify without compiling a handoff task card.".to_string(),
            rejected_lighter: Some(RejectedPath {
                path: ValuePath::ReadOnlyAdvisory,
                reason: "The user explicitly authorized mutation, so stopping at advice would under-deliver.".to_string(),
            }),
            rejected_heavier: Some(RejectedPath {
                path: ValuePath::ClaudeCodeRoute,
                reason: "No cross-agent or detached handoff was requested, so a task card would add ceremony without additional control.".to_string(),
            }),
            requires_user_confirmation: false,
            needs_planner_judgment: false,
            advisory: true,
            authority_note: note,
        }
    } else {
        ValueRoute {
            recommended_path: ValuePath::PlanFirst,
            rationale: "No strong entry signal and the change is non-trivial; default to forming a short plan before editing. The planner must confirm the minimal sufficient path.".to_string(),
            rejected_lighter: Some(RejectedPath {
                path: ValuePath::DirectEdit,
                reason: "Acceptable only if the planner confirms the change is small, local, and low-risk.".to_string(),
            }),
            rejected_heavier: Some(RejectedPath {
                path: ValuePath::ClaudeCodeRoute,
                reason: "Delegate only if the work is large or benefits from an isolated bounded handoff.".to_string(),
            }),
            requires_user_confirmation: false,
            needs_planner_judgment: true,
            advisory: true,
            authority_note: note,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Every phrase the task card requires the classifier to recognize.
    #[test]
    fn required_triggers_all_fire() {
        let positives = [
            "给我提示词",
            "生成提示词",
            "帮我生成任务卡",
            "交给 Claude Code",
            "交给 Claude Code 执行",
            "给 CC 执行",
            "给 cc 执行这个",
            "写个 prompt",
            "写一个 prompt 给我",
            "hand off to claude",
            "让 Claude 做",
            "让 Claude Code 做这件事",
            "帮我写个任务卡拉去执行",
            "give me a prompt",
            "make a prompt for claude code",
            "generate a task card",
        ];
        for p in positives {
            let c = classify(p);
            assert!(
                c.is_task_card_request,
                "expected task-card request for input {p:?}, got {:?}",
                c.kind
            );
            assert!(
                !c.matched_triggers.is_empty(),
                "expected matched triggers for {p:?}"
            );
        }
    }

    /// Spacing and casing must not change the verdict.
    #[test]
    fn spacing_and_casing_insensitive() {
        for variant in ["交给 Claude Code", "交给ClaudeCode", "交给  CLAUDE  code"] {
            let c = classify(variant);
            assert!(c.is_task_card_request, "{variant:?} should match");
            assert_eq!(c.kind, PromptRequestKind::Handoff, "{variant:?}");
        }
        for variant in ["GIVE ME A PROMPT", "give me a prompt", "Give Me A Prompt"] {
            let c = classify(variant);
            assert_eq!(c.kind, PromptRequestKind::PromptRequest, "{variant:?}");
        }
    }

    /// Ordinary prose / questions must NOT trigger the gate.
    #[test]
    fn negatives_do_not_fire() {
        let negatives = [
            "解释这段代码",
            "这是什么意思",
            "帮我看看这个 bug",
            "what does this function do?",
            "summarize this file",
            "运行测试",
            "fix the failing test",
            "这个 prompt engineering 技术怎么样", // bare 'prompt' as topic, not a request
            "我在写一个 promptbook 库",
            "我觉得任务卡体系不该限制 Codex",
            "删除任务卡前置门禁",
            "task cards should be handoff contracts",
            "taskcard validation is a separate concern",
        ];
        for n in negatives {
            let c = classify(n);
            assert_eq!(
                c.kind,
                PromptRequestKind::NotARequest,
                "input {n:?} should NOT be a task-card request (matched {:?})",
                c.matched_triggers
            );
            assert!(!c.is_task_card_request, "{n:?}");
        }
    }

    /// Precedence: an explicit task-card mention wins over handoff/prompt words.
    #[test]
    fn precedence_task_card_wins() {
        let c = classify("按这个方案出任务卡，然后交给 Claude Code 执行");
        assert_eq!(c.kind, PromptRequestKind::TaskCardRequest);
        assert!(c.matched_triggers.len() >= 2, "{:?}", c.matched_triggers);
        // handoff beats prompt
        let c2 = classify("写个 prompt 然后交给 cc 执行");
        assert_eq!(c2.kind, PromptRequestKind::Handoff);
    }

    /// JSON shape is stable (kind is snake_case; flags present).
    #[test]
    fn json_shape_stable() {
        let c = classify("给我提示词");
        let v = serde_json::to_value(&c).expect("serialize");
        assert_eq!(v["kind"], "prompt_request");
        assert_eq!(v["is_task_card_request"], true);
        assert!(v["matched_triggers"].is_array());
        assert!(v["trigger_lang"].is_array());
        assert_eq!(v["detected_advisory_intent"], false);
        assert_eq!(v["mutation_allowed"], true);

        let c2 = classify("评估一下这个方案");
        let v2 = serde_json::to_value(&c2).expect("serialize");
        assert_eq!(v2["kind"], "advisory_intent");
        assert_eq!(v2["detected_advisory_intent"], true);
        assert_eq!(v2["mutation_allowed"], false);
    }

    // ── Advisory Intent Tests ────────────────────────────────────────────────

    #[test]
    fn advisory_triggers_fire() {
        let positives = [
            "你看看是否加个路由之类的模块",
            "是否需要加一个缓存层",
            "要不要把这个拆成两个文件",
            "建议怎么做比较好",
            "评估一下这个方案的风险",
            "帮我评估一下可行性",
            "你觉得这样做合适吗",
            "分析一下这段代码的性能",
            "看看怎么样",
            "给个建议",
            "咨询一下架构设计",
            "你怎么看这个实现",
            "有没有必要加这个功能",
            "should we add a caching layer",
            "do you think this approach works",
            "would you recommend using redis",
            "assess whether we need a migration",
            "give me advice on the architecture",
            "what do you think about this design",
        ];
        for p in positives {
            let c = classify(p);
            assert!(
                c.detected_advisory_intent,
                "expected advisory intent for {p:?}, got {:?}",
                c.kind
            );
            assert!(
                !c.mutation_allowed,
                "mutation must be blocked for advisory {p:?}"
            );
            assert!(
                !c.is_task_card_request,
                "advisory must not be a task-card request: {p:?}"
            );
        }
    }

    #[test]
    fn advisory_negative_examples() {
        let negatives = [
            "按这个改",
            "实现这个功能",
            "修复 typo",
            "开始做",
            "帮我写个函数",
            "implement the feature",
            "fix the bug",
            "refactor this module",
        ];
        for n in negatives {
            let c = classify(n);
            assert!(
                !c.detected_advisory_intent,
                "input {n:?} should NOT be advisory (matched {:?})",
                c.matched_triggers
            );
        }
    }

    #[test]
    fn advisory_override_clears_block() {
        let inputs = [
            "评估一下这个方案，然后按这个改",
            "你觉得怎么样，开始实现",
            "分析一下然后开始实现",
            "你看看是否需要，动手吧",
        ];
        for p in inputs {
            let c = classify(p);
            assert!(
                c.detected_advisory_intent,
                "advisory must be detected for {p:?}"
            );
            assert!(
                c.mutation_allowed,
                "mutation must be allowed when override present: {p:?} (overrides: {:?})",
                c.advisory_override_triggers
            );
            assert!(
                !c.advisory_override_triggers.is_empty(),
                "override triggers must not be empty: {p:?}"
            );
        }
    }

    /// A consultation phrased as a question must NOT be cleared to mutate, even
    /// when it contains an imperative-looking phrase (adversarial-review finding).
    #[test]
    fn advisory_question_with_override_phrase_still_blocks() {
        let questions = [
            "should we go ahead with this refactor?",
            "should we start implementing this?",
            "do you think we should implement this?",
            "是否需要现在就动手？",
            "要不要直接开始实现？",
            "你觉得要不要去改这里？",
        ];
        for q in questions {
            let c = classify(q);
            assert!(
                c.detected_advisory_intent,
                "question must still be advisory: {q:?}"
            );
            assert!(
                !c.mutation_allowed,
                "a question must NOT clear advisory no-mutation: {q:?} (overrides: {:?})",
                c.advisory_override_triggers
            );
            assert!(
                c.advisory_override_triggers.is_empty(),
                "a question must collect no override triggers: {q:?}"
            );
        }
    }

    /// "go ahead" is no longer a bare override — it reads as part of a question.
    #[test]
    fn go_ahead_is_not_a_bare_override() {
        let c = classify("should we go ahead with this");
        assert!(c.detected_advisory_intent);
        assert!(
            !c.mutation_allowed,
            "'go ahead' must not clear advisory no-mutation"
        );
    }

    #[test]
    fn advisory_loses_to_task_card() {
        let c = classify("评估一下然后生成任务卡");
        assert_eq!(c.kind, PromptRequestKind::TaskCardRequest);
        assert!(c.is_task_card_request);
        assert!(c.detected_advisory_intent);
        assert!(c.mutation_allowed);
    }

    #[test]
    fn advisory_loses_to_handoff() {
        let c = classify("你觉得怎么样，交给 Claude Code");
        assert_eq!(c.kind, PromptRequestKind::Handoff);
        assert!(c.is_task_card_request);
        assert!(c.detected_advisory_intent);
        assert!(c.mutation_allowed);
    }

    #[test]
    fn advisory_does_not_affect_existing_gates() {
        let c = classify("给我提示词");
        assert_eq!(c.kind, PromptRequestKind::PromptRequest);
        assert!(c.is_task_card_request);
        assert!(!c.detected_advisory_intent);
        assert!(c.mutation_allowed);

        let c2 = classify("解释这段代码");
        assert_eq!(c2.kind, PromptRequestKind::NotARequest);
        assert!(!c2.is_task_card_request);
        assert!(!c2.detected_advisory_intent);
        assert!(c2.mutation_allowed);
    }

    #[test]
    fn bare_execute_does_not_override() {
        let c = classify("评估一下执行策略是否合理");
        assert!(c.detected_advisory_intent);
        assert!(
            !c.mutation_allowed,
            "bare '执行' in '执行策略' must NOT be an override"
        );
        assert!(c.advisory_override_triggers.is_empty());
    }

    #[test]
    fn governance_miss_well_formed() {
        let miss = GovernanceMiss::new(
            "bad_output_shape",
            "output_shape",
            "这是一段普通回答，不是任务卡\n第二行",
            Some("给我提示词"),
        );
        assert_eq!(miss.event, "governance_miss");
        assert_eq!(miss.detected_kind, "prompt_request");
        assert_eq!(miss.blocked_reason, "bad_output_shape");
        assert_eq!(miss.stage, "output_shape");
        assert!(!miss.sample_redacted.contains('\n'));
        assert!(!miss.matched_triggers.is_empty());

        // Without a request, kind is unclassified and triggers empty.
        let miss2 = GovernanceMiss::new("validation_failed", "validate", "x", None);
        assert_eq!(miss2.detected_kind, "unclassified");
        assert!(miss2.matched_triggers.is_empty());
    }

    #[test]
    fn redact_sample_bounds_length() {
        let long = "a".repeat(500);
        let r = redact_sample(&long, 160);
        // 160 chars + ellipsis
        assert_eq!(r.chars().count(), 161);
        assert!(r.ends_with('…'));
    }

    // ── Value Route Tests ────────────────────────────────────────────────────

    #[test]
    fn value_route_advisory_intent_is_read_only() {
        let c = classify("评估一下这个方案的风险");
        let vr = derive_value_route(&c, false, false);
        assert_eq!(vr.recommended_path, ValuePath::ReadOnlyAdvisory);
        assert!(
            vr.rejected_lighter.is_none(),
            "nothing is lighter than advisory"
        );
        assert_eq!(
            vr.rejected_heavier.as_ref().map(|r| r.path),
            Some(ValuePath::DirectEdit)
        );
        assert!(vr.requires_user_confirmation);
        assert!(!vr.needs_planner_judgment);
        assert!(vr.advisory);
    }

    #[test]
    fn value_route_request_without_instruction_is_plan_first() {
        let c = classify("给我提示词");
        let vr = derive_value_route(&c, false, false);
        assert_eq!(vr.recommended_path, ValuePath::PlanFirst);
        assert_eq!(
            vr.rejected_lighter.as_ref().map(|r| r.path),
            Some(ValuePath::DirectEdit)
        );
        assert_eq!(
            vr.rejected_heavier.as_ref().map(|r| r.path),
            Some(ValuePath::ClaudeCodeRoute)
        );
        assert!(vr.requires_user_confirmation);
        assert!(!vr.needs_planner_judgment);
    }

    #[test]
    fn value_route_request_with_instruction_is_claude_code_route() {
        let c = classify("按这个方案出任务卡交给 Claude Code 执行");
        let vr = derive_value_route(&c, true, false);
        assert_eq!(vr.recommended_path, ValuePath::ClaudeCodeRoute);
        assert_eq!(
            vr.rejected_lighter.as_ref().map(|r| r.path),
            Some(ValuePath::PlanFirst)
        );
        assert!(vr.rejected_heavier.is_none());
        assert!(!vr.requires_user_confirmation);
    }

    #[test]
    fn value_route_explicit_execution_authorization_is_direct_edit() {
        let request = "可以，开改";
        let c = classify(request);
        let vr = derive_value_route(&c, false, detect_current_task_approval(request));
        assert_eq!(vr.recommended_path, ValuePath::DirectEdit);
        assert_eq!(
            vr.rejected_lighter.as_ref().map(|r| r.path),
            Some(ValuePath::ReadOnlyAdvisory)
        );
        assert_eq!(
            vr.rejected_heavier.as_ref().map(|r| r.path),
            Some(ValuePath::ClaudeCodeRoute)
        );
        assert!(!vr.requires_user_confirmation);
    }

    #[test]
    fn value_route_default_is_plan_first_needs_judgment() {
        let c = classify("重构数据写入层并保留旧基线");
        let vr = derive_value_route(&c, false, false);
        assert_eq!(vr.recommended_path, ValuePath::PlanFirst);
        assert!(vr.needs_planner_judgment);
    }

    #[test]
    fn value_route_json_shape_stable() {
        let c = classify("给我提示词");
        let vr = derive_value_route(&c, false, false);
        let v = serde_json::to_value(&vr).expect("serialize");
        assert_eq!(v["recommended_path"], "plan-first");
        assert_eq!(v["rejected_lighter"]["path"], "direct-edit");
        assert_eq!(v["rejected_heavier"]["path"], "claude-code-route");
        assert_eq!(v["advisory"], true);
        assert!(v["rationale"].is_string());
        assert!(v["authority_note"].is_string());
        // The boundary note must name the four authorities it does NOT change.
        let note = v["authority_note"].as_str().unwrap();
        assert!(note.contains("permission mode"));
        assert!(note.contains("Review gate"));
        assert!(note.contains("Verification gate"));
    }

    #[test]
    fn value_path_str_roundtrips_serialized_form() {
        for (path, s) in [
            (ValuePath::ReadOnlyAdvisory, "read-only-advisory"),
            (ValuePath::DirectEdit, "direct-edit"),
            (ValuePath::PlanFirst, "plan-first"),
            (ValuePath::ClaudeCodeRoute, "claude-code-route"),
            (ValuePath::StopForScope, "stop-for-scope"),
        ] {
            assert_eq!(path.as_str(), s);
            assert_eq!(serde_json::to_value(path).unwrap(), s);
        }
    }

    // ── classify_demand ──────────────────────────────────────────────────────

    /// Each non-handoff demand kind fires on representative phrases.
    #[test]
    fn demand_kinds_fire() {
        let cases = [
            ("测试挂了，帮我看下", DemandKind::Debug),
            ("the build throws an error", DemandKind::Debug),
            ("stack trace 看不懂", DemandKind::Debug),
            ("做完了，验证一下", DemandKind::Verify),
            ("ready to commit, please verify", DemandKind::Verify),
            ("帮我做一次代码审查", DemandKind::CodeReview),
            ("please review my code", DemandKind::CodeReview),
            ("帮我写个commit", DemandKind::Commit),
            ("generate a conventional commit message", DemandKind::Commit),
            ("这块要重构一下", DemandKind::Architecture),
            ("let's refactor this module", DemandKind::Architecture),
            ("设计一个登录功能", DemandKind::Brainstorm),
            ("design a caching layer", DemandKind::Brainstorm),
            ("查一下 React useEffect 文档", DemandKind::DocsLookup),
            ("how do i use this api", DemandKind::DocsLookup),
        ];
        for (input, want) in cases {
            let d = classify_demand(input);
            assert_eq!(d.kind, want, "input {input:?} → {:?}", d.kind);
            assert!(!d.matched_triggers.is_empty(), "no triggers for {input:?}");
        }
    }

    /// Matt/Superpowers flow requests are specific engineering workflow asks:
    /// PRDs, issue slicing, decision maps, issue triage, branch/spec review,
    /// merge-conflict resolution, and context handoff. They should enter the
    /// Matt/Superpowers subroute instead of reading as ordinary prose.
    #[test]
    fn demand_matt_superpowers_flow_fires_on_specific_workflow_phrases() {
        let cases = [
            "turn this conversation into a PRD",
            "把这个方案整理成 PRD",
            "break this PRD into issues",
            "把这个计划拆成 issues",
            "make a decision map for this loose idea",
            "给这个松散想法做决策映射",
            "triage issue #42 into ready-for-agent",
            "resolve these merge conflicts",
            "write a handoff document for the next agent",
            "run a two-axis review since main",
        ];
        for input in cases {
            let d = classify_demand(input);
            assert_eq!(
                d.kind.as_str(),
                "matt-superpowers",
                "input {input:?} -> {:?}",
                d.kind
            );
            assert!(!d.matched_triggers.is_empty(), "no triggers for {input:?}");
        }
    }

    /// The Matt/Superpowers subroute must not steal AGS's existing executor
    /// handoff gate or broad code-review demand. Those are root governance
    /// concepts, not Matt workflow shortcuts.
    #[test]
    fn demand_matt_superpowers_does_not_steal_core_governance_demands() {
        assert_eq!(
            classify_demand("交给 Claude Code 执行").kind,
            DemandKind::TaskCardHandoff
        );
        assert_eq!(
            classify_demand("please review my code").kind,
            DemandKind::CodeReview
        );
    }

    /// Inflected English debug phrasings still route (recall hardening).
    #[test]
    fn demand_debug_inflections_fire() {
        for input in ["I'm getting errors", "it crashed", "the app keeps crashing"] {
            assert_eq!(classify_demand(input).kind, DemandKind::Debug, "{input:?}");
        }
    }

    /// `推荐` is verb-bound: a recommender-system *topic* must NOT route to
    /// Brainstorm, while a genuine "recommend me ..." request does.
    #[test]
    fn demand_recommend_is_verb_bound() {
        assert_eq!(
            classify_demand("推荐系统的代码在哪").kind,
            DemandKind::None,
            "recommender-system topic must not route"
        );
        assert_eq!(
            classify_demand("推荐一下用什么缓存方案").kind,
            DemandKind::Brainstorm
        );
    }

    /// Handoff/task-card demand is delegated to `classify` (no duplicated table).
    #[test]
    fn demand_handoff_delegates_to_classify() {
        for input in [
            "按这个方案出任务卡",
            "交给 Claude Code 执行",
            "give me a prompt",
        ] {
            let d = classify_demand(input);
            assert_eq!(d.kind, DemandKind::TaskCardHandoff, "{input:?}");
            assert!(!d.matched_triggers.is_empty(), "{input:?}");
        }
    }

    /// Ordinary prose / pure questions yield no demand.
    #[test]
    fn demand_none_on_prose() {
        for input in ["解释这段代码", "这是什么意思", "what does this function do"] {
            let d = classify_demand(input);
            assert_eq!(
                d.kind,
                DemandKind::None,
                "{input:?} matched {:?}",
                d.matched_triggers
            );
            assert!(d.matched_triggers.is_empty(), "{input:?}");
        }
    }

    /// Precedence: handoff outranks debug; debug outranks verify; verify
    /// outranks architecture.
    #[test]
    fn demand_precedence() {
        // handoff > debug
        assert_eq!(
            classify_demand("报错了，按这个方案出任务卡交给 cc 执行").kind,
            DemandKind::TaskCardHandoff
        );
        // debug > verify
        assert_eq!(
            classify_demand("done, but the test failure is back").kind,
            DemandKind::Debug
        );
        // verify > architecture
        assert_eq!(
            classify_demand("重构做完了，验证一下").kind,
            DemandKind::Verify
        );
        // code review > architecture
        assert_eq!(
            classify_demand("重构后帮我做一次代码审查").kind,
            DemandKind::CodeReview
        );
    }

    /// Spacing and casing must not change the verdict.
    #[test]
    fn demand_spacing_and_casing_insensitive() {
        for variant in ["REFACTOR this", "refactor this", "Refactor This"] {
            assert_eq!(
                classify_demand(variant).kind,
                DemandKind::Architecture,
                "{variant:?}"
            );
        }
    }

    /// Stable kebab-case identifiers, identical to the serialized form.
    #[test]
    fn demand_kind_str_roundtrips_serialized_form() {
        for (kind, s) in [
            (DemandKind::TaskCardHandoff, "task-card-handoff"),
            (DemandKind::Debug, "debug"),
            (DemandKind::Verify, "verify"),
            (DemandKind::CodeReview, "code-review"),
            (DemandKind::Commit, "commit"),
            (DemandKind::MattSuperpowers, "matt-superpowers"),
            (DemandKind::Architecture, "architecture"),
            (DemandKind::Brainstorm, "brainstorm"),
            (DemandKind::MailSend, "mail-send"),
            (DemandKind::Messaging, "messaging"),
            (DemandKind::Approval, "approval"),
            (DemandKind::CalendarQuery, "calendar-query"),
            (DemandKind::LarkDoc, "lark-doc"),
            (DemandKind::SheetOp, "sheet-op"),
            (DemandKind::LarkCollab, "lark-collab"),
            (DemandKind::DocsLookup, "docs-lookup"),
            (DemandKind::None, "none"),
        ] {
            assert_eq!(kind.as_str(), s);
            assert_eq!(serde_json::to_value(kind).unwrap(), s);
        }
    }

    /// Headline disambiguation: an email-send request that mentions a document
    /// must route to MailSend, NOT DocsLookup (which would wrongly hit context7).
    #[test]
    fn demand_mail_beats_docs() {
        let zh = classify_demand("把这个文档发邮件给张三");
        assert_eq!(
            zh.kind,
            DemandKind::MailSend,
            "triggers={:?}",
            zh.matched_triggers
        );
        let en = classify_demand("send an email with this doc to alice");
        assert_eq!(
            en.kind,
            DemandKind::MailSend,
            "triggers={:?}",
            en.matched_triggers
        );
    }

    /// Generic documentation lookup is NOT stolen by the platform-bound Lark
    /// doc triggers — it stays DocsLookup (→ context7).
    #[test]
    fn demand_generic_docs_stays_docs_lookup() {
        assert_eq!(
            classify_demand("帮我查 React useEffect 最新文档").kind,
            DemandKind::DocsLookup
        );
    }

    /// The new Lark demand kinds fire on representative phrases.
    #[test]
    fn demand_lark_kinds_fire() {
        assert_eq!(
            classify_demand("帮我查一下飞书日历今天安排").kind,
            DemandKind::CalendarQuery
        );
        assert_eq!(
            classify_demand("给飞书群发一条消息").kind,
            DemandKind::Messaging
        );
        assert_eq!(classify_demand("发起审批流程").kind, DemandKind::Approval);
        assert_eq!(
            classify_demand("打开飞书文档看一下").kind,
            DemandKind::LarkDoc
        );
        assert_eq!(
            classify_demand("更新飞书表格里的数据").kind,
            DemandKind::SheetOp
        );
        assert_eq!(
            classify_demand("帮我查一下飞书妙记").kind,
            DemandKind::LarkCollab
        );
    }

    /// Development demands win over external mutations (governance policy):
    /// a debug request that also asks to email the log routes to Debug.
    #[test]
    fn demand_dev_beats_external_mutation() {
        assert_eq!(
            classify_demand("报错了，把崩溃日志发邮件给我").kind,
            DemandKind::Debug
        );
    }

    #[test]
    fn demand_skill_authoring_detected() {
        for req in [
            "创建一个新技能",
            "帮我新建技能",
            "create a new skill",
            "author a skill for me",
            "use the skill-creator",
        ] {
            assert_eq!(
                classify_demand(req).kind,
                DemandKind::SkillAuthoring,
                "request `{req}` should classify as skill-authoring"
            );
        }
    }

    /// A bug report that merely mentions a skill must stay Debug (higher
    /// precedence), not be stolen by the skill-authoring triggers.
    #[test]
    fn demand_skill_authoring_does_not_steal_debug() {
        assert_eq!(
            classify_demand("这个新技能模块报错了").kind,
            DemandKind::Debug
        );
    }

    /// Co-occurrence: a creation verb + skill noun split by a skill NAME (which a
    /// single bounded substring cannot span) must still classify as
    /// skill-authoring. These are the exact reproduction phrases from the task.
    #[test]
    fn demand_skill_authoring_cooccurrence_spans_skill_name() {
        for req in [
            "创建一个新的 Hermes skill",
            "帮我创建一个新的 Hermes 技能",
            "build the Foo skill for me",
            "create a TempoFlow plugin",
            "做一个叫 Hermes 的技能",
        ] {
            let d = classify_demand(req);
            assert_eq!(
                d.kind,
                DemandKind::SkillAuthoring,
                "request `{req}` should classify as skill-authoring (matched {:?})",
                d.matched_triggers
            );
            assert!(
                !d.matched_triggers.is_empty(),
                "co-occurrence must record a matched trigger for `{req}`"
            );
        }
    }

    /// The co-occurrence rule must not over-fire: a creation verb with no skill
    /// noun, or a skill noun with no creation verb, stays out of skill-authoring.
    #[test]
    fn demand_skill_authoring_cooccurrence_requires_both_verb_and_noun() {
        // Verb, but no skill/plugin noun → must not become skill-authoring.
        assert_ne!(
            classify_demand("创建一个登录页面").kind,
            DemandKind::SkillAuthoring
        );
        // Skill noun, but no creation verb → not skill-authoring (plain prose).
        assert_eq!(
            classify_demand("这个 skill 在哪个目录").kind,
            DemandKind::None
        );
    }

    /// A calendar trigger must not steal a refactor request for a calendar module.
    #[test]
    fn demand_calendar_does_not_steal_refactor() {
        assert_eq!(
            classify_demand("重构日程模块的代码").kind,
            DemandKind::Architecture
        );
    }

    #[test]
    fn detect_current_task_approval_fires_on_execution_instructions() {
        for yes in [
            "按这个方案开始实现",
            "可以，开改",
            "一口气做完核验",
            "把它修复",
            "implement this now",
            "just get it done",
            "做完验证一下再说",
        ] {
            assert!(
                detect_current_task_approval(yes),
                "should detect approval: {yes:?}"
            );
        }
        for no in [
            "你看看这个执行策略对不对",
            "解释一下执行器的设计",
            "should we go ahead with this refactor?",
            "should we start implementing this?",
            "帮我查飞书日历",
        ] {
            assert!(
                !detect_current_task_approval(no),
                "should NOT detect approval: {no:?}"
            );
        }
    }
}
