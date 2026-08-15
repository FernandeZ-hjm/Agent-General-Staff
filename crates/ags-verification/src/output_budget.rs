//! Shared contract v2 presentation budgets.

pub const DEFAULT_HUMAN_LINE_BUDGET: usize = 5;
pub const DEFAULT_JSON_BYTE_BUDGET: usize = 16 * 1024;
pub const TOOL_SCHEMA_BYTE_BUDGET: usize = 8 * 1024;

pub fn check_human_output_budget(output: &str) -> Result<(), String> {
    let lines = output.lines().count();
    if lines > DEFAULT_HUMAN_LINE_BUDGET {
        return Err(format!(
            "default human output is {lines} lines; budget is {DEFAULT_HUMAN_LINE_BUDGET}"
        ));
    }
    Ok(())
}

pub fn check_json_output_budget(output: &[u8]) -> Result<(), String> {
    if output.len() > DEFAULT_JSON_BYTE_BUDGET {
        return Err(format!(
            "default JSON output is {} bytes; budget is {DEFAULT_JSON_BYTE_BUDGET}",
            output.len()
        ));
    }
    serde_json::from_slice::<serde_json::Value>(output)
        .map(|_| ())
        .map_err(|error| format!("default JSON output is invalid: {error}"))
}

pub fn check_tool_schema_budget(schema: &[u8]) -> Result<(), String> {
    if schema.len() > TOOL_SCHEMA_BYTE_BUDGET {
        return Err(format!(
            "MCP tool schema is {} bytes; budget is {TOOL_SCHEMA_BYTE_BUDGET}",
            schema.len()
        ));
    }
    serde_json::from_slice::<serde_json::Value>(schema)
        .map(|_| ())
        .map_err(|error| format!("MCP tool schema is invalid JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_success_output_has_a_five_line_hard_limit() {
        assert!(check_human_output_budget("one\ntwo\nthree\nfour\nfive\n").is_ok());
        assert!(check_human_output_budget("1\n2\n3\n4\n5\n6\n").is_err());
    }

    #[test]
    fn json_and_tool_schema_have_independent_byte_limits() {
        assert!(check_json_output_budget(br#"{"status":"ok"}"#).is_ok());
        assert!(check_tool_schema_budget(br#"{"type":"object"}"#).is_ok());
        assert!(check_json_output_budget(&vec![b' '; DEFAULT_JSON_BYTE_BUDGET + 1]).is_err());
        assert!(check_tool_schema_budget(&vec![b' '; TOOL_SCHEMA_BYTE_BUDGET + 1]).is_err());
    }
}
