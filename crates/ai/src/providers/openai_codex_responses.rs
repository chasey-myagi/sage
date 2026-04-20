// OpenAI Codex Responses provider — ported from pi-mono's openai-codex-responses.ts.
// Uses the ChatGPT backend Codex endpoint with JWT authentication.
// API identifier: "openai-codex-responses"

use super::openai_responses_shared::{
    StreamState, convert_messages, convert_tools, process_responses_event,
};
use crate::keys;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Constants (pi-mono)
// ---------------------------------------------------------------------------

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;

// ---------------------------------------------------------------------------
// OpenAiCodexResponsesProvider
// ---------------------------------------------------------------------------

pub struct OpenAiCodexResponsesProvider {
    client: Client,
}

impl Default for OpenAiCodexResponsesProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiCodexResponsesProvider {
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
impl ApiProvider for OpenAiCodexResponsesProvider {
    fn api(&self) -> &str {
        "openai-codex-responses"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key (JWT token)
        let api_key = match &options.api_key {
            Some(k) if !k.is_empty() => k.clone(),
            _ => match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(k) => k,
                Err(e) => return vec![AssistantMessageEvent::Error(format!("API key error: {e}"))],
            },
        };

        // Extract accountId from JWT
        let account_id = match extract_account_id(&api_key) {
            Ok(id) => id,
            Err(e) => return vec![AssistantMessageEvent::Error(e)],
        };

        let base_url_opt = if model.base_url.is_empty() {
            None
        } else {
            Some(model.base_url.as_str())
        };
        let url = resolve_codex_url(base_url_opt);
        let body = build_request_body(model, context, tools, options);
        let body_json = match serde_json::to_string(&body) {
            Ok(s) => s,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "Serialization error: {e}"
                ))];
            }
        };

        // Build headers (pi-mono: buildSSEHeaders)
        let req_headers = build_sse_headers(
            &model.headers,
            &options.headers,
            &account_id,
            &api_key,
            options.session_id.as_deref(),
        );

        // Fetch with retry
        let mut response: Option<reqwest::Response> = None;
        let mut last_error: Option<String> = None;

        'retry: for attempt in 0..=MAX_RETRIES {
            let mut req = self.client.post(&url).body(body_json.clone());

            for (k, v) in &req_headers {
                req = req.header(k.as_str(), v.as_str());
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        response = Some(resp);
                        break 'retry;
                    }

                    let status = resp.status().as_u16();
                    let error_text = resp.text().await.unwrap_or_default();

                    if attempt < MAX_RETRIES && is_retryable_error(status, &error_text) {
                        let delay_ms = BASE_DELAY_MS * 2u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        continue 'retry;
                    }

                    last_error = Some(parse_error_response(status, &error_text));
                    break 'retry;
                }
                Err(e) => {
                    let msg = format!("{e}");
                    last_error = Some(msg.clone());
                    if msg.contains("usage limit") {
                        break 'retry;
                    }
                    if attempt < MAX_RETRIES {
                        let delay_ms = BASE_DELAY_MS * 2u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        continue 'retry;
                    }
                    break 'retry;
                }
            }
        }

        let response = match response {
            Some(r) => r,
            None => {
                return vec![AssistantMessageEvent::Error(
                    last_error.unwrap_or_else(|| "Failed after retries".to_string()),
                )];
            }
        };

        // Parse SSE stream with Codex event mapping
        let mut events = Vec::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        let mut state = StreamState::default();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    events.push(AssistantMessageEvent::Error(format!(
                        "Stream read error: {e}"
                    )));
                    return events;
                }
            };

            byte_buf.extend_from_slice(&chunk);

            while let Some(boundary) = find_event_boundary(&byte_buf) {
                let event_bytes = byte_buf[..boundary].to_vec();
                byte_buf.drain(..boundary + 2);

                let event_str = String::from_utf8_lossy(&event_bytes);
                process_sse_event_block(&event_str, &mut events, &mut state);
            }
        }

        if !byte_buf.is_empty() {
            let remaining = String::from_utf8_lossy(&byte_buf);
            process_sse_event_block(&remaining, &mut events, &mut state);
        }

        events
    }
}

// ---------------------------------------------------------------------------
// URL resolution (pi-mono: resolveCodexUrl)
// ---------------------------------------------------------------------------

/// Resolve the Codex SSE endpoint URL.
pub fn resolve_codex_url(base_url: Option<&str>) -> String {
    let raw = base_url.unwrap_or(DEFAULT_CODEX_BASE_URL);
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

// ---------------------------------------------------------------------------
// Request body builder (pi-mono: buildRequestBody)
// ---------------------------------------------------------------------------

fn build_request_body(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &StreamOptions,
) -> Value {
    let messages = convert_messages(&context.messages);

    let mut body = json!({
        "model": model.id,
        "store": false,
        "stream": true,
        "instructions": context.system_prompt,
        "input": messages,
        "text": { "verbosity": "medium" },
        "include": ["reasoning.encrypted_content"],
        "tool_choice": "auto",
        "parallel_tool_calls": true,
    });

    if let Some(ref session_id) = options.session_id {
        body["prompt_cache_key"] = json!(session_id);
    }

    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }

    // Codex: strict: null → omit the field entirely
    if !tools.is_empty() {
        body["tools"] = json!(convert_tools(tools, None));
    }

    // Reasoning effort (pi-mono: reasoningEffort)
    if let Some(ref effort) = options.reasoning {
        let effort_str = clamp_reasoning_effort(&model.id, effort);
        body["reasoning"] = json!({
            "effort": effort_str,
            "summary": "auto",
        });
    }

    body
}

/// Map ReasoningLevel to Codex effort string, with model-specific clamping.
/// pi-mono: clampReasoningEffort
fn clamp_reasoning_effort(model_id: &str, level: &ReasoningLevel) -> &'static str {
    let id = if model_id.contains('/') {
        model_id.split('/').next_back().unwrap_or(model_id)
    } else {
        model_id
    };

    let is_high_end =
        id.starts_with("gpt-5.2") || id.starts_with("gpt-5.3") || id.starts_with("gpt-5.4");

    match level {
        ReasoningLevel::Minimal => {
            if is_high_end {
                "low"
            } else {
                "minimal"
            }
        }
        ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh => {
            if id == "gpt-5.1" || id == "gpt-5.1-codex-mini" {
                "high"
            } else {
                "xhigh"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error handling (pi-mono: isRetryableError, parseErrorResponse)
// ---------------------------------------------------------------------------

fn is_retryable_error(status: u16, error_text: &str) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504) || {
        let lower = error_text.to_lowercase();
        lower.contains("rate_limit")
            || lower.contains("rate limit")
            || lower.contains("overloaded")
            || lower.contains("service_unavailable")
            || lower.contains("service unavailable")
            || lower.contains("upstream connect")
            || lower.contains("connection refused")
    }
}

fn parse_error_response(status: u16, raw: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw)
        && let Some(err) = parsed.get("error")
    {
        let code = err.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let err_type = err.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let code_or_type = if !code.is_empty() { code } else { err_type };

        if code_or_type.contains("usage_limit_reached")
            || code_or_type.contains("usage_not_included")
            || code_or_type.contains("rate_limit_exceeded")
            || status == 429
        {
            let plan = err
                .get("plan_type")
                .and_then(|v| v.as_str())
                .map(|p| format!(" ({} plan)", p.to_lowercase()))
                .unwrap_or_default();
            let resets_at = err.get("resets_at").and_then(|v| v.as_u64());
            let when = if let Some(ts) = resets_at {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let mins = ts.saturating_sub(now_secs) / 60;
                format!(" Try again in ~{mins} min.")
            } else {
                String::new()
            };
            return format!("You have hit your ChatGPT usage limit{plan}.{when}")
                .trim()
                .to_string();
        }

        if let Some(msg) = err.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
    }
    if !raw.is_empty() {
        raw.to_string()
    } else {
        format!("Request failed with status {status}")
    }
}

// ---------------------------------------------------------------------------
// Auth & Headers (pi-mono: buildSSEHeaders, extractAccountId)
// ---------------------------------------------------------------------------

/// Extract chatgpt_account_id from a JWT token.
/// pi-mono: extractAccountId
pub fn extract_account_id(token: &str) -> Result<String, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Failed to extract accountId from token: invalid JWT format".to_string());
    }

    let payload_b64 = parts[1];
    let padded = match payload_b64.len() % 4 {
        0 => payload_b64.to_string(),
        2 => format!("{payload_b64}=="),
        3 => format!("{payload_b64}="),
        _ => return Err("Failed to extract accountId from token: invalid base64".to_string()),
    };
    let standard = padded.replace('-', "+").replace('_', "/");

    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&standard)
        .map_err(|e| format!("Failed to extract accountId from token: {e}"))?;

    let payload: Value = serde_json::from_slice(&decoded)
        .map_err(|e| format!("Failed to extract accountId from token: {e}"))?;

    let account_id = payload
        .get(JWT_CLAIM_PATH)
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Failed to extract accountId from token: no account ID found".to_string())?
        .to_string();

    if account_id.is_empty() {
        return Err("Failed to extract accountId from token: empty account ID".to_string());
    }

    Ok(account_id)
}

fn build_sse_headers(
    model_headers: &[(String, String)],
    option_headers: &[(String, String)],
    account_id: &str,
    token: &str,
    session_id: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Authorization".to_string(), format!("Bearer {token}")),
        ("chatgpt-account-id".to_string(), account_id.to_string()),
        ("originator".to_string(), "pi".to_string()),
        ("User-Agent".to_string(), "pi".to_string()),
        (
            "OpenAI-Beta".to_string(),
            "responses=experimental".to_string(),
        ),
        ("accept".to_string(), "text/event-stream".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
    ];

    for (k, v) in model_headers {
        headers.push((k.clone(), v.clone()));
    }
    for (k, v) in option_headers {
        headers.push((k.clone(), v.clone()));
    }

    if let Some(sid) = session_id {
        headers.push(("session_id".to_string(), sid.to_string()));
    }

    headers
}

// ---------------------------------------------------------------------------
// SSE parsing (pi-mono: parseSSE + mapCodexEvents)
// ---------------------------------------------------------------------------

/// Find the position of `"\n\n"` event boundary in bytes.
fn find_event_boundary(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Process a complete SSE event block (one or more `"data: ..."` lines).
fn process_sse_event_block(
    block: &str,
    events: &mut Vec<AssistantMessageEvent>,
    state: &mut StreamState,
) {
    let data_lines: Vec<&str> = block
        .lines()
        .filter(|l| l.starts_with("data:"))
        .map(|l| l[5..].trim())
        .collect();

    if data_lines.is_empty() {
        return;
    }

    let data = data_lines.join("\n").trim().to_string();
    if data.is_empty() || data == "[DONE]" {
        return;
    }

    let json: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Codex SSE parse error: {e}");
            return;
        }
    };

    let event_type = match json.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };

    // pi-mono: mapCodexEvents — translate Codex event types to standard Responses API events
    if let Some(mapped) = map_codex_event(event_type) {
        process_responses_event(mapped, &json, state, events);
    }
}

/// Map Codex-specific event types to standard Responses API event types.
/// pi-mono: mapCodexEvents
fn map_codex_event(event_type: &str) -> Option<&'static str> {
    match event_type {
        "error" => Some("error"),
        "response.failed" => Some("response.failed"),
        "response.done" | "response.completed" | "response.incomplete" => {
            Some("response.completed")
        }
        "response.output_item.added" => Some("response.output_item.added"),
        "response.output_item.done" => Some("response.output_item.done"),
        "response.output_text.delta" => Some("response.output_text.delta"),
        "response.reasoning_summary_text.delta" => Some("response.reasoning_summary_text.delta"),
        "response.function_call_arguments.delta" => Some("response.function_call_arguments.delta"),
        _ => None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_codex_url_default() {
        let url = resolve_codex_url(None);
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
    }

    #[test]
    fn test_resolve_codex_url_custom_base() {
        let url = resolve_codex_url(Some("https://my.proxy.com"));
        assert_eq!(url, "https://my.proxy.com/codex/responses");
    }

    #[test]
    fn test_resolve_codex_url_already_has_path() {
        let url = resolve_codex_url(Some("https://x.com/codex/responses"));
        assert_eq!(url, "https://x.com/codex/responses");
    }

    #[test]
    fn test_resolve_codex_url_codex_only() {
        let url = resolve_codex_url(Some("https://x.com/codex"));
        assert_eq!(url, "https://x.com/codex/responses");
    }

    #[test]
    fn test_clamp_reasoning_effort_minimal_standard() {
        assert_eq!(
            clamp_reasoning_effort("o3", &ReasoningLevel::Minimal),
            "minimal"
        );
    }

    #[test]
    fn test_clamp_reasoning_effort_minimal_high_end() {
        assert_eq!(
            clamp_reasoning_effort("gpt-5.2-mini", &ReasoningLevel::Minimal),
            "low"
        );
    }

    #[test]
    fn test_clamp_reasoning_effort_xhigh_gpt51() {
        assert_eq!(
            clamp_reasoning_effort("gpt-5.1", &ReasoningLevel::XHigh),
            "high"
        );
    }

    #[test]
    fn test_clamp_reasoning_effort_xhigh_standard() {
        assert_eq!(
            clamp_reasoning_effort("o3-mini", &ReasoningLevel::XHigh),
            "xhigh"
        );
    }

    #[test]
    fn test_map_codex_event_pass_through() {
        assert_eq!(
            map_codex_event("response.output_text.delta"),
            Some("response.output_text.delta")
        );
    }

    #[test]
    fn test_map_codex_event_normalize_done() {
        assert_eq!(map_codex_event("response.done"), Some("response.completed"));
        assert_eq!(
            map_codex_event("response.incomplete"),
            Some("response.completed")
        );
    }

    #[test]
    fn test_map_codex_event_unknown() {
        assert_eq!(map_codex_event("response.in_progress"), None);
    }

    #[test]
    fn test_find_event_boundary() {
        let buf = b"data: hello\n\ndata: world";
        assert_eq!(find_event_boundary(buf), Some(11));
    }

    #[test]
    fn test_find_event_boundary_none() {
        let buf = b"data: hello\n";
        assert_eq!(find_event_boundary(buf), None);
    }
}
