//! Field definitions and task-card parsing.
use super::*;
use crate::fields::{TaskField, TASK_FIELDS};

// ── Field definitions for parsing ──────────────────────────────────────

/// Find the longest field-definition that is a prefix of `line`.
pub(crate) fn find_field(line: &str) -> Option<(&'static TaskField, &str)> {
    TASK_FIELDS
        .iter()
        .filter_map(|field| line.strip_prefix(field.header).map(|rest| (field, rest)))
        .max_by_key(|(field, _)| field.header.len())
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

            if def.inline {
                let value =
                    rest.trim_start_matches(|c: char| c == ':' || c == '：' || c.is_whitespace());
                fields.insert(def.header.to_string(), value.to_string());
            } else {
                current_field = Some(def.header);
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

/// Extract the stable closure identifiers from an already validated task card.
pub fn closure_contract(card: &ParsedTaskCard) -> TaskClosureContract {
    TaskClosureContract {
        contract_id: field_val(&card.fields, "Contract ID:").to_string(),
        handoff_source: field_val(&card.fields, "Handoff source:").to_string(),
        goal_ids: collect_declared_ids(field_val(&card.fields, "目标："), "G-"),
        acceptance_criteria_ids: collect_declared_ids(field_val(&card.fields, "验收标准："), "AC-"),
        verification_ids: collect_declared_ids(field_val(&card.fields, "Verification gate:"), "V-"),
        evidence_ids: collect_declared_ids(field_val(&card.fields, "Verification gate:"), "EV-"),
    }
}

pub(crate) fn collect_declared_ids(block: &str, prefix: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|line| {
            let body = line.trim().trim_start_matches('-').trim();
            let token = body.split([':', ' ', '→']).next().unwrap_or("");
            is_indexed_id(token, prefix).then(|| token.to_string())
        })
        .collect()
}

pub(crate) fn is_indexed_id(value: &str, prefix: &str) -> bool {
    let Some(number) = value.strip_prefix(prefix) else {
        return false;
    };
    number.len() == 2 && number.chars().all(|character| character.is_ascii_digit())
}

/// Get a field value from the parsed card, or empty string if missing.
pub(crate) fn field_val<'a>(fields: &'a HashMap<String, String>, key: &str) -> &'a str {
    fields.get(key).map(|s| s.as_str()).unwrap_or("")
}

/// Get Workflow authority, defaulting to "none" when absent.
pub(crate) fn get_workflow_authority(fields: &HashMap<String, String>) -> &str {
    fields
        .get("Workflow authority:")
        .map(|s| s.as_str())
        .unwrap_or("none")
}

/// Get the `子任务编排` (subtask orchestration) mode from the slot block.
///
/// The slot is a multi-line block; the mode lives on a `- mode: <value>` bullet
/// (half- or full-width colon). Returns the parsed mode, or `"none"` when the
/// slot is absent or carries no `mode:` line — so cards without the slot keep
/// passing with the no-orchestration default.
pub(crate) fn get_subtask_orchestration_mode(fields: &HashMap<String, String>) -> &str {
    let Some(block) = fields.get("子任务编排：") else {
        return "none";
    };
    for line in block.lines() {
        let t = line
            .trim()
            .trim_start_matches('-')
            .trim_start_matches(|c: char| c == '*' || c.is_whitespace());
        if let Some(rest) = t.strip_prefix("mode:").or_else(|| t.strip_prefix("mode：")) {
            return rest.trim();
        }
    }
    "none"
}
