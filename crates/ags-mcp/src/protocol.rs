//! Minimal MCP wire types used by the contract-v2 stdio adapter.

#![allow(non_snake_case)]

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ToolListResult {
    pub tools: Vec<ToolDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub inputSchema: Value,
}
