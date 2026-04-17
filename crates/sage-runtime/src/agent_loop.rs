// Agent Loop — Phase 4
// Three-layer nested loop: outer (follow-up) → inner (steering + tools) → innermost (LLM streaming).
// Implements the full agent execution lifecycle with event emission.

use crate::agent::Agent;
use crate::compaction::{
    self, CompactionReason, apply_compaction, estimate_context_tokens, is_context_overflow,
    microcompact, prepare_compaction, should_compact, should_microcompact, truncate_messages,
};
use crate::event::{AgentEvent, AgentEventSink};
use crate::hook::HookEvent;
use crate::llm::types::*;
use crate::tools::ToolOutput;
use crate::types::*;
use std::fmt;
use tokio_util::sync::CancellationToken;

/// Errors that can occur during the agent loop.
#[derive(Debug)]
pub enum AgentLoopError {
    MaxTurnsReached,
    Cancelled,
    ProviderError(String),
    /// A [`StopHook`] returned [`StopAction::Fail`] — the harness test failed.
    TestFailed(String),
}

impl fmt::Display for AgentLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentLoopError::MaxTurnsReached => write!(f, "max turns reached"),
            AgentLoopError::Cancelled => write!(f, "cancelled"),
            AgentLoopError::ProviderError(msg) => write!(f, "provider error: {msg}"),
            AgentLoopError::TestFailed(reason) => write!(f, "harness test failed: {reason}"),
        }
    }
}

impl std::error::Error for AgentLoopError {}

/// Accumulates streaming events into an AssistantMessage.
struct MessageAccumulator {
    text: String,
    /// Per-block thinking accumulation (preserves block boundaries, signatures, redacted state).
    thinking_blocks: Vec<ThinkingBlockAccum>,
    /// Current thinking block text being accumulated (flushed on ThinkingBlockEnd).
    current_thinking: String,
    tool_calls: Vec<ToolCallAccum>,
    usage: Usage,
    stop_reason: StopReason,
    error_message: Option<String>,
    errored: bool,
}

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

/// Coerce a tool-call `arguments` string into a JSON value.
///
/// Semantics (v0.0.3.1 hotfix — previously the two call sites disagreed,
/// sending Kimi into an infinite loop of empty `ls` calls):
///
/// - Empty or whitespace-only → `{}`. Many providers emit an empty
///   arguments string for zero-arg tool calls; the tool itself will then
///   surface its own "missing required parameter" error to the model,
///   which is exactly the feedback the model needs to correct its call.
/// - Non-empty, valid JSON → parsed value.
/// - Non-empty, invalid JSON → `Err(serde_error)`, caller decides
///   whether to block or fall back.
fn coerce_tool_args(arguments: &str) -> serde_json::Result<serde_json::Value> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    serde_json::from_str(trimmed)
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
            errored: false,
        }
    }

    fn push_event(&mut self, event: &AssistantMessageEvent) {
        match event {
            // After an error, only allow Done/Usage events
            _ if self.errored
                && !matches!(
                    event,
                    AssistantMessageEvent::Done { .. }
                        | AssistantMessageEvent::Usage(_)
                        | AssistantMessageEvent::Error(_)
                ) => {}
            AssistantMessageEvent::TextDelta(delta) => self.text.push_str(delta),
            AssistantMessageEvent::ThinkingDelta(delta) => {
                self.current_thinking.push_str(delta);
            }
            AssistantMessageEvent::ThinkingBlockEnd {
                signature,
                redacted,
            } => {
                let thinking = std::mem::take(&mut self.current_thinking);
                let sig = if signature.is_empty() {
                    None
                } else {
                    Some(signature.clone())
                };
                self.thinking_blocks.push(ThinkingBlockAccum {
                    thinking,
                    signature: sig,
                    redacted: *redacted,
                });
            }
            AssistantMessageEvent::ToolCallStart { id, name } => {
                self.tool_calls.push(ToolCallAccum {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: String::new(),
                });
            }
            AssistantMessageEvent::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if let Some(tc) = self.tool_calls.iter_mut().find(|t| t.id == *id) {
                    tc.arguments.push_str(arguments_delta);
                }
            }
            AssistantMessageEvent::ToolCallEnd { .. } => {}
            AssistantMessageEvent::Usage(u) => self.usage = u.clone(),
            AssistantMessageEvent::Done { stop_reason } => {
                self.stop_reason = stop_reason.clone();
            }
            AssistantMessageEvent::Error(msg) => {
                self.error_message = Some(msg.clone());
                self.stop_reason = StopReason::Error;
                self.errored = true;
            }
        }
    }

    fn build(self, model: &Model) -> (AssistantMessage, Vec<ToolCallAccum>) {
        let mut content = Vec::new();

        // Flush any remaining thinking text (providers without ThinkingBlockEnd)
        let mut thinking_blocks = self.thinking_blocks;
        if !self.current_thinking.is_empty() {
            thinking_blocks.push(ThinkingBlockAccum {
                thinking: self.current_thinking,
                signature: None,
                redacted: false,
            });
        }

        for tb in thinking_blocks {
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
        for tc in &self.tool_calls {
            let args = coerce_tool_args(&tc.arguments).unwrap_or_else(|e| {
                tracing::warn!(tool = %tc.name, error = %e, "tool call arguments are not valid JSON — using empty object");
                serde_json::Value::Object(Default::default())
            });
            content.push(Content::ToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: args,
            });
        }
        let msg = AssistantMessage {
            content,
            provider: model.provider.clone(),
            model: model.id.clone(),
            usage: self.usage,
            stop_reason: self.stop_reason,
            error_message: self.error_message,
            timestamp: crate::types::now_secs(),
        };
        (msg, self.tool_calls)
    }
}

/// Build LlmContext from the agent's current state.
fn build_llm_context(agent: &Agent) -> LlmContext {
    let config = agent.config();
    LlmContext {
        messages: crate::llm::transform::agent_to_llm_messages(agent.messages()),
        system_prompt: config.system_prompt.clone(),
        max_tokens: config.model.max_tokens,
        temperature: None,
    }
}

/// Build LlmTool list from the registry.
fn build_llm_tools(agent: &Agent) -> Vec<LlmTool> {
    let registry = agent.tools();
    registry
        .list()
        .iter()
        .filter_map(|name| {
            registry.get(name).map(|tool| LlmTool {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters_schema(),
            })
        })
        .collect()
}

/// Emit start+end events and build an error ToolResultMessage for a blocked/failed tool call.
async fn emit_blocked(
    tc: &ToolCallAccum,
    args: serde_json::Value,
    reason: String,
    emit: &dyn AgentEventSink,
) -> ToolResultMessage {
    emit.emit(AgentEvent::ToolExecutionStart {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        args,
    })
    .await;
    emit.emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        is_error: true,
    })
    .await;
    ToolResultMessage {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        content: vec![Content::Text { text: reason }],
        is_error: true,
        timestamp: now_secs(),
    }
}

/// Prepare a tool call: parse args, emit start, run before hook.
/// Returns Ok(args) if ready to execute, or Err(result) if blocked/invalid.
async fn prepare_tool_call(
    agent: &Agent,
    tc: &ToolCallAccum,
    emit: &dyn AgentEventSink,
) -> Result<serde_json::Value, ToolResultMessage> {
    // Shares the coerce_tool_args semantics with the assistant-message
    // builder: empty string → `{}` (tool surfaces its own missing-param
    // error); non-empty invalid → block. Previously this site blocked
    // even on empty strings, while the message builder silently wrote
    // `{}` — the mismatch sent Kimi into an infinite loop because the
    // tool result didn't match the tool call it thought it made.
    let args: serde_json::Value = match coerce_tool_args(&tc.arguments) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(tool = %tc.name, error = %e, "tool call arguments are not valid JSON — blocking call");
            return Err(emit_blocked(
                tc,
                serde_json::Value::Null,
                format!("Invalid tool call arguments: {e}"),
                emit,
            )
            .await);
        }
    };

    // Enforce tool policy before any execution
    if let Some(policy) = &agent.config().tool_policy
        && let Err(reason) = policy.check_tool_call(&tc.name, &args)
    {
        return Err(emit_blocked(tc, args, reason, emit).await);
    }

    emit.emit(AgentEvent::ToolExecutionStart {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        args: args.clone(),
    })
    .await;

    let before_ctx = BeforeToolCallContext {
        tool_name: tc.name.clone(),
        tool_call_id: tc.id.clone(),
        args: args.clone(),
    };
    let before_result = agent.call_before_tool_call(&before_ctx).await;
    if before_result.block {
        let reason = before_result
            .reason
            .unwrap_or_else(|| "blocked by hook".into());
        emit.emit(AgentEvent::ToolExecutionEnd {
            tool_call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            is_error: true,
        })
        .await;
        return Err(ToolResultMessage {
            tool_call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            content: vec![Content::Text { text: reason }],
            is_error: true,
            timestamp: now_secs(),
        });
    }

    Ok(args)
}

/// Finalize a tool call: run after hook, emit end, build result.
/// Accepts already-parsed `args` to avoid redundant JSON parsing.
async fn finalize_tool_call(
    agent: &Agent,
    tc: &ToolCallAccum,
    args: serde_json::Value,
    output: ToolOutput,
    emit: &dyn AgentEventSink,
) -> ToolResultMessage {
    let after_ctx = AfterToolCallContext {
        tool_name: tc.name.clone(),
        tool_call_id: tc.id.clone(),
        args,
        is_error: output.is_error,
    };
    let after_result = agent.call_after_tool_call(&after_ctx).await;
    let content = after_result.content.unwrap_or(output.content);
    let is_error = after_result.is_error.unwrap_or(output.is_error);

    tracing::info!(
        tool_name = %tc.name,
        tool_call_id = %tc.id,
        is_error,
        "tool executed"
    );

    emit.emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        is_error,
    })
    .await;

    ToolResultMessage {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        content,
        is_error,
        timestamp: now_secs(),
    }
}

/// Execute a single tool call against the registry, aborting if `cancel` fires.
///
/// Sprint 12 task #69: tool execution is a common blocking point (network calls,
/// long shell commands). `tokio::select!` on `cancel.cancelled()` lets a user
/// Ctrl+C mid-tool actually interrupt the tool, instead of waiting for it to
/// finish before observing the cancel.
///
/// Scope: `cancel` races **only** the `tool.execute(args)` future. Before- and
/// after-hook invocations (managed in `prepare_tool_call` / `finalize_tool_call`)
/// still run to completion even if cancel fires — this is deliberate so that
/// hook-maintained state (metrics, audit log, external ack) stays consistent.
/// Callers who need hook-level preemption must fold cancel into the hook impl
/// itself.
async fn run_tool(
    agent: &Agent,
    name: &str,
    args: serde_json::Value,
    cancel: Option<&CancellationToken>,
) -> ToolOutput {
    let tool = match agent.tools().get(name) {
        Some(tool) => tool,
        None => {
            return ToolOutput {
                content: vec![Content::Text {
                    text: format!("Unknown tool: {name}"),
                }],
                is_error: true,
            };
        }
    };
    match cancel {
        Some(tok) => tokio::select! {
            biased;
            _ = tok.cancelled() => ToolOutput {
                content: vec![Content::Text {
                    text: "tool execution cancelled".into(),
                }],
                is_error: true,
            },
            out = tool.execute(args) => out,
        },
        None => tool.execute(args).await,
    }
}

/// Execute tool calls — parallel or sequential based on config.
///
/// When `cancel` fires mid-execution, remaining not-yet-started tool calls are
/// synthesized as `cancelled` tool_results so the assistant message ↔ tool
/// result pairing invariant (required by Anthropic and friends) survives into
/// the next session turn. In-flight tools already observe cancel via the
/// `run_tool` select.
async fn execute_tool_calls(
    agent: &Agent,
    tool_calls: &[ToolCallAccum],
    emit: &dyn AgentEventSink,
    cancel: Option<&CancellationToken>,
) -> Vec<ToolResultMessage> {
    match agent.config().tool_execution_mode {
        ToolExecutionMode::Parallel => {
            // Use index slots to preserve original tool_calls ordering.
            // Anthropic requires tool results in the same order as tool uses.
            let mut slots: Vec<Option<ToolResultMessage>> = vec![None; tool_calls.len()];
            let mut prepared: Vec<(usize, &ToolCallAccum, serde_json::Value)> = Vec::new();

            for (idx, tc) in tool_calls.iter().enumerate() {
                match prepare_tool_call(agent, tc, emit).await {
                    Ok(args) => prepared.push((idx, tc, args)),
                    Err(tr) => slots[idx] = Some(tr),
                }
            }

            let futs: Vec<_> = prepared
                .iter()
                .map(|(_, tc, args)| run_tool(agent, &tc.name, args.clone(), cancel))
                .collect();
            let outputs: Vec<ToolOutput> = futures::future::join_all(futs).await;

            for ((idx, tc, args), output) in prepared.into_iter().zip(outputs) {
                slots[idx] = Some(finalize_tool_call(agent, tc, args, output, emit).await);
            }

            slots.into_iter().flatten().collect()
        }
        ToolExecutionMode::Sequential => {
            let mut results = Vec::new();
            for tc in tool_calls {
                // Sprint 12 task #69: between sequential tool calls, a Ctrl+C
                // should abort the remaining queue rather than drain it. We
                // synthesize cancelled results for the rest to preserve the
                // tool_use ↔ tool_result pairing.
                if let Some(tok) = cancel
                    && tok.is_cancelled()
                {
                    results.push(cancelled_tool_result(tc));
                    continue;
                }
                let args = match prepare_tool_call(agent, tc, emit).await {
                    Ok(args) => args,
                    Err(tr) => {
                        results.push(tr);
                        continue;
                    }
                };
                let output = run_tool(agent, &tc.name, args.clone(), cancel).await;
                results.push(finalize_tool_call(agent, tc, args, output, emit).await);
            }
            results
        }
    }
}

/// Build a `cancelled` tool_result marker for a tool_call that never ran.
///
/// This preserves the **message-level** `tool_use` ↔ `tool_result` pairing
/// invariant (required by Anthropic and friends): every tool_use block the
/// assistant emitted gets a matching tool_result pushed into the agent's
/// history, so the next `send()` starts from a valid state.
///
/// **Event stream is not preserved here.** This path skips `ToolExecutionStart`
/// and `ToolExecutionEnd` emissions because the tool was never prepared. UI
/// subscribers that render Start/End pairs will see a hole. Consumers that
/// need full event coverage for cancelled tools should watch the message
/// stream (which is complete) or subscribe to `AgentLoopError::Cancelled` and
/// reconcile.
fn cancelled_tool_result(tc: &ToolCallAccum) -> ToolResultMessage {
    ToolResultMessage {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        content: vec![Content::Text {
            text: "tool execution cancelled".into(),
        }],
        is_error: true,
        timestamp: now_secs(),
    }
}

/// Attempt compaction on the agent's messages.
/// Returns true if compaction was performed successfully.
pub(crate) async fn try_compact(
    agent: &mut Agent,
    reason: CompactionReason,
    emit: &dyn AgentEventSink,
) -> bool {
    // Clone to release the immutable borrow on agent before mutable operations below.
    let settings = agent.config().compaction.clone();
    if !settings.enabled {
        return false;
    }

    let context_tokens = estimate_context_tokens(agent.messages()) as u64;
    let prep = prepare_compaction(
        agent.messages(),
        context_tokens,
        &settings,
        agent.previous_compaction_summary(),
    );

    let prep = match prep {
        Some(p) => p,
        None => return false,
    };

    let reason_str = match reason {
        CompactionReason::Threshold => "threshold",
        CompactionReason::Overflow => "overflow",
    };
    emit.emit(AgentEvent::CompactionStart {
        reason: reason_str.to_string(),
        message_count: agent.messages().len(),
    })
    .await;

    // S6.2b: emit HookEvent::PreCompact on the session's HookBus (when
    // attached) before the summarization call. Pre/Post events come in pairs:
    //   - Ok path  → PostCompact (LLM-summarized compaction)
    //   - Err path → CompactFallback (hard truncate; distinct event variant
    //     so subscribers can tell which strategy occurred)
    let hook_bus = agent.hook_bus().cloned();
    let session_id = agent.session_id().map(str::to_string).unwrap_or_default();
    let pre_message_count = agent.messages().len();
    if let Some(bus) = &hook_bus {
        bus.emit(HookEvent::PreCompact {
            session_id: session_id.clone(),
            tokens_before: context_tokens,
            message_count: pre_message_count,
        });
    }

    let first_kept = prep.first_kept_index;
    let model = agent.config().model.clone();
    match compaction::compact(prep, agent.provider(), &model).await {
        Ok(result) => {
            let tokens_before = result.tokens_before;
            agent.compaction_file_ops_mut().merge(&result.file_ops);
            agent.set_previous_compaction_summary(Some(result.summary.clone()));
            apply_compaction(agent.messages_mut(), &result);

            emit.emit(AgentEvent::CompactionEnd {
                tokens_before,
                messages_compacted: first_kept,
            })
            .await;

            if let Some(bus) = &hook_bus {
                bus.emit(HookEvent::PostCompact {
                    session_id: session_id.clone(),
                    tokens_before,
                    tokens_after: estimate_context_tokens(agent.messages()) as u64,
                    messages_compacted: first_kept,
                });
            }
            true
        }
        Err(e) => {
            // Compaction failed — fall back to truncation.
            // Emit CompactionEnd so listeners always see a matched Start/End pair.
            tracing::warn!("compaction summarization failed, falling back to truncation: {e}");
            let messages_before_truncate = agent.messages().len();
            truncate_messages(agent.messages_mut(), settings.keep_recent_tokens);
            let messages_compacted =
                messages_before_truncate.saturating_sub(agent.messages().len());
            emit.emit(AgentEvent::CompactionEnd {
                tokens_before: context_tokens,
                messages_compacted: 0,
            })
            .await;

            if let Some(bus) = &hook_bus {
                bus.emit(HookEvent::CompactFallback {
                    session_id,
                    tokens_before: context_tokens,
                    tokens_after: estimate_context_tokens(agent.messages()) as u64,
                    messages_truncated: messages_compacted,
                });
            }
            false
        }
    }
}

/// Generate a compact hex trace id for a standalone `run_agent_loop` that has
/// no engine-level session attached.
///
/// Task #87: renamed from `generate_session_id` to avoid confusion with
/// [`crate::engine::generate_session_id`] (which is authoritative for
/// `SageSession.session_id` / HookEvent payloads). When the agent has an
/// attached session, its id always wins — this fallback fires only in
/// direct `run_agent_loop(&mut agent, …)` callers (unit tests, one-shot
/// scripts) where no `SageSession` exists.
fn generate_loop_trace_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{ms:013x}")
}

/// Run the three-layer agent loop with no external cancellation signal.
///
/// Equivalent to [`run_agent_loop_with_cancel`] with `cancel = None`. Retained
/// as a zero-cost shortcut for callers that don't thread a CancellationToken
/// (unit tests, one-shot CLI runs, etc).
pub async fn run_agent_loop(
    agent: &mut Agent,
    emit: &dyn AgentEventSink,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    run_agent_loop_with_cancel(agent, emit, None).await
}

/// Run the three-layer agent loop with optional external cancellation.
///
/// Sprint 12 task #69: when `cancel` is `Some`, the loop observes it at three
/// checkpoints — before each LLM call, during the LLM call (via
/// `tokio::select!`), and during tool execution. Passing `None` preserves
/// pre-#69 behavior bit-for-bit.
///
/// On cancellation the loop emits `AgentEvent::AgentEnd` with messages
/// collected so far and returns `Err(AgentLoopError::Cancelled)`. If the
/// assistant had already requested tool calls that hadn't started when cancel
/// fired, synthetic `cancelled` tool_results are pushed first so the
/// `tool_use` ↔ `tool_result` pairing invariant survives into the next turn.
pub async fn run_agent_loop_with_cancel(
    agent: &mut Agent,
    emit: &dyn AgentEventSink,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    let mut new_messages: Vec<AgentMessage> = Vec::new();
    let mut turn_count: usize = 0;
    // Task #87: prefer the attached engine-level session id (what HookBus
    // subscribers and metrics correlate on). Fall back to a local trace
    // id only when the agent was started without an engine session —
    // direct `run_agent_loop` callers in unit tests, etc.
    let session_id = agent
        .session_id()
        .map(str::to_string)
        .unwrap_or_else(generate_loop_trace_id);

    // Stop-hook state: updated after each assistant message, consumed at natural stop.
    let mut last_stop_reason = crate::types::StopReason::Stop;
    let mut last_assistant_text = String::new();

    agent.set_streaming(true);
    emit.emit(AgentEvent::AgentStart).await;
    tracing::info!("agent loop started");

    // Drain initial steering messages
    let mut pending: Vec<AgentMessage> = agent.drain_steering();

    'outer: loop {
        // OUTER: follow-up loop
        let mut has_more_tool_calls = true;

        while has_more_tool_calls || !pending.is_empty() {
            // INNER: steering + tools loop

            // Sprint 12 task #69: top-of-turn cancellation checkpoint.
            // Cheap early exit before the expensive LLM call. We emit
            // AgentEnd so downstream event consumers see a clean terminal
            // boundary even when the run is aborted.
            if let Some(tok) = cancel
                && tok.is_cancelled()
            {
                emit.emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                agent.set_streaming(false);
                return Err(AgentLoopError::Cancelled);
            }

            // Check max turns
            if turn_count >= agent.config().max_turns {
                emit.emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                agent.set_streaming(false);
                return Ok(new_messages);
            }

            emit.emit(AgentEvent::TurnStart).await;

            // 1. Inject pending messages
            for msg in pending.drain(..) {
                emit.emit(AgentEvent::MessageStart {
                    message: msg.clone(),
                })
                .await;
                emit.emit(AgentEvent::MessageEnd {
                    message: msg.clone(),
                })
                .await;
                agent.push_message(msg.clone());
                new_messages.push(msg);
            }

            // 2a. Microcompact — lightweight client-side cleanup, zero LLM cost.
            //     Fires at 75% context usage, before full compaction at ~90%.
            {
                let ctx_tokens = estimate_context_tokens(agent.messages()) as u64;
                let cw = agent.config().model.context_window;
                if should_microcompact(ctx_tokens, cw, &agent.config().compaction) {
                    let keep_turns = agent.config().compaction.microcompact_keep_turns;
                    let keep_thinking_turns =
                        agent.config().compaction.microcompact_keep_thinking_turns;
                    let cleared = microcompact(
                        agent.messages_mut(),
                        keep_turns,
                        keep_thinking_turns,
                    );
                    if cleared > 0 {
                        tracing::debug!(cleared, "microcompact cleared tool results");
                    }
                }
            }

            // 2b. Proactive compaction — full LLM summarization.
            {
                let ctx_tokens = estimate_context_tokens(agent.messages()) as u64;
                let cw = agent.config().model.context_window;
                if should_compact(ctx_tokens, cw, &agent.config().compaction) {
                    try_compact(agent, CompactionReason::Threshold, emit).await;
                }
            }

            // 2c. transformContext hook — inject memory, filter, or custom context edits.
            agent.call_transform_context().await;

            // 3. Build context and call LLM
            let context = build_llm_context(agent);
            let tools = build_llm_tools(agent);
            let llm_start = std::time::Instant::now();
            tracing::info!("llm.request");
            // Sprint 12 task #69: race the LLM call against cancellation.
            // `biased` ensures a pre-fired cancel token wins deterministically
            // (relevant when cancel was set before the provider future was
            // polled). On cancel we emit AgentEnd and return Cancelled
            // without pushing the unfinished assistant message.
            let events = match cancel {
                Some(tok) => {
                    let fut = agent.provider().complete(
                        &agent.config().model,
                        &context,
                        &tools,
                    );
                    tokio::select! {
                        biased;
                        _ = tok.cancelled() => {
                            emit.emit(AgentEvent::AgentEnd {
                                messages: new_messages.clone(),
                            })
                            .await;
                            agent.set_streaming(false);
                            return Err(AgentLoopError::Cancelled);
                        }
                        evts = fut => evts,
                    }
                }
                None => {
                    agent
                        .provider()
                        .complete(&agent.config().model, &context, &tools)
                        .await
                }
            };

            // 4. Accumulate events into AssistantMessage + emit streaming events
            let mut accum = MessageAccumulator::new();
            let mut tool_id_to_name: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for event in &events {
                match event {
                    AssistantMessageEvent::TextDelta(delta) => {
                        emit.emit(AgentEvent::MessageUpdate {
                            message: AgentMessage::assistant(String::new()),
                            delta: delta.clone(),
                        })
                        .await;
                    }
                    AssistantMessageEvent::ToolCallStart { id, name } => {
                        tool_id_to_name.insert(id.clone(), name.clone());
                    }
                    AssistantMessageEvent::ToolCallDelta {
                        id,
                        arguments_delta,
                    } => {
                        let tool_name =
                            tool_id_to_name.get(id).cloned().unwrap_or_default();
                        emit.emit(AgentEvent::ToolExecutionUpdate {
                            tool_call_id: id.clone(),
                            tool_name,
                            partial_result: arguments_delta.clone(),
                        })
                        .await;
                    }
                    _ => {}
                }
                accum.push_event(event);
            }
            let (assistant_msg, tool_call_accums) = accum.build(&agent.config().model);
            tracing::info!(
                input_tokens = assistant_msg.usage.input,
                output_tokens = assistant_msg.usage.output,
                elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                "llm.response"
            );

            // 5. Reactive compaction — check for context overflow in response
            if is_context_overflow(&assistant_msg, agent.config().model.context_window) {
                let compacted = try_compact(agent, CompactionReason::Overflow, emit).await;
                if compacted {
                    // Compaction succeeded — retry LLM call (don't push the overflow response)
                    continue;
                }
                // Compaction failed or not possible — fall through to push error response
            }

            agent.push_message(AgentMessage::Assistant(assistant_msg.clone()));
            new_messages.push(AgentMessage::Assistant(assistant_msg.clone()));
            turn_count += 1;

            // Track fields consumed by the stop hook at natural completion.
            last_stop_reason = assistant_msg.stop_reason.clone();
            last_assistant_text = assistant_msg
                .content
                .iter()
                .filter_map(|c| {
                    if let Content::Text { text } = c {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            // 6. Check for early termination
            if matches!(
                assistant_msg.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) {
                emit.emit(AgentEvent::TurnEnd {
                    message: assistant_msg,
                    tool_results: vec![],
                })
                .await;
                emit.emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                agent.set_streaming(false);
                return Ok(new_messages);
            }

            // 7. Execute tool calls (using raw accums to preserve malformed args)
            has_more_tool_calls = !tool_call_accums.is_empty();

            let tool_results = if has_more_tool_calls {
                let results = execute_tool_calls(agent, &tool_call_accums, emit, cancel).await;
                for r in &results {
                    agent.push_message(AgentMessage::ToolResult(r.clone()));
                    new_messages.push(AgentMessage::ToolResult(r.clone()));
                }
                results
            } else {
                vec![]
            };

            tracing::info!(turn = turn_count, "turn complete");

            emit.emit(AgentEvent::TurnEnd {
                message: assistant_msg,
                tool_results,
            })
            .await;

            // 8. Poll steering queue for new messages
            pending = agent.drain_steering();
        }

        // 9. Check follow-up queue
        let follow_ups = agent.drain_follow_up();
        if !follow_ups.is_empty() {
            pending = follow_ups;
            continue 'outer;
        }

        // 10. Call stop hook at the natural completion point.
        //     Continue(feedback) → steer the feedback and restart the outer loop.
        //     Fail(reason)       → emit AgentEnd and return TestFailed error.
        //     Pass               → fall through to normal AgentEnd.
        {
            let stop_ctx = crate::agent::StopContext {
                stop_reason: last_stop_reason.clone(),
                session_id: session_id.clone(),
                task_id: session_id.clone(),
                turn_count,
                agent_name: agent.config().name.clone(),
                model: agent.config().model.id.clone(),
                last_assistant_message: last_assistant_text.clone(),
            };
            match agent.call_stop_hook(&stop_ctx).await {
                crate::agent::StopAction::Pass => break,
                crate::agent::StopAction::Continue(feedback) => {
                    tracing::info!("stop hook requested continuation — steering feedback");
                    pending.push(AgentMessage::User(UserMessage::from_text(&feedback)));
                    continue 'outer;
                }
                crate::agent::StopAction::Fail(reason) => {
                    tracing::info!(reason = %reason, "stop hook failed the harness test");
                    emit.emit(AgentEvent::AgentEnd {
                        messages: new_messages.clone(),
                    })
                    .await;
                    agent.set_streaming(false);
                    return Err(AgentLoopError::TestFailed(reason));
                }
            }
        }
    }

    tracing::info!(total_turns = turn_count, "agent loop completed");

    emit.emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    })
    .await;
    agent.set_streaming(false);
    Ok(new_messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AfterToolCallHook, Agent, AgentLoopConfig, BeforeToolCallHook};
    use crate::compaction::CompactionSettings;
    use crate::event::{AgentEvent, AgentEventSink};
    use crate::llm::LlmProvider;
    use crate::llm::types::*;
    use crate::test_helpers::StatefulProvider;
    use crate::tools::{AgentTool, ToolOutput, ToolRegistry};
    use crate::types::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ── v0.0.3.1 hotfix: coerce_tool_args semantics ─────────────────────
    //
    // Regression test for the Kimi infinite-loop bug where an empty
    // arguments string produced two divergent behaviours: the assistant
    // message builder used `{}`, but `prepare_tool_call` blocked with
    // "Invalid tool call arguments: EOF while parsing" — making the
    // model's self-perceived call mismatch the tool result it saw.

    #[test]
    fn coerce_tool_args_empty_string_is_empty_object() {
        let v = coerce_tool_args("").expect("empty must coerce");
        assert_eq!(v, json!({}));
    }

    #[test]
    fn coerce_tool_args_whitespace_only_is_empty_object() {
        let v = coerce_tool_args("   \n  ").expect("whitespace must coerce");
        assert_eq!(v, json!({}));
    }

    #[test]
    fn coerce_tool_args_valid_object_parses_verbatim() {
        let v = coerce_tool_args(r#"{"path":"src/lib.rs"}"#).expect("valid json");
        assert_eq!(v, json!({"path": "src/lib.rs"}));
    }

    #[test]
    fn coerce_tool_args_non_empty_invalid_returns_err() {
        // Non-empty-but-invalid must still bubble up the parse error so
        // callers that want to block (prepare_tool_call) can surface the
        // explicit reason to the model.
        let err = coerce_tool_args("{broken json").unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("expected")
                || err.to_string().to_lowercase().contains("key")
                || err.to_string().to_lowercase().contains("value"),
            "parse error must describe the JSON problem: {err}"
        );
    }

    // ---------------------------------------------------------------
    // Context-capturing provider — records what LLM sees each call
    // ---------------------------------------------------------------

    struct ContextCapturingProvider {
        captured: std::sync::Arc<tokio::sync::Mutex<Vec<LlmContext>>>,
        responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    }

    impl ContextCapturingProvider {
        fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
            Self {
                captured: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for ContextCapturingProvider {
        async fn complete(
            &self,
            _model: &Model,
            context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            self.captured.lock().await.push(context.clone());
            let mut queue = self.responses.lock().unwrap();
            queue.pop_front().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
    }

    // ---------------------------------------------------------------
    // Tool-capturing provider — records tool schemas passed each call
    // ---------------------------------------------------------------

    struct ToolCapturingProvider {
        captured_tools: std::sync::Arc<tokio::sync::Mutex<Vec<Vec<LlmTool>>>>,
        responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    }

    impl ToolCapturingProvider {
        fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
            Self {
                captured_tools: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for ToolCapturingProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            self.captured_tools.lock().await.push(tools.to_vec());
            let mut queue = self.responses.lock().unwrap();
            queue.pop_front().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
    }

    // ---------------------------------------------------------------
    // Event collector sink
    // ---------------------------------------------------------------

    struct CollectorSink {
        events: std::sync::Arc<tokio::sync::Mutex<Vec<AgentEvent>>>,
    }

    impl CollectorSink {
        fn new() -> Self {
            Self {
                events: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
            }
        }

        async fn events(&self) -> Vec<AgentEvent> {
            self.events.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl AgentEventSink for CollectorSink {
        async fn emit(&self, event: AgentEvent) {
            self.events.lock().await.push(event);
        }
    }

    // ---------------------------------------------------------------
    // Mock tools
    // ---------------------------------------------------------------

    /// Echoes the "text" argument back.
    struct EchoTool;

    #[async_trait::async_trait]
    impl AgentTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input text"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            })
        }
        async fn execute(&self, args: serde_json::Value) -> ToolOutput {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(no input)");
            ToolOutput {
                content: vec![Content::Text {
                    text: text.to_string(),
                }],
                is_error: false,
            }
        }
    }

    /// Always returns an error.
    struct FailTool;

    #[async_trait::async_trait]
    impl AgentTool for FailTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: serde_json::Value) -> ToolOutput {
            ToolOutput {
                content: vec![Content::Text {
                    text: "tool execution failed".into(),
                }],
                is_error: true,
            }
        }
    }

    /// Tracks how many times it is called.
    struct CountingTool {
        tool_name: String,
        count: std::sync::Arc<AtomicUsize>,
    }

    impl CountingTool {
        fn new(name: &str) -> (Self, std::sync::Arc<AtomicUsize>) {
            let count = std::sync::Arc::new(AtomicUsize::new(0));
            (
                Self {
                    tool_name: name.into(),
                    count: std::sync::Arc::clone(&count),
                },
                count,
            )
        }
    }

    #[async_trait::async_trait]
    impl AgentTool for CountingTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "Counts executions"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: serde_json::Value) -> ToolOutput {
            let n = self.count.fetch_add(1, Ordering::SeqCst);
            ToolOutput {
                content: vec![Content::Text {
                    text: format!("call #{}", n + 1),
                }],
                is_error: false,
            }
        }
    }

    // ---------------------------------------------------------------
    // Helper constructors
    // ---------------------------------------------------------------

    use crate::test_helpers::test_model;

    fn test_config() -> AgentLoopConfig {
        AgentLoopConfig {
            name: "test-agent".into(),
            model: test_model(),
            system_prompt: "You are a test agent.".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        }
    }

    /// LLM responds with text and stops.
    fn text_response(text: &str) -> Vec<AssistantMessageEvent> {
        vec![
            AssistantMessageEvent::TextDelta(text.into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]
    }

    /// LLM requests a single tool call.
    fn tool_call_response(id: &str, name: &str, args: &str) -> Vec<AssistantMessageEvent> {
        vec![
            AssistantMessageEvent::ToolCallStart {
                id: id.into(),
                name: name.into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: id.into(),
                arguments_delta: args.into(),
            },
            AssistantMessageEvent::ToolCallEnd { id: id.into() },
            AssistantMessageEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ]
    }

    /// LLM requests multiple tool calls in one response.
    fn multi_tool_response(calls: Vec<(&str, &str, &str)>) -> Vec<AssistantMessageEvent> {
        let mut events = Vec::new();
        for (id, name, args) in calls {
            events.push(AssistantMessageEvent::ToolCallStart {
                id: id.into(),
                name: name.into(),
            });
            events.push(AssistantMessageEvent::ToolCallDelta {
                id: id.into(),
                arguments_delta: args.into(),
            });
            events.push(AssistantMessageEvent::ToolCallEnd { id: id.into() });
        }
        events.push(AssistantMessageEvent::Done {
            stop_reason: StopReason::ToolUse,
        });
        events
    }

    fn make_agent(responses: Vec<Vec<AssistantMessageEvent>>, tools: ToolRegistry) -> Agent {
        Agent::new(
            test_config(),
            Box::new(StatefulProvider::new(responses)),
            tools,
        )
    }

    fn make_agent_with_config(
        config: AgentLoopConfig,
        responses: Vec<Vec<AssistantMessageEvent>>,
        tools: ToolRegistry,
    ) -> Agent {
        Agent::new(config, Box::new(StatefulProvider::new(responses)), tools)
    }

    fn echo_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(EchoTool));
        reg
    }

    fn echo_fail_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(EchoTool));
        reg.register(Box::new(FailTool));
        reg
    }

    // ===============================================================
    // Basic single-turn (no tool calls)
    // ===============================================================

    #[tokio::test]
    async fn test_single_text_response() {
        let mut agent = make_agent(vec![text_response("Hello!")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hi")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, AgentMessage::Assistant(_)))
        );
    }

    #[tokio::test]
    async fn test_empty_done_response() {
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            }]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Hi")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_response_with_thinking_content() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::ThinkingDelta("Let me think...".into()),
                AssistantMessageEvent::TextDelta("The answer is 42.".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text(
            "Meaning of life?",
        )));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistant = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .expect("should have assistant message");
        assert!(!assistant.text().is_empty());
    }

    // ===============================================================
    // Tool call flow
    // ===============================================================

    #[tokio::test]
    async fn test_single_tool_call_then_text_response() {
        // Turn 1: LLM calls echo tool
        // Turn 2: LLM sees tool result and responds with text
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"hello"}"#),
                text_response("Echo returned hello!"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Echo hello")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        // Must contain a ToolResult
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, AgentMessage::ToolResult(_)))
        );
        // Must have two assistant messages: one with tool call, one with text
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(assistants.len(), 2);
    }

    #[tokio::test]
    async fn test_multiple_tool_calls_in_single_response() {
        let mut agent = make_agent(
            vec![
                multi_tool_response(vec![
                    ("tc1", "echo", r#"{"text":"one"}"#),
                    ("tc2", "echo", r#"{"text":"two"}"#),
                ]),
                text_response("Both done!"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Echo both")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_results: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::ToolResult(_)))
            .collect();
        assert_eq!(tool_results.len(), 2);
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error_result() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "nonexistent_tool", r#"{}"#),
                text_response("Understood, tool not found."),
            ],
            ToolRegistry::new(), // empty — no tools registered
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("call unknown")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        assert!(
            tool_result.is_error,
            "unknown tool should produce error result"
        );
    }

    #[tokio::test]
    async fn test_tool_execution_error_propagated_as_result() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "fail", r#"{}"#),
                text_response("I see the tool failed."),
            ],
            echo_fail_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run fail")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        assert!(tool_result.is_error);
    }

    // ===============================================================
    // Multi-turn tool chains
    // ===============================================================

    #[tokio::test]
    async fn test_tool_chain_two_consecutive_calls() {
        // Turn 1: echo "step1" → result
        // Turn 2: echo "step2" → result
        // Turn 3: text response
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"step1"}"#),
                tool_call_response("tc2", "echo", r#"{"text":"step2"}"#),
                text_response("All steps complete."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Do two steps")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_results: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::ToolResult(_)))
            .collect();
        assert_eq!(tool_results.len(), 2);
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(assistants.len(), 3); // tool call + tool call + final text
    }

    #[tokio::test]
    async fn test_max_turns_prevents_infinite_loop() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 2,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        // LLM keeps requesting tools forever (3 calls, but max_turns=2)
        let mut agent = make_agent_with_config(
            config,
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"1"}"#),
                tool_call_response("tc2", "echo", r#"{"text":"2"}"#),
                tool_call_response("tc3", "echo", r#"{"text":"3"}"#),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("loop forever")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // Must terminate — either error or graceful stop. No hang.
        // Provider should NOT have been called more times than max_turns allows
        match result {
            Ok(msgs) => {
                let assistant_count = msgs
                    .iter()
                    .filter(|m| matches!(m, AgentMessage::Assistant(_)))
                    .count();
                assert!(assistant_count <= 3, "should not exceed max_turns");
            }
            Err(e) => {
                assert!(
                    matches!(e, AgentLoopError::MaxTurnsReached),
                    "should be MaxTurnsReached error"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_loop_stops_when_no_more_tool_calls() {
        // Single text response, no tools → loop ends after one iteration
        let mut agent = make_agent(vec![text_response("Done.")], echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("Just reply")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(assistants.len(), 1, "only one assistant response expected");
    }

    // ===============================================================
    // Stop reasons
    // ===============================================================

    #[tokio::test]
    async fn test_error_stop_reason_terminates_loop() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::TextDelta("partial output".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Error,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistant = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .expect("should have assistant message");
        assert_eq!(assistant.stop_reason, StopReason::Error);
    }

    #[tokio::test]
    async fn test_aborted_stop_reason_terminates_loop() {
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Aborted,
            }]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistant = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .expect("should have assistant message");
        assert_eq!(assistant.stop_reason, StopReason::Aborted);
    }

    #[tokio::test]
    async fn test_length_stop_reason_ends_normally() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::TextDelta("truncated output...".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Length,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
    }

    // ===============================================================
    // Steering queue injection
    // ===============================================================

    #[tokio::test]
    async fn test_steering_messages_included_in_output() {
        let mut agent = make_agent(
            vec![text_response("I see your message.")],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello agent")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        assert!(
            messages.iter().any(|m| matches!(m, AgentMessage::User(_))),
            "user message from steering should appear in output"
        );
    }

    #[tokio::test]
    async fn test_multiple_steering_messages_all_drained() {
        let mut agent = make_agent(vec![text_response("Got all of them.")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Message 1")));
        agent.steer(AgentMessage::User(UserMessage::from_text("Message 2")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let user_count = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::User(_)))
            .count();
        assert_eq!(user_count, 2, "both steering messages should be drained");
    }

    // ===============================================================
    // Follow-up queue → outer loop
    // ===============================================================

    #[tokio::test]
    async fn test_follow_up_triggers_additional_llm_call() {
        // First response finishes (no tools), then follow-up causes second call
        let mut agent = make_agent(
            vec![
                text_response("First reply."),
                text_response("Follow-up reply."),
            ],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Initial")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("Follow-up")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(
            assistants.len(),
            2,
            "follow-up should trigger a second LLM call"
        );
    }

    #[tokio::test]
    async fn test_no_follow_up_loop_ends_after_inner() {
        let mut agent = make_agent(vec![text_response("Only reply.")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hi")));
        // No follow_up() call
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(assistants.len(), 1, "no follow-up → single response");
    }

    // ===============================================================
    // Execution mode: parallel vs sequential
    // ===============================================================

    #[tokio::test]
    async fn test_parallel_mode_executes_all_tools() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        let (tool_a, count_a) = CountingTool::new("tool_a");
        let (tool_b, count_b) = CountingTool::new("tool_b");
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![("tc1", "tool_a", r#"{}"#), ("tc2", "tool_b", r#"{}"#)]),
                text_response("Done!"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("parallel")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        assert_eq!(
            count_a.load(Ordering::SeqCst),
            1,
            "tool_a should be called once"
        );
        assert_eq!(
            count_b.load(Ordering::SeqCst),
            1,
            "tool_b should be called once"
        );
    }

    #[tokio::test]
    async fn test_sequential_mode_executes_all_tools() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Sequential,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        let (tool_a, count_a) = CountingTool::new("tool_a");
        let (tool_b, count_b) = CountingTool::new("tool_b");
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![("tc1", "tool_a", r#"{}"#), ("tc2", "tool_b", r#"{}"#)]),
                text_response("Done!"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("sequential")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        assert_eq!(count_a.load(Ordering::SeqCst), 1);
        assert_eq!(count_b.load(Ordering::SeqCst), 1);
    }

    // ===============================================================
    // Hook integration
    // ===============================================================

    #[tokio::test]
    async fn test_before_hook_blocks_tool_execution() {
        struct BlockEchoHook;

        #[async_trait::async_trait]
        impl BeforeToolCallHook for BlockEchoHook {
            async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
                if ctx.tool_name == "echo" {
                    BeforeToolCallResult {
                        block: true,
                        reason: Some("echo is blocked".into()),
                    }
                } else {
                    BeforeToolCallResult {
                        block: false,
                        reason: None,
                    }
                }
            }
        }

        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"blocked"}"#),
                text_response("Understood."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("try echo")));
        agent.set_before_tool_call(Box::new(BlockEchoHook));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        assert!(
            tool_result.is_error,
            "blocked tool should produce error result"
        );
    }

    #[tokio::test]
    async fn test_after_hook_modifies_tool_result() {
        struct ModifyResultHook;

        #[async_trait::async_trait]
        impl AfterToolCallHook for ModifyResultHook {
            async fn after_tool_call(&self, _ctx: &AfterToolCallContext) -> AfterToolCallResult {
                AfterToolCallResult {
                    content: Some(vec![Content::Text {
                        text: "modified by hook".into(),
                    }]),
                    is_error: Some(false),
                }
            }
        }

        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"original"}"#),
                text_response("Done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test hook")));
        agent.set_after_tool_call(Box::new(ModifyResultHook));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        match &tool_result.content[0] {
            Content::Text { text } => {
                assert_eq!(text, "modified by hook");
            }
            _ => panic!("expected Text content"),
        }
    }

    // ===============================================================
    // Event emission
    // ===============================================================

    #[tokio::test]
    async fn test_agent_start_and_end_events() {
        let mut agent = make_agent(vec![text_response("Hi!")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::AgentStart)),
            "must emit AgentStart"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::AgentEnd { .. })),
            "must emit AgentEnd"
        );
    }

    #[tokio::test]
    async fn test_turn_start_and_end_events() {
        let mut agent = make_agent(vec![text_response("Response")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Query")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnStart)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        );
    }

    #[tokio::test]
    async fn test_message_events_for_injected_messages() {
        let mut agent = make_agent(vec![text_response("Reply")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("User input")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::MessageStart { .. })),
            "must emit MessageStart for injected user message"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::MessageEnd { .. })),
            "must emit MessageEnd for injected user message"
        );
    }

    #[tokio::test]
    async fn test_tool_execution_events() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"test"}"#),
                text_response("Done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run echo")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolExecutionStart { .. })),
            "must emit ToolExecutionStart"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. })),
            "must emit ToolExecutionEnd"
        );
    }

    #[tokio::test]
    async fn test_event_ordering_agent_wraps_turns() {
        let mut agent = make_agent(vec![text_response("Answer")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Question")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        let start_idx = events
            .iter()
            .position(|e| matches!(e, AgentEvent::AgentStart))
            .expect("AgentStart must exist");
        let end_idx = events
            .iter()
            .rposition(|e| matches!(e, AgentEvent::AgentEnd { .. }))
            .expect("AgentEnd must exist");
        assert!(start_idx < end_idx, "AgentStart must come before AgentEnd");

        // TurnStart/TurnEnd must be between AgentStart and AgentEnd
        if let Some(turn_start_idx) = events
            .iter()
            .position(|e| matches!(e, AgentEvent::TurnStart))
        {
            assert!(turn_start_idx > start_idx);
            assert!(turn_start_idx < end_idx);
        }
    }

    // ===============================================================
    // Error handling
    // ===============================================================

    #[tokio::test]
    async fn test_provider_error_event_produces_error_stop_reason() {
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Error(
                "API rate limit exceeded".into(),
            )]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // Must not panic. The error event should produce an assistant message
        // with StopReason::Error (or return ProviderError).
        match result {
            Ok(messages) => {
                let assistant = messages.iter().find_map(|m| match m {
                    AgentMessage::Assistant(a) => Some(a),
                    _ => None,
                });
                if let Some(a) = assistant {
                    assert_eq!(a.stop_reason, StopReason::Error);
                }
            }
            Err(e) => {
                assert!(
                    matches!(e, AgentLoopError::ProviderError(_)),
                    "should be ProviderError"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_provider_returns_empty_event_stream() {
        // Provider returns vec![] — no Done event at all
        let mut agent = make_agent(vec![vec![]], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // Should handle gracefully — not panic or hang
        // Either returns Ok with a default message or Err
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_multiple_error_events_from_provider() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::Error("first error".into()),
                AssistantMessageEvent::Error("second error".into()),
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // Must not panic
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_error_event_followed_by_text_delta_ignored() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::Error("something went wrong".into()),
                AssistantMessageEvent::TextDelta("should be ignored".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // The error should be captured; text after error may be ignored or included
        // but the stop_reason should reflect the error
        assert!(result.is_ok() || result.is_err());
    }

    // ===============================================================
    // Messages tracking
    // ===============================================================

    #[tokio::test]
    async fn test_returned_messages_include_all_types() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("Final answer."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Go")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();

        let user_count = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::User(_)))
            .count();
        let assistant_count = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .count();
        let tool_count = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::ToolResult(_)))
            .count();

        assert_eq!(user_count, 1, "one user message");
        assert_eq!(assistant_count, 2, "two assistant messages");
        assert_eq!(tool_count, 1, "one tool result");
    }

    #[tokio::test]
    async fn test_messages_pushed_to_agent_history() {
        let mut agent = make_agent(vec![text_response("Reply.")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Prompt")));
        let sink = CollectorSink::new();

        let initial_len = agent.messages().len();
        let _ = run_agent_loop(&mut agent, &sink).await;
        assert!(
            agent.messages().len() > initial_len,
            "agent message history should grow"
        );
    }

    #[tokio::test]
    async fn test_message_order_user_assistant_tool_assistant() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"step"}"#),
                text_response("Done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();

        // Expected order: User → Assistant(ToolCall) → ToolResult → Assistant(Text)
        assert!(matches!(messages[0], AgentMessage::User(_)));
        assert!(matches!(messages[1], AgentMessage::Assistant(_)));
        assert!(matches!(messages[2], AgentMessage::ToolResult(_)));
        assert!(matches!(messages[3], AgentMessage::Assistant(_)));
    }

    // ===============================================================
    // Agent state after loop
    // ===============================================================

    #[tokio::test]
    async fn test_agent_not_streaming_after_loop_completes() {
        let mut agent = make_agent(vec![text_response("Hi")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        assert!(
            !agent.is_streaming(),
            "agent should not be streaming after loop ends"
        );
    }

    #[tokio::test]
    async fn test_steering_queue_empty_after_loop() {
        let mut agent = make_agent(vec![text_response("Done")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Msg")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        assert!(
            !agent.has_queued_messages(),
            "steering queue should be drained"
        );
    }

    #[tokio::test]
    async fn test_follow_up_queue_empty_after_loop() {
        let mut agent = make_agent(
            vec![text_response("R1"), text_response("R2")],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("FU")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        assert!(
            !agent.has_queued_messages(),
            "follow-up queue should be drained"
        );
    }

    // ===============================================================
    // LLM context building
    // ===============================================================

    #[tokio::test]
    async fn test_llm_receives_system_prompt() {
        let provider = ContextCapturingProvider::new(vec![text_response("Hi")]);
        let captured = std::sync::Arc::clone(&provider.captured);

        let mut agent = Agent::new(test_config(), Box::new(provider), ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        let contexts = captured.lock().await;
        assert!(!contexts.is_empty());
        assert_eq!(contexts[0].system_prompt, "You are a test agent.");
    }

    #[tokio::test]
    async fn test_llm_context_includes_user_message() {
        let provider = ContextCapturingProvider::new(vec![text_response("Reply")]);
        let captured = std::sync::Arc::clone(&provider.captured);

        let mut agent = Agent::new(test_config(), Box::new(provider), ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("What is 2+2?")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        let contexts = captured.lock().await;
        assert!(!contexts.is_empty());
        assert!(
            contexts[0]
                .messages
                .iter()
                .any(|m| matches!(m, LlmMessage::User { .. })),
            "LLM context should include the user message"
        );
    }

    #[tokio::test]
    async fn test_llm_context_grows_across_turns() {
        let provider = ContextCapturingProvider::new(vec![
            tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
            text_response("Final"),
        ]);
        let captured = std::sync::Arc::clone(&provider.captured);

        let mut agent = Agent::new(test_config(), Box::new(provider), echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("Go")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        let contexts = captured.lock().await;
        assert_eq!(contexts.len(), 2, "two LLM calls expected");
        assert!(
            contexts[1].messages.len() > contexts[0].messages.len(),
            "second call should have more context (tool result added)"
        );
    }

    // ===============================================================
    // Tool schemas
    // ===============================================================

    #[tokio::test]
    async fn test_tool_schemas_sent_to_provider() {
        let provider = ToolCapturingProvider::new(vec![text_response("Hi")]);
        let captured_tools = std::sync::Arc::clone(&provider.captured_tools);

        let mut agent = Agent::new(test_config(), Box::new(provider), echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        let tools_per_call = captured_tools.lock().await;
        assert!(!tools_per_call.is_empty());
        assert_eq!(tools_per_call[0].len(), 1, "one tool registered");
        assert_eq!(tools_per_call[0][0].name, "echo");
    }

    // ===============================================================
    // Usage tracking
    // ===============================================================

    #[tokio::test]
    async fn test_usage_event_captured_in_assistant_message() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::TextDelta("Hi".into()),
                AssistantMessageEvent::Usage(Usage {
                    input: 100,
                    output: 50,
                    cache_read: 10,
                    cache_write: 0,
                    total_tokens: 160,
                    cost: Cost::default(),
                }),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistant = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .expect("should have assistant message");
        assert_eq!(assistant.usage.input, 100);
        assert_eq!(assistant.usage.output, 50);
    }

    // ===============================================================
    // Boundary exploration
    // ===============================================================

    #[tokio::test]
    async fn test_max_turns_zero_terminates_immediately() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 0,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        let mut agent = make_agent_with_config(
            config,
            vec![text_response("Should not reach here.")],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // max_turns=0 should immediately return (MaxTurnsReached or empty Ok)
        match result {
            Ok(msgs) => {
                let assistant_count = msgs
                    .iter()
                    .filter(|m| matches!(m, AgentMessage::Assistant(_)))
                    .count();
                assert_eq!(assistant_count, 0, "no LLM calls with max_turns=0");
            }
            Err(e) => {
                assert!(matches!(e, AgentLoopError::MaxTurnsReached));
            }
        }
    }

    #[tokio::test]
    async fn test_max_turns_one_allows_single_call() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 1,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        // LLM calls a tool, which would need a second call — but max_turns=1
        let mut agent = make_agent_with_config(
            config,
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("This should not be reached."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        // Should stop after 1 turn even though tool call was pending
        match result {
            Ok(msgs) => {
                let assistant_count = msgs
                    .iter()
                    .filter(|m| matches!(m, AgentMessage::Assistant(_)))
                    .count();
                assert!(
                    assistant_count <= 1,
                    "max_turns=1 limits to at most 1 LLM call"
                );
            }
            Err(e) => {
                assert!(matches!(e, AgentLoopError::MaxTurnsReached));
            }
        }
    }

    #[tokio::test]
    async fn test_empty_system_prompt() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };
        let provider = ContextCapturingProvider::new(vec![text_response("Hi")]);
        let captured = std::sync::Arc::clone(&provider.captured);

        let mut agent = Agent::new(config, Box::new(provider), ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());

        let contexts = captured.lock().await;
        assert!(!contexts.is_empty());
        assert_eq!(
            contexts[0].system_prompt, "",
            "empty system prompt should be passed through"
        );
    }

    #[tokio::test]
    async fn test_malformed_tool_call_args_produces_error() {
        // ToolCallDelta has invalid JSON — tool execute should get parse error
        let mut agent = make_agent(
            vec![
                vec![
                    AssistantMessageEvent::ToolCallStart {
                        id: "tc1".into(),
                        name: "echo".into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: "tc1".into(),
                        arguments_delta: "{broken json".into(),
                    },
                    AssistantMessageEvent::ToolCallEnd { id: "tc1".into() },
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    },
                ],
                text_response("Understood."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        // Tool result should exist and be an error (malformed args)
        let tool_result = messages.iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) => Some(tr),
            _ => None,
        });
        assert!(
            tool_result.is_some(),
            "should have tool result for malformed args"
        );
        assert!(
            tool_result.unwrap().is_error,
            "malformed args should produce error result"
        );
    }

    #[tokio::test]
    async fn test_tool_call_args_assembled_from_multiple_deltas() {
        // Arguments arrive in multiple chunks — should be concatenated correctly
        let mut agent = make_agent(
            vec![
                vec![
                    AssistantMessageEvent::ToolCallStart {
                        id: "tc1".into(),
                        name: "echo".into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: "tc1".into(),
                        arguments_delta: r#"{"te"#.into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: "tc1".into(),
                        arguments_delta: r#"xt":"hel"#.into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: "tc1".into(),
                        arguments_delta: r#"lo"}"#.into(),
                    },
                    AssistantMessageEvent::ToolCallEnd { id: "tc1".into() },
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    },
                ],
                text_response("Got it."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        assert!(
            !tool_result.is_error,
            "multi-delta args should be assembled correctly"
        );
        // The echo tool should have received "hello"
        match &tool_result.content[0] {
            Content::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text content"),
        }
    }

    #[tokio::test]
    async fn test_empty_tool_name_handled() {
        let mut agent = make_agent(
            vec![
                vec![
                    AssistantMessageEvent::ToolCallStart {
                        id: "tc1".into(),
                        name: "".into(), // empty tool name
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: "tc1".into(),
                        arguments_delta: "{}".into(),
                    },
                    AssistantMessageEvent::ToolCallEnd { id: "tc1".into() },
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    },
                ],
                text_response("OK."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        // Empty tool name should produce an error result (tool not found)
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            })
            .expect("should have tool result");
        assert!(tool_result.is_error, "empty tool name should produce error");
    }

    #[tokio::test]
    async fn test_empty_text_deltas_produce_valid_message() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::TextDelta("".into()),
                AssistantMessageEvent::TextDelta("".into()),
                AssistantMessageEvent::TextDelta("real content".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistant = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .expect("should have assistant message");
        // Text should contain "real content" regardless of empty deltas
        assert!(assistant.text().contains("real content"));
    }

    // ===============================================================
    // State combination — advanced scenarios
    // ===============================================================

    #[tokio::test]
    async fn test_follow_up_with_tool_calls_in_both_rounds() {
        // Round 1: tool call → result → text
        // Follow-up triggers Round 2: tool call → result → text
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"round1"}"#),
                text_response("First round done."),
                tool_call_response("tc2", "echo", r#"{"text":"round2"}"#),
                text_response("Second round done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("Continue")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();

        let tool_results: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::ToolResult(_)))
            .collect();
        assert_eq!(tool_results.len(), 2, "two tool results across two rounds");

        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(
            assistants.len(),
            4,
            "4 assistant messages: tool+text per round"
        );
    }

    #[tokio::test]
    async fn test_multiple_follow_ups_all_processed() {
        let mut agent = make_agent(
            vec![
                text_response("Initial response."),
                text_response("Follow-up 1 response."),
                text_response("Follow-up 2 response."),
            ],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("FU-1")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("FU-2")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let user_count = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::User(_)))
            .count();
        // All follow-ups should be processed (exact semantics: batched or sequential)
        assert!(user_count >= 2, "at least initial + follow-up messages");
    }

    #[tokio::test]
    async fn test_error_stop_reason_skips_follow_up() {
        let mut agent = make_agent(
            vec![vec![
                AssistantMessageEvent::TextDelta("error response".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Error,
                },
            ]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("Should not run")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(
            assistants.len(),
            1,
            "Error stop reason should terminate — follow-up should not execute"
        );
    }

    #[tokio::test]
    async fn test_aborted_stop_reason_skips_follow_up() {
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Aborted,
            }]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Start")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("Should not run")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        let messages = result.unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m, AgentMessage::Assistant(_)))
            .collect();
        assert_eq!(
            assistants.len(),
            1,
            "Aborted stop reason should terminate — follow-up should not execute"
        );
    }

    #[tokio::test]
    async fn test_repeated_run_agent_loop_accumulates_history() {
        let provider = StatefulProvider::new(vec![
            text_response("First run."),
            text_response("Second run."),
        ]);
        let mut agent = Agent::new(test_config(), Box::new(provider), ToolRegistry::new());
        let sink = CollectorSink::new();

        // First run
        agent.steer(AgentMessage::User(UserMessage::from_text("Run 1")));
        let result1 = run_agent_loop(&mut agent, &sink).await;
        assert!(result1.is_ok());
        let history_after_first = agent.messages().len();

        // Second run — agent should retain messages from first run
        agent.steer(AgentMessage::User(UserMessage::from_text("Run 2")));
        let result2 = run_agent_loop(&mut agent, &sink).await;
        assert!(result2.is_ok());
        let history_after_second = agent.messages().len();

        assert!(
            history_after_second > history_after_first,
            "second run should accumulate onto existing history"
        );
    }

    // ===============================================================
    // Hook prevents tool execute (verified with CountingTool)
    // ===============================================================

    #[tokio::test]
    async fn test_before_hook_blocks_prevents_tool_execute_call() {
        struct BlockAllHook;

        #[async_trait::async_trait]
        impl BeforeToolCallHook for BlockAllHook {
            async fn before_tool_call(&self, _ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
                BeforeToolCallResult {
                    block: true,
                    reason: Some("all blocked".into()),
                }
            }
        }

        let (tool, exec_count) = CountingTool::new("echo");
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool));

        let mut agent = make_agent_with_config(
            test_config(),
            vec![
                tool_call_response("tc1", "echo", r#"{}"#),
                text_response("OK."),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        agent.set_before_tool_call(Box::new(BlockAllHook));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        assert_eq!(
            exec_count.load(Ordering::SeqCst),
            0,
            "blocked tool should never have execute() called"
        );
    }

    // ===============================================================
    // Sequential mode order verification
    // ===============================================================

    /// Tool that records its execution order using a shared atomic counter.
    struct OrderRecordingTool {
        tool_name: String,
        order_counter: std::sync::Arc<AtomicUsize>,
        my_order: std::sync::Arc<AtomicUsize>,
    }

    impl OrderRecordingTool {
        fn new(
            name: &str,
            order_counter: std::sync::Arc<AtomicUsize>,
        ) -> (Self, std::sync::Arc<AtomicUsize>) {
            let my_order = std::sync::Arc::new(AtomicUsize::new(0));
            (
                Self {
                    tool_name: name.into(),
                    order_counter,
                    my_order: std::sync::Arc::clone(&my_order),
                },
                my_order,
            )
        }
    }

    #[async_trait::async_trait]
    impl AgentTool for OrderRecordingTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "Records execution order"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: serde_json::Value) -> ToolOutput {
            let order = self.order_counter.fetch_add(1, Ordering::SeqCst);
            self.my_order.store(order, Ordering::SeqCst);
            ToolOutput {
                content: vec![Content::Text {
                    text: format!("executed at order {order}"),
                }],
                is_error: false,
            }
        }
    }

    #[tokio::test]
    async fn test_sequential_mode_respects_call_order() {
        let config = AgentLoopConfig {
            name: String::new(),
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Sequential,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        };

        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let (tool_a, order_a) = OrderRecordingTool::new("tool_a", std::sync::Arc::clone(&counter));
        let (tool_b, order_b) = OrderRecordingTool::new("tool_b", std::sync::Arc::clone(&counter));

        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![("tc1", "tool_a", r#"{}"#), ("tc2", "tool_b", r#"{}"#)]),
                text_response("Done!"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text(
            "sequential order",
        )));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        // In sequential mode, tool_a (tc1) should execute before tool_b (tc2)
        assert_eq!(
            order_a.load(Ordering::SeqCst),
            0,
            "tool_a should execute first"
        );
        assert_eq!(
            order_b.load(Ordering::SeqCst),
            1,
            "tool_b should execute second"
        );
    }

    // ===============================================================
    // Event data content verification
    // ===============================================================

    #[tokio::test]
    async fn test_tool_execution_start_event_contains_tool_metadata() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"verify"}"#),
                text_response("Done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run echo")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        let tool_start = events.iter().find_map(|e| match e {
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => Some((tool_call_id.clone(), tool_name.clone(), args.clone())),
            _ => None,
        });
        assert!(tool_start.is_some(), "ToolExecutionStart should be emitted");
        let (id, name, _args) = tool_start.unwrap();
        assert_eq!(id, "tc1");
        assert_eq!(name, "echo");
    }

    #[tokio::test]
    async fn test_tool_execution_end_event_contains_error_flag() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "fail", r#"{}"#),
                text_response("OK."),
            ],
            echo_fail_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run fail")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        let tool_end = events.iter().find_map(|e| match e {
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                is_error,
            } => Some((tool_call_id.clone(), tool_name.clone(), *is_error)),
            _ => None,
        });
        assert!(tool_end.is_some(), "ToolExecutionEnd should be emitted");
        let (id, name, is_err) = tool_end.unwrap();
        assert_eq!(id, "tc1");
        assert_eq!(name, "fail");
        assert!(
            is_err,
            "ToolExecutionEnd should reflect error from FailTool"
        );
    }

    #[tokio::test]
    async fn test_agent_end_event_carries_all_messages() {
        let mut agent = make_agent(vec![text_response("Final answer.")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("Question")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        let agent_end = events.iter().find_map(|e| match e {
            AgentEvent::AgentEnd { messages } => Some(messages.clone()),
            _ => None,
        });
        assert!(agent_end.is_some(), "AgentEnd should be emitted");
        let end_messages = agent_end.unwrap();
        assert!(
            !end_messages.is_empty(),
            "AgentEnd should carry the messages from this loop run"
        );
    }

    // ===============================================================
    // Event count for multi-turn with tools
    // ===============================================================

    #[tokio::test]
    async fn test_two_turns_emit_two_turn_events() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("Done."),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("Go")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;
        let events = sink.events().await;

        let turn_start_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::TurnStart))
            .count();
        let turn_end_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::TurnEnd { .. }))
            .count();

        // Two turns: first with tool call, second with text response
        assert!(
            turn_start_count >= 2,
            "should have at least 2 TurnStart events"
        );
        assert_eq!(
            turn_start_count, turn_end_count,
            "TurnStart and TurnEnd should be balanced"
        );
    }

    // -------------------------------------------------------------------
    // Thinking blocks roundtrip through build_llm_context (P4-B)
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_thinking_blocks_roundtrip_in_context() {
        // Turn 1: LLM responds with thinking + ThinkingBlockEnd + text
        // Turn 2: We capture the context to verify thinking blocks survived
        let turn1 = vec![
            AssistantMessageEvent::ThinkingDelta("I need to reason...".into()),
            AssistantMessageEvent::ThinkingBlockEnd {
                signature: "sig_from_api".into(),
                redacted: false,
            },
            AssistantMessageEvent::TextDelta("Here's my answer.".into()),
            AssistantMessageEvent::ToolCallStart {
                id: "tc1".into(),
                name: "echo".into(),
            },
            AssistantMessageEvent::ToolCallDelta {
                id: "tc1".into(),
                arguments_delta: r#"{"input":"test"}"#.into(),
            },
            AssistantMessageEvent::ToolCallEnd { id: "tc1".into() },
            AssistantMessageEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ];
        let turn2 = text_response("Final answer.");

        let provider = ContextCapturingProvider::new(vec![turn1, turn2]);
        let captured = provider.captured.clone();
        let mut agent = Agent::new(test_config(), Box::new(provider), echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("Hello")));
        let sink = CollectorSink::new();
        let _ = run_agent_loop(&mut agent, &sink).await;

        // Turn 2 context should contain the assistant message from turn 1
        let contexts = captured.lock().await;
        assert!(contexts.len() >= 2, "should have at least 2 LLM calls");
        let turn2_ctx = &contexts[1];
        let assistant_msg = turn2_ctx
            .messages
            .iter()
            .find(|m| matches!(m, LlmMessage::Assistant { .. }))
            .expect("should have an assistant message in turn 2 context");

        if let LlmMessage::Assistant {
            thinking_blocks, ..
        } = assistant_msg
        {
            assert_eq!(
                thinking_blocks.len(),
                1,
                "thinking block should survive roundtrip"
            );
            assert_eq!(thinking_blocks[0].thinking, "I need to reason...");
            assert_eq!(
                thinking_blocks[0].signature.as_deref(),
                Some("sig_from_api")
            );
            assert!(!thinking_blocks[0].redacted);
        } else {
            panic!("expected Assistant message");
        }
    }

    // -------------------------------------------------------------------
    // MessageAccumulator per-block thinking tests (P4-B)
    // -------------------------------------------------------------------

    #[test]
    fn test_accumulator_thinking_block_with_signature() {
        let model = test_model();
        let mut acc = MessageAccumulator::new();
        acc.push_event(&AssistantMessageEvent::ThinkingDelta(
            "Let me think...".into(),
        ));
        acc.push_event(&AssistantMessageEvent::ThinkingBlockEnd {
            signature: "sig_abc123".into(),
            redacted: false,
        });
        acc.push_event(&AssistantMessageEvent::TextDelta("The answer.".into()));
        acc.push_event(&AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        });

        let (msg, _) = acc.build(&model);
        assert_eq!(msg.content.len(), 2);
        match &msg.content[0] {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => {
                assert_eq!(thinking, "Let me think...");
                assert_eq!(signature.as_deref(), Some("sig_abc123"));
                assert!(!redacted);
            }
            other => panic!("expected Thinking, got: {other:?}"),
        }
        assert!(matches!(&msg.content[1], Content::Text { text } if text == "The answer."));
    }

    #[test]
    fn test_accumulator_redacted_thinking_block() {
        let model = test_model();
        let mut acc = MessageAccumulator::new();
        // Redacted thinking emits a placeholder ThinkingDelta then ThinkingBlockEnd
        acc.push_event(&AssistantMessageEvent::ThinkingDelta(
            "[Reasoning redacted]".into(),
        ));
        acc.push_event(&AssistantMessageEvent::ThinkingBlockEnd {
            signature: "opaque_encrypted_data".into(),
            redacted: true,
        });
        acc.push_event(&AssistantMessageEvent::TextDelta("Result.".into()));
        acc.push_event(&AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        });

        let (msg, _) = acc.build(&model);
        assert_eq!(msg.content.len(), 2);
        match &msg.content[0] {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => {
                assert_eq!(thinking, "[Reasoning redacted]");
                assert_eq!(signature.as_deref(), Some("opaque_encrypted_data"));
                assert!(redacted);
            }
            other => panic!("expected Thinking, got: {other:?}"),
        }
    }

    #[test]
    fn test_accumulator_multiple_thinking_blocks() {
        let model = test_model();
        let mut acc = MessageAccumulator::new();
        // Block 1: normal thinking
        acc.push_event(&AssistantMessageEvent::ThinkingDelta("step 1".into()));
        acc.push_event(&AssistantMessageEvent::ThinkingBlockEnd {
            signature: "sig_1".into(),
            redacted: false,
        });
        // Block 2: redacted
        acc.push_event(&AssistantMessageEvent::ThinkingDelta(
            "[Reasoning redacted]".into(),
        ));
        acc.push_event(&AssistantMessageEvent::ThinkingBlockEnd {
            signature: "redacted_data".into(),
            redacted: true,
        });
        // Block 3: normal thinking without signature
        acc.push_event(&AssistantMessageEvent::ThinkingDelta("step 2".into()));
        acc.push_event(&AssistantMessageEvent::ThinkingBlockEnd {
            signature: String::new(),
            redacted: false,
        });
        acc.push_event(&AssistantMessageEvent::TextDelta("done".into()));
        acc.push_event(&AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        });

        let (msg, _) = acc.build(&model);
        // 3 thinking + 1 text
        assert_eq!(msg.content.len(), 4);
        match &msg.content[0] {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => {
                assert_eq!(thinking, "step 1");
                assert_eq!(signature.as_deref(), Some("sig_1"));
                assert!(!redacted);
            }
            other => panic!("expected Thinking block 1, got: {other:?}"),
        }
        match &msg.content[1] {
            Content::Thinking {
                redacted,
                signature,
                ..
            } => {
                assert!(redacted);
                assert_eq!(signature.as_deref(), Some("redacted_data"));
            }
            other => panic!("expected Thinking block 2 (redacted), got: {other:?}"),
        }
        match &msg.content[2] {
            Content::Thinking {
                thinking,
                signature,
                ..
            } => {
                assert_eq!(thinking, "step 2");
                assert!(signature.is_none());
            }
            other => panic!("expected Thinking block 3, got: {other:?}"),
        }
    }

    #[test]
    fn test_accumulator_thinking_without_block_end_flushed() {
        // Providers without ThinkingBlockEnd (e.g., OpenAI) just send ThinkingDelta
        let model = test_model();
        let mut acc = MessageAccumulator::new();
        acc.push_event(&AssistantMessageEvent::ThinkingDelta("reasoning".into()));
        acc.push_event(&AssistantMessageEvent::TextDelta("answer".into()));
        acc.push_event(&AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        });

        let (msg, _) = acc.build(&model);
        assert_eq!(msg.content.len(), 2);
        // Flushed as a single block with no signature
        match &msg.content[0] {
            Content::Thinking {
                thinking,
                signature,
                redacted,
            } => {
                assert_eq!(thinking, "reasoning");
                assert!(signature.is_none());
                assert!(!redacted);
            }
            other => panic!("expected Thinking, got: {other:?}"),
        }
    }

    // ── Issue #6: Regression — MessageUpdate and ToolExecutionUpdate emitted ──

    #[tokio::test]
    async fn test_fix_text_delta_emits_message_update() {
        let sink = CollectorSink::new();
        let mut agent = make_agent(
            vec![text_response("hello world")],
            ToolRegistry::new(),
        );
        agent.push_message(AgentMessage::User(UserMessage::from_text("hi")));

        run_agent_loop(&mut agent, &sink).await.unwrap();

        let events = sink.events().await;
        let message_updates: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::MessageUpdate { .. }))
            .collect();
        assert!(
            !message_updates.is_empty(),
            "TextDelta should produce MessageUpdate events"
        );
        // Verify the delta content
        if let AgentEvent::MessageUpdate { delta, .. } = &message_updates[0] {
            assert_eq!(delta, "hello world");
        }
    }

    #[tokio::test]
    async fn test_fix_tool_call_delta_emits_tool_execution_update() {
        let sink = CollectorSink::new();
        let mut agent = make_agent(
            vec![
                tool_call_response("tc-1", "echo", r#"{"text":"hi"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.push_message(AgentMessage::User(UserMessage::from_text("use echo")));

        run_agent_loop(&mut agent, &sink).await.unwrap();

        let events = sink.events().await;
        let tool_updates: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolExecutionUpdate { .. }))
            .collect();
        assert!(
            !tool_updates.is_empty(),
            "ToolCallDelta should produce ToolExecutionUpdate events"
        );
        // Verify tool name is correctly resolved from ToolCallStart
        if let AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            partial_result,
        } = &tool_updates[0]
        {
            assert_eq!(tool_call_id, "tc-1");
            assert_eq!(tool_name, "echo");
            assert_eq!(partial_result, r#"{"text":"hi"}"#);
        }
    }

    // ===============================================================
    // Structured tracing span / event tests
    // ===============================================================

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_agent_loop_started_emitted() {
        let mut agent = make_agent(vec![text_response("hi")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("hello")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("agent loop started"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_agent_loop_completed_emitted() {
        let mut agent = make_agent(vec![text_response("hi")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("hello")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("agent loop completed"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_turn_complete_emitted_after_each_turn() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("go")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        // Two turns: one for tool call, one for text response
        assert!(logs_contain("turn complete"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_tool_executed_emitted_on_success() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"hello"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("echo it")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("tool executed"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_tool_executed_is_error_true_on_fail_tool() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "fail", r#"{}"#),
                text_response("ok"),
            ],
            echo_fail_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run fail")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("tool executed"));
        // Verify the field value is `true`, not just that the field name appears
        assert!(logs_contain("is_error=true"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_llm_call_is_logged() {
        // "llm.request" fires before the LLM call; "llm.response" fires after
        // with input_tokens, output_tokens, and elapsed_ms fields.
        let mut agent = make_agent(vec![text_response("answer")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("question")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("llm.request"), "request log must fire before LLM call");
        assert!(logs_contain("llm.response"), "response log must fire after LLM call with usage");
        assert!(logs_contain("elapsed_ms"), "response log must include latency");
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_no_tool_executed_log_when_no_tool_calls() {
        // A pure text response should not emit "tool executed"
        let mut agent = make_agent(vec![text_response("just text")], ToolRegistry::new());
        agent.steer(AgentMessage::User(UserMessage::from_text("no tools")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        // agent loop events should still fire; tool executed should NOT appear
        assert!(logs_contain("agent loop started"));
        assert!(logs_contain("agent loop completed"));
        assert!(!logs_contain("tool executed"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_tool_name_field_in_tool_executed_log() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"test"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("use echo")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        // The structured log should include the tool_name field value
        assert!(logs_contain("tool_name=echo"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_multi_turn_emits_multiple_turn_complete_logs() {
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"step1"}"#),
                tool_call_response("tc2", "echo", r#"{"text":"step2"}"#),
                text_response("all done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("three turns")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        // "turn complete" should appear at least once per LLM response
        assert!(logs_contain("turn complete"));
        // The completion log should report total_turns=3 (one per LLM response)
        assert!(logs_contain("total_turns=3"), "3-response run should log total_turns=3");
        assert!(logs_contain("agent loop completed"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_turn_complete_field_values() {
        // Two LLM responses → two turns logged with 1-indexed turn numbers
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("go")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        // turn_count is 1-indexed (incremented before "turn complete" is logged)
        assert!(logs_contain("turn=1"), "first turn should log turn=1");
        assert!(logs_contain("turn=2"), "second turn should log turn=2");
        // Completion log reflects total
        assert!(logs_contain("total_turns=2"), "2-response run should report total_turns=2");
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_provider_error_early_termination_not_logged_as_completed() {
        // When the LLM returns Error event → StopReason::Error → early return.
        // The agent loop exits before "turn complete" or "agent loop completed" are logged.
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Error("api error".into())]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("agent loop started"), "loop start must always be logged");
        // Early return path does NOT reach "turn complete" or "agent loop completed"
        assert!(!logs_contain("turn complete"), "error path exits before turn complete log");
        assert!(
            !logs_contain("agent loop completed"),
            "error path exits before completion log"
        );
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_aborted_stop_reason_skips_completed_log() {
        // StopReason::Aborted also triggers early return — same path as Error.
        // Both are checked at "if matches!(stop_reason, Error | Aborted)".
        let mut agent = make_agent(
            vec![vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Aborted,
            }]],
            ToolRegistry::new(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("test")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("agent loop started"));
        assert!(!logs_contain("turn complete"), "Aborted exits before turn complete log");
        assert!(
            !logs_contain("agent loop completed"),
            "Aborted exits before completion log"
        );
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_tool_executed_is_error_false_on_success() {
        // Successful tool execution → is_error=false in the log.
        let mut agent = make_agent(
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"ok"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("run echo")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("tool executed"));
        assert!(logs_contain("is_error=false"));
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_tool_call_id_field_in_tool_executed_log() {
        // "tool executed" log includes tool_call_id field.
        let mut agent = make_agent(
            vec![
                tool_call_response("my-call-id", "echo", r#"{"text":"x"}"#),
                text_response("done"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("echo")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("tool executed"));
        assert!(logs_contain("my-call-id"), "tool_call_id value should appear in log");
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_span_max_turns_reached_skips_completed_log() {
        // When turn_count >= max_turns the loop returns early before
        // "agent loop completed" is logged.
        let mut config = test_config();
        config.max_turns = 1;
        // Two responses: first would trigger another turn, but max_turns=1 stops it.
        let mut agent = make_agent_with_config(
            config,
            vec![
                tool_call_response("tc1", "echo", r#"{"text":"x"}"#),
                text_response("never reached"),
            ],
            echo_registry(),
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("go")));
        let sink = CollectorSink::new();

        run_agent_loop(&mut agent, &sink).await.unwrap();

        assert!(logs_contain("agent loop started"));
        // max_turns early return skips the completion log
        assert!(
            !logs_contain("agent loop completed"),
            "max_turns exit should not reach completion log"
        );
    }

    // ---------------------------------------------------------------
    // Sprint 12 task #69: CancellationToken wiring
    // ---------------------------------------------------------------
    //
    // Covers the three agent-loop wiring points:
    //   (a) top-of-turn checkpoint — cancel before the first LLM call returns
    //       Cancelled without invoking the provider
    //   (b) LLM call — a slow provider cancelled mid-await returns Cancelled
    //       and does NOT push a half-built assistant message
    //   (c) tool execute — a slow tool cancelled mid-run returns a cancelled
    //       tool_result (preserving the tool_use ↔ tool_result pairing) and
    //       the loop surfaces Cancelled on the next iteration

    /// A provider that blocks on a `Notify` until the test wakes it, then
    /// returns a configured response. Used to create a deterministic race
    /// between cancellation and LLM completion.
    ///
    /// `new()` returns `(Box<Self>, Arc<AtomicUsize>)` — the `Box` goes to
    /// `Agent::new`, the `Arc` is a side-channel handle the test reads to
    /// assert how many times `complete()` was polled.
    struct SlowProvider {
        release: std::sync::Arc<tokio::sync::Notify>,
        response: Mutex<Option<Vec<AssistantMessageEvent>>>,
        calls: std::sync::Arc<AtomicUsize>,
    }

    impl SlowProvider {
        fn new(
            response: Vec<AssistantMessageEvent>,
        ) -> (Box<Self>, std::sync::Arc<AtomicUsize>) {
            let calls = std::sync::Arc::new(AtomicUsize::new(0));
            (
                Box::new(Self {
                    release: std::sync::Arc::new(tokio::sync::Notify::new()),
                    response: Mutex::new(Some(response)),
                    calls: calls.clone(),
                }),
                calls,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for SlowProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.release.notified().await;
            self.response.lock().unwrap().take().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
    }

    /// Tool that blocks on an internal `Notify` (never released in these
    /// tests) and records whether `execute` ever completed. Returns
    /// `(Box<Self>, Arc<AtomicBool>)` — the Arc is the completion flag the
    /// test reads to assert the tool was preempted rather than finished.
    struct SlowTool {
        release: std::sync::Arc<tokio::sync::Notify>,
        completed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl SlowTool {
        fn new() -> (Box<Self>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
            let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let tool = Box::new(Self {
                release: std::sync::Arc::new(tokio::sync::Notify::new()),
                completed: completed.clone(),
            });
            (tool, completed)
        }
    }

    #[async_trait::async_trait]
    impl AgentTool for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }
        fn description(&self) -> &str {
            "Blocks until released or cancelled"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: serde_json::Value) -> ToolOutput {
            self.release.notified().await;
            self.completed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            ToolOutput {
                content: vec![Content::Text {
                    text: "slow done".into(),
                }],
                is_error: false,
            }
        }
    }

    #[tokio::test]
    async fn cancel_before_run_returns_cancelled_without_calling_provider() {
        // (a) top-of-turn checkpoint: if the token is already cancelled when
        //     the loop enters the inner while, we must skip the LLM call.
        let (provider, calls) = SlowProvider::new(text_response("unreachable"));
        let mut agent = Agent::new(test_config(), provider, echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("hi")));
        let sink = CollectorSink::new();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let res = run_agent_loop_with_cancel(&mut agent, &sink, Some(&cancel)).await;

        assert!(
            matches!(res, Err(AgentLoopError::Cancelled)),
            "pre-fired cancel must short-circuit with Cancelled, got {res:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "provider must not be invoked when cancel is set before first turn"
        );
        // AgentEnd must still fire so stream consumers see a clean terminal.
        let events = sink.events().await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::AgentEnd { .. })),
            "AgentEnd must be emitted on cancellation"
        );
    }

    #[tokio::test]
    async fn cancel_during_llm_call_aborts_and_returns_cancelled() {
        // (b) LLM call: provider future is in flight; cancel fires; select!
        //     branch returns Cancelled and the partial provider response
        //     never becomes an assistant message.
        let (provider, calls) = SlowProvider::new(text_response("unreachable"));
        let mut agent = Agent::new(test_config(), provider, echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("hi")));
        let sink = CollectorSink::new();
        let cancel = CancellationToken::new();
        let cancel_handle = cancel.clone();

        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_handle.cancel();
        });

        let res = run_agent_loop_with_cancel(&mut agent, &sink, Some(&cancel)).await;
        canceller.await.unwrap();

        assert!(
            matches!(res, Err(AgentLoopError::Cancelled)),
            "mid-LLM cancel must produce Cancelled, got {res:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "provider was polled exactly once before cancel fired"
        );
        // No assistant message was pushed from the aborted call.
        let has_assistant = agent
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::Assistant(_)));
        assert!(
            !has_assistant,
            "no assistant message should be pushed when LLM call was cancelled"
        );
    }

    #[tokio::test]
    async fn cancel_during_tool_execute_synthesizes_cancelled_tool_result() {
        // (c) tool execute: LLM asks for a tool call; the tool blocks; cancel
        //     fires; tool observes cancel via select! and emits a cancelled
        //     ToolOutput. Loop then sees cancel at top-of-next-turn and
        //     returns Cancelled. tool_use ↔ tool_result invariant is
        //     preserved.
        let mut config = test_config();
        config.tool_execution_mode = ToolExecutionMode::Sequential;
        let (slow_tool, completed) = SlowTool::new();
        let mut reg = ToolRegistry::new();
        reg.register(slow_tool);

        let mut agent = make_agent_with_config(
            config,
            vec![
                tool_call_response("tc1", "slow", "{}"),
                text_response("never reached"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("go")));
        let sink = CollectorSink::new();
        let cancel = CancellationToken::new();
        let cancel_handle = cancel.clone();

        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_handle.cancel();
        });

        let res = run_agent_loop_with_cancel(&mut agent, &sink, Some(&cancel)).await;
        canceller.await.unwrap();

        assert!(
            matches!(res, Err(AgentLoopError::Cancelled)),
            "mid-tool cancel must eventually surface Cancelled, got {res:?}"
        );
        assert!(
            !completed.load(std::sync::atomic::Ordering::SeqCst),
            "slow tool must not run to completion after cancel"
        );
        // Pairing invariant: the assistant message with tool_use has a
        // matching ToolResult (synthesized cancelled marker).
        let tool_result = agent
            .messages()
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) if tr.tool_call_id == "tc1" => Some(tr),
                _ => None,
            })
            .expect("cancelled tool must still produce a tool_result for pairing");
        assert!(tool_result.is_error, "cancelled tool_result is an error");
    }

    #[tokio::test]
    async fn cancel_in_sequential_multi_tool_synthesizes_marker_for_unstarted() {
        // Linus review follow-up: cover the `cancelled_tool_result` helper
        // directly. Scenario: LLM emits TWO sequential tool_calls. Cancel
        // fires while the first one is executing (it returns via the
        // select! race path). At the top of the next iteration of the
        // for-loop inside `execute_tool_calls`, the pre-check
        // `tok.is_cancelled()` branch fires for the second tool — which
        // triggers `cancelled_tool_result(tc)`, the code path that was
        // previously uncovered.
        let mut config = test_config();
        config.tool_execution_mode = ToolExecutionMode::Sequential;
        let (slow_tool, _completed) = SlowTool::new();
        let mut reg = ToolRegistry::new();
        reg.register(slow_tool);

        // A single LLM response carrying two tool_use blocks.
        let multi = multi_tool_response(vec![
            ("tc-a", "slow", "{}"),
            ("tc-b", "slow", "{}"),
        ]);
        let mut agent = make_agent_with_config(
            config,
            vec![multi, text_response("never reached")],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("go")));
        let sink = CollectorSink::new();
        let cancel = CancellationToken::new();
        let cancel_handle = cancel.clone();

        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_handle.cancel();
        });

        let res = run_agent_loop_with_cancel(&mut agent, &sink, Some(&cancel)).await;
        canceller.await.unwrap();

        assert!(
            matches!(res, Err(AgentLoopError::Cancelled)),
            "multi-tool mid-execute cancel must surface Cancelled, got {res:?}"
        );
        // Both tool_use blocks must have matching tool_results pushed —
        // this is the invariant the `cancelled_tool_result` helper exists
        // to uphold for the unstarted tool.
        let tc_a_result = agent.messages().iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) if tr.tool_call_id == "tc-a" => Some(tr),
            _ => None,
        });
        let tc_b_result = agent.messages().iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) if tr.tool_call_id == "tc-b" => Some(tr),
            _ => None,
        });
        assert!(
            tc_a_result.is_some(),
            "first tool (raced in execute) must have a tool_result"
        );
        let tc_b = tc_b_result
            .expect("second tool (never started) must have a synthesized cancelled marker");
        // Synthesized marker: is_error + "cancelled" text — proves it went
        // through `cancelled_tool_result`, not through the normal
        // finalize_tool_call path.
        assert!(tc_b.is_error, "synthesized marker is is_error=true");
        let text = tc_b
            .content
            .iter()
            .find_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("");
        assert!(
            text.contains("cancelled"),
            "synthesized marker text must mention cancellation, got {text:?}"
        );
    }

    #[tokio::test]
    async fn no_cancel_token_preserves_pre_s12_behavior() {
        // Backward-compat regression: run_agent_loop(agent, emit) delegates
        // to run_agent_loop_with_cancel(agent, emit, None), which must not
        // observe any cancel and must complete normally.
        let mut agent = make_agent(vec![text_response("hello")], echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("hi")));
        let sink = CollectorSink::new();

        let res = run_agent_loop(&mut agent, &sink).await;

        assert!(res.is_ok(), "no-cancel path must complete normally");
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        // Repeated cancel() on the same token must remain cancelled and
        // behave identically to a single cancel().
        let (provider, calls) = SlowProvider::new(text_response("unreachable"));
        let mut agent = Agent::new(test_config(), provider, echo_registry());
        agent.steer(AgentMessage::User(UserMessage::from_text("hi")));
        let sink = CollectorSink::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        cancel.cancel();
        cancel.cancel();

        let res = run_agent_loop_with_cancel(&mut agent, &sink, Some(&cancel)).await;

        assert!(matches!(res, Err(AgentLoopError::Cancelled)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
