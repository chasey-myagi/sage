// OpenAI-Compatible Provider — Phase 5
// Makes HTTP calls to OpenAI-compatible LLM APIs with SSE streaming.

use crate::llm::keys;
use crate::llm::stream::parse_sse_chunk;
use crate::llm::types::*;
use crate::llm::LlmProvider;
use reqwest::Client;
use serde_json::{json, Value};

/// Provider that calls any OpenAI-compatible chat completions API.
pub struct OpenAiCompatProvider {
    client: Client,
}

impl OpenAiCompatProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Build the JSON request body for the chat completions API.
    fn build_request_body(
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Value {
        let mut messages = Vec::new();

        // System prompt
        if !context.system_prompt.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": context.system_prompt,
            }));
        }

        // Conversation messages
        for msg in &context.messages {
            match msg {
                LlmMessage::System { content } => {
                    messages.push(json!({
                        "role": "system",
                        "content": content,
                    }));
                }
                LlmMessage::User { content } => {
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
                                json!({
                                    "id": tc.id,
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
                }
                LlmMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": content,
                    }));
                }
            }
        }

        let max_tokens_key = match model.compat.max_tokens_field {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        };

        let mut body = json!({
            "model": model.id,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        body[max_tokens_key] = json!(context.max_tokens);

        if let Some(temp) = context.temperature {
            body["temperature"] = json!(temp);
        }

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
                        },
                    })
                })
                .collect();
            body["tools"] = json!(tool_defs);
        }

        body
    }
}

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
impl LlmProvider for OpenAiCompatProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        let api_key = match keys::resolve_api_key_from_env(&model.api_key_env) {
            Ok(key) => key,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "API key error: {e}"
                ))];
            }
        };

        let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
        let body = Self::build_request_body(model, context, tools);

        let response = match self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
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

        // Parse SSE stream chunk-by-chunk (not batching entire response into memory)
        use futures::StreamExt;

        // Use a byte buffer to avoid corrupting multi-byte UTF-8 sequences
        // at chunk boundaries. Chunks from bytes_stream() can split anywhere,
        // including mid-character. We only decode to String after finding a
        // complete line (delimited by b'\n').
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

            // Process all complete lines in the buffer
            while let Some(newline_pos) = byte_buf.iter().position(|&b| b == b'\n') {
                let line_bytes = byte_buf[..newline_pos].to_vec();
                byte_buf.drain(..=newline_pos);
                let line = String::from_utf8_lossy(&line_bytes);
                process_sse_line(&line, &mut events);
            }
        }

        // Flush remaining data after stream ends (final chunk may lack trailing newline)
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
        let body = OpenAiCompatProvider::build_request_body(&model, &context, &[]);

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
        // System prompt + user message
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
    }

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
        let body = OpenAiCompatProvider::build_request_body(&model, &context, &tools);

        assert!(body.get("tools").is_some());
        let tool_array = body["tools"].as_array().unwrap();
        assert_eq!(tool_array.len(), 1);
        assert_eq!(tool_array[0]["function"]["name"], "bash");
        assert!(body["temperature"].as_f64().unwrap() > 0.69 && body["temperature"].as_f64().unwrap() < 0.71);
    }

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
        let body = OpenAiCompatProvider::build_request_body(&model, &context, &[]);

        let messages = body["messages"].as_array().unwrap();
        // system + user + assistant + tool = 4
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2]["role"], "assistant");
        assert!(messages[2]["tool_calls"].is_array());
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "tc1");
    }

    #[test]
    fn test_build_request_body_max_completion_tokens() {
        let mut model = test_model();
        model.compat.max_tokens_field = MaxTokensField::MaxCompletionTokens;
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 2048,
            temperature: None,
        };
        let body = OpenAiCompatProvider::build_request_body(&model, &context, &[]);

        // Should use max_completion_tokens instead of max_tokens
        assert_eq!(body["max_completion_tokens"], 2048);
        assert!(body.get("max_tokens").is_none() || body["max_tokens"].is_null());
        // Verify the old bug is fixed: no literal "max_tokens_key" field
        assert!(body.get("max_tokens_key").is_none());
    }

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
        let body = OpenAiCompatProvider::build_request_body(&model, &context, &[]);

        let messages = body["messages"].as_array().unwrap();
        // No system message when prompt is empty
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    use crate::test_helpers::test_model;
}
