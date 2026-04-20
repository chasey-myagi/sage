// Event stream — mirrors pi-mono AgentEvent + sage-runtime event.rs.
//
// AgentEvent is the union of all observable events emitted by the agent loop.
// Maps 1:1 with pi-mono's AgentEvent type in packages/agent/src/types.ts.

use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// All observable events emitted by the agent loop.
///
/// Mirrors pi-mono's AgentEvent type (packages/agent/src/types.ts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    // ── Agent lifecycle ──────────────────────────────────────────────
    /// The agent loop has started.
    AgentStart,
    /// The agent loop has completed. `messages` is all new messages produced.
    AgentEnd {
        messages: Vec<AgentMessage>,
    },
    /// A fatal error occurred; the run will not continue normally.
    RunError {
        error: String,
    },
    // ── Turn lifecycle ───────────────────────────────────────────────
    /// A new turn (LLM call + tool round) has begun.
    TurnStart,
    /// The current turn has completed.
    TurnEnd {
        message: AssistantMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    // ── Message lifecycle ────────────────────────────────────────────
    /// A new message object has been created (streaming started or fully received).
    MessageStart {
        message: AgentMessage,
    },
    /// A streaming text delta was received during assistant message streaming.
    MessageUpdate {
        message: AgentMessage,
        delta: String,
    },
    /// A message has been fully received.
    MessageEnd {
        message: AgentMessage,
    },
    // ── Tool execution lifecycle ─────────────────────────────────────
    /// A tool call has started.
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    /// A tool is streaming incremental output.
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        #[serde(rename = "partialResult")]
        partial_result: String,
    },
    /// A tool call has completed.
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: AgentToolResult,
        is_error: bool,
    },
    // ── Compaction ───────────────────────────────────────────────────
    /// Context compaction has started.
    CompactionStart {
        reason: String,
        message_count: usize,
    },
    /// Context compaction has completed.
    CompactionEnd {
        tokens_before: u64,
        messages_compacted: usize,
    },
}

/// Who an [`AgentEvent`] is intended for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    User,
    Developer,
    Internal,
}

impl AgentEvent {
    /// Return the intended audience for this event.
    pub fn visibility(&self) -> Visibility {
        match self {
            AgentEvent::MessageUpdate { .. } | AgentEvent::RunError { .. } => Visibility::User,
            AgentEvent::ToolExecutionStart { .. }
            | AgentEvent::ToolExecutionUpdate { .. }
            | AgentEvent::ToolExecutionEnd { .. }
            | AgentEvent::MessageStart { .. }
            | AgentEvent::MessageEnd { .. } => Visibility::Developer,
            AgentEvent::AgentStart
            | AgentEvent::AgentEnd { .. }
            | AgentEvent::TurnStart
            | AgentEvent::TurnEnd { .. }
            | AgentEvent::CompactionStart { .. }
            | AgentEvent::CompactionEnd { .. } => Visibility::Internal,
        }
    }
}

/// Trait for receiving agent events.
#[async_trait::async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}

// ── EventStream implementation ─────────────────────────────────────

enum StreamItem<T, R> {
    Event(T),
    End(R),
}

/// Error returned when sending to a closed or dropped stream.
#[derive(Debug)]
pub struct SendError(String);

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SendError {}

/// Sends events into an EventStream.
pub struct EventSender<T, R> {
    tx: mpsc::UnboundedSender<StreamItem<T, R>>,
    ended: Arc<AtomicBool>,
}

impl<T, R> Clone for EventSender<T, R> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            ended: Arc::clone(&self.ended),
        }
    }
}

impl<T, R> EventSender<T, R> {
    /// Send an event. Fails if the receiver has been dropped or `end` was already called.
    pub fn send(&self, event: T) -> Result<(), SendError> {
        if self.ended.load(Ordering::SeqCst) {
            return Err(SendError("stream has ended".into()));
        }
        self.tx
            .send(StreamItem::Event(event))
            .map_err(|_| SendError("receiver has been dropped".into()))
    }

    /// Signal end-of-stream and deliver the final result. Idempotent.
    pub fn end(&self, result: R) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(StreamItem::End(result));
    }
}

/// Receives events from an EventStream.
pub struct EventReceiver<T, R> {
    rx: mpsc::UnboundedReceiver<StreamItem<T, R>>,
    stored_result: Option<R>,
    done: bool,
}

impl<T, R: Default> EventReceiver<T, R> {
    /// Receive the next event, or `None` when the stream has ended.
    pub async fn next(&mut self) -> Option<T> {
        if self.done {
            return None;
        }
        match self.rx.recv().await {
            Some(StreamItem::Event(e)) => Some(e),
            Some(StreamItem::End(r)) => {
                self.stored_result = Some(r);
                self.done = true;
                None
            }
            None => {
                self.done = true;
                None
            }
        }
    }

    /// Drain all remaining events and return the final result value.
    pub async fn result(mut self) -> R {
        while self.next().await.is_some() {}
        self.stored_result.take().unwrap_or_default()
    }
}

/// Async event stream backed by an unbounded mpsc channel.
pub struct EventStream<T, R> {
    _phantom: PhantomData<(T, R)>,
}

impl<T, R: Default> EventStream<T, R> {
    /// Create a new `(EventSender, EventReceiver)` pair.
    pub fn new() -> (EventSender<T, R>, EventReceiver<T, R>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let sender = EventSender {
            tx,
            ended: Arc::new(AtomicBool::new(false)),
        };
        let receiver = EventReceiver {
            rx,
            stored_result: None,
            done: false,
        };
        (sender, receiver)
    }
}

/// Type alias for agent event streams.
pub type AgentEventStream = EventStream<AgentEvent, Vec<AgentMessage>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct MockEventSink {
        events: Arc<Mutex<Vec<AgentEvent>>>,
    }

    impl MockEventSink {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentEventSink for MockEventSink {
        async fn emit(&self, event: AgentEvent) {
            self.events.lock().await.push(event);
        }
    }

    #[tokio::test]
    async fn stream_send_one_event_and_receive() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.send(AgentEvent::AgentStart).unwrap();
        sender.end(vec![]);
        let event = receiver.next().await;
        assert!(matches!(event, Some(AgentEvent::AgentStart)));
    }

    #[tokio::test]
    async fn stream_multiple_events_received_in_fifo_order() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.send(AgentEvent::AgentStart).unwrap();
        sender.send(AgentEvent::TurnStart).unwrap();
        sender
            .send(AgentEvent::AgentEnd { messages: vec![] })
            .unwrap();
        sender.end(vec![]);
        let e1 = receiver.next().await.unwrap();
        let e2 = receiver.next().await.unwrap();
        let e3 = receiver.next().await.unwrap();
        assert!(matches!(e1, AgentEvent::AgentStart));
        assert!(matches!(e2, AgentEvent::TurnStart));
        assert!(matches!(e3, AgentEvent::AgentEnd { .. }));
    }

    #[tokio::test]
    async fn stream_end_without_events_still_returns_result() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.end(vec![]);
        let none = receiver.next().await;
        assert!(none.is_none());
        let result = receiver.result().await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn stream_send_after_end_returns_error() {
        let (sender, _receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.end(vec![]);
        let result = sender.send(AgentEvent::AgentStart);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stream_dropping_sender_completes_receiver() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.send(AgentEvent::AgentStart).unwrap();
        drop(sender);
        let event = receiver.next().await;
        assert!(matches!(event, Some(AgentEvent::AgentStart)));
        let none = receiver.next().await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn stream_result_before_drain_auto_drains() {
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.send(AgentEvent::AgentStart).unwrap();
        sender.send(AgentEvent::TurnStart).unwrap();
        sender.end(vec![AgentMessage::assistant("answer".into())]);
        let result = receiver.result().await;
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn mock_sink_collects_events() {
        let sink = MockEventSink::new();
        sink.emit(AgentEvent::AgentStart).await;
        sink.emit(AgentEvent::TurnStart).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn agent_event_visibility_user() {
        let ev = AgentEvent::MessageUpdate {
            message: AgentMessage::assistant("hi".into()),
            delta: "hi".into(),
        };
        assert_eq!(ev.visibility(), Visibility::User);
    }

    #[test]
    fn agent_event_visibility_developer() {
        let ev = AgentEvent::ToolExecutionStart {
            tool_call_id: "id1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({"command": "ls"}),
        };
        assert_eq!(ev.visibility(), Visibility::Developer);
    }

    #[test]
    fn agent_event_visibility_internal() {
        assert_eq!(AgentEvent::AgentStart.visibility(), Visibility::Internal);
        assert_eq!(AgentEvent::TurnStart.visibility(), Visibility::Internal);
    }

    #[test]
    fn run_error_is_user_visible() {
        let ev = AgentEvent::RunError {
            error: "something went wrong".into(),
        };
        assert_eq!(ev.visibility(), Visibility::User);
    }

    #[tokio::test]
    async fn stream_sender_is_cloneable_multi_producer() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        let sender2 = sender.clone();
        let t1 = tokio::spawn(async move {
            for _ in 0..5 {
                sender.send(AgentEvent::TurnStart).unwrap();
            }
        });
        let t2 = tokio::spawn(async move {
            for _ in 0..5 {
                sender2.send(AgentEvent::TurnStart).unwrap();
            }
        });
        t1.await.unwrap();
        t2.await.unwrap();
        let mut count = 0;
        while let Some(_) = receiver.next().await {
            count += 1;
        }
        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn stream_end_called_twice_second_is_noop() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.end(vec![AgentMessage::assistant("first".into())]);
        sender.end(vec![AgentMessage::assistant("second".into())]);
        while receiver.next().await.is_some() {}
        let result = receiver.result().await;
        assert_eq!(result.len(), 1);
    }
}
