use agent_runner::AgentConfig;
use agent_runtime::agent::Agent;
use agent_runtime::agent_loop::run_agent_loop;
use agent_runtime::event::{AgentEvent, AgentEventSink};
use agent_runtime::llm::keys;
use agent_runtime::llm::models;
use agent_runtime::llm::openai_compat::OpenAiCompatProvider;
use agent_runtime::llm::types::{InputType, Model, ModelCost, ProviderCompat, api};
use agent_runtime::tools::ToolRegistry;
use agent_runtime::types::*;
use anyhow::{Context, Result, bail};

/// Main serve loop.
///
/// 1. Connect to Rune Runtime
/// 2. Register `agents.execute` rune
/// 3. Handle incoming tasks: parse config → create sandbox → run agent → return result
pub async fn run(runtime_addr: String, _caster_id: String, _max_concurrent: usize) -> Result<()> {
    tracing::info!("connecting to Rune Runtime at {}", runtime_addr);

    // TODO: Phase 2 — Rune Caster SDK integration
    tracing::info!("agent caster running (stub mode)");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

/// Run a local test: load config → create agent → run loop → print result.
pub async fn run_local_test(
    config_path: &str,
    message: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<()> {
    // 1. Load agent config
    let yaml = tokio::fs::read_to_string(config_path).await?;
    let config: AgentConfig = serde_yaml::from_str(&yaml)?;
    tracing::info!(agent = %config.name, "loaded config");

    // 2. Execute via agent loop
    let result = handle_execute(
        config,
        message.to_string(),
        provider_override,
        model_override,
    )
    .await?;

    // 3. Print result
    println!("{result}");
    Ok(())
}

/// Handle a single agent execution request.
async fn handle_execute(
    config: AgentConfig,
    message: String,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<String> {
    let provider_name = provider_override.unwrap_or(&config.llm.provider);
    let model_id = model_override.unwrap_or(&config.llm.model);

    // 1. Resolve model — try built-in catalog first, then construct from config
    let model = resolve_model_from_config(&config, provider_name, model_id)?;
    tracing::info!(
        provider = %model.provider,
        model = %model.id,
        "resolved model"
    );

    // 2. Create LLM provider
    let llm_provider = OpenAiCompatProvider::new();

    // 3. Register tools
    let tool_names = config.tools.tool_names();
    let mut registry = ToolRegistry::new();
    for name in &tool_names {
        if let Some(tool) = agent_runtime::tools::create_tool(name) {
            registry.register(tool);
        } else {
            tracing::warn!(tool = %name, "unknown tool, skipping");
        }
    }
    tracing::info!(tools = ?tool_names, "registered tools");

    // 4. Create Agent
    let loop_config = agent_runtime::agent::AgentLoopConfig {
        model,
        system_prompt: config.system_prompt.clone(),
        max_turns: config.constraints.max_turns as usize,
        tool_execution_mode: ToolExecutionMode::Parallel,
        tool_policy: Some(config.tools.to_policy()),
    };

    let mut agent = Agent::new(loop_config, Box::new(llm_provider), registry);
    agent.steer(AgentMessage::User(UserMessage::from_text(&message)));

    // 5. Run agent loop with terminal event sink
    let sink = TerminalEventSink;
    let result = run_agent_loop(&mut agent, &sink).await;

    match result {
        Ok(messages) => {
            // Extract final assistant text
            let final_text: String = messages
                .iter()
                .rev()
                .find_map(|m| match m {
                    AgentMessage::Assistant(a) => {
                        let text = a.text();
                        if text.is_empty() { None } else { Some(text) }
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "(no response)".into());
            Ok(final_text)
        }
        Err(e) => bail!("Agent loop error: {e}"),
    }
}

/// Resolve a Model from config — try built-in catalog, fallback to config fields.
fn resolve_model_from_config(
    config: &AgentConfig,
    provider: &str,
    model_id: &str,
) -> Result<Model> {
    // Try built-in catalog
    if let Some(mut model) = models::resolve_model(provider, model_id) {
        // Apply config overrides
        if let Some(ref base_url) = config.llm.base_url {
            model.base_url = base_url.clone();
        }
        if let Some(ref api_key_env) = config.llm.api_key_env {
            model.api_key_env = api_key_env.clone();
        }
        return Ok(model);
    }

    // Not in catalog — construct from config
    let base_url = config
        .llm
        .base_url
        .clone()
        .context("base_url required for non-catalog model")?;
    let api_key_env = config
        .llm
        .api_key_env
        .clone()
        .unwrap_or_else(|| keys::api_key_env_var(provider));

    Ok(Model {
        id: model_id.into(),
        name: model_id.into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: provider.into(),
        base_url,
        api_key_env,
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: config.llm.max_tokens,
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

/// Terminal event sink — prints agent events to stdout.
struct TerminalEventSink;

#[async_trait::async_trait]
impl AgentEventSink for TerminalEventSink {
    async fn emit(&self, event: AgentEvent) {
        match &event {
            AgentEvent::AgentStart => {
                eprintln!("--- Agent started ---");
            }
            AgentEvent::AgentEnd { messages } => {
                eprintln!("--- Agent finished ({} messages) ---", messages.len());
            }
            AgentEvent::TurnStart => {
                eprintln!("  [turn]");
            }
            AgentEvent::TurnEnd { .. } => {}
            AgentEvent::MessageStart { message } => match message {
                AgentMessage::User(u) => {
                    eprintln!(
                        "  > User: {}",
                        u.content
                            .iter()
                            .filter_map(|c| match c {
                                Content::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("")
                    );
                }
                _ => {}
            },
            AgentEvent::MessageUpdate { delta, .. } => {
                eprint!("{delta}");
            }
            AgentEvent::MessageEnd { .. } => {
                eprintln!();
            }
            AgentEvent::ToolExecutionStart { tool_name, .. } => {
                eprintln!("  [tool: {tool_name}]");
            }
            AgentEvent::ToolExecutionUpdate { partial_result, .. } => {
                eprint!("{partial_result}");
            }
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } => {
                if *is_error {
                    eprintln!("  [tool: {tool_name} — ERROR]");
                }
            }
        }
    }
}
