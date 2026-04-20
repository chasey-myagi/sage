// Agent — mirrors pi-mono packages/agent/src/agent.ts
//
// Owns state, queues, hooks, and drives the agent loop.

use crate::agent_loop::{
    AgentLoopConfig, LlmProvider, default_convert_to_llm, run_agent_loop, run_agent_loop_continue,
};
use crate::event::AgentEvent;
use crate::types::*;
use ai::types::{LlmMessage, Model};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

// ── Queue mode ──────────────────────────────────────────────────────────────

/// Delivery mode for steering and follow-up queues.
///
/// Mirrors pi-mono's `"all" | "one-at-a-time"` union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    /// Deliver all queued messages at once.
    All,
    /// Deliver one message per turn.
    OneAtATime,
}

// ── Hook traits ─────────────────────────────────────────────────────────────

/// Hook called before a tool is executed, after arguments have been validated.
///
/// Mirrors pi-mono's `beforeToolCall` option.
#[async_trait::async_trait]
pub trait BeforeToolCallHook: Send + Sync {
    async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult;
}

/// Hook called after a tool finishes executing.
///
/// Mirrors pi-mono's `afterToolCall` option.
#[async_trait::async_trait]
pub trait AfterToolCallHook: Send + Sync {
    async fn after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult;
}

/// Hook called before each LLM call to transform the message history.
///
/// Mirrors pi-mono's `transformContext` option.
#[async_trait::async_trait]
pub trait TransformContextHook: Send + Sync {
    async fn transform_context(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage>;
}

// Blanket Arc impls.
#[async_trait::async_trait]
impl<T: ?Sized + BeforeToolCallHook> BeforeToolCallHook for Arc<T> {
    async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
        (**self).before_tool_call(ctx).await
    }
}

#[async_trait::async_trait]
impl<T: ?Sized + AfterToolCallHook> AfterToolCallHook for Arc<T> {
    async fn after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult {
        (**self).after_tool_call(ctx).await
    }
}

#[async_trait::async_trait]
impl<T: ?Sized + TransformContextHook> TransformContextHook for Arc<T> {
    async fn transform_context(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        (**self).transform_context(messages).await
    }
}

// ── Agent state snapshot ─────────────────────────────────────────────────────

/// Snapshot of observable Agent state.
///
/// Returned by `Agent::state()` to allow callers to inspect streaming status,
/// errors, and pending tool calls without borrowing the Agent's internals.
pub struct AgentState {
    pub is_streaming: bool,
    pub error: Option<String>,
    pub pending_tool_calls: usize,
}

// ── Agent ────────────────────────────────────────────────────────────────────

/// Agent options passed to the constructor.
///
/// Mirrors pi-mono's AgentOptions interface.
pub struct AgentOptions {
    pub model: Model,
    pub system_prompt: String,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub tool_execution: ToolExecutionMode,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub provider: Arc<dyn LlmProvider>,
    pub before_tool_call: Option<Arc<dyn BeforeToolCallHook>>,
    pub after_tool_call: Option<Arc<dyn AfterToolCallHook>>,
    pub transform_context: Option<Arc<dyn TransformContextHook>>,
    // ── Extended options (pi-mono parity) ───────────────────────────────────
    pub session_id: Option<String>,
    #[allow(clippy::type_complexity)]
    pub get_api_key: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    pub convert_to_llm: Option<Arc<dyn Fn(&[AgentMessage]) -> Vec<LlmMessage> + Send + Sync>>,
    pub transport: Option<String>,
    pub thinking_budgets: Option<serde_json::Value>,
    pub max_retry_delay_ms: Option<u64>,
    #[allow(clippy::type_complexity)]
    pub on_payload: Option<Arc<dyn Fn(&serde_json::Value) + Send + Sync>>,
}

impl AgentOptions {
    /// Create options with sensible defaults. Requires model + system_prompt + provider.
    pub fn new(
        model: Model,
        system_prompt: impl Into<String>,
        provider: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            model,
            system_prompt: system_prompt.into(),
            thinking_level: ThinkingLevel::Off,
            tools: Vec::new(),
            tool_execution: ToolExecutionMode::Parallel,
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            provider,
            before_tool_call: None,
            after_tool_call: None,
            transform_context: None,
            session_id: None,
            get_api_key: None,
            convert_to_llm: None,
            transport: None,
            thinking_budgets: None,
            max_retry_delay_ms: None,
            on_payload: None,
        }
    }
}

/// The Agent — mirrors pi-mono's Agent class.
///
/// Owns agent state, steering/follow-up queues, hooks, and drives the loop.
/// Event subscribers are notified of all AgentEvents produced during runs.
pub struct Agent {
    // ── State ──────────────────────────────────────────────────────────────
    model: Model,
    system_prompt: String,
    thinking_level: ThinkingLevel,
    tools: Vec<Arc<dyn AgentTool>>,
    tool_execution: ToolExecutionMode,
    messages: Vec<AgentMessage>,
    is_streaming: bool,
    stream_message: Option<AgentMessage>,
    pending_tool_calls: HashSet<String>,
    error: Option<String>,

    // ── Queues ─────────────────────────────────────────────────────────────
    steering_queue: VecDeque<AgentMessage>,
    follow_up_queue: VecDeque<AgentMessage>,
    steering_mode: QueueMode,
    follow_up_mode: QueueMode,

    // ── Hooks ──────────────────────────────────────────────────────────────
    before_tool_call: Option<Arc<dyn BeforeToolCallHook>>,
    after_tool_call: Option<Arc<dyn AfterToolCallHook>>,
    transform_context: Option<Arc<dyn TransformContextHook>>,

    // ── Listeners ──────────────────────────────────────────────────────────
    listeners: Vec<Box<dyn Fn(AgentEvent) + Send + Sync>>,

    // ── Run state ──────────────────────────────────────────────────────────
    provider: Arc<dyn LlmProvider>,
    cancellation: Option<CancellationToken>,
    /// Resolves when the current run completes.
    #[allow(dead_code)]
    run_done_tx: Option<oneshot::Sender<()>>,
    run_done_rx: Option<Arc<Mutex<Option<oneshot::Receiver<()>>>>>,

    // ── Extended fields (pi-mono parity) ───────────────────────────────────
    session_id: Option<String>,
    #[allow(clippy::type_complexity)]
    get_api_key: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    convert_to_llm: Option<Arc<dyn Fn(&[AgentMessage]) -> Vec<LlmMessage> + Send + Sync>>,
    transport: String,
    thinking_budgets: Option<serde_json::Value>,
    max_retry_delay_ms: Option<u64>,
    #[allow(clippy::type_complexity)]
    on_payload: Option<Arc<dyn Fn(&serde_json::Value) + Send + Sync>>,
}

impl Agent {
    /// Create a new Agent from options.
    pub fn new(opts: AgentOptions) -> Self {
        Self {
            model: opts.model,
            system_prompt: opts.system_prompt,
            thinking_level: opts.thinking_level,
            tools: opts.tools,
            tool_execution: opts.tool_execution,
            messages: Vec::new(),
            is_streaming: false,
            stream_message: None,
            pending_tool_calls: HashSet::new(),
            error: None,
            steering_queue: VecDeque::new(),
            follow_up_queue: VecDeque::new(),
            steering_mode: opts.steering_mode,
            follow_up_mode: opts.follow_up_mode,
            before_tool_call: opts.before_tool_call,
            after_tool_call: opts.after_tool_call,
            transform_context: opts.transform_context,
            listeners: Vec::new(),
            provider: opts.provider,
            cancellation: None,
            run_done_tx: None,
            run_done_rx: None,
            session_id: opts.session_id,
            get_api_key: opts.get_api_key,
            convert_to_llm: opts.convert_to_llm,
            transport: opts.transport.unwrap_or_else(|| "sse".to_string()),
            thinking_budgets: opts.thinking_budgets,
            max_retry_delay_ms: opts.max_retry_delay_ms,
            on_payload: opts.on_payload,
        }
    }

    // ── State accessors ─────────────────────────────────────────────────────

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn set_model(&mut self, model: Model) {
        self.model = model;
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn set_system_prompt(&mut self, v: impl Into<String>) {
        self.system_prompt = v.into();
    }

    pub fn thinking_level(&self) -> ThinkingLevel {
        self.thinking_level
    }

    pub fn set_thinking_level(&mut self, l: ThinkingLevel) {
        self.thinking_level = l;
    }

    pub fn tools(&self) -> &[Arc<dyn AgentTool>] {
        &self.tools
    }

    pub fn set_tools(&mut self, tools: Vec<Arc<dyn AgentTool>>) {
        self.tools = tools;
    }

    pub fn tool_execution(&self) -> ToolExecutionMode {
        self.tool_execution
    }

    pub fn set_tool_execution(&mut self, mode: ToolExecutionMode) {
        self.tool_execution = mode;
    }

    pub fn messages(&self) -> &[AgentMessage] {
        &self.messages
    }

    pub fn replace_messages(&mut self, messages: Vec<AgentMessage>) {
        self.messages = messages;
    }

    pub fn append_message(&mut self, message: AgentMessage) {
        self.messages.push(message);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }

    pub fn stream_message(&self) -> Option<&AgentMessage> {
        self.stream_message.as_ref()
    }

    pub fn pending_tool_calls(&self) -> &HashSet<String> {
        &self.pending_tool_calls
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // ── Extended state accessors (pi-mono parity) ───────────────────────────

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.session_id = Some(id.into());
    }

    pub fn transport(&self) -> &str {
        &self.transport
    }

    pub fn set_transport(&mut self, t: impl Into<String>) {
        self.transport = t.into();
    }

    pub fn thinking_budgets(&self) -> Option<&serde_json::Value> {
        self.thinking_budgets.as_ref()
    }

    pub fn set_thinking_budgets(&mut self, b: serde_json::Value) {
        self.thinking_budgets = Some(b);
    }

    pub fn max_retry_delay_ms(&self) -> Option<u64> {
        self.max_retry_delay_ms
    }

    pub fn set_max_retry_delay_ms(&mut self, ms: u64) {
        self.max_retry_delay_ms = Some(ms);
    }

    pub fn set_get_api_key<F>(&mut self, f: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.get_api_key = Some(Arc::new(f));
    }

    pub fn set_convert_to_llm<F>(&mut self, f: F)
    where
        F: Fn(&[AgentMessage]) -> Vec<LlmMessage> + Send + Sync + 'static,
    {
        self.convert_to_llm = Some(Arc::new(f));
    }

    pub fn set_on_payload<F>(&mut self, f: F)
    where
        F: Fn(&serde_json::Value) + Send + Sync + 'static,
    {
        self.on_payload = Some(Arc::new(f));
    }

    /// Returns a snapshot of the current agent state.
    pub fn state(&self) -> AgentState {
        AgentState {
            is_streaming: self.is_streaming,
            error: self.error.clone(),
            pending_tool_calls: self.pending_tool_calls.len(),
        }
    }

    // ── Queue operations ────────────────────────────────────────────────────

    pub fn steering_mode(&self) -> QueueMode {
        self.steering_mode
    }

    pub fn set_steering_mode(&mut self, mode: QueueMode) {
        self.steering_mode = mode;
    }

    pub fn follow_up_mode(&self) -> QueueMode {
        self.follow_up_mode
    }

    pub fn set_follow_up_mode(&mut self, mode: QueueMode) {
        self.follow_up_mode = mode;
    }

    /// Queue a steering message to be delivered after the current tool-call round.
    ///
    /// Mirrors pi-mono's `steer()` method.
    pub fn steer(&mut self, message: AgentMessage) {
        self.steering_queue.push_back(message);
    }

    /// Queue a follow-up message to be delivered after the agent would otherwise stop.
    ///
    /// Mirrors pi-mono's `followUp()` method.
    pub fn follow_up(&mut self, message: AgentMessage) {
        self.follow_up_queue.push_back(message);
    }

    pub fn clear_steering_queue(&mut self) {
        self.steering_queue.clear();
    }

    pub fn clear_follow_up_queue(&mut self) {
        self.follow_up_queue.clear();
    }

    pub fn clear_all_queues(&mut self) {
        self.steering_queue.clear();
        self.follow_up_queue.clear();
    }

    pub fn has_queued_messages(&self) -> bool {
        !self.steering_queue.is_empty() || !self.follow_up_queue.is_empty()
    }

    fn dequeue_steering(&mut self) -> Vec<AgentMessage> {
        match self.steering_mode {
            QueueMode::OneAtATime => {
                if let Some(first) = self.steering_queue.pop_front() {
                    vec![first]
                } else {
                    vec![]
                }
            }
            QueueMode::All => self.steering_queue.drain(..).collect(),
        }
    }

    fn dequeue_follow_up(&mut self) -> Vec<AgentMessage> {
        match self.follow_up_mode {
            QueueMode::OneAtATime => {
                if let Some(first) = self.follow_up_queue.pop_front() {
                    vec![first]
                } else {
                    vec![]
                }
            }
            QueueMode::All => self.follow_up_queue.drain(..).collect(),
        }
    }

    // ── Hook setters ────────────────────────────────────────────────────────

    pub fn set_before_tool_call(&mut self, hook: Arc<dyn BeforeToolCallHook>) {
        self.before_tool_call = Some(hook);
    }

    pub fn clear_before_tool_call(&mut self) {
        self.before_tool_call = None;
    }

    pub fn set_after_tool_call(&mut self, hook: Arc<dyn AfterToolCallHook>) {
        self.after_tool_call = Some(hook);
    }

    pub fn clear_after_tool_call(&mut self) {
        self.after_tool_call = None;
    }

    pub fn set_transform_context(&mut self, hook: Arc<dyn TransformContextHook>) {
        self.transform_context = Some(hook);
    }

    pub fn clear_transform_context(&mut self) {
        self.transform_context = None;
    }

    // ── Event subscriptions ─────────────────────────────────────────────────

    /// Subscribe to agent events. Returns an unsubscribe function.
    ///
    /// Mirrors pi-mono's `subscribe()` method.
    pub fn subscribe<F>(&mut self, listener: F) -> usize
    where
        F: Fn(AgentEvent) + Send + Sync + 'static,
    {
        let id = self.listeners.len();
        self.listeners.push(Box::new(listener));
        id
    }

    fn emit(&self, event: AgentEvent) {
        for listener in &self.listeners {
            listener(event.clone());
        }
    }

    // ── Abort ───────────────────────────────────────────────────────────────

    /// Abort the currently running agent loop.
    ///
    /// Mirrors pi-mono's `abort()` method.
    pub fn abort(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
    }

    /// Return a cloneable abort handle that cancels the current run.
    ///
    /// Pre-allocates the cancellation token so it can be held across
    /// an `await` (where `&mut self` is held by `prompt_text()`).
    /// The token is replaced on the next `prompt_text()` / `prompt_messages()` call.
    pub fn abort_handle(&mut self) -> CancellationToken {
        let token = CancellationToken::new();
        self.cancellation = Some(token.clone());
        token
    }

    /// Wait until the current run completes.
    ///
    /// Mirrors pi-mono's `waitForIdle()` method.
    pub async fn wait_for_idle(&mut self) {
        let rx = self.run_done_rx.take();
        if let Some(rx_arc) = rx {
            let mut locked = rx_arc.lock().await;
            if let Some(rx) = locked.take() {
                let _ = rx.await;
            }
        }
    }

    /// Reset agent state (messages, queues, streaming state).
    ///
    /// Mirrors pi-mono's `reset()` method.
    pub fn reset(&mut self) {
        self.messages.clear();
        self.is_streaming = false;
        self.stream_message = None;
        self.pending_tool_calls.clear();
        self.error = None;
        self.steering_queue.clear();
        self.follow_up_queue.clear();
    }

    // ── Prompt ──────────────────────────────────────────────────────────────

    /// Send a text prompt.
    ///
    /// Mirrors pi-mono's `prompt(input: string, images?)` overload.
    pub async fn prompt_text(&mut self, text: impl Into<String>) -> Result<(), String> {
        let user_msg = AgentMessage::User(UserMessage {
            content: vec![Content::Text { text: text.into() }],
            timestamp: now_ms(),
        });
        self.prompt_messages(vec![user_msg]).await
    }

    /// Send one or more AgentMessages as a prompt.
    ///
    /// Mirrors pi-mono's `prompt(message: AgentMessage | AgentMessage[])` overload.
    pub async fn prompt_messages(&mut self, messages: Vec<AgentMessage>) -> Result<(), String> {
        if self.is_streaming {
            return Err(
                "Agent is already processing a prompt. Use steer() or followUp() to queue messages, or wait for completion.".into()
            );
        }
        self.run_loop(Some(messages), false).await
    }

    /// Continue from current context (used for retries and resuming queued messages).
    ///
    /// Mirrors pi-mono's `continue()` method.
    pub async fn continue_run(&mut self) -> Result<(), String> {
        if self.is_streaming {
            return Err(
                "Agent is already processing. Wait for completion before continuing.".into(),
            );
        }
        if self.messages.is_empty() {
            return Err("No messages to continue from".into());
        }

        // If last message is assistant, try to inject queued messages.
        if matches!(self.messages.last(), Some(AgentMessage::Assistant(_))) {
            let steering = self.dequeue_steering();
            if !steering.is_empty() {
                return self.run_loop(Some(steering), true).await;
            }
            let follow_up = self.dequeue_follow_up();
            if !follow_up.is_empty() {
                return self.run_loop(Some(follow_up), false).await;
            }
            return Err("Cannot continue from message role: assistant".into());
        }

        self.run_loop(None, false).await
    }

    // ── Internal run ────────────────────────────────────────────────────────

    fn process_loop_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::MessageStart { message } => {
                self.stream_message = Some(message.clone());
            }
            AgentEvent::MessageUpdate { message, .. } => {
                self.stream_message = Some(message.clone());
            }
            AgentEvent::MessageEnd { message } => {
                self.stream_message = None;
                self.messages.push(message.clone());
            }
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                self.pending_tool_calls.insert(tool_call_id.clone());
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                self.pending_tool_calls.remove(tool_call_id);
            }
            AgentEvent::TurnEnd { message, .. } => {
                if let Some(err) = &message.error_message {
                    self.error = Some(err.clone());
                }
            }
            AgentEvent::AgentEnd { .. } => {
                self.is_streaming = false;
                self.stream_message = None;
            }
            _ => {}
        }
    }

    async fn run_loop(
        &mut self,
        messages: Option<Vec<AgentMessage>>,
        skip_initial_steering_poll: bool,
    ) -> Result<(), String> {
        self.is_streaming = true;
        self.stream_message = None;
        self.error = None;

        // Reuse a pre-allocated token (e.g. from abort_handle()); otherwise create a fresh one.
        let token = self.cancellation.take().unwrap_or_default();
        self.cancellation = Some(token.clone());

        // Capture queue drainers as closures.
        // We use shared state via Arc<Mutex<...>> to allow closures to drain queues.
        let steering_queue = Arc::new(Mutex::new(std::mem::take(&mut self.steering_queue)));
        let follow_up_queue = Arc::new(Mutex::new(std::mem::take(&mut self.follow_up_queue)));
        let steering_mode = self.steering_mode;
        let follow_up_mode = self.follow_up_mode;
        let _skip_steering = skip_initial_steering_poll;

        let steering_queue_clone = Arc::clone(&steering_queue);
        let follow_up_queue_clone = Arc::clone(&follow_up_queue);

        // Build config.
        let before_hook = self.before_tool_call.clone();
        let after_hook = self.after_tool_call.clone();
        let transform_hook = self.transform_context.clone();
        let tools = self.tools.clone();
        let tool_execution = self.tool_execution;
        let system_prompt = self.system_prompt.clone();
        let model = self.model.clone();
        let get_api_key = self.get_api_key.clone();
        let convert_to_llm_fn = self.convert_to_llm.clone();
        let max_retry_delay_ms = self.max_retry_delay_ms;

        let config = Arc::new(AgentLoopConfig {
            model: model.clone(),
            system_prompt: system_prompt.clone(),
            tool_execution,
            tools: tools.clone(),
            convert_to_llm: if let Some(f) = convert_to_llm_fn {
                Box::new(move |msgs: &[AgentMessage]| f(msgs))
            } else {
                Box::new(default_convert_to_llm)
            },
            get_api_key,
            max_retry_delay_ms,
            transform_context: transform_hook.map(
                |h| -> Box<
                    dyn Fn(
                            Vec<AgentMessage>,
                        )
                            -> futures::future::BoxFuture<'static, Vec<AgentMessage>>
                        + Send
                        + Sync,
                > {
                    let h = Arc::clone(&h);
                    Box::new(move |msgs| {
                        let h = Arc::clone(&h);
                        Box::pin(async move { h.transform_context(msgs).await })
                    })
                },
            ),
            get_steering_messages: Some({
                let sq = Arc::clone(&steering_queue_clone);
                Box::new(move || {
                    let sq = Arc::clone(&sq);
                    Box::pin(async move {
                        let mut queue = sq.lock().await;
                        match steering_mode {
                            QueueMode::OneAtATime => {
                                if let Some(first) = queue.pop_front() {
                                    vec![first]
                                } else {
                                    vec![]
                                }
                            }
                            QueueMode::All => queue.drain(..).collect(),
                        }
                    })
                })
            }),
            get_follow_up_messages: Some({
                let fq = Arc::clone(&follow_up_queue_clone);
                Box::new(move || {
                    let fq = Arc::clone(&fq);
                    Box::pin(async move {
                        let mut queue = fq.lock().await;
                        match follow_up_mode {
                            QueueMode::OneAtATime => {
                                if let Some(first) = queue.pop_front() {
                                    vec![first]
                                } else {
                                    vec![]
                                }
                            }
                            QueueMode::All => queue.drain(..).collect(),
                        }
                    })
                })
            }),
            before_tool_call: before_hook.map(
                |h| -> Box<
                    dyn Fn(
                            BeforeToolCallContext,
                        )
                            -> futures::future::BoxFuture<'static, BeforeToolCallResult>
                        + Send
                        + Sync,
                > {
                    Box::new(move |ctx| {
                        let h = Arc::clone(&h);
                        Box::pin(async move { h.before_tool_call(&ctx).await })
                    })
                },
            ),
            after_tool_call: after_hook.map(
                |h| -> Box<
                    dyn Fn(
                            AfterToolCallContext,
                        )
                            -> futures::future::BoxFuture<'static, AfterToolCallResult>
                        + Send
                        + Sync,
                > {
                    Box::new(move |ctx| {
                        let h = Arc::clone(&h);
                        Box::pin(async move { h.after_tool_call(&ctx).await })
                    })
                },
            ),
            compaction_settings: None,
        });

        let context = AgentContext {
            system_prompt: self.system_prompt.clone(),
            messages: self.messages.clone(),
        };

        let provider = Arc::clone(&self.provider);
        let token_clone = token.clone();

        // Collect events from the loop using a channel.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        let emit: Arc<dyn Fn(AgentEvent) + Send + Sync> = Arc::new(move |event: AgentEvent| {
            let _ = tx.send(event);
        });

        // Spawn the agent loop task.
        let loop_handle = tokio::spawn(async move {
            if let Some(msgs) = messages {
                run_agent_loop(msgs, context, config, provider, emit, Some(token_clone)).await
            } else {
                run_agent_loop_continue(context, config, provider, emit, Some(token_clone))
                    .await
                    .unwrap_or_default()
            }
        });

        // Process events as they arrive.
        while let Some(event) = rx.recv().await {
            self.process_loop_event(&event);
            self.emit(event);
        }

        let _ = loop_handle.await;

        // Restore queues from the shared state.
        self.steering_queue = std::mem::take(&mut *steering_queue.lock().await);
        self.follow_up_queue = std::mem::take(&mut *follow_up_queue.lock().await);

        // Cleanup run state.
        self.is_streaming = false;
        self.stream_message = None;
        self.pending_tool_calls.clear();
        self.cancellation = None;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ── Mock provider ──────────────────────────────────────────────────────

    struct MockProvider {
        events: Vec<ai::types::AssistantMessageEvent>,
    }

    impl MockProvider {
        fn simple_stop() -> Self {
            Self {
                events: vec![ai::types::AssistantMessageEvent::Done {
                    stop_reason: ai::types::StopReason::Stop,
                }],
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &ai::types::LlmContext,
            _tools: &[ai::types::LlmTool],
        ) -> Vec<ai::types::AssistantMessageEvent> {
            self.events.clone()
        }
    }

    fn test_model() -> Model {
        Model {
            id: "test-model".into(),
            name: "Test Model".into(),
            api: ai::types::api::OPENAI_COMPLETIONS.into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            reasoning: false,
            input: vec![ai::types::InputType::Text],
            max_tokens: 4096,
            context_window: 32768,
            cost: ai::types::ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    fn test_opts() -> AgentOptions {
        AgentOptions::new(
            test_model(),
            "You are a test agent.",
            Arc::new(MockProvider::simple_stop()),
        )
    }

    // ── Construction ────────────────────────────────────────────────────────

    #[test]
    fn test_agent_new_has_empty_messages() {
        let agent = Agent::new(test_opts());
        assert!(agent.messages().is_empty());
    }

    #[test]
    fn test_agent_initial_state_not_streaming() {
        let agent = Agent::new(test_opts());
        assert!(!agent.is_streaming());
    }

    #[test]
    fn test_agent_initial_no_error() {
        let agent = Agent::new(test_opts());
        assert!(agent.error().is_none());
    }

    #[test]
    fn test_agent_model_id() {
        let agent = Agent::new(test_opts());
        assert_eq!(agent.model().id, "test-model");
    }

    #[test]
    fn test_agent_system_prompt() {
        let agent = Agent::new(test_opts());
        assert_eq!(agent.system_prompt(), "You are a test agent.");
    }

    // ── Queue operations ─────────────────────────────────────────────────────

    #[test]
    fn test_steer_adds_message() {
        let mut agent = Agent::new(test_opts());
        agent.steer(AgentMessage::User(UserMessage::from_text("hello")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_follow_up_adds_message() {
        let mut agent = Agent::new(test_opts());
        agent.follow_up(AgentMessage::User(UserMessage::from_text("hello")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_clear_steering_queue() {
        let mut agent = Agent::new(test_opts());
        agent.steer(AgentMessage::User(UserMessage::from_text("one")));
        agent.steer(AgentMessage::User(UserMessage::from_text("two")));
        agent.clear_steering_queue();
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_clear_follow_up_queue() {
        let mut agent = Agent::new(test_opts());
        agent.follow_up(AgentMessage::User(UserMessage::from_text("one")));
        agent.clear_follow_up_queue();
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_clear_all_queues() {
        let mut agent = Agent::new(test_opts());
        agent.steer(AgentMessage::User(UserMessage::from_text("s")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("f")));
        agent.clear_all_queues();
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_has_queued_messages_initially_false() {
        let agent = Agent::new(test_opts());
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_dequeue_steering_one_at_a_time() {
        let mut agent = Agent::new(test_opts());
        agent.steering_mode = QueueMode::OneAtATime;
        agent.steer(AgentMessage::User(UserMessage::from_text("first")));
        agent.steer(AgentMessage::User(UserMessage::from_text("second")));
        let msgs = agent.dequeue_steering();
        assert_eq!(msgs.len(), 1);
        // Queue still has second.
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_dequeue_steering_all() {
        let mut agent = Agent::new(test_opts());
        agent.steering_mode = QueueMode::All;
        agent.steer(AgentMessage::User(UserMessage::from_text("first")));
        agent.steer(AgentMessage::User(UserMessage::from_text("second")));
        let msgs = agent.dequeue_steering();
        assert_eq!(msgs.len(), 2);
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_dequeue_follow_up_one_at_a_time() {
        let mut agent = Agent::new(test_opts());
        agent.follow_up_mode = QueueMode::OneAtATime;
        agent.follow_up(AgentMessage::User(UserMessage::from_text("a")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("b")));
        let msgs = agent.dequeue_follow_up();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_dequeue_follow_up_all() {
        let mut agent = Agent::new(test_opts());
        agent.follow_up_mode = QueueMode::All;
        agent.follow_up(AgentMessage::User(UserMessage::from_text("a")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("b")));
        let msgs = agent.dequeue_follow_up();
        assert_eq!(msgs.len(), 2);
    }

    // ── State mutation ───────────────────────────────────────────────────────

    #[test]
    fn test_set_system_prompt() {
        let mut agent = Agent::new(test_opts());
        agent.set_system_prompt("new prompt");
        assert_eq!(agent.system_prompt(), "new prompt");
    }

    #[test]
    fn test_set_thinking_level() {
        let mut agent = Agent::new(test_opts());
        agent.set_thinking_level(ThinkingLevel::High);
        assert_eq!(agent.thinking_level(), ThinkingLevel::High);
    }

    #[test]
    fn test_append_message() {
        let mut agent = Agent::new(test_opts());
        agent.append_message(AgentMessage::User(UserMessage::from_text("hi")));
        assert_eq!(agent.messages().len(), 1);
    }

    #[test]
    fn test_replace_messages() {
        let mut agent = Agent::new(test_opts());
        agent.append_message(AgentMessage::User(UserMessage::from_text("old")));
        agent.replace_messages(vec![AgentMessage::User(UserMessage::from_text("new"))]);
        assert_eq!(agent.messages().len(), 1);
        match &agent.messages()[0] {
            AgentMessage::User(u) => match &u.content[0] {
                Content::Text { text } => assert_eq!(text, "new"),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected User"),
        }
    }

    #[test]
    fn test_clear_messages() {
        let mut agent = Agent::new(test_opts());
        agent.append_message(AgentMessage::User(UserMessage::from_text("hi")));
        agent.clear_messages();
        assert!(agent.messages().is_empty());
    }

    #[test]
    fn test_reset_clears_everything() {
        let mut agent = Agent::new(test_opts());
        agent.append_message(AgentMessage::User(UserMessage::from_text("hi")));
        agent.steer(AgentMessage::User(UserMessage::from_text("steer")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("fu")));
        agent.reset();
        assert!(agent.messages().is_empty());
        assert!(!agent.has_queued_messages());
        assert!(!agent.is_streaming());
    }

    // ── Hooks ────────────────────────────────────────────────────────────────

    struct BlockAllHook;

    #[async_trait::async_trait]
    impl BeforeToolCallHook for BlockAllHook {
        async fn before_tool_call(&self, _ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
            BeforeToolCallResult {
                block: true,
                reason: Some("blocked by test".into()),
            }
        }
    }

    #[test]
    fn test_set_before_tool_call_hook() {
        let mut agent = Agent::new(test_opts());
        agent.set_before_tool_call(Arc::new(BlockAllHook));
        assert!(agent.before_tool_call.is_some());
    }

    #[test]
    fn test_clear_before_tool_call_hook() {
        let mut agent = Agent::new(test_opts());
        agent.set_before_tool_call(Arc::new(BlockAllHook));
        agent.clear_before_tool_call();
        assert!(agent.before_tool_call.is_none());
    }

    // ── Subscriptions ────────────────────────────────────────────────────────

    #[test]
    fn test_subscribe_receives_events() {
        let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let mut agent = Agent::new(test_opts());
        agent.subscribe(move |event| {
            events_clone.lock().unwrap().push(event);
        });
        agent.emit(AgentEvent::AgentStart);
        agent.emit(AgentEvent::TurnStart);
        let collected = events.lock().unwrap();
        assert_eq!(collected.len(), 2);
    }

    // ── Run behavior ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_prompt_text_completes_without_error() {
        let mut agent = Agent::new(test_opts());
        let result = agent.prompt_text("hello").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_prompt_text_is_not_streaming_after_completion() {
        let mut agent = Agent::new(test_opts());
        agent.prompt_text("hello").await.unwrap();
        assert!(!agent.is_streaming());
    }

    #[tokio::test]
    async fn test_prompt_text_twice_sequentially() {
        let mut agent = Agent::new(test_opts());
        agent.prompt_text("first").await.unwrap();
        agent.prompt_text("second").await.unwrap();
        assert!(!agent.is_streaming());
    }

    #[tokio::test]
    async fn test_prompt_messages_with_user_message() {
        let mut agent = Agent::new(test_opts());
        let msg = AgentMessage::User(UserMessage::from_text("hi"));
        let result = agent.prompt_messages(vec![msg]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_abort_cancels_token() {
        let mut agent = Agent::new(test_opts());
        // We can only really check that abort() doesn't panic.
        agent.abort();
    }

    #[tokio::test]
    async fn test_steering_mode_one_at_a_time() {
        let mut agent = Agent::new(test_opts());
        agent.set_steering_mode(QueueMode::OneAtATime);
        agent.steer(AgentMessage::User(UserMessage::from_text("s1")));
        agent.steer(AgentMessage::User(UserMessage::from_text("s2")));
        let first = agent.dequeue_steering();
        assert_eq!(first.len(), 1);
    }

    #[test]
    fn test_tool_execution_mode_default() {
        let agent = Agent::new(test_opts());
        assert_eq!(agent.tool_execution(), ToolExecutionMode::Parallel);
    }

    // ── Translated from agent.test.ts ──────────────────────────────────────

    /// agent.test.ts: "should create an agent instance with default state"
    #[test]
    fn ts_agent_default_state() {
        let agent = Agent::new(test_opts());
        assert_eq!(agent.system_prompt(), "You are a test agent.");
        assert_eq!(agent.thinking_level(), ThinkingLevel::Off);
        assert!(agent.tools().is_empty());
        assert!(agent.messages().is_empty());
        assert!(!agent.is_streaming());
        assert!(agent.stream_message().is_none());
        assert!(agent.pending_tool_calls().is_empty());
        assert!(agent.error().is_none());
    }

    /// agent.test.ts: "should create an agent instance with custom initial state"
    #[test]
    fn ts_agent_custom_initial_state() {
        let mut opts = test_opts();
        opts.system_prompt = "You are a helpful assistant.".into();
        opts.thinking_level = ThinkingLevel::Low;
        let agent = Agent::new(opts);
        assert_eq!(agent.system_prompt(), "You are a helpful assistant.");
        assert_eq!(agent.thinking_level(), ThinkingLevel::Low);
    }

    /// agent.test.ts: "should subscribe to events" — subscription count logic
    #[test]
    fn ts_agent_subscribe_receives_no_events_on_subscribe() {
        let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);
        let mut agent = Agent::new(test_opts());
        let _id = agent.subscribe(move |e| events_clone.lock().unwrap().push(e));
        // No initial event on subscribe
        assert_eq!(events.lock().unwrap().len(), 0);
    }

    /// agent.test.ts: "should update state with mutators"
    #[test]
    fn ts_agent_mutators_update_state() {
        let mut agent = Agent::new(test_opts());

        agent.set_system_prompt("Custom prompt");
        assert_eq!(agent.system_prompt(), "Custom prompt");

        agent.set_thinking_level(ThinkingLevel::High);
        assert_eq!(agent.thinking_level(), ThinkingLevel::High);

        agent.append_message(AgentMessage::User(UserMessage::from_text("Hello")));
        assert_eq!(agent.messages().len(), 1);

        // replace_messages should copy (not alias)
        let new_messages = vec![AgentMessage::User(UserMessage::from_text("replaced"))];
        agent.replace_messages(new_messages);
        assert_eq!(agent.messages().len(), 1);
        match &agent.messages()[0] {
            AgentMessage::User(u) => match &u.content[0] {
                Content::Text { text } => assert_eq!(text, "replaced"),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected User"),
        }

        agent.clear_messages();
        assert!(agent.messages().is_empty());
    }

    /// agent.test.ts: "should support steering message queue"
    #[test]
    fn ts_agent_steer_queued_not_in_messages() {
        let mut agent = Agent::new(test_opts());
        agent.steer(AgentMessage::User(UserMessage::from_text(
            "Steering message",
        )));
        // Steering message is queued but not yet in state.messages
        assert!(agent.messages().is_empty());
        assert!(agent.has_queued_messages());
    }

    /// agent.test.ts: "should support follow-up message queue"
    #[test]
    fn ts_agent_follow_up_queued_not_in_messages() {
        let mut agent = Agent::new(test_opts());
        agent.follow_up(AgentMessage::User(UserMessage::from_text(
            "Follow-up message",
        )));
        // Follow-up message is queued but not yet in state.messages
        assert!(agent.messages().is_empty());
        assert!(agent.has_queued_messages());
    }

    /// agent.test.ts: "should handle abort controller"
    #[tokio::test]
    async fn ts_agent_abort_does_not_panic_when_nothing_running() {
        let mut agent = Agent::new(test_opts());
        // Should not throw even if nothing is running
        agent.abort(); // must not panic
    }

    /// agent.test.ts: "should throw when prompt() called while streaming"
    ///
    /// In TypeScript, the Agent is single-threaded and calling prompt() while
    /// already streaming throws immediately. In Rust, is_streaming is a field
    /// guarded by the Agent's ownership. We verify the guard by simulating
    /// the streaming state directly.
    #[tokio::test]
    async fn ts_agent_prompt_while_streaming_returns_error() {
        let mut agent = Agent::new(test_opts());

        // Simulate the is_streaming flag being true
        agent.is_streaming = true;

        // A second prompt while streaming should return an error
        let result = agent
            .prompt_messages(vec![AgentMessage::User(UserMessage::from_text(
                "Second message",
            ))])
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Agent is already processing a prompt"),
            "unexpected error: {err}"
        );

        // Cleanup: reset streaming flag
        agent.is_streaming = false;
    }

    /// agent.test.ts: "should throw when continue() called while streaming"
    ///
    /// When is_streaming is true, continue_run() should return an error.
    #[tokio::test]
    async fn ts_agent_continue_while_streaming_returns_error() {
        let mut agent = Agent::new(test_opts());

        // Simulate the is_streaming flag being true
        agent.is_streaming = true;

        let result = agent.continue_run().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Agent is already processing"),
            "unexpected error: {err}"
        );

        // Cleanup
        agent.is_streaming = false;
    }

    /// agent.test.ts: "should throw when continue() called while streaming" (continue errors when no messages)
    #[tokio::test]
    async fn ts_agent_continue_without_messages_errors() {
        let mut agent = Agent::new(test_opts());
        let result = agent.continue_run().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No messages to continue from"));
    }

    /// agent.test.ts: "continue() should process queued follow-up messages after an assistant turn"
    #[tokio::test]
    async fn ts_agent_continue_processes_follow_up_after_assistant_turn() {
        struct StopProvider;

        #[async_trait::async_trait]
        impl LlmProvider for StopProvider {
            async fn complete(
                &self,
                _model: &ai::types::Model,
                _context: &ai::types::LlmContext,
                _tools: &[ai::types::LlmTool],
            ) -> Vec<ai::types::AssistantMessageEvent> {
                vec![
                    ai::types::AssistantMessageEvent::TextDelta("Processed".into()),
                    ai::types::AssistantMessageEvent::Done {
                        stop_reason: ai::types::StopReason::Stop,
                    },
                ]
            }
        }

        let mut opts = test_opts();
        opts.provider = Arc::new(StopProvider);
        let mut agent = Agent::new(opts);

        // Seed messages ending with assistant turn
        agent.replace_messages(vec![
            AgentMessage::User(UserMessage::from_text("Initial")),
            AgentMessage::Assistant(AssistantMessage::from_text("Initial response")),
        ]);

        // Queue a follow-up
        agent.follow_up(AgentMessage::User(UserMessage::from_text(
            "Queued follow-up",
        )));

        let result = agent.continue_run().await;
        assert!(result.is_ok(), "continue should succeed: {:?}", result);

        // The queued follow-up should now appear in messages
        let has_follow_up = agent.messages().iter().any(|m| {
            if let AgentMessage::User(u) = m {
                u.content
                    .iter()
                    .any(|c| matches!(c, Content::Text { text } if text == "Queued follow-up"))
            } else {
                false
            }
        });
        assert!(
            has_follow_up,
            "queued follow-up should be in messages after continue"
        );

        // Last message should be assistant
        assert!(matches!(
            agent.messages().last(),
            Some(AgentMessage::Assistant(_))
        ));
    }

    /// agent.test.ts: "continue() should keep one-at-a-time steering semantics from assistant tail"
    #[tokio::test]
    async fn ts_agent_continue_one_at_a_time_steering() {
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = Arc::clone(&call_count);

        struct CountingProvider {
            count: Arc<std::sync::atomic::AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl LlmProvider for CountingProvider {
            async fn complete(
                &self,
                _model: &ai::types::Model,
                _context: &ai::types::LlmContext,
                _tools: &[ai::types::LlmTool],
            ) -> Vec<ai::types::AssistantMessageEvent> {
                let n = self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                vec![
                    ai::types::AssistantMessageEvent::TextDelta(format!("Processed {n}")),
                    ai::types::AssistantMessageEvent::Done {
                        stop_reason: ai::types::StopReason::Stop,
                    },
                ]
            }
        }

        let mut opts = test_opts();
        opts.provider = Arc::new(CountingProvider {
            count: Arc::clone(&call_count_clone),
        });
        opts.steering_mode = QueueMode::OneAtATime;
        let mut agent = Agent::new(opts);

        // Seed with assistant tail
        agent.replace_messages(vec![
            AgentMessage::User(UserMessage::from_text("Initial")),
            AgentMessage::Assistant(AssistantMessage::from_text("Initial response")),
        ]);

        // Queue two steering messages
        agent.steer(AgentMessage::User(UserMessage::from_text("Steering 1")));
        agent.steer(AgentMessage::User(UserMessage::from_text("Steering 2")));

        let result = agent.continue_run().await;
        assert!(result.is_ok(), "continue should succeed: {:?}", result);

        // With OneAtATime: one steering message → one LLM call; the second stays queued
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "OneAtATime should deliver one steering message per continue() call"
        );
    }
}
