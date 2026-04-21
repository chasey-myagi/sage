// MCP client — wraps StdioTransport and implements the MCP handshake + tool calls.

use super::transport::{StdioTransport, TransportError};
use super::types::{McpClientInfo, McpServerConfig, McpServerInfo, McpToolInfo, McpToolResult};

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct McpClient {
    transport: StdioTransport,
    pub server_info: Option<McpServerInfo>,
}

impl McpClient {
    /// Spawn the MCP server and perform the initialize handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
        let transport = StdioTransport::spawn(&config.command, &config.args).await?;
        let mut client = Self {
            transport,
            server_info: None,
        };
        client.initialize(McpClientInfo::default()).await?;
        Ok(client)
    }

    async fn initialize(&mut self, client_info: McpClientInfo) -> Result<(), McpError> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "clientInfo": {
                "name": client_info.name,
                "version": client_info.version,
            }
        });

        let result = self.transport.send_request("initialize", params).await?;

        if let Ok(info) = serde_json::from_value::<McpServerInfo>(result) {
            self.server_info = Some(info);
        }

        // Send the initialized notification (fire-and-forget; no response expected).
        // We ignore errors here — some servers don't require it.
        let _ = self
            .send_notification("notifications/initialized", serde_json::json!({}))
            .await;

        Ok(())
    }

    /// Send a notification (no response expected).
    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), McpError> {
        self.transport.send_notification(method, params).await?;
        Ok(())
    }

    /// List all tools available on the connected MCP server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, McpError> {
        let result = self
            .transport
            .send_request("tools/list", serde_json::json!({}))
            .await?;

        let tools = result
            .get("tools")
            .and_then(|t| serde_json::from_value::<Vec<McpToolInfo>>(t.clone()).ok())
            .unwrap_or_default();

        Ok(tools)
    }

    /// Call a tool on the MCP server by its original (non-prefixed) name.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.transport.send_request("tools/call", params).await?;

        serde_json::from_value::<McpToolResult>(result)
            .map_err(|e| McpError::Protocol(format!("invalid tool result: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_error_display_transport() {
        let err = McpError::Protocol("bad response".to_string());
        assert!(err.to_string().contains("bad response"));
    }
}
