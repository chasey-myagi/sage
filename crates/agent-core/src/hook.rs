// Hook bus — Sprint 6 (S6.1)
// Unified HookEvent enum + HookHandler trait + broadcast bus.
// Phase 1: additive only — existing BeforeToolCallHook / AfterToolCallHook /
// StopHook traits in agent.rs are untouched. Phase 2 (S6.2) will wire those
// implementations to drive through HookHandler.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::broadcast;

/// Lifecycle events observable by hook handlers.
///
/// Variant names map 1:1 to the canonical hook event names used by YAML
/// configuration keys and the stdin JSON protocol (`"event"` field).
#[derive(Debug, Clone)]
pub enum HookEvent {
    SessionStart {
        session_id: String,
        agent_name: String,
        model: String,
    },
    UserPromptSubmit {
        session_id: String,
        text: String,
    },
    PreToolUse {
        session_id: String,
        tool_name: String,
        tool_call_id: String,
        args: Value,
    },
    PostToolUse {
        session_id: String,
        tool_name: String,
        tool_call_id: String,
        args: Value,
        is_error: bool,
        duration_ms: u64,
    },
    PreCompact {
        session_id: String,
        tokens_before: u64,
        message_count: usize,
    },
    PostCompact {
        session_id: String,
        tokens_before: u64,
        tokens_after: u64,
        messages_compacted: usize,
    },
    CompactFallback {
        session_id: String,
        tokens_before: u64,
        tokens_after: u64,
        messages_truncated: usize,
    },
    Stop {
        session_id: String,
        agent_name: String,
        model: String,
        turn_count: u32,
        stop_reason: String,
        last_assistant_message: String,
    },
    SessionEnd {
        session_id: String,
        duration_ms: u64,
        turn_count: u32,
        success: bool,
    },
}

impl HookEvent {
    /// Canonical event name — matches YAML config keys and stdin JSON
    /// `"event"` field (CC hook protocol).
    pub fn name(&self) -> &'static str {
        match self {
            HookEvent::SessionStart { .. } => "SessionStart",
            HookEvent::UserPromptSubmit { .. } => "UserPromptSubmit",
            HookEvent::PreToolUse { .. } => "PreToolUse",
            HookEvent::PostToolUse { .. } => "PostToolUse",
            HookEvent::PreCompact { .. } => "PreCompact",
            HookEvent::PostCompact { .. } => "PostCompact",
            HookEvent::CompactFallback { .. } => "CompactFallback",
            HookEvent::Stop { .. } => "Stop",
            HookEvent::SessionEnd { .. } => "SessionEnd",
        }
    }

    /// Session this event belongs to.
    pub fn session_id(&self) -> &str {
        match self {
            HookEvent::SessionStart { session_id, .. }
            | HookEvent::UserPromptSubmit { session_id, .. }
            | HookEvent::PreToolUse { session_id, .. }
            | HookEvent::PostToolUse { session_id, .. }
            | HookEvent::PreCompact { session_id, .. }
            | HookEvent::PostCompact { session_id, .. }
            | HookEvent::CompactFallback { session_id, .. }
            | HookEvent::Stop { session_id, .. }
            | HookEvent::SessionEnd { session_id, .. } => session_id,
        }
    }
}

/// Outcome returned by a [`HookHandler`]. `Intervene` is only honored by the
/// engine for events where intervention is meaningful (e.g. `PreToolUse`,
/// `Stop`); for observe-only events it is reported but not acted upon.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    Allow,
    Intervene { message: String },
}

#[async_trait::async_trait]
pub trait HookHandler: Send + Sync {
    async fn handle(&self, event: &HookEvent) -> HookOutcome;
}

/// Multi-subscriber bus for [`HookEvent`]. The engine emits; handlers
/// subscribe. Bounded channel — slow subscribers observing `RecvError::Lagged`
/// is the caller's responsibility.
///
/// When constructed via [`HookBus::with_session_start_replay`], the bus caches
/// the last `SessionStart` event so a subscriber attaching after the engine
/// has already emitted it (the common `SageEngine::session()` →
/// `session.hook_bus().subscribe()` pattern) still observes the start event.
/// Replay is intentionally scoped to `SessionStart` alone — all other variants
/// follow pure broadcast semantics (subscribe-before-emit-or-miss) regardless
/// of construction mode.
#[derive(Clone)]
pub struct HookBus {
    tx: broadcast::Sender<HookEvent>,
    last_session_start: Option<Arc<Mutex<Option<HookEvent>>>>,
}

impl HookBus {
    /// Construct a bus with pure broadcast semantics: emits prior to any
    /// subscription are unconditionally dropped.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            last_session_start: None,
        }
    }

    /// Construct a bus that caches the most recent `SessionStart` emit and
    /// replays it once to any subsequent subscriber. Used by [`SageSession`]
    /// so that the engine can emit `SessionStart` synchronously during
    /// session construction without racing against the caller's `subscribe()`.
    ///
    /// [`SageSession`]: crate::engine::SageSession
    pub fn with_session_start_replay(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            last_session_start: Some(Arc::new(Mutex::new(None))),
        }
    }

    /// Subscribe to the bus. If this bus was built with session-start replay
    /// enabled and a `SessionStart` has already been cached, the returned
    /// [`HookReceiver`] yields the cached event first before forwarding live
    /// broadcasts.
    pub fn subscribe(&self) -> HookReceiver {
        // Clone the cached SessionStart under the lock so the snapshot is
        // consistent with the broadcast rx we attach next — any SessionStart
        // emitted after this point is observed through the live channel.
        let pending = self
            .last_session_start
            .as_ref()
            .and_then(|cache| cache.lock().ok().and_then(|g| g.clone()));
        HookReceiver {
            rx: self.tx.subscribe(),
            pending,
        }
    }

    /// Non-blocking emit. Drops the event if no subscribers are attached
    /// (broadcast semantics). When replay is enabled, a `SessionStart` emit
    /// is additionally cached so late subscribers can replay it exactly once
    /// on the next `subscribe()` call.
    pub fn emit(&self, event: HookEvent) {
        if let (Some(cache), HookEvent::SessionStart { .. }) =
            (self.last_session_start.as_ref(), &event)
            && let Ok(mut guard) = cache.lock()
        {
            *guard = Some(event.clone());
        }
        let _ = self.tx.send(event);
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for HookBus {
    fn default() -> Self {
        Self::new(256)
    }
}

/// Subscriber handle returned by [`HookBus::subscribe`].
///
/// Wraps a [`broadcast::Receiver`] with a one-shot `SessionStart` replay slot
/// populated when the source bus has replay enabled. The first `recv` /
/// `try_recv` yields the cached event (if any); subsequent calls behave
/// identically to the raw broadcast receiver.
pub struct HookReceiver {
    rx: broadcast::Receiver<HookEvent>,
    pending: Option<HookEvent>,
}

impl HookReceiver {
    pub async fn recv(&mut self) -> Result<HookEvent, broadcast::error::RecvError> {
        if let Some(e) = self.pending.take() {
            return Ok(e);
        }
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Result<HookEvent, broadcast::error::TryRecvError> {
        if let Some(e) = self.pending.take() {
            return Ok(e);
        }
        self.rx.try_recv()
    }

    /// Produce an independent receiver attached to the **live** broadcast
    /// stream only; any SessionStart replay still pending on `self` is NOT
    /// propagated. Callers who want a fresh SessionStart replay should call
    /// `HookBus::subscribe` again instead.
    pub fn resubscribe(&self) -> HookReceiver {
        HookReceiver {
            rx: self.rx.resubscribe(),
            pending: None,
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::broadcast::error::TryRecvError;

    // ── sample constructors ──────────────────────────────────────────

    fn sample_session_start(sid: &str) -> HookEvent {
        HookEvent::SessionStart {
            session_id: sid.into(),
            agent_name: "test-agent".into(),
            model: "test-model".into(),
        }
    }

    fn sample_user_prompt_submit(sid: &str) -> HookEvent {
        HookEvent::UserPromptSubmit {
            session_id: sid.into(),
            text: "hello".into(),
        }
    }

    fn sample_pre_tool_use(sid: &str) -> HookEvent {
        HookEvent::PreToolUse {
            session_id: sid.into(),
            tool_name: "bash".into(),
            tool_call_id: "tc-1".into(),
            args: json!({"command": "ls"}),
        }
    }

    fn sample_post_tool_use(sid: &str) -> HookEvent {
        HookEvent::PostToolUse {
            session_id: sid.into(),
            tool_name: "bash".into(),
            tool_call_id: "tc-1".into(),
            args: json!({"command": "ls"}),
            is_error: false,
            duration_ms: 12,
        }
    }

    fn sample_pre_compact(sid: &str) -> HookEvent {
        HookEvent::PreCompact {
            session_id: sid.into(),
            tokens_before: 100_000,
            message_count: 50,
        }
    }

    fn sample_post_compact(sid: &str) -> HookEvent {
        HookEvent::PostCompact {
            session_id: sid.into(),
            tokens_before: 100_000,
            tokens_after: 20_000,
            messages_compacted: 40,
        }
    }

    fn sample_stop(sid: &str) -> HookEvent {
        HookEvent::Stop {
            session_id: sid.into(),
            agent_name: "test-agent".into(),
            model: "test-model".into(),
            turn_count: 3,
            stop_reason: "stop".into(),
            last_assistant_message: "done".into(),
        }
    }

    fn sample_session_end(sid: &str) -> HookEvent {
        HookEvent::SessionEnd {
            session_id: sid.into(),
            duration_ms: 1000,
            turn_count: 3,
            success: true,
        }
    }

    // ── HookEvent::name() — one per variant ───────────────────────────

    #[test]
    fn session_start_name_returns_SessionStart() {
        assert_eq!(sample_session_start("s").name(), "SessionStart");
    }

    #[test]
    fn user_prompt_submit_name_returns_UserPromptSubmit() {
        assert_eq!(sample_user_prompt_submit("s").name(), "UserPromptSubmit");
    }

    #[test]
    fn pre_tool_use_name_returns_PreToolUse() {
        assert_eq!(sample_pre_tool_use("s").name(), "PreToolUse");
    }

    #[test]
    fn post_tool_use_name_returns_PostToolUse() {
        assert_eq!(sample_post_tool_use("s").name(), "PostToolUse");
    }

    #[test]
    fn pre_compact_name_returns_PreCompact() {
        assert_eq!(sample_pre_compact("s").name(), "PreCompact");
    }

    #[test]
    fn post_compact_name_returns_PostCompact() {
        assert_eq!(sample_post_compact("s").name(), "PostCompact");
    }

    #[test]
    fn stop_name_returns_Stop() {
        assert_eq!(sample_stop("s").name(), "Stop");
    }

    #[test]
    fn session_end_name_returns_SessionEnd() {
        assert_eq!(sample_session_end("s").name(), "SessionEnd");
    }

    // ── HookEvent::session_id() ──────────────────────────────────────

    #[test]
    fn session_start_session_id_returns_field_value() {
        assert_eq!(sample_session_start("sess-abc").session_id(), "sess-abc");
    }

    #[test]
    fn pre_tool_use_session_id_returns_field_value() {
        assert_eq!(sample_pre_tool_use("sess-xyz").session_id(), "sess-xyz");
    }

    #[test]
    fn session_end_session_id_returns_field_value() {
        assert_eq!(sample_session_end("sess-end").session_id(), "sess-end");
    }

    // ── Clone + Debug smoke test ─────────────────────────────────────

    #[test]
    fn hook_event_clone_and_debug_do_not_panic() {
        let ev = sample_pre_tool_use("s");
        let cloned = ev.clone();
        assert_eq!(cloned.session_id(), "s");
        let dbg = format!("{ev:?}");
        assert!(dbg.contains("PreToolUse"));
    }

    // ── HookBus::new + Default ───────────────────────────────────────

    #[test]
    fn bus_new_with_capacity_creates_channel() {
        let bus = HookBus::new(16);
        // A fresh bus has no receivers attached.
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn bus_default_uses_256_capacity() {
        // Indirect verification: Default::default() does not panic, and a
        // sub/emit cycle works (exact capacity check deferred — emit is
        // non-blocking regardless of capacity when there are no receivers).
        let bus = HookBus::default();
        // Should be able to emit 257 events without subscribers — no panic.
        for i in 0..257 {
            bus.emit(sample_user_prompt_submit(&format!("s-{i}")));
        }
        assert_eq!(bus.subscriber_count(), 0);
    }

    // ── HookBus::subscribe + emit ────────────────────────────────────

    #[tokio::test]
    async fn subscribe_then_emit_receives_event() {
        let bus = HookBus::new(8);
        let mut rx = bus.subscribe();
        bus.emit(sample_session_start("s1"));
        let got = rx.recv().await.expect("should receive event");
        assert_eq!(got.session_id(), "s1");
        assert_eq!(got.name(), "SessionStart");
    }

    #[test]
    fn emit_without_subscribers_is_noop() {
        let bus = HookBus::new(8);
        // No panic, no blocking — just dropped.
        bus.emit(sample_session_start("s1"));
        bus.emit(sample_session_end("s1"));
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn two_subscribers_both_receive_same_event() {
        let bus = HookBus::new(8);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        bus.emit(sample_session_start("shared"));
        let e1 = rx1.recv().await.expect("rx1 should receive");
        let e2 = rx2.recv().await.expect("rx2 should receive");
        assert_eq!(e1.session_id(), "shared");
        assert_eq!(e2.session_id(), "shared");
    }

    #[tokio::test]
    async fn subscribe_before_emit_receives_only_subsequent_events() {
        // Broadcast semantics: subscribers only receive events emitted AFTER
        // their subscribe() call.
        let bus = HookBus::new(8);
        bus.emit(sample_session_start("first")); // no subs → dropped
        let mut rx = bus.subscribe();
        bus.emit(sample_session_start("second"));
        let got = rx.recv().await.expect("should receive second");
        assert_eq!(got.session_id(), "second");
        // No more events pending.
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn subscriber_count_reflects_live_subscribers() {
        let bus = HookBus::new(8);
        assert_eq!(bus.subscriber_count(), 0);
        let rx1 = bus.subscribe();
        let rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        drop(rx1);
        assert_eq!(bus.subscriber_count(), 1);
        drop(rx2);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn subscriber_drops_unreceived_old_events() {
        // Capacity = 2; emit 3 events; the oldest is dropped. The next
        // recv() observes Lagged(1) per tokio broadcast semantics.
        let bus = HookBus::new(2);
        let mut rx = bus.subscribe();
        bus.emit(sample_session_start("s1"));
        bus.emit(sample_session_start("s2"));
        bus.emit(sample_session_start("s3"));

        // First recv after overflow returns Lagged error indicating how many
        // were dropped.
        match rx.recv().await {
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                assert!(n >= 1, "expected at least 1 lagged event, got {n}");
            }
            other => panic!("expected Lagged error, got {other:?}"),
        }

        // Subsequent recvs yield the events still in the buffer.
        let a = rx.recv().await.expect("should recv surviving event");
        let b = rx.recv().await.expect("should recv surviving event");
        // Surviving events are the last two (s2, s3).
        assert_eq!(a.session_id(), "s2");
        assert_eq!(b.session_id(), "s3");
    }

    // ── HookHandler stub ─────────────────────────────────────────────

    struct AllowAll;

    #[async_trait::async_trait]
    impl HookHandler for AllowAll {
        async fn handle(&self, _event: &HookEvent) -> HookOutcome {
            HookOutcome::Allow
        }
    }

    #[tokio::test]
    async fn allow_all_handler_returns_allow() {
        let handler = AllowAll;
        let outcome = handler.handle(&sample_pre_tool_use("s")).await;
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    fn sample_compact_fallback(sid: &str) -> HookEvent {
        HookEvent::CompactFallback {
            session_id: sid.into(),
            tokens_before: 80_000,
            tokens_after: 40_000,
            messages_truncated: 10,
        }
    }

    #[test]
    fn compact_fallback_name_returns_CompactFallback() {
        assert_eq!(sample_compact_fallback("s").name(), "CompactFallback");
    }

    #[test]
    fn compact_fallback_session_id_returns_field_value() {
        assert_eq!(
            sample_compact_fallback("sess-fallback").session_id(),
            "sess-fallback"
        );
    }
}
