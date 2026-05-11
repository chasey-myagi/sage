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

use agent_core::agent_loop::LlmProvider;
use agent_core::types::{AgentMessage, AgentTool};
use ai::types::Model;

use super::definition::{AgentDef, AgentModel};
use super::forked::{CacheSafeParams, filter_incomplete_tool_calls};
use super::query_loop::{CanUseTool, QueryLoopParams, run_query_loop};

const OPUS_MODEL_ID: &str = "claude-opus-4-7";
const SONNET_MODEL_ID: &str = "claude-sonnet-4-6";
const HAIKU_MODEL_ID: &str = "claude-haiku-4-5-20251001";

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
    /// Rendered system prompt. When `Some`, overrides `agent_def.system_prompt_fn`.
    /// When `None`, the system prompt is resolved from `agent_def.system_prompt_fn`.
    pub system_prompt: Option<String>,
    /// User context key-value pairs prepended to messages.
    // TODO: wire when fork context sharing is implemented
    pub user_context: HashMap<String, String>,
    /// System context key-value pairs appended to the system prompt.
    // TODO: wire when fork context sharing is implemented
    pub system_context: HashMap<String, String>,
    /// Permission check: returns `Ok(())` if the tool is allowed, `Err` otherwise.
    pub can_use_tool: CanUseTool,
    /// Initial messages for this agent (may include inherited fork context).
    pub messages: Vec<AgentMessage>,
    /// Maximum number of turns. `None` means use the agent def's `max_turns` or 200.
    pub max_turns: Option<u32>,
    /// Restrict which tools the agent may call (subset of its defined tools).
    pub allowed_tools: Option<Vec<String>>,
    /// Override the agent's declared model. Takes precedence over `agent_def.model`.
    pub model_override: Option<AgentModel>,
    /// Cancellation token — abort the agent when signalled.
    pub abort_token: Option<CancellationToken>,
    /// Callback invoked once with cache-safe params (for prompt-cache sharing).
    pub on_cache_safe_params: Option<Box<dyn FnOnce(CacheSafeParams) + Send>>,
    /// LLM provider to use for this agent's requests.
    pub provider: Arc<dyn LlmProvider>,
    /// Tools available to this agent.
    pub tools: Vec<Arc<dyn AgentTool>>,
    /// Resolved model for LLM calls.
    pub model: Model,
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

/// Resolve a system prompt from a `system_prompt_fn` identifier.
///
/// Maps the function name stored in `AgentDef.system_prompt_fn` to an actual
/// prompt string. Callers that already have a rendered prompt should pass it
/// directly via `RunAgentParams.system_prompt`; this function is only invoked
/// when `system_prompt` is `None`.
///
/// Mirrors CC's `getAgentSystemPrompt` in `runAgent.ts`.
pub fn resolve_system_prompt(fn_name: &str, cwd: Option<&std::path::Path>) -> String {
    use crate::core::system_prompt::{BuildSystemPromptOptions, build_system_prompt};

    let cwd_str = cwd.and_then(|p| p.to_str()).map(|s| s.to_string());

    let read_only_tools = Some(vec![
        "read".to_string(),
        "grep".to_string(),
        "find".to_string(),
    ]);

    match fn_name {
        "get_general_purpose_prompt" => build_system_prompt(BuildSystemPromptOptions {
            cwd: cwd_str,
            ..Default::default()
        }),
        "get_explore_prompt" => build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some(
                "You are a fast exploration agent. Quickly find files, search code, and answer \
                 questions about codebases. Focus on reading and searching — do not modify files."
                    .to_string(),
            ),
            selected_tools: read_only_tools.clone(),
            cwd: cwd_str,
            ..Default::default()
        }),
        "get_plan_prompt" => build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some(
                "You are a software architect agent. Design clear, actionable implementation \
                 plans by reading the code carefully. Do not modify files."
                    .to_string(),
            ),
            selected_tools: read_only_tools.clone(),
            cwd: cwd_str,
            ..Default::default()
        }),
        "get_verification_prompt" => build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some(
                "You are a verification agent. Confirm implementations are correct by running \
                 tests and inspecting the code. Report findings concisely."
                    .to_string(),
            ),
            selected_tools: Some(vec![
                "read".to_string(),
                "grep".to_string(),
                "bash".to_string(),
            ]),
            cwd: cwd_str,
            ..Default::default()
        }),
        "get_code_guide_prompt" => build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some(
                "You are a code guide agent. Help users understand unfamiliar codebases by \
                 providing guided tours, explaining architecture, and highlighting key files."
                    .to_string(),
            ),
            selected_tools: read_only_tools,
            cwd: cwd_str,
            ..Default::default()
        }),
        "get_fork_prompt" => build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some(
                "You are a forked worker. Execute your directive directly without spawning \
                 sub-agents. Report results concisely."
                    .to_string(),
            ),
            cwd: cwd_str,
            ..Default::default()
        }),
        other => {
            // Unknown function name: fall back to a generic prompt with the
            // function name embedded so the agent has some context.
            build_system_prompt(BuildSystemPromptOptions {
                custom_prompt: Some(format!(
                    "You are a helpful coding assistant (agent: {other})."
                )),
                cwd: cwd_str,
                ..Default::default()
            })
        }
    }
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

    // Resolve the system prompt: explicit override → fn_name lookup.
    let system_prompt = params
        .system_prompt
        .unwrap_or_else(|| resolve_system_prompt(&params.agent_def.system_prompt_fn, None));

    // Resolve model: model_override > agent_def.model > params.model.
    // For Inherit we keep the parent model unchanged.
    let model = resolve_model_override(
        params
            .model_override
            .as_ref()
            .or(params.agent_def.model.as_ref()),
        &params.model,
    );

    // Fire the cache-safe params callback before entering the loop so that
    // fork children can share the same prompt-cache prefix.
    if let Some(callback) = params.on_cache_safe_params {
        callback(CacheSafeParams {
            system_prompt: system_prompt.clone(),
            user_context: params.user_context.clone(),
            system_context: params.system_context.clone(),
            fork_context_messages: filter_incomplete_tool_calls(&params.messages),
        });
    }

    let loop_params = QueryLoopParams {
        system_prompt,
        messages: params.messages,
        max_turns,
        can_use_tool: params.can_use_tool,
        allowed_tools: params.allowed_tools,
        abort_token: params.abort_token,
        provider: params.provider,
        tools: params.tools,
        model,
    };

    run_query_loop(loop_params)
}

/// Adjust a model's ID according to an `AgentModel` override.
///
/// For `Inherit` (or `None`) the parent model is returned unchanged.
/// For named aliases (Opus/Sonnet/Haiku) the model ID is replaced while all
/// other provider fields (base_url, api_key_env, …) are preserved from the
/// parent so that credentials remain consistent.
pub fn resolve_model_override(agent_model: Option<&AgentModel>, parent: &Model) -> Model {
    match agent_model {
        None | Some(AgentModel::Inherit) => parent.clone(),
        Some(AgentModel::Opus) => Model {
            id: OPUS_MODEL_ID.to_string(),
            name: OPUS_MODEL_ID.to_string(),
            ..parent.clone()
        },
        Some(AgentModel::Sonnet) => Model {
            id: SONNET_MODEL_ID.to_string(),
            name: SONNET_MODEL_ID.to_string(),
            ..parent.clone()
        },
        Some(AgentModel::Haiku) => Model {
            id: HAIKU_MODEL_ID.to_string(),
            name: HAIKU_MODEL_ID.to_string(),
            ..parent.clone()
        },
        Some(AgentModel::Custom(id)) => Model {
            id: id.clone(),
            name: id.clone(),
            ..parent.clone()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use ai::types::{
        AssistantMessageEvent, InputType, LlmContext, LlmTool, Model, ModelCost, StopReason, api,
    };

    fn test_model() -> Model {
        Model {
            id: "test-model".into(),
            name: "Test Model".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    struct MockProvider {
        responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            let mut q = self.responses.lock().unwrap();
            q.pop_front().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
    }

    fn make_allow_all() -> CanUseTool {
        Arc::new(|_tool, _allowed| Ok(()))
    }

    #[test]
    fn agent_error_display() {
        assert!(
            AgentError::ToolNotFound("read".to_string())
                .to_string()
                .contains("read")
        );
        assert!(
            AgentError::MaxTurnsReached
                .to_string()
                .contains("max turns")
        );
        assert!(AgentError::Aborted.to_string().contains("aborted"));
    }

    #[test]
    fn run_agent_params_construction() {
        use super::super::builtin::register_builtin_agents;

        let agents = register_builtin_agents();
        let def = agents.get("general-purpose").cloned().unwrap();
        let provider = Arc::new(MockProvider::new(vec![]));
        let model = test_model();

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
            provider: Arc::clone(&provider) as Arc<dyn LlmProvider>,
            tools: vec![],
            model,
        };

        assert_eq!(params.max_turns, Some(1));
    }

    #[test]
    fn resolve_system_prompt_returns_nonempty_for_all_builtins() {
        let fn_names = [
            "get_general_purpose_prompt",
            "get_explore_prompt",
            "get_plan_prompt",
            "get_verification_prompt",
            "get_code_guide_prompt",
            "get_fork_prompt",
            "get_unknown_prompt",
        ];
        for name in fn_names {
            let prompt = resolve_system_prompt(name, None);
            assert!(!prompt.is_empty(), "empty prompt for {name}");
        }
    }

    #[test]
    fn resolve_model_override_inherit_keeps_parent() {
        let parent = test_model();
        let result = resolve_model_override(Some(&AgentModel::Inherit), &parent);
        assert_eq!(result.id, parent.id);
    }

    #[test]
    fn resolve_model_override_opus_changes_id() {
        let parent = test_model();
        let result = resolve_model_override(Some(&AgentModel::Opus), &parent);
        assert_eq!(result.id, OPUS_MODEL_ID);
        assert_eq!(result.name, OPUS_MODEL_ID);
        // Provider credentials are preserved.
        assert_eq!(result.api_key_env, parent.api_key_env);
    }

    #[test]
    fn resolve_model_override_sonnet_changes_id() {
        let parent = test_model();
        let result = resolve_model_override(Some(&AgentModel::Sonnet), &parent);
        assert_eq!(result.id, SONNET_MODEL_ID);
        assert_eq!(result.name, SONNET_MODEL_ID);
        assert_eq!(result.api_key_env, parent.api_key_env);
    }

    #[test]
    fn resolve_model_override_haiku_changes_id() {
        let parent = test_model();
        let result = resolve_model_override(Some(&AgentModel::Haiku), &parent);
        assert_eq!(result.id, HAIKU_MODEL_ID);
        assert_eq!(result.name, HAIKU_MODEL_ID);
        assert_eq!(result.api_key_env, parent.api_key_env);
    }

    #[test]
    fn resolve_model_override_custom_sets_id() {
        let parent = test_model();
        let result =
            resolve_model_override(Some(&AgentModel::Custom("my-model".to_string())), &parent);
        assert_eq!(result.id, "my-model");
    }

    #[tokio::test]
    async fn run_agent_yields_messages_with_explicit_system_prompt() {
        use super::super::builtin::register_builtin_agents;
        use futures::StreamExt;

        let agents = register_builtin_agents();
        let def = agents.get("general-purpose").cloned().unwrap();
        let provider = Arc::new(MockProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("done".to_string()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]));
        let model = test_model();

        let params = RunAgentParams {
            agent_def: def,
            system_prompt: Some("Custom prompt".to_string()),
            user_context: HashMap::new(),
            system_context: HashMap::new(),
            can_use_tool: make_allow_all(),
            messages: vec![AgentMessage::User(agent_core::types::UserMessage {
                content: vec![agent_core::types::Content::Text {
                    text: "hello".to_string(),
                }],
                timestamp: 0,
            })],
            max_turns: Some(5),
            allowed_tools: None,
            model_override: None,
            abort_token: None,
            on_cache_safe_params: None,
            provider: Arc::clone(&provider) as Arc<dyn LlmProvider>,
            tools: vec![],
            model,
        };

        let messages: Vec<_> = run_agent(params).collect().await;
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Ok(AgentMessage::Assistant(_)))),
            "expected at least one assistant message"
        );
    }
}
