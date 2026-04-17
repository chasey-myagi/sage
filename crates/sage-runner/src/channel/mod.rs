// channel/mod.rs — ChannelAdapter trait (Sprint 8) + FeishuChannel stub.
//
// Sprint 8 extends the `ChannelAdapter` trait from a simple `send(&str)` into
// an event-driven interface that:
//   * advertises platform-formatting hints to be injected into the agent's
//     system prompt (`channel_hints()`);
//   * declares the event visibility level it cares about (`visibility_filter()`);
//   * receives fully-typed `AgentEvent`s (`send(event)`), letting the channel
//     decide whether/how to render each event for its platform.
//
// The trait lives in `sage-runner` because it is part of the agent wiring
// surface; real implementations (e.g. `FeishuChannel`) live in their own
// platform crates (see `sage-channel-feishu`).

use anyhow::Result;
use async_trait::async_trait;

use sage_runtime::event::{AgentEvent, Visibility};

/// A bidirectional communication channel for an agent.
///
/// Implementors translate between platform-specific message formats
/// (Feishu cards, Slack blocks, etc.) and the agent's event stream.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// A short, human-readable platform name used in logs and diagnostics
    /// (e.g. `"feishu"`, `"slack"`).
    fn name(&self) -> &str;

    /// Return a block of platform-formatting instructions to be injected into
    /// the agent's system prompt.
    ///
    /// Example for Feishu: rules about markdown dialect, `<at user_id="...">`
    /// mention syntax, per-message length limits, supported block types.
    fn channel_hints(&self) -> &str;

    /// Which event audience this channel cares about.
    ///
    /// Defaults to [`Visibility::User`] — most channels only surface
    /// user-facing text and errors, not developer-level tool noise.
    fn visibility_filter(&self) -> Visibility {
        Visibility::User
    }

    /// Deliver an [`AgentEvent`] to the platform.
    ///
    /// Implementations are free to filter out events they don't care about
    /// (e.g. non-text deltas, internal lifecycle) — the returned future
    /// should resolve to `Ok(())` even when the event is intentionally dropped.
    async fn send(&self, event: AgentEvent) -> Result<()>;
}

/// Feishu (Lark) channel adapter — stub in `sage-runner`.
///
/// The real Sprint 8 implementation (HTTP client, webhook server, signature
/// verification) lives in the `sage-channel-feishu` crate. This stub exists
/// so that the trait extensions are exercised by unit tests without pulling
/// the full networking stack into `sage-runner`.
pub struct FeishuChannel {
    /// Outbound webhook URL for sending messages.
    pub webhook_url: String,
}

#[async_trait]
impl ChannelAdapter for FeishuChannel {
    fn name(&self) -> &str {
        "feishu"
    }

    fn channel_hints(&self) -> &str {
        // Minimal stub hints; the richer real implementation is in
        // `sage-channel-feishu`. Keep this non-empty so downstream callers
        // can rely on the invariant without special-casing stubs.
        "feishu: reply with plain text or feishu-card markdown"
    }

    async fn send(&self, _event: AgentEvent) -> Result<()> {
        todo!("stub FeishuChannel in sage-runner; use sage-channel-feishu for the real implementation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_runtime::event::{AgentEvent, Visibility};

    // ── trait default behaviour ─────────────────────────────────────────────

    #[test]
    fn visibility_filter_default_returns_user() {
        let ch = FeishuChannel {
            webhook_url: "https://example.invalid/hook".to_string(),
        };
        assert_eq!(ch.visibility_filter(), Visibility::User);
    }

    #[test]
    fn channel_hints_and_name_return_feishu_strings() {
        let ch = FeishuChannel {
            webhook_url: "https://example.invalid/hook".to_string(),
        };
        assert_eq!(ch.name(), "feishu");
        let hints = ch.channel_hints();
        assert!(!hints.is_empty(), "channel_hints must not be empty");
        assert!(
            hints.to_lowercase().contains("feishu"),
            "channel_hints should mention the platform name, got: {hints}"
        );
    }

    // An override-style impl to prove the default can be shadowed.
    struct DevChannel;

    #[async_trait]
    impl ChannelAdapter for DevChannel {
        fn name(&self) -> &str {
            "dev"
        }
        fn channel_hints(&self) -> &str {
            "dev: raw events"
        }
        fn visibility_filter(&self) -> Visibility {
            Visibility::Developer
        }
        async fn send(&self, _event: AgentEvent) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn visibility_filter_can_be_overridden() {
        let ch = DevChannel;
        assert_eq!(ch.visibility_filter(), Visibility::Developer);
    }
}
