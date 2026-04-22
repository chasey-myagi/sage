//! Permission approval types for the interactive TUI.
//!
//! When the permission engine returns `PermissionDecision::Ask`, the agent task
//! sends an `ApprovalRequest` over the channel and awaits the `ApprovalResponse`
//! from the TUI's key handler.

use tokio::sync::oneshot;

/// A request for the TUI to ask the user whether to allow a tool call.
pub struct ApprovalRequest {
    pub tool_name: String,
    pub message: String,
    pub response_tx: oneshot::Sender<ApprovalResponse>,
}

/// The user's decision for a pending approval request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResponse {
    Allow,
    Deny,
}
