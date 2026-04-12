// SSE stream parsing — Phase 2
// Parses OpenAI-compatible SSE chunks into AssistantMessageEvent.

use crate::llm::types::AssistantMessageEvent;
use crate::types::{StopReason, Usage};

/// Parses an SSE data chunk (JSON) into an AssistantMessageEvent.
pub fn parse_sse_chunk(
    data: &str,
) -> Result<Option<AssistantMessageEvent>, Box<dyn std::error::Error>> {
    let trimmed = data.trim();

    if trimmed == "[DONE]" {
        return Ok(None);
    }

    if trimmed.is_empty() {
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_str(trimmed)?;

    // Error object
    if let Some(error) = json.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Ok(Some(AssistantMessageEvent::Error(msg.to_string())));
    }

    // Usage (only when choices is empty/absent)
    let choices = json.get("choices").and_then(|c| c.as_array());
    let empty_choices = choices.map_or(true, |c| c.is_empty());

    if let Some(usage) = json.get("usage") {
        if usage.is_object() && empty_choices {
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
                .unwrap_or(0);
            let cache_read = usage
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if input > 0 || output > 0 || total > 0 {
                return Ok(Some(AssistantMessageEvent::Usage(Usage {
                    input,
                    output,
                    cache_read,
                    cache_write: 0,
                    total_tokens: total,
                    ..Usage::default()
                })));
            }
        }
    }

    // First choice
    let choice = match choices {
        Some(arr) if !arr.is_empty() => &arr[0],
        _ => return Ok(None),
    };

    // finish_reason
    if let Some(finish_reason) = choice.get("finish_reason") {
        if !finish_reason.is_null() {
            let reason_str = finish_reason.as_str().unwrap_or("");
            let stop_reason = match reason_str {
                "stop" => StopReason::Stop,
                "length" => StopReason::Length,
                "tool_calls" => StopReason::ToolUse,
                "content_filter" => StopReason::Error,
                _ => StopReason::Stop,
            };
            return Ok(Some(AssistantMessageEvent::Done { stop_reason }));
        }
    }

    // Delta
    let delta = match choice.get("delta") {
        Some(d) if d.is_object() => d,
        _ => return Ok(None),
    };

    // reasoning_content (thinking)
    if let Some(reasoning) = delta.get("reasoning_content") {
        if !reasoning.is_null() {
            let text = reasoning.as_str().unwrap_or("").to_string();
            return Ok(Some(AssistantMessageEvent::ThinkingDelta(text)));
        }
    }

    // tool_calls
    if let Some(tool_calls) = delta.get("tool_calls") {
        if let Some(arr) = tool_calls.as_array() {
            if let Some(tc) = arr.first() {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(func) = tc.get("function") {
                    let name = func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = func
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if !name.is_empty() {
                        return Ok(Some(AssistantMessageEvent::ToolCallStart { id, name }));
                    } else if !args.is_empty() {
                        return Ok(Some(AssistantMessageEvent::ToolCallDelta {
                            id,
                            arguments_delta: args,
                        }));
                    }
                }
            }
        }
    }

    // content
    if let Some(content) = delta.get("content") {
        if content.is_null() {
            return Ok(None);
        }
        let text = content.as_str().unwrap_or("").to_string();
        return Ok(Some(AssistantMessageEvent::TextDelta(text)));
    }

    Ok(None)
}

/// Incrementally accumulates JSON fragments from streaming tool call arguments.
pub struct IncrementalJsonParser {
    buf: String,
}

impl IncrementalJsonParser {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
        }
    }

    pub fn push(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
    }

    pub fn buffer(&self) -> &str {
        &self.buf
    }

    pub fn complete(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let val = serde_json::from_str(&self.buf)?;
        Ok(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::*;
    use crate::types::*;

    // ========================================================================
    // parse_sse_chunk — text delta
    // ========================================================================

    #[test]
    fn test_parse_text_delta() {
        let data = r#"{"choices":[{"delta":{"content":"Hello"},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        assert!(matches!(event, AssistantMessageEvent::TextDelta(s) if s == "Hello"));
    }

    #[test]
    fn test_parse_text_delta_empty_content() {
        let data = r#"{"choices":[{"delta":{"content":""},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap();
        // Empty content delta may be None or an empty TextDelta
        match event {
            Some(AssistantMessageEvent::TextDelta(s)) => assert!(s.is_empty()),
            None => {} // also acceptable
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_delta_multiline() {
        let data = r#"{"choices":[{"delta":{"content":"line1\nline2"},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::TextDelta(s) => assert!(s.contains('\n')),
            _ => panic!("expected TextDelta"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — tool call
    // ========================================================================

    #[test]
    fn test_parse_tool_call_start() {
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_001","type":"function","function":{"name":"bash","arguments":""}}]},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "call_001");
                assert_eq!(name, "bash");
            }
            _ => panic!("expected ToolCallStart"),
        }
    }

    #[test]
    fn test_parse_tool_call_delta() {
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"com"}}]},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::ToolCallDelta {
                id: _,
                arguments_delta,
            } => {
                assert_eq!(arguments_delta, r#"{"com"#);
            }
            _ => panic!("expected ToolCallDelta"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — thinking / reasoning
    // ========================================================================

    #[test]
    fn test_parse_thinking_delta() {
        // OpenAI-style reasoning_content
        let data = r#"{"choices":[{"delta":{"reasoning_content":"thinking step"},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        assert!(matches!(
            event,
            AssistantMessageEvent::ThinkingDelta(s) if s == "thinking step"
        ));
    }

    #[test]
    fn test_parse_thinking_delta_empty() {
        let data = r#"{"choices":[{"delta":{"reasoning_content":""},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap();
        match event {
            Some(AssistantMessageEvent::ThinkingDelta(s)) => assert!(s.is_empty()),
            None => {} // also acceptable
            other => panic!("unexpected: {:?}", other),
        }
    }

    // ========================================================================
    // parse_sse_chunk — usage
    // ========================================================================

    #[test]
    fn test_parse_usage() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 100);
                assert_eq!(u.output, 50);
                assert_eq!(u.total_tokens, 150);
            }
            _ => panic!("expected Usage event"),
        }
    }

    #[test]
    fn test_parse_usage_with_cache() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":200,"completion_tokens":80,"total_tokens":280,"prompt_tokens_details":{"cached_tokens":50}}}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::Usage(u) => {
                assert_eq!(u.input, 200);
                assert_eq!(u.cache_read, 50);
            }
            _ => panic!("expected Usage event with cache"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — done / stop
    // ========================================================================

    #[test]
    fn test_parse_done_stop() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        assert!(
            matches!(event, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::Stop)
        );
    }

    #[test]
    fn test_parse_done_length() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"length","index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        assert!(
            matches!(event, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::Length)
        );
    }

    #[test]
    fn test_parse_done_tool_calls() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        assert!(
            matches!(event, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::ToolUse)
        );
    }

    #[test]
    fn test_parse_sse_done_marker() {
        // The special [DONE] marker
        let result = parse_sse_chunk("[DONE]").unwrap();
        assert!(result.is_none(), "[DONE] should return Ok(None)");
    }

    // ========================================================================
    // parse_sse_chunk — error handling
    // ========================================================================

    #[test]
    fn test_parse_malformed_json() {
        let data = r#"not valid json"#;
        let result = parse_sse_chunk(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_data() {
        let result = parse_sse_chunk("");
        // Empty data should be Ok(None) or Err — but not panic
        match result {
            Ok(None) => {}
            Err(_) => {}
            Ok(Some(_)) => panic!("empty data should not produce an event"),
        }
    }

    #[test]
    fn test_parse_missing_choices() {
        let data = r#"{"id":"chatcmpl-123"}"#;
        let result = parse_sse_chunk(data).unwrap();
        // Missing choices is not necessarily an error; it might just produce None
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_empty_choices_array() {
        let data = r#"{"choices":[]}"#;
        let result = parse_sse_chunk(data).unwrap();
        // No events from empty choices
        assert!(result.is_none());
    }

    // ========================================================================
    // IncrementalJsonParser
    // ========================================================================

    #[test]
    fn test_incremental_parser_new() {
        let parser = IncrementalJsonParser::new();
        assert!(parser.buffer().is_empty());
    }

    #[test]
    fn test_incremental_parser_push_single() {
        let mut parser = IncrementalJsonParser::new();
        parser.push(r#"{"command":"ls"}"#);
        assert_eq!(parser.buffer(), r#"{"command":"ls"}"#);
        let val = parser.complete().unwrap();
        assert_eq!(val["command"], "ls");
    }

    #[test]
    fn test_incremental_parser_push_multiple() {
        let mut parser = IncrementalJsonParser::new();
        parser.push(r#"{"com"#);
        parser.push(r#"mand"#);
        parser.push(r#"":"ls"#);
        parser.push(r#""}"#);
        assert_eq!(parser.buffer(), r#"{"command":"ls"}"#);
        let val = parser.complete().unwrap();
        assert_eq!(val["command"], "ls");
    }

    #[test]
    fn test_incremental_parser_incomplete_json() {
        let mut parser = IncrementalJsonParser::new();
        parser.push(r#"{"command"#);
        // Incomplete JSON should fail to parse
        assert!(parser.complete().is_err());
        assert_eq!(parser.buffer(), r#"{"command"#);
    }

    #[test]
    fn test_incremental_parser_nested_json() {
        let mut parser = IncrementalJsonParser::new();
        parser.push(r#"{"file_path":"/tmp/a.txt","content":"hello\nworld"}"#);
        let val = parser.complete().unwrap();
        assert_eq!(val["file_path"], "/tmp/a.txt");
        assert_eq!(val["content"], "hello\nworld");
    }

    #[test]
    fn test_incremental_parser_empty_object() {
        let mut parser = IncrementalJsonParser::new();
        parser.push("{}");
        let val = parser.complete().unwrap();
        assert!(val.is_object());
        assert!(val.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_incremental_parser_buffer_persists() {
        let mut parser = IncrementalJsonParser::new();
        parser.push("abc");
        parser.push("def");
        assert_eq!(parser.buffer(), "abcdef");
    }

    // ========================================================================
    // parse_sse_chunk — multiple choices
    // ========================================================================

    #[test]
    fn test_parse_sse_chunk_multiple_choices() {
        // Some providers may return multiple choices; we should handle index 0
        let data = r#"{"choices":[{"delta":{"content":"first"},"index":0},{"delta":{"content":"second"},"index":1}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        // Should parse the first choice (index 0)
        match event {
            AssistantMessageEvent::TextDelta(s) => assert_eq!(s, "first"),
            _ => panic!("expected TextDelta from first choice"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — content_filter finish_reason
    // ========================================================================

    #[test]
    fn test_parse_sse_chunk_content_filter_finish_reason() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"content_filter","index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        // content_filter should map to some Done/Error event
        match event {
            AssistantMessageEvent::Done { stop_reason } => {
                // content_filter might map to Error or Stop depending on implementation
                assert!(
                    stop_reason == StopReason::Error
                        || stop_reason == StopReason::Stop
                        || stop_reason == StopReason::Length,
                    "content_filter should map to a recognized stop reason"
                );
            }
            AssistantMessageEvent::Error(_) => {
                // Also acceptable: content_filter as an Error event
            }
            _ => panic!("content_filter finish_reason should produce Done or Error"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — content + tool_calls simultaneously
    // ========================================================================

    #[test]
    fn test_parse_sse_chunk_content_and_tool_calls() {
        // A delta that contains both content text and tool_calls
        let data = r#"{"choices":[{"delta":{"content":"thinking...","tool_calls":[{"index":0,"id":"call_mix","type":"function","function":{"name":"bash","arguments":""}}]},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        // The implementation should prioritize one or the other;
        // most commonly text content is emitted first
        match event {
            AssistantMessageEvent::TextDelta(s) => {
                assert_eq!(s, "thinking...");
            }
            AssistantMessageEvent::ToolCallStart { id, name } => {
                assert_eq!(id, "call_mix");
                assert_eq!(name, "bash");
            }
            _ => panic!("expected TextDelta or ToolCallStart when both are present"),
        }
    }

    // ========================================================================
    // parse_sse_chunk — error object in response
    // ========================================================================

    #[test]
    fn test_parse_sse_chunk_error_object() {
        let data = r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error","code":"rate_limit"}}"#;
        let result = parse_sse_chunk(data);
        // An error object should produce either an Error event or an Err result
        match result {
            Ok(Some(AssistantMessageEvent::Error(msg))) => {
                assert!(
                    msg.contains("rate") || msg.contains("Rate"),
                    "error message should contain rate limit info"
                );
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                assert!(
                    !err_msg.is_empty(),
                    "error should have a message"
                );
            }
            Ok(None) => {
                // Some implementations might ignore error objects — acceptable
            }
            Ok(Some(_)) => panic!("error object should not produce a non-error event"),
        }
    }

    // ========================================================================
    // IncrementalJsonParser — consecutive complete JSONs
    // ========================================================================

    #[test]
    fn test_incremental_parser_two_complete_jsons() {
        // Parse first complete JSON
        let mut parser = IncrementalJsonParser::new();
        parser.push(r#"{"a":1}"#);
        let val1 = parser.complete().unwrap();
        assert_eq!(val1["a"], 1);

        // Parse second complete JSON (simulating parser reuse or new parser)
        let mut parser2 = IncrementalJsonParser::new();
        parser2.push(r#"{"b":2}"#);
        let val2 = parser2.complete().unwrap();
        assert_eq!(val2["b"], 2);
    }

    // ========================================================================
    // IncrementalJsonParser — large JSON (~several KB arguments)
    // ========================================================================

    #[test]
    fn test_incremental_parser_large_json() {
        let mut parser = IncrementalJsonParser::new();
        // Build a large JSON string (~10KB of arguments)
        let large_value = "x".repeat(10_000);
        let json_str = format!(r#"{{"command":"{}"}}"#, large_value);
        // Feed in chunks
        let chunk_size = 512;
        for chunk in json_str.as_bytes().chunks(chunk_size) {
            parser.push(std::str::from_utf8(chunk).unwrap());
        }
        let val = parser.complete().unwrap();
        assert_eq!(val["command"].as_str().unwrap().len(), 10_000);
    }

    // ========================================================================
    // 边界: 超长 content delta
    // ========================================================================

    #[test]
    fn test_parse_very_large_text_delta() {
        let big_content = "a".repeat(100_000);
        let data = format!(
            r#"{{"choices":[{{"delta":{{"content":"{}"}},"index":0}}]}}"#,
            big_content
        );
        let event = parse_sse_chunk(&data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::TextDelta(s) => assert_eq!(s.len(), 100_000),
            _ => panic!("expected TextDelta"),
        }
    }

    // ========================================================================
    // 边界: Unicode / emoji / 零宽字符 delta
    // ========================================================================

    #[test]
    fn test_parse_unicode_chinese_delta() {
        let data = r#"{"choices":[{"delta":{"content":"你好世界🌍"},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::TextDelta(s) => {
                assert!(s.contains("你好"));
                assert!(s.contains("🌍"));
            }
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn test_parse_zero_width_chars_delta() {
        // Zero-width joiner + zero-width space
        let data = r#"{"choices":[{"delta":{"content":"a\u200Db\u200Bc"},"index":0}]}"#;
        let event = parse_sse_chunk(data).unwrap().unwrap();
        match event {
            AssistantMessageEvent::TextDelta(s) => assert!(s.contains('a')),
            _ => panic!("expected TextDelta"),
        }
    }

    // ========================================================================
    // 错误: content 为 null
    // ========================================================================

    #[test]
    fn test_parse_null_content_delta() {
        let data = r#"{"choices":[{"delta":{"content":null},"index":0}]}"#;
        let result = parse_sse_chunk(data).unwrap();
        // null content should be treated as no content — Ok(None) or empty delta
        match result {
            None => {}
            Some(AssistantMessageEvent::TextDelta(s)) => assert!(s.is_empty()),
            other => panic!("unexpected for null content: {:?}", other),
        }
    }

    // ========================================================================
    // 错误: 未知 finish_reason
    // ========================================================================

    #[test]
    fn test_parse_unknown_finish_reason() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"unknown_reason_xyz","index":0}]}"#;
        let result = parse_sse_chunk(data);
        // Unknown finish_reason should not panic — map to Done/Error or Ok(None)
        match result {
            Ok(Some(AssistantMessageEvent::Done { .. })) => {}
            Ok(Some(AssistantMessageEvent::Error(_))) => {}
            Ok(None) => {}
            Err(_) => {}
            Ok(Some(other)) => panic!("unexpected event for unknown finish_reason: {:?}", other),
        }
    }

    // ========================================================================
    // 状态组合: 完整 SSE 事件流序列
    // ========================================================================

    #[test]
    fn test_parse_complete_text_stream_sequence() {
        // Simulate a complete text generation: 3 text deltas → usage → done
        let chunks = vec![
            r#"{"choices":[{"delta":{"content":"Hello"},"index":0}]}"#,
            r#"{"choices":[{"delta":{"content":" "},"index":0}]}"#,
            r#"{"choices":[{"delta":{"content":"world"},"index":0}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#,
        ];

        let mut text = String::new();
        let mut got_usage = false;
        let mut got_done = false;

        for chunk in chunks {
            if let Ok(Some(event)) = parse_sse_chunk(chunk) {
                match event {
                    AssistantMessageEvent::TextDelta(s) => text.push_str(&s),
                    AssistantMessageEvent::Usage(u) => {
                        got_usage = true;
                        assert_eq!(u.total_tokens, 13);
                    }
                    AssistantMessageEvent::Done { stop_reason } => {
                        got_done = true;
                        assert_eq!(stop_reason, StopReason::Stop);
                    }
                    _ => {}
                }
            }
        }

        assert_eq!(text, "Hello world");
        assert!(got_usage, "should have received usage event");
        assert!(got_done, "should have received done event");
    }

    // ========================================================================
    // 状态组合: tool_call 分段解析完整状态机
    // ========================================================================

    #[test]
    fn test_parse_complete_tool_call_sequence() {
        // ToolCallStart → multiple ToolCallDeltas → Done(ToolUse)
        let chunks = vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_seq","type":"function","function":{"name":"read","arguments":""}}]},"index":0}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]},"index":0}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\""}}]},"index":0}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"/tmp\"}"}}]},"index":0}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#,
        ];

        let mut got_start = false;
        let mut args_buffer = String::new();
        let mut got_done = false;

        for chunk in chunks {
            if let Ok(Some(event)) = parse_sse_chunk(chunk) {
                match event {
                    AssistantMessageEvent::ToolCallStart { id, name } => {
                        got_start = true;
                        assert_eq!(id, "call_seq");
                        assert_eq!(name, "read");
                    }
                    AssistantMessageEvent::ToolCallDelta {
                        arguments_delta, ..
                    } => {
                        args_buffer.push_str(&arguments_delta);
                    }
                    AssistantMessageEvent::Done { stop_reason } => {
                        got_done = true;
                        assert_eq!(stop_reason, StopReason::ToolUse);
                    }
                    _ => {}
                }
            }
        }

        assert!(got_start, "should have received ToolCallStart");
        assert!(got_done, "should have received Done");
        // Accumulated arguments should form valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&args_buffer).unwrap();
        assert_eq!(parsed["path"], "/tmp");
    }
}
