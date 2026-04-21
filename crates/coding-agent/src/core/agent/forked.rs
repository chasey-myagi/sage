//! Fork sub-agent mechanism — mirrors CC `utils/forkedAgent.ts` and
//! `tools/AgentTool/forkSubagent.ts`.
//!
//! The fork path lets the agent spawn a child that inherits the parent's full
//! conversation context. To keep the Anthropic prompt cache shared between all
//! fork children (and the parent), every `tool_result` in the inherited messages
//! is replaced with a fixed placeholder string — so all children send the same
//! byte-identical API request prefix.

use std::collections::HashMap;

use agent_core::types::{AgentMessage, Content, ToolResultMessage};

/// Placeholder injected into inherited `ToolResult` messages so all fork
/// children produce an identical API request prefix (enabling prompt cache hits).
///
/// Mirrors CC's `FORK_PLACEHOLDER_RESULT`.
pub const FORK_PLACEHOLDER_RESULT: &str = "Fork started — processing in background";

/// XML tag embedded in the fork boilerplate injected at the start of each
/// fork-child conversation. Its presence marks a conversation as inside a fork
/// child, preventing recursive fork attempts.
///
/// Mirrors CC's `FORK_BOILERPLATE_TAG`.
pub const FORK_BOILERPLATE_TAG: &str = "fork_boilerplate_tag";

/// Cache-critical parameters that must be byte-identical between the parent
/// API request and all fork children so they share the parent's prompt cache.
///
/// Mirrors CC's `CacheSafeParams` type.
pub struct CacheSafeParams {
    /// Rendered system prompt — must be byte-identical to the parent's.
    pub system_prompt: String,
    /// User context key-value pairs prepended to messages.
    pub user_context: HashMap<String, String>,
    /// System context key-value pairs appended to the system prompt.
    pub system_context: HashMap<String, String>,
    /// Parent messages to inherit (tool results will be replaced by placeholder).
    pub fork_context_messages: Vec<AgentMessage>,
}

/// Replace all `ToolResult` content in the inherited message list with the
/// fork placeholder, so that every fork child sends the same byte-identical
/// API request prefix and can share the parent's prompt cache.
///
/// Mirrors CC's `buildForkedMessages` logic.
pub fn build_forked_messages(parent_messages: &[AgentMessage]) -> Vec<AgentMessage> {
    parent_messages
        .iter()
        .map(|msg| match msg {
            AgentMessage::ToolResult(tr) => AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: tr.tool_call_id.clone(),
                tool_name: tr.tool_name.clone(),
                content: vec![Content::Text {
                    text: FORK_PLACEHOLDER_RESULT.to_string(),
                }],
                details: None,
                is_error: false,
                timestamp: tr.timestamp,
            }),
            other => other.clone(),
        })
        .collect()
}

/// Returns `true` if the conversation is already inside a fork child.
///
/// Fork children cannot recursively fork (they keep the Agent tool in their
/// pool for cache-identical tool definitions, so we reject at call time by
/// checking for the boilerplate tag).
///
/// Mirrors CC's `isInForkChild`.
pub fn is_in_fork_child(messages: &[AgentMessage]) -> bool {
    let tag = format!("<{FORK_BOILERPLATE_TAG}>");
    messages.iter().any(|m| {
        if let AgentMessage::User(user) = m {
            user.content.iter().any(|block| {
                if let Content::Text { text } = block {
                    text.contains(&tag)
                } else {
                    false
                }
            })
        } else {
            false
        }
    })
}

/// Build the boilerplate user message injected at the start of a fork child's
/// conversation to mark it as a fork child and prevent recursive forking.
pub fn build_fork_boilerplate_message(directive: &str) -> String {
    format!(
        "<{tag}>\n{directive}\n</{tag}>",
        tag = FORK_BOILERPLATE_TAG,
        directive = directive.trim(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::types::{Content, ToolResultMessage, UserMessage};

    fn tool_result_msg(id: &str, text: &str) -> AgentMessage {
        AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: id.to_string(),
            tool_name: "read".to_string(),
            content: vec![Content::Text {
                text: text.to_string(),
            }],
            details: None,
            is_error: false,
            timestamp: 0,
        })
    }

    fn user_text_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: text.to_string(),
            }],
            timestamp: 0,
        })
    }

    #[test]
    fn build_forked_messages_replaces_tool_results() {
        let messages = vec![
            user_text_msg("hello"),
            tool_result_msg("tool-1", "real result data"),
        ];

        let forked = build_forked_messages(&messages);
        assert_eq!(forked.len(), 2);

        // User message is unchanged
        if let AgentMessage::User(u) = &forked[0] {
            if let Content::Text { text } = &u.content[0] {
                assert_eq!(text, "hello");
            } else {
                panic!("expected Text content");
            }
        } else {
            panic!("expected User message");
        }

        // ToolResult is replaced with placeholder
        if let AgentMessage::ToolResult(tr) = &forked[1] {
            assert_eq!(tr.tool_call_id, "tool-1");
            if let Content::Text { text } = &tr.content[0] {
                assert_eq!(text, FORK_PLACEHOLDER_RESULT);
            } else {
                panic!("expected Text content");
            }
        } else {
            panic!("expected ToolResult message");
        }
    }

    #[test]
    fn build_forked_messages_preserves_non_tool_result_messages() {
        let messages = vec![user_text_msg("query"), user_text_msg("follow-up")];
        let forked = build_forked_messages(&messages);
        assert_eq!(forked.len(), 2);
    }

    #[test]
    fn is_in_fork_child_detects_boilerplate_tag() {
        let messages = vec![user_text_msg(&format!("<{FORK_BOILERPLATE_TAG}>"))];
        assert!(is_in_fork_child(&messages));
    }

    #[test]
    fn is_in_fork_child_false_without_tag() {
        let messages = vec![user_text_msg("no tag here")];
        assert!(!is_in_fork_child(&messages));
    }

    #[test]
    fn is_in_fork_child_false_for_empty_messages() {
        assert!(!is_in_fork_child(&[]));
    }

    #[test]
    fn build_fork_boilerplate_message_contains_tag() {
        let msg = build_fork_boilerplate_message("analyze the code");
        assert!(msg.contains(&format!("<{FORK_BOILERPLATE_TAG}>")));
        assert!(msg.contains(&format!("</{FORK_BOILERPLATE_TAG}>")));
        assert!(msg.contains("analyze the code"));
    }
}
