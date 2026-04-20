// Anthropic Messages API provider — ported from pi-mono's anthropic.ts.
// Implements streaming via SSE against the Anthropic Messages API.

use crate::keys;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use crate::types::{StopReason, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    client: Client,
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ApiProvider trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for AnthropicProvider {
    fn api(&self) -> &str {
        "anthropic-messages"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key: prefer options override, then env var
        let api_key = match &options.api_key {
            Some(key) if !key.is_empty() => key.clone(),
            _ => match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(key) => key,
                Err(e) => {
                    return vec![AssistantMessageEvent::Error(format!("API key error: {e}"))];
                }
            },
        };

        let oauth = is_oauth_token(&api_key);
        let base_url = model.base_url.trim_end_matches('/');
        let url = format!("{base_url}/messages");
        let body = build_request_body(model, context, tools, options, oauth);

        // Build beta headers
        let betas = build_beta_headers(model, options, &api_key);
        let mut req = self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        if !betas.is_empty() {
            req = req.header("anthropic-beta", betas.join(","));
        }

        // Apply model-level and per-request headers
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
            return vec![crate::provider_errors::handle_error_response(response, model).await];
        }

        // Parse SSE stream with byte-buffer approach (same as openai_compat.rs)
        let mut events = Vec::new();
        // Emit stream-start event (pi-mono: { type: "start" })
        events.push(AssistantMessageEvent::Start);
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();

        // Track active content blocks for id propagation
        let mut active_blocks: Vec<BlockState> = Vec::new();

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

            while let Some(newline_pos) = byte_buf.iter().position(|&b| b == b'\n') {
                let line_bytes = byte_buf[..newline_pos].to_vec();
                byte_buf.drain(..=newline_pos);
                let line = String::from_utf8_lossy(&line_bytes);
                process_sse_line(&line, &mut events, &mut active_blocks);
            }
        }

        // Flush remaining data
        if !byte_buf.is_empty() {
            let remaining = String::from_utf8_lossy(&byte_buf);
            for line in remaining.lines() {
                process_sse_line(line, &mut events, &mut active_blocks);
            }
        }

        // OAuth post-processing: reverse-map CC tool names back to caller names.
        // The API returns CC canonical names (e.g. "Read") when using OAuth tokens;
        // we map them back to the caller's original tool names via fromClaudeCodeName.
        if oauth {
            for event in &mut events {
                if let AssistantMessageEvent::ToolCallStart { name, .. } = event {
                    *name = from_claude_code_name(name, tools).to_string();
                }
            }
        }

        events
    }
}

// ---------------------------------------------------------------------------
// Block tracking state for SSE parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum BlockType {
    Text,
    Thinking,
    /// Redacted thinking — opaque data that must be passed back to the API.
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        #[allow(dead_code)]
        name: String,
    },
}

#[derive(Debug, Clone)]
struct BlockState {
    index: u64,
    block_type: BlockType,
    /// Accumulated signature for thinking blocks (base64 chunks).
    signature: String,
}

// ---------------------------------------------------------------------------
// Claude Code tool name mapping (OAuth stealth mode)
// ---------------------------------------------------------------------------
//
// When using an OAuth token the Anthropic API routes requests through the
// Claude Code proxy. The proxy enforces CC-canonical tool casing for all
// built-in tools (e.g. "read_file" must be sent as "Read"). Responses from
// the API also use the CC canonical name, so we need the reverse mapping too.
//
// Source: pi-mono anthropic.ts — `claudeCodeTools`, `toClaudeCodeName`,
//         `fromClaudeCodeName`.

/// Claude Code 2.x canonical tool names (exact casing).
const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// Convert a tool name to its Claude Code canonical casing.
///
/// Matches case-insensitively; returns the original name unchanged if not
/// found in the Claude Code tool list.
fn to_claude_code_name(name: &str) -> String {
    let lower = name.to_lowercase();
    CLAUDE_CODE_TOOLS
        .iter()
        .copied()
        .find(|cc| cc.to_lowercase() == lower)
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Reverse-map a Claude Code canonical tool name back to the caller's name.
///
/// Searches `tools` case-insensitively. Falls back to `name` unchanged when
/// no match is found (non-CC built-ins are passed through).
fn from_claude_code_name<'a>(name: &'a str, tools: &'a [LlmTool]) -> &'a str {
    if tools.is_empty() {
        return name;
    }
    let lower = name.to_lowercase();
    tools
        .iter()
        .find(|t| t.name.to_lowercase() == lower)
        .map(|t| t.name.as_str())
        .unwrap_or(name)
}

// ---------------------------------------------------------------------------
// Thinking support helpers
// ---------------------------------------------------------------------------

/// Check if a model supports adaptive thinking (Opus 4.6 / Sonnet 4.6).
fn supports_adaptive_thinking(model_id: &str) -> bool {
    model_id.contains("opus-4-6")
        || model_id.contains("opus-4.6")
        || model_id.contains("sonnet-4-6")
        || model_id.contains("sonnet-4.6")
}

/// Check if model is Opus 4.6 (for xhigh → max effort mapping).
fn is_opus(model_id: &str) -> bool {
    model_id.contains("opus-4-6") || model_id.contains("opus-4.6")
}

/// Map a ReasoningLevel to Anthropic's effort string.
fn map_thinking_level_to_effort(level: &ReasoningLevel, model_id: &str) -> &'static str {
    match level {
        ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh => {
            if is_opus(model_id) {
                "max"
            } else {
                "high"
            }
        }
    }
}

/// Detect if an API key is an OAuth token (starts with "sk-ant-oat").
fn is_oauth_token(api_key: &str) -> bool {
    api_key.contains("sk-ant-oat")
}

// ---------------------------------------------------------------------------
// Request body construction
// ---------------------------------------------------------------------------

fn build_request_body(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &StreamOptions,
    is_oauth: bool,
) -> Value {
    let max_tokens = options.max_tokens.unwrap_or(context.max_tokens);

    let mut body = json!({
        "model": model.id,
        "max_tokens": max_tokens,
        "stream": true,
    });

    // System prompt — with cache_control ephemeral on blocks.
    // For OAuth tokens we MUST include the Claude Code identity block first
    // (mirrors pi-mono buildParams OAuth branch).
    let cache_cc = options
        .cache_retention
        .as_ref()
        .map(|r| build_cache_control(r, &model.base_url));

    if is_oauth {
        // Claude Code identity block (mandatory for OAuth)
        let mut cc_block = json!({
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
        });
        if let Some(ref cc) = cache_cc {
            cc_block["cache_control"] = cc.clone();
        }
        let mut system_blocks = vec![cc_block];

        // Append caller's system prompt as a second block if provided
        if !context.system_prompt.is_empty() {
            let mut user_block = json!({
                "type": "text",
                "text": &context.system_prompt,
            });
            if let Some(ref cc) = cache_cc {
                user_block["cache_control"] = cc.clone();
            }
            system_blocks.push(user_block);
        }
        body["system"] = json!(system_blocks);
    } else if !context.system_prompt.is_empty() {
        // Non-OAuth: single system block with optional cache_control
        let mut system_block = json!({
            "type": "text",
            "text": &context.system_prompt,
        });
        if let Some(ref cc) = cache_cc {
            system_block["cache_control"] = cc.clone();
        }
        body["system"] = json!([system_block]);
    }

    // Temperature — disabled when thinking is enabled (Anthropic constraint)
    let thinking_enabled = options.thinking_enabled.unwrap_or(false);
    if !thinking_enabled {
        let temp = options.temperature.or(context.temperature);
        if let Some(t) = temp {
            body["temperature"] = json!(t);
        }
    }

    // Messages — with cache_control on last user message
    let mut messages = convert_messages(&context.messages);
    if let Some(ref retention) = options.cache_retention {
        apply_cache_control_to_last_user_message(&mut messages, retention, &model.base_url);
    }
    body["messages"] = json!(messages);

    // Tools
    if !tools.is_empty() {
        body["tools"] = json!(convert_tools(tools, is_oauth));
    }

    // Thinking configuration
    if thinking_enabled {
        if supports_adaptive_thinking(&model.id) {
            // Opus 4.6 / Sonnet 4.6: adaptive thinking
            body["thinking"] = json!({ "type": "adaptive" });
            // Effort level via output_config
            if let Some(ref effort) = options.reasoning {
                body["output_config"] = json!({
                    "effort": map_thinking_level_to_effort(effort, &model.id),
                });
            }
        } else {
            // Older models: budget-based thinking
            let budget = options.thinking_budget_tokens.unwrap_or(1024);
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget,
            });
        }
    }

    body
}

/// Build cache_control object. Extended retention adds ttl for api.anthropic.com.
fn build_cache_control(retention: &crate::registry::CacheRetention, base_url: &str) -> Value {
    use crate::registry::CacheRetention;
    match retention {
        CacheRetention::Extended if base_url.contains("api.anthropic.com") => {
            json!({ "type": "ephemeral", "ttl": "1h" })
        }
        _ => json!({ "type": "ephemeral" }),
    }
}

/// Apply cache_control ephemeral to the last content block of the last user message.
fn apply_cache_control_to_last_user_message(
    messages: &mut [Value],
    retention: &crate::registry::CacheRetention,
    base_url: &str,
) {
    // Walk backwards to find the last user message
    for msg in messages.iter_mut().rev() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
            if let Some(content) = msg.get_mut("content")
                && let Some(blocks) = content.as_array_mut()
                && let Some(last_block) = blocks.last_mut()
            {
                last_block["cache_control"] = build_cache_control(retention, base_url);
            }
            break;
        }
    }
}

/// Build beta headers for the Anthropic API request.
fn build_beta_headers(model: &Model, options: &StreamOptions, api_key: &str) -> Vec<String> {
    let mut betas = Vec::new();

    let thinking_enabled = options.thinking_enabled.unwrap_or(false);
    let is_adaptive = supports_adaptive_thinking(&model.id);

    if is_oauth_token(api_key) {
        // OAuth tokens get OAuth-specific betas
        betas.push("oauth-2025-04-20".into());
        betas.push("claude-code-20250219".into());
        if thinking_enabled && !is_adaptive {
            betas.push("interleaved-thinking-2025-05-14".into());
        }
    } else {
        // API key auth gets fine-grained-tool-streaming
        betas.push("fine-grained-tool-streaming-2025-05-14".into());
        if thinking_enabled && !is_adaptive {
            betas.push("interleaved-thinking-2025-05-14".into());
        }
    }

    betas
}

// ---------------------------------------------------------------------------
// Message transformation: cross-provider compatibility preprocessing
// ---------------------------------------------------------------------------

/// Normalize a tool call ID for Anthropic API compatibility.
///
/// Anthropic requires IDs matching `^[a-zA-Z0-9_-]+$` (max 64 chars).
/// OpenAI Responses API and other providers may emit IDs with `|` or other
/// special characters and/or lengths exceeding 64 chars.
///
/// Mirrors `normalizeToolCallId()` from pi-mono `anthropic.ts`.
fn normalize_tool_call_id(id: &str) -> String {
    let normalized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    normalized[..normalized.len().min(64)].to_string()
}

/// Preprocess messages for cross-provider compatibility before conversion.
///
/// Applies three transformations (ported from pi-mono `transformMessages()`):
///
/// 1. **Tool call ID normalization** — IDs from other providers (e.g. OpenAI
///    Responses with `|`-separated segments) are sanitized to match Anthropic's
///    `^[a-zA-Z0-9_-]+$` requirement (max 64 chars).  The corresponding
///    `LlmMessage::Tool.tool_call_id` is updated to match the new ID so the
///    API sees a consistent pair.
///
/// 2. **Orphaned tool call injection** — When an `Assistant` message contains
///    tool calls but the immediately-following messages do not include a
///    matching `LlmMessage::Tool` for every call, synthetic error tool-result
///    messages are inserted.  This keeps the message stream well-formed and
///    avoids API validation errors.
///
/// Note: The pi-mono `transformMessages()` also filters `stopReason === 'error'
/// | 'aborted'` assistant messages. That logic requires per-message stop-reason
/// metadata which `LlmMessage::Assistant` does not carry; it must be handled at
/// the agent-session layer before messages reach this function.
fn transform_messages(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    // ---- Pass 1: normalize tool call IDs --------------------------------
    // Build a map of original → normalized IDs for assistant tool calls,
    // then update matching Tool messages.
    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let pass1: Vec<LlmMessage> = messages
        .iter()
        .map(|msg| match msg {
            LlmMessage::Assistant {
                content,
                tool_calls,
                thinking_blocks,
            } => {
                let normalized_calls: Vec<LlmToolCall> = tool_calls
                    .iter()
                    .map(|tc| {
                        let norm_id = normalize_tool_call_id(&tc.id);
                        if norm_id != tc.id {
                            id_map.insert(tc.id.clone(), norm_id.clone());
                            LlmToolCall {
                                id: norm_id,
                                function: tc.function.clone(),
                            }
                        } else {
                            tc.clone()
                        }
                    })
                    .collect();
                LlmMessage::Assistant {
                    content: content.clone(),
                    tool_calls: normalized_calls,
                    thinking_blocks: thinking_blocks.clone(),
                }
            }
            LlmMessage::Tool {
                tool_call_id,
                content,
                tool_name,
            } => {
                let updated_id = id_map
                    .get(tool_call_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| tool_call_id.clone());
                LlmMessage::Tool {
                    tool_call_id: updated_id,
                    content: content.clone(),
                    tool_name: tool_name.clone(),
                }
            }
            other => other.clone(),
        })
        .collect();

    // ---- Pass 2: insert synthetic tool results for orphaned calls --------
    // When an Assistant message has tool_calls but a following user/assistant
    // message arrives before all results are provided, insert placeholder
    // Tool messages so the Anthropic API sees a valid turn structure.
    let mut result: Vec<LlmMessage> = Vec::with_capacity(pass1.len());
    let mut pending_tool_call_ids: Vec<(String, String)> = Vec::new(); // (id, name)
    let mut seen_tool_result_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for msg in &pass1 {
        match msg {
            LlmMessage::Assistant { tool_calls, .. } => {
                // Flush any previously-orphaned calls before processing this assistant msg
                if !pending_tool_call_ids.is_empty() {
                    for (id, name) in pending_tool_call_ids.drain(..) {
                        if !seen_tool_result_ids.contains(&id) {
                            result.push(LlmMessage::Tool {
                                tool_call_id: id,
                                content: "No result provided".to_string(),
                                tool_name: Some(name),
                            });
                        }
                    }
                    seen_tool_result_ids.clear();
                }

                // Track this assistant's tool calls as potentially pending
                if !tool_calls.is_empty() {
                    pending_tool_call_ids = tool_calls
                        .iter()
                        .map(|tc| (tc.id.clone(), tc.function.name.clone()))
                        .collect();
                    seen_tool_result_ids.clear();
                }

                result.push(msg.clone());
            }
            LlmMessage::Tool { tool_call_id, .. } => {
                seen_tool_result_ids.insert(tool_call_id.clone());
                result.push(msg.clone());
            }
            LlmMessage::User { .. } => {
                // User message interrupts tool flow — flush orphaned calls first
                if !pending_tool_call_ids.is_empty() {
                    for (id, name) in pending_tool_call_ids.drain(..) {
                        if !seen_tool_result_ids.contains(&id) {
                            result.push(LlmMessage::Tool {
                                tool_call_id: id,
                                content: "No result provided".to_string(),
                                tool_name: Some(name),
                            });
                        }
                    }
                    seen_tool_result_ids.clear();
                }
                result.push(msg.clone());
            }
            other => {
                result.push(other.clone());
            }
        }
    }

    // Flush any remaining orphaned calls at end of message list
    if !pending_tool_call_ids.is_empty() {
        for (id, name) in pending_tool_call_ids.drain(..) {
            if !seen_tool_result_ids.contains(&id) {
                result.push(LlmMessage::Tool {
                    tool_call_id: id,
                    content: "No result provided".to_string(),
                    tool_name: Some(name),
                });
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Message conversion: LlmMessage -> Anthropic format
// ---------------------------------------------------------------------------

fn convert_messages(messages: &[LlmMessage]) -> Vec<Value> {
    let transformed = transform_messages(messages);
    let messages = transformed.as_slice();

    let mut result = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        match &messages[i] {
            LlmMessage::System { .. } => {
                // System prompt is handled as top-level field; skip here.
            }
            LlmMessage::User { content } => {
                let blocks: Vec<Value> = content
                    .iter()
                    .map(|c| match c {
                        LlmContent::Text(text) => json!({
                            "type": "text",
                            "text": text,
                        }),
                        LlmContent::Image { url } => convert_image_content(url),
                    })
                    .collect();

                if !blocks.is_empty() {
                    result.push(json!({
                        "role": "user",
                        "content": blocks,
                    }));
                }
            }
            LlmMessage::Assistant {
                content,
                tool_calls,
                thinking_blocks,
            } => {
                let mut blocks = Vec::new();

                // Thinking blocks go BEFORE text content
                for tb in thinking_blocks {
                    if tb.redacted {
                        // Redacted thinking: pass back the encrypted signature
                        if let Some(ref sig) = tb.signature {
                            blocks.push(json!({
                                "type": "redacted_thinking",
                                "data": sig,
                            }));
                        }
                    } else {
                        // Normal thinking block
                        let mut block = json!({
                            "type": "thinking",
                            "thinking": tb.thinking,
                        });
                        if let Some(ref sig) = tb.signature {
                            block["signature"] = json!(sig);
                        }
                        blocks.push(block);
                    }
                }

                // Text content
                if !content.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": content,
                    }));
                }

                // Tool use blocks
                for tc in tool_calls {
                    let input: Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.function.name,
                        "input": input,
                    }));
                }

                if !blocks.is_empty() {
                    result.push(json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                }
            }
            LlmMessage::Tool {
                tool_call_id,
                content,
                ..
            } => {
                // Merge consecutive Tool messages into a single user message
                // (required for z.ai Anthropic endpoint compatibility).
                let mut tool_results = vec![json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                })];

                // Look ahead for consecutive Tool messages
                while i + 1 < messages.len() {
                    if let LlmMessage::Tool {
                        tool_call_id: next_id,
                        content: next_content,
                        ..
                    } = &messages[i + 1]
                    {
                        tool_results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": next_id,
                            "content": next_content,
                        }));
                        i += 1;
                    } else {
                        break;
                    }
                }

                result.push(json!({
                    "role": "user",
                    "content": tool_results,
                }));
            }
        }
        i += 1;
    }

    result
}

/// Parse a data URL (e.g. `data:image/png;base64,iVBOR...`) into Anthropic's
/// image source format. Falls back to a text placeholder on parse failure.
fn convert_image_content(url: &str) -> Value {
    if let Some(rest) = url.strip_prefix("data:")
        && let Some((meta, data)) = rest.split_once(',')
    {
        let media_type = meta.trim_end_matches(";base64");
        return json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data,
            },
        });
    }

    // Non-data URL or unparseable — use a text placeholder.
    json!({
        "type": "text",
        "text": format!("[image: {url}]"),
    })
}

// ---------------------------------------------------------------------------
// Tool conversion: LlmTool -> Anthropic format
// ---------------------------------------------------------------------------

fn convert_tools(tools: &[LlmTool], is_oauth: bool) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            let name = if is_oauth {
                to_claude_code_name(&t.name)
            } else {
                t.name.clone()
            };
            json!({
                "name": name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Stop reason mapping
// ---------------------------------------------------------------------------

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "end_turn" | "stop_sequence" | "pause_turn" => StopReason::Stop,
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}

// ---------------------------------------------------------------------------
// SSE line processing (Anthropic event format)
// ---------------------------------------------------------------------------

/// Process a single SSE line from the Anthropic stream.
///
/// Anthropic SSE format:
///   event: <event_type>
///   data: <json_payload>
///
/// We track the current event type via a simple approach: each `data:` line is
/// self-describing (the JSON payload contains a `type` field matching the event
/// type), so we parse based on the JSON `type` field directly.
fn process_sse_line(
    line: &str,
    events: &mut Vec<AssistantMessageEvent>,
    active_blocks: &mut Vec<BlockState>,
) {
    let line = line.trim();

    // Skip empty lines, comments, and event-type lines
    if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
        return;
    }

    let data = match line.strip_prefix("data: ") {
        Some(d) => d,
        None => return,
    };

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Anthropic SSE parse error: {e}, data: {data}");
            return;
        }
    };

    let event_type = match json.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };

    match event_type {
        // ------------------------------------------------------------------
        // message_start: extract initial usage
        // ------------------------------------------------------------------
        "message_start" => {
            if let Some(usage) = json.pointer("/message/usage") {
                let input = usage
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cache_write = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if input > 0 || cache_read > 0 || cache_write > 0 {
                    events.push(AssistantMessageEvent::Usage(Usage {
                        input,
                        output: 0,
                        cache_read,
                        cache_write,
                        total_tokens: input + cache_read + cache_write,
                        ..Usage::default()
                    }));
                }
            }
        }

        // ------------------------------------------------------------------
        // content_block_start
        // ------------------------------------------------------------------
        "content_block_start" => {
            let index = json.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            let block = json.get("content_block");

            let block_type_str = block
                .and_then(|b| b.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match block_type_str {
                "tool_use" => {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    active_blocks.push(BlockState {
                        index,
                        block_type: BlockType::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                        },
                        signature: String::new(),
                    });

                    events.push(AssistantMessageEvent::ToolCallStart { id, name });
                }
                "thinking" => {
                    active_blocks.push(BlockState {
                        index,
                        block_type: BlockType::Thinking,
                        signature: String::new(),
                    });
                    events.push(AssistantMessageEvent::ThinkingStart {
                        content_index: index,
                    });
                }
                "redacted_thinking" => {
                    // Opaque redacted thinking — store the data payload for pass-back.
                    let data = block
                        .and_then(|b| b.get("data"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    active_blocks.push(BlockState {
                        index,
                        block_type: BlockType::RedactedThinking { data },
                        signature: String::new(),
                    });

                    // Emit thinking_start then a delta with placeholder text (pi-mono alignment)
                    events.push(AssistantMessageEvent::ThinkingStart {
                        content_index: index,
                    });
                    events.push(AssistantMessageEvent::ThinkingDelta(
                        "[Reasoning redacted]".to_string(),
                    ));
                }
                _ => {
                    // "text" or anything else
                    active_blocks.push(BlockState {
                        index,
                        block_type: BlockType::Text,
                        signature: String::new(),
                    });
                    events.push(AssistantMessageEvent::TextStart {
                        content_index: index,
                    });
                }
            }
        }

        // ------------------------------------------------------------------
        // content_block_delta
        // ------------------------------------------------------------------
        "content_block_delta" => {
            let delta = match json.get("delta") {
                Some(d) => d,
                None => return,
            };

            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match delta_type {
                "text_delta" => {
                    let text = delta
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(AssistantMessageEvent::TextDelta(text));
                }
                "thinking_delta" => {
                    let thinking = delta
                        .get("thinking")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(AssistantMessageEvent::ThinkingDelta(thinking));
                }
                "input_json_delta" => {
                    let index = json.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    let partial_json = delta
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Find the tool_use block to get its id
                    let id = active_blocks
                        .iter()
                        .find(|b| b.index == index)
                        .and_then(|b| match &b.block_type {
                            BlockType::ToolUse { id, .. } => Some(id.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    events.push(AssistantMessageEvent::ToolCallDelta {
                        id,
                        arguments_delta: partial_json,
                    });
                }
                "signature_delta" => {
                    // Accumulate thinking signature on the matching thinking block.
                    // Signatures are tracked in BlockState but not emitted as events —
                    // they're used at a higher layer for verification.
                    let index = json.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(block) = active_blocks.iter_mut().find(|b| b.index == index)
                        && let Some(sig) = delta.get("signature").and_then(|s| s.as_str())
                    {
                        block.signature.push_str(sig);
                    }
                }
                _ => {
                    // Unknown delta types silently ignored
                }
            }
        }

        // ------------------------------------------------------------------
        // content_block_stop
        // ------------------------------------------------------------------
        "content_block_stop" => {
            let index = json.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

            if let Some(pos) = active_blocks.iter().position(|b| b.index == index) {
                let block = active_blocks.remove(pos);
                match block.block_type {
                    BlockType::ToolUse { id, .. } => {
                        events.push(AssistantMessageEvent::ToolCallEnd { id });
                    }
                    BlockType::Thinking => {
                        events.push(AssistantMessageEvent::ThinkingBlockEnd {
                            signature: block.signature,
                            redacted: false,
                        });
                    }
                    BlockType::RedactedThinking { data } => {
                        // For redacted blocks, the "signature" is the opaque data payload
                        events.push(AssistantMessageEvent::ThinkingBlockEnd {
                            signature: data,
                            redacted: true,
                        });
                    }
                    BlockType::Text => {}
                }
            }
        }

        // ------------------------------------------------------------------
        // message_delta: stop reason + final usage
        // ------------------------------------------------------------------
        "message_delta" => {
            let stop_reason_str = json
                .pointer("/delta/stop_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("end_turn");

            let output_tokens = json
                .pointer("/usage/output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if output_tokens > 0 {
                events.push(AssistantMessageEvent::Usage(Usage {
                    input: 0,
                    output: output_tokens,
                    cache_read: 0,
                    cache_write: 0,
                    total_tokens: output_tokens,
                    ..Usage::default()
                }));
            }

            events.push(AssistantMessageEvent::Done {
                stop_reason: map_stop_reason(stop_reason_str),
            });
        }

        // ------------------------------------------------------------------
        // error
        // ------------------------------------------------------------------
        "error" => {
            let message = json
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Anthropic error")
                .to_string();
            events.push(AssistantMessageEvent::Error(message));
        }

        // Unknown event types are silently ignored
        _ => {}
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;

    // -----------------------------------------------------------------------
    // convert_messages
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_user_text_message() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("Hello, Claude".into())],
        }];
        let result = convert_messages(&messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "Hello, Claude");
    }

    #[test]
    fn test_convert_user_image_message() {
        let messages = vec![LlmMessage::User {
            content: vec![
                LlmContent::Text("Look at this".into()),
                LlmContent::Image {
                    url: "data:image/png;base64,iVBORw0KGgo=".into(),
                },
            ],
        }];
        let result = convert_messages(&messages);

        assert_eq!(result.len(), 1);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "iVBORw0KGgo=");
    }

    #[test]
    fn test_convert_assistant_message_with_text_and_tool_calls() {
        let messages = vec![LlmMessage::Assistant {
            content: "Let me check that.".into(),
            tool_calls: vec![LlmToolCall {
                id: "call_001".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];
        let result = convert_messages(&messages);

        // transform_messages inserts a synthetic tool result for the orphaned
        // call_001 (no matching Tool message follows) — so we get 2 Anthropic
        // messages: the assistant turn + a user turn with the synthetic tool_result.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "assistant");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "Let me check that.");
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["id"], "call_001");
        assert_eq!(blocks[1]["name"], "bash");
        assert_eq!(blocks[1]["input"]["command"], "ls");
        // Second message is the synthetic tool_result user turn
        assert_eq!(result[1]["role"], "user");
        let result_blocks = result[1]["content"].as_array().unwrap();
        assert_eq!(result_blocks.len(), 1);
        assert_eq!(result_blocks[0]["type"], "tool_result");
        assert_eq!(result_blocks[0]["tool_use_id"], "call_001");
    }

    #[test]
    fn test_convert_assistant_message_empty_content_with_tool_calls() {
        let messages = vec![LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "call_002".into(),
                function: LlmFunctionCall {
                    name: "read".into(),
                    arguments: r#"{"path":"/tmp/a.txt"}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];
        let result = convert_messages(&messages);

        // transform_messages inserts a synthetic tool result for the orphaned
        // call_002 (no matching Tool message follows) — 2 Anthropic messages.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "assistant");
        let blocks = result[0]["content"].as_array().unwrap();
        // Only tool_use block, no text block since content is empty
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "tool_use");
        // Second message is the synthetic tool_result user turn
        assert_eq!(result[1]["role"], "user");
    }

    #[test]
    fn test_convert_tool_result_message() {
        let messages = vec![LlmMessage::Tool {
            tool_call_id: "call_001".into(),
            content: "file1.txt\nfile2.txt".into(),
            tool_name: None,
        }];
        let result = convert_messages(&messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "call_001");
        assert_eq!(blocks[0]["content"], "file1.txt\nfile2.txt");
    }

    #[test]
    fn test_convert_system_message_skipped() {
        let messages = vec![
            LlmMessage::System {
                content: "You are helpful.".into(),
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("Hi".into())],
            },
        ];
        let result = convert_messages(&messages);

        // System messages are skipped; only user message remains
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
    }

    #[test]
    fn test_convert_full_conversation() {
        let messages = vec![
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
                content: "file1.txt".into(),
                tool_name: None,
            },
        ];
        let result = convert_messages(&messages);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[1]["role"], "assistant");
        assert_eq!(result[2]["role"], "user"); // tool_result wrapped in user
    }

    #[test]
    fn test_convert_consecutive_tool_results_merged() {
        // Two consecutive Tool messages should be merged into one user message
        let messages = vec![
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
                content: "contents here".into(),
                tool_name: None,
            },
        ];
        let result = convert_messages(&messages);

        // assistant + single merged user (with 2 tool_result blocks)
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "assistant");
        assert_eq!(result[1]["role"], "user");
        let blocks = result[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "call_a");
        assert_eq!(blocks[1]["type"], "tool_result");
        assert_eq!(blocks[1]["tool_use_id"], "call_b");
    }

    #[test]
    fn test_convert_non_consecutive_tool_results_separate() {
        // Tool messages separated by a User message should NOT be merged
        let messages = vec![
            LlmMessage::Tool {
                tool_call_id: "call_a".into(),
                content: "result_a".into(),
                tool_name: None,
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("Continue".into())],
            },
            LlmMessage::Tool {
                tool_call_id: "call_b".into(),
                content: "result_b".into(),
                tool_name: None,
            },
        ];
        let result = convert_messages(&messages);

        // Each tool result is separate (user message breaks the sequence)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["role"], "user"); // first tool
        assert_eq!(result[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(result[1]["role"], "user"); // actual user message
        assert_eq!(result[2]["role"], "user"); // second tool
        assert_eq!(result[2]["content"].as_array().unwrap().len(), 1);
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
                },
                "required": ["command"]
            }),
        }];
        let result = convert_tools(&tools, false);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[0]["description"], "Run a bash command");
        assert_eq!(result[0]["input_schema"]["type"], "object");
        assert_eq!(
            result[0]["input_schema"]["properties"]["command"]["type"],
            "string"
        );
        assert_eq!(result[0]["input_schema"]["required"][0], "command");
    }

    #[test]
    fn test_convert_tools_multiple() {
        let tools = vec![
            LlmTool {
                name: "bash".into(),
                description: "Run a command".into(),
                parameters: json!({"type": "object"}),
            },
            LlmTool {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            },
        ];
        let result = convert_tools(&tools, false);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[1]["name"], "read");
    }

    #[test]
    fn test_convert_tools_empty() {
        let result = convert_tools(&[], false);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // map_stop_reason
    // -----------------------------------------------------------------------

    #[test]
    fn test_map_stop_reason_end_turn() {
        assert_eq!(map_stop_reason("end_turn"), StopReason::Stop);
    }

    #[test]
    fn test_map_stop_reason_stop_sequence() {
        assert_eq!(map_stop_reason("stop_sequence"), StopReason::Stop);
    }

    #[test]
    fn test_map_stop_reason_max_tokens() {
        assert_eq!(map_stop_reason("max_tokens"), StopReason::Length);
    }

    #[test]
    fn test_map_stop_reason_tool_use() {
        assert_eq!(map_stop_reason("tool_use"), StopReason::ToolUse);
    }

    #[test]
    fn test_map_stop_reason_pause_turn() {
        assert_eq!(map_stop_reason("pause_turn"), StopReason::Stop);
    }

    #[test]
    fn test_map_stop_reason_unknown() {
        assert_eq!(map_stop_reason("something_unexpected"), StopReason::Error);
    }

    // -----------------------------------------------------------------------
    // Thinking block serialization in convert_messages (P4-B)
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_assistant_with_normal_thinking_block() {
        let messages = vec![LlmMessage::Assistant {
            content: "The answer is 42.".into(),
            tool_calls: vec![],
            thinking_blocks: vec![ThinkingBlock {
                thinking: "Let me reason step by step...".into(),
                signature: Some("sig_abc".into()),
                redacted: false,
            }],
        }];
        let result = convert_messages(&messages);

        assert_eq!(result.len(), 1);
        let blocks = result[0]["content"].as_array().unwrap();
        // thinking block comes BEFORE text
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "Let me reason step by step...");
        assert_eq!(blocks[0]["signature"], "sig_abc");
        assert_eq!(blocks[1]["type"], "text");
        assert_eq!(blocks[1]["text"], "The answer is 42.");
    }

    #[test]
    fn test_convert_assistant_with_redacted_thinking_block() {
        let messages = vec![LlmMessage::Assistant {
            content: "result".into(),
            tool_calls: vec![],
            thinking_blocks: vec![ThinkingBlock {
                thinking: String::new(),
                signature: Some("opaque_encrypted_payload".into()),
                redacted: true,
            }],
        }];
        let result = convert_messages(&messages);

        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "redacted_thinking");
        assert_eq!(blocks[0]["data"], "opaque_encrypted_payload");
        // redacted_thinking should NOT have "thinking" key
        assert!(blocks[0].get("thinking").is_none());
        assert_eq!(blocks[1]["type"], "text");
    }

    #[test]
    fn test_convert_assistant_with_mixed_thinking_blocks() {
        let messages = vec![LlmMessage::Assistant {
            content: "final answer".into(),
            tool_calls: vec![],
            thinking_blocks: vec![
                ThinkingBlock {
                    thinking: "step 1 reasoning".into(),
                    signature: Some("sig_1".into()),
                    redacted: false,
                },
                ThinkingBlock {
                    thinking: String::new(),
                    signature: Some("redacted_data".into()),
                    redacted: true,
                },
                ThinkingBlock {
                    thinking: "step 2 reasoning".into(),
                    signature: None,
                    redacted: false,
                },
            ],
        }];
        let result = convert_messages(&messages);

        let blocks = result[0]["content"].as_array().unwrap();
        // 3 thinking + 1 text
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "step 1 reasoning");
        assert_eq!(blocks[0]["signature"], "sig_1");
        assert_eq!(blocks[1]["type"], "redacted_thinking");
        assert_eq!(blocks[1]["data"], "redacted_data");
        assert_eq!(blocks[2]["type"], "thinking");
        assert_eq!(blocks[2]["thinking"], "step 2 reasoning");
        assert!(
            blocks[2].get("signature").is_none() || blocks[2]["signature"].is_null(),
            "thinking block without signature should omit signature key"
        );
        assert_eq!(blocks[3]["type"], "text");
    }

    #[test]
    fn test_convert_assistant_thinking_blocks_before_tool_use() {
        let messages = vec![LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "call_001".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                },
            }],
            thinking_blocks: vec![ThinkingBlock {
                thinking: "I should list files".into(),
                signature: None,
                redacted: false,
            }],
        }];
        let result = convert_messages(&messages);

        let blocks = result[0]["content"].as_array().unwrap();
        // thinking + tool_use (no text block since content is empty)
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "I should list files");
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["name"], "bash");
    }

    #[test]
    fn test_convert_assistant_no_thinking_blocks_unchanged() {
        // Verify that messages without thinking blocks still serialize correctly
        let messages = vec![LlmMessage::Assistant {
            content: "hello".into(),
            tool_calls: vec![],
            thinking_blocks: vec![],
        }];
        let result = convert_messages(&messages);

        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "hello");
    }

    #[test]
    fn test_convert_assistant_redacted_without_signature_skipped() {
        // Redacted block without signature should be skipped (nothing to pass back)
        let messages = vec![LlmMessage::Assistant {
            content: "result".into(),
            tool_calls: vec![],
            thinking_blocks: vec![ThinkingBlock {
                thinking: String::new(),
                signature: None,
                redacted: true,
            }],
        }];
        let result = convert_messages(&messages);

        let blocks = result[0]["content"].as_array().unwrap();
        // Only text block — redacted without signature is skipped
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
    }

    // -----------------------------------------------------------------------
    // SSE parsing: process_sse_line
    // -----------------------------------------------------------------------

    #[test]
    fn test_sse_message_start() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","usage":{"input_tokens":100,"cache_read_input_tokens":50,"cache_creation_input_tokens":10}}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 100);
                assert_eq!(u.cache_read, 50);
                assert_eq!(u.cache_write, 10);
            }
            other => panic!("expected Usage, got {:?}", other),
        }
    }

    #[test]
    fn test_sse_content_block_start_text() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        // Text block start emits TextStart with the content index
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::TextStart { content_index } => {
                assert_eq!(*content_index, 0);
            }
            other => panic!("expected TextStart, got {:?}", other),
        }
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn test_sse_content_block_start_tool_use() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"bash"}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "bash");
            }
            other => panic!("expected ToolCallStart, got {:?}", other),
        }
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn test_sse_text_delta() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssistantMessageEvent::TextDelta(s) if s == "Hello"));
    }

    #[test]
    fn test_sse_thinking_delta() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me consider..."}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingDelta(s) if s == "Let me consider...")
        );
    }

    #[test]
    fn test_sse_input_json_delta() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 1,
            block_type: BlockType::ToolUse {
                id: "toolu_01".into(),
                name: "bash".into(),
            },
            signature: String::new(),
        }];
        let line = r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(arguments_delta, r#"{"command":"ls"}"#);
            }
            other => panic!("expected ToolCallDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_sse_content_block_stop_tool_use() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 1,
            block_type: BlockType::ToolUse {
                id: "toolu_01".into(),
                name: "bash".into(),
            },
            signature: String::new(),
        }];
        let line = r#"data: {"type":"content_block_stop","index":1}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ToolCallEnd { id } => {
                assert_eq!(id, "toolu_01");
            }
            other => panic!("expected ToolCallEnd, got {:?}", other),
        }
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_sse_message_delta_done() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        // Should emit Usage + Done
        assert_eq!(events.len(), 2);
        match &events[0] {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.output, 42);
            }
            other => panic!("expected Usage, got {:?}", other),
        }
        match &events[1] {
            AssistantMessageEvent::Done { stop_reason } => {
                assert_eq!(*stop_reason, StopReason::Stop);
            }
            other => panic!("expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_sse_message_delta_tool_use_stop() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":10}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 2);
        match &events[1] {
            AssistantMessageEvent::Done { stop_reason } => {
                assert_eq!(*stop_reason, StopReason::ToolUse);
            }
            other => panic!("expected Done with ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn test_sse_error_event() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line =
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error(msg) => {
                assert_eq!(msg, "Overloaded");
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_sse_skip_empty_and_comment_lines() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();

        process_sse_line("", &mut events, &mut blocks);
        process_sse_line(": comment", &mut events, &mut blocks);
        process_sse_line("event: message_start", &mut events, &mut blocks);

        assert!(events.is_empty());
    }

    #[test]
    fn test_sse_malformed_json_warns_and_continues() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = "data: not-valid-json";

        process_sse_line(line, &mut events, &mut blocks);

        // Should not panic; no event emitted
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------------
    // convert_image_content
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_image_data_url() {
        let result = convert_image_content("data:image/jpeg;base64,/9j/4AAQ==");
        assert_eq!(result["type"], "image");
        assert_eq!(result["source"]["type"], "base64");
        assert_eq!(result["source"]["media_type"], "image/jpeg");
        assert_eq!(result["source"]["data"], "/9j/4AAQ==");
    }

    #[test]
    fn test_convert_image_non_data_url_fallback() {
        let result = convert_image_content("https://example.com/photo.png");
        assert_eq!(result["type"], "text");
        assert!(result["text"].as_str().unwrap().contains("example.com"));
    }

    // -----------------------------------------------------------------------
    // build_request_body
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_basic() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "You are helpful.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, false);

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
        // System prompt is now an array of blocks
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys.len(), 1);
        assert_eq!(sys[0]["type"], "text");
        assert_eq!(sys[0]["text"], "You are helpful.");
        assert!(body["messages"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn test_build_request_body_empty_system_prompt() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, false);

        // No system field when prompt is empty
        assert!(body.get("system").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let model = crate::test_helpers::test_model();
        let context = crate::test_helpers::test_context();
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run command".into(),
            parameters: json!({"type": "object"}),
        }];
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &tools, &options, false);

        assert!(body.get("tools").is_some());
        let tool_array = body["tools"].as_array().unwrap();
        assert_eq!(tool_array.len(), 1);
        assert_eq!(tool_array[0]["name"], "bash");
        assert_eq!(tool_array[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn test_build_request_body_options_override() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: Some(0.5),
        };
        let options = StreamOptions {
            temperature: Some(0.9),
            max_tokens: Some(2048),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        assert_eq!(body["max_tokens"], 2048);
        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.9).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Adaptive thinking
    // -----------------------------------------------------------------------

    #[test]
    fn test_supports_adaptive_thinking() {
        assert!(supports_adaptive_thinking("claude-opus-4-6-20260412"));
        assert!(supports_adaptive_thinking("claude-sonnet-4-6-20260412"));
        assert!(supports_adaptive_thinking("claude-opus-4.6"));
        assert!(supports_adaptive_thinking("claude-sonnet-4.6"));
        assert!(!supports_adaptive_thinking("claude-sonnet-4-20250514"));
        assert!(!supports_adaptive_thinking("claude-3-5-haiku-20241022"));
    }

    #[test]
    fn test_build_request_body_adaptive_thinking() {
        let mut model = crate::test_helpers::test_model();
        model.id = "claude-opus-4-6-20260412".into();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "test".into(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            reasoning: Some(ReasoningLevel::High),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        // Temperature should NOT be set when thinking is enabled
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn test_build_request_body_budget_thinking() {
        let mut model = crate::test_helpers::test_model();
        model.id = "claude-sonnet-4-20250514".into(); // non-adaptive model
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "test".into(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            thinking_budget_tokens: Some(2048),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 2048);
    }

    #[test]
    fn test_build_request_body_budget_thinking_default() {
        let mut model = crate::test_helpers::test_model();
        model.id = "claude-sonnet-4-20250514".into();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        // Default budget is 1024
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 1024);
    }

    // -----------------------------------------------------------------------
    // Temperature disabled with thinking
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_no_temperature_with_thinking() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: Some(0.7),
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            temperature: Some(0.9),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        // Temperature must NOT be set when thinking is enabled
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn test_build_request_body_temperature_without_thinking() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: Some(0.7),
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, false);

        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Cache control
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_cache_control_on_system() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            }],
            system_prompt: "You are helpful.".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            cache_retention: Some(crate::registry::CacheRetention::Standard),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_build_request_body_cache_control_on_last_user() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![
                LlmMessage::User {
                    content: vec![LlmContent::Text("first".into())],
                },
                LlmMessage::Assistant {
                    content: "ok".into(),
                    tool_calls: vec![],
                    thinking_blocks: vec![],
                },
                LlmMessage::User {
                    content: vec![LlmContent::Text("second".into())],
                },
            ],
            system_prompt: "test".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions {
            cache_retention: Some(crate::registry::CacheRetention::Standard),
            ..StreamOptions::default()
        };
        let body = build_request_body(&model, &context, &[], &options, false);

        let messages = body["messages"].as_array().unwrap();
        // Last user message (index 2) should have cache_control on its last block
        let last_user = &messages[2];
        let blocks = last_user["content"].as_array().unwrap();
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");

        // First user message should NOT have cache_control
        let first_user = &messages[0];
        let blocks = first_user["content"].as_array().unwrap();
        assert!(blocks[0].get("cache_control").is_none());
    }

    #[test]
    fn test_build_cache_control_extended_anthropic() {
        use crate::registry::CacheRetention;
        let cc = build_cache_control(&CacheRetention::Extended, "https://api.anthropic.com/v1");
        assert_eq!(cc["type"], "ephemeral");
        assert_eq!(cc["ttl"], "1h");
    }

    #[test]
    fn test_build_cache_control_extended_non_anthropic() {
        use crate::registry::CacheRetention;
        let cc = build_cache_control(&CacheRetention::Extended, "https://other.proxy.com/v1");
        assert_eq!(cc["type"], "ephemeral");
        assert!(cc.get("ttl").is_none());
    }

    // -----------------------------------------------------------------------
    // Effort level mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_map_thinking_level_to_effort() {
        assert_eq!(
            map_thinking_level_to_effort(&ReasoningLevel::Low, "claude-opus-4-6"),
            "low"
        );
        assert_eq!(
            map_thinking_level_to_effort(&ReasoningLevel::Medium, "claude-opus-4-6"),
            "medium"
        );
        assert_eq!(
            map_thinking_level_to_effort(&ReasoningLevel::High, "claude-opus-4-6"),
            "high"
        );
        assert_eq!(
            map_thinking_level_to_effort(&ReasoningLevel::XHigh, "claude-opus-4-6"),
            "max"
        );
        assert_eq!(
            map_thinking_level_to_effort(&ReasoningLevel::XHigh, "claude-sonnet-4-6"),
            "high"
        );
    }

    // -----------------------------------------------------------------------
    // Beta headers
    // -----------------------------------------------------------------------

    #[test]
    fn test_beta_headers_api_key() {
        let model = crate::test_helpers::test_model();
        let options = StreamOptions::default();
        let betas = build_beta_headers(&model, &options, "sk-ant-api-key");

        assert!(betas.contains(&"fine-grained-tool-streaming-2025-05-14".to_string()));
        assert!(!betas.iter().any(|b| b.contains("oauth")));
    }

    #[test]
    fn test_beta_headers_oauth() {
        let model = crate::test_helpers::test_model();
        let options = StreamOptions::default();
        let betas = build_beta_headers(&model, &options, "sk-ant-oat-some-token");

        assert!(betas.contains(&"oauth-2025-04-20".to_string()));
        assert!(betas.contains(&"claude-code-20250219".to_string()));
        assert!(!betas.iter().any(|b| b.contains("fine-grained")));
    }

    #[test]
    fn test_beta_headers_interleaved_thinking_non_adaptive() {
        let mut model = crate::test_helpers::test_model();
        model.id = "claude-sonnet-4-20250514".into(); // non-adaptive
        let options = StreamOptions {
            thinking_enabled: Some(true),
            ..StreamOptions::default()
        };
        let betas = build_beta_headers(&model, &options, "sk-ant-api-key");

        assert!(betas.contains(&"interleaved-thinking-2025-05-14".to_string()));
    }

    #[test]
    fn test_beta_headers_no_interleaved_for_adaptive() {
        let mut model = crate::test_helpers::test_model();
        model.id = "claude-opus-4-6-20260412".into(); // adaptive
        let options = StreamOptions {
            thinking_enabled: Some(true),
            ..StreamOptions::default()
        };
        let betas = build_beta_headers(&model, &options, "sk-ant-api-key");

        // Adaptive models don't need interleaved-thinking beta
        assert!(!betas.iter().any(|b| b.contains("interleaved-thinking")));
    }

    // -----------------------------------------------------------------------
    // Signature delta tracking
    // -----------------------------------------------------------------------

    #[test]
    fn test_sse_signature_delta_accumulated() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 0,
            block_type: BlockType::Thinking,
            signature: String::new(),
        }];

        // First signature chunk
        let line1 = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#;
        process_sse_line(line1, &mut events, &mut blocks);

        // Second signature chunk
        let line2 = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"def"}}"#;
        process_sse_line(line2, &mut events, &mut blocks);

        // No events emitted for signature_delta (it's tracked internally)
        assert!(events.is_empty());
        // But the signature was accumulated on the block
        assert_eq!(blocks[0].signature, "abcdef");
    }

    // -----------------------------------------------------------------------
    // Redacted thinking
    // -----------------------------------------------------------------------

    #[test]
    fn test_sse_redacted_thinking_block() {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        let line = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque_base64_payload"}}"#;

        process_sse_line(line, &mut events, &mut blocks);

        // Should emit ThinkingStart then ThinkingDelta with placeholder text (pi-mono alignment)
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], AssistantMessageEvent::ThinkingStart { content_index } if *content_index == 0)
        );
        assert!(
            matches!(&events[1], AssistantMessageEvent::ThinkingDelta(s) if s == "[Reasoning redacted]")
        );
        // Block should be RedactedThinking with the data
        assert_eq!(blocks.len(), 1);
        match &blocks[0].block_type {
            BlockType::RedactedThinking { data } => {
                assert_eq!(data, "opaque_base64_payload");
            }
            other => panic!("expected RedactedThinking, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // content_block_stop emits ThinkingBlockEnd (P4-B)
    // -----------------------------------------------------------------------

    #[test]
    fn test_sse_thinking_block_stop_emits_signature() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 0,
            block_type: BlockType::Thinking,
            signature: "sig_accumulated".into(),
        }];
        let line = r#"data: {"type":"content_block_stop","index":0}"#;
        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ThinkingBlockEnd {
                signature,
                redacted,
            } => {
                assert_eq!(signature, "sig_accumulated");
                assert!(!redacted);
            }
            other => panic!("expected ThinkingBlockEnd, got: {other:?}"),
        }
        assert!(blocks.is_empty(), "block should be removed");
    }

    #[test]
    fn test_sse_redacted_thinking_block_stop_emits_data() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 0,
            block_type: BlockType::RedactedThinking {
                data: "opaque_encrypted_payload".into(),
            },
            signature: String::new(),
        }];
        let line = r#"data: {"type":"content_block_stop","index":0}"#;
        process_sse_line(line, &mut events, &mut blocks);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::ThinkingBlockEnd {
                signature,
                redacted,
            } => {
                assert_eq!(signature, "opaque_encrypted_payload");
                assert!(redacted);
            }
            other => panic!("expected ThinkingBlockEnd, got: {other:?}"),
        }
    }

    #[test]
    fn test_sse_text_block_stop_no_event() {
        let mut events = Vec::new();
        let mut blocks = vec![BlockState {
            index: 0,
            block_type: BlockType::Text,
            signature: String::new(),
        }];
        let line = r#"data: {"type":"content_block_stop","index":0}"#;
        process_sse_line(line, &mut events, &mut blocks);

        assert!(events.is_empty(), "text block stop should not emit events");
        assert!(blocks.is_empty());
    }

    // -----------------------------------------------------------------------
    // OAuth detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_oauth_token() {
        assert!(is_oauth_token("sk-ant-oat-some-token-here"));
        assert!(!is_oauth_token("sk-ant-api03-some-key"));
        assert!(!is_oauth_token("regular-api-key"));
    }

    // -----------------------------------------------------------------------
    // Claude Code tool name mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_claude_code_name_known_tools() {
        // Exact casing matches
        assert_eq!(to_claude_code_name("read"), "Read");
        assert_eq!(to_claude_code_name("write"), "Write");
        assert_eq!(to_claude_code_name("edit"), "Edit");
        assert_eq!(to_claude_code_name("bash"), "Bash");
        assert_eq!(to_claude_code_name("grep"), "Grep");
        assert_eq!(to_claude_code_name("glob"), "Glob");
        assert_eq!(to_claude_code_name("webfetch"), "WebFetch");
        assert_eq!(to_claude_code_name("websearch"), "WebSearch");
        assert_eq!(to_claude_code_name("todowrite"), "TodoWrite");
    }

    #[test]
    fn test_to_claude_code_name_case_insensitive() {
        // Caller may use any casing
        assert_eq!(to_claude_code_name("READ"), "Read");
        assert_eq!(to_claude_code_name("BASH"), "Bash");
        assert_eq!(to_claude_code_name("WebFetch"), "WebFetch");
    }

    #[test]
    fn test_to_claude_code_name_unknown_passthrough() {
        // Unknown tool names pass through unchanged
        let result = to_claude_code_name("my_custom_tool");
        assert_eq!(result, "my_custom_tool");
    }

    #[test]
    fn test_from_claude_code_name_matches_tool_list() {
        // Tools registered with lowercase names (as the caller would pass them).
        // The API sends back the CC canonical casing: "Read", "Bash".
        // fromClaudeCodeName does a case-insensitive lookup against the tools list.
        let tools = vec![
            LlmTool {
                name: "read".into(),
                description: "reads a file".into(),
                parameters: json!({"type": "object"}),
            },
            LlmTool {
                name: "bash".into(),
                description: "runs bash".into(),
                parameters: json!({"type": "object"}),
            },
        ];
        // CC sends back "Read"; caller registered it as "read"
        assert_eq!(from_claude_code_name("Read", &tools), "read");
        // CC sends back "Bash"; caller registered it as "bash"
        assert_eq!(from_claude_code_name("Bash", &tools), "bash");
    }

    #[test]
    fn test_from_claude_code_name_no_tools_passthrough() {
        // No tools list → name passes through unchanged
        assert_eq!(from_claude_code_name("Read", &[]), "Read");
    }

    #[test]
    fn test_from_claude_code_name_no_match_passthrough() {
        let tools = vec![LlmTool {
            name: "other_tool".into(),
            description: "something else".into(),
            parameters: json!({"type": "object"}),
        }];
        // "Read" doesn't match "other_tool" → pass through
        assert_eq!(from_claude_code_name("Read", &tools), "Read");
    }

    // -----------------------------------------------------------------------
    // OAuth system prompt injection
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_request_body_oauth_system_prompt_injected() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, true);

        // Must have system array even with empty user prompt
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys.len(), 1);
        assert_eq!(sys[0]["type"], "text");
        assert!(
            sys[0]["text"].as_str().unwrap().contains("Claude Code"),
            "OAuth must inject Claude Code identity block"
        );
    }

    #[test]
    fn test_build_request_body_oauth_system_prompt_prepended() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "Custom system prompt".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, true);

        // Two blocks: CC identity first, caller's prompt second
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys.len(), 2);
        assert!(sys[0]["text"].as_str().unwrap().contains("Claude Code"));
        assert_eq!(sys[1]["text"], "Custom system prompt");
    }

    #[test]
    fn test_build_request_body_non_oauth_no_cc_identity() {
        let model = crate::test_helpers::test_model();
        let context = LlmContext {
            messages: vec![],
            system_prompt: "My prompt".into(),
            max_tokens: 1024,
            temperature: None,
        };
        let options = StreamOptions::default();
        let body = build_request_body(&model, &context, &[], &options, false);

        let sys = body["system"].as_array().unwrap();
        // Non-OAuth: only the user's prompt, no CC identity prepended
        assert_eq!(sys.len(), 1);
        assert_eq!(sys[0]["text"], "My prompt");
    }

    #[test]
    fn test_convert_tools_oauth_uses_cc_names() {
        let tools = vec![
            LlmTool {
                name: "read".into(),
                description: "read file".into(),
                parameters: json!({"type": "object"}),
            },
            LlmTool {
                name: "bash".into(),
                description: "run command".into(),
                parameters: json!({"type": "object"}),
            },
            LlmTool {
                name: "my_custom".into(),
                description: "custom tool".into(),
                parameters: json!({"type": "object"}),
            },
        ];
        let result = convert_tools(&tools, true);

        assert_eq!(result[0]["name"], "Read"); // mapped
        assert_eq!(result[1]["name"], "Bash"); // mapped
        assert_eq!(result[2]["name"], "my_custom"); // unknown, unchanged
    }

    #[test]
    fn test_convert_tools_non_oauth_names_unchanged() {
        let tools = vec![LlmTool {
            name: "read".into(),
            description: "read file".into(),
            parameters: json!({"type": "object"}),
        }];
        let result = convert_tools(&tools, false);

        assert_eq!(result[0]["name"], "read"); // not mapped
    }

    // -----------------------------------------------------------------------
    // normalize_tool_call_id
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_tool_call_id_clean_passthrough() {
        assert_eq!(normalize_tool_call_id("call_abc123"), "call_abc123");
        assert_eq!(normalize_tool_call_id("toolu-01ABC"), "toolu-01ABC");
    }

    #[test]
    fn test_normalize_tool_call_id_replaces_special_chars() {
        // OpenAI Responses API emits IDs like "call_|abc|def"
        let result = normalize_tool_call_id("call_|abc|def");
        assert!(!result.contains('|'));
        assert_eq!(result, "call__abc_def");
    }

    #[test]
    fn test_normalize_tool_call_id_truncates_to_64() {
        let long_id = "a".repeat(100);
        let result = normalize_tool_call_id(&long_id);
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn test_normalize_tool_call_id_preserves_allowed_chars() {
        // All allowed chars must pass through untouched
        let id = "aZ0_-";
        assert_eq!(normalize_tool_call_id(id), id);
    }

    // -----------------------------------------------------------------------
    // transform_messages: tool call ID normalization
    // -----------------------------------------------------------------------

    #[test]
    fn test_transform_normalizes_tool_call_ids_in_assistant_and_tool() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "call_|openai|pipe|id".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: r#"{"command":"ls"}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "call_|openai|pipe|id".into(),
                content: "result".into(),
                tool_name: None,
            },
        ];

        let transformed = transform_messages(&messages);

        // Both the assistant tool_call id and the Tool tool_call_id should be updated
        match &transformed[0] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls[0].id, "call__openai_pipe_id");
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
        match &transformed[1] {
            LlmMessage::Tool { tool_call_id, .. } => {
                assert_eq!(tool_call_id, "call__openai_pipe_id");
            }
            other => panic!("expected Tool, got {:?}", other),
        }
    }

    #[test]
    fn test_transform_clean_ids_unchanged() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "toolu_01".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: r#"{}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "toolu_01".into(),
                content: "ok".into(),
                tool_name: None,
            },
        ];

        let transformed = transform_messages(&messages);
        match &transformed[0] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls[0].id, "toolu_01");
            }
            _ => panic!("expected Assistant"),
        }
        match &transformed[1] {
            LlmMessage::Tool { tool_call_id, .. } => {
                assert_eq!(tool_call_id, "toolu_01");
            }
            _ => panic!("expected Tool"),
        }
    }

    // -----------------------------------------------------------------------
    // transform_messages: orphaned tool call injection
    // -----------------------------------------------------------------------

    #[test]
    fn test_transform_inserts_synthetic_tool_result_for_orphaned_call() {
        // Assistant has a tool call but no Tool message follows
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "tc_orphan".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: r#"{}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("continue".into())],
            },
        ];

        let transformed = transform_messages(&messages);

        // Should be: assistant, synthetic tool result, user
        assert_eq!(transformed.len(), 3);
        match &transformed[1] {
            LlmMessage::Tool {
                tool_call_id,
                content,
                ..
            } => {
                assert_eq!(tool_call_id, "tc_orphan");
                assert!(content.contains("No result provided"));
            }
            other => panic!("expected synthetic Tool, got {:?}", other),
        }
        assert!(matches!(&transformed[2], LlmMessage::User { .. }));
    }

    #[test]
    fn test_transform_no_synthetic_result_when_all_results_present() {
        // All tool calls have matching Tool messages — no injection needed
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "tc_ok".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: r#"{}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "tc_ok".into(),
                content: "result".into(),
                tool_name: None,
            },
        ];

        let transformed = transform_messages(&messages);
        // No synthetic messages inserted
        assert_eq!(transformed.len(), 2);
        assert!(matches!(&transformed[0], LlmMessage::Assistant { .. }));
        assert!(matches!(&transformed[1], LlmMessage::Tool { .. }));
    }

    #[test]
    fn test_transform_partial_orphan_injection() {
        // Two tool calls; only one has a result
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    LlmToolCall {
                        id: "tc_a".into(),
                        function: LlmFunctionCall {
                            name: "bash".into(),
                            arguments: r#"{}"#.into(),
                        },
                    },
                    LlmToolCall {
                        id: "tc_b".into(),
                        function: LlmFunctionCall {
                            name: "read".into(),
                            arguments: r#"{}"#.into(),
                        },
                    },
                ],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "tc_a".into(),
                content: "result_a".into(),
                tool_name: None,
            },
            // tc_b result is missing — orphaned
            LlmMessage::User {
                content: vec![LlmContent::Text("next".into())],
            },
        ];

        let transformed = transform_messages(&messages);

        // Should be: assistant, tool_a, synthetic_tool_b, user
        assert_eq!(transformed.len(), 4);
        match &transformed[2] {
            LlmMessage::Tool { tool_call_id, .. } => {
                assert_eq!(tool_call_id, "tc_b");
            }
            other => panic!("expected synthetic Tool for tc_b, got {:?}", other),
        }
    }

    #[test]
    fn test_transform_orphan_at_end_of_messages() {
        // Orphaned tool call at end of message list (no following message)
        let messages = vec![LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "tc_end".into(),
                function: LlmFunctionCall {
                    name: "bash".into(),
                    arguments: r#"{}"#.into(),
                },
            }],
            thinking_blocks: vec![],
        }];

        let transformed = transform_messages(&messages);

        // assistant + synthetic tool result
        assert_eq!(transformed.len(), 2);
        match &transformed[1] {
            LlmMessage::Tool { tool_call_id, .. } => {
                assert_eq!(tool_call_id, "tc_end");
            }
            other => panic!("expected synthetic Tool, got {:?}", other),
        }
    }

    #[test]
    fn test_transform_user_messages_pass_through_unchanged() {
        let messages = vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("hello".into())],
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("world".into())],
            },
        ];
        let transformed = transform_messages(&messages);
        assert_eq!(transformed.len(), 2);
        assert!(matches!(&transformed[0], LlmMessage::User { .. }));
        assert!(matches!(&transformed[1], LlmMessage::User { .. }));
    }

    // -----------------------------------------------------------------------
    // convert_messages with transform: integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_convert_messages_with_orphaned_tool_call_inserts_synthetic_result() {
        // An assistant message with a tool call followed immediately by a user
        // message (no Tool message) should produce a tool_result block in the output.
        let messages = vec![
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
            LlmMessage::User {
                content: vec![LlmContent::Text("ignore that".into())],
            },
        ];

        let result = convert_messages(&messages);

        // Output: assistant(tool_use), user(tool_result + user_content)
        // The synthetic tool result is inserted before the user message.
        // After merging: assistant msg, then a user msg with tool_result,
        // then another user msg — OR the synthetic tool_result and real user
        // are both present as separate user messages.
        // Either way there should be ≥3 items: assistant + synthetic tool_result + real user.
        assert!(
            result.len() >= 3,
            "expected ≥3 messages, got {}",
            result.len()
        );

        // First is the assistant message
        assert_eq!(result[0]["role"], "assistant");
        let assistant_blocks = result[0]["content"].as_array().unwrap();
        assert!(assistant_blocks.iter().any(|b| b["type"] == "tool_use"));

        // Find the synthetic tool_result user message
        let tool_result_msg = result.iter().find(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_array()
                    .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                    .unwrap_or(false)
        });
        assert!(
            tool_result_msg.is_some(),
            "expected a user message with tool_result block"
        );
    }
}
