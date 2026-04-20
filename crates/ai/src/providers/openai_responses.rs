// OpenAI Responses API Provider
// Implements the Responses API (POST /responses) which is distinct from Chat Completions.
// See: https://platform.openai.com/docs/api-reference/responses

use super::openai_responses_shared::{
    StreamState, convert_messages, convert_tools, process_responses_event,
};
#[cfg(test)]
use super::openai_responses_shared::{map_stop_reason, normalize_id};
use crate::keys;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Provider struct
// ---------------------------------------------------------------------------

/// Provider for the OpenAI Responses API (`POST {base_url}/responses`).
///
/// The Responses API differs from Chat Completions in request format (uses
/// `input` and `instructions` instead of `messages`), response event types
/// (`response.*`), and tool call representation.
pub struct OpenAiResponsesProvider {
    client: Client,
}

impl OpenAiResponsesProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request body builder
// ---------------------------------------------------------------------------

pub fn build_request_body(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &StreamOptions,
) -> Value {
    let input = convert_messages(&context.messages);

    let mut body = json!({
        "model": model.id,
        "stream": true,
        "input": input,
        "store": false,
    });

    // System prompt → `instructions` field
    if !context.system_prompt.is_empty() {
        body["instructions"] = json!(context.system_prompt);
    }

    // Max output tokens
    let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);
    if max_tokens > 0 {
        body["max_output_tokens"] = json!(max_tokens);
    }

    // Temperature
    let temp = options.temperature.or(context.temperature);
    if let Some(t) = temp {
        body["temperature"] = json!(t);
    }

    // Tools (strict: false matches pi-mono openai-responses default)
    if !tools.is_empty() {
        body["tools"] = json!(convert_tools(tools, Some(false)));
    }

    // Reasoning configuration
    if model.reasoning {
        if let Some(ref effort) = options.reasoning {
            let effort_str = match effort {
                crate::types::ReasoningLevel::Minimal => "low",
                crate::types::ReasoningLevel::Low => "low",
                crate::types::ReasoningLevel::Medium => "medium",
                crate::types::ReasoningLevel::High => "high",
                crate::types::ReasoningLevel::XHigh => "high",
            };
            body["reasoning"] = json!({
                "effort": effort_str,
                "summary": "auto",
            });
            // Include encrypted reasoning content for context pass-back
            body["include"] = json!(["reasoning.encrypted_content"]);
        } else if model.provider != "github-copilot" {
            // github-copilot doesn't recognize the reasoning field
            body["reasoning"] = json!({ "effort": "none" });
        }
    }

    // Session ID for prompt caching.
    // pi-mono: only send prompt_cache_key when cache_retention is not "none".
    if options.cache_retention.is_some() {
        if let Some(ref session_id) = options.session_id {
            body["prompt_cache_key"] = json!(session_id);
        }
    }

    // Prompt cache retention: "24h" for long retention on api.openai.com
    if let Some(ref retention) = options.cache_retention {
        if let Some(ttl) = get_prompt_cache_retention(&model.base_url, retention) {
            body["prompt_cache_retention"] = json!(ttl);
        }
    }

    body
}

/// Get prompt cache retention TTL. Only "long" retention on api.openai.com → "24h".
fn get_prompt_cache_retention(
    base_url: &str,
    retention: &crate::registry::CacheRetention,
) -> Option<&'static str> {
    use crate::registry::CacheRetention;
    if matches!(retention, CacheRetention::Extended) && base_url.contains("api.openai.com") {
        Some("24h")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Get cost multiplier for service tier.
/// flex = 0.5x cost, priority = 2x cost, default = 1x.
///
/// Mirrors pi-mono `openai-responses.ts` so cost reporting can align when
/// the Rust side wires up service-tier pricing.
#[allow(dead_code)]
fn get_service_tier_cost_multiplier(tier: &str) -> f64 {
    match tier {
        "flex" => 0.5,
        "priority" => 2.0,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// ApiProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for OpenAiResponsesProvider {
    fn api(&self) -> &str {
        api::OPENAI_RESPONSES
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key
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

        let url = format!("{}/responses", model.base_url.trim_end_matches('/'));
        let body = build_request_body(model, context, tools, options);

        // Build request with headers
        let mut request = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json");

        // Apply model-level headers
        for (k, v) in &model.headers {
            request = request.header(k.as_str(), v.as_str());
        }
        // Apply per-request headers
        for (k, v) in &options.headers {
            request = request.header(k.as_str(), v.as_str());
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
            return vec![crate::provider_errors::handle_error_response(response, model).await];
        }

        parse_sse_stream(response).await
    }
}

// ---------------------------------------------------------------------------
// Shared SSE stream parser (reused by Azure OpenAI provider)
// ---------------------------------------------------------------------------

/// Parse an SSE response stream from the OpenAI Responses API.
///
/// This is extracted from the ApiProvider impl to allow reuse by the Azure
/// OpenAI Responses provider, which uses the same SSE protocol.
pub async fn parse_sse_stream(response: reqwest::Response) -> Vec<AssistantMessageEvent> {
    use futures::StreamExt;

    let mut events = Vec::new();
    let mut state = StreamState::default();
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut current_event_type = String::new();
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

        // Process all complete lines in the buffer
        while let Some(newline_pos) = byte_buf.iter().position(|&b| b == b'\n') {
            let line_bytes = byte_buf[..newline_pos].to_vec();
            byte_buf.drain(..=newline_pos);
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim();

            if line.is_empty() {
                current_event_type.clear();
                continue;
            }

            if line.starts_with(':') {
                continue;
            }

            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event_type = event_type.trim().to_string();
            } else if let Some(data_str) = line.strip_prefix("data: ") {
                let data_str = data_str.trim();
                if data_str.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(data_str) {
                    Ok(data) => {
                        let event_type = if current_event_type.is_empty() {
                            data["type"].as_str().unwrap_or("").to_string()
                        } else {
                            current_event_type.clone()
                        };
                        if !event_type.is_empty() {
                            process_responses_event(&event_type, &data, &mut state, &mut events);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Responses API SSE parse error: {e}, data: {data_str}");
                    }
                }
            }
        }
    }

    // Flush remaining data after stream ends
    if !byte_buf.is_empty() {
        let remaining = String::from_utf8_lossy(&byte_buf);
        for line in remaining.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event_type = event_type.trim().to_string();
            } else if let Some(data_str) = line.strip_prefix("data: ") {
                if let Ok(data) = serde_json::from_str::<Value>(data_str.trim()) {
                    let event_type = if current_event_type.is_empty() {
                        data["type"].as_str().unwrap_or("").to_string()
                    } else {
                        current_event_type.clone()
                    };
                    if !event_type.is_empty() {
                        process_responses_event(&event_type, &data, &mut state, &mut events);
                    }
                }
            }
        }
    }

    events
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // convert_messages (re-exported from shared)
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_user_text_simple() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("Hello".into())],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"], "Hello");
    }

    #[test]
    fn test_convert_user_multimodal() {
        let messages = vec![LlmMessage::User {
            content: vec![
                LlmContent::Text("Describe this image".into()),
                LlmContent::Image {
                    url: "data:image/png;base64,abc123".into(),
                },
            ],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "Describe this image");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,abc123");
    }

    #[test]
    fn test_convert_system_skipped() {
        let messages = vec![
            LlmMessage::System {
                content: "You are a helper.".into(),
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("Hi".into())],
            },
        ];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    #[test]
    fn test_convert_assistant_text_only() {
        let messages = vec![LlmMessage::Assistant {
            content: "I can help with that.".into(),
            tool_calls: vec![],
            thinking_blocks: vec![],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "assistant");
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "I can help with that.");
    }

    #[test]
    fn test_convert_assistant_with_tool_calls() {
        let messages = vec![LlmMessage::Assistant {
            content: "Let me check.".into(),
            tool_calls: vec![LlmToolCall {
                id: "call_001".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_001");
        assert_eq!(input[1]["name"], "bash");
        assert_eq!(input[1]["arguments"], r#"{"command":"ls"}"#);
    }

    #[test]
    fn test_convert_assistant_empty_content_with_tool_calls() {
        let messages = vec![LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "call_002".into(),
                function: LlmFunctionCall {
                    name: "read".into(),
                    arguments: r#"{"path":"/tmp/file.txt"}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["name"], "read");
    }

    #[test]
    fn test_convert_tool_result() {
        let messages = vec![LlmMessage::Tool {
            tool_call_id: "call_001".into(),
            content: "file1.txt\nfile2.txt".into(),
            tool_name: None,
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call_output");
        assert_eq!(input[0]["call_id"], "call_001");
        assert_eq!(input[0]["output"], "file1.txt\nfile2.txt");
    }

    #[test]
    fn test_convert_full_conversation() {
        let messages = vec![
            LlmMessage::System {
                content: "You are helpful.".into(),
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("List files".into())],
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "call_010".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: r#"{"command":"ls"}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "call_010".into(),
                content: "README.md\nsrc/".into(),
                tool_name: None,
            },
            LlmMessage::Assistant {
                content: "Here are your files.".into(),
                tool_calls: vec![],
                thinking_blocks: vec![],
            },
        ];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 4);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[3]["type"], "message");
    }

    // -----------------------------------------------------------------------
    // convert_tools (re-exported from shared)
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_tools_basic() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a bash command".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        }];
        let result = convert_tools(&tools, Some(false));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[0]["description"], "Run a bash command");
        assert_eq!(result[0]["parameters"]["type"], "object");
    }

    #[test]
    fn test_convert_tools_multiple() {
        let tools = vec![
            LlmTool {
                name: "bash".into(),
                description: "Run a bash command".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
            LlmTool {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            },
        ];
        let result = convert_tools(&tools, Some(false));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[1]["name"], "read");
    }

    #[test]
    fn test_convert_tools_empty() {
        let result = convert_tools(&[], None);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // build_request_body
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_basic() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "Be helpful.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options);

        assert_eq!(body["model"], "test-responses-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "Be helpful.");
        assert_eq!(body["max_output_tokens"], 1024);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    #[test]
    fn test_build_request_body_no_system_prompt() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: Some(0.5),
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options);

        assert!(body.get("instructions").is_none());
        assert!(body["temperature"].as_f64().unwrap() > 0.49);
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: None,
        };
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run command".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &tools, &options);

        let tools_arr = body["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 1);
        assert_eq!(tools_arr[0]["name"], "bash");
    }

    #[test]
    fn test_build_request_body_options_override() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 512,
            temperature: Some(0.3),
        };
        let options = StreamOptions {
            max_tokens: Some(2048),
            temperature: Some(0.9),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);

        assert_eq!(body["max_output_tokens"], 2048);
        assert!(body["temperature"].as_f64().unwrap() > 0.89);
    }

    // -----------------------------------------------------------------------
    // process_responses_event (via shared)
    // -----------------------------------------------------------------------

    #[test]
    fn test_event_text_delta() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        let data = json!({ "delta": "Hello world" });
        process_responses_event("response.output_text.delta", &data, &mut state, &mut events);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Hello world"));
    }

    #[test]
    fn test_event_thinking_delta() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        let data = json!({ "delta": "Let me think..." });
        process_responses_event(
            "response.reasoning_summary_text.delta",
            &data,
            &mut state,
            &mut events,
        );
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingDelta(s) if s == "Let me think...")
        );
    }

    #[test]
    fn test_event_function_call_flow() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let added_data = json!({
            "item": {
                "type": "function_call",
                "index": 0,
                "id": "fc_abc",
                "call_id": "call_123",
                "name": "bash",
            }
        });
        process_responses_event(
            "response.output_item.added",
            &added_data,
            &mut state,
            &mut events,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "call_123|fc_abc");
                assert_eq!(name, "bash");
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }

        let delta_data = json!({ "delta": r#"{"command":"ls"}"#, "item_index": 0 });
        process_responses_event(
            "response.function_call_arguments.delta",
            &delta_data,
            &mut state,
            &mut events,
        );
        assert_eq!(events.len(), 2);
        match &events[1] {
            AssistantMessageEvent::ToolCallDelta { id, arguments_delta } => {
                assert_eq!(id, "call_123|fc_abc");
                assert_eq!(arguments_delta, r#"{"command":"ls"}"#);
            }
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }

        let done_data = json!({
            "item": {
                "type": "function_call",
                "index": 0,
                "id": "fc_abc",
                "call_id": "call_123",
                "name": "bash",
                "arguments": r#"{"command":"ls"}"#,
            }
        });
        process_responses_event(
            "response.output_item.done",
            &done_data,
            &mut state,
            &mut events,
        );
        assert_eq!(events.len(), 3);
        match &events[2] {
            AssistantMessageEvent::ToolCallEnd { id } => {
                assert_eq!(id, "call_123|fc_abc");
            }
            other => panic!("expected ToolCallEnd, got {other:?}"),
        }
    }

    #[test]
    fn test_event_response_completed_with_usage() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let data = json!({
            "response": {
                "status": "completed",
                "usage": {
                    "input_tokens": 150,
                    "output_tokens": 50,
                    "input_tokens_details": { "cached_tokens": 30 }
                }
            }
        });
        process_responses_event("response.completed", &data, &mut state, &mut events);
        assert_eq!(events.len(), 2);

        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 120); // 150 - 30
                assert_eq!(u.output, 50);
                assert_eq!(u.cache_read, 30);
                assert_eq!(u.total_tokens, 200); // 150 + 50
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        match &events[1] {
            AssistantMessageEvent::Done { stop_reason } => {
                assert_eq!(*stop_reason, StopReason::Stop);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn test_event_response_completed_incomplete() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let data = json!({
            "response": {
                "status": "incomplete",
                "usage": { "input_tokens": 100, "output_tokens": 4096 }
            }
        });
        process_responses_event("response.completed", &data, &mut state, &mut events);
        let done = events
            .iter()
            .find(|e| matches!(e, AssistantMessageEvent::Done { .. }));
        assert!(matches!(
            done,
            Some(AssistantMessageEvent::Done { stop_reason }) if *stop_reason == StopReason::Length
        ));
    }

    #[test]
    fn test_event_error() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let data = json!({ "code": "rate_limit", "message": "Rate limit exceeded" });
        process_responses_event("error", &data, &mut state, &mut events);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error(msg) => {
                assert!(msg.contains("rate_limit"));
                assert!(msg.contains("Rate limit exceeded"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_event_response_failed() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let data = json!({
            "response": {
                "error": { "code": "server_error", "message": "Internal server error" }
            }
        });
        process_responses_event("response.failed", &data, &mut state, &mut events);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error(msg) => {
                assert!(msg.contains("server_error"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_event_message_item_added_no_event() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        let data = json!({
            "item": { "type": "message", "index": 0, "id": "msg_001", "role": "assistant" }
        });
        process_responses_event("response.output_item.added", &data, &mut state, &mut events);
        assert!(events.is_empty());
        assert_eq!(state.item_types.get(&0).unwrap(), "message");
    }

    #[test]
    fn test_event_unknown_type_ignored() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        process_responses_event("response.some_future_event", &json!({}), &mut state, &mut events);
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------------
    // map_stop_reason (re-exported from shared)
    // -----------------------------------------------------------------------

    #[test]
    fn test_map_stop_reason_variants() {
        assert_eq!(map_stop_reason("completed"), StopReason::Stop);
        assert_eq!(map_stop_reason("incomplete"), StopReason::Length);
        assert_eq!(map_stop_reason("cancelled"), StopReason::Error);
        assert_eq!(map_stop_reason("failed"), StopReason::Error);
        assert_eq!(map_stop_reason("in_progress"), StopReason::Stop);
        assert_eq!(map_stop_reason("queued"), StopReason::Stop);
        assert_eq!(map_stop_reason("unknown_future"), StopReason::Stop);
    }

    // -----------------------------------------------------------------------
    // ApiProvider::api()
    // -----------------------------------------------------------------------

    #[test]
    fn test_api_identifier() {
        let provider = OpenAiResponsesProvider::new();
        assert_eq!(provider.api(), "openai-responses");
    }

    // -----------------------------------------------------------------------
    // Test helper
    // -----------------------------------------------------------------------

    fn test_model() -> Model {
        Model {
            id: "test-responses-model".into(),
            name: "Test Responses Model".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: "openai".into(),
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
            compat: None,
        }
    }

    // -----------------------------------------------------------------------
    // Additional feature tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_has_store_false() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["store"], false);
    }

    #[test]
    fn test_build_request_body_reasoning() {
        let mut model = test_model();
        model.reasoning = true;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            reasoning: Some(crate::types::ReasoningLevel::Medium),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
        let include = body["include"].as_array().unwrap();
        assert_eq!(include[0], "reasoning.encrypted_content");
    }

    #[test]
    fn test_build_request_body_reasoning_default_none() {
        let mut model = test_model();
        model.reasoning = true;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    #[test]
    fn test_build_request_body_session_id() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            session_id: Some("sess_abc123".into()),
            cache_retention: Some(crate::registry::CacheRetention::Standard),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["prompt_cache_key"], "sess_abc123");
    }

    #[test]
    fn test_build_request_body_session_id_skipped_without_cache() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            session_id: Some("sess_abc123".into()),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn test_service_tier_cost_multiplier() {
        assert!((get_service_tier_cost_multiplier("flex") - 0.5).abs() < 0.01);
        assert!((get_service_tier_cost_multiplier("priority") - 2.0).abs() < 0.01);
        assert!((get_service_tier_cost_multiplier("default") - 1.0).abs() < 0.01);
        assert!((get_service_tier_cost_multiplier("auto") - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_normalize_id() {
        assert_eq!(normalize_id("short", 64), "short");
        let long = "a".repeat(100);
        assert_eq!(normalize_id(&long, 64).len(), 64);
    }

    #[test]
    fn test_tool_call_stop_reason_correction() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        state.item_types.insert(0, "function_call".into());
        state.item_ids.insert(0, "call_123|fc_abc".into());

        let data = json!({
            "response": {
                "status": "completed",
                "usage": { "input_tokens": 100, "output_tokens": 50 }
            }
        });
        process_responses_event("response.completed", &data, &mut state, &mut events);

        let done = events
            .iter()
            .find(|e| matches!(e, AssistantMessageEvent::Done { .. }));
        assert!(matches!(
            done,
            Some(AssistantMessageEvent::Done { stop_reason }) if *stop_reason == StopReason::ToolUse
        ));
    }

    #[test]
    fn test_build_request_body_cache_retention_openai() {
        let model = test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            session_id: Some("sess_123".into()),
            cache_retention: Some(crate::registry::CacheRetention::Extended),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["prompt_cache_key"], "sess_123");
        assert_eq!(body["prompt_cache_retention"], "24h");
    }

    #[test]
    fn test_build_request_body_cache_retention_non_openai() {
        let mut model = test_model();
        model.base_url = "https://other.proxy.com/v1".into();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            cache_retention: Some(crate::registry::CacheRetention::Extended),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert!(body.get("prompt_cache_retention").is_none());
    }

    #[test]
    fn test_get_prompt_cache_retention() {
        use crate::registry::CacheRetention;
        assert_eq!(
            get_prompt_cache_retention("https://api.openai.com/v1", &CacheRetention::Extended),
            Some("24h")
        );
        assert_eq!(
            get_prompt_cache_retention("https://api.openai.com/v1", &CacheRetention::Standard),
            None
        );
        assert_eq!(
            get_prompt_cache_retention("https://other.com/v1", &CacheRetention::Extended),
            None
        );
    }

    #[test]
    fn test_build_request_body_reasoning_includes_encrypted() {
        let mut model = test_model();
        model.reasoning = true;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            reasoning: Some(crate::types::ReasoningLevel::High),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["reasoning"]["effort"], "high");
        let include = body["include"].as_array().unwrap();
        assert!(include.iter().any(|v| v == "reasoning.encrypted_content"));
    }

    #[test]
    fn test_build_request_body_reasoning_none_no_include() {
        let mut model = test_model();
        model.reasoning = true;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options);
        assert_eq!(body["reasoning"]["effort"], "none");
        assert!(body.get("include").is_none());
    }
}
