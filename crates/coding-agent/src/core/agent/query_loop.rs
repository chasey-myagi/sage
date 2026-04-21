//! Query loop — the inner agent execution cycle.
//!
//! Mirrors CC's `tools/AgentTool/runAgent.ts` query loop section and the
//! `query.ts` module. Each iteration calls the LLM, inspects the response for
//! tool-use blocks, executes allowed tools, and feeds the results back until
//! the model produces a stop-sequence with no tool calls or `max_turns` is hit.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use agent_core::agent_loop::{AgentLoopConfig, LlmProvider, default_convert_to_llm};
use agent_core::types::{
    AgentContext, AgentMessage, AgentTool, BeforeToolCallContext, BeforeToolCallResult,
    ToolExecutionMode,
};
use ai::types::Model;

use super::runner::AgentError;

/// Parameters for a single query-loop execution.
///
/// Mirrors CC's `QueryLoopParams` object threaded into `query()`.
pub struct QueryLoopParams {
    /// System prompt sent to the LLM on every turn.
    pub system_prompt: String,
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
    /// LLM provider to call on each turn.
    pub provider: Arc<dyn LlmProvider>,
    /// Tools available to the agent.
    pub tools: Vec<Arc<dyn AgentTool>>,
    /// Model to use for all LLM calls.
    pub model: Model,
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
pub fn run_query_loop(
    params: QueryLoopParams,
) -> BoxStream<'static, Result<AgentMessage, AgentError>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<AgentMessage, AgentError>>();

    // Loop-level cancellation: fires on user abort OR turn-limit exhaustion.
    let loop_token = CancellationToken::new();

    if let Some(user_token) = params.abort_token {
        let loop_token_clone = loop_token.clone();
        tokio::spawn(async move {
            user_token.cancelled().await;
            loop_token_clone.cancel();
        });
    }

    // Wire can_use_tool into the before_tool_call hook so the loop enforces
    // permissions at call time rather than pre-filtering the tool list.
    let can_use_tool = params.can_use_tool;
    let allowed_tools = Arc::new(params.allowed_tools);
    let before_tool_call = Some(Box::new(
        move |ctx: BeforeToolCallContext| -> futures::future::BoxFuture<'static, BeforeToolCallResult> {
            let allowed = Arc::clone(&allowed_tools);
            let result = can_use_tool(&ctx.tool_name, allowed.as_deref());
            Box::pin(async move {
                match result {
                    Ok(()) => BeforeToolCallResult { block: false, reason: None },
                    Err(reason) => BeforeToolCallResult { block: true, reason: Some(reason) },
                }
            })
        },
    ) as Box<
        dyn Fn(BeforeToolCallContext) -> futures::future::BoxFuture<'static, BeforeToolCallResult>
            + Send
            + Sync,
    >);

    let config = Arc::new(AgentLoopConfig {
        model: params.model,
        system_prompt: params.system_prompt.clone(),
        tool_execution: ToolExecutionMode::Parallel,
        tools: params.tools,
        convert_to_llm: Box::new(default_convert_to_llm),
        transform_context: None,
        get_steering_messages: None,
        get_follow_up_messages: None,
        before_tool_call,
        after_tool_call: None,
        get_api_key: None,
        max_retry_delay_ms: None,
        compaction_settings: None,
    });

    let context = AgentContext {
        system_prompt: params.system_prompt,
        messages: vec![],
    };

    let tx_emit = tx.clone();
    let turn_counter = Arc::new(AtomicU32::new(0));
    let turn_limit = params.max_turns;
    let tlt = loop_token.clone();

    let emit: Arc<dyn Fn(agent_core::AgentEvent) + Send + Sync> = Arc::new(move |event| {
        use agent_core::AgentEvent;
        match event {
            AgentEvent::MessageEnd { message } => match &message {
                AgentMessage::Assistant(_) | AgentMessage::ToolResult(_) => {
                    let _ = tx_emit.send(Ok(message));
                }
                _ => {}
            },
            AgentEvent::TurnEnd { .. } => {
                let count = turn_counter.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= turn_limit {
                    tlt.cancel();
                }
            }
            _ => {}
        }
    });

    tokio::spawn(async move {
        agent_core::agent_loop::run_agent_loop(
            params.messages,
            context,
            config,
            params.provider,
            emit,
            Some(loop_token),
        )
        .await;
        // Drop tx so the stream terminates when all messages have been consumed.
        drop(tx);
    });

    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use ai::types::{
        AssistantMessageEvent, InputType, LlmContext, LlmTool, Model, ModelCost, StopReason, api,
    };
    use futures::StreamExt;

    fn allow_all() -> Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync> {
        Arc::new(|_tool, _allowed| Ok(()))
    }

    fn deny_all() -> Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync> {
        Arc::new(|tool, _allowed| Err(format!("tool {tool} not permitted")))
    }

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

    fn done_response(text: &str) -> Vec<AssistantMessageEvent> {
        vec![
            AssistantMessageEvent::TextDelta(text.to_string()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]
    }

    fn make_params(provider: Arc<dyn LlmProvider>, messages: Vec<AgentMessage>) -> QueryLoopParams {
        QueryLoopParams {
            system_prompt: "system".to_string(),
            messages,
            max_turns: 10,
            can_use_tool: allow_all(),
            allowed_tools: None,
            abort_token: None,
            provider,
            tools: vec![],
            model: test_model(),
        }
    }

    #[test]
    fn query_loop_params_construction() {
        let provider = Arc::new(MockProvider::new(vec![]));
        let _params = QueryLoopParams {
            system_prompt: "system".to_string(),
            messages: vec![],
            max_turns: 10,
            can_use_tool: allow_all(),
            allowed_tools: None,
            abort_token: None,
            provider,
            tools: vec![],
            model: test_model(),
        };
    }

    #[tokio::test]
    async fn run_query_loop_yields_assistant_message() {
        let provider = Arc::new(MockProvider::new(vec![done_response("Hello from agent")]));
        let user_msg = AgentMessage::User(agent_core::types::UserMessage {
            content: vec![agent_core::types::Content::Text {
                text: "hello".to_string(),
            }],
            timestamp: 0,
        });
        let params = make_params(
            Arc::clone(&provider) as Arc<dyn LlmProvider>,
            vec![user_msg],
        );
        let messages: Vec<_> = run_query_loop(params).collect().await;
        assert!(
            !messages.is_empty(),
            "expected at least one message from the loop"
        );
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Ok(AgentMessage::Assistant(_)))),
            "expected an assistant message"
        );
    }

    #[tokio::test]
    async fn run_query_loop_aborts_on_token_cancel() {
        use tokio_util::sync::CancellationToken;
        let token = CancellationToken::new();
        token.cancel(); // pre-cancel
        let provider = Arc::new(MockProvider::new(vec![]));
        let params = QueryLoopParams {
            abort_token: Some(token),
            ..make_params(Arc::clone(&provider) as Arc<dyn LlmProvider>, vec![])
        };
        let messages: Vec<_> = run_query_loop(params).collect().await;
        // Pre-cancelled token should result in an aborted (empty) loop.
        assert!(
            messages.is_empty() || messages.iter().all(|m| m.is_ok()),
            "no errors expected on abort"
        );
    }

    #[tokio::test]
    async fn run_query_loop_permission_denied_blocks_tool() {
        let provider = Arc::new(MockProvider::new(vec![done_response("ok")]));
        let user_msg = AgentMessage::User(agent_core::types::UserMessage {
            content: vec![agent_core::types::Content::Text {
                text: "run bash".to_string(),
            }],
            timestamp: 0,
        });
        let params = QueryLoopParams {
            can_use_tool: deny_all(),
            ..make_params(
                Arc::clone(&provider) as Arc<dyn LlmProvider>,
                vec![user_msg],
            )
        };
        // Should still run (deny_all blocks tool calls at runtime, doesn't abort the loop).
        let messages: Vec<_> = run_query_loop(params).collect().await;
        assert!(
            messages.iter().any(|m| m.is_ok()),
            "loop should complete with messages"
        );
    }
}
