// SageEngine — builder-pattern API for creating and running AI agents.
// "SQLite of AI Agents" — zero-config embed, single crate dependency.

use crate::agent::{
    AfterToolCallHook, Agent, AgentLoopConfig, BeforeToolCallHook, StopAction, StopContext,
    StopHook, TransformContextHook,
};
use crate::agent_loop::{AgentLoopError, run_agent_loop};
use crate::event::{AgentEvent, AgentEventSink, EventReceiver, EventSender, EventStream};
use crate::hook::{HookBus, HookEvent};
use crate::llm::types::*;
use crate::llm::{self, LlmProvider};
use crate::tools::backend::{LocalBackend, SandboxBackend, ToolBackend};
use crate::tools::policy::ToolPolicy;
use crate::tools::{self, AgentTool, ToolRegistry};
use crate::types::*;
use std::sync::Arc;
use std::time::Instant;

// ── SandboxSettings ──────────────────────────────────────────────────

/// Sandbox VM configuration for automatic lifecycle management.
///
/// When set via `SageEngineBuilder::sandbox()`, each `run()` call will:
/// 1. Create a sandbox VM with these settings
/// 2. Use `SandboxBackend` for all tool I/O
/// 3. Stop the VM when the agent loop completes
#[derive(Debug, Clone)]
pub struct SandboxSettings {
    pub cpus: u32,
    pub memory_mib: u32,
    pub volumes: Vec<sage_sandbox::VolumeMount>,
    pub network_enabled: bool,
    /// Security enforcement config passed to the guest agent.
    /// When set, the guest applies seccomp, landlock, and resource limits.
    pub security: Option<sage_protocol::GuestSecurityConfig>,
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            cpus: 1,
            memory_mib: 512,
            volumes: Vec::new(),
            network_enabled: false,
            security: None,
        }
    }
}

// ── Error type ────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SageError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("model resolution failed: {0}")]
    ModelResolution(String),

    #[error("agent loop error: {0}")]
    AgentLoop(#[from] AgentLoopError),

    #[error("sandbox error: {0}")]
    Sandbox(String),

    #[error("agent timed out after {0}s")]
    Timeout(u64),
}

// ── resolve_or_construct_model ────────────────────────────────────────

/// Resolve or construct a Model.
/// Tries the built-in catalog first; on miss, constructs from provided fields.
pub fn resolve_or_construct_model(
    provider: &str,
    model_id: &str,
    max_tokens: u32,
    base_url: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<Model, SageError> {
    // 1. Try built-in catalog
    if let Some(mut model) = llm::models::resolve_model(provider, model_id) {
        // Apply overrides on catalog hit
        model.max_tokens = max_tokens;
        if let Some(url) = base_url {
            model.base_url = url.to_string();
        }
        if let Some(env) = api_key_env {
            model.api_key_env = env.to_string();
        }
        return Ok(model);
    }

    // 2. Catalog miss — construct from provided fields
    let url = base_url.ok_or_else(|| {
        SageError::ModelResolution(format!(
            "model '{model_id}' not in catalog; base_url required"
        ))
    })?;

    let mut model = custom_model(provider, model_id, max_tokens, Some(url), api_key_env);
    if model.api_key_env.is_empty() {
        model.api_key_env = llm::keys::api_key_env_var(provider);
    }
    Ok(model)
}

/// Construct a minimal Model for custom/injected providers.
/// Shares defaults with `resolve_or_construct_model`'s catalog-miss path.
fn custom_model(
    provider: &str,
    model_id: &str,
    max_tokens: u32,
    base_url: Option<&str>,
    api_key_env: Option<&str>,
) -> Model {
    let id = if model_id.is_empty() {
        "custom"
    } else {
        model_id
    };
    let prov = if provider.is_empty() {
        "custom"
    } else {
        provider
    };
    Model {
        id: id.into(),
        name: id.into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: prov.into(),
        base_url: base_url.unwrap_or_default().to_string(),
        api_key_env: api_key_env.unwrap_or_default().to_string(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens,
        context_window: 128000,
        cost: ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: Some(ProviderCompat::default()),
    }
}

// ── RoutingProvider ───────────────────────────────────────────────────

/// Routes to the correct `ApiProvider` based on `model.api` field.
/// Uses the global `llm::registry` to resolve Anthropic, Google, OpenAI, etc.
struct RoutingProvider;

#[async_trait::async_trait]
impl LlmProvider for RoutingProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        match llm::registry::get_provider(&model.api) {
            Some(provider) => {
                let options = llm::registry::StreamOptions {
                    max_tokens: Some(context.max_tokens),
                    temperature: context.temperature,
                    // Enable prompt caching by default. Providers that support it
                    // (Anthropic: cache_control blocks; OpenAI Responses: prompt_cache_key)
                    // will apply it; others silently ignore this field.
                    cache_retention: Some(llm::registry::CacheRetention::Standard),
                    ..Default::default()
                };
                provider.stream(model, context, tools, &options).await
            }
            None => {
                vec![
                    AssistantMessageEvent::Error(format!(
                        "No provider registered for API: {}",
                        model.api
                    )),
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::Error,
                    },
                ]
            }
        }
    }
}

// ── ArcProvider wrapper ───────────────────────────────────────────────

/// Wraps `Arc<dyn LlmProvider>` into `Box<dyn LlmProvider>` for Agent::new.
struct ArcProvider(Arc<dyn LlmProvider>);

#[async_trait::async_trait]
impl LlmProvider for ArcProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        self.0.complete(model, context, tools).await
    }
}

// ── ChannelSink ───────────────────────────────────────────────────────

/// Bridges AgentEventSink → EventSender channel.
struct ChannelSink {
    sender: EventSender<AgentEvent, Vec<AgentMessage>>,
}

#[async_trait::async_trait]
impl AgentEventSink for ChannelSink {
    async fn emit(&self, event: AgentEvent) {
        if self.sender.send(event).is_err() {
            tracing::debug!("event channel closed — receiver dropped");
        }
    }
}

// ── SageEngine ────────────────────────────────────────────────────────

/// Sage execution engine — holds atomic config fields, creates a fresh
/// Agent instance on each `run()` call.
pub struct SageEngine {
    // Agent config
    name: String,
    system_prompt: String,
    max_turns: usize,
    timeout_secs: Option<u64>,
    tool_execution_mode: ToolExecutionMode,
    tool_policy: Option<ToolPolicy>,

    // Tools
    builtin_tool_names: Vec<String>,
    extra_tools: Vec<Arc<dyn AgentTool>>,
    backend: Option<Arc<dyn ToolBackend>>,

    // LLM
    provider_name: String,
    model_id: String,
    max_tokens: u32,
    base_url: Option<String>,
    api_key_env: Option<String>,
    custom_llm_provider: Option<Arc<dyn LlmProvider>>,

    // Sandbox
    sandbox_settings: Option<SandboxSettings>,

    // Hooks
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
    transform_context_hook: Option<Arc<dyn TransformContextHook>>,
    stop_hook: Option<Arc<dyn StopHook>>,

    // Context budget override
    context_budget: Option<crate::compaction::ContextBudget>,
}

impl std::fmt::Debug for SageEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SageEngine")
            .field("system_prompt", &self.system_prompt)
            .field("provider_name", &self.provider_name)
            .field("model_id", &self.model_id)
            .field("max_turns", &self.max_turns)
            .field("timeout_secs", &self.timeout_secs)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

impl SageEngine {
    pub fn builder() -> SageEngineBuilder {
        SageEngineBuilder {
            name: None,
            system_prompt: None,
            max_turns: None,
            timeout_secs: None,
            tool_execution_mode: None,
            tool_policy: None,
            builtin_tool_names: Vec::new(),
            extra_tools: Vec::new(),
            backend: None,
            sandbox_settings: None,
            provider_name: None,
            model_id: None,
            max_tokens: None,
            base_url: None,
            api_key_env: None,
            custom_llm_provider: None,
            before_hook: None,
            after_hook: None,
            transform_context_hook: None,
            stop_hook: None,
            context_budget: None,
        }
    }

    /// Execute the agent loop, returning an event receiver.
    ///
    /// Each call creates a fresh Agent and ToolRegistry. The LLM provider
    /// is shared via `Arc` when injected with `.llm_provider()`, or a new
    /// `RoutingProvider` is created for catalog/constructed models.
    pub async fn run(
        &self,
        message: &str,
    ) -> Result<EventReceiver<AgentEvent, Vec<AgentMessage>>, SageError> {
        // 1. Resolve model + provider (single pass, no double-resolve)
        let (model, provider): (Model, Arc<dyn LlmProvider>) = match &self.custom_llm_provider {
            Some(p) => {
                let model = custom_model(
                    &self.provider_name,
                    &self.model_id,
                    self.max_tokens,
                    self.base_url.as_deref(),
                    self.api_key_env.as_deref(),
                );
                (model, Arc::clone(p))
            }
            None => {
                let model = resolve_or_construct_model(
                    &self.provider_name,
                    &self.model_id,
                    self.max_tokens,
                    self.base_url.as_deref(),
                    self.api_key_env.as_deref(),
                )?;
                // Register builtin ApiProviders once per process
                static PROVIDERS_INIT: std::sync::Once = std::sync::Once::new();
                PROVIDERS_INIT.call_once(llm::register_builtin_providers);
                (model, Arc::new(RoutingProvider))
            }
        };

        // 2. Resolve backend — sandbox (auto-lifecycle) > explicit backend > local
        let sandbox_handle: Option<Arc<sage_sandbox::SandboxHandle>> = if let Some(ref settings) =
            self.sandbox_settings
        {
            if settings.network_enabled {
                return Err(SageError::Sandbox(
                    "network-enabled sandbox is not implemented".into(),
                ));
            }

            let mut sb = sage_sandbox::SandboxBuilder::new(format!("sage-{}", std::process::id()))
                .cpus(settings.cpus)
                .memory_mib(settings.memory_mib);
            for vol in &settings.volumes {
                sb = sb.mount(&vol.host_path, &vol.guest_path, vol.read_only);
            }
            if let Some(ref sec) = settings.security {
                sb = sb.security(sec.clone());
            }
            let handle = sb
                .create()
                .await
                .map_err(|e| SageError::Sandbox(e.to_string()))?;
            Some(Arc::new(handle))
        } else {
            None
        };

        let backend: Arc<dyn ToolBackend> = if let Some(ref handle) = sandbox_handle {
            SandboxBackend::new(Arc::clone(handle))
        } else {
            self.backend.clone().unwrap_or_else(|| LocalBackend::new())
        };
        let mut registry = ToolRegistry::new();
        for name in &self.builtin_tool_names {
            if let Some(tool) = tools::create_tool(name, backend.clone()) {
                registry.register(tool);
            } else {
                tracing::warn!(tool_name = %name, "unknown builtin tool name — skipped");
            }
        }
        for tool in &self.extra_tools {
            registry.register(Box::new(ArcTool(Arc::clone(tool))));
        }

        // 3. AgentLoopConfig — apply ContextBudget override if provided
        let mut compaction = crate::compaction::CompactionSettings::default();
        if let Some(ref budget) = self.context_budget {
            budget.apply_to(&mut compaction);
        }
        let loop_config = AgentLoopConfig {
            name: self.name.clone(),
            model,
            system_prompt: self.system_prompt.clone(),
            max_turns: self.max_turns,
            tool_execution_mode: self.tool_execution_mode,
            tool_policy: self.tool_policy.clone(),
            compaction,
        };

        // 4. Create Agent
        let mut agent = Agent::new(loop_config, Box::new(ArcProvider(provider)), registry);

        // 5. Set hooks
        if let Some(ref hook) = self.before_hook {
            agent.set_before_tool_call(Box::new(ArcBeforeHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.after_hook {
            agent.set_after_tool_call(Box::new(ArcAfterHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.transform_context_hook {
            agent.set_transform_context(Box::new(ArcTransformContextHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.stop_hook {
            agent.set_stop_hook(Box::new(ArcStopHook(Arc::clone(hook))));
        }

        // 6. Steer initial message
        agent.steer(AgentMessage::User(UserMessage::from_text(message)));

        // 7. Create EventStream + spawn agent loop
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();
        let sink = ChannelSink {
            sender: sender.clone(),
        };
        let timeout_secs = self.timeout_secs;
        tokio::spawn(async move {
            let result = match timeout_secs {
                Some(timeout_secs) => {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        run_agent_loop(&mut agent, &sink),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            sink.emit(AgentEvent::RunError {
                                error: format!("agent run timed out after {timeout_secs}s"),
                            })
                            .await;

                            if let Some(handle) = sandbox_handle
                                && let Err(e) = handle.stop().await
                            {
                                tracing::error!(error = %e, "failed to stop sandbox");
                            }
                            sender.end(vec![]);
                            return;
                        }
                    }
                }
                None => run_agent_loop(&mut agent, &sink).await,
            };

            // Stop sandbox VM if one was created for this run
            if let Some(handle) = sandbox_handle
                && let Err(e) = handle.stop().await
            {
                tracing::error!(error = %e, "failed to stop sandbox");
            }
            match result {
                Ok(messages) => sender.end(messages),
                Err(e) => {
                    tracing::error!(error = %e, "agent loop failed");
                    sink.emit(AgentEvent::RunError {
                        error: e.to_string(),
                    })
                    .await;
                    sender.end(vec![]);
                }
            }
        });

        Ok(receiver)
    }

    pub fn sandbox_settings(&self) -> Option<&SandboxSettings> {
        self.sandbox_settings.as_ref()
    }

    pub fn timeout_secs(&self) -> Option<u64> {
        self.timeout_secs
    }

    /// Create a stateful [`SageSession`] that persists conversation history
    /// across multiple [`SageSession::send`] calls.
    ///
    /// Unlike [`run`], which creates a fresh agent for every invocation,
    /// `session()` builds the agent once. This is the entry point for daemon
    /// and interactive chat modes where the agent must remember prior turns.
    ///
    /// Sandbox lifecycle is not managed by the session — pass `dev: true` via
    /// [`build_engine_for_agent`] (in `sage-cli`) to skip VM creation.
    pub async fn session(&self) -> Result<SageSession, SageError> {
        // 1. Resolve model + provider (same logic as run())
        let (model, provider): (Model, Arc<dyn LlmProvider>) = match &self.custom_llm_provider {
            Some(p) => {
                let model = custom_model(
                    &self.provider_name,
                    &self.model_id,
                    self.max_tokens,
                    self.base_url.as_deref(),
                    self.api_key_env.as_deref(),
                );
                (model, Arc::clone(p))
            }
            None => {
                let model = resolve_or_construct_model(
                    &self.provider_name,
                    &self.model_id,
                    self.max_tokens,
                    self.base_url.as_deref(),
                    self.api_key_env.as_deref(),
                )?;
                static PROVIDERS_INIT: std::sync::Once = std::sync::Once::new();
                PROVIDERS_INIT.call_once(llm::register_builtin_providers);
                (model, Arc::new(RoutingProvider))
            }
        };

        // 2. Use direct backend (sessions don't manage sandbox lifecycle)
        let backend: Arc<dyn ToolBackend> =
            self.backend.clone().unwrap_or_else(|| LocalBackend::new());
        let mut registry = ToolRegistry::new();
        for name in &self.builtin_tool_names {
            if let Some(tool) = tools::create_tool(name, backend.clone()) {
                registry.register(tool);
            } else {
                tracing::warn!(tool_name = %name, "unknown builtin tool name — skipped");
            }
        }
        for tool in &self.extra_tools {
            registry.register(Box::new(ArcTool(Arc::clone(tool))));
        }

        // 3. AgentLoopConfig
        let mut compaction = crate::compaction::CompactionSettings::default();
        if let Some(ref budget) = self.context_budget {
            budget.apply_to(&mut compaction);
        }
        let loop_config = AgentLoopConfig {
            name: self.name.clone(),
            model,
            system_prompt: self.system_prompt.clone(),
            max_turns: self.max_turns,
            tool_execution_mode: self.tool_execution_mode,
            tool_policy: self.tool_policy.clone(),
            compaction,
        };

        // 4. Create Agent
        let mut agent = Agent::new(loop_config, Box::new(ArcProvider(provider)), registry);

        // 5. Set hooks
        if let Some(ref hook) = self.before_hook {
            agent.set_before_tool_call(Box::new(ArcBeforeHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.after_hook {
            agent.set_after_tool_call(Box::new(ArcAfterHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.transform_context_hook {
            agent.set_transform_context(Box::new(ArcTransformContextHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.stop_hook {
            agent.set_stop_hook(Box::new(ArcStopHook(Arc::clone(hook))));
        }

        // S6.2a: construct lifecycle state for HookEvent emission.
        let session_id = generate_session_id();
        let agent_name = self.name.clone();
        let model_id = self.model_id.clone();
        // Replay-enabled bus: SessionStart is cached so the caller's
        // `session.hook_bus().subscribe()` (which necessarily happens after
        // session() returns) still observes the start event. This replaces
        // the earlier spawn+yield hack that gambled on task ordering.
        let hook_bus = HookBus::with_session_start_replay(256);
        let started_at = Instant::now();

        // Emit SessionStart BEFORE constructing SageSession so that if any
        // later clone / allocation on the construction path panics, Drop
        // won't emit a "SessionEnd without matching SessionStart" to
        // downstream telemetry.
        hook_bus.emit(HookEvent::SessionStart {
            session_id: session_id.clone(),
            agent_name: agent_name.clone(),
            model: model_id.clone(),
        });

        Ok(SageSession {
            agent,
            timeout_secs: self.timeout_secs,
            session_id,
            agent_name,
            model_id,
            hook_bus,
            started_at,
            turn_count: 0,
            closed: false,
        })
    }
}

/// Generate a compact hex session identifier for a [`SageSession`].
///
/// Mirrors the ID scheme used by the agent loop so that correlated
/// `AgentEvent` and `HookEvent` streams share compatible identifiers.
fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{ms:013x}")
}

// ── SageSession ───────────────────────────────────────────────────────

/// Stateful multi-turn agent session.
///
/// Obtained via [`SageEngine::session`]. The agent retains its full
/// conversation history across successive [`send`][SageSession::send] calls,
/// enabling contextual follow-up without re-sending prior turns.
pub struct SageSession {
    agent: Agent,
    timeout_secs: Option<u64>,
    session_id: String,
    // S6.2a: fields consumed by HookEvent payloads once emission is wired.
    #[allow(dead_code)]
    agent_name: String,
    #[allow(dead_code)]
    model_id: String,
    hook_bus: HookBus,
    started_at: Instant,
    /// Incremented at the start of each [`SageSession::send`] call.
    ///
    /// Semantic contract: this tracks **attempted** turns, not successful
    /// ones. A `send()` that returns an error still counts — the intent is
    /// "how many turns were attempted", not "how many completed". Consumers
    /// that need success-only counts should subscribe to `TurnEnd` events
    /// via the session's [`HookBus`]. `SessionEnd.turn_count` reports this
    /// field's value at session close (or drop) time.
    turn_count: u32,
    /// Set to true by `close()` so `Drop` does not double-emit SessionEnd.
    closed: bool,
}

impl SageSession {
    /// Steer a user message and run the agent loop, streaming events to `sink`.
    ///
    /// The agent's message history is preserved on return, so a subsequent
    /// `send()` inherits the full conversation context.
    pub async fn send(
        &mut self,
        message: &str,
        sink: &dyn AgentEventSink,
    ) -> Result<(), SageError> {
        // Counted at send() entry (not after completion) so SessionEnd.turn_count
        // reflects user-visible turns regardless of mid-loop failures. A send()
        // that returns an error still counts — it's "how many turns were
        // attempted", not "how many succeeded". Telemetry consumers that need
        // success-only counts should subscribe to TurnEnd events directly.
        self.turn_count += 1;
        self.agent
            .steer(AgentMessage::User(UserMessage::from_text(message)));
        match self.timeout_secs {
            Some(secs) => tokio::time::timeout(
                std::time::Duration::from_secs(secs),
                run_agent_loop(&mut self.agent, sink),
            )
            .await
            .map_err(|_| SageError::Timeout(secs))?
            .map(|_| ())
            .map_err(SageError::AgentLoop),
            None => run_agent_loop(&mut self.agent, sink)
                .await
                .map(|_| ())
                .map_err(SageError::AgentLoop),
        }
    }

    /// Clear all conversation history — the next `send()` starts fresh.
    pub fn reset(&mut self) {
        self.agent.messages_mut().clear();
    }

    /// Read-only view of the current conversation history.
    pub fn messages(&self) -> &[AgentMessage] {
        self.agent.messages()
    }

    /// Borrow the session's [`HookBus`].
    ///
    /// External observers subscribe here to receive lifecycle [`HookEvent`]s
    /// (`SessionStart`, `SessionEnd`, `PreCompact`, `PostCompact`, ...).
    ///
    /// [`HookEvent`]: crate::hook::HookEvent
    pub fn hook_bus(&self) -> &HookBus {
        &self.hook_bus
    }

    /// Stable session identifier — shared across all `HookEvent`s emitted by
    /// this session. Matches the scheme used by the agent loop so logs can be
    /// correlated.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Explicitly close the session and emit `HookEvent::SessionEnd`.
    ///
    /// Consumes the session — any further interaction must happen before the
    /// call. `success` is propagated verbatim into the emitted event. When a
    /// session is dropped without calling `close`, no `SessionEnd` is emitted
    /// and a warning is logged.
    pub async fn close(mut self, success: bool) -> Result<(), SageError> {
        let duration_ms =
            u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.hook_bus.emit(HookEvent::SessionEnd {
            session_id: self.session_id.clone(),
            duration_ms,
            turn_count: self.turn_count,
            success,
        });
        self.closed = true;
        Ok(())
    }
}

impl Drop for SageSession {
    fn drop(&mut self) {
        if !self.closed {
            // Session dropped without explicit close() — treat as unclean
            // failure. SessionEnd still fires so telemetry sees every session
            // terminate (with success=false + warn log so operators know why).
            let duration_ms =
                u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.hook_bus.emit(HookEvent::SessionEnd {
                session_id: self.session_id.clone(),
                duration_ms,
                turn_count: self.turn_count,
                success: false,
            });
            tracing::warn!(
                session_id = %self.session_id,
                "SageSession dropped without close() — SessionEnd emitted with success=false"
            );
        }
    }
}

// ── Arc wrappers for hooks and tools ──────────────────────────────────

struct ArcBeforeHook(Arc<dyn BeforeToolCallHook>);

#[async_trait::async_trait]
impl BeforeToolCallHook for ArcBeforeHook {
    async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
        self.0.before_tool_call(ctx).await
    }
}

struct ArcAfterHook(Arc<dyn AfterToolCallHook>);

#[async_trait::async_trait]
impl AfterToolCallHook for ArcAfterHook {
    async fn after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult {
        self.0.after_tool_call(ctx).await
    }
}

struct ArcTransformContextHook(Arc<dyn TransformContextHook>);

#[async_trait::async_trait]
impl TransformContextHook for ArcTransformContextHook {
    async fn transform_context(&self, messages: &mut Vec<AgentMessage>) {
        self.0.transform_context(messages).await
    }
}

struct ArcStopHook(Arc<dyn StopHook>);

#[async_trait::async_trait]
impl StopHook for ArcStopHook {
    async fn on_stop(&self, ctx: &StopContext) -> StopAction {
        self.0.on_stop(ctx).await
    }
}

/// Wraps `Arc<dyn AgentTool>` so it can be registered as `Box<dyn AgentTool>`.
struct ArcTool(Arc<dyn AgentTool>);

#[async_trait::async_trait]
impl AgentTool for ArcTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.0.parameters_schema()
    }
    async fn execute(&self, args: serde_json::Value) -> crate::tools::ToolOutput {
        self.0.execute(args).await
    }
}

// ── SageEngineBuilder ─────────────────────────────────────────────────

pub struct SageEngineBuilder {
    name: Option<String>,
    system_prompt: Option<String>,
    max_turns: Option<usize>,
    timeout_secs: Option<u64>,
    tool_execution_mode: Option<ToolExecutionMode>,
    tool_policy: Option<ToolPolicy>,
    builtin_tool_names: Vec<String>,
    extra_tools: Vec<Arc<dyn AgentTool>>,
    backend: Option<Arc<dyn ToolBackend>>,
    sandbox_settings: Option<SandboxSettings>,
    provider_name: Option<String>,
    model_id: Option<String>,
    max_tokens: Option<u32>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    custom_llm_provider: Option<Arc<dyn LlmProvider>>,
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
    transform_context_hook: Option<Arc<dyn TransformContextHook>>,
    stop_hook: Option<Arc<dyn StopHook>>,
    context_budget: Option<crate::compaction::ContextBudget>,
}

impl SageEngineBuilder {
    // ── Agent config ──

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }

    /// Set the system prompt from a [`SystemPromptBuilder`].
    ///
    /// All sections are joined into a flat `String` (sections separated by `"\n\n"`).
    /// **The `cacheable` flag on individual sections is not propagated to the provider** —
    /// cache_control injection happens at the provider layer based on `StreamOptions`,
    /// not per-section. The builder is useful for structured composition; caching is
    /// controlled separately via `StreamOptions::cache_retention`.
    pub fn system_prompt_builder(
        mut self,
        builder: crate::system_prompt::SystemPromptBuilder,
    ) -> Self {
        self.system_prompt = Some(builder.build().to_string());
        self
    }

    pub fn max_turns(mut self, n: usize) -> Self {
        self.max_turns = Some(n);
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    pub fn tool_execution_mode(mut self, mode: ToolExecutionMode) -> Self {
        self.tool_execution_mode = Some(mode);
        self
    }

    pub fn tool_policy(mut self, policy: ToolPolicy) -> Self {
        self.tool_policy = Some(policy);
        self
    }

    // ── Tools ──

    pub fn builtin_tools(mut self, names: &[&str]) -> Self {
        self.builtin_tool_names = names.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn register_tool(mut self, tool: impl AgentTool + 'static) -> Self {
        self.extra_tools.push(Arc::new(tool));
        self
    }

    /// Set a custom `ToolBackend` for tool I/O operations.
    ///
    /// When set, all builtin tools delegate I/O to this backend instead of
    /// the default `LocalBackend`. Use `SandboxBackend::new(handle)` to
    /// execute tools inside a microVM.
    pub fn backend(mut self, backend: Arc<dyn ToolBackend>) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Enable sandbox mode with automatic VM lifecycle management.
    ///
    /// When set, each `run()` call creates a sandbox VM and uses
    /// `SandboxBackend` for all tool I/O.  The VM is stopped when the
    /// agent loop completes.
    ///
    /// This is mutually exclusive with `.backend()` — if both are set,
    /// `.sandbox()` takes precedence.
    pub fn sandbox(mut self, settings: SandboxSettings) -> Self {
        self.sandbox_settings = Some(settings);
        self
    }

    // ── LLM ──

    pub fn provider(mut self, provider: &str) -> Self {
        self.provider_name = Some(provider.to_string());
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model_id = Some(model.to_string());
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    pub fn base_url(mut self, url: &str) -> Self {
        self.base_url = Some(url.to_string());
        self
    }

    pub fn api_key_env(mut self, env_var: &str) -> Self {
        self.api_key_env = Some(env_var.to_string());
        self
    }

    pub fn llm_provider(mut self, provider: impl LlmProvider + 'static) -> Self {
        self.custom_llm_provider = Some(Arc::new(provider));
        self
    }

    // ── Hooks ──

    pub fn on_before_tool_call(mut self, hook: impl BeforeToolCallHook + 'static) -> Self {
        self.before_hook = Some(Arc::new(hook));
        self
    }

    pub fn on_after_tool_call(mut self, hook: impl AfterToolCallHook + 'static) -> Self {
        self.after_hook = Some(Arc::new(hook));
        self
    }

    /// Set a hook called before each LLM call to transform the message history.
    ///
    /// Use this to inject memory, filter sensitive content, or apply custom compression
    /// strategies (e.g. retrieval-augmented memory, project-context injection).
    pub fn on_transform_context(mut self, hook: impl TransformContextHook + 'static) -> Self {
        self.transform_context_hook = Some(Arc::new(hook));
        self
    }

    /// Set a hook called when the agent is about to stop.
    ///
    /// Returning [`StopAction::Continue`] injects feedback and restarts the loop —
    /// the core mechanism for the Harness evaluator-driven iteration.
    pub fn on_stop(mut self, hook: impl StopHook + 'static) -> Self {
        self.stop_hook = Some(Arc::new(hook));
        self
    }

    /// Set an explicit context budget. Overrides the per-field thresholds in
    /// the default [`CompactionSettings`] with computed values.
    pub fn context_budget(mut self, budget: crate::compaction::ContextBudget) -> Self {
        self.context_budget = Some(budget);
        self
    }

    // ── Build ──

    pub fn build(self) -> Result<SageEngine, SageError> {
        let system_prompt = self
            .system_prompt
            .ok_or(SageError::MissingField("system_prompt"))?;

        // Must have either custom_llm_provider or (provider_name + model_id)
        if self.custom_llm_provider.is_none()
            && (self.provider_name.is_none() || self.model_id.is_none())
        {
            return Err(SageError::MissingField("provider+model or llm_provider"));
        }

        // Extract provider_name before moving self.custom_llm_provider into the struct,
        // so the logic doesn't depend on field initialization order.
        let provider_name = self.provider_name.unwrap_or_else(|| {
            if self.custom_llm_provider.is_some() {
                "custom".into()
            } else {
                String::new()
            }
        });

        Ok(SageEngine {
            name: self.name.unwrap_or_default(),
            system_prompt,
            max_turns: self.max_turns.unwrap_or(10),
            timeout_secs: self.timeout_secs.filter(|secs| *secs > 0),
            tool_execution_mode: self
                .tool_execution_mode
                .unwrap_or(ToolExecutionMode::Parallel),
            tool_policy: self.tool_policy,
            builtin_tool_names: self.builtin_tool_names,
            extra_tools: self.extra_tools,
            backend: self.backend,
            sandbox_settings: self.sandbox_settings,
            provider_name,
            model_id: self.model_id.unwrap_or_else(|| "custom".to_string()),
            max_tokens: self.max_tokens.unwrap_or(4096),
            base_url: self.base_url,
            api_key_env: self.api_key_env,
            custom_llm_provider: self.custom_llm_provider,
            before_hook: self.before_hook,
            after_hook: self.after_hook,
            transform_context_hook: self.transform_context_hook,
            stop_hook: self.stop_hook,
            context_budget: self.context_budget,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AfterToolCallHook, BeforeToolCallHook};
    use crate::test_helpers::StatefulProvider;
    use crate::tools::{AgentTool, ToolOutput};
    use serial_test::serial;

    // ── Helpers ───────────────────────────────────────────────────

    /// Create a StatefulProvider that returns a single text response then stops.
    fn simple_provider(text: &str) -> StatefulProvider {
        StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta(text.to_string()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]])
    }

    /// Create a StatefulProvider that makes a tool call, then responds with text.
    fn tool_call_provider(tool_name: &str, tool_args: &str) -> StatefulProvider {
        StatefulProvider::new(vec![
            // Turn 1: tool call
            vec![
                AssistantMessageEvent::ToolCallStart {
                    id: "tc-1".into(),
                    name: tool_name.into(),
                },
                AssistantMessageEvent::ToolCallDelta {
                    id: "tc-1".into(),
                    arguments_delta: tool_args.into(),
                },
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            // Turn 2: text response
            vec![
                AssistantMessageEvent::TextDelta("done".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ])
    }

    /// Minimal custom tool for testing.
    struct EchoTool;

    #[async_trait::async_trait]
    impl AgentTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"input": {"type": "string"}}})
        }
        async fn execute(&self, args: serde_json::Value) -> ToolOutput {
            let input = args
                .get("input")
                .and_then(|v| v.as_str())
                .unwrap_or("no input");
            ToolOutput {
                content: vec![Content::Text {
                    text: format!("echo: {input}"),
                }],
                is_error: false,
            }
        }
    }

    /// Extract text from a Content slice (test helper to avoid repetition).
    fn text_of(content: &[Content]) -> String {
        content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

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

    struct NoopAfterHook;

    #[async_trait::async_trait]
    impl AfterToolCallHook for NoopAfterHook {
        async fn after_tool_call(&self, _ctx: &AfterToolCallContext) -> AfterToolCallResult {
            AfterToolCallResult {
                content: None,
                is_error: None,
            }
        }
    }

    // =================================================================
    // Builder tests (~9)
    // =================================================================

    #[test]
    fn builder_default_values() {
        let b = SageEngine::builder();
        assert!(b.system_prompt.is_none());
        assert!(b.max_turns.is_none());
        assert!(b.timeout_secs.is_none());
        assert!(b.tool_execution_mode.is_none());
        assert!(b.tool_policy.is_none());
        assert!(b.builtin_tool_names.is_empty());
        assert!(b.extra_tools.is_empty());
        assert!(b.sandbox_settings.is_none());
        assert!(b.provider_name.is_none());
        assert!(b.model_id.is_none());
        assert!(b.max_tokens.is_none());
        assert!(b.base_url.is_none());
        assert!(b.api_key_env.is_none());
        assert!(b.custom_llm_provider.is_none());
        assert!(b.before_hook.is_none());
        assert!(b.after_hook.is_none());
    }

    #[test]
    fn builder_minimal_build_succeeds() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .build();
        assert!(engine.is_ok());
        let e = engine.unwrap();
        assert_eq!(e.system_prompt, "test");
        assert_eq!(e.max_turns, 10);
        assert_eq!(e.max_tokens, 4096);
        assert_eq!(e.tool_execution_mode, ToolExecutionMode::Parallel);
    }

    #[test]
    fn builder_missing_system_prompt_fails() {
        let result = SageEngine::builder()
            .provider("test")
            .model("test-model")
            .build();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("system_prompt"),
            "error should mention system_prompt: {}",
            err
        );
    }

    #[test]
    fn builder_missing_provider_and_llm_provider_fails() {
        let result = SageEngine::builder().system_prompt("test").build();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("provider+model or llm_provider"),
            "error should mention provider: {}",
            err
        );
    }

    #[test]
    fn builder_llm_provider_without_provider_name_succeeds() {
        let provider = simple_provider("hello");
        let result = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(provider)
            .build();
        assert!(result.is_ok());
        let e = result.unwrap();
        assert_eq!(e.model_id, "custom");
    }

    #[test]
    fn builder_register_tool() {
        let result = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .register_tool(EchoTool)
            .build();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().extra_tools.len(), 1);
    }

    #[test]
    fn builder_hooks_registered() {
        let result = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .on_before_tool_call(BlockAllHook)
            .on_after_tool_call(NoopAfterHook)
            .build();
        assert!(result.is_ok());
        let e = result.unwrap();
        assert!(e.before_hook.is_some());
        assert!(e.after_hook.is_some());
    }

    #[test]
    fn builder_multiple_tools() {
        let result = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .register_tool(EchoTool)
            .register_tool(EchoTool) // second instance
            .build();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().extra_tools.len(), 2);
    }

    #[test]
    fn builder_chaining() {
        let result = SageEngine::builder()
            .system_prompt("You are a test agent")
            .provider("qwen")
            .model("qwen-plus")
            .max_turns(5)
            .max_tokens(8192)
            .tool_execution_mode(ToolExecutionMode::Sequential)
            .builtin_tools(&["bash", "read"])
            .register_tool(EchoTool)
            .base_url("http://custom.api")
            .api_key_env("MY_KEY")
            .on_before_tool_call(BlockAllHook)
            .on_after_tool_call(NoopAfterHook)
            .build();
        assert!(result.is_ok());
        let e = result.unwrap();
        assert_eq!(e.system_prompt, "You are a test agent");
        assert_eq!(e.provider_name, "qwen");
        assert_eq!(e.model_id, "qwen-plus");
        assert_eq!(e.max_turns, 5);
        assert_eq!(e.max_tokens, 8192);
        assert_eq!(e.tool_execution_mode, ToolExecutionMode::Sequential);
        assert_eq!(e.builtin_tool_names, vec!["bash", "read"]);
        assert_eq!(e.extra_tools.len(), 1);
        assert_eq!(e.base_url.as_deref(), Some("http://custom.api"));
        assert_eq!(e.api_key_env.as_deref(), Some("MY_KEY"));
        assert!(e.before_hook.is_some());
        assert!(e.after_hook.is_some());
    }

    // =================================================================
    // Engine run tests (~9)
    // =================================================================

    #[tokio::test]
    async fn run_emits_agent_start_and_end() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(simple_provider("hello"))
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = rx.next().await {
            events.push(event);
        }

        assert!(matches!(events.first(), Some(AgentEvent::AgentStart)));
        assert!(matches!(events.last(), Some(AgentEvent::AgentEnd { .. })));
    }

    #[tokio::test]
    async fn run_stream_terminates() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(simple_provider("hello"))
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut count = 0;
        while let Some(_) = rx.next().await {
            count += 1;
        }
        // Stream must eventually return None
        assert!(count > 0);
        assert!(rx.next().await.is_none());
    }

    #[tokio::test]
    async fn run_returns_final_messages() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(simple_provider("hello world"))
            .build()
            .unwrap();

        let rx = engine.run("hi").await.unwrap();
        let messages = rx.result().await;
        assert!(!messages.is_empty(), "should return at least one message");
    }

    #[tokio::test]
    async fn run_with_tool_calls() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider("echo", r#"{"input":"test"}"#))
            .register_tool(EchoTool)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut has_tool_start = false;
        let mut has_tool_end = false;
        while let Some(event) = rx.next().await {
            match event {
                AgentEvent::ToolExecutionStart { .. } => has_tool_start = true,
                AgentEvent::ToolExecutionEnd { .. } => has_tool_end = true,
                _ => {}
            }
        }
        assert!(has_tool_start, "should have ToolExecutionStart event");
        assert!(has_tool_end, "should have ToolExecutionEnd event");
    }

    #[tokio::test]
    async fn run_multiple_times() {
        let provider = StatefulProvider::new(vec![
            // Run 1
            vec![
                AssistantMessageEvent::TextDelta("response 1".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            // Run 2
            vec![
                AssistantMessageEvent::TextDelta("response 2".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(provider)
            .build()
            .unwrap();

        // First run
        let rx1 = engine.run("first").await.unwrap();
        let msgs1 = rx1.result().await;
        assert!(!msgs1.is_empty());

        // Second run — should not interfere with first
        let rx2 = engine.run("second").await.unwrap();
        let msgs2 = rx2.result().await;
        assert!(!msgs2.is_empty());
    }

    #[tokio::test]
    async fn run_hook_blocks_tool() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider("echo", r#"{"input":"test"}"#))
            .register_tool(EchoTool)
            .on_before_tool_call(BlockAllHook)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut tool_blocked = false;
        let mut tool_succeeded = false;
        while let Some(event) = rx.next().await {
            match &event {
                AgentEvent::ToolExecutionEnd { is_error: true, .. } => tool_blocked = true,
                AgentEvent::ToolExecutionEnd {
                    is_error: false, ..
                } => tool_succeeded = true,
                _ => {}
            }
        }
        assert!(tool_blocked, "blocked tool should emit is_error: true");
        assert!(!tool_succeeded, "no tool should have succeeded");
    }

    #[tokio::test]
    async fn run_max_turns_respected() {
        // Provider always requests tool calls (infinite loop without max_turns)
        let responses: Vec<Vec<AssistantMessageEvent>> = (0..20)
            .map(|i| {
                vec![
                    AssistantMessageEvent::ToolCallStart {
                        id: format!("tc-{i}"),
                        name: "echo".into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: format!("tc-{i}"),
                        arguments_delta: r#"{"input":"loop"}"#.into(),
                    },
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    },
                ]
            })
            .collect();

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(StatefulProvider::new(responses))
            .register_tool(EchoTool)
            .max_turns(3)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut turn_count = 0;
        while let Some(event) = rx.next().await {
            if matches!(event, AgentEvent::TurnStart) {
                turn_count += 1;
            }
        }
        assert!(
            turn_count <= 3,
            "should respect max_turns=3, got {} turns",
            turn_count
        );
    }

    #[tokio::test]
    async fn run_unknown_tool_error() {
        // Provider requests a tool that doesn't exist
        let provider = StatefulProvider::new(vec![
            vec![
                AssistantMessageEvent::ToolCallStart {
                    id: "tc-1".into(),
                    name: "nonexistent_tool".into(),
                },
                AssistantMessageEvent::ToolCallDelta {
                    id: "tc-1".into(),
                    arguments_delta: "{}".into(),
                },
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            }],
        ]);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(provider)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut has_error_tool = false;
        while let Some(event) = rx.next().await {
            if matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. }) {
                has_error_tool = true;
            }
        }
        assert!(has_error_tool, "unknown tool should produce error event");
    }

    #[tokio::test]
    async fn run_custom_tool_executes() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider("echo", r#"{"input":"hello"}"#))
            .register_tool(EchoTool)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut tool_ended_ok = false;
        while let Some(event) = rx.next().await {
            if matches!(
                event,
                AgentEvent::ToolExecutionEnd {
                    is_error: false,
                    ..
                }
            ) {
                tool_ended_ok = true;
            }
        }
        assert!(tool_ended_ok, "custom tool should execute successfully");
    }

    // =================================================================
    // resolve_or_construct_model tests (~3)
    // =================================================================

    #[test]
    fn resolve_catalog_hit() {
        // "deepseek" + "deepseek-chat" is in the built-in catalog
        let result = resolve_or_construct_model("deepseek", "deepseek-chat", 4096, None, None);
        assert!(result.is_ok(), "catalog model should resolve: {:?}", result);
        let model = result.unwrap();
        assert_eq!(model.id, "deepseek-chat");
        assert_eq!(model.provider, "deepseek");
    }

    #[test]
    fn resolve_catalog_miss_with_base_url() {
        let result = resolve_or_construct_model(
            "custom-provider",
            "custom-model",
            8192,
            Some("http://my-api.com/v1"),
            Some("MY_API_KEY"),
        );
        assert!(result.is_ok());
        let model = result.unwrap();
        assert_eq!(model.id, "custom-model");
        assert_eq!(model.provider, "custom-provider");
        assert_eq!(model.base_url, "http://my-api.com/v1");
        assert_eq!(model.api_key_env, "MY_API_KEY");
        assert_eq!(model.max_tokens, 8192);
    }

    #[test]
    fn resolve_catalog_miss_no_base_url_fails() {
        let result =
            resolve_or_construct_model("unknown-provider", "unknown-model", 4096, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("base_url required"),
            "error should mention base_url: {}",
            err
        );
    }

    // =================================================================
    // P2 Integration: multi-turn tool calls (>3 turns)
    // =================================================================

    #[tokio::test]
    async fn run_multi_turn_tool_calls_four_turns() {
        // Provider returns tool calls for 4 consecutive turns, then a final text response.
        // This verifies the agent loop sustains >3 turns without interruption.
        let mut responses: Vec<Vec<AssistantMessageEvent>> = (0..4)
            .map(|i| {
                vec![
                    AssistantMessageEvent::ToolCallStart {
                        id: format!("tc-{i}"),
                        name: "echo".into(),
                    },
                    AssistantMessageEvent::ToolCallDelta {
                        id: format!("tc-{i}"),
                        arguments_delta: format!(r#"{{"input":"turn {i}"}}"#),
                    },
                    AssistantMessageEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    },
                ]
            })
            .collect();
        // Turn 5: final text response (no more tool calls)
        responses.push(vec![
            AssistantMessageEvent::TextDelta("all done".into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(StatefulProvider::new(responses))
            .register_tool(EchoTool)
            .max_turns(10) // plenty of room
            .build()
            .unwrap();

        let mut rx = engine.run("go").await.unwrap();
        let mut turn_count = 0;
        let mut tool_exec_count = 0;
        while let Some(event) = rx.next().await {
            match event {
                AgentEvent::TurnStart => turn_count += 1,
                AgentEvent::ToolExecutionEnd { is_error, .. } => {
                    assert!(!is_error, "tool should succeed on each turn");
                    tool_exec_count += 1;
                }
                _ => {}
            }
        }
        assert_eq!(tool_exec_count, 4, "should execute tools on 4 turns");
        assert_eq!(turn_count, 5, "should have 5 turns total (4 tool + 1 text)");
    }

    // =================================================================
    // P2 Integration: multiple tool calls in a single turn
    // =================================================================

    #[tokio::test]
    async fn run_two_tool_calls_in_single_turn() {
        // Provider returns 2 tool calls in a single LLM response.
        // Both should be dispatched and executed (mode=Parallel uses join_all).
        let provider = StatefulProvider::new(vec![
            // Turn 1: two tool calls
            vec![
                AssistantMessageEvent::ToolCallStart {
                    id: "tc-a".into(),
                    name: "echo".into(),
                },
                AssistantMessageEvent::ToolCallDelta {
                    id: "tc-a".into(),
                    arguments_delta: r#"{"input":"alpha"}"#.into(),
                },
                AssistantMessageEvent::ToolCallStart {
                    id: "tc-b".into(),
                    name: "echo".into(),
                },
                AssistantMessageEvent::ToolCallDelta {
                    id: "tc-b".into(),
                    arguments_delta: r#"{"input":"beta"}"#.into(),
                },
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            // Turn 2: text response
            vec![
                AssistantMessageEvent::TextDelta("done".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(provider)
            .register_tool(EchoTool)
            .tool_execution_mode(ToolExecutionMode::Parallel)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut tool_names: Vec<String> = Vec::new();
        while let Some(event) = rx.next().await {
            if let AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } = event
            {
                assert!(!is_error);
                tool_names.push(tool_name);
            }
        }
        assert_eq!(
            tool_names.len(),
            2,
            "both tool calls in single turn should execute"
        );
        assert!(tool_names.iter().all(|n| n == "echo"));
    }

    // =================================================================
    // P2 Integration: steering queue verification
    // =================================================================

    #[tokio::test]
    async fn run_steering_message_appears_in_events() {
        // Verify that the user message passed to engine.run() is emitted
        // as a MessageStart event (the steering mechanism).
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(simple_provider("response"))
            .build()
            .unwrap();

        let mut rx = engine.run("hello from user").await.unwrap();
        let mut user_messages = Vec::new();
        while let Some(event) = rx.next().await {
            if let AgentEvent::MessageStart {
                message: AgentMessage::User(u),
            } = &event
            {
                user_messages.push(text_of(&u.content));
            }
        }
        assert_eq!(
            user_messages.len(),
            1,
            "should have exactly one user message"
        );
        assert_eq!(user_messages[0], "hello from user");
    }

    #[tokio::test]
    async fn run_steering_message_in_result_messages() {
        // Verify that the steered user message appears in the final result
        // messages (AgentEnd payload), confirming it was processed by the loop.
        let engine = SageEngine::builder()
            .system_prompt("You are a test assistant")
            .llm_provider(simple_provider("ok"))
            .build()
            .unwrap();

        let rx = engine.run("this is the user query").await.unwrap();
        let messages = rx.result().await;
        assert!(
            messages.len() >= 2,
            "should have user + assistant messages, got {}",
            messages.len()
        );
        // First message should be the steered user message
        match &messages[0] {
            AgentMessage::User(u) => {
                assert_eq!(text_of(&u.content), "this is the user query");
            }
            other => panic!("expected User message, got {:?}", other),
        }
    }

    // =================================================================
    // P2 Integration: real LLM (Qwen via DashScope) — gated by env var
    // =================================================================

    #[tokio::test]
    #[ignore] // Run with: DASHSCOPE_API_KEY=... cargo test -- --ignored test_real_qwen
    async fn test_real_qwen_single_turn() {
        // #[ignore] is the single gate — if you run --ignored, the key must be set.
        let engine = SageEngine::builder()
            .system_prompt("You are a helpful assistant. Reply in one short sentence.")
            .provider("qwen")
            .model("qwen-plus")
            .max_tokens(256)
            .max_turns(1)
            .build()
            .unwrap();

        let mut rx = engine
            .run("What is 2+2? Reply with just the number.")
            .await
            .unwrap();
        let mut has_start = false;
        let mut has_end = false;
        while let Some(event) = rx.next().await {
            match &event {
                AgentEvent::AgentStart => has_start = true,
                AgentEvent::AgentEnd { messages } => {
                    has_end = true;
                    assert!(!messages.is_empty(), "should return messages");
                    for msg in messages {
                        if let AgentMessage::Assistant(a) = msg {
                            if let Some(err) = &a.error_message {
                                // API error (expired key, rate limit) — the plumbing works
                                // but the external service rejected us. Skip content check.
                                eprintln!("LLM API error (plumbing OK): {err}");
                                return;
                            }
                            let text = text_of(&a.content);
                            eprintln!("Real LLM response: {text}");
                            assert!(!text.is_empty(), "assistant text should not be empty");
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(has_start, "should emit AgentStart");
        assert!(has_end, "should emit AgentEnd");
    }

    // =================================================================
    // RoutingProvider tests
    // =================================================================

    #[tokio::test]
    #[serial]
    async fn routing_provider_missing_api_returns_error_and_done() {
        let provider = RoutingProvider;
        let model = custom_model("fake", "fake-model", 4096, None, None);
        let context = LlmContext {
            messages: vec![],
            system_prompt: "test".into(),
            max_tokens: 4096,
            temperature: None,
        };
        // RoutingProvider should fail because no ApiProvider is registered for
        // the model's api ("openai-completions" from custom_model default).
        // We clear first to ensure a clean slate, then verify error + Done.
        llm::registry::clear_providers();
        let events = provider.complete(&model, &context, &[]).await;

        assert!(
            events.len() >= 2,
            "should have Error + Done events, got {:?}",
            events
        );
        assert!(
            matches!(&events[0], AssistantMessageEvent::Error(msg) if msg.contains("No provider registered")),
            "first event should be Error: {:?}",
            events[0]
        );
        assert!(
            matches!(
                &events[1],
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Error
                }
            ),
            "second event should be Done with Error stop_reason: {:?}",
            events[1]
        );
        // Re-register so other tests aren't affected
        llm::register_builtin_providers();
    }

    // =================================================================
    // Sandbox settings tests
    // =================================================================

    #[test]
    fn sandbox_settings_default() {
        let s = SandboxSettings::default();
        assert_eq!(s.cpus, 1);
        assert_eq!(s.memory_mib, 512);
        assert!(s.volumes.is_empty());
        assert!(!s.network_enabled);
    }

    #[test]
    fn builder_sandbox_sets_settings() {
        let settings = SandboxSettings {
            cpus: 2,
            memory_mib: 2048,
            volumes: vec![sage_sandbox::VolumeMount {
                host_path: "/host/ws".into(),
                guest_path: "/workspace".into(),
                read_only: false,
            }],
            network_enabled: false,
            security: None,
        };
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .sandbox(settings)
            .build()
            .unwrap();
        let ss = engine.sandbox_settings.as_ref().unwrap();
        assert_eq!(ss.cpus, 2);
        assert_eq!(ss.memory_mib, 2048);
        assert_eq!(ss.volumes.len(), 1);
        assert_eq!(ss.volumes[0].host_path, "/host/ws");
    }

    #[test]
    fn builder_without_sandbox_has_none() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .build()
            .unwrap();
        assert!(engine.sandbox_settings.is_none());
    }

    #[test]
    fn sandbox_settings_with_multiple_volumes() {
        let settings = SandboxSettings {
            cpus: 4,
            memory_mib: 4096,
            volumes: vec![
                sage_sandbox::VolumeMount {
                    host_path: "/host/ws".into(),
                    guest_path: "/workspace".into(),
                    read_only: false,
                },
                sage_sandbox::VolumeMount {
                    host_path: "/host/data".into(),
                    guest_path: "/data".into(),
                    read_only: true,
                },
            ],
            network_enabled: false,
            security: None,
        };
        assert_eq!(settings.volumes.len(), 2);
        assert!(settings.volumes[1].read_only);
    }

    #[test]
    fn sandbox_settings_debug_impl() {
        let settings = SandboxSettings::default();
        // Debug trait must be derived for logging
        let debug = format!("{settings:?}");
        assert!(debug.contains("SandboxSettings"));
        assert!(debug.contains("cpus"));
    }

    // =================================================================
    // Backend injection tests
    // =================================================================

    /// A mock ToolBackend that records all method calls for verification.
    struct TrackingBackend {
        calls: tokio::sync::Mutex<Vec<(String, String)>>,
    }

    impl TrackingBackend {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        async fn get_calls(&self) -> Vec<(String, String)> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl crate::tools::backend::ToolBackend for TrackingBackend {
        async fn shell(
            &self,
            command: &str,
            _timeout_secs: u64,
        ) -> Result<crate::tools::backend::ShellOutput, String> {
            self.calls
                .lock()
                .await
                .push(("shell".into(), command.into()));
            Ok(crate::tools::backend::ShellOutput {
                stdout: "tracked-output\n".into(),
                stderr: String::new(),
                success: true,
            })
        }

        async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
            self.calls
                .lock()
                .await
                .push(("read_file".into(), path.into()));
            Ok(b"tracked-content".to_vec())
        }

        async fn write_file(&self, path: &str, _data: &[u8]) -> Result<(), String> {
            self.calls
                .lock()
                .await
                .push(("write_file".into(), path.into()));
            Ok(())
        }

        async fn list_dir(
            &self,
            path: &str,
        ) -> Result<Vec<crate::tools::backend::DirEntry>, String> {
            self.calls
                .lock()
                .await
                .push(("list_dir".into(), path.into()));
            Ok(vec![crate::tools::backend::DirEntry {
                name: "mock.txt".into(),
                is_dir: false,
                size: 42,
            }])
        }
    }

    #[test]
    fn builder_default_has_no_backend() {
        let b = SageEngine::builder();
        assert!(
            b.backend.is_none(),
            "default builder should have no backend"
        );
    }

    #[test]
    fn builder_timeout_secs_sets_value() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .timeout_secs(42)
            .build()
            .unwrap();

        assert_eq!(engine.timeout_secs(), Some(42));
    }

    #[test]
    fn builder_with_custom_backend() {
        let tracking = TrackingBackend::new();
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .backend(tracking as Arc<dyn crate::tools::backend::ToolBackend>)
            .build()
            .unwrap();
        assert!(
            engine.backend.is_some(),
            "engine should store the custom backend"
        );
    }

    #[test]
    fn builder_without_backend_uses_default() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .provider("test")
            .model("test-model")
            .build()
            .unwrap();
        assert!(
            engine.backend.is_none(),
            "engine without .backend() should have None"
        );
    }

    #[tokio::test]
    async fn run_with_custom_backend_builtin_tool_uses_it() {
        // Provider calls "bash" tool with a command. The TrackingBackend should
        // capture the shell() call instead of executing on the host.
        let tracking = TrackingBackend::new();
        let tracking_ref = Arc::clone(&tracking);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider(
                "bash",
                r#"{"command":"echo hello","timeout":10}"#,
            ))
            .builtin_tools(&["bash"])
            .backend(tracking as Arc<dyn crate::tools::backend::ToolBackend>)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        while let Some(_) = rx.next().await {}

        let calls = tracking_ref.get_calls().await;
        assert!(
            calls.iter().any(|(method, _)| method == "shell"),
            "TrackingBackend.shell() should have been called, got: {:?}",
            calls
        );
        let shell_call = calls.iter().find(|(m, _)| m == "shell").unwrap();
        assert!(
            shell_call.1.contains("echo hello"),
            "shell command should contain 'echo hello', got: {}",
            shell_call.1
        );
    }

    #[tokio::test]
    async fn run_with_custom_backend_read_tool_uses_it() {
        let tracking = TrackingBackend::new();
        let tracking_ref = Arc::clone(&tracking);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider(
                "read",
                r#"{"file_path":"/test/file.txt"}"#,
            ))
            .builtin_tools(&["read"])
            .backend(tracking as Arc<dyn crate::tools::backend::ToolBackend>)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        while let Some(_) = rx.next().await {}

        let calls = tracking_ref.get_calls().await;
        assert!(
            calls.iter().any(|(method, _)| method == "read_file"),
            "TrackingBackend.read_file() should have been called, got: {:?}",
            calls
        );
    }

    #[tokio::test]
    async fn run_with_custom_backend_write_tool_uses_it() {
        let tracking = TrackingBackend::new();
        let tracking_ref = Arc::clone(&tracking);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider(
                "write",
                r#"{"file_path":"/test/out.txt","content":"hello"}"#,
            ))
            .builtin_tools(&["write"])
            .backend(tracking as Arc<dyn crate::tools::backend::ToolBackend>)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        while let Some(_) = rx.next().await {}

        let calls = tracking_ref.get_calls().await;
        assert!(
            calls.iter().any(|(method, _)| method == "write_file"),
            "TrackingBackend.write_file() should have been called, got: {:?}",
            calls
        );
    }

    #[tokio::test]
    async fn run_with_custom_backend_ls_tool_uses_it() {
        let tracking = TrackingBackend::new();
        let tracking_ref = Arc::clone(&tracking);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider("ls", r#"{"path":"/tmp"}"#))
            .builtin_tools(&["ls"])
            .backend(tracking as Arc<dyn crate::tools::backend::ToolBackend>)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        while let Some(_) = rx.next().await {}

        let calls = tracking_ref.get_calls().await;
        assert!(
            calls.iter().any(|(method, _)| method == "list_dir"),
            "TrackingBackend.list_dir() should have been called, got: {:?}",
            calls
        );
    }

    #[tokio::test]
    async fn run_without_backend_uses_local() {
        // Without .backend(), engine should use LocalBackend (host execution).
        // We verify by running a real bash command that writes to a temp file.
        let test_path =
            std::env::temp_dir().join(format!("sage_local_backend_test_{}", std::process::id()));
        let test_path_str = test_path.to_str().unwrap().to_string();

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider(
                "bash",
                &format!(
                    r#"{{"command":"echo localtest > {}","timeout":10}}"#,
                    test_path_str
                ),
            ))
            .builtin_tools(&["bash"])
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        while let Some(_) = rx.next().await {}

        // The file should exist on the host because LocalBackend ran the command
        let content = std::fs::read_to_string(&test_path);
        assert!(
            content.is_ok(),
            "file should exist on host via LocalBackend: {:?}",
            content
        );
        assert_eq!(content.unwrap().trim(), "localtest");
        let _ = std::fs::remove_file(&test_path);
    }

    /// A ToolBackend that always returns errors, for testing error propagation.
    struct FailingBackend;

    #[async_trait::async_trait]
    impl crate::tools::backend::ToolBackend for FailingBackend {
        async fn shell(
            &self,
            _command: &str,
            _timeout_secs: u64,
        ) -> Result<crate::tools::backend::ShellOutput, String> {
            Err("sandbox unavailable".into())
        }
        async fn read_file(&self, _path: &str) -> Result<Vec<u8>, String> {
            Err("sandbox unavailable".into())
        }
        async fn write_file(&self, _path: &str, _data: &[u8]) -> Result<(), String> {
            Err("sandbox unavailable".into())
        }
        async fn list_dir(
            &self,
            _path: &str,
        ) -> Result<Vec<crate::tools::backend::DirEntry>, String> {
            Err("sandbox unavailable".into())
        }
    }

    #[tokio::test]
    async fn run_with_failing_backend_propagates_error_in_events() {
        let backend: Arc<dyn crate::tools::backend::ToolBackend> = Arc::new(FailingBackend);

        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(tool_call_provider(
                "bash",
                r#"{"command":"echo hi","timeout":10}"#,
            ))
            .builtin_tools(&["bash"])
            .backend(backend)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = rx.next().await {
            events.push(event);
        }

        // The agent loop should complete (not panic). The ToolExecutionEnd
        // event should have is_error=true when the backend returns Err.
        let tool_end = events
            .iter()
            .find(|e| matches!(e, AgentEvent::ToolExecutionEnd { is_error: true, .. }));
        assert!(
            tool_end.is_some(),
            "should emit ToolExecutionEnd with is_error=true when backend fails, events: {:?}",
            events
                .iter()
                .map(|e| std::mem::discriminant(e))
                .collect::<Vec<_>>()
        );
    }

    struct SlowProvider {
        sleep_ms: u64,
    }

    #[async_trait::async_trait]
    impl LlmProvider for SlowProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
            vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            }]
        }
    }

    #[tokio::test]
    async fn test_fix_engine_timeout_emits_run_error_event() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(SlowProvider { sleep_ms: 2_000 })
            .timeout_secs(1)
            .build()
            .unwrap();

        let mut rx = engine.run("hi").await.unwrap();
        let run = tokio::time::timeout(std::time::Duration::from_millis(1_500), async move {
            let mut saw_run_error = false;
            while let Some(event) = rx.next().await {
                if matches!(event, AgentEvent::RunError { .. }) {
                    saw_run_error = true;
                }
            }
            saw_run_error
        })
        .await
        .expect("engine run should terminate after the configured timeout");

        assert!(run, "engine should emit a RunError event on task timeout");
    }

    #[tokio::test]
    async fn test_fix_network_enabled_sandbox_rejected_before_create() {
        let engine = SageEngine::builder()
            .system_prompt("test")
            .llm_provider(simple_provider("hello"))
            .sandbox(SandboxSettings {
                cpus: 1,
                memory_mib: 256,
                volumes: vec![],
                network_enabled: true,
                security: None,
            })
            .build()
            .unwrap();

        let err = match engine.run("hi").await {
            Ok(_) => panic!("network-enabled sandbox should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("network-enabled sandbox is not implemented"),
            "unexpected error: {err}"
        );
    }

    // =================================================================
    // S6.2a: SageSession HookEvent lifecycle emission
    // =================================================================
    //
    // These tests lock the behavioral contract for Sprint 6 S6.2a: SageSession
    // owns a HookBus and emits SessionStart / SessionEnd / PreCompact /
    // PostCompact at the documented lifecycle points. Stubs in engine.rs /
    // agent_loop.rs / compaction.rs mark the emission sites with `TODO S6.2a`.
    // Tests here will stay red until the implementer wires the emits through.

    use crate::hook::HookEvent;
    use tokio::sync::broadcast::error::TryRecvError;
    use tokio::time::{Duration, timeout};

    /// Build a bare-minimum engine + session for lifecycle tests.
    fn lifecycle_engine(provider: StatefulProvider) -> SageEngine {
        SageEngine::builder()
            .name("session-test-agent")
            .system_prompt("test")
            .llm_provider(provider)
            .build()
            .expect("engine builder should succeed")
    }

    /// Build an engine whose `.name()` and `.model()` are explicit — used to
    /// assert payload fields on SessionStart.
    fn named_engine(name: &str, provider_name: &str, model: &str) -> SageEngine {
        SageEngine::builder()
            .name(name)
            .system_prompt("test")
            .provider(provider_name)
            .model(model)
            .base_url("http://localhost")
            .api_key_env("TEST_KEY")
            .build()
            .expect("engine builder should succeed")
    }

    /// Wait briefly for a HookEvent on `rx`, panicking on timeout. Timeouts are
    /// the expected failure mode for the red-phase stubs.
    async fn expect_event(rx: &mut crate::hook::HookReceiver, label: &str) -> HookEvent {
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for HookEvent::{label}"))
            .unwrap_or_else(|e| panic!("HookEvent channel closed while awaiting {label}: {e:?}"))
    }

    // ── SessionStart emission ────────────────────────────────────────

    #[tokio::test]
    async fn session_start_event_emitted_after_engine_session_call() {
        let engine = lifecycle_engine(simple_provider("hello"));
        // Subscribe via a lightweight pre-subscription channel: we build a
        // fresh HookBus through the engine, subscribe, then trigger a second
        // session(). Since HookBus subscribes only receive *subsequent* emits,
        // the only reliable path is to subscribe to the session returned by
        // session() and assert SessionStart is still recoverable. Broadcast
        // capacity (default 256) means the SessionStart emitted during
        // construction remains buffered for the first subscriber.
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();

        // Ensure we don't accidentally miss the event: request a second
        // broadcast by re-emitting via the public API is not possible for
        // SessionStart. Instead, the implementer MUST emit SessionStart after
        // the HookBus is owned by the session AND the subscriber has a chance
        // to attach — meaning either (a) construction defers emit via
        // `tokio::spawn`, or (b) this test instead subscribes via a getter on
        // SageEngineBuilder. For the red phase we assert the former contract.
        let event = expect_event(&mut rx, "SessionStart").await;
        match event {
            HookEvent::SessionStart {
                session_id,
                agent_name,
                model,
            } => {
                assert_eq!(session_id, session.session_id());
                assert_eq!(agent_name, "session-test-agent");
                assert!(!model.is_empty(), "model should be populated");
            }
            other => panic!("expected SessionStart, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_start_includes_agent_name_and_model() {
        let engine = named_engine("payload-agent", "deepseek", "deepseek-chat");
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();

        let event = expect_event(&mut rx, "SessionStart").await;
        match event {
            HookEvent::SessionStart {
                agent_name, model, ..
            } => {
                assert_eq!(agent_name, "payload-agent");
                assert_eq!(model, "deepseek-chat");
            }
            other => panic!("expected SessionStart, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_start_not_re_emitted_on_send() {
        // SessionStart fires exactly once per session — calling send() must
        // not produce a second one.
        let engine = lifecycle_engine(simple_provider("hello"));
        let mut session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();

        // Drain SessionStart (expected once).
        let _ = expect_event(&mut rx, "SessionStart").await;

        // Drive a turn — any subsequent SessionStart is a bug.
        let sink = crate::event::EventStream::<AgentEvent, Vec<AgentMessage>>::new().0;
        struct NoopSink;
        #[async_trait::async_trait]
        impl AgentEventSink for NoopSink {
            async fn emit(&self, _event: AgentEvent) {}
        }
        session
            .send("follow-up", &NoopSink)
            .await
            .expect("send should succeed");
        let _ = sink; // keep the sender alive above cleanup order

        // Drain any trailing events; none should be SessionStart.
        loop {
            match rx.try_recv() {
                Ok(HookEvent::SessionStart { .. }) => {
                    panic!("SessionStart must not fire a second time on send()")
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                Err(TryRecvError::Lagged(_)) => continue,
            }
        }
    }

    // ── SessionEnd emission ──────────────────────────────────────────

    #[tokio::test]
    async fn session_end_event_emitted_on_close() {
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _start = expect_event(&mut rx, "SessionStart").await;

        session.close(true).await.expect("close should succeed");
        let event = expect_event(&mut rx, "SessionEnd").await;
        assert!(
            matches!(event, HookEvent::SessionEnd { .. }),
            "expected SessionEnd, got {event:?}"
        );
    }

    #[tokio::test]
    async fn session_end_success_field_reflects_argument() {
        // close(true)
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;
        session.close(true).await.unwrap();
        let ok = expect_event(&mut rx, "SessionEnd(true)").await;
        match ok {
            HookEvent::SessionEnd { success, .. } => assert!(success),
            other => panic!("expected SessionEnd, got {other:?}"),
        }

        // close(false)
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;
        session.close(false).await.unwrap();
        let fail = expect_event(&mut rx, "SessionEnd(false)").await;
        match fail {
            HookEvent::SessionEnd { success, .. } => assert!(!success),
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_end_duration_ms_covers_session_lifetime() {
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        session.close(true).await.unwrap();

        let event = expect_event(&mut rx, "SessionEnd").await;
        match event {
            HookEvent::SessionEnd { duration_ms, .. } => {
                assert!(
                    duration_ms >= 10,
                    "duration_ms should be >= 10, got {duration_ms}"
                );
            }
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_end_turn_count_matches_turns() {
        // Three successive send() calls — each completes one turn.
        let provider = StatefulProvider::new(vec![
            vec![
                AssistantMessageEvent::TextDelta("a".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            vec![
                AssistantMessageEvent::TextDelta("b".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            vec![
                AssistantMessageEvent::TextDelta("c".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);
        let engine = lifecycle_engine(provider);
        let mut session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;

        struct NoopSink;
        #[async_trait::async_trait]
        impl AgentEventSink for NoopSink {
            async fn emit(&self, _event: AgentEvent) {}
        }

        for _ in 0..3 {
            session.send("hi", &NoopSink).await.unwrap();
        }
        session.close(true).await.unwrap();

        // Drain until we find SessionEnd.
        loop {
            let ev = expect_event(&mut rx, "SessionEnd").await;
            match ev {
                HookEvent::SessionEnd { turn_count, .. } => {
                    assert_eq!(
                        turn_count, 3,
                        "expected turn_count=3 across three send() calls, got {turn_count}"
                    );
                    break;
                }
                _ => continue,
            }
        }
    }

    // ── PreCompact / PostCompact emission ────────────────────────────
    //
    // Wiring: the session-level HookBus needs to be threaded through
    // run_agent_loop and into try_compact. The implementer of S6.2b will
    // either (a) carry an Option<HookBus> in AgentLoopConfig, or (b) pass it
    // alongside the AgentEventSink. Tests below assume the bus is reachable
    // from the compaction code path and that both events fire on a real
    // compact trigger (small context_window + many short messages).
    //
    // These tests are marked #[ignore] because triggering real compaction
    // requires crafting the provider to respond with enough context tokens
    // to cross the 90% threshold — doable but fragile without helper
    // infrastructure. The red-phase contract is encoded in the assertions;
    // S6.2b should flip the ignores off once the wiring lands.

    #[tokio::test]
    #[ignore = "S6.2b: enable once HookBus is wired into compaction path"]
    async fn pre_compact_emitted_before_post_compact() {
        // Expected behavior: trigger compaction → observe PreCompact followed
        // by PostCompact (Pre must come first).
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;

        // Placeholder: an implementer-provided hook should trigger compaction.
        // For now, the test body intentionally fails fast if the ignored gate
        // is lifted without wiring. The actual trigger mechanism (e.g. a
        // test-only `force_compact()` on SageSession, or a tiny-context
        // model config) is left to the implementer.

        let mut saw_pre = false;
        while let Ok(Ok(ev)) = timeout(Duration::from_millis(50), rx.recv()).await {
            match ev {
                HookEvent::PreCompact { .. } => saw_pre = true,
                HookEvent::PostCompact { .. } => {
                    assert!(saw_pre, "PostCompact arrived before PreCompact");
                    return;
                }
                _ => {}
            }
        }
        panic!("expected PreCompact then PostCompact");
    }

    #[tokio::test]
    #[ignore = "S6.2b: enable once HookBus is wired into compaction path"]
    async fn pre_compact_payload_has_tokens_before_and_message_count() {
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;

        // Trigger compaction (see comment on pre_compact_emitted_before_post_compact).
        while let Ok(Ok(ev)) = timeout(Duration::from_millis(50), rx.recv()).await {
            if let HookEvent::PreCompact {
                tokens_before,
                message_count,
                ..
            } = ev
            {
                assert!(
                    tokens_before > 0,
                    "tokens_before must be populated, got {tokens_before}"
                );
                assert!(
                    message_count > 0,
                    "message_count must be populated, got {message_count}"
                );
                return;
            }
        }
        panic!("expected PreCompact event");
    }

    #[tokio::test]
    #[ignore = "S6.2b: enable once HookBus is wired into compaction path"]
    async fn post_compact_payload_has_tokens_after_and_messages_compacted() {
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();
        let _ = expect_event(&mut rx, "SessionStart").await;

        while let Ok(Ok(ev)) = timeout(Duration::from_millis(50), rx.recv()).await {
            if let HookEvent::PostCompact {
                tokens_before,
                tokens_after,
                messages_compacted,
                ..
            } = ev
            {
                assert!(tokens_before > 0, "tokens_before must be populated");
                assert!(
                    tokens_after <= tokens_before,
                    "tokens_after {tokens_after} must not exceed tokens_before {tokens_before}"
                );
                assert!(
                    messages_compacted > 0,
                    "messages_compacted must be positive after a real compact"
                );
                return;
            }
        }
        panic!("expected PostCompact event");
    }

    // ── HookBus accessibility ────────────────────────────────────────

    #[tokio::test]
    async fn hook_bus_accessible_via_session_getter() {
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        // The subscribe() call itself is the contract — it must succeed and
        // the bus must remain live across the session's lifetime.
        let rx = session.hook_bus().subscribe();
        assert_eq!(
            session.hook_bus().subscriber_count(),
            1,
            "subscribe() should register exactly one receiver"
        );
        drop(rx);
        assert_eq!(
            session.hook_bus().subscriber_count(),
            0,
            "dropping the receiver should decrement the subscriber count"
        );
    }

    #[tokio::test]
    async fn hook_bus_subscriber_sees_all_lifecycle_events_in_order() {
        // Red-phase contract: a single subscriber that attaches early enough
        // observes SessionStart → SessionEnd in order. (PreCompact / PostCompact
        // are covered by the dedicated #[ignore]'d tests above.)
        let engine = lifecycle_engine(simple_provider("hello"));
        let session = engine.session().await.expect("session build");
        let mut rx = session.hook_bus().subscribe();

        let first = expect_event(&mut rx, "SessionStart").await;
        assert!(
            matches!(first, HookEvent::SessionStart { .. }),
            "first event must be SessionStart, got {first:?}"
        );

        session.close(true).await.unwrap();
        let last = expect_event(&mut rx, "SessionEnd").await;
        assert!(
            matches!(last, HookEvent::SessionEnd { .. }),
            "last event must be SessionEnd, got {last:?}"
        );
    }
}
