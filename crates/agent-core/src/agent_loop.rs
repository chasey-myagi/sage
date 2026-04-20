// Agent Loop — mirrors pi-mono packages/agent/src/agent-loop.ts
//
// Three-layer structure:
//   run_agent_loop / run_agent_loop_continue → run_loop (inner) → stream_assistant_response
//
// Tool execution is either sequential or parallel as configured.

use crate::compaction::{
    CompactionReason, CompactionSettings, apply_compaction, calculate_context_tokens, compact,
    is_context_overflow, microcompact, prepare_compaction, should_compact, should_microcompact,
};
use crate::event::AgentEvent;
use crate::types::*;
use ai::types::{
    AssistantMessageEvent, LlmContent, LlmContext, LlmFunctionCall, LlmMessage, LlmTool,
    LlmToolCall, Model, StopReason, ThinkingBlock, Usage,
};
use std::fmt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Errors that can occur during the agent loop.
#[derive(Debug)]
pub enum AgentLoopError {
    MaxTurnsReached,
    Cancelled,
    ProviderError(String),
}

impl fmt::Display for AgentLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentLoopError::MaxTurnsReached => write!(f, "max turns reached"),
            AgentLoopError::Cancelled => write!(f, "cancelled"),
            AgentLoopError::ProviderError(msg) => write!(f, "provider error: {msg}"),
        }
    }
}

impl std::error::Error for AgentLoopError {}

/// LLM provider trait used by the agent loop.
///
/// Matches ai::LlmProvider (same signature) so both can be used interchangeably.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent>;
}

// Blanket impl: Arc<dyn LlmProvider> also implements LlmProvider.
#[async_trait::async_trait]
impl<T: ?Sized + LlmProvider> LlmProvider for Arc<T> {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        (**self).complete(model, context, tools).await
    }
}

/// Configuration for the agent loop.
///
/// Mirrors pi-mono's AgentLoopConfig interface.
pub struct AgentLoopConfig {
    pub model: Model,

    /// Converts AgentMessage[] to LLM-compatible messages before each LLM call.
    pub convert_to_llm: Box<dyn Fn(&[AgentMessage]) -> Vec<LlmMessage> + Send + Sync>,

    /// Optional transform applied to messages before `convert_to_llm`.
    pub transform_context: Option<
        Box<
            dyn Fn(Vec<AgentMessage>) -> futures::future::BoxFuture<'static, Vec<AgentMessage>>
                + Send
                + Sync,
        >,
    >,

    /// Returns steering messages to inject mid-run.
    pub get_steering_messages: Option<
        Box<dyn Fn() -> futures::future::BoxFuture<'static, Vec<AgentMessage>> + Send + Sync>,
    >,

    /// Returns follow-up messages after the agent would otherwise stop.
    pub get_follow_up_messages: Option<
        Box<dyn Fn() -> futures::future::BoxFuture<'static, Vec<AgentMessage>> + Send + Sync>,
    >,

    /// Tool execution mode.
    pub tool_execution: ToolExecutionMode,

    /// Called before a tool is executed.
    pub before_tool_call: Option<
        Box<
            dyn Fn(
                    BeforeToolCallContext,
                ) -> futures::future::BoxFuture<'static, BeforeToolCallResult>
                + Send
                + Sync,
        >,
    >,

    /// Called after a tool finishes executing.
    pub after_tool_call: Option<
        Box<
            dyn Fn(AfterToolCallContext) -> futures::future::BoxFuture<'static, AfterToolCallResult>
                + Send
                + Sync,
        >,
    >,

    /// Available tools.
    pub tools: Vec<Arc<dyn AgentTool>>,

    /// System prompt.
    pub system_prompt: String,

    /// Optional API key resolver — called with the model's api_key_env to look up a key at runtime.
    pub get_api_key: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,

    /// Maximum retry backoff delay in milliseconds.
    pub max_retry_delay_ms: Option<u64>,

    /// Compaction settings. When `Some`, compaction is enabled in the loop.
    /// When `None`, no compaction is performed (matches the default pi-mono behavior
    /// where compaction is injected externally via `transformContext`).
    pub compaction_settings: Option<CompactionSettings>,
}

// ── Public entry points ────────────────────────────────────────────────────

/// Start an agent loop with new prompt messages.
///
/// Mirrors pi-mono's `runAgentLoop` function.
pub async fn run_agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: Arc<AgentLoopConfig>,
    provider: Arc<dyn LlmProvider>,
    emit: Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> Vec<AgentMessage> {
    let mut new_messages: Vec<AgentMessage> = prompts.clone();
    let mut current_context = AgentContext {
        system_prompt: context.system_prompt.clone(),
        messages: {
            let mut m = context.messages.clone();
            m.extend(prompts.clone());
            m
        },
    };

    emit(AgentEvent::AgentStart);
    emit(AgentEvent::TurnStart);
    for prompt in &prompts {
        emit(AgentEvent::MessageStart {
            message: prompt.clone(),
        });
        emit(AgentEvent::MessageEnd {
            message: prompt.clone(),
        });
    }

    run_loop(
        &mut current_context,
        &mut new_messages,
        &config,
        provider,
        &emit,
        token,
    )
    .await;
    new_messages
}

/// Continue an agent loop from existing context (no new messages).
///
/// Mirrors pi-mono's `runAgentLoopContinue` function.
pub async fn run_agent_loop_continue(
    context: AgentContext,
    config: Arc<AgentLoopConfig>,
    provider: Arc<dyn LlmProvider>,
    emit: Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    if context.messages.is_empty() {
        return Err(AgentLoopError::ProviderError(
            "Cannot continue: no messages in context".into(),
        ));
    }
    if matches!(context.messages.last(), Some(AgentMessage::Assistant(_))) {
        return Err(AgentLoopError::ProviderError(
            "Cannot continue from message role: assistant".into(),
        ));
    }

    let mut new_messages: Vec<AgentMessage> = Vec::new();
    let mut current_context = context;

    emit(AgentEvent::AgentStart);
    emit(AgentEvent::TurnStart);

    run_loop(
        &mut current_context,
        &mut new_messages,
        &config,
        provider,
        &emit,
        token,
    )
    .await;
    Ok(new_messages)
}

// ── Main loop ─────────────────────────────────────────────────────────────

/// Main loop logic shared by run_agent_loop and run_agent_loop_continue.
///
/// Mirrors pi-mono's private `runLoop` function.
async fn run_loop(
    current_context: &mut AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    config: &AgentLoopConfig,
    provider: Arc<dyn LlmProvider>,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) {
    let mut first_turn = true;

    let mut pending_messages: Vec<AgentMessage> =
        if let Some(get_steering) = &config.get_steering_messages {
            get_steering().await
        } else {
            vec![]
        };

    loop {
        let mut has_more_tool_calls = true;

        while has_more_tool_calls || !pending_messages.is_empty() {
            if token.as_ref().map(|t| t.is_cancelled()).unwrap_or(false) {
                emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                });
                return;
            }

            if !first_turn {
                emit(AgentEvent::TurnStart);
            } else {
                first_turn = false;
            }

            if !pending_messages.is_empty() {
                for message in pending_messages.drain(..) {
                    emit(AgentEvent::MessageStart {
                        message: message.clone(),
                    });
                    emit(AgentEvent::MessageEnd {
                        message: message.clone(),
                    });
                    current_context.messages.push(message.clone());
                    new_messages.push(message);
                }
            }

            let assistant_message = stream_assistant_response(
                current_context,
                config,
                provider.clone(),
                emit,
                token.clone(),
            )
            .await;

            new_messages.push(AgentMessage::Assistant(assistant_message.clone()));

            if matches!(
                assistant_message.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) {
                emit(AgentEvent::TurnEnd {
                    message: assistant_message,
                    tool_results: vec![],
                });
                emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                });
                return;
            }

            let tool_calls: Vec<(String, String, serde_json::Value)> = assistant_message
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some((id.clone(), name.clone(), arguments.clone())),
                    _ => None,
                })
                .collect();
            has_more_tool_calls = !tool_calls.is_empty();

            let mut tool_results: Vec<ToolResultMessage> = Vec::new();
            if has_more_tool_calls {
                let results = execute_tool_calls(
                    current_context,
                    &assistant_message,
                    &tool_calls,
                    config,
                    emit,
                    token.clone(),
                )
                .await;

                for result in &results {
                    current_context
                        .messages
                        .push(AgentMessage::ToolResult(result.clone()));
                    new_messages.push(AgentMessage::ToolResult(result.clone()));
                }
                tool_results = results;
            }

            emit(AgentEvent::TurnEnd {
                message: assistant_message.clone(),
                tool_results,
            });

            // Context compaction — triggered after each turn when settings are configured.
            //
            // Two triggers mirror pi-mono's compaction logic:
            //  1. Reactive: LLM reported context overflow (stop_reason == Error + overflow pattern).
            //  2. Proactive: total context tokens exceed the threshold.
            //
            // Before full compaction, microcompact (lightweight client-side cleanup) may fire
            // at a lower threshold to strip old tool results and thinking blocks.
            if let Some(ref settings) = config.compaction_settings {
                let context_tokens = calculate_context_tokens(&assistant_message.usage);
                let context_window = config.model.context_window as u32;

                let overflow = is_context_overflow(&assistant_message, context_window);
                let needs_compact =
                    overflow || should_compact(context_tokens, context_window, settings);

                if needs_compact {
                    let reason = if overflow {
                        CompactionReason::Overflow
                    } else {
                        CompactionReason::Threshold
                    };
                    let reason_str = match reason {
                        CompactionReason::Overflow => "overflow".to_string(),
                        CompactionReason::Threshold => "threshold".to_string(),
                    };

                    // Extract previous summary for iterative compaction (if first message is
                    // a CompactionSummary, pass it forward so the LLM can update it).
                    let previous_summary: Option<String> =
                        current_context.messages.first().and_then(|m| {
                            if let AgentMessage::CompactionSummary(cs) = m {
                                Some(cs.summary.clone())
                            } else {
                                None
                            }
                        });

                    if let Some(preparation) = prepare_compaction(
                        &current_context.messages,
                        context_tokens,
                        settings,
                        previous_summary.as_deref(),
                    ) {
                        let messages_compacted = preparation.first_kept_index;
                        emit(AgentEvent::CompactionStart {
                            reason: reason_str,
                            message_count: messages_compacted,
                        });

                        match compact(preparation, provider.as_ref(), &config.model).await {
                            Ok(result) => {
                                let tokens_before = result.tokens_before;
                                apply_compaction(&mut current_context.messages, &result);
                                emit(AgentEvent::CompactionEnd {
                                    tokens_before,
                                    messages_compacted,
                                });
                            }
                            Err(e) => {
                                // Compaction failure is non-fatal: log as RunError and continue.
                                emit(AgentEvent::RunError {
                                    error: format!("compaction failed: {e}"),
                                });
                            }
                        }
                    }
                } else if should_microcompact(context_tokens, context_window, settings) {
                    // Microcompact: lightweight cleanup, no LLM call.
                    microcompact(
                        &mut current_context.messages,
                        settings.microcompact_keep_turns,
                        settings.microcompact_keep_thinking_turns,
                    );
                }
            }

            pending_messages = if let Some(get_steering) = &config.get_steering_messages {
                get_steering().await
            } else {
                vec![]
            };
        }

        let follow_up = if let Some(get_follow_up) = &config.get_follow_up_messages {
            get_follow_up().await
        } else {
            vec![]
        };

        if !follow_up.is_empty() {
            pending_messages = follow_up;
            continue;
        }

        break;
    }

    emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    });
}

// ── LLM streaming ─────────────────────────────────────────────────────────

/// Stream an assistant response from the LLM.
///
/// Mirrors pi-mono's `streamAssistantResponse`.
async fn stream_assistant_response(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    provider: Arc<dyn LlmProvider>,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> AssistantMessage {
    let messages = if let Some(transform) = &config.transform_context {
        transform(context.messages.clone()).await
    } else {
        context.messages.clone()
    };

    let llm_messages = (config.convert_to_llm)(&messages);

    let llm_tools: Vec<LlmTool> = config
        .tools
        .iter()
        .map(|t| LlmTool {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        })
        .collect();

    let llm_context = LlmContext {
        system_prompt: context.system_prompt.clone(),
        messages: llm_messages,
        max_tokens: config.model.max_tokens,
        temperature: None,
    };

    let events = provider
        .complete(&config.model, &llm_context, &llm_tools)
        .await;

    let mut accum = MessageAccumulator::new();
    let mut emitted_start = false;
    let mut last_delta = String::new();

    for event in &events {
        match event {
            AssistantMessageEvent::TextDelta(delta) => {
                if !emitted_start {
                    let partial = accum.build_partial(&config.model);
                    emit(AgentEvent::MessageStart {
                        message: AgentMessage::Assistant(partial.clone()),
                    });
                    emitted_start = true;
                    context.messages.push(AgentMessage::Assistant(partial));
                }
                accum.text.push_str(delta);
                last_delta = delta.clone();
                let partial = accum.build_partial(&config.model);
                *context.messages.last_mut().unwrap() = AgentMessage::Assistant(partial.clone());
                emit(AgentEvent::MessageUpdate {
                    message: AgentMessage::Assistant(partial),
                    delta: last_delta.clone(),
                });
            }
            AssistantMessageEvent::ThinkingDelta(delta) => {
                accum.current_thinking.push_str(delta);
                // Emit message_update so UI can show thinking in real-time,
                // mirroring pi-mono's thinking_start/thinking_delta handling.
                if emitted_start {
                    let partial = accum.build_partial(&config.model);
                    *context.messages.last_mut().unwrap() =
                        AgentMessage::Assistant(partial.clone());
                    emit(AgentEvent::MessageUpdate {
                        message: AgentMessage::Assistant(partial),
                        delta: delta.clone(),
                    });
                }
            }
            AssistantMessageEvent::ThinkingBlockEnd {
                signature,
                redacted,
            } => {
                let thinking = std::mem::take(&mut accum.current_thinking);
                let sig = if signature.is_empty() {
                    None
                } else {
                    Some(signature.clone())
                };
                accum.thinking_blocks.push(ThinkingBlockAccum {
                    thinking,
                    signature: sig,
                    redacted: *redacted,
                });
            }
            AssistantMessageEvent::ToolCallStart { id, name } => {
                accum.tool_calls.push(ToolCallAccum {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: String::new(),
                });
            }
            AssistantMessageEvent::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                // Match by id when present; fall back to the last tool call when the
                // provider sends an empty id (e.g. DashScope streaming format where
                // only the first chunk carries the tool call id).
                let target = if id.is_empty() {
                    accum.tool_calls.last_mut()
                } else {
                    accum.tool_calls.iter_mut().find(|t| t.id == *id)
                };
                if let Some(tc) = target {
                    tc.arguments.push_str(arguments_delta);
                }
            }
            AssistantMessageEvent::ToolCallEnd { .. } => {}
            // Structural markers — no content to accumulate.
            AssistantMessageEvent::Start
            | AssistantMessageEvent::TextStart { .. }
            | AssistantMessageEvent::ThinkingStart { .. } => {}
            AssistantMessageEvent::Usage(u) => {
                accum.usage = u.clone();
            }
            AssistantMessageEvent::Done { stop_reason } => {
                accum.stop_reason = stop_reason.clone();
            }
            AssistantMessageEvent::Error(msg) => {
                accum.error_message = Some(msg.clone());
                accum.stop_reason = StopReason::Error;
            }
        }
    }

    let final_message = accum.build_final(&config.model);

    if !emitted_start {
        emit(AgentEvent::MessageStart {
            message: AgentMessage::Assistant(final_message.clone()),
        });
        context
            .messages
            .push(AgentMessage::Assistant(final_message.clone()));
    } else {
        *context.messages.last_mut().unwrap() = AgentMessage::Assistant(final_message.clone());
    }
    emit(AgentEvent::MessageEnd {
        message: AgentMessage::Assistant(final_message.clone()),
    });

    final_message
}

// ── Message accumulator ────────────────────────────────────────────────────

struct ThinkingBlockAccum {
    thinking: String,
    signature: Option<String>,
    redacted: bool,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

struct MessageAccumulator {
    text: String,
    thinking_blocks: Vec<ThinkingBlockAccum>,
    current_thinking: String,
    tool_calls: Vec<ToolCallAccum>,
    usage: Usage,
    stop_reason: StopReason,
    error_message: Option<String>,
}

impl MessageAccumulator {
    fn new() -> Self {
        Self {
            text: String::new(),
            thinking_blocks: Vec::new(),
            current_thinking: String::new(),
            tool_calls: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
        }
    }

    fn build_partial(&self, model: &Model) -> AssistantMessage {
        AssistantMessage {
            content: self.build_content(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            usage: self.usage.clone(),
            stop_reason: self.stop_reason.clone(),
            error_message: self.error_message.clone(),
            timestamp: crate::types::now_ms(),
        }
    }

    fn build_final(mut self, model: &Model) -> AssistantMessage {
        if !self.current_thinking.is_empty() {
            self.thinking_blocks.push(ThinkingBlockAccum {
                thinking: std::mem::take(&mut self.current_thinking),
                signature: None,
                redacted: false,
            });
        }
        let usage = self.usage.clone();
        let stop_reason = self.stop_reason.clone();
        let error_message = self.error_message.clone();
        AssistantMessage {
            content: self.build_content_owned(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            usage,
            stop_reason,
            error_message,
            timestamp: crate::types::now_ms(),
        }
    }

    fn build_content(&self) -> Vec<Content> {
        let mut content = Vec::new();
        for tb in &self.thinking_blocks {
            if !tb.thinking.is_empty() || tb.redacted {
                content.push(Content::Thinking {
                    thinking: tb.thinking.clone(),
                    signature: tb.signature.clone(),
                    redacted: tb.redacted,
                });
            }
        }
        if !self.text.is_empty() {
            content.push(Content::Text {
                text: self.text.clone(),
            });
        }
        for tc in &self.tool_calls {
            let args = coerce_tool_args(&tc.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            content.push(Content::ToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: args,
            });
        }
        content
    }

    fn build_content_owned(self) -> Vec<Content> {
        let mut content = Vec::new();
        for tb in self.thinking_blocks {
            if !tb.thinking.is_empty() || tb.redacted {
                content.push(Content::Thinking {
                    thinking: tb.thinking,
                    signature: tb.signature,
                    redacted: tb.redacted,
                });
            }
        }
        if !self.text.is_empty() {
            content.push(Content::Text { text: self.text });
        }
        for tc in self.tool_calls {
            let args = coerce_tool_args(&tc.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            content.push(Content::ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: args,
            });
        }
        content
    }
}

fn coerce_tool_args(arguments: &str) -> serde_json::Result<serde_json::Value> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    serde_json::from_str(trimmed)
}

// ── Tool execution ─────────────────────────────────────────────────────────

async fn execute_tool_calls(
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_calls: &[(String, String, serde_json::Value)],
    config: &AgentLoopConfig,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> Vec<ToolResultMessage> {
    match config.tool_execution {
        ToolExecutionMode::Sequential => {
            execute_tool_calls_sequential(
                context,
                assistant_message,
                tool_calls,
                config,
                emit,
                token,
            )
            .await
        }
        ToolExecutionMode::Parallel => {
            execute_tool_calls_parallel(context, assistant_message, tool_calls, config, emit, token)
                .await
        }
    }
}

async fn execute_tool_calls_sequential(
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_calls: &[(String, String, serde_json::Value)],
    config: &AgentLoopConfig,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> Vec<ToolResultMessage> {
    let mut results = Vec::new();
    for (id, name, args) in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: id.clone(),
            tool_name: name.clone(),
            args: args.clone(),
        });
        let preparation =
            prepare_tool_call(context, assistant_message, id, name, args, config).await;
        match preparation {
            PreparedToolCall::Immediate { result, is_error } => {
                results.push(emit_tool_call_outcome(id, name, result, is_error, emit).await);
            }
            PreparedToolCall::Ready {
                tool,
                validated_args,
            } => {
                let executed = execute_prepared_tool_call(
                    id,
                    name,
                    &tool,
                    validated_args,
                    emit,
                    token.clone(),
                )
                .await;
                let final_result = finalize_tool_call(
                    context,
                    assistant_message,
                    id,
                    name,
                    args,
                    executed.result,
                    executed.is_error,
                    config,
                    emit,
                )
                .await;
                results.push(final_result);
            }
        }
    }
    results
}

async fn execute_tool_calls_parallel(
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_calls: &[(String, String, serde_json::Value)],
    config: &AgentLoopConfig,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> Vec<ToolResultMessage> {
    let mut immediate_results: Vec<(usize, ToolResultMessage)> = Vec::new();
    let mut runnable: Vec<(
        usize,
        String,
        String,
        serde_json::Value,
        Arc<dyn AgentTool>,
        serde_json::Value,
    )> = Vec::new();

    for (idx, (id, name, args)) in tool_calls.iter().enumerate() {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: id.clone(),
            tool_name: name.clone(),
            args: args.clone(),
        });
        let preparation =
            prepare_tool_call(context, assistant_message, id, name, args, config).await;
        match preparation {
            PreparedToolCall::Immediate { result, is_error } => {
                let tr = emit_tool_call_outcome(id, name, result, is_error, emit).await;
                immediate_results.push((idx, tr));
            }
            PreparedToolCall::Ready {
                tool,
                validated_args,
            } => {
                runnable.push((
                    idx,
                    id.clone(),
                    name.clone(),
                    args.clone(),
                    tool,
                    validated_args,
                ));
            }
        }
    }

    let mut running: Vec<(
        usize,
        String,
        String,
        serde_json::Value,
        tokio::task::JoinHandle<ExecutedToolCall>,
    )> = Vec::new();
    for (idx, id, name, orig_args, tool, validated_args) in runnable {
        let id_c = id.clone();
        let name_c = name.clone();
        let emit_c = Arc::clone(emit);
        let tok_c = token.clone();
        let handle = tokio::spawn(async move {
            execute_prepared_tool_call(&id_c, &name_c, &tool, validated_args, &emit_c, tok_c).await
        });
        running.push((idx, id, name, orig_args, handle));
    }

    let mut parallel_results: Vec<(usize, ToolResultMessage)> = Vec::new();
    for (idx, id, name, args, handle) in running {
        let executed = handle.await.unwrap_or_else(|e| ExecutedToolCall {
            result: AgentToolResult {
                content: vec![Content::Text {
                    text: format!("Tool panicked: {e}"),
                }],
                details: serde_json::Value::Null,
            },
            is_error: true,
        });
        let final_result = finalize_tool_call(
            context,
            assistant_message,
            &id,
            &name,
            &args,
            executed.result,
            executed.is_error,
            config,
            emit,
        )
        .await;
        parallel_results.push((idx, final_result));
    }

    let mut all: Vec<(usize, ToolResultMessage)> = immediate_results;
    all.extend(parallel_results);
    all.sort_by_key(|(idx, _)| *idx);
    all.into_iter().map(|(_, tr)| tr).collect()
}

enum PreparedToolCall {
    Immediate {
        result: AgentToolResult,
        is_error: bool,
    },
    Ready {
        tool: Arc<dyn AgentTool>,
        validated_args: serde_json::Value,
    },
}

struct ExecutedToolCall {
    result: AgentToolResult,
    is_error: bool,
}

/// Validate tool arguments against the tool's JSON Schema.
///
/// Mirrors pi-mono's `validateToolArguments` in `packages/ai/src/utils/validation.ts`.
///
/// pi-mono uses AJV for full JSON Schema validation when runtime code generation is
/// available, and falls back to returning the raw args otherwise. Here we perform the
/// minimal structural checks that are always safe:
///
/// 1. Args must be a JSON object (not null, array, or a primitive).
/// 2. If the schema declares `required` fields, each must be present in the args.
///
/// Validation errors are returned as `Err(String)` with a human-readable message;
/// on success the (potentially unchanged) args are returned as `Ok`.
fn validate_tool_arguments(
    tool_name: &str,
    schema: &serde_json::Value,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Args must be a JSON object — LLMs occasionally emit null or an array.
    if !args.is_object() {
        return Err(format!(
            "Validation failed for tool \"{tool_name}\": arguments must be a JSON object, got {}",
            match args {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        ));
    }

    // Check required fields when the schema declares them.
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        let obj = args.as_object().unwrap(); // safe: checked above
        let missing: Vec<&str> = required
            .iter()
            .filter_map(|r| r.as_str())
            .filter(|field| !obj.contains_key(*field))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "Validation failed for tool \"{tool_name}\": missing required field(s): {}",
                missing.join(", ")
            ));
        }
    }

    Ok(args.clone())
}

async fn prepare_tool_call(
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    id: &str,
    name: &str,
    args: &serde_json::Value,
    config: &AgentLoopConfig,
) -> PreparedToolCall {
    let tool = config.tools.iter().find(|t| t.name() == name).cloned();
    let Some(tool) = tool else {
        return PreparedToolCall::Immediate {
            result: create_error_result(&format!("Tool {name} not found")),
            is_error: true,
        };
    };

    // Validate arguments against the tool's JSON Schema before proceeding.
    // Mirrors pi-mono's `validateToolArguments` call at agent-loop.ts line 475.
    let validated_args = match validate_tool_arguments(name, &tool.parameters_schema(), args) {
        Ok(v) => v,
        Err(msg) => {
            return PreparedToolCall::Immediate {
                result: create_error_result(&msg),
                is_error: true,
            };
        }
    };

    if let Some(before) = &config.before_tool_call {
        let ctx = BeforeToolCallContext {
            assistant_message: assistant_message.clone(),
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: validated_args.clone(),
            messages: context.messages.clone(),
        };
        let result = before(ctx).await;
        if result.block {
            let reason = result
                .reason
                .unwrap_or_else(|| "Tool execution was blocked".into());
            return PreparedToolCall::Immediate {
                result: create_error_result(&reason),
                is_error: true,
            };
        }
    }

    PreparedToolCall::Ready {
        tool,
        validated_args,
    }
}

async fn execute_prepared_tool_call(
    id: &str,
    name: &str,
    tool: &Arc<dyn AgentTool>,
    args: serde_json::Value,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
    token: Option<CancellationToken>,
) -> ExecutedToolCall {
    // Build the on_update callback that emits ToolExecutionUpdate events.
    // Mirrors pi-mono's `executePreparedToolCall` where each `onUpdate` call
    // pushes a `tool_execution_update` event into the stream.
    let emit_clone = Arc::clone(emit);
    let id_str = id.to_string();
    let name_str = name.to_string();
    let args_snapshot = args.clone();
    let on_update: crate::types::OnUpdateFn = Box::new(move |partial_result: String| {
        emit_clone(AgentEvent::ToolExecutionUpdate {
            tool_call_id: id_str.clone(),
            tool_name: name_str.clone(),
            args: args_snapshot.clone(),
            partial_result,
        });
    });

    let result = tool.execute(id, args, token, Some(&on_update)).await;
    ExecutedToolCall {
        result,
        is_error: false,
    }
}

async fn finalize_tool_call(
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    id: &str,
    name: &str,
    args: &serde_json::Value,
    mut result: AgentToolResult,
    mut is_error: bool,
    config: &AgentLoopConfig,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
) -> ToolResultMessage {
    if let Some(after) = &config.after_tool_call {
        let ctx = AfterToolCallContext {
            assistant_message: assistant_message.clone(),
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            args: args.clone(),
            result: result.clone(),
            is_error,
            messages: context.messages.clone(),
        };
        let after_result = after(ctx).await;
        if let Some(content) = after_result.content {
            result.content = content;
        }
        if let Some(details) = after_result.details {
            result.details = details;
        }
        if let Some(err) = after_result.is_error {
            is_error = err;
        }
    }
    emit_tool_call_outcome(id, name, result, is_error, emit).await
}

async fn emit_tool_call_outcome(
    id: &str,
    name: &str,
    result: AgentToolResult,
    is_error: bool,
    emit: &Arc<dyn Fn(AgentEvent) + Send + Sync>,
) -> ToolResultMessage {
    emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: id.to_string(),
        tool_name: name.to_string(),
        result: result.clone(),
        is_error,
    });

    let tool_result = ToolResultMessage {
        tool_call_id: id.to_string(),
        tool_name: name.to_string(),
        content: result.content,
        details: Some(result.details),
        is_error,
        timestamp: crate::types::now_ms(),
    };

    emit(AgentEvent::MessageStart {
        message: AgentMessage::ToolResult(tool_result.clone()),
    });
    emit(AgentEvent::MessageEnd {
        message: AgentMessage::ToolResult(tool_result.clone()),
    });

    tool_result
}

fn create_error_result(message: &str) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text {
            text: message.to_string(),
        }],
        details: serde_json::Value::Object(Default::default()),
    }
}

// ── Default convert_to_llm ──────────────────────────────────────────────────

/// Default implementation of convert_to_llm.
///
/// Mirrors pi-mono's `defaultConvertToLlm` function.
pub fn default_convert_to_llm(messages: &[AgentMessage]) -> Vec<LlmMessage> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            AgentMessage::User(u) => {
                let content = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(LlmContent::Text(text.clone())),
                        Content::Image { data, mime_type } => Some(LlmContent::Image {
                            url: format!("data:{};base64,{}", mime_type, data),
                        }),
                        _ => None,
                    })
                    .collect();
                Some(LlmMessage::User { content })
            }
            AgentMessage::Assistant(a) => {
                let text: String = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();

                let tool_calls: Vec<LlmToolCall> = a
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

                let thinking_blocks: Vec<ThinkingBlock> = a
                    .content
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
                    .collect();

                Some(LlmMessage::Assistant {
                    content: text,
                    tool_calls,
                    thinking_blocks,
                })
            }
            AgentMessage::ToolResult(r) => {
                let content: String = r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                Some(LlmMessage::Tool {
                    tool_call_id: r.tool_call_id.clone(),
                    content,
                    tool_name: Some(r.tool_name.clone()),
                })
            }
            AgentMessage::CompactionSummary(cs) => Some(LlmMessage::User {
                content: vec![LlmContent::Text(format!(
                    "[Previous conversation summary]\n\n{}",
                    cs.summary
                ))],
            }),
        })
        .collect()
}

// ── Translated from agent-loop.test.ts ────────────────────────────────────

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::test_helpers::{StatefulProvider, test_model};
    use ai::types::{AssistantMessageEvent, StopReason as SR, Usage};
    use std::sync::{Arc, Mutex};

    // ── Helpers ────────────────────────────────────────────────────────────

    fn done_response(text: &str) -> Vec<AssistantMessageEvent> {
        vec![
            AssistantMessageEvent::TextDelta(text.to_string()),
            AssistantMessageEvent::Done {
                stop_reason: SR::Stop,
            },
        ]
    }

    fn tool_call_response(id: &str, name: &str, args: &str) -> Vec<AssistantMessageEvent> {
        vec![
            AssistantMessageEvent::ToolCallStart {
                id: id.to_string(),
                name: name.to_string(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: id.to_string(),
                arguments_delta: args.to_string(),
            },
            AssistantMessageEvent::ToolCallEnd { id: id.to_string() },
            AssistantMessageEvent::Done {
                stop_reason: SR::ToolUse,
            },
        ]
    }

    fn make_context(messages: Vec<AgentMessage>) -> AgentContext {
        AgentContext {
            system_prompt: "You are helpful.".into(),
            messages,
        }
    }

    fn user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::from_text(text))
    }

    fn make_config(provider: Arc<dyn LlmProvider>) -> Arc<AgentLoopConfig> {
        Arc::new(AgentLoopConfig {
            model: test_model(),
            system_prompt: "You are helpful.".into(),
            tool_execution: ToolExecutionMode::Parallel,
            tools: vec![],
            convert_to_llm: Box::new(|msgs| default_convert_to_llm(msgs)),
            transform_context: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            get_api_key: None,
            max_retry_delay_ms: None,
            compaction_settings: None,
        })
    }

    fn no_emit() -> Arc<dyn Fn(AgentEvent) + Send + Sync> {
        Arc::new(|_| {})
    }

    fn collecting_emit() -> (
        Arc<dyn Fn(AgentEvent) + Send + Sync>,
        Arc<Mutex<Vec<AgentEvent>>>,
    ) {
        let events = Arc::new(Mutex::new(Vec::<AgentEvent>::new()));
        let events_clone = Arc::clone(&events);
        let emit: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(move |e| {
            events_clone.lock().unwrap().push(e);
        });
        (emit, events)
    }

    // ── agentLoop with AgentMessage ────────────────────────────────────────

    /// Translated from: "should emit events with AgentMessage types"
    #[tokio::test]
    async fn agent_loop_emits_events_with_agent_message_types() {
        let provider = Arc::new(StatefulProvider::new(vec![done_response("Hi there!")]));
        let config = make_config(Arc::clone(&provider) as Arc<dyn LlmProvider>);
        let context = make_context(vec![]);
        let (emit, events) = collecting_emit();

        let messages = run_agent_loop(
            vec![user_msg("Hello")],
            context,
            config,
            provider,
            emit,
            None,
        )
        .await;

        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0], AgentMessage::User(_)));
        assert!(matches!(messages[1], AgentMessage::Assistant(_)));

        let collected = events.lock().unwrap();
        let types: Vec<&str> = collected
            .iter()
            .map(|e| match e {
                AgentEvent::AgentStart => "agent_start",
                AgentEvent::AgentEnd { .. } => "agent_end",
                AgentEvent::TurnStart => "turn_start",
                AgentEvent::TurnEnd { .. } => "turn_end",
                AgentEvent::MessageStart { .. } => "message_start",
                AgentEvent::MessageEnd { .. } => "message_end",
                AgentEvent::MessageUpdate { .. } => "message_update",
                AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
                AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
                _ => "other",
            })
            .collect();

        assert!(types.contains(&"agent_start"), "missing agent_start");
        assert!(types.contains(&"turn_start"), "missing turn_start");
        assert!(types.contains(&"message_start"), "missing message_start");
        assert!(types.contains(&"message_end"), "missing message_end");
        assert!(types.contains(&"turn_end"), "missing turn_end");
        assert!(types.contains(&"agent_end"), "missing agent_end");
    }

    /// Translated from: "should apply transformContext before convertToLlm"
    #[tokio::test]
    async fn agent_loop_transform_context_applied_before_convert() {
        let provider = Arc::new(StatefulProvider::new(vec![done_response("Response")]));
        let transformed_count = Arc::new(Mutex::new(0usize));
        let transformed_count_clone = Arc::clone(&transformed_count);

        let old_messages = vec![
            user_msg("old message 1"),
            AgentMessage::Assistant(AssistantMessage::from_text("old response 1")),
            user_msg("old message 2"),
            AgentMessage::Assistant(AssistantMessage::from_text("old response 2")),
        ];

        let config = Arc::new(AgentLoopConfig {
            model: test_model(),
            system_prompt: "You are helpful.".into(),
            tool_execution: ToolExecutionMode::Sequential,
            tools: vec![],
            convert_to_llm: Box::new(|msgs| default_convert_to_llm(msgs)),
            transform_context: Some(Box::new(move |msgs: Vec<AgentMessage>| {
                let count = Arc::clone(&transformed_count_clone);
                Box::pin(async move {
                    // Keep only last 2 messages (prune old ones)
                    let pruned = msgs.into_iter().rev().take(2).rev().collect::<Vec<_>>();
                    *count.lock().unwrap() = pruned.len();
                    pruned
                }) as futures::future::BoxFuture<'static, Vec<AgentMessage>>
            })),
            get_steering_messages: None,
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            get_api_key: None,
            max_retry_delay_ms: None,
            compaction_settings: None,
        });

        let context = make_context(old_messages);
        run_agent_loop(
            vec![user_msg("new message")],
            context,
            config,
            provider,
            no_emit(),
            None,
        )
        .await;

        // transformContext should have been called, keeping only last 2
        let count = *transformed_count.lock().unwrap();
        assert_eq!(
            count, 2,
            "transformContext should have pruned to 2 messages"
        );
    }

    /// Translated from: "should handle tool calls and results"
    #[tokio::test]
    async fn agent_loop_handles_tool_calls_and_results() {
        struct EchoTool;

        #[async_trait::async_trait]
        impl AgentTool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn label(&self) -> &str {
                "Echo"
            }
            fn description(&self) -> &str {
                "Echo tool"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {"value": {"type": "string"}}})
            }
            async fn execute(
                &self,
                _id: &str,
                args: serde_json::Value,
                _signal: Option<tokio_util::sync::CancellationToken>,
                _on_update: Option<&crate::types::OnUpdateFn>,
            ) -> AgentToolResult {
                let val = args["value"].as_str().unwrap_or("").to_string();
                AgentToolResult {
                    content: vec![Content::Text {
                        text: format!("echoed: {val}"),
                    }],
                    details: serde_json::Value::Null,
                }
            }
        }

        let responses = vec![
            tool_call_response("tool-1", "echo", r#"{"value":"hello"}"#),
            done_response("done"),
        ];
        let provider = Arc::new(StatefulProvider::new(responses));

        let config = Arc::new(AgentLoopConfig {
            model: test_model(),
            system_prompt: String::new(),
            tool_execution: ToolExecutionMode::Sequential,
            tools: vec![Arc::new(EchoTool) as Arc<dyn AgentTool>],
            convert_to_llm: Box::new(|msgs| default_convert_to_llm(msgs)),
            transform_context: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            get_api_key: None,
            max_retry_delay_ms: None,
            compaction_settings: None,
        });

        let context = make_context(vec![]);
        let (emit, events) = collecting_emit();

        run_agent_loop(
            vec![user_msg("echo something")],
            context,
            config,
            provider,
            emit,
            None,
        )
        .await;

        let collected = events.lock().unwrap();
        let tool_start = collected
            .iter()
            .find(|e| matches!(e, AgentEvent::ToolExecutionStart { .. }));
        let tool_end = collected
            .iter()
            .find(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }));
        assert!(tool_start.is_some(), "expected tool_execution_start");
        assert!(tool_end.is_some(), "expected tool_execution_end");
        if let Some(AgentEvent::ToolExecutionEnd { is_error, .. }) = tool_end {
            assert!(!is_error, "expected tool to succeed");
        }
    }

    /// Translated from: "should execute tool calls in parallel and emit tool results in source order"
    #[tokio::test]
    async fn agent_loop_parallel_tool_execution_preserves_order() {
        use tokio::sync::Notify;

        let first_notified = Arc::new(Notify::new());
        let first_done = Arc::new(Notify::new());

        // A tool that blocks until notified, to prove parallel execution
        struct OrderedEchoTool {
            first_notified: Arc<Notify>,
            first_done: Arc<Notify>,
        }

        #[async_trait::async_trait]
        impl AgentTool for OrderedEchoTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn label(&self) -> &str {
                "Echo"
            }
            fn description(&self) -> &str {
                "Echo tool"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                id: &str,
                args: serde_json::Value,
                _signal: Option<tokio_util::sync::CancellationToken>,
                _on_update: Option<&crate::types::OnUpdateFn>,
            ) -> AgentToolResult {
                let val = args["value"].as_str().unwrap_or("").to_string();
                if val == "first" {
                    // Signal we started, then wait for release
                    self.first_notified.notify_one();
                    self.first_done.notified().await;
                }
                if val == "second" {
                    // Wait until first has started (proving parallel)
                    self.first_notified.notified().await;
                    // Don't wait for first to finish — release it
                    self.first_done.notify_one();
                }
                AgentToolResult {
                    content: vec![Content::Text {
                        text: format!("echoed: {val}"),
                    }],
                    details: serde_json::Value::Null,
                }
            }
        }

        let tool_response = vec![
            AssistantMessageEvent::ToolCallStart {
                id: "tool-1".into(),
                name: "echo".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "tool-1".into(),
                arguments_delta: r#"{"value":"first"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "tool-1".into(),
            },
            AssistantMessageEvent::ToolCallStart {
                id: "tool-2".into(),
                name: "echo".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "tool-2".into(),
                arguments_delta: r#"{"value":"second"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "tool-2".into(),
            },
            AssistantMessageEvent::Done {
                stop_reason: SR::ToolUse,
            },
        ];
        let provider = Arc::new(StatefulProvider::new(vec![
            tool_response,
            done_response("done"),
        ]));

        let config = Arc::new(AgentLoopConfig {
            model: test_model(),
            system_prompt: String::new(),
            tool_execution: ToolExecutionMode::Parallel,
            tools: vec![Arc::new(OrderedEchoTool {
                first_notified: Arc::clone(&first_notified),
                first_done: Arc::clone(&first_done),
            }) as Arc<dyn AgentTool>],
            convert_to_llm: Box::new(|msgs| default_convert_to_llm(msgs)),
            transform_context: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            get_api_key: None,
            max_retry_delay_ms: None,
            compaction_settings: None,
        });

        let context = make_context(vec![]);
        let (emit, events) = collecting_emit();

        run_agent_loop(
            vec![user_msg("echo both")],
            context,
            config,
            provider,
            emit,
            None,
        )
        .await;

        // Collect tool result IDs from message_end events, in emission order
        let result_ids: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| {
                if let AgentEvent::MessageEnd {
                    message: AgentMessage::ToolResult(tr),
                } = e
                {
                    Some(tr.tool_call_id.clone())
                } else {
                    None
                }
            })
            .collect();

        // Results should be in source order (tool-1 before tool-2)
        assert_eq!(
            result_ids,
            vec!["tool-1", "tool-2"],
            "parallel tool results should be emitted in source call order"
        );
    }

    /// Translated from: "should inject queued messages after all tool calls complete"
    ///
    /// The steering callback should only deliver its message AFTER at least one tool
    /// has executed — mirroring the TS condition `executed.length >= 1 && !queuedDelivered`.
    /// We count tool executions via a shared counter and only return the interrupt
    /// once that counter is >= 2 (both tools done) and not yet delivered.
    #[tokio::test]
    async fn agent_loop_steering_messages_injected_after_tool_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let executed_count = Arc::new(AtomicUsize::new(0));
        let ec = Arc::clone(&executed_count);

        struct CountingEcho {
            count: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl AgentTool for CountingEcho {
            fn name(&self) -> &str {
                "echo"
            }
            fn label(&self) -> &str {
                "Echo"
            }
            fn description(&self) -> &str {
                "Echo"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _id: &str,
                args: serde_json::Value,
                _signal: Option<tokio_util::sync::CancellationToken>,
                _on_update: Option<&crate::types::OnUpdateFn>,
            ) -> AgentToolResult {
                self.count.fetch_add(1, Ordering::SeqCst);
                let val = args["value"].as_str().unwrap_or("").to_string();
                AgentToolResult {
                    content: vec![Content::Text {
                        text: format!("ok:{val}"),
                    }],
                    details: serde_json::Value::Null,
                }
            }
        }

        let two_tool_response = vec![
            AssistantMessageEvent::ToolCallStart {
                id: "tool-1".into(),
                name: "echo".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "tool-1".into(),
                arguments_delta: r#"{"value":"first"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "tool-1".into(),
            },
            AssistantMessageEvent::ToolCallStart {
                id: "tool-2".into(),
                name: "echo".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "tool-2".into(),
                arguments_delta: r#"{"value":"second"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd {
                id: "tool-2".into(),
            },
            AssistantMessageEvent::Done {
                stop_reason: SR::ToolUse,
            },
        ];
        let provider = Arc::new(StatefulProvider::new(vec![
            two_tool_response,
            done_response("done"),
        ]));

        let delivered = Arc::new(Mutex::new(false));
        let delivered_clone = Arc::clone(&delivered);
        let ec2 = Arc::clone(&executed_count);

        let config = Arc::new(AgentLoopConfig {
            model: test_model(),
            system_prompt: String::new(),
            tool_execution: ToolExecutionMode::Sequential,
            tools: vec![Arc::new(CountingEcho { count: ec }) as Arc<dyn AgentTool>],
            convert_to_llm: Box::new(|msgs| default_convert_to_llm(msgs)),
            transform_context: None,
            get_steering_messages: Some(Box::new(move || {
                let delivered = Arc::clone(&delivered_clone);
                let ec = Arc::clone(&ec2);
                Box::pin(async move {
                    let mut d = delivered.lock().unwrap();
                    // Only deliver interrupt AFTER both tools have executed (mirrors TS condition)
                    if ec.load(Ordering::SeqCst) >= 2 && !*d {
                        *d = true;
                        vec![user_msg("interrupt")]
                    } else {
                        vec![]
                    }
                }) as futures::future::BoxFuture<'static, Vec<AgentMessage>>
            })),
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            get_api_key: None,
            max_retry_delay_ms: None,
            compaction_settings: None,
        });

        let context = make_context(vec![]);
        let (emit, events) = collecting_emit();

        run_agent_loop(
            vec![user_msg("start")],
            context,
            config,
            provider,
            emit,
            None,
        )
        .await;

        let collected = events.lock().unwrap();

        // Find tool result events and steering message start events, in order
        let sequence: Vec<String> = collected
            .iter()
            .filter_map(|e| match e {
                AgentEvent::MessageStart {
                    message: AgentMessage::ToolResult(tr),
                } => Some(format!("tool:{}", tr.tool_call_id)),
                AgentEvent::MessageStart {
                    message: AgentMessage::User(u),
                } => {
                    if let Some(Content::Text { text }) = u.content.first() {
                        if text == "interrupt" {
                            return Some("interrupt".to_string());
                        }
                    }
                    None
                }
                _ => None,
            })
            .collect();

        assert!(
            sequence.contains(&"interrupt".to_string()),
            "interrupt not found in sequence"
        );
        let tool1_pos = sequence
            .iter()
            .position(|s| s == "tool:tool-1")
            .unwrap_or(usize::MAX);
        let tool2_pos = sequence
            .iter()
            .position(|s| s == "tool:tool-2")
            .unwrap_or(usize::MAX);
        let interrupt_pos = sequence
            .iter()
            .position(|s| s == "interrupt")
            .unwrap_or(usize::MAX);
        assert!(
            tool1_pos < interrupt_pos,
            "tool-1 should come before interrupt"
        );
        assert!(
            tool2_pos < interrupt_pos,
            "tool-2 should come before interrupt"
        );
    }

    // ── agentLoopContinue with AgentMessage ───────────────────────────────

    /// Translated from: "should throw when context has no messages"
    #[tokio::test]
    async fn agent_loop_continue_errors_on_empty_context() {
        let provider = Arc::new(StatefulProvider::new(vec![]));
        let config = make_config(Arc::clone(&provider) as Arc<dyn LlmProvider>);
        let context = make_context(vec![]);

        let result = run_agent_loop_continue(context, config, provider, no_emit(), None).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("no messages in context") || err_msg.contains("Cannot continue"),
            "error should mention no messages: {err_msg}"
        );
    }

    /// Translated from: "should continue from existing context without emitting user message events"
    #[tokio::test]
    async fn agent_loop_continue_does_not_re_emit_existing_user_messages() {
        let provider = Arc::new(StatefulProvider::new(vec![done_response("Response")]));
        let config = make_config(Arc::clone(&provider) as Arc<dyn LlmProvider>);
        let context = make_context(vec![user_msg("Hello")]);
        let (emit, events) = collecting_emit();

        let result = run_agent_loop_continue(context, config, provider, emit, None).await;
        assert!(result.is_ok());
        let messages = result.unwrap();

        // Should only return the new assistant message
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0], AgentMessage::Assistant(_)));

        // Should NOT have a user MessageEnd event (existing user is not re-emitted)
        let collected = events.lock().unwrap();
        let user_message_ends: Vec<_> = collected
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentEvent::MessageEnd {
                        message: AgentMessage::User(_)
                    }
                )
            })
            .collect();
        assert!(
            user_message_ends.is_empty(),
            "continue should not re-emit existing user message end events"
        );
    }

    /// Translated from: "should allow custom message types as last message (caller responsibility)"
    #[tokio::test]
    async fn agent_loop_continue_accepts_context_ending_in_user_message() {
        let provider = Arc::new(StatefulProvider::new(vec![done_response(
            "Response to custom message",
        )]));

        // Use a user message as stand-in for a "custom" message type (we can't do open union in Rust)
        let custom_msg = AgentMessage::User(UserMessage::from_text("Hook content"));
        let context = make_context(vec![custom_msg]);

        let config = make_config(Arc::clone(&provider) as Arc<dyn LlmProvider>);

        let result = run_agent_loop_continue(context, config, provider, no_emit(), None).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0], AgentMessage::Assistant(_)));
    }

    /// Additional: "should fail when last message is assistant"
    #[tokio::test]
    async fn agent_loop_continue_errors_when_last_message_is_assistant() {
        let provider = Arc::new(StatefulProvider::new(vec![]));
        let config = make_config(Arc::clone(&provider) as Arc<dyn LlmProvider>);
        let context = make_context(vec![
            user_msg("Hello"),
            AgentMessage::Assistant(AssistantMessage::from_text("Hi there")),
        ]);

        let result = run_agent_loop_continue(context, config, provider, no_emit(), None).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Cannot continue from message role: assistant"),
            "unexpected error: {err_msg}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_tool_args_empty_string() {
        let result = coerce_tool_args("").unwrap();
        assert!(result.is_object());
        assert!(result.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_coerce_tool_args_whitespace_only() {
        let result = coerce_tool_args("   ").unwrap();
        assert!(result.is_object());
    }

    #[test]
    fn test_coerce_tool_args_valid_json() {
        let result = coerce_tool_args(r#"{"key": "value"}"#).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn test_coerce_tool_args_invalid_json_returns_err() {
        let result = coerce_tool_args("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_error_result() {
        let result = create_error_result("Tool not found");
        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            Content::Text { text } => assert_eq!(text, "Tool not found"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_default_convert_to_llm_user_message() {
        let messages = vec![AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "hello".into(),
            }],
            timestamp: 0,
        })];
        let llm = default_convert_to_llm(&messages);
        assert_eq!(llm.len(), 1);
        assert!(matches!(llm[0], LlmMessage::User { .. }));
    }

    #[test]
    fn test_default_convert_to_llm_compaction_summary_becomes_user() {
        let messages = vec![AgentMessage::CompactionSummary(CompactionSummaryMessage {
            summary: "summary text".into(),
            tokens_before: 1000,
            timestamp: 0,
        })];
        let llm = default_convert_to_llm(&messages);
        assert_eq!(llm.len(), 1);
        assert!(matches!(llm[0], LlmMessage::User { .. }));
    }

    #[test]
    fn test_default_convert_to_llm_tool_result() {
        let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "tc1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            details: None,
            is_error: false,
            timestamp: 0,
        })];
        let llm = default_convert_to_llm(&messages);
        assert_eq!(llm.len(), 1);
        assert!(matches!(llm[0], LlmMessage::Tool { .. }));
    }

    #[test]
    fn test_message_accumulator_empty() {
        let accum = MessageAccumulator::new();
        let model = ai::types::Model {
            id: "test".into(),
            name: "Test".into(),
            api: ai::types::api::OPENAI_COMPLETIONS.into(),
            provider: "test".into(),
            base_url: "".into(),
            api_key_env: "".into(),
            reasoning: false,
            input: vec![],
            max_tokens: 4096,
            context_window: 128000,
            cost: ai::types::ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        };
        let msg = accum.build_final(&model);
        assert!(msg.content.is_empty());
        assert!(matches!(msg.stop_reason, StopReason::Stop));
    }

    #[test]
    fn test_agent_loop_error_display() {
        assert_eq!(
            format!("{}", AgentLoopError::MaxTurnsReached),
            "max turns reached"
        );
        assert_eq!(format!("{}", AgentLoopError::Cancelled), "cancelled");
        assert!(format!("{}", AgentLoopError::ProviderError("oops".into())).contains("oops"));
    }

    // ── validate_tool_arguments ────────────────────────────────────────────

    #[test]
    fn test_validate_tool_arguments_accepts_plain_object() {
        let schema = serde_json::json!({"type": "object"});
        let args = serde_json::json!({"key": "value"});
        let result = validate_tool_arguments("my_tool", &schema, &args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), args);
    }

    #[test]
    fn test_validate_tool_arguments_rejects_null() {
        let schema = serde_json::json!({"type": "object"});
        let args = serde_json::Value::Null;
        let result = validate_tool_arguments("my_tool", &schema, &args);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("my_tool"),
            "error should mention tool name: {msg}"
        );
        assert!(msg.contains("null"), "error should mention null: {msg}");
    }

    #[test]
    fn test_validate_tool_arguments_rejects_array() {
        let schema = serde_json::json!({"type": "object"});
        let args = serde_json::json!([1, 2, 3]);
        let result = validate_tool_arguments("my_tool", &schema, &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("array"));
    }

    #[test]
    fn test_validate_tool_arguments_rejects_string() {
        let schema = serde_json::json!({"type": "object"});
        let args = serde_json::json!("hello");
        let result = validate_tool_arguments("my_tool", &schema, &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("string"));
    }

    #[test]
    fn test_validate_tool_arguments_required_fields_present() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            }
        });
        let args = serde_json::json!({"path": "/tmp/foo.txt", "content": "hello"});
        let result = validate_tool_arguments("write_file", &schema, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tool_arguments_required_field_missing() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            }
        });
        // "content" is missing
        let args = serde_json::json!({"path": "/tmp/foo.txt"});
        let result = validate_tool_arguments("write_file", &schema, &args);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("write_file"),
            "should mention tool name: {msg}"
        );
        assert!(
            msg.contains("content"),
            "should mention missing field: {msg}"
        );
    }

    #[test]
    fn test_validate_tool_arguments_no_required_in_schema() {
        // Schema with no "required" key — any object is accepted.
        let schema = serde_json::json!({"type": "object", "properties": {}});
        let args = serde_json::json!({});
        let result = validate_tool_arguments("my_tool", &schema, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tool_arguments_empty_object_with_required_fields_fails() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["id"]
        });
        let args = serde_json::json!({});
        let result = validate_tool_arguments("get_item", &schema, &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("id"));
    }
}
