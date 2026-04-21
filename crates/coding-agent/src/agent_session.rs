//! Agent session wiring — connects the CLI to `agent-core` and the `ai` provider registry.
//!
//! `run_agent_session` is the concrete entry point for print-mode execution:
//! it resolves a model, creates an LLM provider, wires up the default tools,
//! subscribes to events, sends the user message, and waits for completion.

use std::io::Write as _;
use std::sync::Arc;

use agent_core::agent::{Agent, AgentOptions};
use agent_core::agent_loop::LlmProvider;
use agent_core::tools::backend::LocalBackend;
use agent_core::tools::{AgentTool as SimpleTool, ToolOutput, create_tool};
use agent_core::types::{AgentToolResult, OnUpdateFn};
use ai::registry::{ApiProviderRegistry, StreamOptions};
use ai::types::{AssistantMessageEvent, InputType, Model, ModelCost, Usage};

/// Events sent through the interactive-mode channel.
#[derive(Debug, Clone)]
pub enum AgentDelta {
    /// A streaming text fragment.
    Text(String),
    /// Token usage snapshot after a turn completes.
    TurnUsage { usage: Usage, model: String, is_fast: bool },
    /// A tool call has started.
    ToolStart { name: String, args_preview: String },
    /// A tool call has completed.
    ToolEnd { name: String, success: bool, output_preview: String },
    /// A fatal agent error.
    Error(String),
}

// ── Registry-backed LLM provider adapter ────────────────────────────────────

/// Adapts `ai::registry::ApiProviderRegistry` + `StreamOptions` into the
/// `agent_core::agent_loop::LlmProvider` trait.
struct RegistryProvider {
    registry: Arc<ApiProviderRegistry>,
    options: StreamOptions,
}

#[async_trait::async_trait]
impl LlmProvider for RegistryProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &ai::types::LlmContext,
        tools: &[ai::types::LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        let provider = match self.registry.get(&model.api) {
            Some(p) => p,
            None => {
                return vec![AssistantMessageEvent::Error(format!(
                    "No provider registered for API: {}",
                    model.api
                ))];
            }
        };
        provider.stream(model, context, tools, &self.options).await
    }
}

// ── Tool adapter: tools::AgentTool → types::AgentTool ───────────────────────

/// Wraps a `agent_core::tools::AgentTool` (simple interface) into the
/// `agent_core::types::AgentTool` trait (full interface expected by `Agent`).
struct ToolAdapter {
    inner: Box<dyn SimpleTool>,
    tool_description: String,
    tool_schema: serde_json::Value,
}

impl ToolAdapter {
    fn new(tool: Box<dyn SimpleTool>) -> Self {
        let description = tool.description().to_string();
        let schema = tool.parameters_schema();
        Self {
            inner: tool,
            tool_description: description,
            tool_schema: schema,
        }
    }
}

#[async_trait::async_trait]
impl agent_core::types::AgentTool for ToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn label(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool_schema.clone()
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: serde_json::Value,
        _signal: Option<tokio_util::sync::CancellationToken>,
        _on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult {
        let output: ToolOutput = self.inner.execute(args).await;
        AgentToolResult {
            content: output.content,
            details: serde_json::Value::Null,
        }
    }
}

// ── Model construction ───────────────────────────────────────────────────────

/// Build a `Model` from a provider spec + optional model id override.
///
/// Defaults: `anthropic` provider, `claude-sonnet-4-5` model.
fn build_model(provider_id: Option<&str>, model_id: Option<&str>) -> anyhow::Result<Model> {
    let provider_id = provider_id.unwrap_or("anthropic");
    let model_id = model_id.unwrap_or("claude-sonnet-4-5");

    let spec = ai::provider_specs::resolve_provider(provider_id).ok_or_else(|| {
        anyhow::anyhow!(
            "Unknown provider '{}'. Run with --list-models to see available providers.",
            provider_id
        )
    })?;

    Ok(Model {
        id: model_id.to_string(),
        name: model_id.to_string(),
        api: spec.api_kind.to_string(),
        provider: provider_id.to_string(),
        base_url: spec.base_url.to_string(),
        api_key_env: spec.api_key_env.to_string(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: spec.default_max_tokens,
        context_window: spec.default_context_window,
        cost: ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: None,
    })
}

// ── Default tool list ────────────────────────────────────────────────────────

/// Create the default coding-agent tools backed by the local filesystem,
/// wrapped as `types::AgentTool` for use with `Agent`.
fn create_default_tools(backend: Arc<LocalBackend>) -> Vec<Arc<dyn agent_core::types::AgentTool>> {
    ["bash", "read", "write", "edit", "grep", "find", "ls"]
        .iter()
        .filter_map(|name| {
            create_tool(
                name,
                Arc::clone(&backend) as Arc<dyn agent_core::tools::backend::ToolBackend>,
            )
        })
        .map(|t| -> Arc<dyn agent_core::types::AgentTool> { Arc::new(ToolAdapter::new(t)) })
        .collect()
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Extract a short human-readable summary of tool args for display in the TUI.
fn args_to_preview(args: &serde_json::Value) -> String {
    for key in &["command", "file_path", "pattern", "path"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return v.chars().take(80).collect();
        }
    }
    args.to_string().chars().take(80).collect()
}

/// Run an agent session, sending [`AgentDelta`] events through `tx` instead of stdout.
///
/// Drops `tx` when the agent finishes so the receiver knows the stream ended.
pub async fn run_agent_session_to_channel(
    message: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
    tx: tokio::sync::mpsc::UnboundedSender<AgentDelta>,
) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }

    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;
    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    let options = StreamOptions { api_key, ..StreamOptions::default() };
    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider { registry: Arc::clone(&registry), options });

    let mut agent = Agent::new(AgentOptions::new(
        model,
        "You are a helpful coding assistant.",
        provider,
    ));

    let backend = LocalBackend::new();
    let tools = create_default_tools(backend);
    agent.set_tools(tools);

    agent.subscribe(move |event| {
        use agent_core::AgentEvent;
        match event {
            AgentEvent::MessageUpdate { delta, .. } => {
                let _ = tx.send(AgentDelta::Text(delta.clone()));
            }
            AgentEvent::TurnEnd { message, .. } => {
                let is_fast = message.model.to_ascii_lowercase().contains("fast");
                let _ = tx.send(AgentDelta::TurnUsage {
                    usage: message.usage.clone(),
                    model: message.model.clone(),
                    is_fast,
                });
            }
            AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
                let _ = tx.send(AgentDelta::ToolStart {
                    name: tool_name.clone(),
                    args_preview: args_to_preview(&args),
                });
            }
            AgentEvent::ToolExecutionEnd { tool_name, result, is_error, .. } => {
                use agent_core::types::Content;
                let text = result.content.iter().find_map(|c| {
                    if let Content::Text { text } = c { Some(text.as_str()) } else { None }
                }).unwrap_or("");
                let first_nonempty = text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect::<String>();
                let _ = tx.send(AgentDelta::ToolEnd {
                    name: tool_name.clone(),
                    success: !is_error,
                    output_preview: first_nonempty,
                });
            }
            AgentEvent::RunError { error } => {
                let _ = tx.send(AgentDelta::Error(error.clone()));
            }
            _ => {}
        }
    });

    agent.prompt_text(message).await.map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Run a single-shot agent session in print mode.
///
/// Resolves the model + provider, wires up tools, streams events to stdout,
/// and returns when the agent has finished.
pub async fn run_agent_session(
    message: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }

    // 1. Build model descriptor.
    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;

    // 2. Register built-in API providers and wrap in our adapter.
    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    let options = StreamOptions {
        api_key,
        ..StreamOptions::default()
    };

    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider {
        registry: Arc::clone(&registry),
        options,
    });

    // 3. Build agent.
    let mut agent = Agent::new(AgentOptions::new(
        model,
        "You are a helpful coding assistant.",
        provider,
    ));

    // 4. Attach default tools (local backend, no workspace root).
    let backend = LocalBackend::new();
    let tools = create_default_tools(backend);
    agent.set_tools(tools);

    // 5. Subscribe to events: stream text deltas to stdout.
    let stdout = std::io::stdout();
    agent.subscribe(move |event| {
        use agent_core::AgentEvent;
        match event {
            AgentEvent::MessageUpdate { delta, .. } => {
                let mut out = stdout.lock();
                let _ = out.write_all(delta.as_bytes());
                let _ = out.flush();
            }
            AgentEvent::AgentEnd { .. } => {
                let mut out = stdout.lock();
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
            AgentEvent::RunError { error } => {
                eprintln!("\nError: {error}");
            }
            AgentEvent::TurnEnd { message, .. } => {
                use agent_core::types::StopReason;
                if let Some(err) = &message.error_message {
                    eprintln!("Error: {err}");
                } else if message.stop_reason == StopReason::Error {
                    eprintln!("Error: agent stopped with error");
                }
            }
            _ => {}
        }
    });

    // 6. Send the message and wait.
    agent
        .prompt_text(message)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
