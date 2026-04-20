// Shared logic for OpenAI Responses API providers.
//
// Used by both `openai_responses` and `openai_codex_responses`. Extracted
// from the two files to eliminate duplication, following the same split as
// pi-mono's `openai-responses-shared.ts`.

use crate::types::*;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Message conversion (LlmMessage → Responses API `input` array)
// ---------------------------------------------------------------------------

/// Convert an `LlmMessage` list into a Responses API `input` array.
///
/// System messages are skipped — the system prompt is expected to be sent via
/// the provider-specific `instructions` field.
pub fn convert_messages(messages: &[LlmMessage]) -> Vec<Value> {
    let mut input = Vec::new();

    for msg in messages {
        match msg {
            LlmMessage::System { .. } => {
                // System messages are handled via the `instructions` field.
            }

            LlmMessage::User { content } => {
                if content.len() == 1
                    && let LlmContent::Text(text) = &content[0]
                {
                    input.push(json!({ "role": "user", "content": text }));
                    continue;
                }
                // Multimodal or multi-part content.
                let parts: Vec<Value> = content
                    .iter()
                    .map(|c| match c {
                        LlmContent::Text(text) => json!({
                            "type": "input_text",
                            "text": text,
                        }),
                        LlmContent::Image { url } => json!({
                            "type": "input_image",
                            "image_url": url,
                            "detail": "auto",
                        }),
                    })
                    .collect();
                input.push(json!({ "role": "user", "content": parts }));
            }

            LlmMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": content, "annotations": [] }],
                        "status": "completed",
                    }));
                }
                for tc in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "id": format!("fc_{}", tc.id),
                        "call_id": tc.id,
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }));
                }
            }

            LlmMessage::Tool {
                tool_call_id,
                content,
                ..
            } => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
    }

    input
}

// ---------------------------------------------------------------------------
// Tool conversion (LlmTool → Responses API `tools` array)
// ---------------------------------------------------------------------------

/// Convert an `LlmTool` list into a Responses API `tools` array.
///
/// `strict` controls the `strict` field on each function tool entry.
/// Pass `None` to omit the field entirely (Codex behaviour).
pub fn convert_tools(tools: &[LlmTool], strict: Option<bool>) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            let mut def = json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            });
            if let Some(s) = strict {
                def["strict"] = json!(s);
            }
            def
        })
        .collect()
}

// ---------------------------------------------------------------------------
// SSE stream state
// ---------------------------------------------------------------------------

/// State tracker for output items while processing a Responses API SSE stream.
#[derive(Debug, Default)]
pub struct StreamState {
    /// Item type ("message", "function_call", "reasoning") keyed by output index.
    pub item_types: std::collections::HashMap<u64, String>,
    /// Composite tool-call id (`call_id|item_id`) keyed by output index.
    pub item_ids: std::collections::HashMap<u64, String>,
    /// Raw `call_id` keyed by output index (used by the standard Responses provider).
    pub item_call_ids: std::collections::HashMap<u64, String>,
}

// ---------------------------------------------------------------------------
// SSE event processing
// ---------------------------------------------------------------------------

/// Process a single Responses API SSE event and append zero or more
/// `AssistantMessageEvent` entries to `events`.
pub fn process_responses_event(
    event_type: &str,
    data: &Value,
    state: &mut StreamState,
    events: &mut Vec<AssistantMessageEvent>,
) {
    match event_type {
        "response.output_item.added" => {
            let item = &data["item"];
            let item_type = item["type"].as_str().unwrap_or("");
            let item_index = item["index"].as_u64().unwrap_or(0);

            state.item_types.insert(item_index, item_type.to_string());

            if item_type == "function_call" {
                let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                let id = item["id"].as_str().unwrap_or("").to_string();
                let name = item["name"].as_str().unwrap_or("").to_string();

                let composite_id = if !id.is_empty() {
                    normalize_id(&format!("{call_id}|{id}"), 64)
                } else {
                    normalize_id(&call_id, 64)
                };

                state.item_ids.insert(item_index, composite_id.clone());
                state.item_call_ids.insert(item_index, call_id);

                events.push(AssistantMessageEvent::ToolCallStart {
                    id: composite_id,
                    name,
                });
            }
        }

        "response.output_text.delta" => {
            let delta = data["delta"].as_str().unwrap_or("");
            if !delta.is_empty() {
                events.push(AssistantMessageEvent::TextDelta(delta.to_string()));
            }
        }

        "response.reasoning_summary_text.delta" => {
            let delta = data["delta"].as_str().unwrap_or("");
            if !delta.is_empty() {
                events.push(AssistantMessageEvent::ThinkingDelta(delta.to_string()));
            }
        }

        "response.function_call_arguments.delta" => {
            let delta = data["delta"].as_str().unwrap_or("");
            let item_index = data["item_index"]
                .as_u64()
                .or_else(|| data["output_index"].as_u64())
                .unwrap_or(0);

            let id = state.item_ids.get(&item_index).cloned().unwrap_or_default();
            if !delta.is_empty() {
                events.push(AssistantMessageEvent::ToolCallDelta {
                    id,
                    arguments_delta: delta.to_string(),
                });
            }
        }

        "response.output_item.done" => {
            let item = &data["item"];
            if item["type"].as_str() == Some("function_call") {
                let item_index = item["index"].as_u64().unwrap_or(0);
                let id = state.item_ids.get(&item_index).cloned().unwrap_or_default();
                events.push(AssistantMessageEvent::ToolCallEnd { id });
            }
        }

        "response.completed" => {
            let response = &data["response"];
            let status = response["status"].as_str().unwrap_or("completed");

            if let Some(usage_obj) = response.get("usage")
                && usage_obj.is_object()
            {
                let input_tokens = usage_obj["input_tokens"].as_u64().unwrap_or(0);
                let output_tokens = usage_obj["output_tokens"].as_u64().unwrap_or(0);
                let cached_tokens = usage_obj
                    .pointer("/input_tokens_details/cached_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                events.push(AssistantMessageEvent::Usage(Usage {
                    input: input_tokens.saturating_sub(cached_tokens),
                    output: output_tokens,
                    cache_read: cached_tokens,
                    cache_write: 0,
                    total_tokens: input_tokens + output_tokens,
                    ..Usage::default()
                }));
            }

            let has_tool_calls = state.item_types.values().any(|t| t == "function_call");
            let stop_reason = if has_tool_calls && status == "completed" {
                StopReason::ToolUse
            } else {
                map_stop_reason(status)
            };
            events.push(AssistantMessageEvent::Done { stop_reason });
        }

        "response.failed" => {
            let response = &data["response"];
            let error = response.get("error");
            let details = response.get("incomplete_details");
            let msg = if let Some(err) = error {
                let code = err["code"].as_str().unwrap_or("unknown");
                let message = err["message"].as_str().unwrap_or("no message");
                format!("{code}: {message}")
            } else if let Some(det) = details {
                let reason = det["reason"].as_str().unwrap_or("unknown");
                format!("incomplete: {reason}")
            } else {
                "Unknown error".to_string()
            };
            events.push(AssistantMessageEvent::Error(msg));
        }

        "error" => {
            let code = data["code"].as_str().unwrap_or("unknown");
            let message = data["message"].as_str().unwrap_or("Unknown error");
            events.push(AssistantMessageEvent::Error(format!(
                "Error Code {code}: {message}"
            )));
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Stop reason mapping
// ---------------------------------------------------------------------------

/// Map an OpenAI Responses API response status string to `StopReason`.
pub fn map_stop_reason(status: &str) -> StopReason {
    match status {
        "completed" => StopReason::Stop,
        "incomplete" => StopReason::Length,
        "cancelled" | "failed" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate `id` to at most `max_len` bytes.
pub fn normalize_id(id: &str, max_len: usize) -> String {
    if id.len() <= max_len {
        id.to_string()
    } else {
        id[..max_len].to_string()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // convert_messages
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
                LlmContent::Text("Describe".into()),
                LlmContent::Image {
                    url: "data:image/png;base64,abc".into(),
                },
            ],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[1]["type"], "input_image");
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
    fn test_convert_assistant_text() {
        let messages = vec![LlmMessage::Assistant {
            content: "I can help.".into(),
            tool_calls: vec![],
            thinking_blocks: vec![],
        }];
        let input = convert_messages(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["text"], "I can help.");
    }

    #[test]
    fn test_convert_tool_result() {
        let messages = vec![LlmMessage::Tool {
            tool_call_id: "call_001".into(),
            content: "result".into(),
            tool_name: None,
        }];
        let input = convert_messages(&messages);
        assert_eq!(input[0]["type"], "function_call_output");
        assert_eq!(input[0]["call_id"], "call_001");
        assert_eq!(input[0]["output"], "result");
    }

    // -----------------------------------------------------------------------
    // convert_tools
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_tools_no_strict() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run bash".into(),
            parameters: json!({"type": "object"}),
        }];
        let result = convert_tools(&tools, None);
        assert_eq!(result[0]["type"], "function");
        assert!(result[0].get("strict").is_none());
    }

    #[test]
    fn test_convert_tools_with_strict_false() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run bash".into(),
            parameters: json!({"type": "object"}),
        }];
        let result = convert_tools(&tools, Some(false));
        assert_eq!(result[0]["strict"], false);
    }

    // -----------------------------------------------------------------------
    // process_responses_event
    // -----------------------------------------------------------------------

    #[test]
    fn test_text_delta() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        process_responses_event(
            "response.output_text.delta",
            &json!({ "delta": "hi" }),
            &mut state,
            &mut events,
        );
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "hi"));
    }

    #[test]
    fn test_thinking_delta() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        process_responses_event(
            "response.reasoning_summary_text.delta",
            &json!({ "delta": "thinking..." }),
            &mut state,
            &mut events,
        );
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingDelta(s) if s == "thinking...")
        );
    }

    #[test]
    fn test_function_call_flow() {
        let mut state = StreamState::default();
        let mut events = Vec::new();

        process_responses_event(
            "response.output_item.added",
            &json!({ "item": { "type": "function_call", "index": 0, "id": "fc_abc", "call_id": "call_123", "name": "bash" } }),
            &mut state,
            &mut events,
        );
        match &events[0] {
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "call_123|fc_abc");
                assert_eq!(name, "bash");
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }

        process_responses_event(
            "response.function_call_arguments.delta",
            &json!({ "delta": "{}", "item_index": 0 }),
            &mut state,
            &mut events,
        );
        assert!(
            matches!(&events[1], AssistantMessageEvent::ToolCallDelta { id, .. } if id == "call_123|fc_abc")
        );

        process_responses_event(
            "response.output_item.done",
            &json!({ "item": { "type": "function_call", "index": 0 } }),
            &mut state,
            &mut events,
        );
        assert!(
            matches!(&events[2], AssistantMessageEvent::ToolCallEnd { id } if id == "call_123|fc_abc")
        );
    }

    #[test]
    fn test_response_completed_usage_and_stop() {
        let mut state = StreamState::default();
        let mut events = Vec::new();
        process_responses_event(
            "response.completed",
            &json!({ "response": { "status": "completed", "usage": { "input_tokens": 150, "output_tokens": 50, "input_tokens_details": { "cached_tokens": 30 } } } }),
            &mut state,
            &mut events,
        );
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 120);
                assert_eq!(u.cache_read, 30);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        assert!(
            matches!(&events[1], AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::Stop)
        );
    }

    #[test]
    fn test_response_completed_tool_use() {
        let mut state = StreamState::default();
        state.item_types.insert(0, "function_call".into());
        let mut events = Vec::new();
        process_responses_event(
            "response.completed",
            &json!({ "response": { "status": "completed", "usage": { "input_tokens": 10, "output_tokens": 5 } } }),
            &mut state,
            &mut events,
        );
        assert!(
            matches!(&events[1], AssistantMessageEvent::Done { stop_reason } if *stop_reason == StopReason::ToolUse)
        );
    }

    // -----------------------------------------------------------------------
    // map_stop_reason
    // -----------------------------------------------------------------------

    #[test]
    fn test_map_stop_reason() {
        assert_eq!(map_stop_reason("completed"), StopReason::Stop);
        assert_eq!(map_stop_reason("incomplete"), StopReason::Length);
        assert_eq!(map_stop_reason("cancelled"), StopReason::Error);
        assert_eq!(map_stop_reason("failed"), StopReason::Error);
        assert_eq!(map_stop_reason("in_progress"), StopReason::Stop);
        assert_eq!(map_stop_reason("queued"), StopReason::Stop);
    }

    // -----------------------------------------------------------------------
    // normalize_id
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_id() {
        assert_eq!(normalize_id("short", 64), "short");
        let long = "a".repeat(100);
        assert_eq!(normalize_id(&long, 64).len(), 64);
    }
}
