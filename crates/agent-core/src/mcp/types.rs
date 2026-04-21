// MCP protocol types — JSON-RPC 2.0 messages and MCP-specific structures.

use serde::{Deserialize, Serialize};

// ─── JSON-RPC 2.0 ─────────────────────────────────────────────────────────────

/// JSON-RPC 2.0 notification — like a request but with no `id`, so the server
/// must not send a response (MCP spec §3.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

// ─── MCP protocol types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientInfo {
    pub name: String,
    pub version: String,
}

impl Default for McpClientInfo {
    fn default() -> Self {
        Self {
            name: "sage".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub content: Vec<McpContentBlock>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl McpToolResult {
    pub fn to_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_error(&self) -> bool {
        self.is_error.unwrap_or(false)
    }
}

// ─── MCP server configuration ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_request_sets_version() {
        let req = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, 1);
        assert_eq!(req.method, "tools/list");
    }

    #[test]
    fn mcp_client_info_default_name_is_sage() {
        let info = McpClientInfo::default();
        assert_eq!(info.name, "sage");
    }

    #[test]
    fn mcp_tool_result_to_text_joins_blocks() {
        let result = McpToolResult {
            content: vec![
                McpContentBlock {
                    content_type: "text".to_string(),
                    text: Some("hello".to_string()),
                },
                McpContentBlock {
                    content_type: "text".to_string(),
                    text: Some("world".to_string()),
                },
            ],
            is_error: None,
        };
        assert_eq!(result.to_text(), "hello\nworld");
    }

    #[test]
    fn mcp_tool_result_is_error_defaults_false() {
        let result = McpToolResult {
            content: vec![],
            is_error: None,
        };
        assert!(!result.is_error());
    }

    #[test]
    fn mcp_tool_result_is_error_true_when_set() {
        let result = McpToolResult {
            content: vec![],
            is_error: Some(true),
        };
        assert!(result.is_error());
    }

    #[test]
    fn jsonrpc_response_serializes_without_null_fields() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"error\":null"));
    }
}
