/// Shared utilities for compaction and branch summarization.
///
/// Mirrors pi-mono packages/coding-agent/src/core/compaction/utils.ts
use serde_json::Value;

// ============================================================================
// File Operation Tracking
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct FileOperations {
    pub read: std::collections::HashSet<String>,
    pub written: std::collections::HashSet<String>,
    pub edited: std::collections::HashSet<String>,
}

pub fn create_file_ops() -> FileOperations {
    FileOperations::default()
}

/// Extract file operations from tool calls in an assistant message (serialized JSON value).
pub fn extract_file_ops_from_message(message: &Value, file_ops: &mut FileOperations) {
    let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
    if role != "assistant" {
        return;
    }

    let content = match message.get("content").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for block in content {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if block_type != "toolCall" {
            continue;
        }

        let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let path = block
            .get("arguments")
            .and_then(|a| a.get("path"))
            .and_then(|p| p.as_str());

        if let Some(p) = path {
            match name {
                "read" => {
                    file_ops.read.insert(p.to_string());
                }
                "write" => {
                    file_ops.written.insert(p.to_string());
                }
                "edit" => {
                    file_ops.edited.insert(p.to_string());
                }
                _ => {}
            }
        }
    }
}

/// Compute final file lists from file operations.
/// Returns read_files (files only read, not modified) and modified_files.
pub fn compute_file_lists(file_ops: &FileOperations) -> (Vec<String>, Vec<String>) {
    let mut modified: std::collections::HashSet<String> = file_ops.edited.clone();
    for f in &file_ops.written {
        modified.insert(f.clone());
    }

    let mut read_only: Vec<String> = file_ops
        .read
        .iter()
        .filter(|f| !modified.contains(*f))
        .cloned()
        .collect();
    read_only.sort();

    let mut modified_files: Vec<String> = modified.into_iter().collect();
    modified_files.sort();

    (read_only, modified_files)
}

/// Format file operations as XML tags for summary.
pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
    let mut sections = Vec::new();

    if !read_files.is_empty() {
        sections.push(format!(
            "<read-files>\n{}\n</read-files>",
            read_files.join("\n")
        ));
    }
    if !modified_files.is_empty() {
        sections.push(format!(
            "<modified-files>\n{}\n</modified-files>",
            modified_files.join("\n")
        ));
    }
    if sections.is_empty() {
        return String::new();
    }
    format!("\n\n{}", sections.join("\n\n"))
}

// ============================================================================
// Message Serialization
// ============================================================================

/// Maximum characters for a tool result in serialized summaries.
const TOOL_RESULT_MAX_CHARS: usize = 2000;

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let truncated_chars = text.len() - max_chars;
    format!(
        "{}\n\n[... {} more characters truncated]",
        &text[..max_chars],
        truncated_chars
    )
}

/// Serialize LLM messages (as JSON Values) to text for summarization.
/// Call this after converting custom message types to standard user/assistant/toolResult.
pub fn serialize_conversation(messages: &[Value]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        match role {
            "user" => {
                let content = msg.get("content");
                let text = match content {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|c| {
                            if c.get("type")?.as_str()? == "text" {
                                c.get("text")?.as_str().map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    parts.push(format!("[User]: {}", text));
                }
            }
            "assistant" => {
                let content = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };

                let mut text_parts: Vec<String> = Vec::new();
                let mut thinking_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<String> = Vec::new();

                for block in content {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match block_type {
                        "text" => {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(t.to_string());
                            }
                        }
                        "thinking" => {
                            if let Some(t) = block.get("thinking").and_then(|t| t.as_str()) {
                                thinking_parts.push(t.to_string());
                            }
                        }
                        "toolCall" => {
                            let name =
                                block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                            let args = block.get("arguments").cloned().unwrap_or(Value::Null);
                            let args_str = if let Some(obj) = args.as_object() {
                                obj.iter()
                                    .map(|(k, v)| format!("{}={}", k, v))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            } else {
                                args.to_string()
                            };
                            tool_calls.push(format!("{}({})", name, args_str));
                        }
                        _ => {}
                    }
                }

                if !thinking_parts.is_empty() {
                    parts.push(format!(
                        "[Assistant thinking]: {}",
                        thinking_parts.join("\n")
                    ));
                }
                if !text_parts.is_empty() {
                    parts.push(format!("[Assistant]: {}", text_parts.join("\n")));
                }
                if !tool_calls.is_empty() {
                    parts.push(format!(
                        "[Assistant tool calls]: {}",
                        tool_calls.join("; ")
                    ));
                }
            }
            "toolResult" => {
                let content = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };
                let text: String = content
                    .iter()
                    .filter_map(|c| {
                        if c.get("type")?.as_str()? == "text" {
                            c.get("text")?.as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&text, TOOL_RESULT_MAX_CHARS)
                    ));
                }
            }
            _ => {}
        }
    }

    parts.join("\n\n")
}

// ============================================================================
// Summarization System Prompt
// ============================================================================

pub const SUMMARIZATION_SYSTEM_PROMPT: &str =
    "You are a context summarization assistant. Your task is to read a conversation \
between a user and an AI coding assistant, then produce a structured summary following \
the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any \
questions in the conversation. ONLY output the structured summary.";

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_file_lists_read_only() {
        let mut ops = create_file_ops();
        ops.read.insert("foo.rs".to_string());
        ops.read.insert("bar.rs".to_string());

        let (read, modified) = compute_file_lists(&ops);
        assert_eq!(read, vec!["bar.rs", "foo.rs"]);
        assert!(modified.is_empty());
    }

    #[test]
    fn test_compute_file_lists_modified_excluded_from_read() {
        let mut ops = create_file_ops();
        ops.read.insert("foo.rs".to_string());
        ops.read.insert("bar.rs".to_string());
        ops.edited.insert("foo.rs".to_string());

        let (read, modified) = compute_file_lists(&ops);
        assert_eq!(read, vec!["bar.rs"]);
        assert_eq!(modified, vec!["foo.rs"]);
    }

    #[test]
    fn test_format_file_operations_empty() {
        let result = format_file_operations(&[], &[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_file_operations_with_files() {
        let read = vec!["a.rs".to_string()];
        let modified = vec!["b.rs".to_string()];
        let result = format_file_operations(&read, &modified);
        assert!(result.contains("<read-files>"));
        assert!(result.contains("a.rs"));
        assert!(result.contains("<modified-files>"));
        assert!(result.contains("b.rs"));
    }

    #[test]
    fn test_truncate_for_summary() {
        let short = "hello";
        assert_eq!(truncate_for_summary(short, 100), "hello");

        let long_str = "a".repeat(3000);
        let result = truncate_for_summary(&long_str, 2000);
        assert!(result.contains("1000 more characters truncated"));
    }

    #[test]
    fn test_serialize_conversation_user() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "Hello world"
        })];
        let result = serialize_conversation(&messages);
        assert_eq!(result, "[User]: Hello world");
    }

    #[test]
    fn test_serialize_conversation_assistant_text() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "Hi there"}]
        })];
        let result = serialize_conversation(&messages);
        assert_eq!(result, "[Assistant]: Hi there");
    }

    // ── compaction-serialization.test.ts ─────────────────────────────────────

    #[test]
    fn test_serialize_conversation_truncates_long_tool_results() {
        // 5000 'x' chars: should be truncated to 2000 + notice
        let long_content = "x".repeat(5000);
        let messages = vec![serde_json::json!({
            "role": "toolResult",
            "toolCallId": "tc1",
            "toolName": "read",
            "content": [{"type": "text", "text": long_content}],
            "isError": false,
            "timestamp": 0
        })];

        let result = serialize_conversation(&messages);

        assert!(result.contains("[Tool result]:"), "should start with [Tool result]:");
        assert!(
            result.contains("more characters truncated"),
            "should contain truncation notice"
        );
        // First 2000 chars should be present
        assert!(
            result.contains(&"x".repeat(2000)),
            "first 2000 chars should be preserved"
        );
        // But not 3000 consecutive x's
        assert!(
            !result.contains(&"x".repeat(3000)),
            "should not have 3000 consecutive x's"
        );
    }

    #[test]
    fn test_serialize_conversation_does_not_truncate_short_tool_results() {
        let short_content = "x".repeat(1500);
        let messages = vec![serde_json::json!({
            "role": "toolResult",
            "toolCallId": "tc1",
            "toolName": "read",
            "content": [{"type": "text", "text": short_content.clone()}],
            "isError": false,
            "timestamp": 0
        })];

        let result = serialize_conversation(&messages);

        assert_eq!(result, format!("[Tool result]: {}", short_content));
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn test_serialize_conversation_does_not_truncate_user_or_assistant_messages() {
        let long_text = "y".repeat(5000);
        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": long_text.clone(),
                "timestamp": 0
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": long_text.clone()}],
                "provider": "anthropic",
                "model": "test",
                "timestamp": 0
            }),
        ];

        let result = serialize_conversation(&messages);

        assert!(!result.contains("truncated"), "should not truncate user/assistant messages");
        assert!(result.contains(&long_text), "full text should appear");
    }
}
