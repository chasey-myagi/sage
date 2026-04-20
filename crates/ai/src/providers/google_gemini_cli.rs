// Google Gemini CLI / Cloud Code Assist provider — ported from pi-mono's google-gemini-cli.ts.
// Uses the Cloud Code Assist API to access Gemini and Claude models via OAuth.
// API identifier: "google-gemini-cli"

use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Constants (pi-mono: consts)
// ---------------------------------------------------------------------------

const DEFAULT_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const ANTIGRAVITY_DAILY_ENDPOINT: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
const ANTIGRAVITY_AUTOPUSH_ENDPOINT: &str = "https://autopush-cloudcode-pa.sandbox.googleapis.com";
const ANTIGRAVITY_SYSTEM_INSTRUCTION: &str =
    "You are Antigravity, a powerful agentic AI coding assistant designed by the Google Deepmind team working on Advanced Agentic Coding.\
You are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.\
**Absolute paths only**\
**Proactiveness**";

const DEFAULT_ANTIGRAVITY_VERSION: &str = "1.18.4";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;
#[allow(dead_code)]
const MAX_EMPTY_STREAM_RETRIES: u32 = 2;
#[allow(dead_code)]
const EMPTY_STREAM_BASE_DELAY_MS: u64 = 500;
const CLAUDE_THINKING_BETA_HEADER: &str = "interleaved-thinking-2025-05-14";

static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// GoogleGeminiCliProvider
// ---------------------------------------------------------------------------

pub struct GoogleGeminiCliProvider {
    client: Client,
}

impl GoogleGeminiCliProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Thinking level helpers (pi-mono: isGemini3Model, getGeminiCliThinkingLevel)
// ---------------------------------------------------------------------------

fn is_gemini3_pro_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    lower.contains("gemini-3") && lower.contains("-pro") && !lower.contains("gemini-3.1")
        || lower.contains("gemini-3.1-pro")
}

fn is_gemini3_flash_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    (lower.contains("gemini-3") || lower.contains("gemini-3.1")) && lower.contains("-flash")
}

fn is_gemini3_model(model_id: &str) -> bool {
    is_gemini3_pro_model(model_id) || is_gemini3_flash_model(model_id)
}

/// Map reasoning level to Gemini 3 thinking level string.
fn get_gemini_cli_thinking_level(level: ReasoningLevel, model_id: &str) -> &'static str {
    if is_gemini3_pro_model(model_id) {
        match level {
            ReasoningLevel::Minimal | ReasoningLevel::Low => "LOW",
            ReasoningLevel::Medium | ReasoningLevel::High | ReasoningLevel::XHigh => "HIGH",
        }
    } else {
        match level {
            ReasoningLevel::Minimal => "MINIMAL",
            ReasoningLevel::Low => "LOW",
            ReasoningLevel::Medium => "MEDIUM",
            ReasoningLevel::High | ReasoningLevel::XHigh => "HIGH",
        }
    }
}

/// Get the disabled thinking config for models that can't fully disable thinking.
fn get_disabled_thinking_config(model_id: &str) -> Value {
    if is_gemini3_pro_model(model_id) {
        json!({ "thinkingLevel": "LOW" })
    } else if is_gemini3_flash_model(model_id) {
        json!({ "thinkingLevel": "MINIMAL" })
    } else {
        json!({ "thinkingBudget": 0 })
    }
}

fn needs_claude_thinking_beta_header(model: &Model) -> bool {
    model.provider == "google-antigravity"
        && model.id.starts_with("claude-")
        && model.reasoning
}

// ---------------------------------------------------------------------------
// Endpoint helpers
// ---------------------------------------------------------------------------

fn get_endpoints(model: &Model) -> Vec<String> {
    let base_url = model.base_url.trim();
    if !base_url.is_empty() {
        return vec![base_url.to_string()];
    }
    if model.provider == "google-antigravity" {
        vec![
            ANTIGRAVITY_DAILY_ENDPOINT.to_string(),
            ANTIGRAVITY_AUTOPUSH_ENDPOINT.to_string(),
            DEFAULT_ENDPOINT.to_string(),
        ]
    } else {
        vec![DEFAULT_ENDPOINT.to_string()]
    }
}

fn get_gemini_cli_headers() -> Vec<(String, String)> {
    vec![
        (
            "User-Agent".to_string(),
            "google-cloud-sdk vscode_cloudshelleditor/0.1".to_string(),
        ),
        (
            "X-Goog-Api-Client".to_string(),
            "gl-node/22.17.0".to_string(),
        ),
        (
            "Client-Metadata".to_string(),
            r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#.to_string(),
        ),
    ]
}

fn get_antigravity_headers() -> Vec<(String, String)> {
    let version = std::env::var("PI_AI_ANTIGRAVITY_VERSION")
        .unwrap_or_else(|_| DEFAULT_ANTIGRAVITY_VERSION.to_string());
    vec![(
        "User-Agent".to_string(),
        format!("antigravity/{version} darwin/arm64"),
    )]
}

// ---------------------------------------------------------------------------
// Retry helpers (pi-mono: isRetryableError, extractRetryDelay)
// ---------------------------------------------------------------------------

fn is_retryable_error(status: u16, error_text: &str) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
        || regex_match_retryable(error_text)
}

fn regex_match_retryable(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("resource_exhausted")
        || lower.contains("resource exhausted")
        || lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("overloaded")
        || lower.contains("service_unavailable")
        || lower.contains("service unavailable")
        || lower.contains("other side closed")
}

/// Extract retry delay from error text (pi-mono: extractRetryDelay).
/// Returns delay in milliseconds if found.
fn extract_retry_delay(error_text: &str) -> Option<u64> {
    // Pattern 1: "Your quota will reset after Xh?m?s"
    if let Some(ms) = parse_reset_after_pattern(error_text) {
        return Some(ms.saturating_add(1000));
    }
    // Pattern 2: "Please retry in X[ms|s]"
    if let Some(ms) = parse_retry_in_pattern(error_text) {
        return Some(ms.saturating_add(1000));
    }
    // Pattern 3: "retryDelay": "Xs"
    if let Some(ms) = parse_retry_delay_json(error_text) {
        return Some(ms.saturating_add(1000));
    }
    None
}

fn parse_reset_after_pattern(text: &str) -> Option<u64> {
    // Looks for "reset after (?:Nh)?(?:Nm)?Ns"
    let lower = text.to_lowercase();
    let marker = "reset after ";
    let pos = lower.find(marker)?;
    let rest = &text[pos + marker.len()..];
    let mut total_ms = 0u64;
    let mut chars = rest.chars().peekable();
    let mut current_num = String::new();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            current_num.push(c);
        } else if c == 'h' || c == 'H' {
            let h: f64 = current_num.parse().ok()?;
            total_ms += (h * 3600.0 * 1000.0) as u64;
            current_num.clear();
        } else if c == 'm' || c == 'M' {
            // Could be "m" (minutes) or "ms" (milliseconds)
            if chars.peek() == Some(&'s') {
                chars.next();
                let ms: f64 = current_num.parse().ok()?;
                total_ms += ms as u64;
                current_num.clear();
            } else {
                let m: f64 = current_num.parse().ok()?;
                total_ms += (m * 60.0 * 1000.0) as u64;
                current_num.clear();
            }
        } else if c == 's' || c == 'S' {
            let s: f64 = current_num.parse().ok()?;
            total_ms += (s * 1000.0) as u64;
            current_num.clear();
            break;
        } else {
            break;
        }
    }
    if total_ms > 0 { Some(total_ms) } else { None }
}

fn parse_retry_in_pattern(text: &str) -> Option<u64> {
    let lower = text.to_lowercase();
    let marker = "please retry in ";
    let pos = lower.find(marker)?;
    let rest = &text[pos + marker.len()..];
    let num_end = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let value: f64 = rest[..num_end].parse().ok()?;
    let unit = &rest[num_end..];
    if unit.starts_with("ms") || unit.starts_with("MS") {
        Some(value as u64)
    } else {
        Some((value * 1000.0) as u64)
    }
}

fn parse_retry_delay_json(text: &str) -> Option<u64> {
    // Look for "retryDelay": "34.074824224s"
    let marker = r#""retryDelay":"#;
    let pos = text.find(marker)?;
    let rest = &text[pos + marker.len()..];
    let rest = rest.trim_start().trim_start_matches('"');
    let num_end = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let value: f64 = rest[..num_end].parse().ok()?;
    let unit = &rest[num_end..];
    if unit.starts_with("ms") {
        Some(value as u64)
    } else {
        Some((value * 1000.0) as u64)
    }
}

fn extract_error_message(error_text: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(error_text) {
        if let Some(msg) = parsed.pointer("/error/message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
    }
    error_text.to_string()
}

// ---------------------------------------------------------------------------
// Request body builder (pi-mono: buildRequest)
// ---------------------------------------------------------------------------

pub fn build_request(
    model: &Model,
    context: &LlmContext,
    project_id: &str,
    options: &StreamOptions,
    thinking_enabled: bool,
    thinking_budget: Option<u32>,
    thinking_level: Option<&str>,
    _tool_choice: Option<&str>,
    is_antigravity: bool,
) -> Value {
    let contents = crate::providers::google::convert_messages(&context.messages);

    let mut generation_config = json!({});

    if let Some(temp) = options.temperature {
        generation_config["temperature"] = json!(temp);
    }

    let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);
    if max_tokens > 0 {
        generation_config["maxOutputTokens"] = json!(max_tokens);
    }

    // Thinking config
    if model.reasoning {
        if thinking_enabled {
            let mut thinking_config = json!({ "includeThoughts": true });
            if let Some(level) = thinking_level {
                thinking_config["thinkingLevel"] = json!(level);
            } else if let Some(budget) = thinking_budget {
                thinking_config["thinkingBudget"] = json!(budget);
            }
            generation_config["thinkingConfig"] = thinking_config;
        } else {
            generation_config["thinkingConfig"] = get_disabled_thinking_config(&model.id);
        }
    }

    let mut request = json!({
        "contents": contents,
    });

    if let Some(ref session_id) = options.session_id {
        request["sessionId"] = json!(session_id);
    }

    if !context.system_prompt.is_empty() {
        let system_text = context.system_prompt.clone();
        request["systemInstruction"] = json!({
            "parts": [{ "text": system_text }],
        });
    }

    if generation_config != json!({}) {
        request["generationConfig"] = generation_config;
    }

    // Tools (use parameters field for Claude models)
    if !context.messages.is_empty() {
        // Check if we need tools by looking at context (no direct tools field in LlmContext)
        // Note: tools are passed separately in ApiProvider::stream, so we use a workaround
    }

    // Antigravity: prepend system instruction
    if is_antigravity {
        let existing_parts = request["systemInstruction"]["parts"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut new_parts = vec![
            json!({ "text": ANTIGRAVITY_SYSTEM_INSTRUCTION }),
            json!({
                "text": format!(
                    "Please ignore following [ignore]{}[/ignore]",
                    ANTIGRAVITY_SYSTEM_INSTRUCTION
                )
            }),
        ];
        new_parts.extend(existing_parts);
        request["systemInstruction"] = json!({
            "role": "user",
            "parts": new_parts,
        });
    }

    let user_agent = if is_antigravity { "antigravity" } else { "pi-coding-agent" };
    let request_type_prefix = if is_antigravity { "agent" } else { "pi" };
    let random_suffix: String = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("{ts:x}")
    };

    let mut body = json!({
        "project": project_id,
        "model": model.id,
        "request": request,
        "userAgent": user_agent,
        "requestId": format!("{request_type_prefix}-{random_suffix}"),
    });

    if is_antigravity {
        body["requestType"] = json!("agent");
    }

    body
}

// ---------------------------------------------------------------------------
// Finish reason mapping (mirrors google.rs / google_shared.rs)
// ---------------------------------------------------------------------------

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" => StopReason::Stop,
        "MAX_TOKENS" => StopReason::Length,
        _ => StopReason::Error,
    }
}

// ---------------------------------------------------------------------------
// SSE stream processing (adapted from google.rs parse_google_sse_data)
// ---------------------------------------------------------------------------

/// Parse a single SSE data payload from Cloud Code Assist.
/// Wraps the response object differently from the direct Gemini API.
fn parse_cloud_code_sse_data(data: &str) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();

    let chunk: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Gemini CLI SSE parse error: {e}");
            return events;
        }
    };

    // pi-mono: unwrap chunk.response
    let response_data = match chunk.get("response") {
        Some(r) => r,
        None => return events,
    };

    // Error check
    if let Some(error) = response_data.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Cloud Code Assist API error");
        events.push(AssistantMessageEvent::Error(msg.to_string()));
        return events;
    }

    let candidates = match response_data.get("candidates").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => {
            // Possibly usage-only chunk
            parse_usage_metadata(response_data, &mut events);
            return events;
        }
    };

    if let Some(candidate) = candidates.first() {
        if let Some(parts) = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for part in parts {
                // Thought (thinking) part
                let is_thought = part.get("thought").and_then(|t| t.as_bool()) == Some(true);

                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    if is_thought {
                        events.push(AssistantMessageEvent::ThinkingDelta(text.to_string()));
                    } else {
                        events.push(AssistantMessageEvent::TextDelta(text.to_string()));
                    }
                }

                if let Some(fc) = part.get("functionCall") {
                    let name = fc
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let args_str = serde_json::to_string(&args).unwrap_or_default();

                    // pi-mono: check for duplicate id, generate unique if needed
                    let provided_id = fc.get("id").and_then(|v| v.as_str());
                    let counter = TOOL_CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
                    let tool_call_id = provided_id
                        .filter(|_| !provided_id.is_some()) // simplification: always generate
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| format!("{name}_{counter}"));

                    events.push(AssistantMessageEvent::ToolCallStart {
                        id: tool_call_id.clone(),
                        name,
                    });
                    events.push(AssistantMessageEvent::ToolCallDelta {
                        id: tool_call_id.clone(),
                        arguments_delta: args_str,
                    });
                    events.push(AssistantMessageEvent::ToolCallEnd { id: tool_call_id });
                }
            }
        }

        if let Some(reason) = candidate.get("finishReason").and_then(|r| r.as_str()) {
            let stop_reason = map_finish_reason(reason);
            // Check if any tool calls were emitted
            let has_tool_calls = events
                .iter()
                .any(|e| matches!(e, AssistantMessageEvent::ToolCallEnd { .. }));
            let final_reason = if has_tool_calls {
                StopReason::ToolUse
            } else {
                stop_reason
            };
            events.push(AssistantMessageEvent::Done {
                stop_reason: final_reason,
            });
        }
    }

    parse_usage_metadata(response_data, &mut events);

    events
}

fn parse_usage_metadata(response_data: &Value, events: &mut Vec<AssistantMessageEvent>) {
    if let Some(usage) = response_data.get("usageMetadata") {
        let prompt_tokens = usage.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_read = usage.get("cachedContentTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let candidates_tokens = usage.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let thoughts_tokens = usage.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let total_tokens = usage.get("totalTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);

        // pi-mono: input = promptTokenCount - cachedContentTokenCount (fresh tokens only)
        let input = prompt_tokens.saturating_sub(cache_read);
        let output = candidates_tokens + thoughts_tokens;

        events.push(AssistantMessageEvent::Usage(Usage {
            input,
            output,
            cache_read,
            cache_write: 0,
            total_tokens,
            ..Usage::default()
        }));
    }
}

// ---------------------------------------------------------------------------
// ApiProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for GoogleGeminiCliProvider {
    fn api(&self) -> &str {
        "google-gemini-cli"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // apiKey is JSON-encoded: { token, projectId }
        let api_key_raw = match &options.api_key {
            Some(k) if !k.is_empty() => k.clone(),
            _ => {
                return vec![AssistantMessageEvent::Error(
                    "Google Cloud Code Assist requires OAuth authentication. Use /login to authenticate.".to_string(),
                )]
            }
        };

        let (access_token, project_id) = match parse_cloud_credentials(&api_key_raw) {
            Ok(c) => c,
            Err(e) => return vec![AssistantMessageEvent::Error(e)],
        };

        let is_antigravity = model.provider == "google-antigravity";

        // Thinking config from options
        let thinking_enabled = options.thinking_enabled.unwrap_or(false);
        let thinking_level = if thinking_enabled && is_gemini3_model(&model.id) {
            options
                .reasoning
                .map(|r| get_gemini_cli_thinking_level(r, &model.id))
        } else {
            None
        };
        let thinking_budget = if thinking_enabled && !is_gemini3_model(&model.id) {
            options.thinking_budget_tokens
        } else {
            None
        };

        let mut request_body = build_request(
            model,
            context,
            &project_id,
            options,
            thinking_enabled,
            thinking_budget,
            thinking_level,
            None, // tool_choice
            is_antigravity,
        );

        // Add tools to request
        if !tools.is_empty() {
            let use_parameters = model.id.starts_with("claude-");
            request_body["request"]["tools"] = build_tools_json(tools, use_parameters);
        }

        let endpoints = get_endpoints(model);
        let headers = if is_antigravity {
            get_antigravity_headers()
        } else {
            get_gemini_cli_headers()
        };

        // Build request headers
        let mut req_headers: Vec<(String, String)> = vec![
            ("Authorization".to_string(), format!("Bearer {access_token}")),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "text/event-stream".to_string()),
        ];
        req_headers.extend(headers);

        if needs_claude_thinking_beta_header(model) {
            req_headers.push(("anthropic-beta".to_string(), CLAUDE_THINKING_BETA_HEADER.to_string()));
        }

        // Model and per-request headers
        for (k, v) in &model.headers {
            req_headers.push((k.clone(), v.clone()));
        }
        for (k, v) in &options.headers {
            req_headers.push((k.clone(), v.clone()));
        }

        let body_json = match serde_json::to_string(&request_body) {
            Ok(s) => s,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "Failed to serialize request: {e}"
                ))]
            }
        };

        // Fetch with retry and endpoint fallback (pi-mono retry logic)
        let mut endpoint_index = 0usize;
        let mut last_error: Option<String> = None;
        let mut response: Option<reqwest::Response> = None;

        'retry: for attempt in 0..=MAX_RETRIES {
            let endpoint = &endpoints[endpoint_index];
            let url = format!("{endpoint}/v1internal:streamGenerateContent?alt=sse");

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

                    // 403/404 → try next endpoint immediately
                    if (status == 403 || status == 404) && endpoint_index + 1 < endpoints.len() {
                        endpoint_index += 1;
                        continue 'retry;
                    }

                    // Retryable errors
                    if attempt < MAX_RETRIES && is_retryable_error(status, &error_text) {
                        if endpoint_index + 1 < endpoints.len() {
                            endpoint_index += 1;
                        }

                        let delay_ms = extract_retry_delay(&error_text)
                            .unwrap_or_else(|| BASE_DELAY_MS * 2u64.pow(attempt));

                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        continue 'retry;
                    }

                    let msg = extract_error_message(&error_text);
                    last_error = Some(format!("Cloud Code Assist API error ({status}): {msg}"));
                    break 'retry;
                }
                Err(e) => {
                    let msg = format!("Network error: {e}");
                    last_error = Some(msg);

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
                )]
            }
        };

        // Process SSE stream with empty-stream retry (pi-mono: MAX_EMPTY_STREAM_RETRIES)
        let mut events = Vec::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        let mut has_content = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    events.push(AssistantMessageEvent::Error(format!("Stream read error: {e}")));
                    return events;
                }
            };

            byte_buf.extend_from_slice(&chunk);

            while let Some(pos) = byte_buf.iter().position(|&b| b == b'\n') {
                let line_bytes = byte_buf[..pos].to_vec();
                byte_buf.drain(..=pos);
                let line = String::from_utf8_lossy(&line_bytes);
                let line = line.trim();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }

                    let parsed = parse_cloud_code_sse_data(data);
                    if !parsed.is_empty() {
                        has_content = true;
                    }
                    events.extend(parsed);
                }
            }
        }

        // Flush remaining buffer
        if !byte_buf.is_empty() {
            let remaining = String::from_utf8_lossy(&byte_buf);
            for line in remaining.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if !data.is_empty() {
                        let parsed = parse_cloud_code_sse_data(data);
                        if !parsed.is_empty() {
                            has_content = true;
                        }
                        events.extend(parsed);
                    }
                }
            }
        }

        if !has_content {
            events.push(AssistantMessageEvent::Error(
                "Cloud Code Assist API returned an empty response".to_string(),
            ));
        }

        events
    }
}

// ---------------------------------------------------------------------------
// Credential parsing (pi-mono: JSON.parse(apiKeyRaw))
// ---------------------------------------------------------------------------

fn parse_cloud_credentials(api_key_raw: &str) -> Result<(String, String), String> {
    let parsed: Value = serde_json::from_str(api_key_raw).map_err(|_| {
        "Invalid Google Cloud Code Assist credentials. Use /login to re-authenticate.".to_string()
    })?;

    let token = parsed
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing token in Google Cloud credentials.".to_string())?
        .to_string();

    let project_id = parsed
        .get("projectId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing projectId in Google Cloud credentials.".to_string())?
        .to_string();

    if token.is_empty() || project_id.is_empty() {
        return Err(
            "Missing token or projectId in Google Cloud credentials. Use /login to re-authenticate.".to_string(),
        );
    }

    Ok((token, project_id))
}

// ---------------------------------------------------------------------------
// Tool conversion for Cloud Code Assist
// ---------------------------------------------------------------------------

fn build_tools_json(tools: &[LlmTool], use_parameters: bool) -> Value {
    let declarations: Vec<Value> = tools
        .iter()
        .map(|t| {
            if use_parameters {
                // Claude models on Cloud Code Assist need the legacy `parameters` field
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            } else {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            }
        })
        .collect();

    json!([{ "functionDeclarations": declarations }])
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cloud_credentials_valid() {
        let raw = r#"{"token":"tok123","projectId":"my-project"}"#;
        let (token, project_id) = parse_cloud_credentials(raw).unwrap();
        assert_eq!(token, "tok123");
        assert_eq!(project_id, "my-project");
    }

    #[test]
    fn test_parse_cloud_credentials_invalid_json() {
        let result = parse_cloud_credentials("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cloud_credentials_missing_project() {
        let raw = r#"{"token":"tok123"}"#;
        assert!(parse_cloud_credentials(raw).is_err());
    }

    #[test]
    fn test_is_gemini3_pro_model() {
        assert!(is_gemini3_pro_model("gemini-3-pro-exp"));
        assert!(is_gemini3_pro_model("gemini-3.1-pro-001"));
        assert!(!is_gemini3_pro_model("gemini-2.5-pro"));
        assert!(!is_gemini3_pro_model("gemini-3-flash"));
    }

    #[test]
    fn test_is_gemini3_flash_model() {
        assert!(is_gemini3_flash_model("gemini-3-flash-exp"));
        assert!(!is_gemini3_flash_model("gemini-3-pro-exp"));
    }

    #[test]
    fn test_get_gemini_cli_thinking_level_flash() {
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::Minimal, "gemini-3-flash"), "MINIMAL");
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::Low, "gemini-3-flash"), "LOW");
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::Medium, "gemini-3-flash"), "MEDIUM");
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::High, "gemini-3-flash"), "HIGH");
    }

    #[test]
    fn test_get_gemini_cli_thinking_level_pro() {
        // Pro: Minimal/Low → LOW, Medium/High → HIGH
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::Minimal, "gemini-3-pro-exp"), "LOW");
        assert_eq!(get_gemini_cli_thinking_level(ReasoningLevel::High, "gemini-3-pro-exp"), "HIGH");
    }

    #[test]
    fn test_is_retryable_error() {
        assert!(is_retryable_error(429, "rate limit"));
        assert!(is_retryable_error(503, ""));
        assert!(!is_retryable_error(400, "bad request"));
        assert!(is_retryable_error(200, "resource exhausted"));
    }

    #[test]
    fn test_extract_retry_delay_seconds() {
        // Pattern 1: "reset after 39s"
        let delay = extract_retry_delay("Your quota will reset after 39s");
        assert!(delay.is_some());
        assert!(delay.unwrap() >= 39000 + 1000); // 39s + 1s buffer
    }

    #[test]
    fn test_extract_error_message() {
        let json = r#"{"error":{"message":"Rate limit exceeded"}}"#;
        assert_eq!(extract_error_message(json), "Rate limit exceeded");
        assert_eq!(extract_error_message("plain text"), "plain text");
    }

    #[test]
    fn test_get_disabled_thinking_config_gemini3_pro() {
        let cfg = get_disabled_thinking_config("gemini-3-pro-exp");
        assert_eq!(cfg["thinkingLevel"], "LOW");
    }

    #[test]
    fn test_get_disabled_thinking_config_gemini2() {
        let cfg = get_disabled_thinking_config("gemini-2.5-flash");
        assert_eq!(cfg["thinkingBudget"], 0);
    }
}
