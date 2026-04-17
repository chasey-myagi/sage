// channel/mod.rs — ChannelAdapter trait + FeishuChannel stub.
//
// Full Feishu implementation is Phase 2 (requires API tokens + webhook server).
// This module defines the abstraction so the trait is stable for callers.

use anyhow::Result;
use async_trait::async_trait;

/// A bidirectional communication channel for an agent.
///
/// Implementors translate between platform-specific message formats
/// (Feishu cards, Slack blocks, etc.) and the agent's plain text protocol.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Send a text reply to the channel.
    async fn send(&self, text: &str) -> Result<()>;
    /// A human-readable name for logging (e.g. "feishu", "slack").
    fn name(&self) -> &str;
}

/// Feishu (Lark) channel adapter — stub implementation.
///
/// To enable: set `FEISHU_WEBHOOK_URL` and implement the axum webhook handler
/// that receives incoming messages and routes them to the daemon socket.
pub struct FeishuChannel {
    /// Outbound webhook URL for sending card messages.
    pub webhook_url: String,
}

#[async_trait]
impl ChannelAdapter for FeishuChannel {
    async fn send(&self, text: &str) -> Result<()> {
        // TODO: POST a Feishu card message to self.webhook_url
        // Card format: { "msg_type": "text", "content": { "text": text } }
        tracing::info!(
            channel = "feishu",
            chars = text.len(),
            "channel send (stub — not wired)"
        );
        Ok(())
    }

    fn name(&self) -> &str {
        "feishu"
    }
}
