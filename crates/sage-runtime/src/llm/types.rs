// LLM-side types — Phase 2
// Defines types needed for LLM API calls, enriched for multi-provider support.

use std::collections::HashMap;

use crate::types::{StopReason, Usage};
use serde::{Deserialize, Serialize};

// ============================================================================
// API type constants (matching pi-mono's KnownApi)
// ============================================================================

pub mod api {
    pub const OPENAI_COMPLETIONS: &str = "openai-completions";
    pub const OPENAI_RESPONSES: &str = "openai-responses";
    pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";
    pub const GOOGLE_GENERATIVE_AI: &str = "google-generative-ai";
    pub const AZURE_OPENAI_RESPONSES: &str = "azure-openai-responses";
    pub const BEDROCK_CONVERSE_STREAM: &str = "bedrock-converse-stream";
}

// ============================================================================
// Provider constants (matching pi-mono's KnownProvider)
// ============================================================================

pub mod provider {
    pub const ANTHROPIC: &str = "anthropic";
    pub const OPENAI: &str = "openai";
    pub const GOOGLE: &str = "google";
    pub const AZURE_OPENAI: &str = "azure-openai-responses";
    pub const XAI: &str = "xai";
    pub const GROQ: &str = "groq";
    pub const DEEPSEEK: &str = "deepseek";
    pub const QWEN: &str = "qwen";
    pub const DOUBAO: &str = "doubao";
    pub const KIMI: &str = "kimi";
    pub const MINIMAX: &str = "minimax";
    pub const ZAI: &str = "zai";
    pub const OPENROUTER: &str = "openrouter";
    pub const CEREBRAS: &str = "cerebras";
    pub const MISTRAL: &str = "mistral";
    pub const GITHUB_COPILOT: &str = "github-copilot";
    pub const AMAZON_BEDROCK: &str = "amazon-bedrock";
}

// ============================================================================
// Input / cache types
// ============================================================================

/// Supported input modalities for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputType {
    Text,
    Image,
}

/// Cache retention policy for prompt caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheRetention {
    None,
    Short,
    Long,
}

// ============================================================================
// LLM message types
// ============================================================================

/// Content block in an LLM message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmContent {
    Text(String),
    Image { url: String },
}

/// A thinking block preserved for multi-turn conversation roundtrip.
///
/// Aligns with pi-mono's `ThinkingContent` type.  Redacted blocks carry an
/// opaque encrypted signature that must be passed back to the API so the
/// provider can maintain reasoning continuity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingBlock {
    pub thinking: String,
    /// Opaque signature for redacted blocks (base64-encoded encrypted payload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub redacted: bool,
}

/// Message in the LLM API format (OpenAI-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmMessage {
    System {
        content: String,
    },
    User {
        content: Vec<LlmContent>,
    },
    Assistant {
        content: String,
        tool_calls: Vec<LlmToolCall>,
        /// Thinking blocks from reasoning models (preserved for multi-turn roundtrip).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        thinking_blocks: Vec<ThinkingBlock>,
    },
    Tool {
        tool_call_id: String,
        content: String,
        /// Tool name for providers that require it in tool results (pi-mono: toolName).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
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

// ============================================================================
// Model configuration
// ============================================================================

/// Built-in model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    pub base_url: String,
    pub api_key_env: String,
    pub reasoning: bool,
    pub input: Vec<InputType>,
    pub max_tokens: u32,
    pub context_window: u32,
    pub cost: ModelCost,
    pub headers: Vec<(String, String)>,
    pub compat: Option<ProviderCompat>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingFormat {
    /// OpenAI-native reasoning_effort parameter.
    OpenAI,
    /// Qwen `enable_thinking: bool` parameter.
    Qwen,
    /// Qwen via chat template: `chat_template_kwargs.enable_thinking`.
    QwenChatTemplate,
    /// ZAI (Zhipu) `enable_thinking: bool` parameter.
    Zai,
    /// OpenRouter `reasoning.effort` object.
    OpenRouter,
}

/// Reasoning effort level (unified across providers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReasoningLevel {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

/// Provider-specific compatibility flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCompat {
    pub max_tokens_field: MaxTokensField,
    pub supports_reasoning_effort: bool,
    pub thinking_format: Option<ThinkingFormat>,
    pub requires_tool_result_name: bool,
    pub requires_assistant_after_tool_result: bool,
    pub requires_thinking_as_text: bool,
    pub supports_strict_mode: bool,
    pub supports_store: bool,
    pub supports_developer_role: bool,
    /// Per-level reasoning effort overrides (pi-mono: reasoningEffortMap).
    ///
    /// When a key is present, its value replaces the default mapping for that
    /// reasoning level.  Used by Groq Qwen3-32b to remap all levels to "default".
    #[serde(default)]
    pub reasoning_effort_map: HashMap<ReasoningLevel, String>,
}

impl Default for ProviderCompat {
    fn default() -> Self {
        Self {
            max_tokens_field: MaxTokensField::MaxTokens,
            supports_reasoning_effort: false,
            thinking_format: None,
            requires_tool_result_name: false,
            requires_assistant_after_tool_result: false,
            requires_thinking_as_text: false,
            supports_strict_mode: false,
            supports_store: true,
            supports_developer_role: true,
            reasoning_effort_map: HashMap::new(),
        }
    }
}

// ============================================================================
// Streaming events
// ============================================================================

/// Events emitted during an assistant message stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssistantMessageEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// Marks the end of a thinking block with its accumulated signature and metadata.
    ThinkingBlockEnd {
        signature: String,
        redacted: bool,
    },
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

    #[test]
    fn test_api_constants() {
        assert_eq!(api::OPENAI_COMPLETIONS, "openai-completions");
        assert_eq!(api::ANTHROPIC_MESSAGES, "anthropic-messages");
    }

    #[test]
    fn test_input_type_serde() {
        let t = InputType::Text;
        let json = serde_json::to_string(&t).unwrap();
        let back: InputType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, InputType::Text);
    }

    #[test]
    fn test_model_with_all_fields() {
        let model = Model {
            id: "test".into(),
            name: "Test Model".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat::default()),
        };
        assert_eq!(model.api, "openai-completions");
        assert!(model.input.contains(&InputType::Image));
    }

    #[test]
    fn test_provider_compat_default() {
        let compat = ProviderCompat::default();
        assert!(matches!(compat.max_tokens_field, MaxTokensField::MaxTokens));
        assert!(!compat.supports_reasoning_effort);
    }

    #[test]
    fn test_provider_compat_default_has_empty_reasoning_effort_map() {
        let compat = ProviderCompat::default();
        assert!(compat.reasoning_effort_map.is_empty());
    }

    #[test]
    fn test_provider_compat_reasoning_effort_map_serde_roundtrip() {
        let mut map = HashMap::new();
        map.insert(ReasoningLevel::Minimal, "default".into());
        map.insert(ReasoningLevel::High, "default".into());
        let compat = ProviderCompat {
            reasoning_effort_map: map,
            ..ProviderCompat::default()
        };
        let json = serde_json::to_string(&compat).unwrap();
        let back: ProviderCompat = serde_json::from_str(&json).unwrap();
        assert_eq!(back.reasoning_effort_map.len(), 2);
        assert_eq!(
            back.reasoning_effort_map[&ReasoningLevel::Minimal],
            "default"
        );
    }

    #[test]
    fn test_reasoning_level_hash_equality() {
        let mut map = HashMap::new();
        map.insert(ReasoningLevel::Low, "low_val".to_string());
        assert_eq!(map.get(&ReasoningLevel::Low).unwrap(), "low_val");
        assert!(map.get(&ReasoningLevel::High).is_none());
    }

    #[test]
    fn test_cache_retention_variants() {
        let r = CacheRetention::Long;
        assert!(matches!(r, CacheRetention::Long));
    }
}
