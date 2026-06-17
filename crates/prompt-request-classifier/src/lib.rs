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
//! - **Recall-biased (fail-closed)** — when in doubt the classifier prefers to
//!   flag a request (so the gate engages) over missing it. A false positive
//!   merely reminds the host to emit a canonical task card; a false negative is
//!   the bypass this whole component exists to prevent.

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
    ("任务卡", PromptRequestKind::TaskCardRequest, "zh"),
    ("生成任务卡", PromptRequestKind::TaskCardRequest, "zh"),
    ("出任务卡", PromptRequestKind::TaskCardRequest, "zh"),
    ("task card", PromptRequestKind::TaskCardRequest, "en"),
    ("taskcard", PromptRequestKind::TaskCardRequest, "en"),
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
    ("handoff", PromptRequestKind::Handoff, "en"),
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
    ("按这个改", "zh"),
    ("开始实现", "zh"),
    ("落地这个方案", "zh"),
    ("开始做", "zh"),
    ("去改", "zh"),
    ("动手", "zh"),
    ("implement this", "en"),
    ("execute this", "en"),
    ("start implementing", "en"),
];

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
    let is_question = {
        let trimmed = request.trim_end();
        trimmed.ends_with('?') || trimmed.ends_with('？')
    };

    let mut advisory_override_triggers: Vec<String> = Vec::new();
    if detected_advisory_intent && !is_question {
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
/// has issued an explicit task-card instruction; `trivial_possible` marks a
/// small/local (typo-class) change. Deterministic: same inputs → same route.
///
/// `stop-for-scope` is part of the taxonomy but is never auto-recommended here —
/// it is a planner/host-selected escape hatch when scope, authority, or risk is
/// unclear, which cannot be inferred reliably from request text alone.
pub fn derive_value_route(
    classification: &Classification,
    task_card_requested: bool,
    trivial_possible: bool,
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
                    reason: "Handoff before the task-card instruction is premature — the three-gate threshold (方案 OK → 任务卡指令 → 路由) is not yet met.".to_string(),
                }),
                requires_user_confirmation: true,
                needs_planner_judgment: false,
                advisory: true,
                authority_note: note,
            }
        }
    } else if trivial_possible {
        ValueRoute {
            recommended_path: ValuePath::DirectEdit,
            rationale: "Small, local, low-risk change; edit in-session and verify with the narrowest relevant check.".to_string(),
            rejected_lighter: Some(RejectedPath {
                path: ValuePath::ReadOnlyAdvisory,
                reason: "A concrete fix was requested, not just advice.".to_string(),
            }),
            rejected_heavier: Some(RejectedPath {
                path: ValuePath::PlanFirst,
                reason: "A trivial, local change does not need a plan/confirmation gate.".to_string(),
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
            "任务卡",
            "帮我生成任务卡",
            "交给 Claude Code",
            "交给 Claude Code 执行",
            "给 CC 执行",
            "给 cc 执行这个",
            "写个 prompt",
            "写一个 prompt 给我",
            "handoff",
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

        let c2 = classify("任务卡");
        assert_eq!(c2.kind, PromptRequestKind::TaskCardRequest);
        assert!(c2.is_task_card_request);
        assert!(!c2.detected_advisory_intent);
        assert!(c2.mutation_allowed);

        let c3 = classify("解释这段代码");
        assert_eq!(c3.kind, PromptRequestKind::NotARequest);
        assert!(!c3.is_task_card_request);
        assert!(!c3.detected_advisory_intent);
        assert!(c3.mutation_allowed);
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
    fn value_route_trivial_is_direct_edit() {
        let c = classify("fix the typo in the readme");
        let vr = derive_value_route(&c, false, true);
        assert_eq!(vr.recommended_path, ValuePath::DirectEdit);
        assert_eq!(
            vr.rejected_lighter.as_ref().map(|r| r.path),
            Some(ValuePath::ReadOnlyAdvisory)
        );
        assert_eq!(
            vr.rejected_heavier.as_ref().map(|r| r.path),
            Some(ValuePath::PlanFirst)
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
}
