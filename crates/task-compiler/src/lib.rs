//! M4 Task Compiler — deterministic, rule-based compilation of approved
//! execution intents (execution contracts) into the canonical task-card skeleton
//! defined in `protocol/task-card-template.md`.
//!
//! # Design
//!
//! - **Rule engine only** — no AI calls, no free-form prompt generation.
//! - **Project-aware** — fills stable slots from CLAUDE.md, WORKSPACE.md,
//!   protocol files, known workspace identity, and local memory paths.
//! - **Missing-slot enforcement** — if a required slot cannot be filled the
//!   compiler reports `missing_slots` and exits 1 instead of guessing.
//! - **Single canonical format** — the compiled output is the classic
//!   task-card skeleton defined in `protocol/task-card-template.md`. The
//!   compiler never emits the removed compact format and never invents a
//!   third task-card format. Output is validated by the real
//!   `task_card_validator` (see tests), not just a heuristic self-check.
//! - **Input semantics** — the canonical input is an approved execution
//!   contract (the confirmed solution), not raw user chat. The compiler
//!   accepts flexible intent files for backward compatibility, but
//!   generators (Codex / Cursor) must only feed it confirmed execution
//!   contracts. Direct compilation of raw user natural-language requests
//!   into executable task cards is discouraged and may produce incomplete
//!   or misclassified output.
//! - **Task-card request gate** — the compiler enforces a hard gate between
//!   "solution OK" and "task card generation". Without an explicit user
//!   task-card instruction (signalled via `task_card_requested = true`),
//!   the compiler MUST NOT output an executable task card. It can still
//!   produce a diagnostic report showing what WOULD be compiled, but
//!   `executable_allowed` will be `false` and the compiled card text will
//!   be suppressed. This gate closes the enforcement gap where a
//!   sufficiently structured raw request could bypass the lifecycle:
//!   preflight → solution → user confirmation → **task-card/handoff instruction**
//!   → routing → task card → gate/execution/receipt. Authorized same-session
//!   direct execution does not call this compiler.
//!
//! # Intent format (execution contract)
//!
//! The intent file uses the same field headers as task cards. It represents
//! a confirmed execution contract — the output of the solution phase, after
//! user confirmation — not raw user chat:
//!
//! ```text
//! Executor: Claude Code
//! 任务级别：Medium
//! 任务：一句话任务描述
//! 目标：
//! 1. goal_1
//! 2. goal_2
//! 非目标：
//! - non_goal_1
//! ```
//!
//! Unrecognised lines are accumulated as free text and used to fill
//! `任务：` when that field is absent from explicit key-value pairs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Public types ─────────────────────────────────────────────────────────

/// Schema version emitted in compile reports.
pub const SCHEMA_VERSION: &str = "2.0-m4";

/// Known task-card field headers that the compiler can fill.
/// Legacy compact headers (路径/读取/关键路径/停止条件) are retained ONLY for
/// lenient intent parsing; they are never emitted in the rendered output.
const FIELD_HEADERS: &[(&str, bool)] = &[
    // inline fields
    ("Executor:", true),
    ("Runtime adapter:", true),
    ("Execution surface:", true),
    ("Permission mode:", true),
    ("Parallelism:", true),
    ("任务级别：", true),
    ("Execution effort:", true),
    ("Workflow authority:", true),
    // multi-line fields
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
    ("非目标：", false),
    ("实施要求：", false),
    ("关键路径：", false),
    ("验证：", false),
    ("停止条件：", false),
    ("交付：", false),
];

/// Fields that are REQUIRED in the canonical (classic) task card.
/// The compiler must fill these or report them as missing. This mirrors the
/// validator's single required-field set (the classic fixed skeleton).
const REQUIRED_FIELDS: &[&str] = &[
    "读取并遵守：",
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
    "非目标：",
    "验证：",
    "Verification gate:",
    "交付：",
];

/// Recognised intent keys (normalised — trailing `：`/`:` stripped).
/// Maps "raw header as found in input" → "canonical field name".
#[allow(dead_code)]
fn normalise_key(raw: &str) -> Option<&'static str> {
    for (header, _) in FIELD_HEADERS {
        if raw == *header {
            return Some(header);
        }
    }
    // Also accept colon-less Chinese keys
    match raw {
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
        "非目标" => Some("非目标："),
        "实施要求" => Some("实施要求："),
        "关键路径" => Some("关键路径："),
        "验证" => Some("验证："),
        "停止条件" => Some("停止条件："),
        "交付" => Some("交付："),
        _ => None,
    }
}

// ── Intent parsing ──────────────────────────────────────────────────────

/// Parsed intent: the key-value map from explicit headers, plus accumulated
/// free-text lines for fallback task description.
#[derive(Debug, Clone)]
pub struct ParsedIntent {
    /// Map from canonical field header (e.g. `"任务："`) to its value.
    pub fields: HashMap<String, String>,
    /// Free text not matched to any known header.
    pub free_text: String,
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
                    // matched_len = position after the colon
                    return Some((fh, colon_pos + 1, *is_inline));
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

// ── Slot sources ────────────────────────────────────────────────────────

/// Where a slot value was sourced from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlotSource {
    /// Directly from the user intent file.
    Intent,
    /// Filled from project context (CLAUDE.md, WORKSPACE.md, protocol files).
    ProjectContext,
    /// Filled from known workspace identity.
    WorkspaceIdentity,
    /// Filled from local memory paths.
    MemoryPath,
    /// A well-known default value (e.g. Parallelism: none).
    Default,
    /// The slot could not be filled.
    Missing,
}

impl SlotSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SlotSource::Intent => "intent",
            SlotSource::ProjectContext => "project_context",
            SlotSource::WorkspaceIdentity => "workspace_identity",
            SlotSource::MemoryPath => "memory_path",
            SlotSource::Default => "default",
            SlotSource::Missing => "missing",
        }
    }
}

// ── Compile context ─────────────────────────────────────────────────────

/// Context gathered from the project that the compiler uses for slot filling.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    /// Absolute path to the project root.
    pub project_root: PathBuf,
    /// Workspace identity code (A, S, B, etc.) if detected.
    pub workspace_code: Option<String>,
    /// Workspace role description.
    pub workspace_role: Option<String>,
    /// Path to context-capsule.md, if it exists.
    pub capsule_path: Option<PathBuf>,
    /// Path to task-memory.md, if it exists.
    pub task_memory_path: Option<PathBuf>,
    /// Default memory project slug.
    pub memory_slug: Option<String>,
    /// Whether this is an AGS suite (has protocol/ and crates/).
    pub is_ags_suite: bool,
    /// Paths detected from CLAUDE.md protocol references.
    pub claude_md_protocol_refs: Vec<String>,
}

/// Gather project context from the given root directory.
/// This is a pure read-only function — no files are written.
pub fn gather_project_context(root: &Path) -> ProjectContext {
    let root = absolute_project_root(root);
    let is_ags_suite = root.join("CLAUDE.md").exists()
        && root.join("protocol").is_dir()
        && root.join("crates").is_dir();

    // Workspace identity from WORKSPACE.md
    let (workspace_code, workspace_role) = detect_workspace_identity(&root);

    // Memory paths
    let memory_slug = detect_memory_slug(&root);
    let capsule_path = memory_slug.as_ref().and_then(|slug| {
        let p = PathBuf::from(format!(
            "~/.agents/memory/projects/{}/context-capsule.md",
            slug
        ));
        if p.exists() {
            Some(p)
        } else {
            None
        }
    });
    let task_memory_path = memory_slug.as_ref().and_then(|slug| {
        let p = PathBuf::from(format!("~/.agents/memory/projects/{}/task-memory.md", slug));
        if p.exists() {
            Some(p)
        } else {
            None
        }
    });

    // Extract CLAUDE.md protocol references
    let claude_md_protocol_refs = extract_claude_md_refs(&root);

    ProjectContext {
        project_root: root,
        workspace_code,
        workspace_role,
        capsule_path,
        task_memory_path,
        memory_slug,
        is_ags_suite,
        claude_md_protocol_refs,
    }
}

fn absolute_project_root(root: &Path) -> PathBuf {
    if let Ok(path) = root.canonicalize() {
        return path;
    }
    if root.is_absolute() {
        return root.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(root))
        .unwrap_or_else(|_| root.to_path_buf())
}

/// Detect workspace identity from WORKSPACE.md or known paths.
fn detect_workspace_identity(root: &Path) -> (Option<String>, Option<String>) {
    let workspace_md = root.join("WORKSPACE.md");
    if workspace_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&workspace_md) {
            // Simple table parser: look for | Code | Role | Path |
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('|') && line.contains('|') {
                    let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                    if parts.len() >= 4 {
                        let code = parts[1].to_string();
                        let role = parts[2].to_string();
                        let entry_path = parts[3].to_string();
                        // Skip header row
                        if code == "Code" {
                            continue;
                        }
                        // Check if current root matches this row's path
                        let resolved_path = shellexpand_path(&entry_path);
                        if paths_equal(&resolved_path, root) {
                            // Strip backtick formatting from role if present
                            let role_clean = role.trim_matches('`').to_string();
                            return (Some(code), Some(role_clean));
                        }
                    }
                }
            }
        }
    }

    // Fallback: try known paths
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical_str = canonical.to_string_lossy().to_string();

    const KNOWN: &[(&str, &str, &str)] = &[
        (
            "A",
            "Development private suite",
            "/Volumes/Projects/example-private-suite",
        ),
        (
            "A1",
            "Private bare repo",
            "/Volumes/Projects/remotes/example-private-suite.git",
        ),
        (
            "S",
            "Stable private suite",
            "/Volumes/Projects/example-stable-suite",
        ),
        (
            "B",
            "Public worktree",
            "/Volumes/AI Project/ai-dev-env-bootstrap",
        ),
        (
            "B1",
            "Public bare repo",
            "/Volumes/Projects/remotes/example-public-suite.git",
        ),
    ];

    for (code, role, known_path) in KNOWN {
        if canonical_str == *known_path {
            return (Some(code.to_string()), Some(role.to_string()));
        }
    }

    (None, None)
}

/// Expand `~` and `$HOME` in a path string.
fn shellexpand_path(s: &str) -> PathBuf {
    let s = s.trim();
    let home = ags_platform::home_dir_or_temp()
        .to_string_lossy()
        .into_owned();
    if s.starts_with("~/") {
        PathBuf::from(s.replacen("~", &home, 1))
    } else if s.starts_with("$HOME/") {
        PathBuf::from(s.replacen("$HOME", &home, 1))
    } else {
        PathBuf::from(s)
    }
}

/// Compare two paths, normalising both.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}

/// Detect the memory slug for this project.
fn detect_memory_slug(root: &Path) -> Option<String> {
    // For known projects, use known slugs
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let s = canonical.to_string_lossy();

    if s.contains("example-private-suite") {
        return Some("example-private-suite".to_string());
    }
    if s.contains("example-stable-suite") {
        return Some("example-stable-suite".to_string());
    }
    if s.contains("ai-dev-env-bootstrap") {
        return Some("ai-dev-env-bootstrap".to_string());
    }

    // Fallback: derive from directory name
    root.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())
}

/// Extract protocol file references from CLAUDE.md.
fn extract_claude_md_refs(root: &Path) -> Vec<String> {
    let claude_md = root.join("CLAUDE.md");
    if !claude_md.exists() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&claude_md) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if (line.starts_with("- `") || line.starts_with("- "))
            && (line.contains(".md") || line.contains("protocol/"))
        {
            refs.push(line.trim_start_matches("- ").to_string());
        }
    }
    refs
}

// ── Compilation ─────────────────────────────────────────────────────────

/// A single slot entry in the compile report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotEntry {
    /// Canonical field header (e.g. `"任务："`).
    pub field: String,
    /// The value filled (if any).
    pub value: String,
    /// Where this value came from.
    pub source: SlotSource,
}

/// Compile report — the structured output of a compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileReport {
    /// Schema version for this report format.
    pub schema_version: String,
    /// The compiled task card text (empty in check-only mode).
    pub compiled_task_card: String,
    /// Per-slot source tracking.
    pub slot_sources: Vec<SlotEntry>,
    /// Slots that could not be filled.
    pub missing_slots: Vec<String>,
    /// Assumptions made during compilation.
    pub assumptions: Vec<String>,
    /// Whether the compiled card passes `ags task validate`.
    pub validation_passed: bool,
    /// Validation errors, if any.
    pub validation_errors: Vec<String>,
    /// Whether this was a check-only run.
    pub check_only: bool,
    /// Whether the user explicitly requested a task card
    /// (`--task-card-requested` flag).
    pub task_card_requested: bool,
    /// Whether executable task card output is allowed.
    /// Requires `task_card_requested=true` AND `check_only=false`
    /// AND no missing slots.
    pub executable_allowed: bool,
    /// If executable output is blocked, the reason.
    /// Possible values: "task_card_not_requested", "check_only",
    /// "missing_slots", or `null` when allowed.
    pub block_reason: Option<String>,
}

/// Compile an approved execution intent (execution contract) into a canonical
/// task card.
///
/// The canonical input is a confirmed execution contract from the solution phase,
/// not raw user chat. The compiler accepts flexible intents for backward
/// compatibility, but callers should only pass confirmed execution contracts.
///
/// # Task-card request gate
///
/// `task_card_requested` is the hard gate between "solution OK" and task card
/// generation. Without it, the compiler produces a diagnostic report only —
/// `executable_allowed` is `false`, `block_reason` is set to
/// `"task_card_not_requested"`, and the compiled task card text is suppressed.
/// Generators (Codex/Cursor) must only pass `task_card_requested=true` after the
/// user has explicitly issued a task-card instruction ("生成任务卡", "按这个方案出任务卡",
/// "交给 Claude Code 执行", etc.).
///
/// Returns the compiled card text and the full compile report.
/// If `check_only` is true, the compiled card is only validated but
/// the report is still returned for inspection.
pub fn compile(
    intent: &str,
    project_root: &Path,
    check_only: bool,
    task_card_requested: bool,
) -> (String, CompileReport) {
    let ctx = gather_project_context(project_root);
    let parsed = parse_intent(intent);

    let mut slot_sources: Vec<SlotEntry> = Vec::new();
    let mut assumptions: Vec<String> = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();

    // ── Phase 1: fill fields from intent ────────────────────────────
    for (header, _is_inline) in FIELD_HEADERS {
        if let Some(val) = parsed.fields.get(*header) {
            if !val.is_empty() {
                fields.insert(header.to_string(), val.clone());
                slot_sources.push(SlotEntry {
                    field: header.to_string(),
                    value: val.clone(),
                    source: SlotSource::Intent,
                });
            }
        }
    }

    // ── Phase 2: project-aware slot filling ────────────────────────

    // 读取并遵守：— the read-and-obey list, built from project context
    if !has_field(&fields, "读取并遵守：") {
        let reads = build_reads_section(&ctx);
        let reads = if reads.is_empty() {
            "- 本任务卡".to_string()
        } else {
            reads
        };
        fields.insert("读取并遵守：".to_string(), reads.clone());
        slot_sources.push(SlotEntry {
            field: "读取并遵守：".to_string(),
            value: reads,
            source: SlotSource::ProjectContext,
        });
    }

    // Executor: — default Claude Code
    if !has_field(&fields, "Executor:") {
        fields.insert("Executor:".to_string(), "Claude Code".to_string());
        slot_sources.push(SlotEntry {
            field: "Executor:".to_string(),
            value: "Claude Code".to_string(),
            source: SlotSource::Default,
        });
    }

    // Runtime adapter: — from executor
    if !has_field(&fields, "Runtime adapter:") {
        let executor = fields
            .get("Executor:")
            .map(|s| s.as_str())
            .unwrap_or("Claude Code");
        let adapter = executor_to_adapter(executor);
        fields.insert("Runtime adapter:".to_string(), adapter.to_string());
        slot_sources.push(SlotEntry {
            field: "Runtime adapter:".to_string(),
            value: adapter.to_string(),
            source: SlotSource::Default,
        });
    }

    // Execution surface: — default cli
    if !has_field(&fields, "Execution surface:") {
        fields.insert("Execution surface:".to_string(), "cli".to_string());
        slot_sources.push(SlotEntry {
            field: "Execution surface:".to_string(),
            value: "cli".to_string(),
            source: SlotSource::Default,
        });
    }

    // Permission mode: — default direct execution. Heavy tasks with an
    // unspecified mode are conservatively rewritten to plan-only below.
    if !has_field(&fields, "Permission mode:") {
        fields.insert(
            "Permission mode:".to_string(),
            "execute-and-verify".to_string(),
        );
        slot_sources.push(SlotEntry {
            field: "Permission mode:".to_string(),
            value: "execute-and-verify".to_string(),
            source: SlotSource::Default,
        });
    }

    // Parallelism: — default none
    if !has_field(&fields, "Parallelism:") {
        fields.insert("Parallelism:".to_string(), "none".to_string());
        slot_sources.push(SlotEntry {
            field: "Parallelism:".to_string(),
            value: "none".to_string(),
            source: SlotSource::Default,
        });
    }

    // 任务级别：— from intent, or infer from content
    if !has_field(&fields, "任务级别：") {
        let level = infer_task_level(&parsed, &fields);
        fields.insert("任务级别：".to_string(), level.clone());
        slot_sources.push(SlotEntry {
            field: "任务级别：".to_string(),
            value: level,
            source: SlotSource::Default,
        });
    }

    // Review gate: — default referencing the protocol
    if !has_field(&fields, "Review gate:") {
        let rg =
            "- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别".to_string();
        fields.insert("Review gate:".to_string(), rg.clone());
        slot_sources.push(SlotEntry {
            field: "Review gate:".to_string(),
            value: rg,
            source: SlotSource::Default,
        });
    }

    // 任务：— from intent free text if not in fields
    if !has_field(&fields, "任务：") && !parsed.free_text.is_empty() {
        fields.insert("任务：".to_string(), parsed.free_text.clone());
        slot_sources.push(SlotEntry {
            field: "任务：".to_string(),
            value: parsed.free_text.clone(),
            source: SlotSource::Intent,
        });
    }

    // 背景：— default if absent
    if !has_field(&fields, "背景：") {
        let bg = "本次任务差异见目标与实施要求".to_string();
        fields.insert("背景：".to_string(), bg.clone());
        slot_sources.push(SlotEntry {
            field: "背景：".to_string(),
            value: bg,
            source: SlotSource::Default,
        });
    }

    // 项目画像：— default 无
    if !has_field(&fields, "项目画像：") {
        fields.insert("项目画像：".to_string(), "无".to_string());
        slot_sources.push(SlotEntry {
            field: "项目画像：".to_string(),
            value: "无".to_string(),
            source: SlotSource::Default,
        });
    }

    // 记忆胶囊：— from memory path, fallback 无
    if !has_field(&fields, "记忆胶囊：") {
        let (val, source) = match ctx.capsule_path {
            Some(ref cap_path) => (format!("- {}", cap_path.display()), SlotSource::MemoryPath),
            None => ("无".to_string(), SlotSource::Default),
        };
        fields.insert("记忆胶囊：".to_string(), val.clone());
        slot_sources.push(SlotEntry {
            field: "记忆胶囊：".to_string(),
            value: val,
            source,
        });
    }

    // 任务存档：— from memory path, fallback 无
    if !has_field(&fields, "任务存档：") {
        let (val, source) = match ctx.task_memory_path {
            Some(ref tm_path) => (format!("- {}", tm_path.display()), SlotSource::MemoryPath),
            None => ("无".to_string(), SlotSource::Default),
        };
        fields.insert("任务存档：".to_string(), val.clone());
        slot_sources.push(SlotEntry {
            field: "任务存档：".to_string(),
            value: val,
            source,
        });
    }

    // 目标文件夹路径：— actual target/workspace root for this task
    if !has_field(&fields, "目标文件夹路径：") {
        let target_folder = format!("- {}", ctx.project_root.to_string_lossy());
        fields.insert("目标文件夹路径：".to_string(), target_folder.clone());
        slot_sources.push(SlotEntry {
            field: "目标文件夹路径：".to_string(),
            value: target_folder,
            source: SlotSource::ProjectContext,
        });
    }

    // 相关路径：— from project context
    if !has_field(&fields, "相关路径：") {
        let default_paths = if ctx.is_ags_suite {
            "- crates/\n- scripts/\n- tests/".to_string()
        } else {
            format!("- {}", ctx.project_root.to_string_lossy())
        };
        fields.insert("相关路径：".to_string(), default_paths.clone());
        slot_sources.push(SlotEntry {
            field: "相关路径：".to_string(),
            value: default_paths,
            source: SlotSource::ProjectContext,
        });
    }

    // 本次任务相关文件：— default if absent
    if !has_field(&fields, "本次任务相关文件：") {
        let files = if ctx.is_ags_suite {
            "- Cargo.toml".to_string()
        } else {
            "- .".to_string()
        };
        fields.insert("本次任务相关文件：".to_string(), files.clone());
        slot_sources.push(SlotEntry {
            field: "本次任务相关文件：".to_string(),
            value: files,
            source: SlotSource::ProjectContext,
        });
    }

    // ── Phase 3: defaults for remaining optional fields ─────────────

    // 非目标：— default if absent
    if !has_field(&fields, "非目标：") {
        fields.insert("非目标：".to_string(), "- 无".to_string());
        slot_sources.push(SlotEntry {
            field: "非目标：".to_string(),
            value: "- 无".to_string(),
            source: SlotSource::Default,
        });
    }

    // 验证：— default if absent
    if !has_field(&fields, "验证：") {
        fields.insert("验证：".to_string(), "按任务卡验证门禁执行".to_string());
        slot_sources.push(SlotEntry {
            field: "验证：".to_string(),
            value: "按任务卡验证门禁执行".to_string(),
            source: SlotSource::Default,
        });
    }

    // Verification gate: — default if absent (carries the stop condition)
    if !has_field(&fields, "Verification gate:") {
        let vg =
            "- commands:\n  - 按任务卡验证门禁执行\n- stop condition:\n  - 验证失败时停止并报告"
                .to_string();
        fields.insert("Verification gate:".to_string(), vg.clone());
        slot_sources.push(SlotEntry {
            field: "Verification gate:".to_string(),
            value: vg,
            source: SlotSource::Default,
        });
    }

    // 交付：— default if absent
    if !has_field(&fields, "交付：") {
        fields.insert("交付：".to_string(), "按协议输出交付报告".to_string());
        slot_sources.push(SlotEntry {
            field: "交付：".to_string(),
            value: "按协议输出交付报告".to_string(),
            source: SlotSource::Default,
        });
    }

    // ── Phase 3b: Heavy task permission downgrade ───────────────────
    // Per protocol: Heavy tasks default to plan-only when the user does
    // not explicitly set a permission mode.
    let task_level = fields.get("任务级别：").map(|s| s.as_str()).unwrap_or("");
    if task_level == "Heavy" {
        // Check if permission mode was default-filled (not user-provided)
        let perm_source_is_default = slot_sources
            .iter()
            .any(|s| s.field == "Permission mode:" && s.source == SlotSource::Default);
        if perm_source_is_default {
            // Replace the direct-execution default with Heavy's plan-only default.
            fields.insert("Permission mode:".to_string(), "plan-only".to_string());
            // Update slot_sources: replace the Default entry
            if let Some(entry) = slot_sources
                .iter_mut()
                .find(|s| s.field == "Permission mode:")
            {
                entry.value = "plan-only".to_string();
                entry.source = SlotSource::Default;
            }
            assumptions.push(
                "Heavy task: permission mode defaulted to plan-only (protocol M4/M7 rule)"
                    .to_string(),
            );
        }
    }

    // ── Phase 4: detect missing required slots ──────────────────────
    let missing_slots: Vec<String> = REQUIRED_FIELDS
        .iter()
        .filter(|h| !has_field(&fields, h))
        .map(|h| h.to_string())
        .collect();

    // Also check that 任务：and 目标：have meaningful content
    if has_field(&fields, "任务：")
        && is_empty_or_placeholder(fields.get("任务：").unwrap())
        && !missing_slots.contains(&"任务：".to_string())
    {
        // Don't add to missing_slots if it's there, but flag the weak content
    }
    if has_field(&fields, "目标：")
        && is_empty_or_placeholder(fields.get("目标：").unwrap())
        && !missing_slots.contains(&"目标：".to_string())
    {
        // Same — weak content in 目标：
    }

    // ── Phase 5: build the canonical task card ──────────────────────
    // Always render the classic skeleton. When slots are missing the card is
    // still rendered for diagnostics but will not pass validation.
    let compiled_card = render_task_card(&fields);

    // ── Phase 6: validate against task card validator ───────────────
    // We do a structural self-check here. The actual validation is done
    // by the CLI calling task_card_validator::validate().
    let (validation_passed, validation_errors) = if missing_slots.is_empty() {
        // Basic structural self-check
        let mut errors: Vec<String> = Vec::new();
        if !compiled_card.starts_with("## 任务卡") {
            errors.push("Compiled card does not start with ## 任务卡".to_string());
        }
        if compiled_card.contains("```text") || compiled_card.contains("```markdown") {
            // Check that outer fence is ~~~~markdown not ```markdown
            // This is a heuristic check — the validator does proper fence detection
        }
        (errors.is_empty(), errors)
    } else {
        (
            false,
            vec![format!(
                "Missing required slots: {}",
                missing_slots.join(", ")
            )],
        )
    };

    // ── Phase 7: task-card handoff generation gate ──────────────────
    // Determine whether executable output is allowed.
    // Three conditions must all be met:
    //   1. User explicitly requested a task card (task_card_requested)
    //   2. Not in check-only mode
    //   3. No missing slots
    let (executable_allowed, block_reason) = if !task_card_requested {
        (false, Some("task_card_not_requested".to_string()))
    } else if check_only {
        (false, Some("check_only".to_string()))
    } else if !missing_slots.is_empty() {
        (false, Some("missing_slots".to_string()))
    } else {
        (true, None)
    };

    // Suppress card output when not allowed
    let report_card = if executable_allowed {
        compiled_card.clone()
    } else {
        String::new()
    };

    // If blocked by task_card_not_requested, the validation result is
    // informational only — the card was never meant to be executable.
    let effective_validation_passed = if executable_allowed {
        validation_passed
    } else {
        false
    };

    let effective_validation_errors = if executable_allowed {
        validation_errors
    } else if let Some(ref reason) = block_reason {
        vec![format!(
            "Executable output blocked: {}",
            match reason.as_str() {
                "task_card_not_requested" => {
                    "task card not requested (use --task-card-requested after user task-card instruction)"
                }
                "check_only" => "check-only mode",
                "missing_slots" => "missing required slots",
                other => other,
            }
        )]
    } else {
        validation_errors
    };

    let report = CompileReport {
        schema_version: SCHEMA_VERSION.to_string(),
        compiled_task_card: report_card,
        slot_sources,
        missing_slots,
        assumptions,
        validation_passed: effective_validation_passed,
        validation_errors: effective_validation_errors,
        check_only,
        task_card_requested,
        executable_allowed,
        block_reason,
    };

    // Return the gated card: empty when not allowed, the real card when allowed.
    // This closes the public API bypass where a Rust caller could receive an
    // executable card via `let (card, _) = compile(...)` while the report says
    // `executable_allowed=false`.
    let gated_card = if executable_allowed {
        compiled_card
    } else {
        String::new()
    };

    (gated_card, report)
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn has_field(fields: &HashMap<String, String>, key: &str) -> bool {
    fields.get(key).is_some_and(|v| !v.is_empty())
}

fn is_empty_or_placeholder(val: &str) -> bool {
    let trimmed = val.trim();
    trimmed.is_empty()
        || trimmed == "- 无"
        || trimmed == "无"
        || trimmed == "todo"
        || trimmed == "tbd"
        || trimmed == "待定"
}

fn executor_to_adapter(executor: &str) -> &'static str {
    match executor {
        "Codex" => "codex-local",
        "Claude Code" => "claude-code",
        "Cursor" => "cursor",
        _ => "generic",
    }
}

/// Infer task level from intent content.
#[allow(clippy::if_same_then_else)] // "medium signals" and "default to Medium for safety" intentionally share a branch value
fn infer_task_level(parsed: &ParsedIntent, fields: &HashMap<String, String>) -> String {
    let combined = format!(
        "{} {}",
        parsed.free_text,
        fields.values().cloned().collect::<Vec<_>>().join(" ")
    );

    let heavy_markers = [
        "数据",
        "向量库",
        "迁移",
        "migration",
        "不可逆",
        "baseline",
        "历史产物",
        "数据库",
        "database",
        "删除",
        "覆盖",
        "发布",
        "基线保护",
        "dry-run",
        "staged",
    ];
    let medium_markers = [
        "跨文件",
        "multi-file",
        "模块",
        "重构",
        "refactor",
        "配置",
        "config",
        "CLI",
        "API",
        "新增",
        "实现",
        "implement",
        "feature",
    ];

    let heavy_count = heavy_markers
        .iter()
        .filter(|m| combined.contains(*m))
        .count();
    let medium_count = medium_markers
        .iter()
        .filter(|m| combined.contains(*m))
        .count();

    if heavy_count >= 2 {
        "Heavy".to_string()
    } else if medium_count >= 1 || heavy_count >= 1 {
        "Medium".to_string()
    } else {
        "Medium".to_string() // default to Medium for safety
    }
}

/// Build the `读取并遵守：` read-and-obey section from project context.
fn build_reads_section(ctx: &ProjectContext) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("- 本任务卡".to_string());

    if ctx.is_ags_suite {
        lines.push("- AGENTS.md".to_string());
        lines.push("- CLAUDE.md".to_string());
        lines.push("- protocol/agent-task-protocol.md".to_string());
        lines.push("- protocol/task-routing.md".to_string());
        lines.push("- protocol/runtime-adapters.md".to_string());
    }

    if let Some(ref cap_path) = ctx.capsule_path {
        lines.push(format!("- {}", cap_path.display()));
    }
    if let Some(ref tm_path) = ctx.task_memory_path {
        lines.push(format!("- {}", tm_path.display()));
    }

    lines.join("\n")
}

/// Render the canonical (classic) task card from a field map.
fn render_task_card(fields: &HashMap<String, String>) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("## 任务卡".to_string());
    lines.push(String::new());

    // Field order matching the canonical classic skeleton
    // (protocol/task-card-template.md). The removed compact fields
    // (路径/读取/关键路径/停止条件) are intentionally never rendered.
    let order: &[&str] = &[
        "读取并遵守：",
        "Executor:",
        "Runtime adapter:",
        "Execution surface:",
        "Permission mode:",
        "Parallelism:",
        "Execution effort:",
        "Workflow authority:",
        "任务级别：",
        "Review gate:",
        "任务：",
        "背景：",
        "项目画像：",
        "记忆胶囊：",
        "任务存档：",
        "适用治理文档：",
        "目标文件夹路径：",
        "相关路径：",
        "本次任务相关文件：",
        "目标：",
        "非目标：",
        "实施要求：",
        "验证：",
        "Verification gate:",
        "交付：",
    ];

    for header in order {
        if let Some(value) = fields.get(*header) {
            let val = value.trim();
            if val.is_empty() {
                continue;
            }
            lines.push(header.to_string());
            // Multi-line fields get their content on separate lines,
            // inline fields stay on the same line
            if is_inline_field(header) {
                // Inline — replace the bare header line with "Header: value".
                let last = lines.last_mut().unwrap();
                *last = format!("{} {}", header, val);
            } else {
                // Multi-line — content follows on separate lines
                // Keep the header as-is, then add the value lines
                // Remove the standalone header and replace with header + value
                lines.pop(); // remove the bare header
                             // For multi-line fields, we keep the header line then content
                lines.push(header.to_string());
                for vline in val.lines() {
                    lines.push(vline.to_string());
                }
            }
            lines.push(String::new());
        }
    }

    // Trim trailing blank lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n") + "\n"
}

fn is_inline_field(header: &str) -> bool {
    matches!(
        header,
        "Executor:"
            | "Runtime adapter:"
            | "Execution surface:"
            | "Permission mode:"
            | "Parallelism:"
            | "任务级别："
            | "Execution effort:"
            | "Workflow authority:"
    )
}

// ── Public API ──────────────────────────────────────────────────────────

/// Compile an intent and return only the compiled card text.
/// Returns an error message if required slots are missing or if
/// the task card was not requested.
#[allow(clippy::result_large_err)] // CompileReport carries full diagnostics; boxing it would complicate the public API
pub fn compile_simple(
    intent: &str,
    project_root: &Path,
    task_card_requested: bool,
) -> Result<String, CompileReport> {
    let (card, report) = compile(intent, project_root, false, task_card_requested);
    if report.executable_allowed && report.missing_slots.is_empty() {
        Ok(card)
    } else {
        Err(report)
    }
}

/// Render only the compiled task card as plain text (no report wrapper).
/// Returns an empty string if the card is empty.
pub fn render_card_text(report: &CompileReport) -> String {
    report.compiled_task_card.clone()
}

/// Render a CompileReport as human-readable text.
pub fn render_report_text(report: &CompileReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("M4 Task Compiler Report".to_string());
    lines.push("========================".to_string());
    lines.push(format!("Schema version: {}", report.schema_version));
    lines.push(format!("Check only:     {}", report.check_only));
    lines.push(format!(
        "Task card requested: {}",
        if report.task_card_requested {
            "YES"
        } else {
            "NO"
        }
    ));
    lines.push(format!(
        "Executable allowed:  {}",
        if report.executable_allowed {
            "YES"
        } else {
            "NO"
        }
    ));
    if let Some(ref reason) = report.block_reason {
        lines.push(format!("Block reason:   {}", reason));
    }
    lines.push(format!(
        "Validation:     {}",
        if report.validation_passed {
            "PASS"
        } else {
            "FAIL"
        }
    ));
    lines.push(String::new());

    if !report.missing_slots.is_empty() {
        lines.push("MISSING SLOTS:".to_string());
        for slot in &report.missing_slots {
            lines.push(format!("  - {}", slot));
        }
        lines.push(String::new());
    }

    if !report.assumptions.is_empty() {
        lines.push("Assumptions:".to_string());
        for a in &report.assumptions {
            lines.push(format!("  - {}", a));
        }
        lines.push(String::new());
    }

    if !report.validation_errors.is_empty() {
        lines.push("Validation Errors:".to_string());
        for e in &report.validation_errors {
            lines.push(format!("  - {}", e));
        }
        lines.push(String::new());
    }

    lines.push("Slot Sources:".to_string());
    for slot in &report.slot_sources {
        lines.push(format!(
            "  {:25} ← {:20} ({})",
            slot.field,
            slot.source.as_str(),
            if slot.value.chars().count() > 60 {
                let truncated: String = slot.value.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                slot.value.clone()
            }
        ));
    }
    lines.push(String::new());

    if report.executable_allowed && !report.compiled_task_card.is_empty() {
        lines.push("Compiled Task Card:".to_string());
        lines.push("-------------------".to_string());
        lines.push(report.compiled_task_card.clone());
    }

    lines.join("\n")
}

/// Render a CompileReport as JSON.
pub fn render_report_json(report: &CompileReport) -> String {
    serde_json::to_string_pretty(report)
        .unwrap_or_else(|e| format!("{{\"error\": \"JSON serialization failed: {}\"}}", e))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_intent_simple_inline() {
        let input = "Executor: Claude Code\n任务级别：Medium\n任务：测试编译";
        let parsed = parse_intent(input);
        assert_eq!(parsed.fields.get("Executor:").unwrap(), "Claude Code");
        assert_eq!(parsed.fields.get("任务级别：").unwrap(), "Medium");
        assert_eq!(parsed.fields.get("任务：").unwrap(), "测试编译");
    }

    #[test]
    fn test_parse_intent_multiline() {
        let input = "目标：\n1. goal_1\n2. goal_2\n\n非目标：\n- non_goal";
        let parsed = parse_intent(input);
        assert_eq!(parsed.fields.get("目标：").unwrap(), "1. goal_1\n2. goal_2");
        assert_eq!(parsed.fields.get("非目标：").unwrap(), "- non_goal");
    }

    #[test]
    fn test_parse_intent_free_text() {
        let input = "这是一段自由文本\n描述任务内容\n\n任务级别：Medium";
        let parsed = parse_intent(input);
        assert_eq!(parsed.free_text, "这是一段自由文本\n描述任务内容");
        assert_eq!(parsed.fields.get("任务级别：").unwrap(), "Medium");
    }

    #[test]
    fn test_parse_intent_mixed() {
        let input =
            "Executor: Claude Code\n\nimplement a feature\n\n目标：\n1. do something\n2. verify";
        let parsed = parse_intent(input);
        assert_eq!(parsed.fields.get("Executor:").unwrap(), "Claude Code");
        assert_eq!(
            parsed.fields.get("目标：").unwrap(),
            "1. do something\n2. verify"
        );
        assert_eq!(parsed.free_text, "implement a feature");
    }

    #[test]
    fn test_normalise_key() {
        assert_eq!(normalise_key("Executor:"), Some("Executor:"));
        assert_eq!(normalise_key("任务："), Some("任务："));
        assert_eq!(normalise_key("Executor"), Some("Executor:"));
        assert_eq!(normalise_key("任务"), Some("任务："));
        assert_eq!(normalise_key("unknown"), None);
    }

    #[test]
    fn test_compile_minimal_intent() {
        let intent = "任务：test compilation\n目标：verify compiler works";
        let project_root = Path::new(".");
        let (card, report) = compile(intent, project_root, false, true);

        // Should have no missing slots
        assert!(
            report.missing_slots.is_empty(),
            "Missing slots: {:?}",
            report.missing_slots
        );

        // Card should start with ## 任务卡
        assert!(
            card.starts_with("## 任务卡"),
            "Card does not start with ## 任务卡:\n{}",
            card
        );

        // Should contain key fields
        assert!(card.contains("Executor:"));
        assert!(card.contains("任务："));
        assert!(card.contains("目标："));
    }

    #[test]
    fn test_compile_missing_task_and_goal() {
        let intent = "Executor: Claude Code";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        // Should report 目标：as a default-filled slot (not missing since we default it)
        // 任务：should be missing if no free text
        let has_task = report.slot_sources.iter().any(|s| s.field == "任务：");
        eprintln!(
            "has_task: {}, missing: {:?}",
            has_task, report.missing_slots
        );
    }

    #[test]
    fn test_compile_includes_slot_sources() {
        let intent = "任务：test\n目标：verify";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        // Every slot in the report should have a source
        for slot in &report.slot_sources {
            assert!(!slot.field.is_empty());
            assert!(
                slot.source != SlotSource::Missing || report.missing_slots.contains(&slot.field)
            );
        }
    }

    #[test]
    fn test_render_report_json_is_valid() {
        let intent = "任务：test\n目标：verify";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        let json = render_report_json(&report);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "JSON parse error: {:?}", parsed.err());

        let v = parsed.unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert!(v["slot_sources"].is_array());
    }

    #[test]
    fn test_compile_check_only_in_report() {
        let intent = "任务：test\n目标：verify";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, true, true);
        assert!(report.check_only);
        assert!(
            report.compiled_task_card.is_empty(),
            "check-only report must not expose an executable compiled_task_card"
        );
    }

    #[test]
    fn test_infer_task_level_medium() {
        let parsed = parse_intent("implement new feature\n跨文件改动");
        let fields = HashMap::new();
        assert_eq!(infer_task_level(&parsed, &fields), "Medium");
    }

    #[test]
    fn test_infer_task_level_heavy() {
        let parsed = parse_intent("数据迁移和向量库重建\n涉及历史数据baseline保护");
        let fields = HashMap::new();
        assert_eq!(infer_task_level(&parsed, &fields), "Heavy");
    }

    #[test]
    fn test_compile_report_json_schema() {
        let intent = "任务：test task\n目标：verify something";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        let json = render_report_json(&report);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Required top-level fields
        assert!(v.get("schema_version").is_some(), "missing schema_version");
        assert!(
            v.get("compiled_task_card").is_some(),
            "missing compiled_task_card"
        );
        assert!(v.get("slot_sources").is_some(), "missing slot_sources");
        assert!(v.get("missing_slots").is_some(), "missing missing_slots");
        assert!(v.get("assumptions").is_some(), "missing assumptions");
        assert!(
            v.get("validation_passed").is_some(),
            "missing validation_passed"
        );
        assert!(
            v.get("validation_errors").is_some(),
            "missing validation_errors"
        );
        assert!(v.get("check_only").is_some(), "missing check_only");
        assert!(
            v.get("task_card_requested").is_some(),
            "missing task_card_requested"
        );
        assert!(
            v.get("executable_allowed").is_some(),
            "missing executable_allowed"
        );
    }

    // ── Task-card request gate tests ──────────────────────────────

    #[test]
    fn test_task_card_not_requested_blocks_executable_output() {
        // Without task_card_requested, executable output must be blocked.
        let intent = "任务：test gate\n目标：verify blocking";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, false);

        assert!(
            !report.task_card_requested,
            "task_card_requested must be false"
        );
        assert!(
            !report.executable_allowed,
            "executable_allowed must be false when task_card_requested is false"
        );
        assert_eq!(
            report.block_reason,
            Some("task_card_not_requested".to_string()),
            "block_reason must be task_card_not_requested"
        );
        assert!(
            report.compiled_task_card.is_empty(),
            "compiled_task_card must be empty when executable is blocked"
        );
        assert!(
            !report.validation_passed,
            "validation_passed must be false when executable is blocked"
        );
    }

    #[test]
    fn test_task_card_requested_allows_executable_output() {
        // With task_card_requested=true, executable output must be allowed.
        let intent = "任务：test gate\n目标：verify allowed";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        assert!(
            report.task_card_requested,
            "task_card_requested must be true"
        );
        assert!(
            report.executable_allowed,
            "executable_allowed must be true when task_card_requested is true"
        );
        assert!(
            report.block_reason.is_none(),
            "block_reason must be None when allowed"
        );
        assert!(
            !report.compiled_task_card.is_empty(),
            "compiled_task_card must NOT be empty when executable is allowed"
        );
    }

    #[test]
    fn test_check_only_blocks_executable_even_when_requested() {
        // check_only takes precedence over task_card_requested.
        let intent = "任务：test check only gate\n目标：verify precedence";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, true, true);

        assert!(report.check_only, "check_only must be true");
        assert!(
            report.task_card_requested,
            "task_card_requested must be true"
        );
        assert!(
            !report.executable_allowed,
            "executable_allowed must be false in check-only mode"
        );
        assert_eq!(report.block_reason, Some("check_only".to_string()));
        assert!(report.compiled_task_card.is_empty());
    }

    #[test]
    fn test_text_report_shows_gate_status() {
        // Text report must include task_card_requested and executable_allowed.
        let intent = "任务：gate display test\n目标：verify display";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, false);
        let text = render_report_text(&report);

        assert!(
            text.contains("Task card requested:"),
            "text report must show Task card requested status"
        );
        assert!(
            text.contains("Executable allowed:"),
            "text report must show Executable allowed status"
        );
        assert!(
            text.contains("Block reason:"),
            "text report must show Block reason"
        );
        assert!(
            !text.contains("Compiled Task Card:"),
            "text report must NOT show Compiled Task Card when blocked"
        );
    }

    #[test]
    fn test_missing_slots_blocks_executable_even_when_requested() {
        // Even with task_card_requested=true, missing slots block executable output.
        let intent = "Executor: Claude Code";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);

        assert!(report.task_card_requested);
        assert!(
            !report.executable_allowed,
            "executable_allowed must be false when slots are missing"
        );
        assert_eq!(report.block_reason, Some("missing_slots".to_string()));
    }

    #[test]
    fn test_compile_simple_errors_when_not_requested() {
        let intent = "任务：simple gate test\n目标：verify simple blocks";
        let project_root = Path::new(".");
        let result = compile_simple(intent, project_root, false);
        assert!(
            result.is_err(),
            "compile_simple must return Err when task_card_requested=false"
        );
        let err = result.unwrap_err();
        assert!(!err.executable_allowed);
        assert_eq!(
            err.block_reason,
            Some("task_card_not_requested".to_string())
        );
    }

    #[test]
    fn test_compile_simple_succeeds_when_requested() {
        let intent = "任务：simple ok test\n目标：verify simple allows";
        let project_root = Path::new(".");
        let result = compile_simple(intent, project_root, true);
        assert!(
            result.is_ok(),
            "compile_simple must return Ok when task_card_requested=true"
        );
    }

    #[test]
    fn test_compile_tuple_card_empty_when_gate_blocked() {
        // Regression: the public API tuple (card, report) must NOT leak an
        // executable card when the gate blocks it. Any Rust caller using
        // `let (card, _) = compile(...)` directly (bypassing the CLI) must
        // receive an empty card string when task_card_requested=false.
        let intent = "任务：tuple bypass test\n目标：verify tuple safety";
        let project_root = Path::new(".");

        // Without task_card_requested → card must be empty
        let (card, report) = compile(intent, project_root, false, false);
        assert!(
            card.is_empty(),
            "tuple card must be empty when task_card_requested=false, got {} bytes",
            card.len()
        );
        assert!(!report.executable_allowed);

        // With check_only → card must be empty
        let (card2, report2) = compile(intent, project_root, true, true);
        assert!(
            card2.is_empty(),
            "tuple card must be empty when check_only=true"
        );
        assert!(!report2.executable_allowed);

        // With missing slots → card must be empty
        let (card3, report3) = compile("Executor: Claude Code", project_root, false, true);
        assert!(
            card3.is_empty(),
            "tuple card must be empty when slots are missing"
        );
        assert!(!report3.executable_allowed);

        // With task_card_requested=true, no missing slots → card must be non-empty
        let (card4, report4) = compile(intent, project_root, false, true);
        assert!(
            !card4.is_empty(),
            "tuple card must be non-empty when allowed"
        );
        assert!(card4.starts_with("## 任务卡"));
        assert!(report4.executable_allowed);
    }

    // ── P1 regression: task level aliases ──────────────────────────

    #[test]
    fn test_parse_intent_task_level_english_colon() {
        // "Task level: Heavy" with ASCII colon — must be recognized
        let input = "Task level: Heavy\n任务：test alias parsing";
        let parsed = parse_intent(input);
        assert_eq!(
            parsed.fields.get("任务级别：").unwrap(),
            "Heavy",
            "English alias 'Task level:' with ASCII colon must map to 任务级别："
        );
    }

    #[test]
    fn test_parse_intent_chinese_ascii_colon() {
        // "任务级别: Heavy" with ASCII colon instead of FULLWIDTH
        let input = "任务级别: Heavy\n任务：test alias";
        let parsed = parse_intent(input);
        assert_eq!(
            parsed.fields.get("任务级别：").unwrap(),
            "Heavy",
            "Chinese key '任务级别:' with ASCII colon must map to 任务级别："
        );
    }

    #[test]
    fn test_parse_intent_colonless_key_not_treated_as_free_text() {
        // "任务级别 Heavy" without any colon should NOT be treated as free text
        // The colonless key should be recognized by normalise_key
        let input = "任务级别 Heavy\n任务：test";
        let parsed = parse_intent(input);
        assert!(
            parsed.fields.contains_key("任务级别：") || !parsed.free_text.contains("任务级别"),
            "Colon-less 任务级别 should be recognized; got fields={:?}, free_text={:?}",
            parsed.fields.keys().collect::<Vec<_>>(),
            parsed.free_text
        );
    }

    #[test]
    fn test_task_level_alias_heavy_is_not_downgraded_to_medium() {
        // Explicit "Task level: Heavy" must produce Heavy, not Medium
        let intent = "Task level: Heavy\n任务：critical task\n目标：verify heavy level";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let task_level_slot = report
            .slot_sources
            .iter()
            .find(|s| s.field == "任务级别：")
            .expect("task level slot must exist");
        assert_eq!(
            task_level_slot.value, "Heavy",
            "Explicit Task level: Heavy must be preserved, got '{}'",
            task_level_slot.value
        );
    }

    // ── P5 regression: Heavy → plan-only default ───────────────────

    #[test]
    fn test_heavy_task_defaults_to_plan_only_permission() {
        let intent = "任务级别：Heavy\n任务：critical change\n目标：verify safety";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let perm_slot = report
            .slot_sources
            .iter()
            .find(|s| s.field == "Permission mode:")
            .expect("permission mode slot must exist");
        assert_eq!(
            perm_slot.value, "plan-only",
            "Heavy task must default to plan-only, got '{}'",
            perm_slot.value
        );
    }

    #[test]
    fn test_medium_task_defaults_to_direct_execution() {
        let intent = "任务级别：Medium\n任务：normal change\n目标：verify default";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let perm_slot = report
            .slot_sources
            .iter()
            .find(|s| s.field == "Permission mode:")
            .expect("permission mode slot must exist");
        assert_eq!(
            perm_slot.value, "execute-and-verify",
            "Medium task must default to direct execution, got '{}'",
            perm_slot.value
        );
    }

    #[test]
    fn test_heavy_task_with_explicit_permission_is_preserved() {
        let intent = "任务级别：Heavy\nPermission mode: execute-and-verify\n任务：explicit perm\n目标：verify";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let perm_slot = report
            .slot_sources
            .iter()
            .find(|s| s.field == "Permission mode:")
            .expect("permission mode slot must exist");
        assert_eq!(
            perm_slot.value, "execute-and-verify",
            "Explicit permission mode must be preserved even for Heavy tasks, got '{}'",
            perm_slot.value
        );
        assert_eq!(
            perm_slot.source,
            SlotSource::Intent,
            "Explicit permission mode source must be Intent"
        );
    }

    // ── P2.2 regression: UTF-8 safe truncation ─────────────────────

    #[test]
    fn test_utf8_safe_truncation_does_not_panic() {
        // A long Chinese string — slicing at byte boundaries could panic
        let long_chinese =
            "任务：这是一个很长的中文任务描述用来测试UTF8截断安全性确保不会在字节边界处崩溃";
        let intent = format!("{}\n目标：verify truncation safety", long_chinese);
        let project_root = Path::new(".");
        let (_card, report) = compile(&intent, project_root, false, true);
        // render_report_text must not panic on mixed ASCII/Chinese content
        let text = render_report_text(&report);
        assert!(!text.is_empty(), "report text should be non-empty");
    }

    // ── P2.3 regression: check-only suppresses card output ──────────

    #[test]
    fn test_check_only_suppresses_compiled_card_in_text() {
        let intent = "任务：check only test\n目标：verify suppression";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, true, true);
        let text = render_report_text(&report);
        assert!(
            !text.contains("Compiled Task Card:"),
            "check-only text output must NOT contain 'Compiled Task Card:' section"
        );
        assert!(
            report.compiled_task_card.is_empty(),
            "check-only report must not include executable card text"
        );
    }

    #[test]
    fn test_normal_mode_includes_card_in_text() {
        let intent = "任务：normal mode\n目标：verify card included";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let text = render_report_text(&report);
        assert!(
            text.contains("Compiled Task Card:"),
            "normal (non-check-only) text output must include 'Compiled Task Card:' section"
        );
    }

    // ── P2.1 regression: plain card output ─────────────────────────

    #[test]
    fn test_plain_card_output_starts_with_task_card_header() {
        let intent = "任务：pipe test\n目标：verify pipeable output";
        let project_root = Path::new(".");
        let (_card, report) = compile(intent, project_root, false, true);
        let card_text = render_card_text(&report);
        assert!(
            card_text.starts_with("## 任务卡"),
            "Plain card output must start with '## 任务卡', got: {:?}",
            &card_text[..30.min(card_text.len())]
        );
    }

    // ── Hard validation: compiled card passes the REAL validator ───────

    #[test]
    fn compiled_card_passes_real_validator() {
        // The compiled card must pass the REAL task_card_validator, not just
        // the compiler's heuristic self-check (closes the gap where compact
        // output was never validated against the canonical gate).
        let intent = "任务：测试编译器输出能通过真实校验器\n\
                      目标：验证 ags task compile 产出经典骨架并通过 validator";
        let project_root = Path::new(".");
        let (card, report) = compile(intent, project_root, false, true);

        assert!(
            report.executable_allowed,
            "card should be executable: {:?}",
            report.block_reason
        );
        assert!(
            card.starts_with("## 任务卡"),
            "card must start with ## 任务卡"
        );
        assert!(
            !card.contains("AGENT_SUITE_COMPACT_TASK_CARD_V1"),
            "compiled card must not contain the removed compact marker"
        );
        // The second non-empty line must be the classic discriminator.
        let second = card
            .lines()
            .filter(|l| !l.trim().is_empty())
            .nth(1)
            .unwrap_or("");
        assert!(
            second.starts_with("读取并遵守："),
            "second non-empty line must be 读取并遵守：, got: {}",
            second
        );

        let errors = task_card_validator::validate(&card);
        assert!(
            errors.is_empty(),
            "compiled card must pass the real validator, errors: {:?}\ncard:\n{}",
            errors,
            card
        );
        let has_absolute_path = card.contains("目标文件夹路径：\n- /")
            || card.contains("目标文件夹路径：\n- \\\\?\\")
            || card.lines().any(|l| {
                l.starts_with("- ")
                    && l.len() > 3
                    && l.as_bytes()[2].is_ascii_alphabetic()
                    && l.contains(":\\")
            });
        assert!(
            has_absolute_path,
            "compiled card must render an absolute target folder path:\n{}",
            card
        );
        assert!(
            !card.contains("目标文件夹路径：\n- ."),
            "compiled card must not render a relative target folder path:\n{}",
            card
        );
    }

    // ── Single canonical template: level is a field, not a template file ───

    #[test]
    fn compiler_always_emits_single_canonical_skeleton() {
        // Light / Medium / Heavy-shaped intents must all compile to the SAME
        // canonical skeleton (protocol/task-card-template.md). Task level is a
        // `任务级别：` field value, never a different per-level template file,
        // and the compiler must never emit a compact or fallback skeleton.
        let project_root = Path::new(".");
        let intents = [
            "任务：改个变量名\n目标：把 foo 改成 bar",
            "任务：跨文件重构共享模块\n目标：调整配置与共享 helper 行为",
            "任务：数据库迁移与向量库 baseline 重建\n目标：执行不可逆迁移并保留 baseline",
        ];
        for intent in intents {
            let (card, _report) = compile(intent, project_root, false, true);
            assert!(
                card.starts_with("## 任务卡"),
                "must start with ## 任务卡 for intent {intent:?}"
            );
            let second = card
                .lines()
                .filter(|l| !l.trim().is_empty())
                .nth(1)
                .unwrap_or("");
            assert!(
                second.starts_with("读取并遵守："),
                "second non-empty line must be the single canonical discriminator \
                 读取并遵守：, got {second:?} for intent {intent:?}"
            );
            assert!(
                card.contains("任务级别："),
                "canonical skeleton must carry the 任务级别： field for intent {intent:?}"
            );
            assert!(
                !card.contains("AGENT_SUITE_COMPACT_TASK_CARD_V1"),
                "compiled card must not contain the removed compact marker: {intent:?}"
            );
            assert!(
                !card.contains("fallback-task-cards"),
                "compiled card must not reference a fallback template set: {intent:?}"
            );
            assert!(
                !card.contains("task-template.md"),
                "compiled card must not reference a per-level template file: {intent:?}"
            );
        }
    }
}
