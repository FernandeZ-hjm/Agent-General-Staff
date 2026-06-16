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
            PromptRequestKind::NotARequest => "not_a_request",
        }
    }
}

/// The result of classifying a single user request.
#[derive(Debug, Clone, Serialize)]
pub struct Classification {
    /// The highest-precedence kind matched
    /// (TaskCardRequest > Handoff > PromptRequest > NotARequest).
    pub kind: PromptRequestKind,
    /// `true` for any positive kind — the AGS task-card pipeline must engage.
    pub is_task_card_request: bool,
    /// Every trigger phrase that matched, in their authored form.
    pub matched_triggers: Vec<String>,
    /// Distinct languages of the matched triggers (`"zh"` / `"en"`).
    pub trigger_lang: Vec<String>,
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
            PromptRequestKind::NotARequest => {}
        }
    }

    // Precedence: TaskCardRequest > Handoff > PromptRequest > NotARequest.
    let kind = if has_task_card {
        PromptRequestKind::TaskCardRequest
    } else if has_handoff {
        PromptRequestKind::Handoff
    } else if has_prompt {
        PromptRequestKind::PromptRequest
    } else {
        PromptRequestKind::NotARequest
    };

    let is_task_card_request = kind != PromptRequestKind::NotARequest;

    Classification {
        kind,
        is_task_card_request,
        matched_triggers,
        trigger_lang: langs,
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
}
