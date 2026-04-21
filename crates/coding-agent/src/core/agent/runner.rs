//! Sub-agent execution engine — mirrors CC `tools/AgentTool/runAgent.ts`.
//!
//! `run_agent` drives a single sub-agent turn: it initialises the agent-specific
//! context, resolves tools and model, calls the optional `on_cache_safe_params`
//! callback (for prompt-cache sharing), then runs the query loop until the agent
//! completes or hits `max_turns`.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::BoxStream;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use agent_core::types::AgentMessage;

use super::definition::{AgentDef, AgentModel};
use super::forked::CacheSafeParams;
use super::query_loop::{QueryLoopParams, run_query_loop};

/// Errors that can occur while running a sub-agent.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("max turns reached")]
    MaxTurnsReached,
    #[error("aborted")]
    Aborted,
    #[error("name generation failed for: {0}")]
    NameGeneration(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Parameters for running a sub-agent.
///
/// Mirrors CC's `runAgent` function arguments.
pub struct RunAgentParams {
    /// Agent definition (type, tools, model, system prompt).
    pub agent_def: AgentDef,
    /// Rendered system prompt. When set, overrides `agent_def.system_prompt_fn`.
    pub system_prompt: Option<String>,
    /// User context key-value pairs prepended to messages.
    pub user_context: HashMap<String, String>,
    /// System context key-value pairs appended to the system prompt.
    pub system_context: HashMap<String, String>,
    /// Permission check: returns `Ok(())` if the tool is allowed, `Err` otherwise.
    pub can_use_tool: Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync>,
    /// Initial messages for this agent (may include inherited fork context).
    pub messages: Vec<AgentMessage>,
    /// Maximum number of turns. `None` means use the agent def's `max_turns` or 200.
    pub max_turns: Option<u32>,
    /// Restrict which tools the agent may call (subset of its defined tools).
    pub allowed_tools: Option<Vec<String>>,
    /// Override the agent's declared model.
    pub model_override: Option<AgentModel>,
    /// Cancellation token — abort the agent when signalled.
    pub abort_token: Option<CancellationToken>,
    /// Callback invoked once with cache-safe params (for prompt-cache sharing).
    pub on_cache_safe_params: Option<Box<dyn FnOnce(CacheSafeParams) + Send>>,
}

/// Accumulated result from a completed sub-agent run.
pub struct AgentRunResult {
    /// All messages produced by the agent (assistant + tool results).
    pub messages: Vec<AgentMessage>,
    /// Total input tokens consumed across all turns.
    pub input_tokens: u64,
    /// Total output tokens produced across all turns.
    pub output_tokens: u64,
}

/// Run a sub-agent and return a stream of messages.
///
/// Each item in the stream is either an assistant message or a tool-result
/// message. The stream ends when the agent completes, hits `max_turns`, or
/// is aborted via `params.abort_token`.
///
/// Mirrors CC's `runAgent` async generator.
pub fn run_agent(params: RunAgentParams) -> BoxStream<'static, Result<AgentMessage, AgentError>> {
    let max_turns = params
        .max_turns
        .or(params.agent_def.max_turns)
        .unwrap_or(200);

    let system_prompt = params
        .system_prompt
        .unwrap_or_else(|| params.agent_def.system_prompt_fn.clone());

    // Fire the cache-safe params callback before entering the loop so that
    // fork children can share the same prompt-cache prefix.
    if let Some(callback) = params.on_cache_safe_params {
        callback(CacheSafeParams {
            system_prompt: system_prompt.clone(),
            user_context: params.user_context.clone(),
            system_context: params.system_context.clone(),
            fork_context_messages: params.messages.clone(),
        });
    }

    let loop_params = QueryLoopParams {
        system_prompt,
        user_context: params.user_context,
        system_context: params.system_context,
        messages: params.messages,
        max_turns,
        can_use_tool: params.can_use_tool,
        allowed_tools: params.allowed_tools,
        abort_token: params.abort_token,
    };

    run_query_loop(loop_params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::types::AgentMessage;

    fn make_allow_all() -> Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync>
    {
        Arc::new(|_tool, _allowed| Ok(()))
    }

    #[test]
    fn agent_error_display() {
        assert!(AgentError::ToolNotFound("read".to_string())
            .to_string()
            .contains("read"));
        assert!(AgentError::MaxTurnsReached.to_string().contains("max turns"));
        assert!(AgentError::Aborted.to_string().contains("aborted"));
    }

    #[test]
    fn run_agent_params_construction() {
        use super::super::builtin::register_builtin_agents;

        let agents = register_builtin_agents();
        let def = agents.get("general-purpose").cloned().unwrap();

        let params = RunAgentParams {
            agent_def: def,
            system_prompt: Some("You are a helpful assistant.".to_string()),
            user_context: HashMap::new(),
            system_context: HashMap::new(),
            can_use_tool: make_allow_all(),
            messages: vec![AgentMessage::User(agent_core::types::UserMessage {
                content: vec![agent_core::types::Content::Text {
                    text: "hello".to_string(),
                }],
                timestamp: 0,
            })],
            max_turns: Some(1),
            allowed_tools: None,
            model_override: None,
            abort_token: None,
            on_cache_safe_params: None,
        };

        // Verify params are wired correctly
        assert_eq!(params.max_turns, Some(1));
    }
}
