// Agent Runtime type system — Phase 1

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Content block within a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Content {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
        redacted: bool,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    Image {
        data: String,
        mime_type: String,
    },
}

/// User message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: Vec<Content>,
    pub timestamp: u64,
}

impl UserMessage {
    pub fn from_text(text: &str) -> Self {
        Self {
            content: vec![Content::Text {
                text: text.to_string(),
            }],
            timestamp: now_secs(),
        }
    }
}

/// Assistant message from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Vec<Content>,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    pub stop_reason: StopReason,
    pub error_message: Option<String>,
    pub timestamp: u64,
}

impl AssistantMessage {
    pub fn new(text: String) -> Self {
        Self {
            content: vec![Content::Text { text }],
            provider: "unknown".into(),
            model: "unknown".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: now_secs(),
        }
    }

    pub fn tool_calls(&self) -> Vec<&Content> {
        self.content
            .iter()
            .filter(|c| matches!(c, Content::ToolCall { .. }))
            .collect()
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Tool execution result message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<Content>,
    pub is_error: bool,
    pub timestamp: u64,
}

/// Top-level agent message envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

impl AgentMessage {
    pub fn assistant(text: String) -> Self {
        AgentMessage::Assistant(AssistantMessage::new(text))
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: Cost,
}

/// Cost breakdown in dollars.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// Reason the LLM stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StopReason::Stop => write!(f, "stop"),
            StopReason::Length => write!(f, "length"),
            StopReason::ToolUse => write!(f, "tool_use"),
            StopReason::Error => write!(f, "error"),
            StopReason::Aborted => write!(f, "aborted"),
        }
    }
}

/// Thinking/reasoning effort level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

/// Tool execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

/// Mutable agent runtime state.
pub struct AgentState {
    pub system_prompt: String,
    pub model: String,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<String>,
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    pub pending_tool_calls: HashSet<String>,
    pub error: Option<String>,
}

/// Immutable context for an LLM call.
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
}

/// Context passed to before-tool-call hooks.
pub struct BeforeToolCallContext {
    pub tool_name: String,
    pub tool_call_id: String,
    pub args: Value,
}

/// Result from a before-tool-call hook.
pub struct BeforeToolCallResult {
    pub block: bool,
    pub reason: Option<String>,
}

/// Context passed to after-tool-call hooks.
pub struct AfterToolCallContext {
    pub tool_name: String,
    pub tool_call_id: String,
    pub args: Value,
    pub is_error: bool,
}

/// Result from an after-tool-call hook.
pub struct AfterToolCallResult {
    pub content: Option<Vec<Content>>,
    pub is_error: Option<bool>,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    // ========================================================================
    // Message construction (happy path)
    // ========================================================================

    #[test]
    fn test_user_message_with_text_content() {
        let msg = UserMessage {
            content: vec![Content::Text {
                text: "hello world".into(),
            }],
            timestamp: 1700000000,
        };
        assert_eq!(msg.content.len(), 1);
        assert_eq!(msg.timestamp, 1700000000);
        assert!(matches!(&msg.content[0], Content::Text { text } if text == "hello world"));
    }

    #[test]
    fn test_user_message_with_mixed_text_and_image() {
        let msg = UserMessage {
            content: vec![
                Content::Text {
                    text: "look at this".into(),
                },
                Content::Image {
                    data: "iVBORw0KGgo=".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 1700000001,
        };
        assert_eq!(msg.content.len(), 2);
        assert!(matches!(&msg.content[0], Content::Text { .. }));
        assert!(matches!(
            &msg.content[1],
            Content::Image { mime_type, .. } if mime_type == "image/png"
        ));
    }

    #[test]
    fn test_assistant_message_with_text_thinking_and_tool_call() {
        let msg = AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: "let me consider...".into(),
                    signature: Some("sig123".into()),
                    redacted: false,
                },
                Content::Text {
                    text: "I'll help you.".into(),
                },
                Content::ToolCall {
                    id: "tc_1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "/tmp/test.txt"}),
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-4-20250514".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 1700000002,
        };
        assert_eq!(msg.content.len(), 3);
        assert_eq!(msg.provider, "anthropic");
        assert_eq!(msg.model, "claude-opus-4-20250514");
        assert!(matches!(msg.stop_reason, StopReason::ToolUse));
    }

    #[test]
    fn test_tool_result_message_with_error_flag() {
        let msg = ToolResultMessage {
            tool_call_id: "tc_1".into(),
            tool_name: "read_file".into(),
            content: vec![Content::Text {
                text: "Permission denied".into(),
            }],
            is_error: true,
            timestamp: 1700000003,
        };
        assert!(msg.is_error);
        assert_eq!(msg.tool_call_id, "tc_1");
        assert_eq!(msg.tool_name, "read_file");
    }

    // ========================================================================
    // Content helpers
    // ========================================================================

    #[test]
    fn test_assistant_message_tool_calls_extracts_only_tool_call_items() {
        let msg = AssistantMessage {
            content: vec![
                Content::Text {
                    text: "doing stuff".into(),
                },
                Content::ToolCall {
                    id: "tc_1".into(),
                    name: "bash".into(),
                    arguments: json!({"cmd": "ls"}),
                },
                Content::ToolCall {
                    id: "tc_2".into(),
                    name: "read".into(),
                    arguments: json!({"path": "/a"}),
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-4-20250514".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        };
        let calls = msg.tool_calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|c| matches!(c, Content::ToolCall { .. })));
    }

    #[test]
    fn test_assistant_message_text_concatenates_all_text_content() {
        let msg = AssistantMessage {
            content: vec![
                Content::Text {
                    text: "Hello ".into(),
                },
                Content::Thinking {
                    thinking: "hmm".into(),
                    signature: None,
                    redacted: false,
                },
                Content::Text {
                    text: "world".into(),
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-4-20250514".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert_eq!(msg.text(), "Hello world");
    }

    #[test]
    fn test_user_message_from_text_creates_single_text_content() {
        let msg = UserMessage::from_text("hello");
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(&msg.content[0], Content::Text { text } if text == "hello"));
        // timestamp should be set (non-zero or at least defined)
        assert!(msg.timestamp > 0);
    }

    // ========================================================================
    // Usage + Cost
    // ========================================================================

    #[test]
    fn test_usage_default_is_all_zeros() {
        let usage = Usage::default();
        assert_eq!(usage.input, 0);
        assert_eq!(usage.output, 0);
        assert_eq!(usage.cache_read, 0);
        assert_eq!(usage.cache_write, 0);
        assert_eq!(usage.total_tokens, 0);
        assert_eq!(usage.cost.total, 0.0);
    }

    #[test]
    fn test_cost_total_equals_sum_of_components() {
        let cost = Cost {
            input: 0.015,
            output: 0.075,
            cache_read: 0.00375,
            cache_write: 0.01875,
            total: 0.1125,
        };
        let sum = cost.input + cost.output + cost.cache_read + cost.cache_write;
        assert!((cost.total - sum).abs() < 1e-10);
    }

    #[test]
    fn test_cost_default_is_all_zeros() {
        let cost = Cost::default();
        assert_eq!(cost.input, 0.0);
        assert_eq!(cost.output, 0.0);
        assert_eq!(cost.cache_read, 0.0);
        assert_eq!(cost.cache_write, 0.0);
        assert_eq!(cost.total, 0.0);
    }

    // ========================================================================
    // StopReason
    // ========================================================================

    #[test]
    fn test_stop_reason_serde_roundtrip() {
        let reasons = vec![
            StopReason::Stop,
            StopReason::Length,
            StopReason::ToolUse,
            StopReason::Error,
            StopReason::Aborted,
        ];
        for reason in &reasons {
            let json = serde_json::to_string(reason).expect("serialize StopReason");
            let back: StopReason = serde_json::from_str(&json).expect("deserialize StopReason");
            assert_eq!(&back, reason);
        }
    }

    #[test]
    fn test_stop_reason_display() {
        assert!(!format!("{}", StopReason::Stop).is_empty());
        assert!(!format!("{}", StopReason::ToolUse).is_empty());
    }

    // ========================================================================
    // ThinkingLevel
    // ========================================================================

    #[test]
    fn test_thinking_level_all_six_variants_exist() {
        let _levels = [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
        ];
    }

    #[test]
    fn test_thinking_level_ordering() {
        assert!(ThinkingLevel::Off < ThinkingLevel::Minimal);
        assert!(ThinkingLevel::Minimal < ThinkingLevel::Low);
        assert!(ThinkingLevel::Low < ThinkingLevel::Medium);
        assert!(ThinkingLevel::Medium < ThinkingLevel::High);
        assert!(ThinkingLevel::High < ThinkingLevel::XHigh);
    }

    // ========================================================================
    // AgentState
    // ========================================================================

    #[test]
    fn test_agent_state_default_construction() {
        let state = AgentState {
            system_prompt: "You are a helpful assistant.".into(),
            model: "claude-opus-4-20250514".into(),
            thinking_level: ThinkingLevel::Medium,
            tools: vec!["bash".into(), "read".into()],
            messages: vec![],
            is_streaming: false,
            pending_tool_calls: HashSet::new(),
            error: None,
        };
        assert_eq!(state.model, "claude-opus-4-20250514");
        assert!(!state.is_streaming);
        assert!(state.pending_tool_calls.is_empty());
        assert!(state.error.is_none());
        assert_eq!(state.tools.len(), 2);
    }

    #[test]
    fn test_agent_state_is_streaming_flag() {
        let state = AgentState {
            system_prompt: String::new(),
            model: "test-model".into(),
            thinking_level: ThinkingLevel::Off,
            tools: vec![],
            messages: vec![],
            is_streaming: true,
            pending_tool_calls: HashSet::from(["tc_1".into(), "tc_2".into()]),
            error: None,
        };
        assert!(state.is_streaming);
        assert_eq!(state.pending_tool_calls.len(), 2);
    }

    // ========================================================================
    // AgentContext
    // ========================================================================

    #[test]
    fn test_agent_context_construction() {
        let ctx = AgentContext {
            system_prompt: "system".into(),
            messages: vec![AgentMessage::User(UserMessage::from_text("hi"))],
        };
        assert_eq!(ctx.system_prompt, "system");
        assert_eq!(ctx.messages.len(), 1);
    }

    // ========================================================================
    // Serde roundtrip
    // ========================================================================

    #[test]
    fn test_agent_message_user_serde_roundtrip() {
        let msg = AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "hello".into(),
            }],
            timestamp: 12345,
        });
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: AgentMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(back, AgentMessage::User(_)));
    }

    #[test]
    fn test_agent_message_assistant_with_tool_calls_roundtrip() {
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Text {
                    text: "I will run a command.".into(),
                },
                Content::ToolCall {
                    id: "tc_99".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "echo hi", "timeout": 5000}),
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-4-20250514".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 99999,
        });
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: AgentMessage = serde_json::from_str(&json).expect("deserialize");
        match back {
            AgentMessage::Assistant(a) => {
                assert_eq!(a.content.len(), 2);
                assert!(matches!(a.stop_reason, StopReason::ToolUse));
            }
            _ => panic!("expected Assistant variant"),
        }
    }

    #[test]
    fn test_tool_result_message_serde_roundtrip() {
        let msg = AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "file.txt\n".into(),
            }],
            is_error: false,
            timestamp: 10,
        });
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: AgentMessage = serde_json::from_str(&json).expect("deserialize");
        match back {
            AgentMessage::ToolResult(tr) => {
                assert!(!tr.is_error);
                assert_eq!(tr.tool_call_id, "tc_1");
            }
            _ => panic!("expected ToolResult variant"),
        }
    }

    // ========================================================================
    // Edge cases
    // ========================================================================

    #[test]
    fn test_empty_content_vectors() {
        let msg = UserMessage {
            content: vec![],
            timestamp: 0,
        };
        assert!(msg.content.is_empty());

        let assistant = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert_eq!(assistant.tool_calls().len(), 0);
        assert_eq!(assistant.text(), "");
    }

    #[test]
    fn test_very_long_text_content() {
        let long_text = "x".repeat(50 * 1024); // 50KB
        let msg = UserMessage {
            content: vec![Content::Text {
                text: long_text.clone(),
            }],
            timestamp: 0,
        };
        match &msg.content[0] {
            Content::Text { text } => assert_eq!(text.len(), 50 * 1024),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_unicode_content_cjk_and_emoji() {
        let msg = UserMessage {
            content: vec![Content::Text {
                text: "你好世界 🌍🚀 こんにちは 한국어".into(),
            }],
            timestamp: 0,
        };
        let json = serde_json::to_string(&AgentMessage::User(msg)).expect("serialize unicode");
        let back: AgentMessage = serde_json::from_str(&json).expect("deserialize unicode");
        match back {
            AgentMessage::User(u) => match &u.content[0] {
                Content::Text { text } => {
                    assert!(text.contains("你好世界"));
                    assert!(text.contains("🌍"));
                    assert!(text.contains("こんにちは"));
                }
                _ => panic!("expected Text"),
            },
            _ => panic!("expected User"),
        }
    }

    #[test]
    fn test_tool_call_with_nested_json_arguments() {
        let nested_args = json!({
            "config": {
                "env": {
                    "KEY": "value",
                    "NESTED": {"a": [1, 2, 3]}
                },
                "flags": ["--verbose", "--dry-run"]
            },
            "count": 42
        });
        let content = Content::ToolCall {
            id: "tc_nested".into(),
            name: "complex_tool".into(),
            arguments: nested_args.clone(),
        };
        match &content {
            Content::ToolCall { arguments, .. } => {
                assert_eq!(arguments["config"]["env"]["NESTED"]["a"][1], 2);
                assert_eq!(arguments["count"], 42);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    // ========================================================================
    // Hook types
    // ========================================================================

    #[test]
    fn test_before_tool_call_result_block_true() {
        let result = BeforeToolCallResult {
            block: true,
            reason: Some("Tool not allowed in sandbox".into()),
        };
        assert!(result.block);
        assert_eq!(
            result.reason.as_deref(),
            Some("Tool not allowed in sandbox")
        );
    }

    #[test]
    fn test_before_tool_call_context_construction() {
        let ctx = BeforeToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc_5".into(),
            args: json!({"command": "rm -rf /"}),
        };
        assert_eq!(ctx.tool_name, "bash");
        assert_eq!(ctx.tool_call_id, "tc_5");
    }

    #[test]
    fn test_after_tool_call_result_with_content_override() {
        let result = AfterToolCallResult {
            content: Some(vec![Content::Text {
                text: "overridden output".into(),
            }]),
            is_error: Some(true),
        };
        assert!(result.content.is_some());
        assert_eq!(result.content.as_ref().unwrap().len(), 1);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn test_after_tool_call_result_with_none_keeps_original() {
        let result = AfterToolCallResult {
            content: None,
            is_error: None,
        };
        assert!(result.content.is_none());
        assert!(result.is_error.is_none());
    }

    #[test]
    fn test_after_tool_call_context_construction() {
        let ctx = AfterToolCallContext {
            tool_name: "read".into(),
            tool_call_id: "tc_7".into(),
            args: json!({"file": "test.rs"}),
            is_error: false,
        };
        assert_eq!(ctx.tool_name, "read");
        assert!(!ctx.is_error);
    }

    // ========================================================================
    // ToolExecutionMode
    // ========================================================================

    #[test]
    fn test_tool_execution_mode_variants() {
        let seq = ToolExecutionMode::Sequential;
        let par = ToolExecutionMode::Parallel;
        assert!(matches!(seq, ToolExecutionMode::Sequential));
        assert!(matches!(par, ToolExecutionMode::Parallel));
    }

    // ========================================================================
    // Thinking content edge case
    // ========================================================================

    #[test]
    fn test_thinking_content_redacted_flag() {
        let content = Content::Thinking {
            thinking: String::new(),
            signature: None,
            redacted: true,
        };
        match &content {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => {
                assert!(thinking.is_empty());
                assert!(signature.is_none());
                assert!(*redacted);
            }
            _ => panic!("expected Thinking"),
        }
    }

    // ========================================================================
    // Serde deserialization of invalid data
    // ========================================================================

    #[test]
    fn test_content_deserialize_unknown_variant_returns_error() {
        let json = r#"{"type":"Unknown","data":"???"}"#;
        let result = serde_json::from_str::<Content>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_message_deserialize_missing_required_fields() {
        // Missing 'content' field
        let json = r#"{"User":{"timestamp":123}}"#;
        let result = serde_json::from_str::<AgentMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_stop_reason_deserialize_invalid_value() {
        let json = r#""invalid_reason""#;
        let result = serde_json::from_str::<StopReason>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_user_message_deserialize_wrong_timestamp_type() {
        let json = r#"{"content":[],"timestamp":"not_a_number"}"#;
        let result = serde_json::from_str::<UserMessage>(json);
        assert!(result.is_err());
    }

    // ========================================================================
    // AssistantMessage.text() edge cases
    // ========================================================================

    #[test]
    fn test_assistant_message_text_with_no_text_content_returns_empty() {
        let msg = AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: "hmm".into(),
                    signature: None,
                    redacted: false,
                },
                Content::ToolCall {
                    id: "tc_1".into(),
                    name: "bash".into(),
                    arguments: json!({"cmd": "ls"}),
                },
            ],
            provider: "test".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        };
        assert_eq!(msg.text(), "");
    }

    #[test]
    fn test_assistant_message_tool_calls_empty_when_no_tool_calls() {
        let msg = AssistantMessage {
            content: vec![Content::Text {
                text: "just text".into(),
            }],
            provider: "test".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        let calls = msg.tool_calls();
        assert!(calls.is_empty());
    }

    // ========================================================================
    // Content::Image serde roundtrip
    // ========================================================================

    #[test]
    fn test_content_image_serde_roundtrip() {
        // base64 data with special chars +, /, =
        let content = Content::Image {
            data: "iVBORw0KGgo+/A==".into(),
            mime_type: "image/png".into(),
        };
        let json = serde_json::to_string(&content).unwrap();
        let deserialized: Content = serde_json::from_str(&json).unwrap();
        match deserialized {
            Content::Image { data, mime_type } => {
                assert_eq!(data, "iVBORw0KGgo+/A==");
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("expected Image variant"),
        }
    }

    // ========================================================================
    // ToolExecutionMode serde roundtrip
    // ========================================================================

    #[test]
    fn test_tool_execution_mode_serde_roundtrip() {
        for mode in [ToolExecutionMode::Sequential, ToolExecutionMode::Parallel] {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: ToolExecutionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    // ========================================================================
    // Usage total_tokens consistency
    // ========================================================================

    #[test]
    fn test_usage_total_tokens_equals_sum_of_components() {
        let usage = Usage {
            input: 1000,
            output: 500,
            cache_read: 200,
            cache_write: 100,
            total_tokens: 1800,
            cost: Cost::default(),
        };
        assert_eq!(usage.total_tokens, usage.input + usage.output + usage.cache_read + usage.cache_write);
    }

    // ========================================================================
    // AgentState with error and messages
    // ========================================================================

    #[test]
    fn test_agent_state_with_error_and_messages() {
        let msg = AgentMessage::User(UserMessage::from_text("hello"));
        let state = AgentState {
            system_prompt: "You are helpful".into(),
            model: "qwen-plus".into(),
            thinking_level: ThinkingLevel::Medium,
            tools: vec!["bash".into(), "read".into()],
            messages: vec![msg],
            is_streaming: false,
            pending_tool_calls: HashSet::new(),
            error: Some("timeout after 300s".into()),
        };
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.error.as_deref(), Some("timeout after 300s"));
        assert_eq!(state.tools.len(), 2);
    }

    // ========================================================================
    // BeforeToolCallResult block=false with no reason
    // ========================================================================

    #[test]
    fn test_before_tool_call_result_allow_execution() {
        let result = BeforeToolCallResult {
            block: false,
            reason: None,
        };
        assert!(!result.block);
        assert!(result.reason.is_none());
    }

    // ========================================================================
    // StopReason Display specific values
    // ========================================================================

    #[test]
    fn test_stop_reason_display_specific_values() {
        assert_eq!(format!("{}", StopReason::Stop), "stop");
        assert_eq!(format!("{}", StopReason::Length), "length");
        assert_eq!(format!("{}", StopReason::ToolUse), "tool_use");
        assert_eq!(format!("{}", StopReason::Error), "error");
        assert_eq!(format!("{}", StopReason::Aborted), "aborted");
    }

    // ========================================================================
    // ThinkingLevel serde roundtrip
    // ========================================================================

    #[test]
    fn test_thinking_level_serde_roundtrip_all_variants() {
        let levels = [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
        ];
        for level in &levels {
            let json = serde_json::to_string(level).unwrap();
            let deserialized: ThinkingLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(*level, deserialized);
        }
    }

    // ========================================================================
    // Content with empty text
    // ========================================================================

    #[test]
    fn test_content_text_empty_string() {
        let content = Content::Text { text: String::new() };
        match &content {
            Content::Text { text } => assert!(text.is_empty()),
            _ => panic!("wrong variant"),
        }
        // serde roundtrip with empty text
        let json = serde_json::to_string(&content).unwrap();
        let rt: Content = serde_json::from_str(&json).unwrap();
        match rt {
            Content::Text { text } => assert!(text.is_empty()),
            _ => panic!("wrong variant"),
        }
    }

    // ========================================================================
    // Thinking content with redacted=true
    // ========================================================================

    #[test]
    fn test_thinking_content_redacted_serde_roundtrip() {
        let content = Content::Thinking {
            thinking: String::new(), // redacted content is empty
            signature: Some("encrypted-sig".into()),
            redacted: true,
        };
        let json = serde_json::to_string(&content).unwrap();
        let rt: Content = serde_json::from_str(&json).unwrap();
        match rt {
            Content::Thinking { thinking, signature, redacted } => {
                assert!(thinking.is_empty());
                assert_eq!(signature.as_deref(), Some("encrypted-sig"));
                assert!(redacted);
            }
            _ => panic!("wrong variant"),
        }
    }

    // ========================================================================
    // Timestamp boundary values
    // ========================================================================

    #[test]
    fn test_user_message_timestamp_zero() {
        let msg = UserMessage {
            content: vec![Content::Text { text: "hello".into() }],
            timestamp: 0,
        };
        let json = serde_json::to_string(&AgentMessage::User(msg)).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        match back {
            AgentMessage::User(u) => assert_eq!(u.timestamp, 0),
            _ => panic!("expected User"),
        }
    }

    #[test]
    fn test_user_message_timestamp_u64_max() {
        let msg = UserMessage {
            content: vec![],
            timestamp: u64::MAX,
        };
        let json = serde_json::to_string(&AgentMessage::User(msg)).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        match back {
            AgentMessage::User(u) => assert_eq!(u.timestamp, u64::MAX),
            _ => panic!("expected User"),
        }
    }

    // ========================================================================
    // Empty tool_name / tool_call_id
    // ========================================================================

    #[test]
    fn test_tool_call_with_empty_id_and_name() {
        let content = Content::ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: json!({}),
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: Content = serde_json::from_str(&json).unwrap();
        match back {
            Content::ToolCall { id, name, arguments } => {
                assert!(id.is_empty());
                assert!(name.is_empty());
                assert_eq!(arguments, json!({}));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_tool_result_message_with_empty_ids() {
        let msg = ToolResultMessage {
            tool_call_id: String::new(),
            tool_name: String::new(),
            content: vec![],
            is_error: false,
            timestamp: 0,
        };
        let json = serde_json::to_string(&AgentMessage::ToolResult(msg)).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        match back {
            AgentMessage::ToolResult(tr) => {
                assert!(tr.tool_call_id.is_empty());
                assert!(tr.tool_name.is_empty());
                assert!(tr.content.is_empty());
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // ========================================================================
    // Cost/Usage extreme values
    // ========================================================================

    #[test]
    fn test_cost_infinity_does_not_roundtrip() {
        let cost = Cost {
            input: f64::INFINITY,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
            total: f64::INFINITY,
        };
        // serde_json serializes non-finite f64 as null
        let json = serde_json::to_string(&cost).unwrap();
        // null cannot be deserialized back to f64 — roundtrip fails
        let result = serde_json::from_str::<Cost>(&json);
        assert!(result.is_err(), "non-finite f64 should not roundtrip through JSON");
    }

    #[test]
    fn test_cost_nan_does_not_roundtrip() {
        let cost = Cost {
            input: f64::NAN,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
            total: f64::NAN,
        };
        // serde_json serializes NaN as null
        let json = serde_json::to_string(&cost).unwrap();
        // null cannot be deserialized back to f64 — roundtrip fails
        let result = serde_json::from_str::<Cost>(&json);
        assert!(result.is_err(), "NaN should not roundtrip through JSON");
    }

    #[test]
    fn test_usage_u64_max_serde_roundtrip() {
        let usage = Usage {
            input: u64::MAX,
            output: u64::MAX,
            cache_read: 0,
            cache_write: 0,
            total_tokens: u64::MAX,
            cost: Cost::default(),
        };
        let json = serde_json::to_string(&usage).unwrap();
        let back: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.input, u64::MAX);
        assert_eq!(back.output, u64::MAX);
        assert_eq!(back.total_tokens, u64::MAX);
    }

    // ========================================================================
    // Content::Image edge cases
    // ========================================================================

    #[test]
    fn test_content_image_empty_data_and_invalid_mime() {
        let content = Content::Image {
            data: String::new(),
            mime_type: "not/a/valid/mime".into(),
        };
        // Should passthrough — no validation at type level
        let json = serde_json::to_string(&content).unwrap();
        let back: Content = serde_json::from_str(&json).unwrap();
        match back {
            Content::Image { data, mime_type } => {
                assert!(data.is_empty());
                assert_eq!(mime_type, "not/a/valid/mime");
            }
            _ => panic!("expected Image"),
        }
    }

    // ========================================================================
    // ToolCall arguments edge cases
    // ========================================================================

    #[test]
    fn test_tool_call_arguments_null() {
        let content = Content::ToolCall {
            id: "tc_null".into(),
            name: "test".into(),
            arguments: json!(null),
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: Content = serde_json::from_str(&json).unwrap();
        match back {
            Content::ToolCall { arguments, .. } => {
                assert!(arguments.is_null());
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_tool_call_arguments_string_type() {
        let content = Content::ToolCall {
            id: "tc_str".into(),
            name: "test".into(),
            arguments: json!("a string argument"),
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: Content = serde_json::from_str(&json).unwrap();
        match back {
            Content::ToolCall { arguments, .. } => {
                assert_eq!(arguments, json!("a string argument"));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_tool_call_arguments_empty_object() {
        let content = Content::ToolCall {
            id: "tc_empty".into(),
            name: "test".into(),
            arguments: json!({}),
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: Content = serde_json::from_str(&json).unwrap();
        match back {
            Content::ToolCall { arguments, .. } => {
                assert!(arguments.as_object().unwrap().is_empty());
            }
            _ => panic!("expected ToolCall"),
        }
    }

    // ========================================================================
    // AgentMessage::assistant() convenience constructor
    // ========================================================================

    #[test]
    fn test_agent_message_assistant_convenience_constructor() {
        let msg = AgentMessage::assistant("test response".to_string());
        match msg {
            AgentMessage::Assistant(a) => {
                assert_eq!(a.text(), "test response");
                // Should have sensible defaults
                assert!(!a.provider.is_empty());
                assert!(!a.model.is_empty());
                assert!(matches!(a.stop_reason, StopReason::Stop));
                assert!(a.error_message.is_none());
                assert!(a.timestamp > 0);
            }
            _ => panic!("expected Assistant"),
        }
    }

    // ========================================================================
    // from_text() edge cases
    // ========================================================================

    #[test]
    fn test_user_message_from_text_empty_string() {
        let msg = UserMessage::from_text("");
        assert_eq!(msg.content.len(), 1);
        match &msg.content[0] {
            Content::Text { text } => assert!(text.is_empty()),
            _ => panic!("expected Text"),
        }
    }

    // ========================================================================
    // text() concatenation semantics
    // ========================================================================

    #[test]
    fn test_assistant_message_text_concatenation_no_separator() {
        let msg = AssistantMessage {
            content: vec![
                Content::Text { text: "Hello".into() },
                Content::Text { text: "World".into() },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        // Documents behavior: text() concatenates without separator
        assert_eq!(msg.text(), "HelloWorld");
    }

    // ========================================================================
    // ToolResultMessage is_error=false normal path
    // ========================================================================

    #[test]
    fn test_tool_result_message_success_path() {
        let msg = ToolResultMessage {
            tool_call_id: "tc_ok".into(),
            tool_name: "bash".into(),
            content: vec![
                Content::Text { text: "file1.txt\nfile2.txt\n".into() },
            ],
            is_error: false,
            timestamp: 1700000010,
        };
        assert!(!msg.is_error);
        assert_eq!(msg.tool_name, "bash");
        assert_eq!(msg.tool_call_id, "tc_ok");
        assert_eq!(msg.content.len(), 1);
    }
}
