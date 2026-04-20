// OpenAI Chat Completions Provider — implements ApiProvider trait.
// Ported from pi-mono's openai-completions.ts with full compat handling.
//
// Covers: OpenAI, DeepSeek, Qwen, Doubao, Kimi, MiniMax, ZAI (Zhipu),
// xAI (Grok), Groq, OpenRouter, and any OpenAI-compatible endpoint.

use std::collections::HashMap;

use std::pin::Pin;

use crate::keys;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use crate::utils::event_stream::event_stream;
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

impl Default for OpenAiCompletionsProvider {
    fn default() -> Self {
        Self::new()
    }
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
                    thinking_blocks,
                } => {
                    // pi-mono: skip assistant messages with no content and no tool_calls.
                    // Some providers reject empty assistant messages.
                    let has_content = !content.is_empty();
                    let has_tools = !tool_calls.is_empty();
                    if !has_content && !has_tools {
                        continue;
                    }

                    let mut msg = json!({
                        "role": "assistant",
                    });

                    // pi-mono: handle thinking blocks on assistant messages.
                    // Filter out empty and redacted blocks.
                    let non_empty_thinking: Vec<&ThinkingBlock> = thinking_blocks
                        .iter()
                        .filter(|b| !b.redacted && !b.thinking.trim().is_empty())
                        .collect();

                    if !non_empty_thinking.is_empty() {
                        if compat.requires_thinking_as_text {
                            // pi-mono: requiresThinkingAsText → prepend thinking as plain text.
                            // We concatenate as a single plain string (not content-part array)
                            // to avoid DeepSeek V3.2's nesting bug (see pi-mono comment).
                            let thinking_text: String = non_empty_thinking
                                .iter()
                                .map(|b| b.thinking.as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            let combined = if has_content {
                                format!("{thinking_text}\n\n{content}")
                            } else {
                                thinking_text
                            };
                            msg["content"] = json!(combined);
                        } else {
                            // pi-mono: use thinkingSignature as the field name on the
                            // assistant message. The signature carries the original SSE
                            // reasoning field name (e.g., "reasoning_content", "reasoning").
                            // If no signature is present, skip — pi-mono doesn't write a
                            // reasoning field when thinkingSignature is empty.
                            if let Some(field_name) = non_empty_thinking[0].signature.as_deref() {
                                let reasoning: String = non_empty_thinking
                                    .iter()
                                    .map(|b| b.thinking.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                msg[field_name] = json!(reasoning);
                            }
                            msg["content"] = json!(content);
                        }
                    } else {
                        msg["content"] = json!(content);
                    }

                    if has_tools {
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
                    tool_name,
                } => {
                    let id = normalize_tool_call_id(tool_call_id, model);
                    let mut tool_msg = json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": content,
                    });
                    // pi-mono: requiresToolResultName — add "name" field when provider needs it.
                    if compat.requires_tool_result_name
                        && let Some(name) = tool_name
                    {
                        tool_msg["name"] = json!(name);
                    }
                    messages.push(tool_msg);
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
                        let effort_str = map_reasoning_effort(
                            options.reasoning.unwrap_or(ReasoningLevel::High),
                            &compat.reasoning_effort_map,
                        );
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
                    let effort_str = map_reasoning_effort(
                        options.reasoning.unwrap_or(ReasoningLevel::High),
                        &compat.reasoning_effort_map,
                    );
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
                    // pi-mono: only include strict field when provider supports it.
                    // Some providers reject unknown fields. Value is false (not true).
                    if compat.supports_strict_mode {
                        def["function"]["strict"] = json!(false);
                    }
                    def
                })
                .collect();
            body["tools"] = json!(tool_defs);
        } else if has_tool_history(&context.messages) {
            // pi-mono: hasToolHistory — send empty tools array when conversation
            // has tool_calls/tool_results but no tools param. Required by some
            // providers (e.g. Anthropic via LiteLLM/proxy).
            body["tools"] = json!([]);
        }

        body
    }
}

// ---------------------------------------------------------------------------
// Tool history detection (pi-mono: hasToolHistory)
// ---------------------------------------------------------------------------

/// Check if conversation messages contain tool calls or tool results.
///
/// Needed because some providers (e.g. Anthropic via proxy) require the tools
/// param to be present when messages include tool_calls or tool role messages.
fn has_tool_history(messages: &[LlmMessage]) -> bool {
    messages.iter().any(|msg| match msg {
        LlmMessage::Tool { .. } => true,
        LlmMessage::Assistant { tool_calls, .. } => !tool_calls.is_empty(),
        _ => false,
    })
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
        || provider == provider::CEREBRAS
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
    let is_groq = provider == provider::GROQ || base_url.contains("groq.com");

    // pi-mono: Groq Qwen3-32b needs all reasoning levels remapped to "default"
    let reasoning_effort_map = if is_groq && model.id == "qwen/qwen3-32b" {
        [
            ReasoningLevel::Minimal,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::XHigh,
        ]
        .into_iter()
        .map(|l| (l, "default".into()))
        .collect()
    } else {
        HashMap::new()
    };

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
        reasoning_effort_map,
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
///
/// If the `reasoning_effort_map` contains an override for this level, use it.
/// Otherwise falls back to the level's own string representation
/// (pi-mono: `reasoningEffortMap[effort] ?? effort`).
fn map_reasoning_effort(
    level: ReasoningLevel,
    reasoning_effort_map: &HashMap<ReasoningLevel, String>,
) -> String {
    if let Some(override_val) = reasoning_effort_map.get(&level) {
        return override_val.clone();
    }
    // pi-mono fallback: return the effort string itself (not a compressed mapping)
    match level {
        ReasoningLevel::Minimal => "minimal",
        ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh => "xhigh",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// SSE line processing
// ---------------------------------------------------------------------------

/// Process a single SSE line: skip empty/comment lines, strip "data: " prefix, parse chunk.
///
/// Uses `parse_sse_chunk_multi` to handle providers (e.g., DashScope/Qwen) that
/// send both the tool call name AND arguments in the same SSE frame.
///
/// Kept for unit tests; production code uses `EventStream` instead.
#[cfg_attr(not(test), allow(dead_code))]
fn process_sse_line(line: &str, events: &mut Vec<AssistantMessageEvent>) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        match crate::stream::parse_sse_chunk_multi(data) {
            Ok(evts) => events.extend(evts),
            Err(e) => {
                tracing::warn!("SSE parse error: {e}, data: {data}");
            }
        }
    }
}

impl ApiProvider for OpenAiCompletionsProvider {
    fn api(&self) -> &str {
        "openai-completions"
    }

    fn stream_events<'a>(
        &'a self,
        model: &'a Model,
        context: &'a LlmContext,
        tools: &'a [LlmTool],
        options: &'a StreamOptions,
    ) -> Pin<Box<dyn futures::Stream<Item = AssistantMessageEvent> + Send + 'a>> {
        // Phase 1: resolve the HTTP response (async), then stream events lazily.
        // We use unfold so we don't need the async-stream crate.
        use futures::StreamExt as _;

        let setup = self.begin_request(model, context, tools, options);

        Box::pin(
            futures::stream::once(setup).flat_map(|result| match result {
                Err(single) => {
                    // Error during setup — emit one error event then stop.
                    futures::future::Either::Left(futures::stream::once(async move { single }))
                }
                Ok(bytes_stream) => {
                    // Happy path: wrap the byte stream in EventStream.
                    futures::future::Either::Right(event_stream(bytes_stream))
                }
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Internal async helpers
// ---------------------------------------------------------------------------

type OkStream = Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>;

impl OpenAiCompletionsProvider {
    /// Initiate the HTTP request and return either the byte stream or an error event.
    async fn begin_request<'a>(
        &'a self,
        model: &'a Model,
        context: &'a LlmContext,
        tools: &'a [LlmTool],
        options: &'a StreamOptions,
    ) -> Result<OkStream, AssistantMessageEvent> {
        // Resolve API key: options.api_key first, then model.api_key_env.
        let api_key = if let Some(ref key) = options.api_key {
            key.clone()
        } else {
            keys::resolve_api_key_from_env(&model.api_key_env)
                .map_err(|e| AssistantMessageEvent::Error(format!("API key error: {e}")))?
        };

        let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
        let body = Self::build_request_body(model, context, tools, options);

        let mut request = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json");

        for (key, value) in &model.headers {
            request = request.header(key.as_str(), value.as_str());
        }
        for (key, value) in &options.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| AssistantMessageEvent::Error(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            let ev = crate::provider_errors::handle_error_response(response, model).await;
            return Err(ev);
        }

        Ok(Box::pin(response.bytes_stream()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::StreamOptions;
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
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "file1.txt\nfile2.txt".into(),
                    tool_name: None,
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
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "done".into(),
                    tool_name: None,
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
        // pi-mono: strict is false, not true. Some providers need the field present
        // but OpenAI's strict JSON schema validation is NOT enabled.
        assert_eq!(tool_arr[0]["function"]["strict"], false);
    }

    // ========================================================================
    // detect_compat — Groq provider
    // ========================================================================

    #[test]
    fn test_detect_compat_groq_provider_name() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::GROQ.into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "llama-3-70b".into();
        let compat = detect_compat(&model);
        // pi-mono: Groq is NOT excluded from supportsReasoningEffort
        assert!(compat.supports_reasoning_effort);
        // Groq is not non-standard — supports store/developer
        assert!(compat.supports_store);
        assert!(compat.supports_developer_role);
    }

    #[test]
    fn test_detect_compat_groq_by_url() {
        let mut model = test_model();
        model.compat = None;
        model.provider = "custom".into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "some-model".into();
        let compat = detect_compat(&model);
        // pi-mono: Groq supports reasoning effort (not excluded like Grok/ZAI)
        assert!(compat.supports_reasoning_effort);
    }

    #[test]
    fn test_detect_compat_groq_qwen3_32b_has_reasoning_map() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::GROQ.into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "qwen/qwen3-32b".into();
        let compat = detect_compat(&model);
        // All 5 reasoning levels should map to "default"
        assert_eq!(compat.reasoning_effort_map.len(), 5);
        assert_eq!(
            compat.reasoning_effort_map[&ReasoningLevel::Minimal],
            "default"
        );
        assert_eq!(compat.reasoning_effort_map[&ReasoningLevel::Low], "default");
        assert_eq!(
            compat.reasoning_effort_map[&ReasoningLevel::Medium],
            "default"
        );
        assert_eq!(
            compat.reasoning_effort_map[&ReasoningLevel::High],
            "default"
        );
        assert_eq!(
            compat.reasoning_effort_map[&ReasoningLevel::XHigh],
            "default"
        );
    }

    #[test]
    fn test_detect_compat_groq_other_model_empty_reasoning_map() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::GROQ.into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "llama-3-70b".into();
        let compat = detect_compat(&model);
        assert!(compat.reasoning_effort_map.is_empty());
    }

    // ========================================================================
    // map_reasoning_effort — with reasoning_effort_map
    // ========================================================================

    #[test]
    fn test_map_reasoning_effort_uses_map_when_present() {
        let mut map = HashMap::new();
        map.insert(ReasoningLevel::High, "default".into());
        let result = map_reasoning_effort(ReasoningLevel::High, &map);
        assert_eq!(result, "default");
    }

    #[test]
    fn test_map_reasoning_effort_falls_back_when_map_empty() {
        let map = HashMap::new();
        // Empty map → return level's own string (pi-mono: effort ?? effort)
        assert_eq!(
            map_reasoning_effort(ReasoningLevel::Minimal, &map),
            "minimal"
        );
        assert_eq!(map_reasoning_effort(ReasoningLevel::Low, &map), "low");
        assert_eq!(map_reasoning_effort(ReasoningLevel::Medium, &map), "medium");
        assert_eq!(map_reasoning_effort(ReasoningLevel::High, &map), "high");
        assert_eq!(map_reasoning_effort(ReasoningLevel::XHigh, &map), "xhigh");
    }

    #[test]
    fn test_map_reasoning_effort_falls_back_for_missing_key() {
        let mut map = HashMap::new();
        map.insert(ReasoningLevel::High, "custom".into());
        // Low is not in map → return level's own string "low"
        let result = map_reasoning_effort(ReasoningLevel::Low, &map);
        assert_eq!(result, "low");
    }

    // ========================================================================
    // build_request_body — Groq Qwen3-32b reasoning remap integration
    // ========================================================================

    #[test]
    fn test_build_request_body_groq_qwen3_reasoning_remapped_to_default() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::GROQ.into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "qwen/qwen3-32b".into();
        model.reasoning = true;
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
        // pi-mono: Groq supports reasoning_effort, and qwen3-32b's map
        // remaps all levels to "default". The request body should contain
        // reasoning_effort = "default" (not "high").
        assert_eq!(body["reasoning_effort"], "default");
    }

    // ========================================================================
    // build_request_body — thinking blocks roundtrip on assistant messages
    // ========================================================================

    #[test]
    fn test_build_request_body_thinking_blocks_as_reasoning_content() {
        // pi-mono: when signature is present, it's used as the field name
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "I analyzed the code.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![ThinkingBlock {
                    thinking: "Let me think about this problem step by step.".into(),
                    signature: Some("reasoning_content".into()),
                    redacted: false,
                }],
            }],
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
        // system + assistant = 2
        let assistant = &messages[1];
        assert_eq!(assistant["role"], "assistant");
        // Thinking should be serialized as reasoning_content field
        assert_eq!(
            assistant["reasoning_content"],
            "Let me think about this problem step by step."
        );
        // Text content should still be present
        assert_eq!(assistant["content"], "I analyzed the code.");
    }

    #[test]
    fn test_build_request_body_thinking_blocks_no_signature_skips_reasoning() {
        // pi-mono: when thinkingSignature is absent, don't write reasoning field
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "I analyzed the code.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![ThinkingBlock {
                    thinking: "Internal reasoning.".into(),
                    signature: None,
                    redacted: false,
                }],
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
        let assistant = &messages[0];
        assert_eq!(assistant["role"], "assistant");
        // No reasoning field should be written when signature is None
        assert!(
            assistant.get("reasoning_content").is_none()
                || assistant["reasoning_content"].is_null()
        );
        assert_eq!(assistant["content"], "I analyzed the code.");
    }

    #[test]
    fn test_build_request_body_thinking_blocks_custom_signature_field() {
        // pi-mono: thinkingSignature carries the SSE field name (e.g., "reasoning")
        // and is used as the field name on the assistant message
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "Answer.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![ThinkingBlock {
                    thinking: "Reasoning via custom field.".into(),
                    signature: Some("reasoning".into()), // llama.cpp uses "reasoning"
                    redacted: false,
                }],
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
        let assistant = &messages[0];
        // Should use the custom field name from signature, not "reasoning_content"
        assert_eq!(assistant["reasoning"], "Reasoning via custom field.");
        assert!(
            assistant.get("reasoning_content").is_none()
                || assistant["reasoning_content"].is_null()
        );
    }

    #[test]
    fn test_build_request_body_thinking_blocks_multiple_joined() {
        // pi-mono: multiple thinking blocks are joined with "\n", field name from first block's signature
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "Done.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![
                    ThinkingBlock {
                        thinking: "Step 1: analyze".into(),
                        signature: Some("reasoning_content".into()),
                        redacted: false,
                    },
                    ThinkingBlock {
                        thinking: "Step 2: implement".into(),
                        signature: Some("reasoning_content".into()),
                        redacted: false,
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
        let assistant = &messages[0];
        assert_eq!(
            assistant["reasoning_content"],
            "Step 1: analyze\nStep 2: implement"
        );
    }

    #[test]
    fn test_build_request_body_thinking_blocks_empty_filtered() {
        // pi-mono: empty thinking blocks are filtered out
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "Result.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![
                    ThinkingBlock {
                        thinking: String::new(),
                        signature: None,
                        redacted: false,
                    },
                    ThinkingBlock {
                        thinking: "   ".into(), // whitespace only
                        signature: None,
                        redacted: false,
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
        let assistant = &messages[0];
        // No reasoning_content when all blocks are empty
        assert!(
            assistant.get("reasoning_content").is_none()
                || assistant["reasoning_content"].is_null()
        );
    }

    #[test]
    fn test_build_request_body_thinking_blocks_requires_thinking_as_text() {
        // pi-mono: requiresThinkingAsText → prepend thinking as plain text
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            requires_thinking_as_text: true,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "The answer is 42.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![ThinkingBlock {
                    thinking: "Let me calculate...".into(),
                    signature: None,
                    redacted: false,
                }],
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
        let assistant = &messages[0];
        // Thinking prepended to content as plain text
        let content = assistant["content"].as_str().unwrap();
        assert!(content.contains("Let me calculate..."));
        assert!(content.contains("The answer is 42."));
        // No separate reasoning_content field
        assert!(
            assistant.get("reasoning_content").is_none()
                || assistant["reasoning_content"].is_null()
        );
    }

    #[test]
    fn test_build_request_body_thinking_blocks_redacted_skipped() {
        // Redacted thinking blocks should not be serialized back for OpenAI-compat
        // (they are Anthropic-specific opaque payloads)
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "Result.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![ThinkingBlock {
                    thinking: String::new(),
                    signature: Some("encrypted_payload_xyz".into()),
                    redacted: true,
                }],
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
        let assistant = &messages[0];
        // Redacted blocks should not produce reasoning_content
        assert!(
            assistant.get("reasoning_content").is_none()
                || assistant["reasoning_content"].is_null()
        );
    }

    // ========================================================================
    // build_request_body — skip empty assistant messages
    // ========================================================================

    #[test]
    fn test_build_request_body_skips_empty_assistant_no_content_no_tools() {
        // pi-mono: skip assistant messages with no content and no tool_calls
        let model = test_model();
        let context = LlmContext {
            messages: vec![
                LlmMessage::User {
                    content: vec![LlmContent::Text("hello".into())],
                },
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![],
                    thinking_blocks: vec![],
                },
                LlmMessage::User {
                    content: vec![LlmContent::Text("world".into())],
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
        // Empty assistant should be skipped: user + user = 2
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn test_build_request_body_keeps_assistant_with_tool_calls() {
        // Assistant with tool_calls but empty content should NOT be skipped
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "tc1".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: "{}".into(),
                    },
                }],
                thinking_blocks: vec![],
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
        assert_eq!(messages[0]["role"], "assistant");
    }

    #[test]
    fn test_build_request_body_keeps_assistant_with_content() {
        // Assistant with text content but no tool_calls should NOT be skipped
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::Assistant {
                content: "I have an answer.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![],
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
        assert_eq!(messages[0]["content"], "I have an answer.");
    }

    // ========================================================================
    // build_request_body — requiresToolResultName
    // ========================================================================

    #[test]
    fn test_build_request_body_tool_result_name_when_required() {
        // pi-mono: add "name" field to tool results when requiresToolResultName is true
        let mut model = test_model();
        model.compat = Some(ProviderCompat {
            requires_tool_result_name: true,
            ..ProviderCompat::default()
        });
        let context = LlmContext {
            messages: vec![
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![LlmToolCall {
                        id: "tc1".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: "{}".into(),
                        },
                    }],
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "output".into(),
                    tool_name: Some("bash".into()),
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
        let tool_msg = &messages[1];
        assert_eq!(tool_msg["role"], "tool");
        assert_eq!(tool_msg["name"], "bash");
    }

    #[test]
    fn test_build_request_body_tool_result_no_name_when_not_required() {
        // Default: no name field on tool results
        let model = test_model();
        let context = LlmContext {
            messages: vec![
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![LlmToolCall {
                        id: "tc1".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: "{}".into(),
                        },
                    }],
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "output".into(),
                    tool_name: Some("bash".into()),
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
        let tool_msg = &messages[1];
        assert_eq!(tool_msg["role"], "tool");
        // name should NOT be present when requires_tool_result_name is false
        assert!(tool_msg.get("name").is_none() || tool_msg["name"].is_null());
    }

    // ========================================================================
    // build_request_body — hasToolHistory sends empty tools array
    // ========================================================================

    #[test]
    fn test_build_request_body_empty_tools_when_history_has_tool_calls() {
        // pi-mono: send tools:[] when conversation has tool history but no tools param
        let model = test_model();
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
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "tc1".into(),
                    content: "done".into(),
                    tool_name: None,
                },
            ],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        // No tools passed but history has tool calls
        let body = OpenAiCompletionsProvider::build_request_body(
            &model,
            &context,
            &[], // empty tools
            &default_options(),
        );
        // Should have tools: [] in the body
        let tools = body.get("tools");
        assert!(tools.is_some(), "tools field should be present");
        assert!(tools.unwrap().as_array().unwrap().is_empty());
    }

    #[test]
    fn test_build_request_body_no_tools_when_no_history() {
        // No tool history and no tools → no tools field
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hello".into())],
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
        assert!(
            body.get("tools").is_none() || body["tools"].is_null(),
            "tools field should not be present when no tools and no history"
        );
    }

    #[test]
    fn test_build_request_body_groq_non_qwen_reasoning_passes_through() {
        let mut model = test_model();
        model.compat = None;
        model.provider = provider::GROQ.into();
        model.base_url = "https://api.groq.com/openai/v1".into();
        model.id = "llama-3-70b".into();
        model.reasoning = true;
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
        // Non-qwen model on Groq: empty reasoning_effort_map, falls back to
        // level's own string "high" (pi-mono: effort ?? effort)
        assert_eq!(body["reasoning_effort"], "high");
    }
}
