// LLM module — Phase 2
pub mod keys;
pub mod models;
pub mod openai_compat;
pub mod providers;
pub mod registry;
pub mod stream;
pub mod transform;
pub mod types;

use std::sync::Arc;
use types::*;

/// Trait for LLM providers (OpenAI-compatible API).
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent>;
}

// ---------------------------------------------------------------------------
// Public convenience API (matching pi-mono's index.ts)
// ---------------------------------------------------------------------------

/// Register all built-in providers.
pub fn register_builtin_providers() {
    use providers::*;
    registry::register_provider(Arc::new(AnthropicProvider::new()));
    registry::register_provider(Arc::new(OpenAiCompletionsProvider::new()));
    registry::register_provider(Arc::new(OpenAiResponsesProvider::new()));
    registry::register_provider(Arc::new(GoogleProvider::new()));
    registry::register_provider(Arc::new(AzureOpenAiResponsesProvider::new()));
}

/// Stream a completion using the model's registered API provider.
pub async fn stream(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &registry::StreamOptions,
) -> Result<Vec<AssistantMessageEvent>, String> {
    let provider = registry::resolve_provider(&model.api)?;
    Ok(provider.stream(model, context, tools, options).await)
}

/// Complete a request (non-streaming convenience — collects all events).
pub async fn complete(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &registry::StreamOptions,
) -> Result<Vec<AssistantMessageEvent>, String> {
    stream(model, context, tools, options).await
}

#[cfg(test)]
mod tests {
    use super::LlmProvider;
    use super::types::*;
    use crate::test_helpers::{test_context, test_model};
    use crate::types::*;

    // ========================================================================
    // LlmProvider trait — mock implementation
    // ========================================================================

    /// A mock provider that returns a predetermined sequence of events.
    struct MockLlmProvider {
        events: Vec<AssistantMessageEvent>,
    }

    impl MockLlmProvider {
        fn new(events: Vec<AssistantMessageEvent>) -> Self {
            Self { events }
        }
    }

    #[async_trait::async_trait]
    impl super::LlmProvider for MockLlmProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            self.events.clone()
        }
    }

    #[tokio::test]
    async fn test_mock_provider_returns_events() {
        let provider = MockLlmProvider::new(vec![
            AssistantMessageEvent::TextDelta("Hello".into()),
            AssistantMessageEvent::TextDelta(" world".into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]);

        let model = test_model();
        let mut context = test_context();
        context.messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("hi".into())],
        }];
        context.system_prompt = "You are helpful.".into();

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Hello"));
        assert!(matches!(&events[2], AssistantMessageEvent::Done { .. }));
    }

    #[tokio::test]
    async fn test_mock_provider_with_tool_call_events() {
        let provider = MockLlmProvider::new(vec![
            AssistantMessageEvent::ToolCallStart {
                id: "call_001".into(),
                name: "bash".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "call_001".into(),
                arguments_delta: r#"{"command":"ls"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "call_001".into(),
            },
            AssistantMessageEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ]);

        let model = test_model();
        let context = test_context();

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 4);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        );
        assert!(matches!(
            &events[3],
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::ToolUse
        ));
    }

    #[tokio::test]
    async fn test_mock_provider_empty_stream() {
        let provider = MockLlmProvider::new(vec![]);
        let model = test_model();
        let context = test_context();
        let events = provider.complete(&model, &context, &[]).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_mock_provider_with_tools() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a command".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];

        let provider = MockLlmProvider::new(vec![AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        }]);

        let model = test_model();
        let context = test_context();

        let events = provider.complete(&model, &context, &tools).await;
        assert_eq!(events.len(), 1);
    }

    // ========================================================================
    // MockLlmProvider — Error event behavior
    // ========================================================================

    #[tokio::test]
    async fn test_mock_provider_returns_error_event() {
        let provider = MockLlmProvider::new(vec![
            AssistantMessageEvent::TextDelta("partial output".into()),
            AssistantMessageEvent::Error("internal server error".into()),
        ]);

        let model = test_model();
        let context = test_context();

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "partial output"));
        assert!(
            matches!(&events[1], AssistantMessageEvent::Error(s) if s == "internal server error")
        );
    }

    // ========================================================================
    // MockLlmProvider — interleaved text + tool_call + error
    // ========================================================================

    #[tokio::test]
    async fn test_mock_provider_interleaved_events() {
        let provider = MockLlmProvider::new(vec![
            AssistantMessageEvent::TextDelta("Let me check.".into()),
            AssistantMessageEvent::ToolCallStart {
                id: "call_inter_001".into(),
                name: "bash".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "call_inter_001".into(),
                arguments_delta: r#"{"command":"ls"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "call_inter_001".into(),
            },
            AssistantMessageEvent::Error("connection reset".into()),
        ]);

        let model = test_model();
        let context = test_context();

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 5);
        // Verify the sequence: text -> tool_call_start -> tool_call_delta -> tool_call_end -> error
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Let me check."));
        assert!(
            matches!(&events[1], AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        );
        assert!(
            matches!(&events[2], AssistantMessageEvent::ToolCallDelta { id, .. } if id == "call_inter_001")
        );
        assert!(
            matches!(&events[3], AssistantMessageEvent::ToolCallEnd { id } if id == "call_inter_001")
        );
        assert!(matches!(&events[4], AssistantMessageEvent::Error(s) if s == "connection reset"));
    }

    // ========================================================================
    // 状态组合: MockProvider stream → IncrementalJsonParser 联动
    // ========================================================================

    #[tokio::test]
    async fn test_provider_stream_feeds_incremental_parser() {
        use super::stream::IncrementalJsonParser;

        // Provider returns tool call deltas that together form valid JSON args
        let provider = MockLlmProvider::new(vec![
            AssistantMessageEvent::ToolCallStart {
                id: "call_parse".into(),
                name: "bash".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "call_parse".into(),
                arguments_delta: r#"{"com"#.into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "call_parse".into(),
                arguments_delta: r#"mand":"ls -la"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "call_parse".into(),
            },
            AssistantMessageEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ]);

        let model = test_model();
        let context = test_context();

        let events = provider.complete(&model, &context, &[]).await;

        // Feed tool call deltas into IncrementalJsonParser
        let mut parser = IncrementalJsonParser::new();
        for event in &events {
            if let AssistantMessageEvent::ToolCallDelta {
                arguments_delta, ..
            } = event
            {
                parser.push(&arguments_delta);
            }
        }

        let args = parser.complete().unwrap();
        assert_eq!(args["command"], "ls -la");
    }

    // ========================================================================
    // Helper: reusable test model/context constructors
    // ========================================================================

    #[test]
    fn test_helper_model_is_valid() {
        let m = test_model();
        assert!(m.max_tokens <= m.context_window);
        assert!(!m.id.is_empty());
    }

    #[test]
    fn test_helper_context_is_valid() {
        let c = test_context();
        assert!(c.max_tokens > 0);
    }
}
