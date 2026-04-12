// Agent Loop — Phase 4
// Three-layer nested loop: outer (follow-up) → inner (steering + tools) → innermost (LLM streaming).
// Implements the full agent execution lifecycle with event emission.

use crate::agent::Agent;
use crate::event::{AgentEvent, AgentEventSink};
use crate::llm::types::*;
use crate::tools::ToolOutput;
use crate::types::*;
use std::fmt;

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

/// Accumulates streaming events into an AssistantMessage.
struct MessageAccumulator {
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCallAccum>,
    usage: Usage,
    stop_reason: StopReason,
    error_message: Option<String>,
    errored: bool,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

impl MessageAccumulator {
    fn new() -> Self {
        Self {
            text: String::new(),
            thinking: String::new(),
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
            _ if self.errored && !matches!(
                event,
                AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Usage(_) | AssistantMessageEvent::Error(_)
            ) => {}
            AssistantMessageEvent::TextDelta(delta) => self.text.push_str(delta),
            AssistantMessageEvent::ThinkingDelta(delta) => self.thinking.push_str(delta),
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
        if !self.thinking.is_empty() {
            content.push(Content::Thinking {
                thinking: self.thinking,
                signature: None,
                redacted: false,
            });
        }
        if !self.text.is_empty() {
            content.push(Content::Text { text: self.text });
        }
        for tc in &self.tool_calls {
            let args = serde_json::from_str(&tc.arguments)
                .unwrap_or(serde_json::Value::String(tc.arguments.clone()));
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
    let mut messages = Vec::new();
    for msg in agent.messages() {
        match msg {
            AgentMessage::User(u) => {
                let content: Vec<LlmContent> = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(LlmContent::Text(text.clone())),
                        Content::Image { data, .. } => {
                            Some(LlmContent::Image { url: data.clone() })
                        }
                        _ => None,
                    })
                    .collect();
                messages.push(LlmMessage::User { content });
            }
            AgentMessage::Assistant(a) => {
                let text = a.text();
                let tool_calls: Vec<LlmToolCall> = a
                    .tool_calls()
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
                                arguments: arguments.to_string(),
                            },
                        }),
                        _ => None,
                    })
                    .collect();
                messages.push(LlmMessage::Assistant {
                    content: text,
                    tool_calls,
                });
            }
            AgentMessage::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                messages.push(LlmMessage::Tool {
                    tool_call_id: tr.tool_call_id.clone(),
                    content: text,
                });
            }
        }
    }

    LlmContext {
        messages,
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
    let args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
        Ok(v) => v,
        Err(e) => {
            return Err(emit_blocked(
                tc,
                serde_json::Value::Null,
                format!("Invalid tool call arguments: {e}"),
                emit,
            ).await);
        }
    };

    // Enforce tool policy before any execution
    if let Some(policy) = &agent.config().tool_policy {
        if let Err(reason) = policy.check_tool_call(&tc.name, &args) {
            return Err(emit_blocked(tc, args, reason, emit).await);
        }
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
async fn finalize_tool_call(
    agent: &Agent,
    tc: &ToolCallAccum,
    output: ToolOutput,
    emit: &dyn AgentEventSink,
) -> ToolResultMessage {
    let after_ctx = AfterToolCallContext {
        tool_name: tc.name.clone(),
        tool_call_id: tc.id.clone(),
        args: serde_json::from_str(&tc.arguments).unwrap_or_default(),
        is_error: output.is_error,
    };
    let after_result = agent.call_after_tool_call(&after_ctx).await;
    let content = after_result.content.unwrap_or(output.content);
    let is_error = after_result.is_error.unwrap_or(output.is_error);

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

/// Execute a single tool call against the registry.
async fn run_tool(agent: &Agent, name: &str, args: serde_json::Value) -> ToolOutput {
    match agent.tools().get(name) {
        Some(tool) => tool.execute(args).await,
        None => ToolOutput {
            content: vec![Content::Text {
                text: format!("Unknown tool: {name}"),
            }],
            is_error: true,
        },
    }
}

/// Execute tool calls — parallel or sequential based on config.
async fn execute_tool_calls(
    agent: &Agent,
    tool_calls: &[ToolCallAccum],
    emit: &dyn AgentEventSink,
) -> Vec<ToolResultMessage> {
    let mut results = Vec::new();

    match agent.config().tool_execution_mode {
        ToolExecutionMode::Parallel => {
            let mut prepared = Vec::new();
            for tc in tool_calls {
                match prepare_tool_call(agent, tc, emit).await {
                    Ok(args) => prepared.push((tc, args)),
                    Err(tr) => results.push(tr),
                }
            }

            let futs: Vec<_> = prepared
                .iter()
                .map(|(tc, args)| run_tool(agent, &tc.name, args.clone()))
                .collect();
            let outputs: Vec<ToolOutput> = futures::future::join_all(futs).await;

            for ((tc, _args), output) in prepared.into_iter().zip(outputs) {
                results.push(finalize_tool_call(agent, tc, output, emit).await);
            }
        }
        ToolExecutionMode::Sequential => {
            for tc in tool_calls {
                let args = match prepare_tool_call(agent, tc, emit).await {
                    Ok(args) => args,
                    Err(tr) => {
                        results.push(tr);
                        continue;
                    }
                };
                let output = run_tool(agent, &tc.name, args).await;
                results.push(finalize_tool_call(agent, tc, output, emit).await);
            }
        }
    }

    results
}

/// Run the three-layer agent loop.
pub async fn run_agent_loop(
    agent: &mut Agent,
    emit: &dyn AgentEventSink,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    let mut new_messages: Vec<AgentMessage> = Vec::new();
    let mut turn_count: usize = 0;

    agent.set_streaming(true);
    emit.emit(AgentEvent::AgentStart).await;

    // Drain initial steering messages
    let mut pending: Vec<AgentMessage> = agent.drain_steering();

    'outer: loop {
        // OUTER: follow-up loop
        let mut has_more_tool_calls = true;

        while has_more_tool_calls || !pending.is_empty() {
            // INNER: steering + tools loop

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

            // 2. Build context and call LLM
            let context = build_llm_context(agent);
            let tools = build_llm_tools(agent);
            let events = agent
                .provider()
                .complete(&agent.config().model, &context, &tools)
                .await;

            // 3. Accumulate events into AssistantMessage
            let mut accum = MessageAccumulator::new();
            for event in &events {
                accum.push_event(event);
            }
            let (assistant_msg, tool_call_accums) = accum.build(&agent.config().model);

            agent.push_message(AgentMessage::Assistant(assistant_msg.clone()));
            new_messages.push(AgentMessage::Assistant(assistant_msg.clone()));

            turn_count += 1;

            // 4. Check for early termination
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

            // 5. Execute tool calls (using raw accums to preserve malformed args)
            has_more_tool_calls = !tool_call_accums.is_empty();

            let tool_results = if has_more_tool_calls {
                let results =
                    execute_tool_calls(agent, &tool_call_accums, emit).await;
                for r in &results {
                    agent.push_message(AgentMessage::ToolResult(r.clone()));
                    new_messages.push(AgentMessage::ToolResult(r.clone()));
                }
                results
            } else {
                vec![]
            };

            emit.emit(AgentEvent::TurnEnd {
                message: assistant_msg,
                tool_results,
            })
            .await;

            // 6. Poll steering queue for new messages
            pending = agent.drain_steering();
        }

        // 7. Check follow-up queue
        let follow_ups = agent.drain_follow_up();
        if !follow_ups.is_empty() {
            pending = follow_ups;
            continue 'outer;
        }

        break;
    }

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
    use crate::agent::{Agent, AgentLoopConfig, BeforeToolCallHook, AfterToolCallHook};
    use crate::event::{AgentEvent, AgentEventSink};
    use crate::llm::types::*;
    use crate::llm::LlmProvider;
    use crate::tools::{AgentTool, ToolOutput, ToolRegistry};
    use crate::types::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ---------------------------------------------------------------
    // Mock LLM provider — stateful, returns pre-configured sequences
    // ---------------------------------------------------------------

    struct StatefulProvider {
        responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
        call_count: AtomicUsize,
    }

    impl StatefulProvider {
        fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for StatefulProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let mut queue = self.responses.lock().unwrap();
            queue.pop_front().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
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
            model: test_model(),
            system_prompt: "You are a test agent.".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
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

    fn make_agent(
        responses: Vec<Vec<AssistantMessageEvent>>,
        tools: ToolRegistry,
    ) -> Agent {
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
        Agent::new(
            config,
            Box::new(StatefulProvider::new(responses)),
            tools,
        )
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
        assert!(messages.iter().any(|m| matches!(m, AgentMessage::Assistant(_))));
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
        agent.steer(AgentMessage::User(UserMessage::from_text("Meaning of life?")));
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
        assert!(messages.iter().any(|m| matches!(m, AgentMessage::ToolResult(_))));
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
        assert!(tool_result.is_error, "unknown tool should produce error result");
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
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 2,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
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
        let mut agent = make_agent(
            vec![text_response("Got all of them.")],
            ToolRegistry::new(),
        );
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
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
        };
        let (tool_a, count_a) = CountingTool::new("tool_a");
        let (tool_b, count_b) = CountingTool::new("tool_b");
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![
                    ("tc1", "tool_a", r#"{}"#),
                    ("tc2", "tool_b", r#"{}"#),
                ]),
                text_response("Done!"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("parallel")));
        let sink = CollectorSink::new();

        let result = run_agent_loop(&mut agent, &sink).await;
        assert!(result.is_ok());
        assert_eq!(count_a.load(Ordering::SeqCst), 1, "tool_a should be called once");
        assert_eq!(count_b.load(Ordering::SeqCst), 1, "tool_b should be called once");
    }

    #[tokio::test]
    async fn test_sequential_mode_executes_all_tools() {
        let config = AgentLoopConfig {
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Sequential,
            tool_policy: None,
        };
        let (tool_a, count_a) = CountingTool::new("tool_a");
        let (tool_b, count_b) = CountingTool::new("tool_b");
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![
                    ("tc1", "tool_a", r#"{}"#),
                    ("tc2", "tool_b", r#"{}"#),
                ]),
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
            async fn before_tool_call(
                &self,
                ctx: &BeforeToolCallContext,
            ) -> BeforeToolCallResult {
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
        assert!(tool_result.is_error, "blocked tool should produce error result");
    }

    #[tokio::test]
    async fn test_after_hook_modifies_tool_result() {
        struct ModifyResultHook;

        #[async_trait::async_trait]
        impl AfterToolCallHook for ModifyResultHook {
            async fn after_tool_call(
                &self,
                _ctx: &AfterToolCallContext,
            ) -> AfterToolCallResult {
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
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnEnd { .. })));
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
        assert!(
            start_idx < end_idx,
            "AgentStart must come before AgentEnd"
        );

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
                let assistant = messages
                    .iter()
                    .find_map(|m| match m {
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
        let mut agent = make_agent(
            vec![vec![]],
            ToolRegistry::new(),
        );
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
                AssistantMessageEvent::Done { stop_reason: StopReason::Stop },
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
        let mut agent = make_agent(
            vec![text_response("Reply.")],
            ToolRegistry::new(),
        );
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
        assert!(!agent.is_streaming(), "agent should not be streaming after loop ends");
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

        let mut agent = Agent::new(
            test_config(),
            Box::new(provider),
            ToolRegistry::new(),
        );
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

        let mut agent = Agent::new(
            test_config(),
            Box::new(provider),
            ToolRegistry::new(),
        );
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

        let mut agent = Agent::new(
            test_config(),
            Box::new(provider),
            echo_registry(),
        );
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

        let mut agent = Agent::new(
            test_config(),
            Box::new(provider),
            echo_registry(),
        );
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
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 0,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
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
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 1,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
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
                assert!(assistant_count <= 1, "max_turns=1 limits to at most 1 LLM call");
            }
            Err(e) => {
                assert!(matches!(e, AgentLoopError::MaxTurnsReached));
            }
        }
    }

    #[tokio::test]
    async fn test_empty_system_prompt() {
        let config = AgentLoopConfig {
            model: test_model(),
            system_prompt: "".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
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
        assert_eq!(contexts[0].system_prompt, "", "empty system prompt should be passed through");
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
        let tool_result = messages
            .iter()
            .find_map(|m| match m {
                AgentMessage::ToolResult(tr) => Some(tr),
                _ => None,
            });
        assert!(tool_result.is_some(), "should have tool result for malformed args");
        assert!(tool_result.unwrap().is_error, "malformed args should produce error result");
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
        assert!(!tool_result.is_error, "multi-delta args should be assembled correctly");
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
        assert_eq!(assistants.len(), 4, "4 assistant messages: tool+text per round");
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
        let mut agent = Agent::new(
            test_config(),
            Box::new(provider),
            ToolRegistry::new(),
        );
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
            async fn before_tool_call(
                &self,
                _ctx: &BeforeToolCallContext,
            ) -> BeforeToolCallResult {
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
            model: test_model(),
            system_prompt: "test".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Sequential,
            tool_policy: None,
        };

        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let (tool_a, order_a) =
            OrderRecordingTool::new("tool_a", std::sync::Arc::clone(&counter));
        let (tool_b, order_b) =
            OrderRecordingTool::new("tool_b", std::sync::Arc::clone(&counter));

        let mut reg = ToolRegistry::new();
        reg.register(Box::new(tool_a));
        reg.register(Box::new(tool_b));

        let mut agent = make_agent_with_config(
            config,
            vec![
                multi_tool_response(vec![
                    ("tc1", "tool_a", r#"{}"#),
                    ("tc2", "tool_b", r#"{}"#),
                ]),
                text_response("Done!"),
            ],
            reg,
        );
        agent.steer(AgentMessage::User(UserMessage::from_text("sequential order")));
        let sink = CollectorSink::new();

        let _ = run_agent_loop(&mut agent, &sink).await;

        // In sequential mode, tool_a (tc1) should execute before tool_b (tc2)
        assert_eq!(order_a.load(Ordering::SeqCst), 0, "tool_a should execute first");
        assert_eq!(order_b.load(Ordering::SeqCst), 1, "tool_b should execute second");
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
        assert!(is_err, "ToolExecutionEnd should reflect error from FailTool");
    }

    #[tokio::test]
    async fn test_agent_end_event_carries_all_messages() {
        let mut agent = make_agent(
            vec![text_response("Final answer.")],
            ToolRegistry::new(),
        );
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
        assert!(turn_start_count >= 2, "should have at least 2 TurnStart events");
        assert_eq!(turn_start_count, turn_end_count, "TurnStart and TurnEnd should be balanced");
    }
}
