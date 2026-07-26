//! Closed handoff wire model and deterministic intent parsing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SCHEMA_VERSION: &str = "2.1-m4";

/// Known task-card field headers that the compiler can fill.
/// Legacy compact headers (路径/读取/关键路径/停止条件) are retained ONLY for
/// lenient intent parsing; they are never emitted in the rendered output.
pub(crate) const FIELD_HEADERS: &[(&str, bool)] = &[
    // inline fields
    ("Contract ID:", true),
    ("Handoff source:", true),
    ("Executor:", true),
    ("Runtime adapter:", true),
    ("Execution surface:", true),
    ("Permission mode:", true),
    ("Parallelism:", true),
    ("任务级别：", true),
    ("Execution effort:", true),
    ("Workflow authority:", true),
    // multi-line fields
    ("读取并遵守：", false),
    ("Review gate:", false),
    ("路径：", false),
    ("读取：", false),
    ("任务：", false),
    ("背景：", false),
    ("项目画像：", false),
    ("记忆胶囊：", false),
    ("任务存档：", false),
    ("适用治理文档：", false),
    ("目标文件夹路径：", false),
    ("相关路径：", false),
    ("本次任务相关文件：", false),
    ("目标：", false),
    ("验收标准：", false),
    ("非目标：", false),
    ("子任务编排：", false),
    ("实施要求：", false),
    ("关键路径：", false),
    ("验证：", false),
    ("Verification gate:", false),
    ("停止条件：", false),
    ("交付：", false),
];

/// Fields that are REQUIRED in the canonical (classic) task card.
/// The compiler must fill these or report them as missing. This mirrors the
/// validator's single required-field set (the classic fixed skeleton).
pub(crate) const REQUIRED_FIELDS: &[&str] = &[
    "读取并遵守：",
    "Contract ID:",
    "Handoff source:",
    "Executor:",
    "Runtime adapter:",
    "Execution surface:",
    "Permission mode:",
    "Parallelism:",
    "任务级别：",
    "Review gate:",
    "任务：",
    "背景：",
    "项目画像：",
    "记忆胶囊：",
    "任务存档：",
    "目标文件夹路径：",
    "相关路径：",
    "本次任务相关文件：",
    "目标：",
    "验收标准：",
    "非目标：",
    "验证：",
    "Verification gate:",
    "交付：",
];

/// Recognised intent keys (normalised — trailing `：`/`:` stripped).
/// Maps "raw header as found in input" → "canonical field name".
#[allow(dead_code)]
pub(crate) fn normalise_key(raw: &str) -> Option<&'static str> {
    for (header, _) in FIELD_HEADERS {
        if raw == *header {
            return Some(header);
        }
    }
    // Also accept colon-less Chinese keys
    match raw {
        "Contract ID" => Some("Contract ID:"),
        "Handoff source" => Some("Handoff source:"),
        "Executor" => Some("Executor:"),
        "Runtime adapter" => Some("Runtime adapter:"),
        "Execution surface" => Some("Execution surface:"),
        "Permission mode" => Some("Permission mode:"),
        "Parallelism" => Some("Parallelism:"),
        "Task level" => Some("任务级别："),
        "任务级别" => Some("任务级别："),
        "Execution effort" => Some("Execution effort:"),
        "Workflow authority" => Some("Workflow authority:"),
        "路径" => Some("路径："),
        "读取" => Some("读取："),
        "任务" => Some("任务："),
        "背景" => Some("背景："),
        "项目画像" => Some("项目画像："),
        "记忆胶囊" => Some("记忆胶囊："),
        "任务存档" => Some("任务存档："),
        "适用治理文档" => Some("适用治理文档："),
        "目标文件夹路径" => Some("目标文件夹路径："),
        "相关路径" => Some("相关路径："),
        "本次任务相关文件" => Some("本次任务相关文件："),
        "目标" => Some("目标："),
        "验收标准" => Some("验收标准："),
        "非目标" => Some("非目标："),
        "子任务编排" => Some("子任务编排："),
        "实施要求" => Some("实施要求："),
        "关键路径" => Some("关键路径："),
        "验证" => Some("验证："),
        "停止条件" => Some("停止条件："),
        "交付" => Some("交付："),
        _ => None,
    }
}

// ── Intent parsing ──────────────────────────────────────────────────────

/// Parsed compiler input: the key-value map from explicit contract headers,
/// plus any unscoped narrative retained so the contract gate can reject it.
#[derive(Debug, Clone)]
pub struct ParsedIntent {
    /// Map from canonical field header (e.g. `"任务："`) to its value.
    pub fields: HashMap<String, String>,
    /// Free text not matched to any known header.
    pub free_text: String,
}

pub const HANDOFF_CONTRACT_SCHEMA_VERSION: &str = "0.3.0-handoff-contract";

/// Structured origin of a newly compiled task card.
///
/// Host Plan mode is an alternative structured handoff signal: its final,
/// decision-complete artifact is the canonical task card. It is not the task
/// card's `Permission mode` and grants no execution authority by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffSource {
    ExplicitHandoff,
    HostPlanMode,
}

impl HandoffSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitHandoff => "explicit-handoff",
            Self::HostPlanMode => "host-plan-mode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskLevel {
    #[serde(alias = "light", alias = "LIGHT")]
    Light,
    #[serde(alias = "medium", alias = "MEDIUM")]
    Medium,
    #[serde(alias = "heavy", alias = "HEAVY")]
    Heavy,
}

impl TaskLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Heavy => "Heavy",
        }
    }
}

/// Typed, closed handoff seam for 0.3.0. `task_level` and `task` are mandatory;
/// callers cannot omit the level and ask the compiler to infer it from prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContract {
    pub schema_version: String,
    pub task_level: TaskLevel,
    pub task: String,
    #[serde(default)]
    pub fields: HashMap<String, String>,
}

/// Parse an intent string into a field map.
///
/// Lines that start with a recognised key followed by `:` or `：` are treated
/// as inline or multi-line field starts.  Lines that don't match any header
/// accumulate as free text.
pub fn parse_intent(input: &str) -> ParsedIntent {
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut free_lines: Vec<String> = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current_value = String::new();

    for raw_line in input.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            if current_key.is_some() {
                current_value.push('\n');
            } else {
                free_lines.push(String::new());
            }
            continue;
        }

        // Try to match as a field header: "Key: value" or "Key：value"
        // A field header line has the form: known_key followed by optional
        // whitespace, then value on the same line for inline fields, or
        // just the key for multi-line fields.
        if let Some((canonical, matched_len, _is_inline)) = find_header_start(trimmed) {
            // Flush previous multi-line field
            if let Some(key) = current_key.take() {
                let v = current_value.trim().to_string();
                if !v.is_empty() {
                    fields.insert(key, v);
                }
                current_value = String::new();
            }

            // Extract the rest after the matched prefix
            let rest = &trimmed[matched_len..];
            let value_part = rest.trim();

            if value_part.is_empty() {
                // Multi-line field — no value on header line
                current_key = Some(canonical.to_string());
            } else {
                // Inline value on same line
                fields.insert(canonical.to_string(), value_part.to_string());
            }
        } else if current_key.is_some() {
            // Continuation of current multi-line field
            current_value.push_str(raw_line);
            current_value.push('\n');
        } else {
            // Free text — not under any field header
            free_lines.push(raw_line.to_string());
        }
    }

    // Flush trailing multi-line field
    if let Some(key) = current_key {
        let v = current_value.trim().to_string();
        if !v.is_empty() {
            fields.insert(key, v);
        }
    }

    let free_text = free_lines.join("\n").trim().to_string();

    ParsedIntent { fields, free_text }
}

/// Whether the input is a structured handoff contract: it must carry an
/// explicit task field and contain no unscoped narrative fallback.
pub fn is_structured_contract_intent(intent: &str) -> bool {
    let parsed = parse_intent(intent);
    parsed.fields.contains_key("任务：") && parsed.free_text.trim().is_empty()
}

/// Check if a trimmed line starts with a known field header.
/// Returns (canonical_header, matched_byte_len, is_inline) if found.
///
/// `matched_byte_len` is the number of bytes consumed by the matched
/// prefix (including colon), so callers can extract the rest of the line.
fn find_header_start(line: &str) -> Option<(&'static str, usize, bool)> {
    // 1. Exact FIELD_HEADERS match
    for (header, is_inline) in FIELD_HEADERS {
        if line.starts_with(header) {
            return Some((header, header.len(), *is_inline));
        }
    }
    // 2. Alias match: normalize the key before the first : or ：
    if let Some(colon_pos) = line.find([':', '：']) {
        let key = line[..colon_pos].trim();
        if let Some(canonical) = normalise_key(key) {
            for (fh, is_inline) in FIELD_HEADERS {
                if *fh == canonical {
                    // `str::find` returns a BYTE offset. Fullwidth `：` is
                    // three UTF-8 bytes, so advancing by one would slice in the
                    // middle of the scalar and panic.
                    let colon_len = line[colon_pos..]
                        .chars()
                        .next()
                        .map(char::len_utf8)
                        .unwrap_or(0);
                    return Some((fh, colon_pos + colon_len, *is_inline));
                }
            }
        }
    }
    // 3. Colon-less inline match: "Key value" pattern
    //    When the line has no colon, try splitting on first whitespace.
    if let Some(space_pos) = line.find(char::is_whitespace) {
        let key = &line[..space_pos];
        if let Some(canonical) = normalise_key(key) {
            for (fh, is_inline) in FIELD_HEADERS {
                if *fh == canonical {
                    // matched_len = position after the key (skip whitespace too)
                    let rest_start = line[space_pos..]
                        .find(|c: char| !c.is_whitespace())
                        .map(|p| space_pos + p)
                        .unwrap_or(line.len());
                    return Some((fh, rest_start, *is_inline));
                }
            }
        }
    }
    None
}
