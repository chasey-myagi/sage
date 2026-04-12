// LLM module — Phase 2
pub mod types;
pub mod models;
pub mod keys;
pub mod transform;
pub mod stream;
pub mod openai_compat;

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

#[cfg(test)]
mod tests {
    use super::types::*;
    use super::LlmProvider;
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

        let model = Model {
            id: "test-model".into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };

        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            }],
            system_prompt: "You are helpful.".into(),
            max_tokens: 4096,
            temperature: None,
        };

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

        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };

        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 1024,
            temperature: None,
        };

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash"));
        assert!(matches!(
            &events[3],
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::ToolUse
        ));
    }

    #[tokio::test]
    async fn test_mock_provider_empty_stream() {
        let provider = MockLlmProvider::new(vec![]);
        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "".into(),
            api_key_env: "".into(),
            max_tokens: 0,
            context_window: 0,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 0,
            temperature: None,
        };
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

        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "".into(),
            api_key_env: "".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };

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

        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };

        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            }],
            system_prompt: "You are helpful.".into(),
            max_tokens: 4096,
            temperature: None,
        };

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "partial output"));
        assert!(matches!(&events[1], AssistantMessageEvent::Error(s) if s == "internal server error"));
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

        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };

        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 1024,
            temperature: None,
        };

        let events = provider.complete(&model, &context, &[]).await;
        assert_eq!(events.len(), 5);
        // Verify the sequence: text -> tool_call_start -> tool_call_delta -> tool_call_end -> error
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Let me check."));
        assert!(matches!(&events[1], AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash"));
        assert!(matches!(&events[2], AssistantMessageEvent::ToolCallDelta { id, .. } if id == "call_inter_001"));
        assert!(matches!(&events[3], AssistantMessageEvent::ToolCallEnd { id } if id == "call_inter_001"));
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

        let model = Model {
            id: "test".into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        };
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };

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

    use crate::test_helpers::{test_model, test_context};

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
