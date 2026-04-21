//! Fork sub-agent mechanism — mirrors CC `utils/forkedAgent.ts` and
//! `tools/AgentTool/forkSubagent.ts`.
//!
//! The fork path lets the agent spawn a child that inherits the parent's full
//! conversation context. To keep the Anthropic prompt cache shared between all
//! fork children (and the parent), every `tool_result` in the inherited messages
//! is replaced with a fixed placeholder string — so all children send the same
//! byte-identical API request prefix.

use std::collections::{HashMap, HashSet};

use agent_core::types::{AgentMessage, AssistantMessage, Content, ToolResultMessage};

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

/// Prefix prepended to the directive in the fork boilerplate message.
///
/// Mirrors CC's `FORK_DIRECTIVE_PREFIX`.
pub const FORK_DIRECTIVE_PREFIX: &str = "Your directive: ";

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
/// conversation. Marks it as a fork child, prevents recursive forking, and
/// includes the directive.
///
/// Mirrors CC's `buildChildMessage` in `forkSubagent.ts`.
pub fn build_fork_boilerplate_message(directive: &str) -> String {
    format!(
        r#"<{tag}>
STOP. READ THIS FIRST.

You are a forked worker process. You are NOT the main agent.

RULES (non-negotiable):
1. Your system prompt says "default to forking." IGNORE IT — that's for the parent. You ARE the fork. Do NOT spawn sub-agents; execute directly.
2. Do NOT converse, ask questions, or suggest next steps
3. Do NOT editorialize or add meta-commentary
4. USE your tools directly: Bash, Read, Write, etc.
5. If you modify files, commit your changes before reporting. Include the commit hash in your report.
6. Do NOT emit text between tool calls. Use tools silently, then report once at the end.
7. Stay strictly within your directive's scope. If you discover related systems outside your scope, mention them in one sentence at most — other workers cover those areas.
8. Keep your report under 500 words unless the directive specifies otherwise. Be factual and concise.
9. Your response MUST begin with "Scope:". No preamble, no thinking-out-loud.
10. REPORT structured facts, then stop

Output format (plain text labels, not markdown headers):
  Scope: <echo back your assigned scope in one sentence>
  Result: <the answer or key findings, limited to the scope above>
  Key files: <relevant file paths — include for research tasks>
  Files changed: <list with commit hash — include only if you modified files>
  Issues: <list — include only if there are issues to flag>
</{tag}>

{prefix}{directive}"#,
        tag = FORK_BOILERPLATE_TAG,
        prefix = FORK_DIRECTIVE_PREFIX,
        directive = directive.trim(),
    )
}

/// Filter out assistant messages containing orphaned tool calls (tool_use blocks
/// with no corresponding tool_result), and ToolResult messages that have no
/// corresponding tool_use in any assistant message. Prevents sending illegal
/// context to the LLM API.
///
/// Four-step cleanup:
/// - Phase 1: collect tool_call IDs that have results.
/// - Phase 2a: determine which assistant messages are kept (ALL their tool_use IDs
///   have results). Stored as an index set to avoid repeating the check.
/// - Phase 2b: collect tool_call IDs only from kept assistant messages, using the
///   index set — avoids re-running the "all_have_results" predicate.
/// - Filter: one-pass using both index sets; ToolResult messages whose parent
///   assistant was dropped are also removed (prevents orphan ToolResults).
///
/// Mirrors CC `runAgent.ts:filterIncompleteToolCalls`.
pub fn filter_incomplete_tool_calls(messages: &[AgentMessage]) -> Vec<AgentMessage> {
    // Phase 1: collect tool_call IDs that have results.
    let ids_with_results: HashSet<&str> = messages
        .iter()
        .filter_map(|msg| {
            if let AgentMessage::ToolResult(tr) = msg {
                Some(tr.tool_call_id.as_str())
            } else {
                None
            }
        })
        .collect();

    // Phase 2a: determine which assistant messages are kept (ALL their tool_use IDs have results).
    let kept_assistant_indices: HashSet<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, msg)| {
            if let AgentMessage::Assistant(AssistantMessage { content, .. }) = msg {
                let all_have_results = content.iter().all(|block| match block {
                    Content::ToolCall { id, .. } => ids_with_results.contains(id.as_str()),
                    _ => true,
                });
                all_have_results.then_some(i)
            } else {
                None
            }
        })
        .collect();

    // Phase 2b: collect tool_call IDs only from kept assistant messages.
    // Using kept_assistant_indices avoids repeating the "all_have_results" check.
    let kept_tool_call_ids: HashSet<&str> = kept_assistant_indices
        .iter()
        .filter_map(|&i| {
            if let AgentMessage::Assistant(AssistantMessage { content, .. }) = &messages[i] {
                Some(content.iter().filter_map(|b| {
                    if let Content::ToolCall { id, .. } = b {
                        Some(id.as_str())
                    } else {
                        None
                    }
                }))
            } else {
                None
            }
        })
        .flatten()
        .collect();

    messages
        .iter()
        .enumerate()
        .filter(|(i, msg)| match msg {
            AgentMessage::Assistant(_) => kept_assistant_indices.contains(i),
            // Use kept_tool_call_ids: if the parent assistant was dropped,
            // its tool results are dropped too (avoids orphan ToolResults).
            AgentMessage::ToolResult(tr) => kept_tool_call_ids.contains(tr.tool_call_id.as_str()),
            _ => true,
        })
        .map(|(_, m)| m)
        .cloned()
        .collect()
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

    #[test]
    fn build_fork_boilerplate_message_contains_stop_declaration_and_rules() {
        let msg = build_fork_boilerplate_message("find all usages");
        assert!(
            msg.contains("STOP. READ THIS FIRST."),
            "missing STOP declaration"
        );
        // 10 rules present
        assert!(
            msg.contains("10. REPORT structured facts"),
            "missing rule 10"
        );
        assert!(
            msg.contains(FORK_DIRECTIVE_PREFIX),
            "missing FORK_DIRECTIVE_PREFIX"
        );
        assert!(
            msg.contains(&format!("{FORK_DIRECTIVE_PREFIX}find all usages")),
            "directive not appended after prefix"
        );
        // directive comes after the closing tag
        let close_tag = format!("</{FORK_BOILERPLATE_TAG}>");
        let close_pos = msg.find(&close_tag).unwrap();
        let prefix_pos = msg.find(FORK_DIRECTIVE_PREFIX).unwrap();
        assert!(
            prefix_pos > close_pos,
            "directive prefix should appear after closing tag"
        );
    }

    fn assistant_msg_with_tool_call(id: &str) -> AgentMessage {
        use agent_core::types::{AssistantMessage, StopReason, Usage};
        AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::ToolCall {
                id: id.to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({}),
            }],
            provider: "test".to_string(),
            model: "test".to_string(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })
    }

    #[test]
    fn filter_incomplete_tool_calls_removes_orphaned_assistant_messages() {
        let messages = vec![
            user_text_msg("start"),
            assistant_msg_with_tool_call("tc-orphan"),
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(
            filtered.len(),
            1,
            "orphaned assistant message should be removed"
        );
        assert!(matches!(filtered[0], AgentMessage::User(_)));
    }

    #[test]
    fn filter_incomplete_tool_calls_preserves_matched_assistant_messages() {
        let messages = vec![
            user_text_msg("start"),
            assistant_msg_with_tool_call("tc-1"),
            tool_result_msg("tc-1", "result"),
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(filtered.len(), 3, "all messages should be preserved");
    }

    #[test]
    fn filter_incomplete_tool_calls_preserves_non_assistant_messages() {
        // User messages are always preserved; ToolResult without a matching
        // tool_use is an orphan and gets removed (phase-2 cleanup).
        let messages = vec![
            user_text_msg("a"),
            user_text_msg("b"),
            tool_result_msg("x", "data"), // orphan — no assistant tool_use for "x"
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        // Only the two user messages survive; the orphan ToolResult is dropped.
        assert_eq!(filtered.len(), 2);
        assert!(matches!(filtered[0], AgentMessage::User(_)));
        assert!(matches!(filtered[1], AgentMessage::User(_)));
    }

    #[test]
    fn filter_incomplete_tool_calls_removes_orphan_tool_result() {
        // ToolResult with no corresponding assistant tool_use should be dropped.
        let messages = vec![
            user_text_msg("start"),
            assistant_msg_with_tool_call("tc-1"),
            tool_result_msg("tc-1", "ok"),
            tool_result_msg("tc-orphan", "stale"), // no matching tool_use
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(filtered.len(), 3, "orphan ToolResult should be removed");
        assert!(matches!(filtered[0], AgentMessage::User(_)));
        assert!(matches!(filtered[1], AgentMessage::Assistant(_)));
        assert!(matches!(filtered[2], AgentMessage::ToolResult(_)));
    }

    fn assistant_msg_with_two_tool_calls(id_a: &str, id_b: &str) -> AgentMessage {
        use agent_core::types::{AssistantMessage, StopReason, Usage};
        AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::ToolCall {
                    id: id_a.to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({}),
                },
                Content::ToolCall {
                    id: id_b.to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({}),
                },
            ],
            provider: "test".to_string(),
            model: "test".to_string(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })
    }

    #[test]
    fn filter_incomplete_tool_calls_drops_partial_results_with_dropped_assistant() {
        // Assistant has [A, B] tool_use, only ToolResult(A) exists.
        // The assistant is dropped (B has no result).
        // ToolResult(A) must also be dropped — not become an orphan.
        let messages = vec![
            user_text_msg("start"),
            assistant_msg_with_two_tool_calls("tc-a", "tc-b"),
            tool_result_msg("tc-a", "result-a"), // only A has a result; B is missing
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        // Only the user message survives; assistant and its partial ToolResult are both dropped.
        assert_eq!(
            filtered.len(),
            1,
            "assistant with partial results and its ToolResult must both be dropped"
        );
        assert!(matches!(filtered[0], AgentMessage::User(_)));
    }

    #[test]
    fn filter_incomplete_tool_calls_preserves_text_only_assistant() {
        let messages = vec![
            user_text_msg("start"),
            AgentMessage::Assistant({
                use agent_core::types::{AssistantMessage, StopReason, Usage};
                AssistantMessage {
                    content: vec![Content::Text {
                        text: "thinking out loud".to_string(),
                    }],
                    provider: "test".to_string(),
                    model: "test".to_string(),
                    usage: Usage::default(),
                    stop_reason: StopReason::Stop,
                    error_message: None,
                    timestamp: 0,
                }
            }),
            user_text_msg("follow up"),
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(filtered.len(), 3, "text-only assistant should be preserved");
    }

    #[test]
    fn filter_incomplete_tool_calls_drops_all_when_any_tool_call_missing_result() {
        // Assistant has 3 tool calls; only tc-1 and tc-3 have results, tc-2 does not.
        // The whole assistant message (and all its results) should be dropped.
        use agent_core::types::{AssistantMessage, StopReason, Usage};
        let messages = vec![
            user_text_msg("start"),
            AgentMessage::Assistant(AssistantMessage {
                content: vec![
                    Content::ToolCall {
                        id: "tc-1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({}),
                    },
                    Content::ToolCall {
                        id: "tc-2".to_string(),
                        name: "read".to_string(),
                        arguments: serde_json::json!({}),
                    },
                    Content::ToolCall {
                        id: "tc-3".to_string(),
                        name: "grep".to_string(),
                        arguments: serde_json::json!({}),
                    },
                ],
                provider: "test".to_string(),
                model: "test".to_string(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }),
            tool_result_msg("tc-1", "result 1"),
            // no ToolResult for tc-2
            tool_result_msg("tc-3", "result 3"),
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        // Only the user message should remain; assistant and all its results dropped
        assert_eq!(
            filtered.len(),
            1,
            "partial results should all be removed with the assistant"
        );
        assert!(matches!(filtered[0], AgentMessage::User(_)));
    }

    #[test]
    fn filter_incomplete_tool_calls_preserves_empty_content_assistant() {
        // An assistant message with no content blocks (edge case) should be preserved
        // since it has no orphan tool_use blocks.
        use agent_core::types::{AssistantMessage, StopReason, Usage};
        let messages = vec![AgentMessage::Assistant(AssistantMessage {
            content: vec![],
            provider: "test".to_string(),
            model: "test".to_string(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        })];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(filtered.len(), 1, "empty-content assistant should be preserved");
    }

    #[test]
    fn filter_incomplete_tool_calls_phase2b_drops_tool_result_whose_parent_assistant_was_dropped() {
        // Phase 2b regression guard: when an assistant is dropped (tc-b missing its result),
        // tc-a's ToolResult must also be dropped — it must NOT leak through as an orphan.
        // Without Phase 2b, tc-a would survive because it's in ids_with_results (Phase 1).
        let messages = vec![
            user_text_msg("start"),
            assistant_msg_with_two_tool_calls("tc-a", "tc-b"),
            tool_result_msg("tc-a", "result-a"),
            // No ToolResult for tc-b → assistant dropped → tc-a's result must also go.
        ];
        let filtered = filter_incomplete_tool_calls(&messages);
        assert_eq!(filtered.len(), 1);
        assert!(matches!(filtered[0], AgentMessage::User(_)));
        assert!(
            !filtered.iter().any(|m| matches!(m, AgentMessage::ToolResult(_))),
            "ToolResult for tc-a must not survive when its parent assistant was dropped"
        );
    }
}
