//! coding-agent — Interactive coding agent CLI.
//!
//! Entry point translated from pi-mono `packages/coding-agent/src/main.ts`.
//!
//! Parses CLI arguments and dispatches to the appropriate run mode:
//! - Interactive TUI (`InteractiveMode`)
//! - Print / single-shot (`run_print_mode`)
//! - RPC JSON server (`run_rpc_mode`)

// Many items in the binary's submodules expose API surface that is only wired
// up for specific features or future expansion; silence dead_code at the
// module level to keep the binary building cleanly.
#[allow(dead_code)]
mod agent_session;
#[allow(dead_code)]
mod bun;
#[allow(dead_code)]
mod cli;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod core;
#[allow(dead_code)]
mod migrations;
#[allow(dead_code)]
mod modes;
#[allow(dead_code)]
mod utils;

use std::env;

use cli::args::{Args, Mode, parse_args, print_help};
use config::{APP_NAME, VERSION, get_agent_dir, is_truthy_env_flag};
use core::model_registry::ModelRegistry;
use core::settings_manager::SettingsManager;
use modes::interactive::{InteractiveMode, InteractiveModeOptions};

// ============================================================================
// Package command handling
// ============================================================================

#[allow(dead_code)]
type PackageCommand = &'static str;

fn get_package_command_usage(command: &str) -> String {
    match command {
        "install" => format!("{APP_NAME} install <source> [-l]"),
        "remove" => format!("{APP_NAME} remove <source> [-l]"),
        "update" => format!("{APP_NAME} update [source]"),
        "list" => format!("{APP_NAME} list"),
        _ => format!("{APP_NAME} {command}"),
    }
}

fn print_package_command_help(command: &str) {
    match command {
        "install" => println!(
            "Usage:\n  {}\n\nInstall a package and add it to settings.\n\nOptions:\n  -l, --local    Install project-locally\n",
            get_package_command_usage("install")
        ),
        "remove" => println!(
            "Usage:\n  {}\n\nRemove a package and its source from settings.\n",
            get_package_command_usage("remove")
        ),
        "update" => println!(
            "Usage:\n  {}\n\nUpdate installed packages.\n",
            get_package_command_usage("update")
        ),
        "list" => println!(
            "Usage:\n  {}\n\nList installed packages from user and project settings.\n",
            get_package_command_usage("list")
        ),
        _ => {}
    }
}

#[allow(dead_code)]
struct PackageCommandOptions {
    command: String,
    source: Option<String>,
    local: bool,
    help: bool,
    invalid_option: Option<String>,
}

fn parse_package_command(args: &[String]) -> Option<PackageCommandOptions> {
    let raw_command = args.first()?;
    let rest = &args[1..];

    let command = match raw_command.as_str() {
        "uninstall" => "remove",
        "install" | "remove" | "update" | "list" => raw_command.as_str(),
        _ => return None,
    }
    .to_string();

    let mut local = false;
    let mut help = false;
    let mut invalid_option: Option<String> = None;
    let mut source: Option<String> = None;

    for arg in rest {
        if arg == "-h" || arg == "--help" {
            help = true;
        } else if arg == "-l" || arg == "--local" {
            if command == "install" || command == "remove" {
                local = true;
            } else {
                invalid_option.get_or_insert_with(|| arg.clone());
            }
        } else if arg.starts_with('-') {
            invalid_option.get_or_insert_with(|| arg.clone());
        } else if source.is_none() {
            source = Some(arg.clone());
        }
    }

    Some(PackageCommandOptions {
        command,
        source,
        local,
        help,
        invalid_option,
    })
}

async fn handle_package_command(args: &[String]) -> bool {
    let Some(opts) = parse_package_command(args) else {
        return false;
    };

    if opts.help {
        print_package_command_help(&opts.command);
        return true;
    }

    if let Some(ref invalid) = opts.invalid_option {
        eprintln!("Unknown option {invalid} for \"{}\".", opts.command);
        eprintln!(
            "Use \"{APP_NAME} --help\" or \"{}\".",
            get_package_command_usage(&opts.command)
        );
        std::process::exit(1);
    }

    if (opts.command == "install" || opts.command == "remove") && opts.source.is_none() {
        eprintln!("Missing {} source.", opts.command);
        eprintln!("Usage: {}", get_package_command_usage(&opts.command));
        std::process::exit(1);
    }

    // In the Rust port, package management is not yet implemented.
    eprintln!("Package management not yet implemented in this build.");
    std::process::exit(1);
}

// ============================================================================
// Settings error reporting
// ============================================================================

fn report_settings_errors(settings_manager: &mut SettingsManager, context: &str) {
    for err in settings_manager.drain_errors() {
        eprintln!("Warning ({context}, {} settings): {}", err.scope, err.error);
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    let raw_args: Vec<String> = env::args().skip(1).collect();

    // Check for offline mode early
    let offline_mode = raw_args.contains(&"--offline".to_string())
        || env::var("PI_OFFLINE")
            .map(|v| is_truthy_env_flag(&v))
            .unwrap_or(false);

    if offline_mode {
        unsafe {
            env::set_var("PI_OFFLINE", "1");
            env::set_var("PI_SKIP_VERSION_CHECK", "1");
        }
    }

    // Handle package sub-commands (install, remove, update, list)
    if handle_package_command(&raw_args).await {
        return;
    }

    // First pass: parse without extension flags
    let first_pass = parse_args(&raw_args, None);

    // Set up core services
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let agent_dir = get_agent_dir();
    let mut settings_manager = SettingsManager::create(&cwd, &agent_dir);
    report_settings_errors(&mut settings_manager, "startup");

    let models_path = config::get_models_path();
    let _model_registry = ModelRegistry::new(&models_path);

    // Second pass (same as first for now — extension flags not yet supported)
    let parsed = first_pass;

    // Handle --version
    if parsed.version {
        println!("{VERSION}");
        return;
    }

    // Handle --help
    if parsed.help {
        print_help(APP_NAME);
        return;
    }

    // Handle --list-models
    if let Some(pattern) = &parsed.list_models {
        list_models(&_model_registry, pattern.as_deref());
        return;
    }

    // Read piped stdin for non-RPC modes
    let stdin_content: Option<String> = if parsed.mode.as_ref() != Some(&Mode::Rpc) {
        read_piped_stdin()
    } else {
        None
    };

    // If stdin content was provided, force print mode
    let mut parsed = parsed;
    if stdin_content.is_some() {
        parsed.print = true;
    }

    // Determine run mode
    let is_interactive = !parsed.print && parsed.mode.is_none();
    let mode = parsed.mode.clone().unwrap_or(Mode::Text);

    // Build initial message
    let initial_message = build_initial_message(&parsed, stdin_content.as_deref());

    if mode == Mode::Rpc {
        eprintln!("RPC mode not yet connected to agent session in this build.");
        std::process::exit(1);
    } else if is_interactive {
        let options = InteractiveModeOptions {
            initial_message: initial_message.clone(),
            verbose: parsed.verbose,
            model_fallback_message: None,
            migrated_providers: vec![],
        };
        let mut interactive = InteractiveMode::new(options);
        if let Err(e) = interactive.run().await {
            eprintln!("Interactive mode error: {e}");
            std::process::exit(1);
        }
    } else {
        // Print mode
        let all_messages = {
            let mut parts: Vec<String> = initial_message.into_iter().collect();
            parts.extend_from_slice(&parsed.messages);
            parts.join("\n")
        };
        if let Err(e) = agent_session::run_agent_session(
            all_messages,
            parsed.model.clone(),
            parsed.provider.clone(),
            parsed.api_key.clone(),
        )
        .await
        {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Read all content from piped stdin.
/// Returns `None` if stdin is a TTY (interactive terminal).
fn read_piped_stdin() -> Option<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return None;
    }
    use std::io::Read;
    let mut content = String::new();
    let _ = std::io::stdin().read_to_string(&mut content);
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Build the initial message from CLI messages and optional stdin content.
fn build_initial_message(parsed: &Args, stdin_content: Option<&str>) -> Option<String> {
    let parts: Vec<String> = parsed
        .messages
        .iter()
        .cloned()
        .chain(stdin_content.map(str::to_string))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// List available models, optionally filtered by a search pattern.
fn list_models(registry: &ModelRegistry, pattern: Option<&str>) {
    let models = registry.find_all(pattern);
    if models.is_empty() {
        if let Some(p) = pattern {
            eprintln!("No models found matching \"{p}\"");
        } else {
            eprintln!(
                "No models loaded. Create {} to add models.",
                config::get_models_path().display()
            );
        }
        return;
    }
    for m in models {
        let name = m.name.as_deref().unwrap_or(&m.id);
        println!("{}/{} — {}", m.provider, m.id, name);
    }
}

/// Minimal print-mode runner (placeholder until AgentSession is wired up).
#[allow(dead_code)]
async fn run_print_mode_simple(
    initial_message: Option<&str>,
    extra_messages: &[String],
    mode: &Mode,
) {
    // In a full implementation, this would create an AgentSession, send the
    // messages, and output the result. For now we print a placeholder.
    match mode {
        Mode::Json => {
            let event = serde_json::json!({
                "type": "info",
                "message": "Agent session not yet connected in this build."
            });
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
        }
        Mode::Text | Mode::Rpc => {
            if let Some(msg) = initial_message {
                eprintln!("[sage] Would send to agent: {msg}");
            }
            for msg in extra_messages {
                eprintln!("[sage] Would send to agent: {msg}");
            }
            eprintln!("[sage] Agent session not yet connected in this build.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cli::args::Args;

    fn sv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn build_initial_message_from_cli_args() {
        let args = Args {
            messages: vec!["hello".to_string(), "world".to_string()],
            ..Default::default()
        };
        let msg = build_initial_message(&args, None);
        assert_eq!(msg.as_deref(), Some("hello\nworld"));
    }

    #[test]
    fn build_initial_message_from_stdin() {
        let args = Args::default();
        let msg = build_initial_message(&args, Some("stdin content"));
        assert_eq!(msg.as_deref(), Some("stdin content"));
    }

    #[test]
    fn build_initial_message_combined() {
        let args = Args {
            messages: vec!["cli msg".to_string()],
            ..Default::default()
        };
        let msg = build_initial_message(&args, Some("from stdin"));
        assert_eq!(msg.as_deref(), Some("cli msg\nfrom stdin"));
    }

    #[test]
    fn build_initial_message_empty() {
        let args = Args::default();
        let msg = build_initial_message(&args, None);
        assert!(msg.is_none());
    }

    #[test]
    fn get_package_command_usage_install() {
        let usage = get_package_command_usage("install");
        assert!(usage.contains("install"));
        assert!(usage.contains("<source>"));
    }

    #[test]
    fn parse_package_command_install() {
        let args = sv(&["install", "npm:@foo/bar"]);
        let opts = parse_package_command(&args);
        assert!(opts.is_some());
        let opts = opts.unwrap();
        assert_eq!(opts.command, "install");
        assert_eq!(opts.source.as_deref(), Some("npm:@foo/bar"));
        assert!(!opts.local);
    }

    #[test]
    fn parse_package_command_install_local() {
        let args = sv(&["install", "npm:@foo/bar", "-l"]);
        let opts = parse_package_command(&args).unwrap();
        assert!(opts.local);
    }

    #[test]
    fn parse_package_command_uninstall_alias() {
        let args = sv(&["uninstall", "npm:@foo/bar"]);
        let opts = parse_package_command(&args).unwrap();
        assert_eq!(opts.command, "remove");
    }

    #[test]
    fn parse_package_command_list() {
        let args = sv(&["list"]);
        let opts = parse_package_command(&args).unwrap();
        assert_eq!(opts.command, "list");
        assert!(opts.source.is_none());
    }

    #[test]
    fn parse_package_command_not_recognized() {
        let args = sv(&["chat"]);
        assert!(parse_package_command(&args).is_none());
    }

    #[test]
    fn parse_package_command_help_flag() {
        let args = sv(&["install", "--help"]);
        let opts = parse_package_command(&args).unwrap();
        assert!(opts.help);
    }

    #[test]
    fn parse_package_command_invalid_option() {
        let args = sv(&["install", "src", "--nope"]);
        let opts = parse_package_command(&args).unwrap();
        assert_eq!(opts.invalid_option.as_deref(), Some("--nope"));
    }

    #[test]
    fn is_truthy_env_flag_in_main() {
        assert!(is_truthy_env_flag("1"));
        assert!(is_truthy_env_flag("yes"));
        assert!(!is_truthy_env_flag("no"));
    }
}
