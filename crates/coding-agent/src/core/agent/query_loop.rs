//! Query loop — the inner agent execution cycle.
//!
//! Mirrors CC's `tools/AgentTool/runAgent.ts` query loop section and the
//! `query.ts` module. Each iteration calls the LLM, inspects the response for
//! tool-use blocks, executes allowed tools, and feeds the results back until
//! the model produces a stop-sequence with no tool calls or `max_turns` is hit.
//!
//! This module defines the parameter types and the loop entry point. The actual
//! LLM transport is supplied by the caller via `AgentLoopConfig`/`LlmProvider`
//! in the agent-core crate; here we expose a portable stream interface that
//! callers can drive without depending on the full agent session infrastructure.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use agent_core::types::AgentMessage;

use super::runner::AgentError;

/// Parameters for a single query-loop execution.
///
/// Mirrors CC's `QueryLoopParams` object threaded into `query()`.
pub struct QueryLoopParams {
    /// System prompt sent to the LLM on every turn.
    pub system_prompt: String,
    /// User context key-value pairs prepended to messages.
    pub user_context: HashMap<String, String>,
    /// System context key-value pairs appended to the system prompt.
    pub system_context: HashMap<String, String>,
    /// Current message history (includes initial prompts + all prior turns).
    pub messages: Vec<AgentMessage>,
    /// Maximum number of LLM turns before the loop terminates.
    pub max_turns: u32,
    /// Permission check for tool calls. Returns `Ok(())` if allowed.
    pub can_use_tool: Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync>,
    /// Optional allowlist of tool names. `None` means no extra restriction.
    pub allowed_tools: Option<Vec<String>>,
    /// Cancellation token — signals the loop to abort cleanly.
    pub abort_token: Option<CancellationToken>,
}

/// Run the inner query loop and yield messages as they are produced.
///
/// The stream emits:
/// - `AgentMessage::Assistant(...)` — each LLM response turn
/// - `AgentMessage::ToolResult(...)` — each tool execution result
///
/// The stream closes when the LLM produces a stop with no tool calls,
/// `max_turns` is exhausted, or the `abort_token` fires.
///
/// Mirrors CC's `query()` async generator called from `runAgent`.
///
/// # Implementation note
///
/// This is a stub that produces an empty stream. The full implementation
/// integrates with `agent_core::agent_loop::run_agent_loop` and the
/// `LlmProvider` trait. Wire-up is deferred to the `AgentTool` integration
/// layer, which holds the concrete provider handle and event emitter.
pub fn run_query_loop(
    _params: QueryLoopParams,
) -> BoxStream<'static, Result<AgentMessage, AgentError>> {
    Box::pin(futures::stream::empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn allow_all() -> Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync> {
        Arc::new(|_tool, _allowed| Ok(()))
    }

    #[test]
    fn query_loop_params_construction() {
        let _params = QueryLoopParams {
            system_prompt: "system".to_string(),
            user_context: HashMap::new(),
            system_context: HashMap::new(),
            messages: vec![],
            max_turns: 10,
            can_use_tool: allow_all(),
            allowed_tools: None,
            abort_token: None,
        };
    }

    #[tokio::test]
    async fn run_query_loop_stub_returns_empty_stream() {
        let params = QueryLoopParams {
            system_prompt: "test".to_string(),
            user_context: HashMap::new(),
            system_context: HashMap::new(),
            messages: vec![],
            max_turns: 5,
            can_use_tool: allow_all(),
            allowed_tools: None,
            abort_token: None,
        };

        let messages: Vec<_> = run_query_loop(params).collect().await;
        // Stub implementation produces no messages; real impl will yield turns.
        assert!(messages.is_empty());
    }
}
