use anyhow::Result;
use clap::{Parser, Subcommand};

mod serve;

#[derive(Parser)]
#[command(name = "sage", about = "Sage — embeddable AI agent execution engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run an agent locally: load config -> execute -> print result
    Run {
        /// Path to agent config YAML
        #[arg(long)]
        config: String,

        /// Message to send to the agent
        #[arg(long)]
        message: String,

        /// Override LLM provider (e.g., qwen, deepseek)
        #[arg(long)]
        provider: Option<String>,

        /// Override model ID (e.g., qwen-plus, deepseek-chat)
        #[arg(long)]
        model: Option<String>,
    },

    /// Start as a Rune Caster service (Phase 2)
    Serve {
        /// Rune Runtime gRPC address
        #[arg(long, default_value = "localhost:50070")]
        runtime: String,

        /// Caster ID for Rune registration
        #[arg(long, default_value = "agents-executor")]
        caster_id: String,

        /// Max concurrent sandbox VMs
        #[arg(long, default_value = "3")]
        max_concurrent: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("sage=debug,sage_sandbox=debug,sage_runner=debug,sage_runtime=debug")
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            config,
            message,
            provider,
            model,
        } => {
            tracing::info!(config = %config, "running local agent");
            serve::run_local_test(&config, &message, provider.as_deref(), model.as_deref()).await
        }
        Commands::Serve {
            runtime,
            caster_id,
            max_concurrent,
        } => {
            tracing::info!(
                runtime = %runtime,
                caster_id = %caster_id,
                max_concurrent = max_concurrent,
                "starting agent caster"
            );
            serve::run(runtime, caster_id, max_concurrent).await
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn cli_run_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "run", "--config", "test.yaml", "--message", "hello"]);
        match cli.command {
            Commands::Run {
                config, message, ..
            } => {
                assert_eq!(config, "test.yaml");
                assert_eq!(message, "hello");
            }
            _ => panic!("expected Run subcommand"),
        }
    }

    #[test]
    fn cli_serve_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "serve", "--runtime", "myhost:9090"]);
        match cli.command {
            Commands::Serve {
                runtime,
                caster_id,
                max_concurrent,
            } => {
                assert_eq!(runtime, "myhost:9090");
                assert_eq!(caster_id, "agents-executor"); // default
                assert_eq!(max_concurrent, 3); // default
            }
            _ => panic!("expected Serve subcommand"),
        }
    }

    #[test]
    fn cli_run_with_overrides() {
        let cli = Cli::parse_from([
            "sage",
            "run",
            "--config",
            "test.yaml",
            "--message",
            "hi",
            "--provider",
            "qwen",
            "--model",
            "qwen-plus",
        ]);
        match cli.command {
            Commands::Run {
                config,
                message,
                provider,
                model,
            } => {
                assert_eq!(config, "test.yaml");
                assert_eq!(message, "hi");
                assert_eq!(provider.as_deref(), Some("qwen"));
                assert_eq!(model.as_deref(), Some("qwen-plus"));
            }
            _ => panic!("expected Run subcommand"),
        }
    }
}
