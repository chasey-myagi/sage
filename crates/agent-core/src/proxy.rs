// Proxy — mirrors pi-mono packages/agent/src/proxy.ts
//
// Provides a stream function that routes LLM calls through a proxy server.
// The server manages auth and proxies requests to LLM providers, stripping
// the `partial` field from delta events to reduce bandwidth.
// We reconstruct the partial message client-side.

use crate::types::*;
use ai::types::{AssistantMessageEvent, Model, StopReason, Usage};
use serde::{Deserialize, Serialize};

/// Proxy event types — server sends these with the partial field stripped.
///
/// Mirrors pi-mono's ProxyAssistantMessageEvent type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProxyEvent {
    Start,
    TextStart {
        content_index: usize,
    },
    TextDelta {
        content_index: usize,
        delta: String,
    },
    TextEnd {
        content_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_signature: Option<String>,
    },
    ThinkingStart {
        content_index: usize,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
    },
    ThinkingEnd {
        content_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_signature: Option<String>,
    },
    ToolcallStart {
        content_index: usize,
        id: String,
        tool_name: String,
    },
    ToolcallDelta {
        content_index: usize,
        delta: String,
    },
    ToolcallEnd {
        content_index: usize,
    },
    Done {
        reason: String,
        usage: ProxyUsage,
    },
    Error {
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        usage: ProxyUsage,
    },
}

/// Usage stats in proxy events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: ProxyCost,
}

/// Cost in proxy events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

impl From<ProxyUsage> for Usage {
    fn from(u: ProxyUsage) -> Self {
        Self {
            input: u.input,
            output: u.output,
            cache_read: u.cache_read,
            cache_write: u.cache_write,
            total_tokens: u.total_tokens,
            cost: ai::types::Cost {
                input: u.cost.input,
                output: u.cost.output,
                cache_read: u.cost.cache_read,
                cache_write: u.cost.cache_write,
                total: u.cost.total,
            },
        }
    }
}

/// Options for proxy stream calls.
///
/// Mirrors pi-mono's ProxyStreamOptions interface.
pub struct ProxyStreamOptions {
    /// Auth token for the proxy server.
    pub auth_token: String,
    /// Proxy server URL (e.g., "https://genai.example.com").
    pub proxy_url: String,
    /// Optional temperature.
    pub temperature: Option<f32>,
    /// Optional max tokens.
    pub max_tokens: Option<u32>,
    /// Optional reasoning level.
    pub reasoning: Option<String>,
    /// Optional cancellation token.
    pub signal: Option<tokio_util::sync::CancellationToken>,
}

/// Partial content block used during proxy stream reconstruction.
#[derive(Debug, Clone)]
enum PartialContent {
    Text {
        text: String,
        text_signature: Option<String>,
    },
    Thinking {
        thinking: String,
        thinking_signature: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        partial_json: String,
    },
}

/// Process a single proxy event and update the partial content array.
///
/// Returns the corresponding AssistantMessageEvent if applicable.
///
/// Mirrors pi-mono's `processProxyEvent` function.
pub fn process_proxy_event(
    event: &ProxyEvent,
    partial_content: &mut Vec<Option<PartialContent>>,
    usage: &mut Usage,
    stop_reason: &mut StopReason,
    error_message: &mut Option<String>,
    model: &Model,
) -> Option<AssistantMessageEvent> {
    match event {
        ProxyEvent::Start => {
            // Return a TextDelta with empty string to signal stream start.
            None
        }

        ProxyEvent::TextStart { content_index } => {
            let idx = *content_index;
            while partial_content.len() <= idx {
                partial_content.push(None);
            }
            partial_content[idx] = Some(PartialContent::Text {
                text: String::new(),
                text_signature: None,
            });
            None
        }

        ProxyEvent::TextDelta {
            content_index,
            delta,
        } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::Text { text, .. })) = partial_content.get_mut(idx) {
                text.push_str(delta);
                Some(AssistantMessageEvent::TextDelta(delta.clone()))
            } else {
                None
            }
        }

        ProxyEvent::TextEnd {
            content_index,
            content_signature,
        } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::Text { text_signature, .. })) =
                partial_content.get_mut(idx)
            {
                *text_signature = content_signature.clone();
            }
            None
        }

        ProxyEvent::ThinkingStart { content_index } => {
            let idx = *content_index;
            while partial_content.len() <= idx {
                partial_content.push(None);
            }
            partial_content[idx] = Some(PartialContent::Thinking {
                thinking: String::new(),
                thinking_signature: None,
            });
            None
        }

        ProxyEvent::ThinkingDelta {
            content_index,
            delta,
        } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::Thinking { thinking, .. })) =
                partial_content.get_mut(idx)
            {
                thinking.push_str(delta);
                Some(AssistantMessageEvent::ThinkingDelta(delta.clone()))
            } else {
                None
            }
        }

        ProxyEvent::ThinkingEnd {
            content_index,
            content_signature,
        } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::Thinking {
                thinking,
                thinking_signature,
            })) = partial_content.get_mut(idx)
            {
                *thinking_signature = content_signature.clone();
                Some(AssistantMessageEvent::ThinkingBlockEnd {
                    signature: content_signature.clone().unwrap_or_default(),
                    redacted: false,
                })
            } else {
                None
            }
        }

        ProxyEvent::ToolcallStart {
            content_index,
            id,
            tool_name,
        } => {
            let idx = *content_index;
            while partial_content.len() <= idx {
                partial_content.push(None);
            }
            partial_content[idx] = Some(PartialContent::ToolCall {
                id: id.clone(),
                name: tool_name.clone(),
                partial_json: String::new(),
            });
            Some(AssistantMessageEvent::ToolCallStart {
                id: id.clone(),
                name: tool_name.clone(),
            })
        }

        ProxyEvent::ToolcallDelta {
            content_index,
            delta,
        } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::ToolCall {
                partial_json, id, ..
            })) = partial_content.get_mut(idx)
            {
                partial_json.push_str(delta);
                Some(AssistantMessageEvent::ToolCallDelta {
                    id: id.clone(),
                    arguments_delta: delta.clone(),
                })
            } else {
                None
            }
        }

        ProxyEvent::ToolcallEnd { content_index } => {
            let idx = *content_index;
            if let Some(Some(PartialContent::ToolCall { id, .. })) = partial_content.get(idx) {
                Some(AssistantMessageEvent::ToolCallEnd { id: id.clone() })
            } else {
                None
            }
        }

        ProxyEvent::Done { reason, usage: u } => {
            *stop_reason = parse_stop_reason(reason);
            *usage = Usage::from(u.clone());
            Some(AssistantMessageEvent::Done {
                stop_reason: stop_reason.clone(),
            })
        }

        ProxyEvent::Error {
            reason,
            error_message: err_msg,
            usage: u,
        } => {
            *stop_reason = parse_stop_reason(reason);
            *error_message = err_msg.clone();
            *usage = Usage::from(u.clone());
            Some(AssistantMessageEvent::Error(
                err_msg.clone().unwrap_or_else(|| reason.clone()),
            ))
        }
    }
}

fn parse_stop_reason(s: &str) -> StopReason {
    match s {
        "stop" => StopReason::Stop,
        "length" => StopReason::Length,
        "toolUse" | "tool_use" => StopReason::ToolUse,
        "aborted" => StopReason::Aborted,
        _ => StopReason::Error,
    }
}

/// Collect all AssistantMessageEvents from a proxy SSE stream.
///
/// This is the Rust equivalent of pi-mono's `streamProxy` function,
/// adapted to return a Vec<AssistantMessageEvent> rather than an EventStream.
///
/// # Errors
/// Returns an Err string if the HTTP request fails. Individual stream errors
/// are encoded as `AssistantMessageEvent::Error`.
pub async fn collect_proxy_events(
    model: &Model,
    context_json: serde_json::Value,
    options: &ProxyStreamOptions,
) -> Result<Vec<AssistantMessageEvent>, String> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": {
            "id": model.id,
            "provider": model.provider,
            "api": model.api,
        },
        "context": context_json,
        "options": {
            "temperature": options.temperature,
            "maxTokens": options.max_tokens,
            "reasoning": options.reasoning,
        }
    });

    let url = format!("{}/api/stream", options.proxy_url);

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", options.auth_token))
        .header("Content-Type", "application/json")
        .json(&body);

    let response = req
        .send()
        .await
        .map_err(|e| format!("Proxy request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let error_msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )
            });
        return Err(format!("Proxy error: {error_msg}"));
    }

    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read proxy response: {e}"))?;

    let mut events = Vec::new();
    let mut partial_content: Vec<Option<PartialContent>> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason = StopReason::Stop;
    let mut error_message: Option<String> = None;

    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            match serde_json::from_str::<ProxyEvent>(data) {
                Ok(proxy_event) => {
                    if let Some(event) = process_proxy_event(
                        &proxy_event,
                        &mut partial_content,
                        &mut usage,
                        &mut stop_reason,
                        &mut error_message,
                        model,
                    ) {
                        events.push(event);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse proxy event: {e}, data: {data}");
                }
            }
        }
    }

    // Always append usage at the end.
    events.push(AssistantMessageEvent::Usage(usage));

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_model() -> Model {
        Model {
            id: "test".into(),
            name: "Test".into(),
            api: ai::types::api::OPENAI_COMPLETIONS.into(),
            provider: "test".into(),
            base_url: "".into(),
            api_key_env: "".into(),
            reasoning: false,
            input: vec![],
            max_tokens: 4096,
            context_window: 128000,
            cost: ai::types::ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    fn default_usage() -> ProxyUsage {
        ProxyUsage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            total_tokens: 0,
            cost: ProxyCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        }
    }

    #[test]
    fn test_proxy_event_start_returns_none() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;
        let result = process_proxy_event(
            &ProxyEvent::Start,
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_proxy_event_text_start_initializes_slot() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;
        process_proxy_event(
            &ProxyEvent::TextStart { content_index: 0 },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );
        assert_eq!(partial_content.len(), 1);
        assert!(matches!(
            &partial_content[0],
            Some(PartialContent::Text { .. })
        ));
    }

    #[test]
    fn test_proxy_event_text_delta_returns_text_delta_event() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        // Initialize slot first.
        process_proxy_event(
            &ProxyEvent::TextStart { content_index: 0 },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        let result = process_proxy_event(
            &ProxyEvent::TextDelta {
                content_index: 0,
                delta: "hello".into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        assert!(matches!(
            result,
            Some(AssistantMessageEvent::TextDelta(ref s)) if s == "hello"
        ));
    }

    #[test]
    fn test_proxy_event_thinking_delta_accumulates() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        process_proxy_event(
            &ProxyEvent::ThinkingStart { content_index: 0 },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        process_proxy_event(
            &ProxyEvent::ThinkingDelta {
                content_index: 0,
                delta: "thinking...".into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        if let Some(Some(PartialContent::Thinking { thinking, .. })) = partial_content.get(0) {
            assert_eq!(thinking, "thinking...");
        } else {
            panic!("expected Thinking content");
        }
    }

    #[test]
    fn test_proxy_event_toolcall_start_emits_event() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        let result = process_proxy_event(
            &ProxyEvent::ToolcallStart {
                content_index: 0,
                id: "tc1".into(),
                tool_name: "bash".into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        assert!(matches!(
            result,
            Some(AssistantMessageEvent::ToolCallStart { id, name })
            if id == "tc1" && name == "bash"
        ));
    }

    #[test]
    fn test_proxy_event_done_sets_stop_reason() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Error;
        let mut error_message = None;

        process_proxy_event(
            &ProxyEvent::Done {
                reason: "stop".into(),
                usage: default_usage(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        assert!(matches!(stop_reason, StopReason::Stop));
    }

    #[test]
    fn test_proxy_event_error_sets_error_state() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        let result = process_proxy_event(
            &ProxyEvent::Error {
                reason: "error".into(),
                error_message: Some("connection refused".into()),
                usage: default_usage(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        assert!(matches!(stop_reason, StopReason::Error));
        assert_eq!(error_message.as_deref(), Some("connection refused"));
        assert!(matches!(result, Some(AssistantMessageEvent::Error(_))));
    }

    #[test]
    fn test_proxy_usage_converts_to_ai_usage() {
        let proxy_usage = ProxyUsage {
            input: 100,
            output: 200,
            cache_read: 50,
            cache_write: 25,
            total_tokens: 375,
            cost: ProxyCost {
                input: 0.01,
                output: 0.02,
                cache_read: 0.005,
                cache_write: 0.0025,
                total: 0.0375,
            },
        };
        let usage = Usage::from(proxy_usage);
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 200);
        assert_eq!(usage.cache_read, 50);
        assert_eq!(usage.total_tokens, 375);
        assert!((usage.cost.total - 0.0375).abs() < 1e-10);
    }

    #[test]
    fn test_proxy_event_serde_roundtrip_text_delta() {
        let event = ProxyEvent::TextDelta {
            content_index: 2,
            delta: "hello world".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ProxyEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            ProxyEvent::TextDelta { content_index: 2, delta } if delta == "hello world"
        ));
    }

    #[test]
    fn test_proxy_event_serde_roundtrip_done() {
        let event = ProxyEvent::Done {
            reason: "stop".into(),
            usage: default_usage(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ProxyEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ProxyEvent::Done { .. }));
    }

    #[test]
    fn test_proxy_event_serde_roundtrip_toolcall_start() {
        let event = ProxyEvent::ToolcallStart {
            content_index: 1,
            id: "tc42".into(),
            tool_name: "read_file".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ProxyEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            ProxyEvent::ToolcallStart { content_index: 1, id, tool_name }
            if id == "tc42" && tool_name == "read_file"
        ));
    }

    #[test]
    fn test_parse_stop_reason_all_variants() {
        assert!(matches!(parse_stop_reason("stop"), StopReason::Stop));
        assert!(matches!(parse_stop_reason("length"), StopReason::Length));
        assert!(matches!(parse_stop_reason("toolUse"), StopReason::ToolUse));
        assert!(matches!(parse_stop_reason("tool_use"), StopReason::ToolUse));
        assert!(matches!(parse_stop_reason("aborted"), StopReason::Aborted));
        assert!(matches!(parse_stop_reason("unknown"), StopReason::Error));
    }

    #[test]
    fn test_toolcall_delta_accumulates_partial_json() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        process_proxy_event(
            &ProxyEvent::ToolcallStart {
                content_index: 0,
                id: "tc1".into(),
                tool_name: "bash".into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        process_proxy_event(
            &ProxyEvent::ToolcallDelta {
                content_index: 0,
                delta: r#"{"cmd":"#.into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        process_proxy_event(
            &ProxyEvent::ToolcallDelta {
                content_index: 0,
                delta: r#""ls"}"#.into(),
            },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        if let Some(Some(PartialContent::ToolCall { partial_json, .. })) = partial_content.get(0) {
            assert_eq!(partial_json, r#"{"cmd":"ls"}"#);
        } else {
            panic!("expected ToolCall content");
        }
    }

    #[test]
    fn test_multiple_content_blocks() {
        let model = test_model();
        let mut partial_content = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = StopReason::Stop;
        let mut error_message = None;

        // Block 0: thinking
        process_proxy_event(
            &ProxyEvent::ThinkingStart { content_index: 0 },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        // Block 1: text
        process_proxy_event(
            &ProxyEvent::TextStart { content_index: 1 },
            &mut partial_content,
            &mut usage,
            &mut stop_reason,
            &mut error_message,
            &model,
        );

        assert_eq!(partial_content.len(), 2);
        assert!(matches!(
            partial_content[0],
            Some(PartialContent::Thinking { .. })
        ));
        assert!(matches!(
            partial_content[1],
            Some(PartialContent::Text { .. })
        ));
    }
}
