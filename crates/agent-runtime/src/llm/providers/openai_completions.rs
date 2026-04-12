// OpenAI Chat Completions Provider — implements ApiProvider trait.
// Ported from pi-mono's openai-completions.ts with full compat handling.
//
// Covers: OpenAI, DeepSeek, Qwen, Doubao, Kimi, MiniMax, ZAI (Zhipu),
// xAI (Grok), Groq, OpenRouter, and any OpenAI-compatible endpoint.

use crate::llm::keys;
use crate::llm::registry::{ApiProvider, StreamOptions};
use crate::llm::stream::parse_sse_chunk;
use crate::llm::types::*;
use reqwest::Client;
use serde_json::{Value, json};

/// Provider for OpenAI-compatible chat completions APIs.
///
/// Registered under the `"openai-completions"` API identifier. Supports any
/// endpoint that speaks the OpenAI `/chat/completions` SSE protocol, including
/// third-party providers (DeepSeek, Qwen, etc.) via `model.compat` flags.
pub struct OpenAiCompletionsProvider {
    client: Client,
}

impl OpenAiCompletionsProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Build the JSON request body for the chat completions API.
    ///
    /// Respects `model.compat` for:
    /// - max-tokens field naming (max_tokens vs max_completion_tokens)
    /// - developer role for reasoning models (supports_developer_role)
    /// - thinkingFormat branches (Qwen/Zai/OpenRouter/OpenAI)
    /// - requiresAssistantAfterToolResult synthetic messages
    /// - store field suppression for non-standard providers
    /// - temperature disable for reasoning models
    ///
    /// Ported from pi-mono openai-completions.ts buildParams().
    fn build_request_body(
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Value {
        let compat = detect_compat(model);

        let mut messages = Vec::new();

        // System prompt — use "developer" role for reasoning models when supported.
        if !context.system_prompt.is_empty() {
            let role = if model.reasoning && compat.supports_developer_role {
                "developer"
            } else {
                "system"
            };
            messages.push(json!({
                "role": role,
                "content": context.system_prompt,
            }));
        }

        // Track last role for requiresAssistantAfterToolResult insertion.
        let mut last_role = "";

        // Conversation messages
        for msg in &context.messages {
            match msg {
                LlmMessage::System { content } => {
                    let role = if model.reasoning && compat.supports_developer_role {
                        "developer"
                    } else {
                        "system"
                    };
                    messages.push(json!({
                        "role": role,
                        "content": content,
                    }));
                    last_role = "system";
                }
                LlmMessage::User { content } => {
                    // Insert synthetic assistant if needed (pi-mono: requiresAssistantAfterToolResult)
                    if compat.requires_assistant_after_tool_result && last_role == "tool" {
                        messages.push(json!({
                            "role": "assistant",
                            "content": "I have processed the tool results.",
                        }));
                    }

                    let parts: Vec<Value> = content
                        .iter()
                        .map(|c| match c {
                            LlmContent::Text(text) => json!({
                                "type": "text",
                                "text": text,
                            }),
                            LlmContent::Image { url } => json!({
                                "type": "image_url",
                                "image_url": { "url": url },
                            }),
                        })
                        .collect();
                    messages.push(json!({
                        "role": "user",
                        "content": parts,
                    }));
                    last_role = "user";
                }
                LlmMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut msg = json!({
                        "role": "assistant",
                        "content": content,
                    });
                    if !tool_calls.is_empty() {
                        let tcs: Vec<Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                let id = normalize_tool_call_id(&tc.id, model);
                                json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.function.name,
                                        "arguments": tc.function.arguments,
                                    },
                                })
                            })
                            .collect();
                        msg["tool_calls"] = json!(tcs);
                    }
                    messages.push(msg);
                    last_role = "assistant";
                }
                LlmMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    let id = normalize_tool_call_id(tool_call_id, model);
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": content,
                    }));
                    last_role = "tool";
                }
            }
        }

        let max_tokens_key = match compat.max_tokens_field {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        };

        let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);

        let mut body = json!({
            "model": model.id,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        body[max_tokens_key] = json!(max_tokens);

        // Temperature: options override, then context, then omit.
        let temperature = options.temperature.or(context.temperature);
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }

        // --- Thinking format handling (pi-mono: thinkingFormat branches) ---
        let has_reasoning_effort = options.reasoning.is_some();
        if let Some(ref fmt) = compat.thinking_format {
            match fmt {
                ThinkingFormat::Zai if model.reasoning => {
                    body["enable_thinking"] = json!(has_reasoning_effort);
                }
                ThinkingFormat::Qwen if model.reasoning => {
                    body["enable_thinking"] = json!(has_reasoning_effort);
                }
                ThinkingFormat::QwenChatTemplate if model.reasoning => {
                    body["chat_template_kwargs"] =
                        json!({ "enable_thinking": has_reasoning_effort });
                }
                ThinkingFormat::OpenRouter if model.reasoning => {
                    if has_reasoning_effort {
                        let effort_str =
                            map_reasoning_effort(options.reasoning.unwrap_or(ReasoningLevel::High));
                        body["reasoning"] = json!({ "effort": effort_str });
                    } else {
                        body["reasoning"] = json!({ "effort": "none" });
                    }
                }
                ThinkingFormat::OpenAI
                    if has_reasoning_effort
                        && model.reasoning
                        && compat.supports_reasoning_effort =>
                {
                    let effort_str =
                        map_reasoning_effort(options.reasoning.unwrap_or(ReasoningLevel::High));
                    body["reasoning_effort"] = json!(effort_str);
                }
                _ => {}
            }
        }

        // Store field (pi-mono: supportsStore)
        if compat.supports_store {
            body["store"] = json!(false);
        }

        // Tools
        if !tools.is_empty() {
            let tool_defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let mut def = json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        },
                    });
                    if compat.supports_strict_mode {
                        def["function"]["strict"] = json!(true);
                    }
                    def
                })
                .collect();
            body["tools"] = json!(tool_defs);
        }

        body
    }
}

// ---------------------------------------------------------------------------
// Compat detection (pi-mono: detectCompat)
// ---------------------------------------------------------------------------

/// Auto-detect provider compatibility flags from model config.
///
/// Uses the model's configured `compat` when available, otherwise infers from
/// provider name and base_url (matching pi-mono's detectCompat function).
fn detect_compat(model: &Model) -> ProviderCompat {
    if let Some(ref compat) = model.compat {
        return compat.clone();
    }

    let provider = model.provider.as_str();
    let base_url = model.base_url.as_str();

    let is_zai = provider == provider::ZAI || base_url.contains("api.z.ai");
    let is_deepseek = provider == provider::DEEPSEEK || base_url.contains("deepseek.com");
    let is_xai = provider == provider::XAI || base_url.contains("api.x.ai");
    let is_openrouter = provider == provider::OPENROUTER || base_url.contains("openrouter.ai");

    // pi-mono: isNonStandard does NOT include openrouter or groq
    let is_non_standard = is_zai
        || is_deepseek
        || is_xai
        || provider == "cerebras"
        || base_url.contains("cerebras.ai")
        || base_url.contains("chutes.ai")
        || provider == "opencode"
        || base_url.contains("opencode.ai");

    let max_tokens_field = if base_url.contains("chutes.ai") {
        MaxTokensField::MaxTokens
    } else {
        MaxTokensField::MaxCompletionTokens
    };

    let is_grok = is_xai; // xAI Grok models

    let thinking_format = if is_zai {
        Some(ThinkingFormat::Zai)
    } else if is_openrouter {
        Some(ThinkingFormat::OpenRouter)
    } else {
        Some(ThinkingFormat::OpenAI)
    };

    ProviderCompat {
        max_tokens_field,
        supports_reasoning_effort: !is_grok && !is_zai,
        thinking_format,
        requires_tool_result_name: false,
        requires_assistant_after_tool_result: false,
        requires_thinking_as_text: false,
        // pi-mono: supportsStrictMode is always true
        supports_strict_mode: true,
        supports_store: !is_non_standard,
        supports_developer_role: !is_non_standard,
    }
}

// ---------------------------------------------------------------------------
// Tool call ID normalization (pi-mono: normalizeToolCallId)
// ---------------------------------------------------------------------------

/// Normalize a tool call ID for OpenAI-compatible APIs.
///
/// Handles pipe-separated IDs from the Responses API (`{call_id}|{item_id}`)
/// and enforces length limits.
fn normalize_tool_call_id(id: &str, model: &Model) -> String {
    if id.contains('|') {
        // Extract the call_id part (before the pipe)
        let call_id = id.split('|').next().unwrap_or(id);
        return sanitize_id(call_id, 40);
    }
    if model.provider == provider::OPENAI && id.len() > 40 {
        return id[..40].to_string();
    }
    id.to_string()
}

/// Sanitize an ID: replace non-alphanumeric/non-dash/non-underscore chars, truncate.
fn sanitize_id(id: &str, max_len: usize) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() > max_len {
        sanitized[..max_len].to_string()
    } else {
        sanitized
    }
}

// ---------------------------------------------------------------------------
// Reasoning effort mapping (pi-mono: mapReasoningEffort)
// ---------------------------------------------------------------------------

/// Map our ReasoningLevel to the string expected by the provider.
fn map_reasoning_effort(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::Minimal => "low",
        ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh => "high",
    }
}

// ---------------------------------------------------------------------------
// SSE line processing
// ---------------------------------------------------------------------------

/// Process a single SSE line: skip empty/comment lines, strip "data: " prefix, parse chunk.
fn process_sse_line(line: &str, events: &mut Vec<AssistantMessageEvent>) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        match parse_sse_chunk(data) {
            Ok(Some(event)) => events.push(event),
            Ok(None) => {} // [DONE] or empty
            Err(e) => {
                tracing::warn!("SSE parse error: {e}, data: {data}");
            }
        }
    }
}

#[async_trait::async_trait]
impl ApiProvider for OpenAiCompletionsProvider {
    fn api(&self) -> &str {
        "openai-completions"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key: options.api_key first, then model.api_key_env.
        let api_key = if let Some(ref key) = options.api_key {
            key.clone()
        } else {
            match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(key) => key,
                Err(e) => {
                    return vec![AssistantMessageEvent::Error(format!("API key error: {e}"))];
                }
            }
        };

        let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
        let body = Self::build_request_body(model, context, tools, options);

        let mut request = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json");

        // Model-level headers.
        for (key, value) in &model.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        // Per-request headers from options.
        for (key, value) in &options.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = match request.json(&body).send().await {
            Ok(resp) => resp,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "HTTP request failed: {e}"
                ))];
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return vec![AssistantMessageEvent::Error(format!(
                "API error {status}: {body_text}"
            ))];
        }

        // Parse SSE stream chunk-by-chunk using a byte buffer to avoid
        // corrupting multi-byte UTF-8 sequences at chunk boundaries.
        use futures::StreamExt;

        let mut events = Vec::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(bytes) => bytes,
                Err(e) => {
                    events.push(AssistantMessageEvent::Error(format!(
                        "Stream read error: {e}"
                    )));
                    break;
                }
            };

            byte_buf.extend_from_slice(&chunk);

            // Process all complete lines in the buffer.
            while let Some(newline_pos) = byte_buf.iter().position(|&b| b == b'\n') {
                let line_bytes = byte_buf[..newline_pos].to_vec();
                byte_buf.drain(..=newline_pos);
                let line = String::from_utf8_lossy(&line_bytes);
                process_sse_line(&line, &mut events);
            }
        }

        // Flush remaining data after stream ends (final chunk may lack trailing newline).
        if !byte_buf.is_empty() {
            let remaining = String::from_utf8_lossy(&byte_buf);
            for line in remaining.lines() {
                process_sse_line(line, &mut events);
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::registry::StreamOptions;
    use crate::test_helpers::test_model;

    fn default_options() -> StreamOptions {
        StreamOptions::default()
    }

    // ========================================================================
    // build_request_body — basic
    // ========================================================================

    #[test]
    fn test_build_request_body_basic() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "You are a helper.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
        // System prompt + user message
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
    }

    // ========================================================================
    // build_request_body — with tools
    // ========================================================================

    #[test]
    fn test_build_request_body_with_tools() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: Some(0.7),
        };
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a bash command".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }];
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &tools,
            &default_options(),
        );

        assert!(body.get("tools").is_some());
        let tool_array = body["tools"].as_array().unwrap();
        assert_eq!(tool_array.len(), 1);
        assert_eq!(tool_array[0]["function"]["name"], "bash");
        assert!(
            body["temperature"].as_f64().unwrap() > 0.69
                && body["temperature"].as_f64().unwrap() < 0.71
        );
    }

    // ========================================================================
    // build_request_body — assistant tool calls
    // ========================================================================

    #[test]
    fn test_build_request_body_with_assistant_tool_calls() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![
                LlmMessage::User {
                    content: vec![LlmContent::Text("list files".into())],
                },
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![LlmToolCall {
                        id: "tc1".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"ls"}"#.into(),
                        },
                    }],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "file1.txt\nfile2.txt".into(),
                },
            ],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        let messages = body["messages"].as_array().unwrap();
        // system + user + assistant + tool = 4
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2]["role"], "assistant");
        assert!(messages[2]["tool_calls"].is_array());
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "tc1");
    }

    // ========================================================================
    // build_request_body — max_completion_tokens compat
    // ========================================================================

    #[test]
    fn test_build_request_body_max_completion_tokens() {
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 2048,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        // Should use max_completion_tokens instead of max_tokens
        assert_eq!(body["max_completion_tokens"], 2048);
        assert!(body.get("max_tokens").is_none() || body["max_tokens"].is_null());
        // Verify no literal "max_tokens_key" field
        assert!(body.get("max_tokens_key").is_none());
    }

    // ========================================================================
    // build_request_body — empty system prompt
    // ========================================================================

    #[test]
    fn test_build_request_body_empty_system_prompt() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            }],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        let messages = body["messages"].as_array().unwrap();
        // No system message when prompt is empty
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    // ========================================================================
    // build_request_body — reasoning model uses "developer" role
    // ========================================================================

    #[test]
    fn test_build_request_body_reasoning_uses_developer_role() {
        let mut model = test_model();
        model.reasoning = true;
        let context = LlmContext {
            messages: vec![LlmMessage::System {
                content: "inline system".into(),
            }],
            system_prompt: "You are a reasoning assistant.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        // Both system prompts should use "developer" role
        assert_eq!(messages[0]["role"], "developer");
        assert_eq!(messages[1]["role"], "developer");
    }

    #[test]
    fn test_build_request_body_non_reasoning_uses_system_role() {
        let mut model = test_model();
        model.reasoning = false;
        let context = LlmContext {
            messages: vec![LlmMessage::System {
                content: "inline system".into(),
            }],
            system_prompt: "You are a regular assistant.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "system");
    }

    // ========================================================================
    // build_request_body — options override temperature and max_tokens
    // ========================================================================

    #[test]
    fn test_build_request_body_options_override_temperature() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: Some(0.5),
        };
        let options = StreamOptions {
            temperature: Some(0.9),
            ..StreamOptions::default()
        };
        let body = OpenAiCompletionsProvider::build_request_body(&model, &context, &[], &options);

        // Options temperature should win
        let temp = body["temperature"].as_f64().unwrap();
        assert!(temp > 0.89 && temp < 0.91);
    }

    #[test]
    fn test_build_request_body_options_override_max_tokens() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let options = StreamOptions {
            max_tokens: Some(8192),
            ..StreamOptions::default()
        };
        let body = OpenAiCompletionsProvider::build_request_body(&model, &context, &[], &options);

        assert_eq!(body["max_tokens"], 8192);
    }

    // ========================================================================
    // api() identifier
    // ========================================================================

    #[test]
    fn test_api_identifier() {
        let provider = OpenAiCompletionsProvider::new();
        assert_eq!(provider.api(), "openai-completions");
    }

    // ========================================================================
    // build_request_body — stream_options always present
    // ========================================================================

    #[test]
    fn test_build_request_body_includes_stream_options() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    // ========================================================================
    // build_request_body — compat defaults when model.compat is None
    // ========================================================================

    #[test]
    fn test_build_request_body_compat_none_uses_detect_compat() {
        let mut model = test_model();
        model.compat = None;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        // detect_compat defaults to max_completion_tokens for unknown providers
        assert_eq!(body["max_completion_tokens"], 1024);
    }

    // ========================================================================
    // process_sse_line — helper function
    // ========================================================================

    #[test]
    fn test_process_sse_line_empty() {
        let mut events = Vec::new();
        process_sse_line("", &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn test_process_sse_line_comment() {
        let mut events = Vec::new();
        process_sse_line(": this is a comment", &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn test_process_sse_line_data() {
        let mut events = Vec::new();
        process_sse_line(
            r#"data: {"choices":[{"delta":{"content":"hi"},"index":0}]}"#,
            &mut events,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "hi"));
    }

    #[test]
    fn test_process_sse_line_done() {
        let mut events = Vec::new();
        process_sse_line("data: [DONE]", &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn test_process_sse_line_no_data_prefix() {
        let mut events = Vec::new();
        // Lines without "data: " prefix are ignored
        process_sse_line("event: message", &mut events);
        assert!(events.is_empty());
    }

    // ========================================================================
    // build_request_body — image content in user message
    // ========================================================================

    #[test]
    fn test_build_request_body_image_content() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![
                    LlmContent::Text("What is this?".into()),
                    LlmContent::Image {
                        url: "data:image/png;base64,abc123".into(),
                    },
                ],
            }],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,abc123");
    }

    // ========================================================================
    // detect_compat
    // ========================================================================

    #[test]
    fn test_detect_compat_zai_provider() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::ZAI.into();
        model.base_url = "https://open.bigmodel.cn/api/paas/v4".into();
        let compat = detect_compat(&model);
        assert!(matches!(compat.thinking_format, Some(ThinkingFormat::Zai)));
        assert!(!compat.supports_store);
        assert!(!compat.supports_developer_role);
    }

    #[test]
    fn test_detect_compat_openrouter() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::OPENROUTER.into();
        let compat = detect_compat(&model);
        assert!(matches!(
            compat.thinking_format,
            Some(ThinkingFormat::OpenRouter)
        ));
        // OpenRouter is NOT non-standard per pi-mono — supports store/developer role
        assert!(compat.supports_store);
        assert!(compat.supports_developer_role);
    }

    #[test]
    fn test_detect_compat_deepseek() {
        let mut model = test_model();
        model.compat = None;
        model.base_url = "https://api.deepseek.com/v1".into();
        model.provider = provider::DEEPSEEK.into();
        let compat = detect_compat(&model);
        assert!(!compat.supports_store);
        assert!(!compat.supports_developer_role);
    }

    #[test]
    fn test_detect_compat_uses_model_compat_when_present() {
        let model = test_model(); // has compat = Some(default)
        let compat = detect_compat(&model);
        // Should use the model's compat, not auto-detect
        assert!(matches!(compat.max_tokens_field, MaxTokensField::MaxTokens));
    }

    // ========================================================================
    // Thinking format request parameters
    // ========================================================================

    #[test]
    fn test_build_request_body_qwen_thinking() {
        let mut model = test_model();
        model.reasoning = true;
        model.compat = Some(ProviderCompat {
            thinking_format: Some(ThinkingFormat::Qwen),
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            reasoning: Some(ReasoningLevel::High),
            ..StreamOptions::default()
        };
        let body = OpenAiCompletionsProvider::build_request_body(&model, &context, &[], &options);
        assert_eq!(body["enable_thinking"], true);
    }

    #[test]
    fn test_build_request_body_zai_thinking() {
        let mut model = test_model();
        model.reasoning = true;
        model.compat = Some(ProviderCompat {
            thinking_format: Some(ThinkingFormat::Zai),
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            reasoning: Some(ReasoningLevel::Medium),
            ..StreamOptions::default()
        };
        let body = OpenAiCompletionsProvider::build_request_body(&model, &context, &[], &options);
        assert_eq!(body["enable_thinking"], true);
    }

    #[test]
    fn test_build_request_body_openrouter_reasoning() {
        let mut model = test_model();
        model.reasoning = true;
        model.compat = Some(ProviderCompat {
            thinking_format: Some(ThinkingFormat::OpenRouter),
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            reasoning: Some(ReasoningLevel::High),
            ..StreamOptions::default()
        };
        let body = OpenAiCompletionsProvider::build_request_body(&model, &context, &[], &options);
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_build_request_body_openrouter_no_effort() {
        let mut model = test_model();
        model.reasoning = true;
        model.compat = Some(ProviderCompat {
            thinking_format: Some(ThinkingFormat::OpenRouter),
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    // ========================================================================
    // requiresAssistantAfterToolResult
    // ========================================================================

    #[test]
    fn test_build_request_body_inserts_synthetic_assistant() {
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            requires_assistant_after_tool_result: true,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![
                LlmMessage::User {
                    content: vec![LlmContent::Text("go".into())],
                },
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![LlmToolCall {
                        id: "tc1".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: "{}".into(),
                        },
                    }],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "done".into(),
                },
                LlmMessage::User {
                    content: vec![LlmContent::Text("next".into())],
                },
            ],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );
        let messages = body["messages"].as_array().unwrap();
        // user + assistant + tool + synthetic_assistant + user = 5
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[3]["role"], "assistant");
        assert_eq!(messages[3]["content"], "I have processed the tool results.");
    }

    // ========================================================================
    // normalize_tool_call_id
    // ========================================================================

    #[test]
    fn test_normalize_tool_call_id_pipe_separated() {
        let model = test_model();
        let result = normalize_tool_call_id("call_123|fc_abc+xyz/def==", &model);
        assert!(!result.contains('|'));
        assert!(result.len() <= 40);
        assert_eq!(result, "call_123");
    }

    #[test]
    fn test_normalize_tool_call_id_openai_long() {
        let mut model = test_model();
        model.provider = provider::OPENAI.into();
        let long_id = "a".repeat(50);
        let result = normalize_tool_call_id(&long_id, &model);
        assert_eq!(result.len(), 40);
    }

    #[test]
    fn test_normalize_tool_call_id_normal() {
        let model = test_model();
        let result = normalize_tool_call_id("call_regular_123", &model);
        assert_eq!(result, "call_regular_123");
    }

    // ========================================================================
    // store field
    // ========================================================================

    #[test]
    fn test_build_request_body_store_false_for_standard() {
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            supports_store: true,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );
        assert_eq!(body["store"], false);
    }

    #[test]
    fn test_build_request_body_no_store_for_non_standard() {
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            supports_store: false,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[],
            &default_options(),
        );
        assert!(body.get("store").is_none() || body["store"].is_null());
    }

    // ========================================================================
    // strict mode
    // ========================================================================

    #[test]
    fn test_build_request_body_strict_mode_on_tools() {
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            supports_strict_mode: true,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run command".into(),
            parameters: json!({"type": "object"}),
        }];
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &tools,
            &default_options(),
        );
        let tool_arr = body["tools"].as_array().unwrap();
        assert_eq!(tool_arr[0]["function"]["strict"], true);
    }
}
