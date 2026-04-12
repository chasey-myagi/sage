// LLM-side types — Phase 2
// Defines types needed for LLM API calls (OpenAI-compatible format).

use crate::types::{StopReason, Usage};
use serde::{Deserialize, Serialize};

/// Content block in an LLM message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmContent {
    Text(String),
    Image { url: String },
}

/// Message in the LLM API format (OpenAI-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmMessage {
    System { content: String },
    User { content: Vec<LlmContent> },
    Assistant { content: String, tool_calls: Vec<LlmToolCall> },
    Tool { tool_call_id: String, content: String },
}

/// A tool call within an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub function: LlmFunctionCall,
}

/// Function details of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Context for an LLM API call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmContext {
    pub messages: Vec<LlmMessage>,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

/// Tool definition for the LLM API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Built-in model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    pub base_url: String,
    pub api_key_env: String,
    pub max_tokens: u32,
    pub context_window: u32,
    pub cost: ModelCost,
    pub compat: ProviderCompat,
}

/// Cost per million tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
}

/// Which field name the provider uses for max tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaxTokensField {
    MaxTokens,
    MaxCompletionTokens,
}

/// Format used for thinking/reasoning content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThinkingFormat {
    OpenAI,
    Qwen,
    Zai,
}

/// Provider-specific compatibility flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCompat {
    pub max_tokens_field: MaxTokensField,
    pub supports_reasoning_effort: bool,
    pub thinking_format: Option<ThinkingFormat>,
    pub requires_tool_result_name: bool,
    pub requires_assistant_after_tool_result: bool,
    pub supports_strict_mode: bool,
}

/// Events emitted during an assistant message stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssistantMessageEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, arguments_delta: String },
    ToolCallEnd { id: String },
    Usage(Usage),
    Done { stop_reason: StopReason },
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StopReason, Usage};
    use serde_json::json;

    // ========================================================================
    // LlmContent
    // ========================================================================

    #[test]
    fn test_llm_content_text() {
        let content = LlmContent::Text("hello world".into());
        match &content {
            LlmContent::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_llm_content_image() {
        let content = LlmContent::Image {
            url: "data:image/png;base64,abc123".into(),
        };
        match &content {
            LlmContent::Image { url } => assert!(url.starts_with("data:image/")),
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn test_llm_content_text_empty() {
        let content = LlmContent::Text(String::new());
        match &content {
            LlmContent::Text(s) => assert!(s.is_empty()),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_llm_content_serde_roundtrip_text() {
        let content = LlmContent::Text("hello".into());
        let json = serde_json::to_string(&content).unwrap();
        let back: LlmContent = serde_json::from_str(&json).unwrap();
        match back {
            LlmContent::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected Text after roundtrip"),
        }
    }

    #[test]
    fn test_llm_content_serde_roundtrip_image() {
        let content = LlmContent::Image {
            url: "data:image/jpeg;base64,/9j/4AAQ".into(),
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: LlmContent = serde_json::from_str(&json).unwrap();
        match back {
            LlmContent::Image { url } => assert_eq!(url, "data:image/jpeg;base64,/9j/4AAQ"),
            _ => panic!("expected Image after roundtrip"),
        }
    }

    // ========================================================================
    // LlmMessage
    // ========================================================================

    #[test]
    fn test_llm_message_system() {
        let msg = LlmMessage::System {
            content: "You are helpful.".into(),
        };
        match &msg {
            LlmMessage::System { content } => assert_eq!(content, "You are helpful."),
            _ => panic!("expected System variant"),
        }
    }

    #[test]
    fn test_llm_message_user_single_text() {
        let msg = LlmMessage::User {
            content: vec![LlmContent::Text("hi".into())],
        };
        match &msg {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 1);
                assert!(matches!(&content[0], LlmContent::Text(s) if s == "hi"));
            }
            _ => panic!("expected User variant"),
        }
    }

    #[test]
    fn test_llm_message_user_multimodal() {
        let msg = LlmMessage::User {
            content: vec![
                LlmContent::Text("what is this?".into()),
                LlmContent::Image {
                    url: "data:image/png;base64,abc".into(),
                },
            ],
        };
        match &msg {
            LlmMessage::User { content } => assert_eq!(content.len(), 2),
            _ => panic!("expected User variant"),
        }
    }

    #[test]
    fn test_llm_message_assistant_with_tool_calls() {
        let msg = LlmMessage::Assistant {
            content: "Let me help.".into(),
            tool_calls: vec![LlmToolCall {
                id: "call_001".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                },
            }],
        };
        match &msg {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "Let me help.");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].function.name, "bash");
            }
            _ => panic!("expected Assistant variant"),
        }
    }

    #[test]
    fn test_llm_message_assistant_no_tool_calls() {
        let msg = LlmMessage::Assistant {
            content: "Sure!".into(),
            tool_calls: vec![],
        };
        match &msg {
            LlmMessage::Assistant { tool_calls, .. } => assert!(tool_calls.is_empty()),
            _ => panic!("expected Assistant variant"),
        }
    }

    #[test]
    fn test_llm_message_tool() {
        let msg = LlmMessage::Tool {
            tool_call_id: "call_001".into(),
            content: "file contents here".into(),
        };
        match &msg {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "call_001");
                assert_eq!(content, "file contents here");
            }
            _ => panic!("expected Tool variant"),
        }
    }

    #[test]
    fn test_llm_message_serde_roundtrip() {
        let msg = LlmMessage::System {
            content: "system prompt".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: LlmMessage = serde_json::from_str(&json).unwrap();
        match back {
            LlmMessage::System { content } => assert_eq!(content, "system prompt"),
            _ => panic!("roundtrip failed"),
        }
    }

    // ========================================================================
    // LlmToolCall / LlmFunctionCall
    // ========================================================================

    #[test]
    fn test_llm_tool_call_construction() {
        let tc = LlmToolCall {
            id: "call_123".into(),
            function: LlmFunctionCall {
                name: "read_file".into(),
                arguments: r#"{"path":"/tmp/a.txt"}"#.into(),
            },
        };
        assert_eq!(tc.id, "call_123");
        assert_eq!(tc.function.name, "read_file");
        // arguments is a raw JSON string, parseable
        let parsed: serde_json::Value = serde_json::from_str(&tc.function.arguments).unwrap();
        assert_eq!(parsed["path"], "/tmp/a.txt");
    }

    #[test]
    fn test_llm_function_call_serde_roundtrip() {
        let fc = LlmFunctionCall {
            name: "bash".into(),
            arguments: r#"{"cmd":"echo hi"}"#.into(),
        };
        let json = serde_json::to_string(&fc).unwrap();
        let back: LlmFunctionCall = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "bash");
        assert_eq!(back.arguments, r#"{"cmd":"echo hi"}"#);
    }

    // ========================================================================
    // LlmContext
    // ========================================================================

    #[test]
    fn test_llm_context_construction() {
        let ctx = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hello".into())],
            }],
            system_prompt: "You are an assistant.".into(),
            max_tokens: 4096,
            temperature: Some(0.7),
        };
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(ctx.system_prompt, "You are an assistant.");
        assert_eq!(ctx.max_tokens, 4096);
        assert_eq!(ctx.temperature, Some(0.7));
    }

    #[test]
    fn test_llm_context_no_temperature() {
        let ctx = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 1024,
            temperature: None,
        };
        assert!(ctx.temperature.is_none());
    }

    // ========================================================================
    // LlmTool
    // ========================================================================

    #[test]
    fn test_llm_tool_construction() {
        let tool = LlmTool {
            name: "bash".into(),
            description: "Execute a shell command".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        };
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.parameters["type"], "object");
        assert!(tool.parameters["properties"]["command"].is_object());
    }

    #[test]
    fn test_llm_tool_serde_roundtrip() {
        let tool = LlmTool {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object"}),
        };
        let json_str = serde_json::to_string(&tool).unwrap();
        let back: LlmTool = serde_json::from_str(&json_str).unwrap();
        assert_eq!(back.name, "read");
        assert_eq!(back.description, "Read a file");
    }

    // ========================================================================
    // Model / ModelCost / ProviderCompat
    // ========================================================================

    #[test]
    fn test_model_construction() {
        let model = Model {
            id: "qwen-plus".into(),
            provider: "qwen".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 2.0,
                cache_read_per_million: 0.2,
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
        assert_eq!(model.id, "qwen-plus");
        assert_eq!(model.provider, "qwen");
        assert_eq!(model.max_tokens, 8192);
        assert_eq!(model.context_window, 131072);
    }

    #[test]
    fn test_model_cost_zero() {
        let cost = ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        };
        assert_eq!(cost.input_per_million, 0.0);
        assert_eq!(cost.cache_write_per_million, 0.0);
    }

    #[test]
    fn test_model_cost_serde_roundtrip() {
        let cost = ModelCost {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.3,
            cache_write_per_million: 3.75,
        };
        let json = serde_json::to_string(&cost).unwrap();
        let back: ModelCost = serde_json::from_str(&json).unwrap();
        assert!((back.input_per_million - 3.0).abs() < f64::EPSILON);
        assert!((back.output_per_million - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_provider_compat_with_thinking() {
        let compat = ProviderCompat {
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            supports_reasoning_effort: true,
            thinking_format: Some(ThinkingFormat::OpenAI),
            requires_tool_result_name: false,
            requires_assistant_after_tool_result: false,
            supports_strict_mode: true,
        };
        assert!(compat.supports_reasoning_effort);
        assert!(matches!(compat.thinking_format, Some(ThinkingFormat::OpenAI)));
        assert!(compat.supports_strict_mode);
    }

    #[test]
    fn test_provider_compat_qwen_thinking() {
        let compat = ProviderCompat {
            max_tokens_field: MaxTokensField::MaxTokens,
            supports_reasoning_effort: false,
            thinking_format: Some(ThinkingFormat::Qwen),
            requires_tool_result_name: false,
            requires_assistant_after_tool_result: false,
            supports_strict_mode: false,
        };
        assert!(matches!(compat.thinking_format, Some(ThinkingFormat::Qwen)));
    }

    #[test]
    fn test_provider_compat_zai_thinking() {
        let compat = ProviderCompat {
            max_tokens_field: MaxTokensField::MaxTokens,
            supports_reasoning_effort: false,
            thinking_format: Some(ThinkingFormat::Zai),
            requires_tool_result_name: true,
            requires_assistant_after_tool_result: true,
            supports_strict_mode: false,
        };
        assert!(compat.requires_tool_result_name);
        assert!(compat.requires_assistant_after_tool_result);
    }

    #[test]
    fn test_max_tokens_field_variants() {
        let a = MaxTokensField::MaxTokens;
        let b = MaxTokensField::MaxCompletionTokens;
        // They should be distinct
        assert!(!matches!(a, MaxTokensField::MaxCompletionTokens));
        assert!(!matches!(b, MaxTokensField::MaxTokens));
    }

    #[test]
    fn test_thinking_format_all_variants() {
        let formats = [ThinkingFormat::OpenAI, ThinkingFormat::Qwen, ThinkingFormat::Zai];
        assert_eq!(formats.len(), 3);
        assert!(matches!(formats[0], ThinkingFormat::OpenAI));
        assert!(matches!(formats[1], ThinkingFormat::Qwen));
        assert!(matches!(formats[2], ThinkingFormat::Zai));
    }

    // ========================================================================
    // AssistantMessageEvent
    // ========================================================================

    #[test]
    fn test_event_text_delta() {
        let event = AssistantMessageEvent::TextDelta("hello ".into());
        assert!(matches!(event, AssistantMessageEvent::TextDelta(s) if s == "hello "));
    }

    #[test]
    fn test_event_thinking_delta() {
        let event = AssistantMessageEvent::ThinkingDelta("reasoning step...".into());
        assert!(matches!(event, AssistantMessageEvent::ThinkingDelta(s) if s == "reasoning step..."));
    }

    #[test]
    fn test_event_tool_call_start() {
        let event = AssistantMessageEvent::ToolCallStart {
            id: "call_001".into(),
            name: "bash".into(),
        };
        match event {
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "call_001");
                assert_eq!(name, "bash");
            }
            _ => panic!("expected ToolCallStart"),
        }
    }

    #[test]
    fn test_event_tool_call_delta() {
        let event = AssistantMessageEvent::ToolCallDelta {
            id: "call_001".into(),
            arguments_delta: r#"{"com"#.into(),
        };
        match event {
            AssistantMessageEvent::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                assert_eq!(id, "call_001");
                assert_eq!(arguments_delta, r#"{"com"#);
            }
            _ => panic!("expected ToolCallDelta"),
        }
    }

    #[test]
    fn test_event_tool_call_end() {
        let event = AssistantMessageEvent::ToolCallEnd {
            id: "call_001".into(),
        };
        assert!(matches!(event, AssistantMessageEvent::ToolCallEnd { id } if id == "call_001"));
    }

    #[test]
    fn test_event_usage() {
        let usage = Usage {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            total_tokens: 150,
            ..Usage::default()
        };
        let event = AssistantMessageEvent::Usage(usage);
        match event {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 100);
                assert_eq!(u.output, 50);
                assert_eq!(u.total_tokens, 150);
            }
            _ => panic!("expected Usage event"),
        }
    }

    #[test]
    fn test_event_done() {
        let event = AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        };
        assert!(
            matches!(event, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::Stop)
        );
    }

    #[test]
    fn test_event_done_tool_use() {
        let event = AssistantMessageEvent::Done {
            stop_reason: StopReason::ToolUse,
        };
        assert!(
            matches!(event, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::ToolUse)
        );
    }

    #[test]
    fn test_event_error() {
        let event = AssistantMessageEvent::Error("rate limit exceeded".into());
        assert!(
            matches!(event, AssistantMessageEvent::Error(s) if s == "rate limit exceeded")
        );
    }

    #[test]
    fn test_event_serde_roundtrip_text_delta() {
        let event = AssistantMessageEvent::TextDelta("some text".into());
        let json = serde_json::to_string(&event).unwrap();
        let back: AssistantMessageEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, AssistantMessageEvent::TextDelta(s) if s == "some text"));
    }

    #[test]
    fn test_event_serde_roundtrip_done() {
        let event = AssistantMessageEvent::Done {
            stop_reason: StopReason::Length,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AssistantMessageEvent = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::Length)
        );
    }

    // ========================================================================
    // Additional coverage — LlmContent::Image empty URL
    // ========================================================================

    #[test]
    fn test_llm_content_image_empty_url() {
        let content = LlmContent::Image {
            url: String::new(),
        };
        match &content {
            LlmContent::Image { url } => assert!(url.is_empty()),
            _ => panic!("expected Image variant"),
        }
    }

    // ========================================================================
    // Additional coverage — LlmMessage serde roundtrips
    // ========================================================================

    #[test]
    fn test_llm_message_user_empty_content_serde_roundtrip() {
        let msg = LlmMessage::User {
            content: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: LlmMessage = serde_json::from_str(&json).unwrap();
        match back {
            LlmMessage::User { content } => assert!(content.is_empty()),
            _ => panic!("expected User variant after roundtrip"),
        }
    }

    #[test]
    fn test_llm_message_assistant_with_tool_calls_serde_roundtrip() {
        let msg = LlmMessage::Assistant {
            content: "Let me help.".into(),
            tool_calls: vec![LlmToolCall {
                id: "call_rt_001".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls -la"}"#.into(),
                },
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: LlmMessage = serde_json::from_str(&json).unwrap();
        match back {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "Let me help.");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "call_rt_001");
                assert_eq!(tool_calls[0].function.name, "bash");
                assert_eq!(tool_calls[0].function.arguments, r#"{"command":"ls -la"}"#);
            }
            _ => panic!("expected Assistant variant after roundtrip"),
        }
    }

    #[test]
    fn test_llm_message_tool_serde_roundtrip() {
        let msg = LlmMessage::Tool {
            tool_call_id: "call_rt_002".into(),
            content: "file contents here\nline2".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: LlmMessage = serde_json::from_str(&json).unwrap();
        match back {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "call_rt_002");
                assert_eq!(content, "file contents here\nline2");
            }
            _ => panic!("expected Tool variant after roundtrip"),
        }
    }

    // ========================================================================
    // Additional coverage — LlmToolCall serde roundtrip
    // ========================================================================

    #[test]
    fn test_llm_tool_call_serde_roundtrip() {
        let tc = LlmToolCall {
            id: "call_full_001".into(),
            function: LlmFunctionCall {
                name: "read_file".into(),
                arguments: r#"{"path":"/tmp/test.txt","encoding":"utf-8"}"#.into(),
            },
        };
        let json = serde_json::to_string(&tc).unwrap();
        let back: LlmToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "call_full_001");
        assert_eq!(back.function.name, "read_file");
        assert_eq!(
            back.function.arguments,
            r#"{"path":"/tmp/test.txt","encoding":"utf-8"}"#
        );
    }

    // ========================================================================
    // Additional coverage — AssistantMessageEvent::Error serde roundtrip
    // ========================================================================

    #[test]
    fn test_event_error_serde_roundtrip() {
        let event = AssistantMessageEvent::Error("rate limit exceeded".into());
        let json = serde_json::to_string(&event).unwrap();
        let back: AssistantMessageEvent = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, AssistantMessageEvent::Error(s) if s == "rate limit exceeded")
        );
    }
}
