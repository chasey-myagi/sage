// MCP (Model Context Protocol) — Stdio transport + client.
//
// Phase 1 MVP: basic JSON-RPC 2.0 over stdio, initialize handshake,
// tools/list and tools/call support.

pub mod client;
pub mod transport;
pub mod types;

pub use client::{McpClient, McpError};
pub use types::{McpServerConfig, McpToolInfo, McpToolResult};
