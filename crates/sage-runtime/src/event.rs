// Event stream — Phase 1

use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// Agent lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    AgentStart,
    RunError {
        error: String,
    },
    AgentEnd {
        messages: Vec<AgentMessage>,
    },
    TurnStart,
    TurnEnd {
        message: AssistantMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: AgentMessage,
    },
    MessageUpdate {
        message: AgentMessage,
        delta: String,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        partial_result: String,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        is_error: bool,
    },
    CompactionStart {
        reason: String,
        message_count: usize,
    },
    CompactionEnd {
        tokens_before: u64,
        /// Number of messages summarized (replaced by one CompactionSummary message).
        messages_compacted: usize,
    },
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
    pub fn send(&self, event: T) -> Result<(), SendError> {
        if self.ended.load(Ordering::SeqCst) {
            return Err(SendError("stream has ended".into()));
        }
        self.tx
            .send(StreamItem::Event(event))
            .map_err(|_| SendError("receiver has been dropped".into()))
    }

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

    // ── helpers ──────────────────────────────────────────────────────

    /// A mock implementation of AgentEventSink that collects events.
    struct MockEventSink {
        events: Arc<Mutex<Vec<AgentEvent>>>,
    }

    impl MockEventSink {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn events(&self) -> Arc<Mutex<Vec<AgentEvent>>> {
            Arc::clone(&self.events)
        }
    }

    #[async_trait::async_trait]
    impl AgentEventSink for MockEventSink {
        async fn emit(&self, event: AgentEvent) {
            self.events.lock().await.push(event);
        }
    }

    // ── EventStream basic operations ────────────────────────────────

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
    async fn stream_events_then_result() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        sender.end(vec![]);

        // drain events
        let event = receiver.next().await;
        assert!(event.is_some());

        // stream terminates
        let none = receiver.next().await;
        assert!(none.is_none());

        // final result available
        let result = receiver.result().await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn stream_end_without_events_still_returns_result() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.end(vec![]);

        // no events
        let none = receiver.next().await;
        assert!(none.is_none());

        // result still available
        let result = receiver.result().await;
        assert!(result.is_empty());
    }

    // ── EventStream async behavior ──────────────────────────────────

    #[tokio::test]
    async fn stream_sender_and_receiver_in_different_tasks() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        let send_task = tokio::spawn(async move {
            sender.send(AgentEvent::AgentStart).unwrap();
            sender.send(AgentEvent::TurnStart).unwrap();
            sender.end(vec![]);
        });

        let recv_task = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = receiver.next().await {
                events.push(event);
            }
            events
        });

        send_task.await.unwrap();
        let events = recv_task.await.unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AgentEvent::AgentStart));
        assert!(matches!(events[1], AgentEvent::TurnStart));
    }

    #[tokio::test]
    async fn stream_receiver_awaits_until_sender_pushes() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        let recv_task = tokio::spawn(async move {
            // This will block until an event arrives
            receiver.next().await
        });

        // Small yield to ensure receiver is waiting
        tokio::task::yield_now().await;

        sender.send(AgentEvent::AgentStart).unwrap();
        sender.end(vec![]);

        let event = recv_task.await.unwrap();
        assert!(matches!(event, Some(AgentEvent::AgentStart)));
    }

    #[tokio::test]
    async fn stream_rapid_push_all_received() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        let count = 100;
        for i in 0..count {
            sender
                .send(AgentEvent::MessageUpdate {
                    message: AgentMessage::assistant(format!("msg-{i}")),
                    delta: format!("delta-{i}"),
                })
                .unwrap();
        }
        sender.end(vec![]);

        let mut received = 0;
        while let Some(_event) = receiver.next().await {
            received += 1;
        }
        assert_eq!(received, count);
    }

    // ── EventStream completion ──────────────────────────────────────

    #[tokio::test]
    async fn stream_terminates_after_end() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        sender.end(vec![]);

        // drain
        let _ = receiver.next().await;

        // must return None (stream terminated)
        let terminated = receiver.next().await;
        assert!(terminated.is_none());
    }

    #[tokio::test]
    async fn stream_result_returns_final_value() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        let final_msg = AgentMessage::assistant("final answer".to_string());
        sender.end(vec![final_msg]);

        // drain events (none)
        while receiver.next().await.is_some() {}

        let result = receiver.result().await;
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn stream_send_after_end_returns_error() {
        let (sender, _receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.end(vec![]);

        // Sending after end() should fail
        let result = sender.send(AgentEvent::AgentStart);
        assert!(result.is_err());
    }

    // ── EventStream drop behavior ───────────────────────────────────

    #[tokio::test]
    async fn stream_dropping_sender_completes_receiver() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        drop(sender);

        // Should still get the buffered event
        let event = receiver.next().await;
        assert!(matches!(event, Some(AgentEvent::AgentStart)));

        // Then stream terminates
        let none = receiver.next().await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn stream_dropping_receiver_causes_send_error() {
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        drop(receiver);

        let result = sender.send(AgentEvent::AgentStart);
        assert!(result.is_err());
    }

    // ── AgentEvent construction ─────────────────────────────────────

    #[tokio::test]
    async fn agent_event_all_variants_constructable() {
        let _start = AgentEvent::AgentStart;

        let _run_error = AgentEvent::RunError {
            error: "boom".to_string(),
        };

        let _end = AgentEvent::AgentEnd { messages: vec![] };

        let _turn_start = AgentEvent::TurnStart;

        let _turn_end = AgentEvent::TurnEnd {
            message: AssistantMessage::new("hello".to_string()),
            tool_results: vec![],
        };

        let _msg_start = AgentEvent::MessageStart {
            message: AgentMessage::assistant("hi".to_string()),
        };

        let _msg_update = AgentEvent::MessageUpdate {
            message: AgentMessage::assistant("hi".to_string()),
            delta: "h".to_string(),
        };

        let _msg_end = AgentEvent::MessageEnd {
            message: AgentMessage::assistant("hi".to_string()),
        };

        let _tool_start = AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-1".to_string(),
            tool_name: "bash".to_string(),
            args: serde_json::json!({"command": "ls"}),
        };

        let _tool_update = AgentEvent::ToolExecutionUpdate {
            tool_call_id: "tc-1".to_string(),
            tool_name: "bash".to_string(),
            partial_result: "file.txt\n".to_string(),
        };

        let _tool_end = AgentEvent::ToolExecutionEnd {
            tool_call_id: "tc-1".to_string(),
            tool_name: "bash".to_string(),
            is_error: false,
        };
    }

    #[tokio::test]
    async fn agent_event_agent_end_carries_messages() {
        let msgs = vec![
            AgentMessage::assistant("answer 1".to_string()),
            AgentMessage::assistant("answer 2".to_string()),
        ];
        let event = AgentEvent::AgentEnd {
            messages: msgs.clone(),
        };

        if let AgentEvent::AgentEnd { messages } = event {
            assert_eq!(messages.len(), 2);
        } else {
            panic!("expected AgentEnd variant");
        }
    }

    #[tokio::test]
    async fn agent_event_turn_end_carries_assistant_msg_and_tool_results() {
        let assistant = AssistantMessage::new("thinking...".to_string());
        let tool_result = ToolResultMessage {
            tool_call_id: "tc-42".to_string(),
            tool_name: "bash".to_string(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            is_error: false,
            timestamp: 0,
        };

        let event = AgentEvent::TurnEnd {
            message: assistant,
            tool_results: vec![tool_result],
        };

        if let AgentEvent::TurnEnd {
            message,
            tool_results,
        } = event
        {
            assert_eq!(tool_results.len(), 1);
            assert_eq!(tool_results[0].tool_call_id, "tc-42");
            assert_eq!(tool_results[0].tool_name, "bash");
            assert!(!tool_results[0].is_error);
            // Verify assistant message is present
            let _ = message;
        } else {
            panic!("expected TurnEnd variant");
        }
    }

    #[tokio::test]
    async fn agent_event_tool_execution_start_carries_metadata() {
        let args = serde_json::json!({
            "file_path": "/tmp/test.rs",
            "content": "fn main() {}"
        });
        let event = AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-99".to_string(),
            tool_name: "write_file".to_string(),
            args: args.clone(),
        };

        if let AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args: event_args,
        } = event
        {
            assert_eq!(tool_call_id, "tc-99");
            assert_eq!(tool_name, "write_file");
            assert_eq!(event_args, args);
        } else {
            panic!("expected ToolExecutionStart variant");
        }
    }

    // ── AgentEventSink trait ────────────────────────────────────────

    #[tokio::test]
    async fn mock_sink_collects_events() {
        let sink = MockEventSink::new();

        sink.emit(AgentEvent::AgentStart).await;
        sink.emit(AgentEvent::TurnStart).await;

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn mock_sink_events_in_order() {
        let sink = MockEventSink::new();

        sink.emit(AgentEvent::AgentStart).await;
        sink.emit(AgentEvent::TurnStart).await;
        sink.emit(AgentEvent::AgentEnd { messages: vec![] }).await;

        let events = sink.events.lock().await;
        assert!(matches!(events[0], AgentEvent::AgentStart));
        assert!(matches!(events[1], AgentEvent::TurnStart));
        assert!(matches!(events[2], AgentEvent::AgentEnd { .. }));
    }

    #[tokio::test]
    async fn mock_sink_async_emit_completes() {
        let sink = MockEventSink::new();
        let events_ref = sink.events();

        // emit from spawned task
        let handle = tokio::spawn(async move {
            sink.emit(AgentEvent::AgentStart).await;
        });

        handle.await.unwrap();

        let events = events_ref.lock().await;
        assert_eq!(events.len(), 1);
    }

    // ── Concurrency ─────────────────────────────────────────────────

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

        // Both senders dropped — receiver drains remaining events
        let mut count = 0;
        while let Some(_) = receiver.next().await {
            count += 1;
        }
        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn stream_two_tasks_pushing_events_all_received() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        let s1 = sender.clone();
        let s2 = sender.clone();
        drop(sender); // drop original so only s1 and s2 remain

        let t1 = tokio::spawn(async move {
            for i in 0..50 {
                s1.send(AgentEvent::MessageUpdate {
                    message: AgentMessage::assistant(format!("a-{i}")),
                    delta: format!("a-{i}"),
                })
                .unwrap();
            }
        });

        let t2 = tokio::spawn(async move {
            for i in 0..50 {
                s2.send(AgentEvent::MessageUpdate {
                    message: AgentMessage::assistant(format!("b-{i}")),
                    delta: format!("b-{i}"),
                })
                .unwrap();
            }
        });

        let (_, _) = tokio::join!(t1, t2);

        let mut count = 0;
        while let Some(_) = receiver.next().await {
            count += 1;
        }
        assert_eq!(count, 100);
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[tokio::test]
    async fn agent_end_with_empty_messages() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender
            .send(AgentEvent::AgentEnd { messages: vec![] })
            .unwrap();
        sender.end(vec![]);

        let event = receiver.next().await.unwrap();
        if let AgentEvent::AgentEnd { messages } = event {
            assert!(messages.is_empty());
        } else {
            panic!("expected AgentEnd");
        }
    }

    #[tokio::test]
    async fn stream_large_number_of_events() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        let total = 2000;

        let send_task = tokio::spawn(async move {
            for _ in 0..total {
                sender.send(AgentEvent::TurnStart).unwrap();
            }
            sender.end(vec![]);
        });

        let recv_task = tokio::spawn(async move {
            let mut count = 0u64;
            while let Some(_) = receiver.next().await {
                count += 1;
            }
            count
        });

        send_task.await.unwrap();
        let count = recv_task.await.unwrap();
        assert_eq!(count, total);
    }

    #[tokio::test]
    async fn stream_event_with_large_payload() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        // ~60 KB string
        let big_content = "x".repeat(60_000);
        let event = AgentEvent::MessageUpdate {
            message: AgentMessage::assistant(big_content.clone()),
            delta: big_content.clone(),
        };

        sender.send(event).unwrap();
        sender.end(vec![]);

        let received = receiver.next().await.unwrap();
        if let AgentEvent::MessageUpdate { delta, .. } = received {
            assert_eq!(delta.len(), 60_000);
        } else {
            panic!("expected MessageUpdate");
        }
    }

    // ── Type alias ──────────────────────────────────────────────────

    #[tokio::test]
    async fn agent_event_stream_type_alias_works() {
        // Verifies the type alias compiles and is usable
        let (sender, mut receiver): (
            EventSender<AgentEvent, Vec<AgentMessage>>,
            EventReceiver<AgentEvent, Vec<AgentMessage>>,
        ) = AgentEventStream::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        sender.end(vec![]);

        let event = receiver.next().await;
        assert!(matches!(event, Some(AgentEvent::AgentStart)));
    }

    // ── result() edge cases ─────────────────────────────────────────

    #[tokio::test]
    async fn stream_result_when_sender_dropped_without_end() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        drop(sender); // dropped WITHOUT calling end()

        // drain events
        while receiver.next().await.is_some() {}

        // result() should return a default/empty value or indicate incomplete
        // The exact behavior must be defined — this test documents it
        let result = receiver.result().await;
        assert!(result.is_empty()); // or should this panic/return Err?
    }

    #[tokio::test]
    async fn stream_result_consumes_receiver() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.end(vec![]);
        while receiver.next().await.is_some() {}

        let result = receiver.result().await;
        assert!(result.is_empty());
        // After calling result(), receiver is consumed — no more operations possible
        // This is enforced by Rust's ownership system
    }

    // ── Send error type verification ────────────────────────────────

    #[tokio::test]
    async fn stream_send_after_end_error_is_descriptive() {
        let (sender, _receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        sender.end(vec![]);

        let err = sender.send(AgentEvent::AgentStart).unwrap_err();
        // Verify the error contains useful information
        let err_msg = format!("{}", err);
        assert!(!err_msg.is_empty(), "error message should be non-empty");
    }

    #[tokio::test]
    async fn stream_send_to_dropped_receiver_error_is_descriptive() {
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        drop(receiver);

        let err = sender.send(AgentEvent::AgentStart).unwrap_err();
        let err_msg = format!("{}", err);
        assert!(!err_msg.is_empty(), "error message should be non-empty");
    }

    // ── end() called twice ─────────────────────────────────────────

    #[tokio::test]
    async fn stream_end_called_twice_second_is_noop_or_error() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.end(vec![AgentMessage::assistant("first".into())]);
        // Second end() — should be a no-op, panic, or return error
        // This test documents the chosen behavior
        sender.end(vec![AgentMessage::assistant("second".into())]);

        // drain
        while receiver.next().await.is_some() {}

        // result should be from the first end() call
        let result = receiver.result().await;
        assert_eq!(result.len(), 1);
    }

    // ── result() before draining events ────────────────────────────

    #[tokio::test]
    async fn stream_result_before_drain_blocks_or_returns() {
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        sender.send(AgentEvent::AgentStart).unwrap();
        sender.send(AgentEvent::TurnStart).unwrap();
        sender.end(vec![AgentMessage::assistant("answer".into())]);

        // Do NOT drain events — call result() directly
        // result() auto-drains remaining events before returning
        let result = receiver.result().await;
        assert_eq!(result.len(), 1);
    }

    // ── Interleaved clone sender operations ────────────────────────

    #[tokio::test]
    async fn stream_interleaved_clone_and_drop_operations() {
        let (sender, mut receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        // send on original
        sender.send(AgentEvent::AgentStart).unwrap();

        // clone
        let sender2 = sender.clone();

        // send on clone
        sender2.send(AgentEvent::TurnStart).unwrap();

        // drop original — clone should still work
        drop(sender);

        // send on clone after original dropped
        sender2
            .send(AgentEvent::TurnEnd {
                message: AssistantMessage {
                    content: vec![Content::Text {
                        text: "done".into(),
                    }],
                    provider: "test".into(),
                    model: "test".into(),
                    usage: Usage::default(),
                    stop_reason: StopReason::Stop,
                    error_message: None,
                    timestamp: 0,
                },
                tool_results: vec![],
            })
            .unwrap();

        // end on clone
        sender2.end(vec![]);

        // all 3 events should be received
        let mut count = 0;
        while let Some(_) = receiver.next().await {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    // ── AgentMessage::assistant() constructor test ──────────────────

    #[tokio::test]
    async fn agent_message_assistant_constructor_sets_defaults() {
        let msg = AgentMessage::assistant("hello world".to_string());
        match msg {
            AgentMessage::Assistant(a) => {
                assert_eq!(a.text(), "hello world");
            }
            _ => panic!("expected Assistant variant"),
        }
    }

    // ── AgentEvent serde roundtrip ──────────────────────────────────

    #[tokio::test]
    async fn agent_event_serde_roundtrip_all_variants() {
        let user_msg = AgentMessage::User(UserMessage::from_text("hello"));
        let assistant_msg = AssistantMessage {
            content: vec![Content::Text {
                text: "response".into(),
            }],
            provider: "test".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        let tool_result = ToolResultMessage {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            is_error: false,
            timestamp: 0,
        };

        let events: Vec<AgentEvent> = vec![
            AgentEvent::AgentStart,
            AgentEvent::RunError {
                error: "boom".into(),
            },
            AgentEvent::AgentEnd {
                messages: vec![user_msg.clone()],
            },
            AgentEvent::TurnStart,
            AgentEvent::TurnEnd {
                message: assistant_msg.clone(),
                tool_results: vec![tool_result.clone()],
            },
            AgentEvent::MessageStart {
                message: user_msg.clone(),
            },
            AgentEvent::MessageUpdate {
                message: AgentMessage::Assistant(assistant_msg.clone()),
                delta: "hello".into(),
            },
            AgentEvent::MessageEnd {
                message: user_msg.clone(),
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                args: serde_json::json!({"cmd": "ls"}),
            },
            AgentEvent::ToolExecutionUpdate {
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                partial_result: "partial output".into(),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                is_error: false,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).expect("serialize AgentEvent");
            let deserialized: AgentEvent =
                serde_json::from_str(&json).expect("deserialize AgentEvent");
            // Re-serialize to verify structural equality
            let json2 = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, json2, "roundtrip failed for event: {json}");
        }
    }

    // ── AgentEventSink concurrent emit ──────────────────────────────

    #[tokio::test]
    async fn mock_sink_concurrent_emit_from_multiple_tasks() {
        let sink = MockEventSink::new();
        let events = sink.events();

        let sink = Arc::new(sink);
        let mut handles = Vec::new();

        for i in 0..10 {
            let s = Arc::clone(&sink);
            handles.push(tokio::spawn(async move {
                s.emit(AgentEvent::MessageUpdate {
                    message: AgentMessage::assistant(format!("msg-{i}")),
                    delta: format!("delta-{i}"),
                })
                .await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let collected = events.lock().await;
        assert_eq!(
            collected.len(),
            10,
            "all concurrent emits should be collected"
        );
    }

    // ── Backpressure / channel semantics ────────────────────────────

    #[tokio::test]
    async fn stream_unbounded_does_not_block_sender() {
        // Verify that sending many events without consuming doesn't block
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        // Push 10000 events without consuming
        for _ in 0..10_000 {
            sender.send(AgentEvent::TurnStart).unwrap();
        }
        sender.end(vec![]);

        // Now consume — all should be there
        let mut count = 0;
        let mut receiver = receiver;
        while let Some(_) = receiver.next().await {
            count += 1;
        }
        assert_eq!(count, 10_000);
    }

    // ── ToolExecutionEnd with is_error=true ─────────────────────────

    #[tokio::test]
    async fn agent_event_tool_execution_end_with_error() {
        let event = AgentEvent::ToolExecutionEnd {
            tool_call_id: "tc_fail".into(),
            tool_name: "bash".into(),
            is_error: true,
        };

        match &event {
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                is_error,
            } => {
                assert_eq!(tool_call_id, "tc_fail");
                assert_eq!(tool_name, "bash");
                assert!(*is_error);
            }
            _ => panic!("wrong variant"),
        }
    }

    // ── TurnEnd with error tool result ──────────────────────────────

    #[tokio::test]
    async fn agent_event_turn_end_with_error_tool_result() {
        let assistant_msg = AssistantMessage {
            content: vec![Content::ToolCall {
                id: "tc_1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "rm -rf /"}),
            }],
            provider: "test".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        };

        let error_result = ToolResultMessage {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "permission denied".into(),
            }],
            is_error: true,
            timestamp: 0,
        };

        let event = AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results: vec![error_result],
        };

        match &event {
            AgentEvent::TurnEnd { tool_results, .. } => {
                assert_eq!(tool_results.len(), 1);
                assert!(tool_results[0].is_error);
            }
            _ => panic!("wrong variant"),
        }
    }
}
