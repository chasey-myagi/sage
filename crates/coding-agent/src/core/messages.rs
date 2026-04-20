//! Custom message types and transformers for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/messages.ts`.
//!
//! Extends the base agent message types with coding-agent-specific roles,
//! and provides converters to LLM-compatible message format.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Compaction / branch summary prefixes
// ============================================================================

pub const COMPACTION_SUMMARY_PREFIX: &str =
    "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";

pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

// ============================================================================
// Custom message roles
// ============================================================================

/// A bash command execution recorded in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashExecutionMessage {
    pub role: String, // "bashExecution"
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub truncated: bool,
    pub full_output_path: Option<String>,
    pub timestamp: i64,
    /// If true, excluded from LLM context (`!!` prefix).
    pub exclude_from_context: Option<bool>,
}

/// Extension-injected message via `sendMessage()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMessage {
    pub role: String, // "custom"
    pub custom_type: String,
    /// Text content or content blocks.
    pub content: Value,
    pub display: bool,
    pub details: Option<Value>,
    pub timestamp: i64,
}

/// A branch summary injected after navigating the session tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummaryMessage {
    pub role: String, // "branchSummary"
    pub summary: String,
    pub from_id: String,
    pub timestamp: i64,
}

/// A compaction summary injected after context compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummaryMessage {
    pub role: String, // "compactionSummary"
    pub summary: String,
    pub tokens_before: u64,
    pub timestamp: i64,
}

// ============================================================================
// Agent message union
// ============================================================================

/// Any message that can appear in an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
    BashExecution(BashExecutionMessage),
    Custom(CustomMessage),
    BranchSummary(BranchSummaryMessage),
    CompactionSummary(CompactionSummaryMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub role: String,
    pub content: Value,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Value,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub role: String,
    pub content: Value,
    pub timestamp: Option<i64>,
}

// ============================================================================
// Text conversion helpers
// ============================================================================

/// Convert a `BashExecutionMessage` to user message text for LLM context.
///
/// Mirrors `bashExecutionToText()` from TypeScript.
pub fn bash_execution_to_text(msg: &BashExecutionMessage) -> String {
    let mut text = format!("Ran `{}`\n", msg.command);

    if !msg.output.is_empty() {
        text.push_str(&format!("```\n{}\n```", msg.output));
    } else {
        text.push_str("(no output)");
    }

    if msg.cancelled {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = msg.exit_code {
        if code != 0 {
            text.push_str(&format!("\n\nCommand exited with code {code}"));
        }
    }

    if msg.truncated {
        if let Some(ref path) = msg.full_output_path {
            text.push_str(&format!("\n\n[Output truncated. Full output: {path}]"));
        }
    }

    text
}

// ============================================================================
// Constructors
// ============================================================================

pub fn create_branch_summary_message(
    summary: impl Into<String>,
    from_id: impl Into<String>,
    timestamp_str: &str,
) -> BranchSummaryMessage {
    let timestamp = chrono_parse_ms(timestamp_str);
    BranchSummaryMessage {
        role: "branchSummary".to_string(),
        summary: summary.into(),
        from_id: from_id.into(),
        timestamp,
    }
}

pub fn create_compaction_summary_message(
    summary: impl Into<String>,
    tokens_before: u64,
    timestamp_str: &str,
) -> CompactionSummaryMessage {
    let timestamp = chrono_parse_ms(timestamp_str);
    CompactionSummaryMessage {
        role: "compactionSummary".to_string(),
        summary: summary.into(),
        tokens_before,
        timestamp,
    }
}

pub fn create_custom_message(
    custom_type: impl Into<String>,
    content: Value,
    display: bool,
    details: Option<Value>,
    timestamp_str: &str,
) -> CustomMessage {
    let timestamp = chrono_parse_ms(timestamp_str);
    CustomMessage {
        role: "custom".to_string(),
        custom_type: custom_type.into(),
        content,
        display,
        details,
        timestamp,
    }
}

/// Parse an ISO 8601 timestamp string to milliseconds since epoch.
/// Falls back to 0 on parse failure.
fn chrono_parse_ms(s: &str) -> i64 {
    // Basic: try RFC3339
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis();
    }
    // Fall back to current time
    chrono::Utc::now().timestamp_millis()
}

// ============================================================================
// LLM conversion
// ============================================================================

/// Convert `AgentMessage` list (including custom types) to LLM-compatible format.
///
/// Mirrors `convertToLlm()` from TypeScript.
pub fn convert_to_llm(messages: &[AgentMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::User(u) => Some(serde_json::json!({
                "role": "user",
                "content": u.content,
                "timestamp": u.timestamp,
            })),
            AgentMessage::Assistant(a) => Some(serde_json::json!({
                "role": "assistant",
                "content": a.content,
                "timestamp": a.timestamp,
            })),
            AgentMessage::ToolResult(tr) => Some(serde_json::json!({
                "role": "toolResult",
                "content": tr.content,
                "timestamp": tr.timestamp,
            })),
            AgentMessage::BashExecution(bash) => {
                if bash.exclude_from_context == Some(true) {
                    return None;
                }
                Some(serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": bash_execution_to_text(bash) }],
                    "timestamp": bash.timestamp,
                }))
            }
            AgentMessage::Custom(c) => {
                let content = if c.content.is_string() {
                    serde_json::json!([{ "type": "text", "text": c.content }])
                } else {
                    c.content.clone()
                };
                Some(serde_json::json!({
                    "role": "user",
                    "content": content,
                    "timestamp": c.timestamp,
                }))
            }
            AgentMessage::BranchSummary(b) => Some(serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": format!("{}{}{}", BRANCH_SUMMARY_PREFIX, b.summary, BRANCH_SUMMARY_SUFFIX),
                }],
                "timestamp": b.timestamp,
            })),
            AgentMessage::CompactionSummary(c) => Some(serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": format!("{}{}{}", COMPACTION_SUMMARY_PREFIX, c.summary, COMPACTION_SUMMARY_SUFFIX),
                }],
                "timestamp": c.timestamp,
            })),
        })
        .collect()
}
