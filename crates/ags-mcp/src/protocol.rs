//! JSON-RPC 2.0 message types for MCP protocol over stdio.
//!
//! Follows the MCP specification (Model Context Protocol) which uses
//! JSON-RPC 2.0 as its transport encoding.
//!
//! Field names use camelCase to match the MCP specification wire format.
//! The `non_snake_case` warnings are suppressed intentionally.

#![allow(non_snake_case)]
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC 2.0 Base Types ──────────────────────────────────────────────────

/// A JSON-RPC 2.0 request message.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 notification (no `id` field).
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 response message.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ── MCP Protocol Constants ───────────────────────────────────────────────────

pub const MCP_VERSION: &str = "2024-11-05";
/// AGS MCP server name — the host initialization adapter (not a governed third-party MCP).
pub const SERVER_NAME: &str = "ags-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── MCP Initialize Types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocolVersion: String,
    pub capabilities: ServerCapabilities,
    pub serverInfo: ServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// ── MCP Tool Types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ToolListResult {
    pub tools: Vec<ToolDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub inputSchema: serde_json::Value,
}

// ── MCP Resource Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ResourceListResult {
    pub resources: Vec<ResourceDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceDef {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimeType: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceReadResult {
    pub contents: Vec<ResourceContent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimeType: Option<String>,
    pub text: String,
}

// ── MCP Prompt Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PromptListResult {
    pub prompts: Vec<PromptDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptGetResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: PromptContent,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptContent {
    pub r#type: String,
    pub text: String,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        }
    }

    pub fn method_not_found(id: Option<Value>) -> Self {
        Self::error(id, -32601, "Method not found")
    }

    pub fn invalid_params(id: Option<Value>, detail: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32602,
                message: format!("Invalid params: {}", detail),
                data: None,
            }),
        }
    }

    pub fn internal_error(id: Option<Value>, detail: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: format!("Internal error: {}", detail),
                data: None,
            }),
        }
    }
}
