use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod chat;
mod context;
mod daemon;
mod harness;
mod known_models;
mod serve;
mod session_archive;
mod skill_install;
mod skill_scorer;
mod triggers;
mod tui;
mod wiki_trigger;

/// Build an [`EnvFilter`] from explicit option values (no side effects, pure function).
///
/// Priority order:
/// 1. `sage_log` — value of `SAGE_LOG` env var (if Some and non-empty)
/// 2. `rust_log` — value of `RUST_LOG` env var (if Some and non-empty)
/// 3. Hard-coded default: `sage=info,sage_sandbox=info,sage_runner=info,sage_runtime=info`
pub fn build_filter(sage_log: Option<&str>, rust_log: Option<&str>) -> EnvFilter {
    let default_filter =
        "sage=info,sage_sandbox=info,sage_runner=info,sage_runtime=info";

    let directive = sage_log
        .filter(|s| !s.is_empty())
        .or_else(|| rust_log.filter(|s| !s.is_empty()))
        .unwrap_or(default_filter);

    // Fall back to the default if the directive is syntactically invalid (e.g. user
    // set SAGE_LOG to a malformed string). Warn on stderr so the user knows their
    // directive was ignored — silent fallback would make debugging miserable.
    EnvFilter::try_new(directive)
        .unwrap_or_else(|e| {
            eprintln!("sage: invalid log directive {directive:?}: {e} — falling back to default");
            EnvFilter::new(default_filter)
        })
}

/// Detect whether JSON logging is requested via `SAGE_LOG_FORMAT`.
///
/// Returns `true` when the env var is set to `"json"` (case-insensitive).
fn use_json_format(log_format: Option<&str>) -> bool {
    log_format
        .map(|s| s.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// Initialize the global tracing subscriber.
///
/// Reads `SAGE_LOG`, `RUST_LOG`, and `SAGE_LOG_FORMAT` from the environment.
/// Call at most once per process.
pub fn init_tracing() {
    let sage_log = std::env::var("SAGE_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    let log_format = std::env::var("SAGE_LOG_FORMAT").ok();

    let filter = build_filter(sage_log.as_deref(), rust_log.as_deref());
    let use_json = use_json_format(log_format.as_deref());

    let result = if use_json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init()
    };
    // Ignore "already initialized" — expected in test harnesses and embedded scenarios.
    let _ = result;
}

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

        /// Run without the microVM sandbox (use host filesystem directly).
        ///
        /// Useful on machines without libkrunfw installed, or for fast
        /// iteration during development. Equivalent to setting
        /// `sandbox.mode: host` in the config. Task #76.
        #[arg(long)]
        dev: bool,
    },

    /// Show skill efficiency scores for an agent's collected metrics.
    ///
    /// Reads `~/.sage/agents/<name>/workspace/metrics/*.json` (per-task
    /// records written by MetricsCollector), aggregates them by skill, and
    /// prints a per-skill stats table with score = tokens_best / tokens_avg.
    /// Task #72 sub-path 3 (renamed under task #88).
    SkillScore {
        /// Agent whose metrics directory to score.
        #[arg(long)]
        agent: String,

        /// Show only skills that qualify for an automatic SkillEvaluation
        /// session (score < 0.5 AND usage_count >= 5).
        #[arg(long)]
        needs_evaluation: bool,
    },

    /// Install / list skills for an agent's workspace. Task #82.
    ///
    /// Skills live under `~/.sage/agents/<agent>/workspace/skills/<name>/`
    /// and are the agent's on-demand knowledge base (SKILL.md + optional
    /// references/). The agent consults `workspace/skills/INDEX.md` to
    /// discover what's available, then reads specific `<name>/SKILL.md`
    /// bodies via its Read tool.
    #[command(subcommand)]
    Skill(SkillAction),

    /// Initialise a new agent workspace at ~/.sage/agents/<name>/
    Init {
        /// Agent name (becomes the workspace directory name under ~/.sage/agents/)
        #[arg(long)]
        agent: String,

        /// Override LLM provider written into config.yaml (e.g., kimi, openai)
        #[arg(long)]
        provider: Option<String>,

        /// Override model id written into config.yaml (e.g., kimi-k1, gpt-4o)
        #[arg(long)]
        model: Option<String>,
    },

    /// List all registered agents in ~/.sage/agents/
    List,

    /// Validate the YAML config for the named agent
    Validate {
        /// Agent name to validate
        #[arg(long)]
        agent: String,
    },

    /// Interactive chat session with an agent in the terminal.
    ///
    /// Streams the agent's output in real time with tool call indicators.
    /// Use `--dev` to run without a VM (direct host execution, faster for dev/CI).
    Chat {
        /// Agent name (must be initialized with `sage init --agent <name>` first)
        #[arg(long)]
        agent: String,

        /// Dev mode: skip the microVM and run the bash tool directly on the host.
        /// Equivalent to `mode: host` in the agent's config.yaml.
        #[arg(long)]
        dev: bool,
    },

    /// Start an agent as a background daemon (attached to a Unix socket).
    Start {
        /// Agent name to start
        #[arg(long)]
        agent: String,
    },

    /// Connect to a running daemon and enter an interactive chat session.
    Connect {
        /// Agent name to connect to
        #[arg(long)]
        agent: String,
    },

    /// Disconnect from a daemon session without stopping it (session stays alive).
    Disconnect {
        /// Agent name to disconnect from
        #[arg(long)]
        agent: String,
    },

    /// Stop a running agent daemon.
    Stop {
        /// Agent name to stop
        #[arg(long)]
        agent: String,
    },

    /// Show the status of all running agent daemons.
    Status,

    /// Send a single message to a running agent daemon and print the reply.
    Send {
        /// Agent name
        #[arg(long)]
        agent: String,

        /// Message to send
        #[arg(long)]
        message: String,
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

    /// Multi-agent TUI: shows all running daemons in a split-panel view.
    Tui {
        /// Limit to specific agents (default: all running daemons)
        #[arg(long, value_delimiter = ',')]
        agents: Option<Vec<String>>,
    },

    /// Scheduled trigger runner: fires messages to daemons on a cron/interval schedule.
    ///
    /// Config: ~/.sage/triggers.yaml
    Triggers {
        #[command(subcommand)]
        action: TriggerAction,
    },

    /// Run a test suite against an agent using eval scripts.
    Test {
        /// Path to test suite YAML file
        #[arg(long)]
        suite: String,
        /// Reporter format
        #[arg(long, default_value = "terminal", value_parser = ["terminal", "json", "junit"])]
        reporter: String,
        /// Filter to run only cases whose name matches this glob/exact pattern.
        /// Supports * (any chars) and ? (single char). Case-sensitive.
        #[arg(long)]
        case: Option<String>,
        /// Max concurrent test cases (default: 1 = serial).
        #[arg(long, default_value = "1")]
        parallel: usize,
        /// Output directory for reporter artifacts (e.g. JUnit XML files).
        #[arg(long)]
        output: Option<String>,
    },

    /// Internal: run as a daemon server process (spawned by `sage start`).
    #[command(hide = true, name = "__daemon-server__")]
    DaemonServer {
        /// Agent name
        #[arg(long)]
        agent: String,

        /// Dev mode: skip microVM, run bash tool directly on host
        #[arg(long)]
        dev: bool,
    },
}

#[derive(Subcommand)]
enum TriggerAction {
    /// Start the trigger scheduler (runs until Ctrl+C)
    Start,
    /// List configured triggers
    List,
}

#[derive(Subcommand)]
enum SkillAction {
    /// Install a skill from a local path or git URL into the agent's workspace.
    ///
    /// Examples:
    ///   sage skill add --agent feishu ~/Dev/cc/lark-base
    ///   sage skill add --agent feishu https://github.com/larksuite/skill-base.git
    ///   sage skill add --agent feishu ~/Dev/cc/foo --name foo-alias
    Add {
        /// Agent whose workspace to install into.
        #[arg(long)]
        agent: String,

        /// Source: local path (./, /, ~), git URL (https://…, git@…, or .git).
        source: String,

        /// Override the installed skill's directory name. Default is derived
        /// from the source basename.
        #[arg(long)]
        name: Option<String>,
    },

    /// List skills installed in the agent's workspace.
    List {
        /// Agent whose workspace to scan.
        #[arg(long)]
        agent: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { agent, provider, model } => {
            tracing::info!(agent = %agent, "initializing agent workspace");
            serve::init_agent(&agent, provider.as_deref(), model.as_deref()).await
        }
        Commands::Skill(action) => match action {
            SkillAction::Add {
                agent,
                source,
                name,
            } => {
                tracing::info!(agent = %agent, source = %source, "installing skill");
                skill_install::run_skill_add(&agent, &source, name.as_deref()).await
            }
            SkillAction::List { agent } => {
                tracing::info!(agent = %agent, "listing skills");
                skill_install::run_skill_list(&agent).await
            }
        },
        Commands::List => {
            tracing::info!("listing agents");
            serve::list_agents().await
        }
        Commands::Validate { agent } => {
            tracing::info!(agent = %agent, "validating agent config");
            serve::validate_agent(&agent).await
        }
        Commands::Chat { agent, dev } => {
            tracing::info!(agent = %agent, dev = dev, "starting chat session");
            chat::run_chat(&agent, dev).await
        }
        Commands::Start { agent } => {
            tracing::info!(agent = %agent, "starting daemon");
            daemon::start_daemon(&agent, false).await
        }
        Commands::SkillScore {
            agent,
            needs_evaluation,
        } => {
            tracing::info!(agent = %agent, needs_evaluation, "scoring crafts");
            serve::run_skill_score(&agent, needs_evaluation).await
        }
        Commands::Connect { agent } => {
            tracing::info!(agent = %agent, "connecting to daemon");
            daemon::connect_interactive(&agent).await
        }
        Commands::Disconnect { agent } => {
            // Disconnect is a no-op from the CLI side — the daemon keeps running.
            // The active connection is held by the terminal; closing it disconnects.
            tracing::info!(agent = %agent, "disconnect: closing local connection");
            println!("sage: use Ctrl+C or /exit within `sage connect` to disconnect.");
            Ok(())
        }
        Commands::Stop { agent } => {
            tracing::info!(agent = %agent, "stopping daemon");
            daemon::stop_daemon(&agent).await
        }
        Commands::Status => {
            daemon::show_status().await
        }
        Commands::Send { agent, message } => {
            tracing::info!(agent = %agent, "sending message to daemon");
            daemon::send_one(&agent, &message).await
        }
        Commands::Run {
            config,
            message,
            provider,
            model,
            dev,
        } => {
            tracing::info!(config = %config, dev, "running local agent");
            serve::run_local_test(
                &config,
                &message,
                provider.as_deref(),
                model.as_deref(),
                dev,
            )
            .await
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
        Commands::Tui { agents } => {
            tui::run_tui(agents).await
        }
        Commands::Triggers { action } => match action {
            TriggerAction::Start => triggers::run_triggers().await,
            TriggerAction::List => triggers::list_triggers().await,
        },
        Commands::Test {
            suite,
            reporter,
            case,
            parallel,
            output,
        } => {
            let rep = match reporter.as_str() {
                "json" => harness::Reporter::Json,
                "junit" => harness::Reporter::Junit,
                _ => harness::Reporter::Terminal,
            };
            let pass = harness::run_test_suite(
                &suite,
                rep,
                case.as_deref(),
                parallel,
                output.as_deref(),
            )
            .await?;
            if !pass {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::DaemonServer { agent, dev } => {
            tracing::info!(agent = %agent, dev = dev, "daemon server process starting");
            daemon::run_server(&agent, dev).await
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    // ===============================================================
    // build_filter() unit tests
    // ===============================================================

    /// SAGE_LOG is present and non-empty → it wins over RUST_LOG and the default.
    #[test]
    fn build_filter_sage_log_takes_priority() {
        let f = build_filter(Some("sage=warn"), Some("info"));
        let s = f.to_string();
        // SAGE_LOG directive must be selected
        assert!(s.contains("sage=warn"), "SAGE_LOG directive should be selected");
        // The bare RUST_LOG "info" must not override
        assert!(!s.eq("info"), "RUST_LOG should be overridden when SAGE_LOG is non-empty");
    }

    /// SAGE_LOG is absent → RUST_LOG is used as fallback.
    #[test]
    fn build_filter_rust_log_fallback() {
        let f = build_filter(None, Some("debug"));
        let s = f.to_string();
        assert!(s.contains("debug"), "RUST_LOG should be used when SAGE_LOG is absent");
        // Verify we didn't accidentally fall through to the default filter
        assert!(!s.contains("sage=info"), "default filter should not appear when RUST_LOG is set");
    }

    /// Non-empty SAGE_LOG → RUST_LOG value must not be selected.
    #[test]
    fn build_filter_sage_log_excludes_rust_log_value() {
        let f = build_filter(Some("sage=debug"), Some("warn"));
        let s = f.to_string();
        assert!(s.contains("sage=debug"), "SAGE_LOG directive must be present");
        // "warn" (RUST_LOG) must not override
        assert!(!s.eq("warn"), "RUST_LOG must be ignored when SAGE_LOG is set");
    }

    /// Both absent → hard-coded default is applied.
    #[test]
    fn build_filter_default_when_both_absent() {
        let f = build_filter(None, None);
        let s = f.to_string();
        // The default is "sage=info,sage_sandbox=info,..." — check for the specific crate prefix
        assert!(s.contains("sage=info"), "default filter should include sage=info");
    }

    /// SAGE_LOG is empty string → falls through to RUST_LOG.
    #[test]
    fn build_filter_empty_sage_log_falls_through_to_rust_log() {
        let f = build_filter(Some(""), Some("error"));
        assert!(f.to_string().contains("error"));
    }

    /// Both SAGE_LOG and RUST_LOG are empty strings → default is used.
    #[test]
    fn build_filter_both_empty_uses_default() {
        let f = build_filter(Some(""), Some(""));
        let s = f.to_string();
        assert!(s.contains("sage=info"), "should fall back to default when both are empty");
    }

    /// SAGE_LOG is absent and RUST_LOG is empty → default is used.
    #[test]
    fn build_filter_absent_and_empty_rust_log_uses_default() {
        let f = build_filter(None, Some(""));
        let s = f.to_string();
        assert!(s.contains("sage=info"), "should fall back to default when RUST_LOG is empty");
    }

    // ===============================================================
    // use_json_format() unit tests
    // ===============================================================

    /// "json" (lowercase) → use JSON format.
    #[test]
    fn use_json_format_lowercase_json_returns_true() {
        assert!(use_json_format(Some("json")));
    }

    /// "JSON" (uppercase) → use JSON format (case-insensitive).
    #[test]
    fn use_json_format_uppercase_json_returns_true() {
        assert!(use_json_format(Some("JSON")));
    }

    /// Mixed-case variants → use JSON format (eq_ignore_ascii_case is case-insensitive).
    #[test]
    fn use_json_format_mixed_case_returns_true() {
        assert!(use_json_format(Some("Json")));
        assert!(use_json_format(Some("jSoN")));
    }

    /// None → text format.
    #[test]
    fn use_json_format_none_returns_false() {
        assert!(!use_json_format(None));
    }

    /// Any value other than "json" → text format.
    #[test]
    fn use_json_format_other_value_returns_false() {
        assert!(!use_json_format(Some("text")));
        assert!(!use_json_format(Some("pretty")));
        assert!(!use_json_format(Some("")));
        // Leading/trailing whitespace is not stripped — " json" != "json"
        assert!(!use_json_format(Some(" json")));
        assert!(!use_json_format(Some("json ")));
    }

    // ===============================================================
    // CLI parsing tests (pre-existing — must not be modified)
    // ===============================================================

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
                dev,
            } => {
                assert_eq!(config, "test.yaml");
                assert_eq!(message, "hi");
                assert_eq!(provider.as_deref(), Some("qwen"));
                assert_eq!(model.as_deref(), Some("qwen-plus"));
                assert!(!dev, "--dev must default to false when flag is absent");
            }
            _ => panic!("expected Run subcommand"),
        }
    }

    // ── Sprint 12 task #72 sub-path 3: `sage skill-score` subcommand ──────

    #[test]
    fn cli_skill_score_requires_agent_name() {
        // `sage skill-score` without --agent must be a clap parse error;
        // the command would be meaningless without pointing at a workspace.
        let res = Cli::try_parse_from(["sage", "skill-score"]);
        assert!(
            res.is_err(),
            "craft-score without --agent must fail clap parsing"
        );
    }

    #[test]
    fn cli_skill_score_parses_with_agent_only() {
        // Happy path: single --agent flag, no --needs-evaluation, dispatches
        // to the CraftScore variant with the expected defaults.
        let cli = Cli::parse_from(["sage", "skill-score", "--agent", "feishu"]);
        match cli.command {
            Commands::SkillScore {
                agent,
                needs_evaluation,
            } => {
                assert_eq!(agent, "feishu");
                assert!(
                    !needs_evaluation,
                    "--needs-evaluation must default to false"
                );
            }
            _ => panic!("expected CraftScore subcommand"),
        }
    }

    #[test]
    fn cli_skill_score_parses_needs_evaluation_flag() {
        // Filtered output mode: only print crafts that qualify for an
        // automatic SkillEvaluation session.
        let cli = Cli::parse_from([
            "sage",
            "skill-score",
            "--agent",
            "knowledge",
            "--needs-evaluation",
        ]);
        match cli.command {
            Commands::SkillScore {
                agent,
                needs_evaluation,
            } => {
                assert_eq!(agent, "knowledge");
                assert!(needs_evaluation, "--needs-evaluation flag must parse to true");
            }
            _ => panic!("expected CraftScore subcommand"),
        }
    }

    // ── Task #82: `sage skill add` / `sage skill list` clap parsing ──────

    #[test]
    fn cli_skill_add_parses_agent_and_source() {
        let cli = Cli::parse_from([
            "sage",
            "skill",
            "add",
            "--agent",
            "feishu",
            "~/Dev/cc/lark-base",
        ]);
        match cli.command {
            Commands::Skill(SkillAction::Add {
                agent,
                source,
                name,
            }) => {
                assert_eq!(agent, "feishu");
                assert_eq!(source, "~/Dev/cc/lark-base");
                assert!(name.is_none(), "--name must default to None");
            }
            _ => panic!("expected Skill::Add subcommand"),
        }
    }

    #[test]
    fn cli_skill_add_parses_name_override() {
        let cli = Cli::parse_from([
            "sage",
            "skill",
            "add",
            "--agent",
            "feishu",
            "https://github.com/u/r.git",
            "--name",
            "nice-alias",
        ]);
        match cli.command {
            Commands::Skill(SkillAction::Add { name, .. }) => {
                assert_eq!(name.as_deref(), Some("nice-alias"));
            }
            _ => panic!("expected Skill::Add"),
        }
    }

    #[test]
    fn cli_skill_list_requires_agent() {
        // `sage skill list` without --agent must be a clap parse error.
        let res = Cli::try_parse_from(["sage", "skill", "list"]);
        assert!(res.is_err(), "skill list without --agent must fail clap");
    }

    #[test]
    fn cli_skill_list_parses_agent() {
        let cli = Cli::parse_from(["sage", "skill", "list", "--agent", "feishu"]);
        match cli.command {
            Commands::Skill(SkillAction::List { agent }) => {
                assert_eq!(agent, "feishu");
            }
            _ => panic!("expected Skill::List"),
        }
    }

    #[test]
    fn cli_run_with_dev_flag_parses() {
        // Task #76: --dev flag toggles Sandbox::Host for machines without
        // libkrunfw. The flag is a bool presence switch (clap default).
        let cli = Cli::parse_from([
            "sage",
            "run",
            "--config",
            "test.yaml",
            "--message",
            "hi",
            "--dev",
        ]);
        match cli.command {
            Commands::Run { dev, .. } => assert!(dev, "--dev must parse to true"),
            _ => panic!("expected Run subcommand"),
        }
    }

    // ── M1: Agent Registry CLI ───────────────────────────────────────────────

    #[test]
    fn cli_init_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "feishu"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Init subcommand"),
        }
    }

    #[test]
    fn cli_list_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "list"]);
        assert!(matches!(cli.command, Commands::List));
    }

    #[test]
    fn cli_validate_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "validate", "--agent", "feishu"]);
        match cli.command {
            Commands::Validate { agent } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Validate subcommand"),
        }
    }

    #[test]
    fn cli_init_missing_agent_flag_fails() {
        let result = Cli::try_parse_from(["sage", "init"]);
        assert!(result.is_err(), "init without --agent must fail");
    }

    #[test]
    fn cli_validate_missing_agent_flag_fails() {
        let result = Cli::try_parse_from(["sage", "validate"]);
        assert!(result.is_err(), "validate without --agent must fail");
    }

    #[test]
    fn cli_init_agent_name_with_hyphens() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "my-coding-agent"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "my-coding-agent"),
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_agent_name_with_underscores() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "coding_agent"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "coding_agent"),
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_list_does_not_accept_unknown_flags() {
        // `sage list` takes no flags — unknown flag should fail
        let result = Cli::try_parse_from(["sage", "list", "--unknown"]);
        assert!(result.is_err(), "list must reject unknown flags");
    }

    // ── M1: boundary — agent name formats ────────────────────────────────────

    #[test]
    fn cli_init_unicode_agent_name_accepted_by_parser() {
        // Unicode/Chinese agent names pass through the CLI layer unchanged
        let cli = Cli::parse_from(["sage", "init", "--agent", "飞书助手"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "飞书助手"),
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_validate_unicode_agent_name_accepted_by_parser() {
        let cli = Cli::parse_from(["sage", "validate", "--agent", "my-agent-中文"]);
        match cli.command {
            Commands::Validate { agent } => assert_eq!(agent, "my-agent-中文"),
            _ => panic!("expected Validate"),
        }
    }

    #[test]
    fn cli_init_path_traversal_name_accepted_by_parser() {
        // The CLI parser accepts any string — path sanitization (rejecting `..` components,
        // empty names, etc.) is the responsibility of `serve::init_agent()`, not the parser.
        // This test documents that the parser does NOT sanitize, so reviewers know where
        // the validation boundary is.
        let cli = Cli::parse_from(["sage", "init", "--agent", "../../etc/passwd"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "../../etc/passwd"),
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_empty_agent_name_accepted_by_parser() {
        // Empty string passes the parser — validation is the handler's responsibility
        let cli = Cli::parse_from(["sage", "init", "--agent", ""]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, ""),
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_numeric_only_agent_name() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "42"]);
        match cli.command {
            Commands::Init { agent, .. } => assert_eq!(agent, "42"),
            _ => panic!("expected Init"),
        }
    }

    // ── M4: init --provider / --model flags ──────────────────────────────────

    #[test]
    fn cli_init_provider_flag_parsed() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "feishu", "--provider", "kimi"]);
        match cli.command {
            Commands::Init { agent, provider, model } => {
                assert_eq!(agent, "feishu");
                assert_eq!(provider.as_deref(), Some("kimi"));
                assert!(model.is_none(), "model must be None when not supplied");
            }
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_model_flag_parsed() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "feishu", "--model", "kimi-k1"]);
        match cli.command {
            Commands::Init { agent, provider, model } => {
                assert_eq!(agent, "feishu");
                assert_eq!(model.as_deref(), Some("kimi-k1"));
                assert!(provider.is_none(), "provider must be None when not supplied");
            }
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_provider_and_model_flags_parsed() {
        let cli = Cli::parse_from([
            "sage", "init", "--agent", "feishu",
            "--provider", "openai", "--model", "gpt-4o",
        ]);
        match cli.command {
            Commands::Init { agent, provider, model } => {
                assert_eq!(agent, "feishu");
                assert_eq!(provider.as_deref(), Some("openai"));
                assert_eq!(model.as_deref(), Some("gpt-4o"));
            }
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn cli_init_without_provider_model_defaults_to_none() {
        let cli = Cli::parse_from(["sage", "init", "--agent", "feishu"]);
        match cli.command {
            Commands::Init { provider, model, .. } => {
                assert!(provider.is_none(), "provider must default to None");
                assert!(model.is_none(), "model must default to None");
            }
            _ => panic!("expected Init"),
        }
    }

    // ── M1: error paths ──────────────────────────────────────────────────────

    #[test]
    fn cli_list_rejects_agent_flag() {
        // spec: `list` takes no arguments at all — not even --agent
        let result = Cli::try_parse_from(["sage", "list", "--agent", "feishu"]);
        assert!(result.is_err(), "list must not accept --agent flag");
    }

    #[test]
    fn cli_init_extra_positional_arg_fails() {
        // positional arg not accepted
        let result = Cli::try_parse_from(["sage", "init", "--agent", "feishu", "extra"]);
        assert!(result.is_err(), "init must reject extra positional arguments");
    }

    #[test]
    fn cli_validate_extra_positional_arg_fails() {
        let result = Cli::try_parse_from(["sage", "validate", "--agent", "feishu", "extra"]);
        assert!(result.is_err(), "validate must reject extra positional arguments");
    }

    // ── Sprint 3 — v0.8: Chat command ────────────────────────────────────────

    #[test]
    fn cli_chat_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "chat", "--agent", "feishu"]);
        match cli.command {
            Commands::Chat { agent, dev } => {
                assert_eq!(agent, "feishu");
                assert!(!dev, "dev should default to false");
            }
            _ => panic!("expected Chat"),
        }
    }

    #[test]
    fn cli_chat_dev_flag_sets_dev_mode() {
        let cli = Cli::parse_from(["sage", "chat", "--agent", "feishu", "--dev"]);
        match cli.command {
            Commands::Chat { agent, dev } => {
                assert_eq!(agent, "feishu");
                assert!(dev, "dev flag must be true when --dev is passed");
            }
            _ => panic!("expected Chat"),
        }
    }

    #[test]
    fn cli_chat_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "chat"]);
        assert!(result.is_err(), "chat without --agent must fail");
    }

    #[test]
    fn cli_chat_unicode_agent_name() {
        let cli = Cli::parse_from(["sage", "chat", "--agent", "飞书助手"]);
        match cli.command {
            Commands::Chat { agent, .. } => assert_eq!(agent, "飞书助手"),
            _ => panic!("expected Chat"),
        }
    }

    #[test]
    fn cli_chat_extra_positional_arg_fails() {
        let result = Cli::try_parse_from(["sage", "chat", "--agent", "feishu", "extra"]);
        assert!(result.is_err(), "chat must reject extra positional arguments");
    }

    // ── Sprint 3 — v0.8: Start / Stop / Connect / Status / Send ─────────────

    #[test]
    fn cli_start_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "start", "--agent", "feishu"]);
        match cli.command {
            Commands::Start { agent } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn cli_start_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "start"]);
        assert!(result.is_err(), "start without --agent must fail");
    }

    #[test]
    fn cli_stop_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "stop", "--agent", "feishu"]);
        match cli.command {
            Commands::Stop { agent } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Stop"),
        }
    }

    #[test]
    fn cli_stop_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "stop"]);
        assert!(result.is_err(), "stop without --agent must fail");
    }

    #[test]
    fn cli_connect_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "connect", "--agent", "feishu"]);
        match cli.command {
            Commands::Connect { agent } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn cli_connect_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "connect"]);
        assert!(result.is_err(), "connect without --agent must fail");
    }

    #[test]
    fn cli_disconnect_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "disconnect", "--agent", "feishu"]);
        match cli.command {
            Commands::Disconnect { agent } => assert_eq!(agent, "feishu"),
            _ => panic!("expected Disconnect"),
        }
    }

    #[test]
    fn cli_disconnect_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "disconnect"]);
        assert!(result.is_err(), "disconnect without --agent must fail");
    }

    #[test]
    fn cli_status_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "status"]);
        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn cli_status_does_not_accept_flags() {
        let result = Cli::try_parse_from(["sage", "status", "--agent", "feishu"]);
        assert!(result.is_err(), "status takes no flags");
    }

    #[test]
    fn cli_send_subcommand_parses() {
        let cli = Cli::parse_from(["sage", "send", "--agent", "feishu", "--message", "hello"]);
        match cli.command {
            Commands::Send { agent, message } => {
                assert_eq!(agent, "feishu");
                assert_eq!(message, "hello");
            }
            _ => panic!("expected Send"),
        }
    }

    #[test]
    fn cli_send_requires_message_flag() {
        let result = Cli::try_parse_from(["sage", "send", "--agent", "feishu"]);
        assert!(result.is_err(), "send without --message must fail");
    }

    #[test]
    fn cli_send_requires_agent_flag() {
        let result = Cli::try_parse_from(["sage", "send", "--message", "hello"]);
        assert!(result.is_err(), "send without --agent must fail");
    }

    #[test]
    fn cli_send_message_with_spaces() {
        let cli = Cli::parse_from([
            "sage", "send", "--agent", "feishu", "--message", "hello world today",
        ]);
        match cli.command {
            Commands::Send { message, .. } => assert_eq!(message, "hello world today"),
            _ => panic!("expected Send"),
        }
    }

    // ── daemon commands: boundary agent names ─────────────────────────────────

    #[test]
    fn cli_start_unicode_agent_name() {
        let cli = Cli::parse_from(["sage", "start", "--agent", "飞书助手"]);
        match cli.command {
            Commands::Start { agent } => assert_eq!(agent, "飞书助手"),
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn cli_stop_unicode_agent_name() {
        let cli = Cli::parse_from(["sage", "stop", "--agent", "飞书助手"]);
        match cli.command {
            Commands::Stop { agent } => assert_eq!(agent, "飞书助手"),
            _ => panic!("expected Stop"),
        }
    }

    #[test]
    fn cli_connect_unicode_agent_name() {
        let cli = Cli::parse_from(["sage", "connect", "--agent", "my-agent-中文"]);
        match cli.command {
            Commands::Connect { agent } => assert_eq!(agent, "my-agent-中文"),
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn cli_send_unicode_agent_name() {
        let cli = Cli::parse_from(["sage", "send", "--agent", "飞书助手", "--message", "你好"]);
        match cli.command {
            Commands::Send { agent, message } => {
                assert_eq!(agent, "飞书助手");
                assert_eq!(message, "你好");
            }
            _ => panic!("expected Send"),
        }
    }

    #[test]
    fn cli_start_path_traversal_accepted_by_parser() {
        // Parser accepts any string — path validation is the handler's responsibility
        let cli = Cli::parse_from(["sage", "start", "--agent", "../../etc/passwd"]);
        match cli.command {
            Commands::Start { agent } => assert_eq!(agent, "../../etc/passwd"),
            _ => panic!("expected Start"),
        }
    }
}
