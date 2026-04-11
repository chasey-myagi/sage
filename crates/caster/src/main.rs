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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("agent_caster=debug,agent_sandbox=debug,agent_runner=debug")
        .init();

    let cli = Cli::parse();
    tracing::info!(
        runtime = %cli.runtime,
        caster_id = %cli.caster_id,
        max_concurrent = cli.max_concurrent,
        "starting agent caster"
    );

    serve::run(cli.runtime, cli.caster_id, cli.max_concurrent).await
}
