// Agent core type system.
// Mirrors pi-mono packages/agent/src/types.ts and aligns with sage-runtime/src/types.rs.

use ai::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

// Re-export ai types used throughout agent-core.
pub use ai::types::{Cost, StopReason, Usage};

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
            timestamp: now_ms(),
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
            timestamp: now_ms(),
        }
    }

    pub fn from_text(text: &str) -> Self {
        Self::new(text.to_string())
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
    /// Optional structured details (mirroring pi-mono AgentToolResult.details).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub is_error: bool,
    pub timestamp: u64,
}

/// Compaction summary message — replaces older messages after context compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummaryMessage {
    /// LLM-generated summary of compacted messages.
    pub summary: String,
    /// Context tokens before compaction.
    pub tokens_before: u64,
    pub timestamp: u64,
}

/// Top-level agent message envelope.
///
/// Mirrors pi-mono's AgentMessage union type. In TypeScript this is an open
/// union; here we use a closed enum with a catch-all Custom variant for
/// application-specific extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
    CompactionSummary(CompactionSummaryMessage),
}

impl AgentMessage {
    pub fn assistant(text: String) -> Self {
        AgentMessage::Assistant(AssistantMessage::new(text))
    }
}

/// Thinking/reasoning level for models that support it.
///
/// Mirrors pi-mono's ThinkingLevel type.
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

impl fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThinkingLevel::Off => write!(f, "off"),
            ThinkingLevel::Minimal => write!(f, "minimal"),
            ThinkingLevel::Low => write!(f, "low"),
            ThinkingLevel::Medium => write!(f, "medium"),
            ThinkingLevel::High => write!(f, "high"),
            ThinkingLevel::XHigh => write!(f, "xhigh"),
        }
    }
}

/// Tool execution mode.
///
/// - `Sequential`: each tool call is executed one at a time.
/// - `Parallel`: tool calls are started concurrently; results are collected in call order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

impl Default for ToolExecutionMode {
    fn default() -> Self {
        ToolExecutionMode::Parallel
    }
}

/// Result from a tool execution.
///
/// Mirrors pi-mono's AgentToolResult<T>.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolResult {
    /// Content blocks (text and/or images).
    pub content: Vec<Content>,
    /// Structured details for UI or logging.
    pub details: Value,
}

/// Context passed to `beforeToolCall`.
///
/// Mirrors pi-mono's BeforeToolCallContext.
pub struct BeforeToolCallContext {
    /// The assistant message that requested the tool call.
    pub assistant_message: AssistantMessage,
    /// The raw tool call ID from the assistant message content.
    pub tool_call_id: String,
    pub tool_name: String,
    /// Validated tool arguments.
    pub args: Value,
    /// Current message history at the time the tool call is prepared.
    pub messages: Vec<AgentMessage>,
}

/// Result from a `beforeToolCall` hook.
///
/// Mirrors pi-mono's BeforeToolCallResult.
pub struct BeforeToolCallResult {
    /// If `true`, the tool call is blocked and an error result is emitted.
    pub block: bool,
    /// Optional reason shown in the error result when blocked.
    pub reason: Option<String>,
}

/// Context passed to `afterToolCall`.
///
/// Mirrors pi-mono's AfterToolCallContext.
pub struct AfterToolCallContext {
    /// The assistant message that requested the tool call.
    pub assistant_message: AssistantMessage,
    /// The raw tool call ID from the assistant message content.
    pub tool_call_id: String,
    pub tool_name: String,
    /// Validated tool arguments.
    pub args: Value,
    /// The executed tool result before any `afterToolCall` overrides are applied.
    pub result: AgentToolResult,
    /// Whether the executed tool result is currently treated as an error.
    pub is_error: bool,
    /// Current message history at the time the tool call is finalised.
    pub messages: Vec<AgentMessage>,
}

/// Partial override returned from `afterToolCall`.
///
/// Omitted fields keep the original executed tool result values.
///
/// Mirrors pi-mono's AfterToolCallResult.
pub struct AfterToolCallResult {
    pub content: Option<Vec<Content>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
}

/// Agent state containing all configuration and conversation data.
///
/// Mirrors pi-mono's AgentState interface.
pub struct AgentState {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<Box<dyn AgentTool>>,
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    pub stream_message: Option<AgentMessage>,
    pub pending_tool_calls: std::collections::HashSet<String>,
    pub error: Option<String>,
}

/// Context passed to the agent loop.
///
/// Mirrors pi-mono's AgentContext.
#[derive(Clone)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
}

/// Callback for streaming tool execution updates.
///
/// Mirrors pi-mono's `AgentToolUpdateCallback<T>` / the `onUpdate` parameter in
/// `AgentTool.execute`.  The `String` payload is a human-readable partial result
/// (e.g. incremental stdout from a long-running command).
pub type OnUpdateFn = Box<dyn Fn(String) + Send + Sync>;

/// Trait that all agent tools implement.
///
/// Mirrors pi-mono's AgentTool interface.
#[async_trait::async_trait]
pub trait AgentTool: Send + Sync {
    /// Tool name (used to match tool calls from the LLM).
    fn name(&self) -> &str;
    /// Human-readable label for display in UI.
    fn label(&self) -> &str;
    /// Tool description for the LLM.
    fn description(&self) -> &str;
    /// JSON Schema for the tool parameters.
    fn parameters_schema(&self) -> Value;
    /// Execute the tool call.
    ///
    /// `on_update` is an optional callback the tool may call zero or more times
    /// during execution to stream incremental output.  The agent loop converts
    /// each call into a [`crate::event::AgentEvent::ToolExecutionUpdate`] event.
    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        signal: Option<tokio_util::sync::CancellationToken>,
        on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult;
}

// Blanket impl: Arc<dyn AgentTool> also implements AgentTool.
#[async_trait::async_trait]
impl<T: ?Sized + AgentTool> AgentTool for std::sync::Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn label(&self) -> &str {
        (**self).label()
    }
    fn description(&self) -> &str {
        (**self).description()
    }
    fn parameters_schema(&self) -> Value {
        (**self).parameters_schema()
    }
    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        signal: Option<tokio_util::sync::CancellationToken>,
        on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult {
        (**self).execute(tool_call_id, args, signal, on_update).await
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ============================================================================
// StopReason Display (mirrors types.rs in sage-runtime)
// ============================================================================

// StopReason is re-exported from ai::types which has Display.

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            details: None,
            is_error: true,
            timestamp: 1700000003,
        };
        assert!(msg.is_error);
        assert_eq!(msg.tool_call_id, "tc_1");
    }

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
        assert!(msg.timestamp > 0);
    }

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
        assert_eq!(format!("{}", StopReason::Stop), "stop");
        assert_eq!(format!("{}", StopReason::Length), "length");
        assert_eq!(format!("{}", StopReason::ToolUse), "tool_use");
        assert_eq!(format!("{}", StopReason::Error), "error");
        assert_eq!(format!("{}", StopReason::Aborted), "aborted");
    }

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
    fn test_agent_message_assistant_convenience_constructor() {
        let msg = AgentMessage::assistant("test response".to_string());
        match msg {
            AgentMessage::Assistant(a) => {
                assert_eq!(a.text(), "test response");
                assert!(!a.provider.is_empty());
                assert!(matches!(a.stop_reason, StopReason::Stop));
                assert!(a.error_message.is_none());
                assert!(a.timestamp > 0);
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn test_tool_execution_mode_variants() {
        let seq = ToolExecutionMode::Sequential;
        let par = ToolExecutionMode::Parallel;
        assert!(matches!(seq, ToolExecutionMode::Sequential));
        assert!(matches!(par, ToolExecutionMode::Parallel));
    }

    #[test]
    fn test_tool_execution_mode_default_is_parallel() {
        assert_eq!(ToolExecutionMode::default(), ToolExecutionMode::Parallel);
    }

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
    fn test_before_tool_call_result_allow_execution() {
        let result = BeforeToolCallResult {
            block: false,
            reason: None,
        };
        assert!(!result.block);
        assert!(result.reason.is_none());
    }

    #[test]
    fn test_after_tool_call_result_with_content_override() {
        let result = AfterToolCallResult {
            content: Some(vec![Content::Text {
                text: "overridden output".into(),
            }]),
            details: None,
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
            details: None,
            is_error: None,
        };
        assert!(result.content.is_none());
        assert!(result.is_error.is_none());
    }

    #[test]
    fn test_agent_tool_result_construction() {
        let result = AgentToolResult {
            content: vec![Content::Text {
                text: "output".into(),
            }],
            details: json!({"key": "value"}),
        };
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.details["key"], "value");
    }

    #[test]
    fn test_content_text_empty_string() {
        let content = Content::Text {
            text: String::new(),
        };
        match &content {
            Content::Text { text } => assert!(text.is_empty()),
            _ => panic!("wrong variant"),
        }
        let json = serde_json::to_string(&content).unwrap();
        let rt: Content = serde_json::from_str(&json).unwrap();
        match rt {
            Content::Text { text } => assert!(text.is_empty()),
            _ => panic!("wrong variant"),
        }
    }

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

    #[test]
    fn test_tool_result_message_success_path() {
        let msg = ToolResultMessage {
            tool_call_id: "tc_ok".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "file1.txt\nfile2.txt\n".into(),
            }],
            details: None,
            is_error: false,
            timestamp: 1700000010,
        };
        assert!(!msg.is_error);
        assert_eq!(msg.tool_name, "bash");
        assert_eq!(msg.tool_call_id, "tc_ok");
        assert_eq!(msg.content.len(), 1);
    }

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
        assert!(msg.tool_calls().is_empty());
    }
}
