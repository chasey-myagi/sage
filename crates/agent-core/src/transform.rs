// Message transformation — converts between AgentMessage and LlmMessage formats.
// Adapted from ai/src/transform.rs for agent-core types (ToolResultMessage has details field).

use crate::types::*;
use ai::types::{
    LlmContent, LlmFunctionCall, LlmMessage, LlmTool, LlmToolCall, ThinkingBlock,
};
use std::sync::atomic::{AtomicU64, Ordering};

static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Extract thinking blocks from a Content slice into ThinkingBlock vec.
pub fn extract_thinking_blocks(content: &[Content]) -> Vec<ThinkingBlock> {
    content
        .iter()
        .filter_map(|c| match c {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => Some(ThinkingBlock {
                thinking: thinking.clone(),
                signature: signature.clone(),
                redacted: *redacted,
            }),
            _ => None,
        })
        .collect()
}

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

                let thinking_blocks = extract_thinking_blocks(&assistant.content);

                LlmMessage::Assistant {
                    content: text,
                    tool_calls,
                    thinking_blocks,
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
                    tool_name: None,
                }
            }
            AgentMessage::CompactionSummary(cs) => LlmMessage::User {
                content: vec![LlmContent::Text(format!(
                    "[Previous conversation summary]\n\n{}",
                    cs.summary
                ))],
            },
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
                        thinking_blocks: vec![],
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

/// Strips thinking content blocks from messages (no-op — thinking lives in thinking_blocks field).
pub fn strip_thinking_blocks(_messages: &mut Vec<LlmMessage>) {}

/// Strip cross-model thinking content from assistant messages (no-op until provenance tracking).
pub fn strip_cross_model_thinking(
    _messages: &mut Vec<LlmMessage>,
    _target_provider: &str,
    _target_model: &str,
) {
}

/// Remove assistant messages that have empty content AND no tool calls.
pub fn skip_empty_assistant_messages(messages: &mut Vec<LlmMessage>) {
    messages.retain(|msg| {
        if let LlmMessage::Assistant {
            content,
            tool_calls,
            thinking_blocks,
        } = msg
        {
            !content.is_empty() || !tool_calls.is_empty() || !thinking_blocks.is_empty()
        } else {
            true
        }
    });
}

/// Normalize tool-call IDs across all messages for cross-provider compatibility.
pub fn normalize_tool_call_ids(messages: &mut Vec<LlmMessage>, max_len: usize) {
    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for msg in messages.iter() {
        if let LlmMessage::Assistant { tool_calls, .. } = msg {
            for tc in tool_calls {
                let normalized = sanitize_id(&tc.id, max_len);
                if normalized != tc.id {
                    id_map.insert(tc.id.clone(), normalized);
                }
            }
        }
    }

    if id_map.is_empty() {
        return;
    }

    for msg in messages.iter_mut() {
        match msg {
            LlmMessage::Assistant { tool_calls, .. } => {
                for tc in tool_calls.iter_mut() {
                    if let Some(new_id) = id_map.get(&tc.id) {
                        tc.id = new_id.clone();
                    }
                }
            }
            LlmMessage::Tool { tool_call_id, .. } => {
                if let Some(new_id) = id_map.get(tool_call_id.as_str()) {
                    *tool_call_id = new_id.clone();
                }
            }
            _ => {}
        }
    }
}

fn sanitize_id(id: &str, max_len: usize) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() > max_len {
        sanitized[..max_len].to_string()
    } else {
        sanitized
    }
}

/// Apply all message transforms in canonical order.
pub fn transform_messages(
    messages: &mut Vec<LlmMessage>,
    target_provider: &str,
    target_model: &str,
) {
    skip_empty_assistant_messages(messages);
    fix_orphaned_tool_results(messages);
    normalize_tool_call_ids(messages, 64);
    strip_cross_model_thinking(messages, target_provider, target_model);
}
