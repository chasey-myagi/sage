// MCP tool adapter — wraps an McpClient tool as an AgentTool.
//
// Tools are discovered via tools/list and exposed with the "mcp__<server>__<tool>" prefix,
// matching the Claude Code convention.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::mcp::{McpClient, McpToolInfo};
use crate::types::Content;

use super::{AgentTool, ToolOutput};

const TOOL_NAME_PREFIX: &str = "mcp__";

/// Build the full prefixed tool name: `mcp__<server>__<tool>`.
pub fn build_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("{TOOL_NAME_PREFIX}{server_name}__{tool_name}")
}

fn error_output(msg: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text: msg.into() }],
        is_error: true,
    }
}

/// An AgentTool backed by a remote MCP server tool.
pub struct McpTool {
    /// Prefixed name: `mcp__<server>__<original_name>`
    full_name: String,
    /// Original tool name as reported by the server.
    original_name: String,
    description: String,
    schema: serde_json::Value,
    client: Arc<Mutex<McpClient>>,
}

impl McpTool {
    pub fn new(server_name: &str, info: &McpToolInfo, client: Arc<Mutex<McpClient>>) -> Self {
        Self {
            full_name: build_mcp_tool_name(server_name, &info.name),
            original_name: info.name.clone(),
            description: info.description.clone().unwrap_or_default(),
            schema: info.input_schema.clone(),
            client,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let mut client = self.client.lock().await;
        match client.call_tool(&self.original_name, args).await {
            Ok(result) => {
                let text = result.to_text();
                ToolOutput {
                    content: vec![Content::Text { text }],
                    is_error: result.is_error(),
                }
            }
            Err(e) => error_output(format!("MCP call failed: {e}")),
        }
    }
}

/// Discover all tools from a connected MCP client and return them as AgentTools.
pub async fn discover_mcp_tools(
    server_name: &str,
    client: Arc<Mutex<McpClient>>,
) -> Result<Vec<McpTool>, crate::mcp::McpError> {
    let tools = {
        let mut locked = client.lock().await;
        locked.list_tools().await?
    };

    Ok(tools
        .iter()
        .map(|info| McpTool::new(server_name, info, Arc::clone(&client)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mcp_tool_name_format() {
        assert_eq!(
            build_mcp_tool_name("my_server", "read_file"),
            "mcp__my_server__read_file"
        );
    }

    #[test]
    fn build_mcp_tool_name_prefix() {
        let name = build_mcp_tool_name("server", "tool");
        assert!(name.starts_with("mcp__"));
    }
}
