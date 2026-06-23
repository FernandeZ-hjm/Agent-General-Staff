//! Orchestration: validate(), validate_files(), and input reading.
use super::*;

// ── Main validate() ────────────────────────────────────────────────────

/// Validate a single input string, returning a list of errors (empty = valid).
pub fn validate(input: &str) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();

    // ── Phase 1: format checks ──

    // Rule 1: first non-empty line must be `## 任务卡`
    let first = first_nonempty_line(input);

    match first {
        Some("## 任务卡") => {}
        Some(line) => {
            errors.push(format!(
                "首行必须为 `## 任务卡`，实际为 `{}`",
                trunc80(line)
            ));
        }
        None => {
            errors.push("文件为空".to_string());
            return errors;
        }
    }

    // Rule 2: reject text-typed code fences
    if let Some(pos) = find_text_fence(input) {
        errors.push(format!("第 {} 行附近：禁止使用 `text` 类型代码围栏", pos));
    }
    check_active_skill_tags(input, &mut errors);

    // Single canonical task-card format: the classic fixed skeleton whose
    // second non-empty line is `读取并遵守：`. The compact task-card format
    // has been removed. Reject it at the structural discriminator position
    // (the second non-empty line) ONLY — never by full-text `contains`, so a
    // legitimate classic card that merely mentions the marker in prose
    // (e.g. a task card *about* removing compact) still passes.
    let second_line = input
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .nth(1); // 0: ## 任务卡, 1: discriminator line

    match second_line {
        Some(line) if line.starts_with("AGENT_SUITE_COMPACT_TASK_CARD_V1") => {
            errors.push(
                "compact 任务卡格式已删除：第二非空行不得为 `AGENT_SUITE_COMPACT_TASK_CARD_V1`；\
                 请使用以 `读取并遵守：` 开头的经典固定骨架（见 protocol/task-card-template.md）"
                    .to_string(),
            );
        }
        Some(line) if line.starts_with("路径：") => {
            errors.push(
                "compact 任务卡格式已删除：第二非空行不得以 `路径：` 开头；\
                 经典固定骨架使用 `读取并遵守：` + `相关路径：`（见 protocol/task-card-template.md）"
                    .to_string(),
            );
        }
        _ => {}
    }

    // Rule 3/4: check required fields (single canonical set).
    let mut missing: Vec<&str> = Vec::new();
    for field in REQUIRED_FIELDS {
        if !input.contains(field) {
            missing.push(field);
        }
    }

    if !missing.is_empty() {
        errors.push(format!("任务卡缺少必需字段: {}", missing.join(", ")));
    }

    // Parse card for semantic checks
    let fields = parse_card(input);

    // ── Phase 2-7: semantic checks ──
    check_field_values(&fields, &mut errors);
    check_field_combinations(&fields, &mut errors);
    check_protected_paths(&fields, &mut errors);
    check_content_quality(&fields, &mut errors);
    check_contradictions(&fields, &mut errors);
    check_execution_authority_gate(&fields, &mut errors);

    errors
}

// ── Output-shape helpers (shared with `ags gate output`) ────────────────

/// The first non-empty line of `input` with any trailing `\r` stripped, or
/// `None` when the input has no non-empty line.
pub fn first_nonempty_line(input: &str) -> Option<&str> {
    input
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .find(|l| !l.trim().is_empty())
}

/// Frontstage output-shape gate: `true` iff the first non-empty line is exactly
/// `## 任务卡`. This is the canonical task-card foreground-output discriminator,
/// shared by the validator (Rule 1 above) and the `ags gate output` check so the
/// two never drift.
pub fn output_is_canonical_header(input: &str) -> bool {
    first_nonempty_line(input) == Some("## 任务卡")
}

// ── Multi-file entry point ─────────────────────────────────────────────

/// Validate one or more files. Returns true if ALL files pass.
///
/// A path of `"-"` reads from stdin.
pub fn validate_files(paths: &[String]) -> bool {
    let mut all_ok = true;

    for path in paths {
        let (content, display_path) = match read_input(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", path, e);
                all_ok = false;
                continue;
            }
        };

        let errors = validate(&content);
        if errors.is_empty() {
            eprintln!("{}: OK", display_path);
        } else {
            all_ok = false;
            eprintln!("{}: FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
        }
    }

    all_ok
}

/// Read file or stdin.
pub(crate) fn read_input(path: &str) -> Result<(String, String), String> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok((buf, "(stdin)".to_string()))
    } else {
        let p = Path::new(path);
        let content = fs::read_to_string(p).map_err(|e| e.to_string())?;
        Ok((content, p.display().to_string()))
    }
}
