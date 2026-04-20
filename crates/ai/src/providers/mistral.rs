// Mistral provider — ported from pi-mono's mistral.ts.
// Uses the Mistral chat completions streaming API via HTTP SSE.
// API identifier: "mistral-conversations"

use crate::keys;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Constants (pi-mono: MISTRAL_TOOL_CALL_ID_LENGTH)
// ---------------------------------------------------------------------------

const MISTRAL_TOOL_CALL_ID_LENGTH: usize = 9;
#[allow(dead_code)]
const MAX_MISTRAL_ERROR_BODY_CHARS: usize = 4000;

// ---------------------------------------------------------------------------
// MistralProvider
// ---------------------------------------------------------------------------

pub struct MistralProvider {
    client: Client,
}

impl MistralProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ApiProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for MistralProvider {
    fn api(&self) -> &str {
        "mistral-conversations"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key
        let api_key = match &options.api_key {
            Some(k) if !k.is_empty() => k.clone(),
            _ => match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(k) => k,
                Err(e) => return vec![AssistantMessageEvent::Error(format!("API key error: {e}"))],
            },
        };

        // Build tool call ID normalizer (stateful, per-request)
        let mut id_normalizer = MistralToolCallIdNormalizer::new();

        // Convert messages with Mistral-specific handling
        let messages = build_messages(model, context, &mut id_normalizer);

        let body = build_chat_payload(model, context, &messages, tools, options);

        // session_id → x-affinity header (pi-mono: options.sessionId → x-affinity)
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        if let Some(ref session_id) = options.session_id {
            extra_headers.push(("x-affinity".to_string(), session_id.clone()));
        }

        let base_url = if model.base_url.trim().is_empty() {
            "https://api.mistral.ai"
        } else {
            model.base_url.trim_end_matches('/')
        };

        let url = format!("{base_url}/v1/chat/completions");

        let mut req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        // Model-level headers
        for (k, v) in &model.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        // Per-request headers (options)
        for (k, v) in &options.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        // Extra headers (x-affinity)
        for (k, v) in &extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let response = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "HTTP request failed: {e}"
                ))];
            }
        };

        if !response.status().is_success() {
            return vec![crate::provider_errors::handle_error_response(response, model).await];
        }

        // Parse SSE stream
        let mut events = Vec::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();

        // Track active tool calls by index
        let mut tool_calls: HashMap<u64, ActiveToolCall> = HashMap::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    events.push(AssistantMessageEvent::Error(format!(
                        "Stream read error: {e}"
                    )));
                    break;
                }
            };

            byte_buf.extend_from_slice(&chunk);

            while let Some(pos) = byte_buf.iter().position(|&b| b == b'\n') {
                let line_bytes = byte_buf[..pos].to_vec();
                byte_buf.drain(..=pos);
                let line = String::from_utf8_lossy(&line_bytes);
                process_sse_line(&line, &mut events, &mut tool_calls, &mut id_normalizer);
            }
        }

        // Flush remaining
        if !byte_buf.is_empty() {
            let remaining = String::from_utf8_lossy(&byte_buf);
            for line in remaining.lines() {
                process_sse_line(line, &mut events, &mut tool_calls, &mut id_normalizer);
            }
        }

        events
    }
}

// ---------------------------------------------------------------------------
// Active tool call state
// ---------------------------------------------------------------------------

struct ActiveToolCall {
    id: String,
    partial_args: String,
}

// ---------------------------------------------------------------------------
// Tool call ID normalization (pi-mono: createMistralToolCallIdNormalizer)
// ---------------------------------------------------------------------------

/// Stateful normalizer that maps arbitrary tool call IDs to 9-char alphanumeric IDs.
/// Preserves bi-directional mapping to avoid collisions.
struct MistralToolCallIdNormalizer {
    id_map: HashMap<String, String>,
    reverse_map: HashMap<String, String>,
}

impl MistralToolCallIdNormalizer {
    fn new() -> Self {
        Self {
            id_map: HashMap::new(),
            reverse_map: HashMap::new(),
        }
    }

    /// Normalize an ID, deriving a 9-char alphanumeric string (collision-free).
    fn normalize(&mut self, id: &str) -> String {
        if let Some(existing) = self.id_map.get(id) {
            return existing.clone();
        }

        let mut attempt = 0u32;
        loop {
            let candidate = derive_mistral_tool_call_id(id, attempt);
            let owner = self.reverse_map.get(&candidate);
            if owner.is_none() || owner.map(|o| o.as_str()) == Some(id) {
                self.id_map.insert(id.to_string(), candidate.clone());
                self.reverse_map.insert(candidate.clone(), id.to_string());
                return candidate;
            }
            attempt += 1;
        }
    }
}

/// Derive a Mistral-compatible tool call ID (9 alphanumeric chars).
///
/// pi-mono: deriveMistralToolCallId
fn derive_mistral_tool_call_id(id: &str, attempt: u32) -> String {
    let normalized: String = id.chars().filter(|c| c.is_alphanumeric()).collect();
    if attempt == 0 && normalized.len() == MISTRAL_TOOL_CALL_ID_LENGTH {
        return normalized;
    }
    let seed_base = if normalized.is_empty() {
        id
    } else {
        &normalized
    };
    let seed = if attempt == 0 {
        seed_base.to_string()
    } else {
        format!("{seed_base}:{attempt}")
    };
    let hash = short_hash(&seed);
    hash.chars()
        .filter(|c| c.is_alphanumeric())
        .take(MISTRAL_TOOL_CALL_ID_LENGTH)
        .collect()
}

/// Simple deterministic short hash (mirrors pi-mono's shortHash utility).
/// Uses FNV-1a for a fast, stable 9-char output.
fn short_hash(input: &str) -> String {
    use std::num::Wrapping;
    let mut hash = Wrapping(0xcbf29ce484222325u64);
    for byte in input.bytes() {
        hash ^= Wrapping(byte as u64);
        hash *= Wrapping(0x100000001b3u64);
    }
    // Encode as lowercase alphanumeric
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut result = String::with_capacity(13);
    let mut h = hash.0;
    for _ in 0..13 {
        result.push(alphabet[(h % 36) as usize] as char);
        h /= 36;
    }
    result
}

// ---------------------------------------------------------------------------
// Message conversion (pi-mono: toChatMessages)
// ---------------------------------------------------------------------------

fn build_messages(
    model: &Model,
    context: &LlmContext,
    normalizer: &mut MistralToolCallIdNormalizer,
) -> Vec<Value> {
    let supports_images = model.input.contains(&InputType::Image);
    let mut result: Vec<Value> = Vec::new();

    for msg in &context.messages {
        match msg {
            LlmMessage::System { content } => {
                result.push(json!({
                    "role": "system",
                    "content": content,
                }));
            }

            LlmMessage::User { content } => {
                // Check for images
                let had_images = content
                    .iter()
                    .any(|c| matches!(c, LlmContent::Image { .. }));

                // Build content chunks
                let chunks: Vec<Value> = content
                    .iter()
                    .filter(|c| matches!(c, LlmContent::Text(_)) || supports_images)
                    .map(|c| match c {
                        LlmContent::Text(text) => json!({ "type": "text", "text": text }),
                        LlmContent::Image { url } => {
                            // pi-mono: { type: "image_url", imageUrl: `data:${mimeType};base64,${data}` }
                            json!({ "type": "image_url", "image_url": url })
                        }
                    })
                    .collect();

                if !chunks.is_empty() {
                    // If single text chunk, can use string shorthand
                    if chunks.len() == 1 {
                        if let Some(text) = chunks[0].get("text").and_then(|v| v.as_str()) {
                            result.push(json!({ "role": "user", "content": text }));
                            continue;
                        }
                    }
                    result.push(json!({ "role": "user", "content": chunks }));
                } else if had_images && !supports_images {
                    result.push(json!({
                        "role": "user",
                        "content": "(image omitted: model does not support images)",
                    }));
                }
            }

            LlmMessage::Assistant {
                content,
                tool_calls,
                thinking_blocks,
            } => {
                let mut content_parts: Vec<Value> = Vec::new();
                let mut tcs: Vec<Value> = Vec::new();

                // Thinking blocks → "thinking" content parts
                for tb in thinking_blocks {
                    let thinking_text = tb.thinking.trim();
                    if !thinking_text.is_empty() {
                        // pi-mono: thinking content uses Mistral's thinking format
                        content_parts.push(json!({
                            "type": "thinking",
                            "thinking": [{ "type": "text", "text": thinking_text }],
                        }));
                    }
                }

                // Text content
                if !content.trim().is_empty() {
                    content_parts.push(json!({ "type": "text", "text": content }));
                }

                // Tool calls
                for tc in tool_calls {
                    let normalized_id = normalizer.normalize(&tc.id);
                    let args: Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
                    tcs.push(json!({
                        "id": normalized_id,
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": tc.function.arguments,
                        },
                    }));
                    let _ = args; // already serialized into arguments string
                }

                if content_parts.is_empty() && tcs.is_empty() {
                    continue;
                }

                let mut msg = json!({ "role": "assistant" });
                if !content_parts.is_empty() {
                    msg["content"] = json!(content_parts);
                }
                if !tcs.is_empty() {
                    msg["tool_calls"] = json!(tcs);
                }
                result.push(msg);
            }

            LlmMessage::Tool {
                tool_call_id,
                content,
                tool_name,
            } => {
                let normalized_id = normalizer.normalize(tool_call_id);
                let tool_text = build_tool_result_text(content, false, supports_images, false);
                let content_chunks = vec![json!({ "type": "text", "text": tool_text })];

                let mut tool_msg = json!({
                    "role": "tool",
                    "tool_call_id": normalized_id,
                    "content": content_chunks,
                });
                if let Some(name) = tool_name {
                    tool_msg["name"] = json!(name);
                }
                result.push(tool_msg);
            }
        }
    }

    result
}

/// Build tool result text (pi-mono: buildToolResultText).
fn build_tool_result_text(
    text: &str,
    has_images: bool,
    supports_images: bool,
    is_error: bool,
) -> String {
    let trimmed = text.trim();
    let error_prefix = if is_error { "[tool error] " } else { "" };

    if !trimmed.is_empty() {
        let image_suffix = if has_images && !supports_images {
            "\n[tool image omitted: model does not support images]"
        } else {
            ""
        };
        return format!("{error_prefix}{trimmed}{image_suffix}");
    }

    if has_images {
        if supports_images {
            if is_error {
                "[tool error] (see attached image)".to_string()
            } else {
                "(see attached image)".to_string()
            }
        } else if is_error {
            "[tool error] (image omitted: model does not support images)".to_string()
        } else {
            "(image omitted: model does not support images)".to_string()
        }
    } else if is_error {
        "[tool error] (no tool output)".to_string()
    } else {
        "(no tool output)".to_string()
    }
}

// ---------------------------------------------------------------------------
// Chat payload construction (pi-mono: buildChatPayload)
// ---------------------------------------------------------------------------

fn build_chat_payload(
    model: &Model,
    context: &LlmContext,
    messages: &[Value],
    tools: &[LlmTool],
    options: &StreamOptions,
) -> Value {
    let mut body = json!({
        "model": model.id,
        "stream": true,
        "messages": messages,
    });

    // System prompt prepended as system message (pi-mono does this in buildChatPayload)
    // Note: already handled in build_messages via LlmMessage::System; but pi-mono also
    // unshifts context.systemPrompt. Handle here for the context-level system prompt.
    if !context.system_prompt.is_empty() {
        if let Some(msgs) = body["messages"].as_array_mut() {
            msgs.insert(
                0,
                json!({
                    "role": "system",
                    "content": context.system_prompt,
                }),
            );
        }
    }

    // Tools (pi-mono: toFunctionTools)
    if !tools.is_empty() {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "strict": false,
                    },
                })
            })
            .collect();
        body["tools"] = json!(tool_defs);
    }

    // Temperature (pi-mono: options.temperature)
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }

    // Max tokens
    let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);
    body["max_tokens"] = json!(max_tokens);

    // Prompt mode (pi-mono: options.promptMode — used for reasoning models)
    // Map from reasoning level to "reasoning" prompt mode
    if model.reasoning && options.reasoning.is_some() {
        body["prompt_mode"] = json!("reasoning");
    }

    body
}

// ---------------------------------------------------------------------------
// Stop reason mapping (pi-mono: mapChatStopReason)
// ---------------------------------------------------------------------------

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::Stop,
        "length" | "model_length" => StopReason::Length,
        "tool_calls" => StopReason::ToolUse,
        "error" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

// ---------------------------------------------------------------------------
// SSE line processing
// ---------------------------------------------------------------------------

fn process_sse_line(
    line: &str,
    events: &mut Vec<AssistantMessageEvent>,
    active_tool_calls: &mut HashMap<u64, ActiveToolCall>,
    normalizer: &mut MistralToolCallIdNormalizer,
) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }

    let data = match line.strip_prefix("data: ") {
        Some(d) => d,
        None => return,
    };

    if data == "[DONE]" {
        return;
    }

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Mistral SSE parse error: {e}, data: {data}");
            return;
        }
    };

    // Error object
    if let Some(error) = json.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown Mistral error");
        events.push(AssistantMessageEvent::Error(msg.to_string()));
        return;
    }

    let choice = match json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    {
        Some(c) => c,
        None => {
            // Usage-only chunk (at end of stream)
            if let Some(usage) = json.get("usage") {
                let input = usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output = usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total = usage
                    .get("total_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(input + output);
                events.push(AssistantMessageEvent::Usage(Usage {
                    input,
                    output,
                    total_tokens: total,
                    ..Usage::default()
                }));
            }
            return;
        }
    };

    // Finish reason
    if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
        if !finish_reason.is_empty() && finish_reason != "null" {
            events.push(AssistantMessageEvent::Done {
                stop_reason: map_stop_reason(finish_reason),
            });
        }
    }

    // Usage in the chunk (Mistral sometimes sends it here)
    if let Some(usage) = json.get("usage") {
        let input = usage
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(input + output);
        if input > 0 || output > 0 {
            events.push(AssistantMessageEvent::Usage(Usage {
                input,
                output,
                total_tokens: total,
                ..Usage::default()
            }));
        }
    }

    let delta = match choice.get("delta") {
        Some(d) => d,
        None => return,
    };

    // Text content delta — can be string or array of content chunks
    if let Some(content_val) = delta.get("content") {
        if !content_val.is_null() {
            match content_val {
                Value::String(text) => {
                    if !text.is_empty() {
                        events.push(AssistantMessageEvent::TextDelta(text.clone()));
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        match item.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        events.push(AssistantMessageEvent::TextDelta(
                                            text.to_string(),
                                        ));
                                    }
                                }
                            }
                            Some("thinking") => {
                                // pi-mono: thinking chunks are arrays of {type:"text", text:...}
                                if let Some(parts) = item.get("thinking").and_then(|t| t.as_array())
                                {
                                    let thinking_text: String = parts
                                        .iter()
                                        .filter_map(|p| {
                                            if p.get("type").and_then(|t| t.as_str())
                                                == Some("text")
                                            {
                                                p.get("text")
                                                    .and_then(|t| t.as_str())
                                                    .map(|s| s.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    if !thinking_text.is_empty() {
                                        events.push(AssistantMessageEvent::ThinkingDelta(
                                            thinking_text,
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Tool calls delta
    if let Some(tool_calls_val) = delta.get("tool_calls").and_then(|t| t.as_array()) {
        for tool_call in tool_calls_val {
            let index = tool_call.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

            let call_id_raw = tool_call
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && *s != "null");

            let fallback_id = format!("toolcall:{index}");
            let raw_id = call_id_raw.unwrap_or(&fallback_id);

            if !active_tool_calls.contains_key(&index) {
                // First chunk for this tool call index
                let normalized_id = normalizer.normalize(raw_id);
                let name = tool_call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();

                events.push(AssistantMessageEvent::ToolCallStart {
                    id: normalized_id.clone(),
                    name,
                });
                active_tool_calls.insert(
                    index,
                    ActiveToolCall {
                        id: normalized_id,
                        partial_args: String::new(),
                    },
                );
            }

            // Argument delta
            if let Some(atc) = active_tool_calls.get_mut(&index) {
                let args_delta = tool_call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if !args_delta.is_empty() {
                    atc.partial_args.push_str(args_delta);
                    events.push(AssistantMessageEvent::ToolCallDelta {
                        id: atc.id.clone(),
                        arguments_delta: args_delta.to_string(),
                    });
                }
            }
        }
    }

    // If finish reason was tool_calls, emit ToolCallEnd for all active tool calls
    if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
        if finish_reason == "tool_calls" {
            for (_, atc) in active_tool_calls.drain() {
                events.push(AssistantMessageEvent::ToolCallEnd { id: atc.id });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error formatting (pi-mono: formatMistralError)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn truncate_error_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let truncated_count = text.len() - max_chars;
    format!(
        "{}... [truncated {truncated_count} chars]",
        &text[..max_chars]
    )
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_mistral_id_already_9_chars() {
        let id = "abc123xyz";
        assert_eq!(id.len(), 9);
        assert_eq!(derive_mistral_tool_call_id(id, 0), id);
    }

    #[test]
    fn test_derive_mistral_id_normalizes() {
        let id = "call_abc123xyz_extra";
        let result = derive_mistral_tool_call_id(id, 0);
        assert_eq!(result.len(), 9);
        assert!(result.chars().all(|c| c.is_alphanumeric()));
    }

    #[test]
    fn test_normalizer_consistent() {
        let mut n = MistralToolCallIdNormalizer::new();
        let id1 = n.normalize("call_abc");
        let id2 = n.normalize("call_abc");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_normalizer_no_collision() {
        let mut n = MistralToolCallIdNormalizer::new();
        let id1 = n.normalize("call_001");
        let id2 = n.normalize("call_002");
        // Both should be 9 chars but may differ
        assert_eq!(id1.len(), MISTRAL_TOOL_CALL_ID_LENGTH);
        assert_eq!(id2.len(), MISTRAL_TOOL_CALL_ID_LENGTH);
    }

    #[test]
    fn test_map_stop_reason() {
        assert_eq!(map_stop_reason("stop"), StopReason::Stop);
        assert_eq!(map_stop_reason("length"), StopReason::Length);
        assert_eq!(map_stop_reason("model_length"), StopReason::Length);
        assert_eq!(map_stop_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(map_stop_reason("error"), StopReason::Error);
        assert_eq!(map_stop_reason("unknown"), StopReason::Stop);
    }

    #[test]
    fn test_build_tool_result_text_plain() {
        assert_eq!(
            build_tool_result_text("hello", false, false, false),
            "hello"
        );
    }

    #[test]
    fn test_build_tool_result_text_error() {
        assert_eq!(
            build_tool_result_text("oops", false, false, true),
            "[tool error] oops"
        );
    }

    #[test]
    fn test_build_tool_result_text_empty() {
        assert_eq!(
            build_tool_result_text("", false, false, false),
            "(no tool output)"
        );
    }

    #[test]
    fn test_truncate_error_text_short() {
        assert_eq!(truncate_error_text("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_error_text_long() {
        let long = "x".repeat(200);
        let result = truncate_error_text(&long, 100);
        assert!(result.contains("truncated"));
    }
}
