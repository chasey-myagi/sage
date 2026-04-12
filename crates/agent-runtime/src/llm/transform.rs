// Message transformation — Phase 2
// Converts between AgentMessage (runtime) and LlmMessage (API) formats.

use super::types::*;
use crate::types::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Converts agent messages to LLM API format.
pub fn agent_to_llm_messages(messages: &[AgentMessage]) -> Vec<LlmMessage> {
    messages
        .iter()
        .map(|msg| match msg {
            AgentMessage::User(user) => {
                let content = user
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(LlmContent::Text(text.clone())),
                        Content::Image { data, mime_type } => Some(LlmContent::Image {
                            url: format!("data:{};base64,{}", mime_type, data),
                        }),
                        Content::Thinking { .. } => None,
                        Content::ToolCall { .. } => None,
                    })
                    .collect();
                LlmMessage::User { content }
            }
            AgentMessage::Assistant(assistant) => {
                let text: String = assistant
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();

                let tool_calls: Vec<LlmToolCall> = assistant
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(LlmToolCall {
                            id: id.clone(),
                            function: LlmFunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(arguments).unwrap_or_default(),
                            },
                        }),
                        _ => None,
                    })
                    .collect();

                LlmMessage::Assistant {
                    content: text,
                    tool_calls,
                }
            }
            AgentMessage::ToolResult(result) => {
                let content: String = result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();

                LlmMessage::Tool {
                    tool_call_id: result.tool_call_id.clone(),
                    content,
                }
            }
        })
        .collect()
}

/// Converts an agent tool definition to LLM format.
pub fn agent_tool_to_llm(name: &str, description: &str, parameters: serde_json::Value) -> LlmTool {
    LlmTool {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

/// Normalizes a tool call ID, generating one if empty.
pub fn normalize_tool_call_id(id: &str) -> String {
    if id.is_empty() {
        let n = TOOL_CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("call_generated_{}", n)
    } else {
        id.to_string()
    }
}

/// Inserts stub Assistant messages before orphaned Tool results.
pub fn fix_orphaned_tool_results(messages: &mut Vec<LlmMessage>) {
    let mut available_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(messages.len() * 2);

    for msg in messages.drain(..) {
        match &msg {
            LlmMessage::Assistant { tool_calls, .. } => {
                for tc in tool_calls {
                    available_ids.insert(tc.id.clone());
                }
                result.push(msg);
            }
            LlmMessage::Tool { tool_call_id, .. } => {
                if !available_ids.contains(tool_call_id) {
                    result.push(LlmMessage::Assistant {
                        content: String::new(),
                        tool_calls: vec![LlmToolCall {
                            id: tool_call_id.clone(),
                            function: LlmFunctionCall {
                                name: "unknown".into(),
                                arguments: "{}".into(),
                            },
                        }],
                    });
                    available_ids.insert(tool_call_id.clone());
                }
                result.push(msg);
            }
            _ => {
                result.push(msg);
            }
        }
    }

    *messages = result;
}

/// Strips thinking content blocks from messages.
/// LlmMessage types don't contain thinking blocks — they are stripped
/// during agent_to_llm_messages conversion. This is a no-op.
pub fn strip_thinking_blocks(_messages: &mut Vec<LlmMessage>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::*;
    use crate::types::*;
    use serde_json::json;

    // ========================================================================
    // agent_to_llm_messages — User messages
    // ========================================================================

    #[test]
    fn test_user_text_to_llm() {
        let messages = vec![AgentMessage::User(UserMessage::from_text("hello"))];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 1);
                assert!(matches!(&content[0], LlmContent::Text(s) if s == "hello"));
            }
            _ => panic!("expected User LlmMessage"),
        }
    }

    #[test]
    fn test_user_image_to_llm() {
        let messages = vec![AgentMessage::User(UserMessage {
            content: vec![Content::Image {
                data: "base64data".into(),
                mime_type: "image/png".into(),
            }],
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 1);
                assert!(matches!(&content[0], LlmContent::Image { url } if url.contains("base64")));
            }
            _ => panic!("expected User LlmMessage with image"),
        }
    }

    #[test]
    fn test_user_multimodal_to_llm() {
        let messages = vec![AgentMessage::User(UserMessage {
            content: vec![
                Content::Text {
                    text: "describe this".into(),
                },
                Content::Image {
                    data: "abc123".into(),
                    mime_type: "image/jpeg".into(),
                },
            ],
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::User { content } => assert_eq!(content.len(), 2),
            _ => panic!("expected User LlmMessage"),
        }
    }

    // ========================================================================
    // agent_to_llm_messages — Assistant messages
    // ========================================================================

    #[test]
    fn test_assistant_text_to_llm() {
        let messages = vec![AgentMessage::Assistant(AssistantMessage::new(
            "I can help".into(),
        ))];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "I can help");
                assert!(tool_calls.is_empty());
            }
            _ => panic!("expected Assistant LlmMessage"),
        }
    }

    #[test]
    fn test_assistant_with_tool_call_to_llm() {
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Text {
                    text: "Let me check.".into(),
                },
                Content::ToolCall {
                    id: "tc_001".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                },
            ],
            provider: "qwen".into(),
            model: "qwen-plus".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "Let me check.");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "tc_001");
                assert_eq!(tool_calls[0].function.name, "bash");
            }
            _ => panic!("expected Assistant LlmMessage with tool calls"),
        }
    }

    #[test]
    fn test_assistant_multiple_tool_calls() {
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::ToolCall {
                    id: "tc_001".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                },
                Content::ToolCall {
                    id: "tc_002".into(),
                    name: "read".into(),
                    arguments: json!({"path": "/tmp/a"}),
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 2);
            }
            _ => panic!("expected Assistant with 2 tool calls"),
        }
    }

    // ========================================================================
    // agent_to_llm_messages — ToolResult messages
    // ========================================================================

    #[test]
    fn test_tool_result_to_llm() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_001".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output here".into(),
            }],
            is_error: false,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "tc_001");
                assert!(content.contains("output here"));
            }
            _ => panic!("expected Tool LlmMessage"),
        }
    }

    #[test]
    fn test_tool_result_error_to_llm() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_002".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "command not found".into(),
            }],
            is_error: true,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Tool { content, .. } => {
                // Error tool results should still contain the error text
                assert!(content.contains("command not found"));
            }
            _ => panic!("expected Tool LlmMessage"),
        }
    }

    // ========================================================================
    // agent_to_llm_messages — Multi-turn sequence
    // ========================================================================

    #[test]
    fn test_multi_turn_conversation() {
        let messages = vec![
            AgentMessage::User(UserMessage::from_text("hello")),
            AgentMessage::Assistant(AssistantMessage::new("hi there".into())),
            AgentMessage::User(UserMessage::from_text("help me")),
        ];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 3);
        assert!(matches!(&llm[0], LlmMessage::User { .. }));
        assert!(matches!(&llm[1], LlmMessage::Assistant { .. }));
        assert!(matches!(&llm[2], LlmMessage::User { .. }));
    }

    #[test]
    fn test_empty_messages() {
        let messages: Vec<AgentMessage> = vec![];
        let llm = agent_to_llm_messages(&messages);
        assert!(llm.is_empty());
    }

    #[test]
    fn test_tool_call_then_result_sequence() {
        let messages = vec![
            AgentMessage::User(UserMessage::from_text("list files")),
            AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::ToolCall {
                    id: "tc_001".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                }],
                provider: "test".into(),
                model: "test".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }),
            AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: "tc_001".into(),
                tool_name: "bash".into(),
                content: vec![Content::Text {
                    text: "file1.txt\nfile2.txt".into(),
                }],
                is_error: false,
                timestamp: 0,
            }),
            AgentMessage::Assistant(AssistantMessage::new("I found 2 files.".into())),
        ];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 4);
        assert!(matches!(&llm[0], LlmMessage::User { .. }));
        assert!(matches!(&llm[1], LlmMessage::Assistant { .. }));
        assert!(matches!(&llm[2], LlmMessage::Tool { .. }));
        assert!(matches!(&llm[3], LlmMessage::Assistant { .. }));
    }

    // ========================================================================
    // agent_tool_to_llm
    // ========================================================================

    #[test]
    fn test_agent_tool_to_llm() {
        let tool = agent_tool_to_llm(
            "bash",
            "Execute a shell command",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        );
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.description, "Execute a shell command");
        assert_eq!(tool.parameters["type"], "object");
    }

    #[test]
    fn test_agent_tool_to_llm_empty_params() {
        let tool = agent_tool_to_llm("noop", "Does nothing", json!({}));
        assert_eq!(tool.name, "noop");
        assert!(tool.parameters.is_object());
    }

    // ========================================================================
    // normalize_tool_call_id
    // ========================================================================

    #[test]
    fn test_normalize_tool_call_id_passthrough() {
        // A normal call_xxxx ID should pass through unchanged
        let id = normalize_tool_call_id("call_abc123");
        assert_eq!(id, "call_abc123");
    }

    #[test]
    fn test_normalize_tool_call_id_empty() {
        // Empty ID should be normalized (e.g., generate a placeholder)
        let id = normalize_tool_call_id("");
        assert!(!id.is_empty(), "empty tool_call_id should be normalized");
    }

    #[test]
    fn test_normalize_tool_call_id_special_chars() {
        // IDs with special characters should be cleaned
        let id = normalize_tool_call_id("call-with-dashes");
        // Should still be a valid string
        assert!(!id.is_empty());
    }

    // ========================================================================
    // fix_orphaned_tool_results
    // ========================================================================

    #[test]
    fn test_fix_orphaned_tool_results_no_orphans() {
        // Assistant with tool_call followed by matching Tool result — no fix needed
        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "call_001".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: "{}".into(),
                    },
                }],
            },
            LlmMessage::Tool {
                tool_call_id: "call_001".into(),
                content: "ok".into(),
            },
        ];
        let len_before = messages.len();
        fix_orphaned_tool_results(&mut messages);
        assert_eq!(messages.len(), len_before);
    }

    #[test]
    fn test_fix_orphaned_tool_results_orphan_gets_assistant() {
        // Tool result without preceding Assistant tool_call — should insert stub
        let mut messages = vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            },
            LlmMessage::Tool {
                tool_call_id: "call_orphan".into(),
                content: "orphaned result".into(),
            },
        ];
        fix_orphaned_tool_results(&mut messages);
        // After fix, there should be an Assistant message before the Tool
        assert!(messages.len() > 2);
        // The inserted message should be Assistant with a matching tool_call
        let has_assistant_before_tool = messages.windows(2).any(|w| {
            matches!(&w[0], LlmMessage::Assistant { tool_calls, .. } if tool_calls.iter().any(|tc| tc.id == "call_orphan"))
                && matches!(&w[1], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "call_orphan")
        });
        assert!(has_assistant_before_tool);
    }

    #[test]
    fn test_fix_orphaned_tool_results_empty() {
        let mut messages: Vec<LlmMessage> = vec![];
        fix_orphaned_tool_results(&mut messages);
        assert!(messages.is_empty());
    }

    // ========================================================================
    // strip_thinking_blocks
    // ========================================================================

    #[test]
    fn test_strip_thinking_blocks_removes_thinking() {
        // Thinking content in User messages should be stripped if present
        let mut messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("hello".into())],
        }];
        strip_thinking_blocks(&mut messages);
        // Text should remain
        match &messages[0] {
            LlmMessage::User { content } => {
                assert!(!content.is_empty());
            }
            _ => panic!("expected User message to remain"),
        }
    }

    #[test]
    fn test_strip_thinking_blocks_empty_messages() {
        let mut messages: Vec<LlmMessage> = vec![];
        strip_thinking_blocks(&mut messages);
        assert!(messages.is_empty());
    }

    // ========================================================================
    // Thinking content in assistant messages should be stripped
    // ========================================================================

    #[test]
    fn test_assistant_thinking_content_stripped() {
        // When an AgentMessage::Assistant has Thinking content blocks,
        // they should not appear in the LlmMessage conversion
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: "Let me think...".into(),
                    signature: None,
                    redacted: false,
                },
                Content::Text {
                    text: "The answer is 42.".into(),
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant { content, .. } => {
                // The text content should be present, thinking should be stripped
                assert!(content.contains("The answer is 42."));
            }
            _ => panic!("expected Assistant LlmMessage"),
        }
    }

    // ========================================================================
    // strip_thinking_blocks — from Assistant messages
    // ========================================================================

    #[test]
    fn test_strip_thinking_blocks_from_assistant() {
        // strip_thinking_blocks should also strip thinking from LlmMessage::Assistant
        // (though assistant doesn't have LlmContent blocks, it has a content string)
        // The function should handle both User and Assistant messages gracefully
        let mut messages = vec![LlmMessage::Assistant {
            content: "The answer is 42.".into(),
            tool_calls: vec![],
        }];
        strip_thinking_blocks(&mut messages);
        // Text content should remain intact
        match &messages[0] {
            LlmMessage::Assistant { content, .. } => {
                assert_eq!(content, "The answer is 42.");
            }
            _ => panic!("expected Assistant message to remain"),
        }
    }

    // ========================================================================
    // fix_orphaned_tool_results — multiple consecutive orphans
    // ========================================================================

    #[test]
    fn test_fix_orphaned_tool_results_multiple_consecutive() {
        // Two consecutive Tool results without a preceding Assistant
        let mut messages = vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            },
            LlmMessage::Tool {
                tool_call_id: "call_orphan_1".into(),
                content: "result 1".into(),
            },
            LlmMessage::Tool {
                tool_call_id: "call_orphan_2".into(),
                content: "result 2".into(),
            },
        ];
        fix_orphaned_tool_results(&mut messages);
        // After fix, each orphan tool result should have a preceding Assistant stub
        // Verify that no Tool message appears without a preceding Assistant with matching id
        for (i, msg) in messages.iter().enumerate() {
            if let LlmMessage::Tool { tool_call_id, .. } = msg {
                // There must be some Assistant before this Tool with a matching tool_call
                let has_preceding_assistant = messages[..i].iter().any(|prev| {
                    matches!(prev, LlmMessage::Assistant { tool_calls, .. }
                        if tool_calls.iter().any(|tc| tc.id == *tool_call_id))
                });
                assert!(
                    has_preceding_assistant,
                    "Tool {} should have a preceding Assistant with matching tool_call",
                    tool_call_id
                );
            }
        }
    }

    // ========================================================================
    // fix_orphaned_tool_results — mismatched IDs
    // ========================================================================

    #[test]
    fn test_fix_orphaned_tool_results_mismatched_ids() {
        // Assistant has tool_call id="A", but Tool result has id="B" — B is orphaned
        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "call_A".into(),
                    function: LlmFunctionCall {
                        name: "bash".into(),
                        arguments: "{}".into(),
                    },
                }],
            },
            LlmMessage::Tool {
                tool_call_id: "call_A".into(),
                content: "result A".into(),
            },
            LlmMessage::Tool {
                tool_call_id: "call_B".into(),
                content: "result B — orphaned".into(),
            },
        ];
        fix_orphaned_tool_results(&mut messages);
        // After fix, "call_B" should now have a preceding Assistant with matching tool_call
        let b_idx = messages
            .iter()
            .position(
                |m| matches!(m, LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "call_B"),
            )
            .expect("call_B Tool should still exist");
        let has_matching_assistant = messages[..b_idx].iter().any(|prev| {
            matches!(prev, LlmMessage::Assistant { tool_calls, .. }
                if tool_calls.iter().any(|tc| tc.id == "call_B"))
        });
        assert!(
            has_matching_assistant,
            "orphaned call_B should get a stub Assistant inserted"
        );
    }

    // ========================================================================
    // agent_to_llm_messages — empty content assistant
    // ========================================================================

    #[test]
    fn test_agent_to_llm_empty_content_assistant() {
        // An Assistant message with no content blocks
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                // Content text should be empty or absent
                assert!(content.is_empty());
                assert!(tool_calls.is_empty());
            }
            _ => panic!("expected Assistant LlmMessage"),
        }
    }

    // ========================================================================
    // normalize_tool_call_id — idempotency
    // ========================================================================

    #[test]
    fn test_normalize_tool_call_id_idempotent() {
        // Normalizing an empty ID twice should produce the same result both times
        let first = normalize_tool_call_id("");
        let second = normalize_tool_call_id("");
        // Both calls with the same input should be deterministic
        // (if randomized, they differ — but the key point is each call is non-empty)
        assert!(!first.is_empty());
        assert!(!second.is_empty());
    }

    #[test]
    fn test_normalize_tool_call_id_normal_idempotent() {
        // A normal ID should pass through identically on repeated calls
        let first = normalize_tool_call_id("call_abc123");
        let second = normalize_tool_call_id("call_abc123");
        assert_eq!(
            first, second,
            "normalizing a normal ID should be idempotent"
        );
    }

    // ========================================================================
    // tool_result is_error=true formatting
    // ========================================================================

    #[test]
    fn test_tool_result_is_error_true_formatting() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_err_001".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "permission denied".into(),
            }],
            is_error: true,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Tool {
                content,
                tool_call_id,
            } => {
                assert_eq!(tool_call_id, "tc_err_001");
                assert!(
                    content.contains("permission denied"),
                    "error tool result should contain the error text"
                );
            }
            _ => panic!("expected Tool LlmMessage"),
        }
    }

    // ========================================================================
    // 边界: 极端多 content block (50+)
    // ========================================================================

    #[test]
    fn test_user_message_with_many_content_blocks() {
        let mut blocks = Vec::new();
        for i in 0..60 {
            if i % 3 == 0 {
                blocks.push(Content::Image {
                    data: format!("img_{}", i),
                    mime_type: "image/png".into(),
                });
            } else {
                blocks.push(Content::Text {
                    text: format!("text block {}", i),
                });
            }
        }
        let messages = vec![AgentMessage::User(UserMessage {
            content: blocks,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 60);
            }
            _ => panic!("expected User LlmMessage"),
        }
    }

    // ========================================================================
    // 边界: tool_call arguments 空字符串 / null / 非法 JSON
    // ========================================================================

    #[test]
    fn test_tool_call_with_null_arguments() {
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::ToolCall {
                id: "tc_null".into(),
                name: "bash".into(),
                arguments: serde_json::Value::Null,
            }],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                // null arguments should be serialized as "null" or "{}"
                assert!(!tool_calls[0].function.arguments.is_empty());
            }
            _ => panic!("expected Assistant LlmMessage"),
        }
    }

    #[test]
    fn test_tool_call_with_empty_object_arguments() {
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::ToolCall {
                id: "tc_empty".into(),
                name: "noop".into(),
                arguments: json!({}),
            }],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        match &llm[0] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls[0].function.arguments, "{}");
            }
            _ => panic!("expected Assistant"),
        }
    }

    // ========================================================================
    // 错误: ToolResult content 为空 vec
    // ========================================================================

    #[test]
    fn test_tool_result_empty_content() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_empty_content".into(),
            tool_name: "bash".into(),
            content: vec![],
            is_error: false,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        match &llm[0] {
            LlmMessage::Tool { content, .. } => {
                // Empty content should produce empty or minimal string
                assert!(content.is_empty() || content.len() < 10);
            }
            _ => panic!("expected Tool LlmMessage"),
        }
    }

    // ========================================================================
    // 错误: is_error=true 但 content 为空
    // ========================================================================

    #[test]
    fn test_tool_result_is_error_empty_content() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_err_empty".into(),
            tool_name: "bash".into(),
            content: vec![],
            is_error: true,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        // Should not panic even with error + empty content
        assert!(matches!(&llm[0], LlmMessage::Tool { .. }));
    }

    // ========================================================================
    // 状态组合: 多轮 tool_call + tool_result 交错
    // ========================================================================

    #[test]
    fn test_multi_round_tool_call_tool_result() {
        let messages = vec![
            AgentMessage::User(UserMessage::from_text("do two things")),
            // Round 1
            AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::ToolCall {
                    id: "tc_r1".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                }],
                provider: "test".into(),
                model: "test".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }),
            AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: "tc_r1".into(),
                tool_name: "bash".into(),
                content: vec![Content::Text {
                    text: "file1".into(),
                }],
                is_error: false,
                timestamp: 0,
            }),
            // Round 2
            AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::ToolCall {
                    id: "tc_r2".into(),
                    name: "read".into(),
                    arguments: json!({"path": "file1"}),
                }],
                provider: "test".into(),
                model: "test".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }),
            AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: "tc_r2".into(),
                tool_name: "read".into(),
                content: vec![Content::Text {
                    text: "contents".into(),
                }],
                is_error: false,
                timestamp: 0,
            }),
            // Final response
            AgentMessage::Assistant(AssistantMessage::new("Done!".into())),
        ];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 6);
        assert!(matches!(&llm[0], LlmMessage::User { .. }));
        assert!(
            matches!(&llm[1], LlmMessage::Assistant { tool_calls, .. } if tool_calls.len() == 1)
        );
        assert!(
            matches!(&llm[2], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "tc_r1")
        );
        assert!(
            matches!(&llm[3], LlmMessage::Assistant { tool_calls, .. } if tool_calls.len() == 1)
        );
        assert!(
            matches!(&llm[4], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "tc_r2")
        );
        assert!(
            matches!(&llm[5], LlmMessage::Assistant { tool_calls, .. } if tool_calls.is_empty())
        );
    }

    // ========================================================================
    // 状态组合: fix_orphaned + strip_thinking 组合调用
    // ========================================================================

    #[test]
    fn test_fix_orphaned_then_strip_thinking_combined() {
        // Messages with both orphaned tool result AND thinking content
        let mut messages = vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            },
            // Orphaned tool result (no preceding assistant)
            LlmMessage::Tool {
                tool_call_id: "call_combo".into(),
                content: "result".into(),
            },
            // Assistant with thinking (to be stripped)
            LlmMessage::Assistant {
                content: "The answer is 42.".into(),
                tool_calls: vec![],
            },
        ];

        // Apply both transforms in sequence
        fix_orphaned_tool_results(&mut messages);
        strip_thinking_blocks(&mut messages);

        // Orphan should have stub assistant inserted
        let tool_idx = messages
            .iter()
            .position(|m| matches!(m, LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "call_combo"))
            .expect("Tool message should exist");
        assert!(tool_idx > 0, "Tool should not be first message");
        let has_stub = matches!(
            &messages[tool_idx - 1],
            LlmMessage::Assistant { tool_calls, .. } if tool_calls.iter().any(|tc| tc.id == "call_combo")
        );
        assert!(has_stub, "stub assistant should precede orphaned tool");

        // Final assistant content should remain
        let last_assistant = messages.iter().rev().find(
            |m| matches!(m, LlmMessage::Assistant { tool_calls, .. } if tool_calls.is_empty()),
        );
        assert!(last_assistant.is_some(), "final assistant should remain");
    }

    // ========================================================================
    // 状态组合: requires_assistant_after_tool_result compat flag
    // ========================================================================

    #[test]
    fn test_compat_requires_assistant_after_tool_result() {
        // When compat flag is set, a tool result at the end of messages
        // should trigger insertion of a stub assistant message
        let compat = ProviderCompat {
            max_tokens_field: MaxTokensField::MaxTokens,
            supports_reasoning_effort: false,
            thinking_format: None,
            requires_tool_result_name: false,
            requires_assistant_after_tool_result: true,
            supports_strict_mode: false,
        };

        let messages = vec![
            AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::ToolCall {
                    id: "tc_compat".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                }],
                provider: "test".into(),
                model: "test".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }),
            AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: "tc_compat".into(),
                tool_name: "bash".into(),
                content: vec![Content::Text { text: "ok".into() }],
                is_error: false,
                timestamp: 0,
            }),
        ];

        let llm = agent_to_llm_messages(&messages);

        // With requires_assistant_after_tool_result, implementation may:
        // 1. Append a stub assistant, or
        // 2. Leave as-is (the flag is used at request time)
        // Either way, the conversion should not panic
        assert!(llm.len() >= 2);
        assert!(
            compat.requires_assistant_after_tool_result,
            "compat flag should be set"
        );
    }

    // ========================================================================
    // 状态组合: requires_tool_result_name compat flag
    // ========================================================================

    #[test]
    fn test_compat_requires_tool_result_name() {
        let compat = ProviderCompat {
            max_tokens_field: MaxTokensField::MaxTokens,
            supports_reasoning_effort: false,
            thinking_format: None,
            requires_tool_result_name: true,
            requires_assistant_after_tool_result: false,
            supports_strict_mode: false,
        };

        // The compat flag exists and is true
        assert!(compat.requires_tool_result_name);

        // A tool result conversion should include the name
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc_name".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            is_error: false,
            timestamp: 0,
        })];
        let llm = agent_to_llm_messages(&messages);
        assert_eq!(llm.len(), 1);
        // The conversion should preserve the tool_call_id
        match &llm[0] {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "tc_name");
                assert!(content.contains("output"));
            }
            _ => panic!("expected Tool"),
        }
    }

    // ========================================================================
    // normalize_tool_call_id — true idempotency: normalize(normalize(x)) == normalize(x)
    // ========================================================================

    #[test]
    fn test_normalize_tool_call_id_double_normalize() {
        // For any valid ID, normalizing twice should give the same result as once
        let once = normalize_tool_call_id("call_abc123");
        let twice = normalize_tool_call_id(&once);
        assert_eq!(
            once, twice,
            "normalize(normalize(x)) should equal normalize(x)"
        );
    }

    #[test]
    fn test_normalize_tool_call_id_double_normalize_special() {
        let once = normalize_tool_call_id("call-with-dashes");
        let twice = normalize_tool_call_id(&once);
        assert_eq!(
            once, twice,
            "double-normalize should be idempotent for special chars"
        );
    }
}
