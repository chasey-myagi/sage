use anyhow::Result;
use sage_runner::AgentConfig;
use sage_runtime::engine::SageEngine;
use sage_runtime::event::AgentEvent;
use sage_runtime::types::*;

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

/// Run a local test: load config → build SageEngine → run → print events.
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

    // 2. Build SageEngine from AgentConfig fields
    let tool_names = config.tools.tool_names();
    let tool_name_refs: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();

    let mut builder = SageEngine::builder()
        .system_prompt(&config.system_prompt)
        .provider(provider_override.unwrap_or(&config.llm.provider))
        .model(model_override.unwrap_or(&config.llm.model))
        .max_tokens(config.llm.max_tokens)
        .max_turns(config.constraints.max_turns as usize)
        .tool_execution_mode(ToolExecutionMode::Parallel)
        .tool_policy(config.tools.to_policy())
        .builtin_tools(&tool_name_refs);

    if let Some(url) = &config.llm.base_url {
        builder = builder.base_url(url);
    }
    if let Some(env) = &config.llm.api_key_env {
        builder = builder.api_key_env(env);
    }

    let engine = builder.build()?;

    // 3. Run and consume events
    let mut rx = engine.run(message).await?;
    while let Some(event) = rx.next().await {
        print_event(&event);
    }

    Ok(())
}

/// Print an agent event to stderr (terminal output).
fn print_event(event: &AgentEvent) {
    match event {
        AgentEvent::AgentStart => {
            eprintln!("--- Agent started ---");
        }
        AgentEvent::AgentEnd { messages } => {
            // Print the final assistant reply — MessageUpdate may not be
            // emitted by the current agent loop, so extract text here.
            for msg in messages {
                if let AgentMessage::Assistant(a) = msg {
                    for c in &a.content {
                        if let Content::Text { text } = c {
                            println!("{text}");
                        }
                    }
                }
            }
            eprintln!("--- Agent finished ---");
        }
        AgentEvent::TurnStart => {
            eprintln!("  [turn]");
        }
        AgentEvent::TurnEnd { .. } => {}
        AgentEvent::MessageStart { message } => {
            if let AgentMessage::User(u) = message {
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
        }
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
        AgentEvent::CompactionStart { reason, message_count } => {
            eprintln!("  [compaction: {reason}, {message_count} messages]");
        }
        AgentEvent::CompactionEnd { tokens_before, messages_compacted } => {
            eprintln!("  [compacted: {messages_compacted} messages, was {tokens_before} tokens]");
        }
    }
}
