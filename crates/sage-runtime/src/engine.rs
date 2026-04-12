// SageEngine — builder-pattern API for creating and running AI agents.
// "SQLite of AI Agents" — zero-config embed, single crate dependency.

use crate::agent::{AfterToolCallHook, Agent, AgentLoopConfig, BeforeToolCallHook};
use crate::agent_loop::{AgentLoopError, run_agent_loop};
use crate::event::{AgentEvent, AgentEventSink, EventReceiver, EventSender, EventStream};
use crate::llm::types::*;
use crate::llm::{self, LlmProvider};
use crate::tools::policy::ToolPolicy;
use crate::tools::{self, AgentTool, ToolRegistry};
use crate::types::*;
use std::sync::{Arc, Once};

// ── Error type ────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SageError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("model resolution failed: {0}")]
    ModelResolution(String),

    #[error("agent loop error: {0}")]
    AgentLoop(#[from] AgentLoopError),
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

    Ok(Model {
        id: model_id.into(),
        name: model_id.into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: provider.into(),
        base_url: url.to_string(),
        api_key_env: api_key_env
            .map(|s| s.to_string())
            .unwrap_or_else(|| llm::keys::api_key_env_var(provider)),
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
    })
}

// ── RoutingProvider ───────────────────────────────────────────────────

/// Routes to the correct `ApiProvider` based on `model.api` field.
/// Uses the global `llm::registry` to resolve Anthropic, Google, OpenAI, etc.
/// Falls back to `OpenAiCompatProvider` for unregistered APIs.
struct RoutingProvider;

#[async_trait::async_trait]
impl LlmProvider for RoutingProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        // Try ApiProvider registry (anthropic-messages, google-generative-ai, etc.)
        if let Some(provider) = llm::registry::get_provider(&model.api) {
            let options = llm::registry::StreamOptions {
                max_tokens: Some(context.max_tokens),
                temperature: context.temperature,
                ..Default::default()
            };
            return provider.stream(model, context, tools, &options).await;
        }

        // Fallback: OpenAI-compatible chat completions
        llm::openai_compat::OpenAiCompatProvider::new()
            .complete(model, context, tools)
            .await
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
        let _ = self.sender.send(event);
    }
}

// ── SageEngine ────────────────────────────────────────────────────────

/// Sage execution engine — holds atomic config fields, creates a fresh
/// Agent instance on each `run()` call.
pub struct SageEngine {
    // Agent config
    system_prompt: String,
    max_turns: usize,
    tool_execution_mode: ToolExecutionMode,
    tool_policy: Option<ToolPolicy>,

    // Tools
    builtin_tool_names: Vec<String>,
    extra_tools: Vec<Arc<dyn AgentTool>>,

    // LLM
    provider_name: String,
    model_id: String,
    max_tokens: u32,
    base_url: Option<String>,
    api_key_env: Option<String>,
    custom_llm_provider: Option<Arc<dyn LlmProvider>>,

    // Hooks
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
}

impl std::fmt::Debug for SageEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SageEngine")
            .field("system_prompt", &self.system_prompt)
            .field("provider_name", &self.provider_name)
            .field("model_id", &self.model_id)
            .field("max_turns", &self.max_turns)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

impl SageEngine {
    pub fn builder() -> SageEngineBuilder {
        SageEngineBuilder {
            system_prompt: None,
            max_turns: None,
            tool_execution_mode: None,
            tool_policy: None,
            builtin_tool_names: Vec::new(),
            extra_tools: Vec::new(),
            provider_name: None,
            model_id: None,
            max_tokens: None,
            base_url: None,
            api_key_env: None,
            custom_llm_provider: None,
            before_hook: None,
            after_hook: None,
        }
    }

    /// Execute the agent loop, returning an event receiver.
    /// Each call creates a fresh Agent, Provider, and ToolRegistry.
    pub async fn run(
        &self,
        message: &str,
    ) -> Result<EventReceiver<AgentEvent, Vec<AgentMessage>>, SageError> {
        // 1. Create LLM provider
        let provider: Arc<dyn LlmProvider> = match &self.custom_llm_provider {
            Some(p) => Arc::clone(p),
            None => {
                // Validate model exists before creating provider
                let _model = resolve_or_construct_model(
                    &self.provider_name,
                    &self.model_id,
                    self.max_tokens,
                    self.base_url.as_deref(),
                    self.api_key_env.as_deref(),
                )?;
                // Ensure builtin ApiProviders are registered once
                static INIT: Once = Once::new();
                INIT.call_once(llm::register_builtin_providers);
                // RoutingProvider dispatches by model.api → correct ApiProvider
                Arc::new(RoutingProvider)
            }
        };

        // 2. Build ToolRegistry
        let mut registry = ToolRegistry::new();
        for name in &self.builtin_tool_names {
            if let Some(tool) = tools::create_tool(name) {
                registry.register(tool);
            }
        }
        for tool in &self.extra_tools {
            registry.register(Box::new(ArcTool(Arc::clone(tool))));
        }

        // 3. Resolve model for AgentLoopConfig
        let model = match &self.custom_llm_provider {
            Some(_) => {
                // Custom provider — build a minimal model struct
                Model {
                    id: if self.model_id.is_empty() {
                        "custom".into()
                    } else {
                        self.model_id.clone()
                    },
                    name: if self.model_id.is_empty() {
                        "custom".into()
                    } else {
                        self.model_id.clone()
                    },
                    api: api::OPENAI_COMPLETIONS.into(),
                    provider: if self.provider_name.is_empty() {
                        "custom".into()
                    } else {
                        self.provider_name.clone()
                    },
                    base_url: self.base_url.clone().unwrap_or_default(),
                    api_key_env: self.api_key_env.clone().unwrap_or_default(),
                    reasoning: false,
                    input: vec![InputType::Text],
                    max_tokens: self.max_tokens,
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
            None => resolve_or_construct_model(
                &self.provider_name,
                &self.model_id,
                self.max_tokens,
                self.base_url.as_deref(),
                self.api_key_env.as_deref(),
            )?,
        };

        // 4. AgentLoopConfig
        let loop_config = AgentLoopConfig {
            model,
            system_prompt: self.system_prompt.clone(),
            max_turns: self.max_turns,
            tool_execution_mode: self.tool_execution_mode,
            tool_policy: self.tool_policy.clone(),
        };

        // 5. Create Agent
        let mut agent = Agent::new(loop_config, Box::new(ArcProvider(provider)), registry);

        // 6. Set hooks
        if let Some(ref hook) = self.before_hook {
            agent.set_before_tool_call(Box::new(ArcBeforeHook(Arc::clone(hook))));
        }
        if let Some(ref hook) = self.after_hook {
            agent.set_after_tool_call(Box::new(ArcAfterHook(Arc::clone(hook))));
        }

        // 7. Steer initial message
        agent.steer(AgentMessage::User(UserMessage::from_text(message)));

        // 8. Create EventStream
        let (sender, receiver) = EventStream::<AgentEvent, Vec<AgentMessage>>::new();

        // 9. Spawn agent loop
        let sink = ChannelSink {
            sender: sender.clone(),
        };
        tokio::spawn(async move {
            let result = run_agent_loop(&mut agent, &sink).await;
            match result {
                Ok(messages) => sender.end(messages),
                Err(e) => {
                    tracing::error!(error = %e, "agent loop failed");
                    // Emit AgentEnd with empty messages so the consumer sees the stream end.
                    // The error is logged; consumers can detect failure via empty result.
                    let _ = sink.emit(AgentEvent::AgentEnd { messages: vec![] }).await;
                    sender.end(vec![]);
                }
            }
        });

        // 10. Return receiver
        Ok(receiver)
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
    system_prompt: Option<String>,
    max_turns: Option<usize>,
    tool_execution_mode: Option<ToolExecutionMode>,
    tool_policy: Option<ToolPolicy>,
    builtin_tool_names: Vec<String>,
    extra_tools: Vec<Arc<dyn AgentTool>>,
    provider_name: Option<String>,
    model_id: Option<String>,
    max_tokens: Option<u32>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    custom_llm_provider: Option<Arc<dyn LlmProvider>>,
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
}

impl SageEngineBuilder {
    // ── Agent config ──

    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }

    pub fn max_turns(mut self, n: usize) -> Self {
        self.max_turns = Some(n);
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

        Ok(SageEngine {
            system_prompt,
            max_turns: self.max_turns.unwrap_or(10),
            tool_execution_mode: self
                .tool_execution_mode
                .unwrap_or(ToolExecutionMode::Parallel),
            tool_policy: self.tool_policy,
            builtin_tool_names: self.builtin_tool_names,
            extra_tools: self.extra_tools,
            provider_name: self.provider_name.unwrap_or_default(),
            model_id: self.model_id.unwrap_or_else(|| "custom".to_string()),
            max_tokens: self.max_tokens.unwrap_or(4096),
            base_url: self.base_url,
            api_key_env: self.api_key_env,
            custom_llm_provider: self.custom_llm_provider,
            before_hook: self.before_hook,
            after_hook: self.after_hook,
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
        assert!(b.tool_execution_mode.is_none());
        assert!(b.tool_policy.is_none());
        assert!(b.builtin_tool_names.is_empty());
        assert!(b.extra_tools.is_empty());
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
}
