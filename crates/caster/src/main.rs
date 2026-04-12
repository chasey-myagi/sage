use anyhow::Result;
use clap::Parser;

mod serve;

#[derive(Parser)]
#[command(name = "agent-caster", about = "Agent Caster — sandbox-isolated agent executor")]
struct Cli {
    /// Rune Runtime gRPC address
    #[arg(long, default_value = "localhost:50070")]
    runtime: String,

    /// Caster ID for Rune registration
    #[arg(long, default_value = "agents-executor")]
    caster_id: String,

    /// Max concurrent sandbox VMs
    #[arg(long, default_value = "3")]
    max_concurrent: usize,

    /// Run a local test: load config → create agent → run loop → print output
    #[arg(long)]
    local_test: bool,

    /// Path to agent config YAML (used with --local-test)
    #[arg(long, default_value = "configs/feishu-assistant.yaml")]
    config: String,

    /// Message to send to the agent (used with --local-test)
    #[arg(long, default_value = "echo 'hello from sandbox'")]
    message: String,

    /// Override LLM provider (e.g., qwen, deepseek)
    #[arg(long)]
    provider: Option<String>,

    /// Override model ID (e.g., qwen-plus, deepseek-chat)
    #[arg(long)]
    model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("agent_caster=debug,agent_sandbox=debug,agent_runner=debug,agent_runtime=debug")
        .init();

    let cli = Cli::parse();

    if cli.local_test {
        tracing::info!(config = %cli.config, "running local test");
        return serve::run_local_test(
            &cli.config,
            &cli.message,
            cli.provider.as_deref(),
            cli.model.as_deref(),
        )
        .await;
    }

    tracing::info!(
        runtime = %cli.runtime,
        caster_id = %cli.caster_id,
        max_concurrent = cli.max_concurrent,
        "starting agent caster"
    );

    serve::run(cli.runtime, cli.caster_id, cli.max_concurrent).await
}
