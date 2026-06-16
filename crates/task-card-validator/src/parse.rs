//! Field definitions and task-card parsing.
use super::*;

// ── Field definitions for parsing ──────────────────────────────────────

pub(crate) struct FieldDef {
    name: &'static str,
    /// true = value on the same line after `:` or `：`.
    is_inline: bool,
}

/// All known task-card field headers.  Must include every field that could
/// appear so the parser can correctly delimit multi-line sections.  The
/// lookup uses longest-prefix match, so order is irrelevant.
pub(crate) const FIELD_DEFS: &[FieldDef] = &[
    // ── inline fields ──
    FieldDef {
        name: "Runtime adapter:",
        is_inline: true,
    },
    FieldDef {
        name: "Execution surface:",
        is_inline: true,
    },
    FieldDef {
        name: "Permission mode:",
        is_inline: true,
    },
    FieldDef {
        name: "Parallelism:",
        is_inline: true,
    },
    FieldDef {
        name: "Executor:",
        is_inline: true,
    },
    FieldDef {
        name: "任务级别：",
        is_inline: true,
    },
    FieldDef {
        name: "Execution effort:",
        is_inline: true,
    },
    FieldDef {
        name: "Workflow authority:",
        is_inline: true,
    },
    // ── multi-line fields ──
    FieldDef {
        name: "本次任务相关文件：",
        is_inline: false,
    },
    FieldDef {
        name: "Verification gate:",
        is_inline: false,
    },
    FieldDef {
        name: "读取并遵守：",
        is_inline: false,
    },
    FieldDef {
        name: "Review gate:",
        is_inline: false,
    },
    FieldDef {
        name: "记忆胶囊：",
        is_inline: false,
    },
    FieldDef {
        name: "停止条件：",
        is_inline: false,
    },
    FieldDef {
        name: "关键路径：",
        is_inline: false,
    },
    FieldDef {
        name: "项目画像：",
        is_inline: false,
    },
    FieldDef {
        name: "任务存档：",
        is_inline: false,
    },
    FieldDef {
        name: "目标文件夹路径：",
        is_inline: false,
    },
    FieldDef {
        name: "相关路径：",
        is_inline: false,
    },
    FieldDef {
        name: "非目标：",
        is_inline: false,
    },
    FieldDef {
        name: "路径：",
        is_inline: false,
    },
    FieldDef {
        name: "读取：",
        is_inline: false,
    },
    FieldDef {
        name: "任务：",
        is_inline: false,
    },
    FieldDef {
        name: "目标：",
        is_inline: false,
    },
    FieldDef {
        name: "验证：",
        is_inline: false,
    },
    FieldDef {
        name: "交付：",
        is_inline: false,
    },
    FieldDef {
        name: "背景：",
        is_inline: false,
    },
];

/// Find the longest field-definition that is a prefix of `line`.
pub(crate) fn find_field(line: &str) -> Option<(&'static FieldDef, &str)> {
    FIELD_DEFS
        .iter()
        .filter_map(|def| line.strip_prefix(def.name).map(|rest| (def, rest)))
        .max_by_key(|(def, _)| def.name.len())
}

// ── Card parsing ───────────────────────────────────────────────────────

/// Parse a task-card into a field-name → value map.
///
/// Inline fields store the portion after `: ` or `：`.
/// Multi-line fields collect text between the field header and the next
/// recognised field header (or EOF).
pub(crate) fn parse_card(input: &str) -> HashMap<String, String> {
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut current_field: Option<&str> = None;
    let mut current_value = String::new();

    for line in input.lines() {
        let trimmed = line.trim();

        if let Some((def, rest)) = find_field(trimmed) {
            // Save the previous multi-line field
            if let Some(fname) = current_field.take() {
                let v = current_value.trim().to_string();
                fields.insert(fname.to_string(), v);
                current_value = String::new();
            }

            if def.is_inline {
                let value =
                    rest.trim_start_matches(|c: char| c == ':' || c == '：' || c.is_whitespace());
                fields.insert(def.name.to_string(), value.to_string());
            } else {
                current_field = Some(def.name);
                let value_start =
                    rest.trim_start_matches(|c: char| c == ':' || c == '：' || c.is_whitespace());
                current_value.push_str(value_start);
                current_value.push('\n');
            }
        } else if current_field.is_some() {
            current_value.push_str(line);
            current_value.push('\n');
        }
    }

    // Save trailing multi-line field
    if let Some(fname) = current_field {
        let v = current_value.trim().to_string();
        fields.insert(fname.to_string(), v);
    }

    fields
}

/// Validate a task card string, returning the parsed fields on success.
///
/// This is the single-call bridge from raw text to structured fields.
/// On validation failure, returns `Err(errors)`.  On success, returns
/// `Ok(ParsedTaskCard)` with parsed fields.
pub fn parse_validated(input: &str) -> Result<ParsedTaskCard, Vec<String>> {
    let errors = validate(input);
    if !errors.is_empty() {
        return Err(errors);
    }
    let fields = parse_card(input);
    Ok(ParsedTaskCard { fields })
}

/// Get a field value from the parsed card, or empty string if missing.
pub(crate) fn field_val<'a>(fields: &'a HashMap<String, String>, key: &str) -> &'a str {
    fields.get(key).map(|s| s.as_str()).unwrap_or("")
}

/// Get Execution effort, defaulting to "unknown" when absent.
///
/// Execution effort describes thinking intensity only; it does NOT gate
/// authority.  This function exists to document the default-semantics
/// contract and is available for future policy checks.
#[allow(dead_code)]
pub(crate) fn get_execution_effort(fields: &HashMap<String, String>) -> &str {
    fields
        .get("Execution effort:")
        .map(|s| s.as_str())
        .unwrap_or("unknown")
}

/// Get Workflow authority, defaulting to "none" when absent.
pub(crate) fn get_workflow_authority(fields: &HashMap<String, String>) -> &str {
    fields
        .get("Workflow authority:")
        .map(|s| s.as_str())
        .unwrap_or("none")
}
