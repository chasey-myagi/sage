// Google Generative AI Provider — streams completions via Gemini's SSE API.
//
// Endpoint: {base_url}/models/{model_id}:streamGenerateContent?alt=sse&key={api_key}
// Default base_url: https://generativelanguage.googleapis.com/v1beta

use crate::llm::keys;
use crate::llm::registry::{ApiProvider, StreamOptions};
use crate::llm::types::*;
use crate::types::{StopReason, Usage};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

// ---------------------------------------------------------------------------
// GoogleProvider
// ---------------------------------------------------------------------------

pub struct GoogleProvider {
    client: Client,
}

impl GoogleProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Message conversion
// ---------------------------------------------------------------------------

/// Convert `LlmMessage` list into Google `contents` array + optional system instruction.
/// System messages are extracted separately (returned as `None` in contents).
pub(crate) fn convert_messages(messages: &[LlmMessage]) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::new();

    // Accumulator for tool-result grouping: Google requires all functionResponse
    // parts from consecutive Tool messages to be merged into a single user turn.
    let mut pending_tool_parts: Vec<Value> = Vec::new();

    for msg in messages {
        match msg {
            LlmMessage::System { .. } => {
                // System messages are handled via systemInstruction, skip here.
            }
            LlmMessage::User { content } => {
                // Flush pending tool results first
                flush_tool_parts(&mut pending_tool_parts, &mut contents);

                let parts: Vec<Value> = content
                    .iter()
                    .map(|c| match c {
                        LlmContent::Text(text) => json!({ "text": text }),
                        LlmContent::Image { url } => {
                            // Attempt to parse data URL: data:<mime>;base64,<data>
                            if let Some(rest) = url.strip_prefix("data:") {
                                if let Some(semi_pos) = rest.find(';') {
                                    let mime = &rest[..semi_pos];
                                    if let Some(data) = rest[semi_pos..].strip_prefix(";base64,") {
                                        return json!({
                                            "inlineData": {
                                                "mimeType": mime,
                                                "data": data,
                                            }
                                        });
                                    }
                                }
                            }
                            // Fallback: treat as a text reference
                            json!({ "text": format!("[image: {}]", url) })
                        }
                    })
                    .collect();

                if !parts.is_empty() {
                    contents.push(json!({ "role": "user", "parts": parts }));
                }
            }
            LlmMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                // Flush pending tool results first
                flush_tool_parts(&mut pending_tool_parts, &mut contents);

                let mut parts: Vec<Value> = Vec::new();
                if !content.is_empty() {
                    parts.push(json!({ "text": content }));
                }
                for tc in tool_calls {
                    let args: Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                    parts.push(json!({
                        "functionCall": {
                            "name": tc.function.name,
                            "args": args,
                        }
                    }));
                }
                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            LlmMessage::Tool {
                tool_call_id: _,
                content,
                ..
            } => {
                // Find the tool name from preceding assistant message's tool_calls.
                // We search backwards through the messages for the matching call.
                let tool_name = find_tool_name_for_result(messages, msg);
                pending_tool_parts.push(json!({
                    "functionResponse": {
                        "name": tool_name,
                        "response": { "content": content },
                    }
                }));
            }
        }
    }

    // Flush any remaining tool results
    flush_tool_parts(&mut pending_tool_parts, &mut contents);

    contents
}

/// Flush accumulated tool-result parts into a single `user` turn.
fn flush_tool_parts(pending: &mut Vec<Value>, contents: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    contents.push(json!({ "role": "user", "parts": pending.drain(..).collect::<Vec<_>>() }));
}

/// Given a Tool message, walk backwards through the message list to find the
/// matching tool call name. Falls back to "unknown" if not found.
fn find_tool_name_for_result(messages: &[LlmMessage], tool_msg: &LlmMessage) -> String {
    let target_id = match tool_msg {
        LlmMessage::Tool { tool_call_id, .. } => tool_call_id,
        _ => return "unknown".into(),
    };

    for msg in messages.iter().rev() {
        if let LlmMessage::Assistant { tool_calls, .. } = msg {
            for tc in tool_calls {
                if tc.id == *target_id {
                    return tc.function.name.clone();
                }
            }
        }
    }

    "unknown".into()
}

// ---------------------------------------------------------------------------
// Tool conversion
// ---------------------------------------------------------------------------

pub(crate) fn convert_tools(tools: &[LlmTool]) -> Value {
    let declarations: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();

    json!([{ "functionDeclarations": declarations }])
}

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

/// Map a Google `finishReason` string to our `StopReason`.
fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" => StopReason::Stop,
        "MAX_TOKENS" => StopReason::Length,
        "SAFETY"
        | "RECITATION"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION"
        | "IMAGE_OTHER"
        | "LANGUAGE"
        | "MALFORMED_FUNCTION_CALL"
        | "UNEXPECTED_TOOL_CALL"
        | "NO_IMAGE"
        | "FINISH_REASON_UNSPECIFIED"
        | "OTHER" => StopReason::Error,
        _ => StopReason::Error,
    }
}

/// Parse a single Google SSE data payload into zero or more events.
pub(crate) fn parse_google_sse_data(data: &str) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Google SSE JSON parse error: {e}, data: {data}");
            return events;
        }
    };

    // --- Error object ---
    if let Some(error) = json.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        events.push(AssistantMessageEvent::Error(msg.to_string()));
        return events;
    }

    // --- Candidates ---
    if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
        if let Some(candidate) = candidates.first() {
            // Parts
            if let Some(parts) = candidate
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
            {
                for part in parts {
                    // Thinking delta: { "thought": true, "text": "..." }
                    if part.get("thought").and_then(|t| t.as_bool()) == Some(true) {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            events.push(AssistantMessageEvent::ThinkingDelta(text.to_string()));
                        }
                        continue;
                    }

                    // Function call: { "functionCall": { "name": ..., "args": ... } }
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = fc.get("args").cloned().unwrap_or(json!({}));
                        let args_str = serde_json::to_string(&args).unwrap_or_default();
                        let counter = TOOL_CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let id = format!("call_google_{counter}");

                        events.push(AssistantMessageEvent::ToolCallStart {
                            id: id.clone(),
                            name,
                        });
                        events.push(AssistantMessageEvent::ToolCallDelta {
                            id: id.clone(),
                            arguments_delta: args_str,
                        });
                        events.push(AssistantMessageEvent::ToolCallEnd { id });
                        continue;
                    }

                    // Text delta: { "text": "..." }
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        events.push(AssistantMessageEvent::TextDelta(text.to_string()));
                    }
                }
            }

            // Finish reason
            if let Some(reason_str) = candidate.get("finishReason").and_then(|r| r.as_str()) {
                events.push(AssistantMessageEvent::Done {
                    stop_reason: map_finish_reason(reason_str),
                });
            }
        }
    }

    // --- Usage metadata ---
    if let Some(usage) = json.get("usageMetadata") {
        let prompt = usage
            .get("promptTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let candidates_tokens = usage
            .get("candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total = usage
            .get("totalTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cached = usage
            .get("cachedContentTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let thoughts_tokens = usage
            .get("thoughtsTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        events.push(AssistantMessageEvent::Usage(Usage {
            input: prompt.saturating_sub(cached),
            output: candidates_tokens + thoughts_tokens,
            cache_read: cached,
            cache_write: 0,
            total_tokens: total,
            ..Usage::default()
        }));
    }

    events
}

/// Process a single raw SSE line, extracting events from `data:` lines.
pub(crate) fn process_google_sse_line(line: &str, events: &mut Vec<AssistantMessageEvent>) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        let parsed = parse_google_sse_data(data);
        events.extend(parsed);
    }
}

// ---------------------------------------------------------------------------
// Gemini thinking helpers
// ---------------------------------------------------------------------------

/// Check if a model is Gemini 3 (Pro, Flash, or Lite).
pub(crate) fn is_gemini3(model_id: &str) -> bool {
    model_id.contains("gemini-3") || model_id.contains("gemini3")
}

/// Check if a model is Gemini 3 Pro specifically.
pub(crate) fn is_gemini3_pro(model_id: &str) -> bool {
    model_id.contains("gemini-3-pro") || model_id.contains("gemini3-pro")
}

/// Map ReasoningLevel to Google ThinkingLevel string for Gemini 3 models.
pub(crate) fn get_gemini3_thinking_level(
    effort: &crate::llm::types::ReasoningLevel,
    model_id: &str,
) -> &'static str {
    use crate::llm::types::ReasoningLevel;
    if is_gemini3_pro(model_id) {
        match effort {
            ReasoningLevel::Minimal | ReasoningLevel::Low => "LOW",
            _ => "HIGH",
        }
    } else {
        // Flash/Lite
        match effort {
            ReasoningLevel::Minimal => "MINIMAL",
            ReasoningLevel::Low => "LOW",
            ReasoningLevel::Medium => "MEDIUM",
            ReasoningLevel::High | ReasoningLevel::XHigh => "HIGH",
        }
    }
}

/// Check if a model is Gemini 3 Flash/Lite (not Pro).
fn is_gemini3_flash(model_id: &str) -> bool {
    is_gemini3(model_id) && !is_gemini3_pro(model_id)
}

/// Get the disabled thinking config for a model.
/// Gemini 3 Pro → thinkingLevel: LOW (cannot fully disable).
/// Gemini 3 Flash/Lite → thinkingLevel: MINIMAL.
/// Gemini 2.x → thinkingBudget: 0.
pub(crate) fn get_disabled_thinking_config(model_id: &str) -> Value {
    if is_gemini3_pro(model_id) {
        json!({ "thinkingLevel": "LOW" })
    } else if is_gemini3_flash(model_id) {
        json!({ "thinkingLevel": "MINIMAL" })
    } else {
        json!({ "thinkingBudget": 0 })
    }
}

/// Get per-model thinking budget for Gemini 2.5 models.
pub(crate) fn get_google_budget(model_id: &str, effort: &crate::llm::types::ReasoningLevel) -> i64 {
    use crate::llm::types::ReasoningLevel;
    if model_id.contains("2.5-pro") || model_id.contains("25-pro") {
        match effort {
            ReasoningLevel::Minimal => 128,
            ReasoningLevel::Low => 2048,
            ReasoningLevel::Medium => 8192,
            ReasoningLevel::High | ReasoningLevel::XHigh => 32768,
        }
    } else if model_id.contains("2.5-flash") || model_id.contains("25-flash") {
        match effort {
            ReasoningLevel::Minimal => 128,
            ReasoningLevel::Low => 2048,
            ReasoningLevel::Medium => 8192,
            ReasoningLevel::High | ReasoningLevel::XHigh => 24576,
        }
    } else {
        -1 // dynamic budget
    }
}

// ---------------------------------------------------------------------------
// Shared request body builder (used by google.rs + google_vertex.rs)
// ---------------------------------------------------------------------------

/// Build the JSON request body for Google / Vertex AI — same wire format.
pub(crate) fn build_google_request_body(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &StreamOptions,
) -> Value {
    let mut body = json!({});

    // Contents
    let contents = convert_messages(&context.messages);
    body["contents"] = json!(contents);

    // System instruction
    if !context.system_prompt.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": &context.system_prompt }]
        });
    }

    // Tools
    if !tools.is_empty() {
        body["tools"] = convert_tools(tools);
    }

    // Generation config
    let mut gen_config = json!({});
    let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);
    gen_config["maxOutputTokens"] = json!(max_tokens);
    if let Some(temp) = options.temperature.or(context.temperature) {
        gen_config["temperature"] = json!(temp);
    }

    // Thinking configuration
    if options.thinking_enabled == Some(true) {
        let mut thinking_config = json!({ "includeThoughts": true });

        if let Some(effort) = &options.reasoning {
            if is_gemini3(&model.id) {
                thinking_config["thinkingLevel"] =
                    json!(get_gemini3_thinking_level(effort, &model.id));
            } else {
                let budget = options
                    .thinking_budget_tokens
                    .map(|b| b as i64)
                    .unwrap_or_else(|| get_google_budget(&model.id, effort));
                if budget > 0 {
                    thinking_config["thinkingBudget"] = json!(budget);
                }
            }
        }
        gen_config["thinkingConfig"] = thinking_config;
    } else if model.reasoning {
        gen_config["thinkingConfig"] = get_disabled_thinking_config(&model.id);
    }

    body["generationConfig"] = gen_config;

    body
}

// ---------------------------------------------------------------------------
// Shared SSE stream reader (used by google.rs + google_vertex.rs)
// ---------------------------------------------------------------------------

/// Read a Google/Vertex AI SSE response stream and parse into events.
pub(crate) async fn read_google_sse_stream(
    response: reqwest::Response,
) -> Vec<AssistantMessageEvent> {
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

        // Process complete lines
        while let Some(newline_pos) = byte_buf.iter().position(|&b| b == b'\n') {
            let line_bytes = byte_buf[..newline_pos].to_vec();
            byte_buf.drain(..=newline_pos);
            let line = String::from_utf8_lossy(&line_bytes);
            process_google_sse_line(&line, &mut events);
        }
    }

    // Flush remaining data
    if !byte_buf.is_empty() {
        let remaining = String::from_utf8_lossy(&byte_buf);
        for line in remaining.lines() {
            process_google_sse_line(line, &mut events);
        }
    }

    events
}

// ---------------------------------------------------------------------------
// ApiProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for GoogleProvider {
    fn api(&self) -> &str {
        "google-generative-ai"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key: options override > env var
        let api_key = match &options.api_key {
            Some(key) if !key.is_empty() => key.clone(),
            _ => match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(key) => key,
                Err(e) => {
                    return vec![AssistantMessageEvent::Error(format!("API key error: {e}"))];
                }
            },
        };

        // Build URL
        let base = if model.base_url.is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            model.base_url.trim_end_matches('/').to_string()
        };
        let url = format!(
            "{base}/models/{}:streamGenerateContent?alt=sse&key={api_key}",
            model.id
        );

        // Build request body (shared with Vertex AI)
        let body = build_google_request_body(model, context, tools, options);

        // Build the HTTP request
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json");

        // Extra headers from model config + options
        for (k, v) in &model.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        for (k, v) in &options.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let response = match req.json(&body).send().await {
            Ok(resp) => resp,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "HTTP request failed: {e}"
                ))];
            }
        };

        if !response.status().is_success() {
            return vec![crate::llm::provider_errors::handle_error_response(response, model).await];
        }

        // Parse SSE stream (shared with Vertex AI)
        read_google_sse_stream(response).await
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // convert_messages
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_messages_user_text() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("Hello".into())],
        }];
        let contents = convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn test_convert_messages_user_image() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Image {
                url: "data:image/png;base64,abc123".into(),
            }],
        }];
        let contents = convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        let part = &contents[0]["parts"][0];
        assert_eq!(part["inlineData"]["mimeType"], "image/png");
        assert_eq!(part["inlineData"]["data"], "abc123");
    }

    #[test]
    fn test_convert_messages_assistant_text_only() {
        let messages = vec![LlmMessage::Assistant {
            content: "Sure!".into(),
            tool_calls: vec![],
            thinking_blocks: vec![],
        }];
        let contents = convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["text"], "Sure!");
    }

    #[test]
    fn test_convert_messages_assistant_with_tool_call() {
        let messages = vec![LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "call_1".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];
        let contents = convert_messages(&messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "model");
        let fc = &contents[0]["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "bash");
        assert_eq!(fc["args"]["command"], "ls");
    }

    #[test]
    fn test_convert_messages_system_skipped() {
        let messages = vec![
            LlmMessage::System {
                content: "You are a helper.".into(),
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            },
        ];
        let contents = convert_messages(&messages);
        // System message is skipped in contents
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn test_convert_messages_tool_results_grouped() {
        // Two consecutive tool results should be merged into one user turn.
        let messages = vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("run two commands".into())],
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    LlmToolCall {
                        id: "call_a".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"ls"}"#.into(),
                        },
                    },
                    LlmToolCall {
                        id: "call_b".into(),
                        function: LlmFunctionCall {
                            name: "read".into(),
                            arguments: r#"{"path":"/tmp"}"#.into(),
                        },
                    },
                ],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "call_a".into(),
                content: "file1.txt".into(),
                tool_name: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call_b".into(),
                content: "contents".into(),
                tool_name: None,
            },
        ];
        let contents = convert_messages(&messages);
        // user + model + user(tool results grouped)
        assert_eq!(contents.len(), 3);

        let tool_turn = &contents[2];
        assert_eq!(tool_turn["role"], "user");
        let parts = tool_turn["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "bash");
        assert_eq!(
            parts[0]["functionResponse"]["response"]["content"],
            "file1.txt"
        );
        assert_eq!(parts[1]["functionResponse"]["name"], "read");
        assert_eq!(
            parts[1]["functionResponse"]["response"]["content"],
            "contents"
        );
    }

    #[test]
    fn test_convert_messages_tool_result_name_lookup() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "call_x".into(),
                    function: LlmFunctionCall {
                        name: "read_file".into(),
                        arguments: "{}".into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "call_x".into(),
                content: "file contents".into(),
                tool_name: None,
            },
        ];
        let contents = convert_messages(&messages);
        // model + user(tool result)
        assert_eq!(contents.len(), 2);
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
    }

    #[test]
    fn test_convert_messages_empty() {
        let contents = convert_messages(&[]);
        assert!(contents.is_empty());
    }

    // -----------------------------------------------------------------------
    // convert_tools
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_tools_basic() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a bash command".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
        }];
        let result = convert_tools(&tools);
        let decls = result[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0]["name"], "bash");
        assert_eq!(decls[0]["description"], "Run a bash command");
        assert_eq!(decls[0]["parameters"]["type"], "object");
    }

    #[test]
    fn test_convert_tools_multiple() {
        let tools = vec![
            LlmTool {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: json!({}),
            },
            LlmTool {
                name: "write".into(),
                description: "Write a file".into(),
                parameters: json!({}),
            },
        ];
        let result = convert_tools(&tools);
        let decls = result[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0]["name"], "read");
        assert_eq!(decls[1]["name"], "write");
    }

    #[test]
    fn test_convert_tools_empty() {
        let result = convert_tools(&[]);
        let decls = result[0]["functionDeclarations"].as_array().unwrap();
        assert!(decls.is_empty());
    }

    // -----------------------------------------------------------------------
    // map_finish_reason
    // -----------------------------------------------------------------------

    #[test]
    fn test_map_finish_reason_stop() {
        assert_eq!(map_finish_reason("STOP"), StopReason::Stop);
    }

    #[test]
    fn test_map_finish_reason_max_tokens() {
        assert_eq!(map_finish_reason("MAX_TOKENS"), StopReason::Length);
    }

    #[test]
    fn test_map_finish_reason_safety() {
        assert_eq!(map_finish_reason("SAFETY"), StopReason::Error);
    }

    #[test]
    fn test_map_finish_reason_recitation() {
        assert_eq!(map_finish_reason("RECITATION"), StopReason::Error);
    }

    #[test]
    fn test_map_finish_reason_blocklist() {
        assert_eq!(map_finish_reason("BLOCKLIST"), StopReason::Error);
    }

    #[test]
    fn test_map_finish_reason_prohibited_content() {
        assert_eq!(map_finish_reason("PROHIBITED_CONTENT"), StopReason::Error);
    }

    #[test]
    fn test_map_finish_reason_malformed_function_call() {
        assert_eq!(
            map_finish_reason("MALFORMED_FUNCTION_CALL"),
            StopReason::Error
        );
    }

    #[test]
    fn test_map_finish_reason_unknown() {
        assert_eq!(map_finish_reason("SOMETHING_NEW"), StopReason::Error);
    }

    // -----------------------------------------------------------------------
    // parse_google_sse_data
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_text_delta() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "Hello world" }]
                }
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Hello world"));
    }

    #[test]
    fn test_parse_thinking_delta() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "thought": true, "text": "Let me think..." }]
                }
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingDelta(s) if s == "Let me think...")
        );
    }

    #[test]
    fn test_parse_function_call() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "bash",
                            "args": { "command": "ls -la" }
                        }
                    }]
                }
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 3); // Start + Delta + End
        assert!(
            matches!(&events[0], AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        );
        match &events[1] {
            AssistantMessageEvent::ToolCallDelta {
                arguments_delta, ..
            } => {
                let parsed: Value = serde_json::from_str(arguments_delta).unwrap();
                assert_eq!(parsed["command"], "ls -la");
            }
            other => panic!("expected ToolCallDelta, got: {other:?}"),
        }
        assert!(matches!(
            &events[2],
            AssistantMessageEvent::ToolCallEnd { .. }
        ));
    }

    #[test]
    fn test_parse_tool_call_ids_are_unique() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "functionCall": { "name": "a", "args": {} } },
                        { "functionCall": { "name": "b", "args": {} } }
                    ]
                }
            }]
        }"#;
        let events = parse_google_sse_data(data);
        // 2 tool calls x 3 events each = 6
        assert_eq!(events.len(), 6);

        let id_a = match &events[0] {
            AssistantMessageEvent::ToolCallStart { id, .. } => id.clone(),
            other => panic!("expected ToolCallStart, got: {other:?}"),
        };
        let id_b = match &events[3] {
            AssistantMessageEvent::ToolCallStart { id, .. } => id.clone(),
            other => panic!("expected ToolCallStart, got: {other:?}"),
        };
        assert_ne!(id_a, id_b);
        assert!(id_a.starts_with("call_google_"));
        assert!(id_b.starts_with("call_google_"));
    }

    #[test]
    fn test_parse_finish_reason_stop() {
        let data = r#"{
            "candidates": [{
                "content": { "parts": [] },
                "finishReason": "STOP"
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert!(events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::Stop
        )));
    }

    #[test]
    fn test_parse_finish_reason_max_tokens() {
        let data = r#"{
            "candidates": [{
                "content": { "parts": [{ "text": "partial..." }] },
                "finishReason": "MAX_TOKENS"
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert!(events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::Length
        )));
    }

    #[test]
    fn test_parse_finish_reason_safety() {
        let data = r#"{
            "candidates": [{
                "content": { "parts": [] },
                "finishReason": "SAFETY"
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert!(events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::Error
        )));
    }

    #[test]
    fn test_parse_usage_metadata() {
        let data = r#"{
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "totalTokenCount": 150,
                "cachedContentTokenCount": 20
            }
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 80); // 100 - 20 cached
                assert_eq!(u.output, 50);
                assert_eq!(u.total_tokens, 150);
                assert_eq!(u.cache_read, 20);
                assert_eq!(u.cache_write, 0);
            }
            other => panic!("expected Usage, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_usage_no_cache() {
        let data = r#"{
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "totalTokenCount": 150
            }
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 100);
                assert_eq!(u.output, 50);
                assert_eq!(u.cache_read, 0);
            }
            other => panic!("expected Usage, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_combined_text_and_finish() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "done." }]
                },
                "finishReason": "STOP"
            }]
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 2); // TextDelta + Done
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "done."));
        assert!(matches!(
            &events[1],
            AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::Stop
        ));
    }

    #[test]
    fn test_parse_error_object() {
        let data = r#"{
            "error": {
                "code": 429,
                "message": "Quota exceeded",
                "status": "RESOURCE_EXHAUSTED"
            }
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], AssistantMessageEvent::Error(s) if s.contains("Quota exceeded"))
        );
    }

    #[test]
    fn test_parse_malformed_json() {
        let events = parse_google_sse_data("not valid json {{{");
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_empty_candidates() {
        let data = r#"{ "candidates": [] }"#;
        let events = parse_google_sse_data(data);
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_mixed_parts() {
        // A response with thinking + text + function call
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "thinking step" },
                        { "text": "visible output" },
                        { "functionCall": { "name": "read", "args": {"path": "/tmp"} } }
                    ]
                }
            }]
        }"#;
        let events = parse_google_sse_data(data);
        // ThinkingDelta + TextDelta + ToolCallStart + ToolCallDelta + ToolCallEnd = 5
        assert_eq!(events.len(), 5);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingDelta(s) if s == "thinking step")
        );
        assert!(matches!(&events[1], AssistantMessageEvent::TextDelta(s) if s == "visible output"));
        assert!(
            matches!(&events[2], AssistantMessageEvent::ToolCallStart { name, .. } if name == "read")
        );
    }

    // -----------------------------------------------------------------------
    // process_google_sse_line
    // -----------------------------------------------------------------------

    #[test]
    fn test_process_sse_line_data_prefix() {
        let mut events = Vec::new();
        process_google_sse_line(
            r#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}"#,
            &mut events,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "hi"));
    }

    #[test]
    fn test_process_sse_line_empty() {
        let mut events = Vec::new();
        process_google_sse_line("", &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn test_process_sse_line_comment() {
        let mut events = Vec::new();
        process_google_sse_line(": keep-alive", &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn test_process_sse_line_no_data_prefix() {
        let mut events = Vec::new();
        process_google_sse_line("event: message", &mut events);
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------------
    // GoogleProvider — constructor & api()
    // -----------------------------------------------------------------------

    #[test]
    fn test_google_provider_api_name() {
        let provider = GoogleProvider::new();
        assert_eq!(provider.api(), "google-generative-ai");
    }

    // -----------------------------------------------------------------------
    // Gemini thinking helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_gemini3() {
        assert!(is_gemini3("gemini-3-pro-001"));
        assert!(is_gemini3("gemini-3-flash-001"));
        assert!(!is_gemini3("gemini-2.5-pro-001"));
    }

    #[test]
    fn test_get_gemini3_thinking_level_pro() {
        use crate::llm::types::ReasoningLevel;
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::Low, "gemini-3-pro-001"),
            "LOW"
        );
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::High, "gemini-3-pro-001"),
            "HIGH"
        );
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::Medium, "gemini-3-pro-001"),
            "HIGH"
        );
    }

    #[test]
    fn test_get_gemini3_thinking_level_flash() {
        use crate::llm::types::ReasoningLevel;
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::Minimal, "gemini-3-flash"),
            "MINIMAL"
        );
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::Low, "gemini-3-flash"),
            "LOW"
        );
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::Medium, "gemini-3-flash"),
            "MEDIUM"
        );
        assert_eq!(
            get_gemini3_thinking_level(&ReasoningLevel::High, "gemini-3-flash"),
            "HIGH"
        );
    }

    #[test]
    fn test_get_google_budget_25_pro() {
        use crate::llm::types::ReasoningLevel;
        assert_eq!(
            get_google_budget("gemini-2.5-pro-001", &ReasoningLevel::Minimal),
            128
        );
        assert_eq!(
            get_google_budget("gemini-2.5-pro-001", &ReasoningLevel::Low),
            2048
        );
        assert_eq!(
            get_google_budget("gemini-2.5-pro-001", &ReasoningLevel::Medium),
            8192
        );
        assert_eq!(
            get_google_budget("gemini-2.5-pro-001", &ReasoningLevel::High),
            32768
        );
    }

    #[test]
    fn test_get_google_budget_25_flash() {
        use crate::llm::types::ReasoningLevel;
        assert_eq!(
            get_google_budget("gemini-2.5-flash-001", &ReasoningLevel::Low),
            2048
        );
        assert_eq!(
            get_google_budget("gemini-2.5-flash-001", &ReasoningLevel::High),
            24576
        );
    }

    #[test]
    fn test_get_google_budget_unknown() {
        use crate::llm::types::ReasoningLevel;
        assert_eq!(
            get_google_budget("some-other-model", &ReasoningLevel::High),
            -1
        );
    }

    #[test]
    fn test_parse_usage_with_thoughts_tokens() {
        let data = r#"{
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "totalTokenCount": 170,
                "thoughtsTokenCount": 20
            }
        }"#;
        let events = parse_google_sse_data(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 100);
                assert_eq!(u.output, 70); // 50 candidates + 20 thoughts
                assert_eq!(u.total_tokens, 170);
            }
            other => panic!("expected Usage, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // get_disabled_thinking_config
    // -----------------------------------------------------------------------

    #[test]
    fn test_disabled_thinking_gemini3_pro() {
        let config = get_disabled_thinking_config("gemini-3-pro-001");
        assert_eq!(config["thinkingLevel"], "LOW");
        assert!(config.get("thinkingBudget").is_none());
    }

    #[test]
    fn test_disabled_thinking_gemini3_flash() {
        let config = get_disabled_thinking_config("gemini-3-flash-001");
        assert_eq!(config["thinkingLevel"], "MINIMAL");
    }

    #[test]
    fn test_disabled_thinking_gemini25() {
        let config = get_disabled_thinking_config("gemini-2.5-pro-001");
        assert_eq!(config["thinkingBudget"], 0);
        assert!(config.get("thinkingLevel").is_none());
    }

    #[test]
    fn test_is_gemini3_flash() {
        assert!(is_gemini3_flash("gemini-3-flash-001"));
        assert!(!is_gemini3_flash("gemini-3-pro-001"));
        assert!(!is_gemini3_flash("gemini-2.5-flash-001"));
    }
}
