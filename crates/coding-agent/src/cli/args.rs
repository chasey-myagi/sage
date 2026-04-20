//! CLI argument parsing for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/args.ts`.

use std::collections::HashMap;

/// Output mode for non-interactive / RPC operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    /// Plain-text output of the final assistant reply.
    #[default]
    Text,
    /// Newline-delimited JSON stream of all session events.
    Json,
    /// JSON-RPC 2.0 server on stdin/stdout (for editor integrations).
    Rpc,
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(Mode::Text),
            "json" => Ok(Mode::Json),
            "rpc" => Ok(Mode::Rpc),
            other => Err(format!("unknown mode: {other}")),
        }
    }
}

/// Thinking / reasoning budget level (mirrors pi-ai `ThinkingLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl std::str::FromStr for ThinkingLevel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(ThinkingLevel::Off),
            "minimal" => Ok(ThinkingLevel::Minimal),
            "low" => Ok(ThinkingLevel::Low),
            "medium" => Ok(ThinkingLevel::Medium),
            "high" => Ok(ThinkingLevel::High),
            "xhigh" => Ok(ThinkingLevel::Xhigh),
            other => Err(format!(
                "invalid thinking level \"{other}\". Valid values: off, minimal, low, medium, high, xhigh"
            )),
        }
    }
}

impl std::fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Minimal => "minimal",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::Xhigh => "xhigh",
        };
        write!(f, "{s}")
    }
}

/// All available tool names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolName {
    Read,
    Bash,
    Edit,
    Write,
    Grep,
    Find,
    Ls,
}

impl std::str::FromStr for ToolName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(ToolName::Read),
            "bash" => Ok(ToolName::Bash),
            "edit" => Ok(ToolName::Edit),
            "write" => Ok(ToolName::Write),
            "grep" => Ok(ToolName::Grep),
            "find" => Ok(ToolName::Find),
            "ls" => Ok(ToolName::Ls),
            other => Err(format!("unknown tool: {other}")),
        }
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ToolName::Read => "read",
            ToolName::Bash => "bash",
            ToolName::Edit => "edit",
            ToolName::Write => "write",
            ToolName::Grep => "grep",
            ToolName::Find => "find",
            ToolName::Ls => "ls",
        };
        write!(f, "{s}")
    }
}

/// All known tool names as a slice.
pub const ALL_TOOL_NAMES: &[&str] = &["read", "bash", "edit", "write", "grep", "find", "ls"];

/// Parsed CLI arguments. Mirrors the `Args` interface in `cli/args.ts`.
#[derive(Debug, Default)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub thinking: Option<ThinkingLevel>,
    pub continue_session: bool,
    pub resume: bool,
    pub help: bool,
    pub version: bool,
    pub mode: Option<Mode>,
    pub no_session: bool,
    pub session: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub models: Option<Vec<String>>,
    pub tools: Option<Vec<ToolName>>,
    pub no_tools: bool,
    pub extensions: Option<Vec<String>>,
    pub no_extensions: bool,
    pub print: bool,
    pub export: Option<String>,
    pub no_skills: bool,
    pub skills: Option<Vec<String>>,
    pub prompt_templates: Option<Vec<String>>,
    pub no_prompt_templates: bool,
    pub themes: Option<Vec<String>>,
    pub no_themes: bool,
    /// `None` = not requested; `Some(None)` = `--list-models`; `Some(Some(pat))` = `--list-models <pat>`
    pub list_models: Option<Option<String>>,
    pub offline: bool,
    pub verbose: bool,
    /// Free-form message strings (positional).
    pub messages: Vec<String>,
    /// `@file` arguments (the `@` prefix is stripped).
    pub file_args: Vec<String>,
    /// Extension-registered flags collected during the second parse pass.
    pub unknown_flags: HashMap<String, FlagValue>,
}

/// Value carried by an extension-registered flag.
#[derive(Debug, Clone)]
pub enum FlagValue {
    Bool(bool),
    Str(String),
}

/// Registered extension flag metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagType {
    Boolean,
    String,
}

/// Parse `argv` (without the program name) into [`Args`].
///
/// An optional `extension_flags` map provides the names and types of flags
/// registered by loaded extensions; unknown flags are silently ignored on the
/// first pass but recorded on the second.
pub fn parse_args(args: &[String], extension_flags: Option<&HashMap<String, FlagType>>) -> Args {
    let mut result = Args::default();
    let mut i = 0;
    let len = args.len();

    while i < len {
        let arg = &args[i];

        if arg == "--help" || arg == "-h" {
            result.help = true;
        } else if arg == "--version" || arg == "-v" {
            result.version = true;
        } else if arg == "--mode" && i + 1 < len {
            i += 1;
            result.mode = args[i].parse().ok();
        } else if arg == "--continue" || arg == "-c" {
            result.continue_session = true;
        } else if arg == "--resume" || arg == "-r" {
            result.resume = true;
        } else if arg == "--provider" && i + 1 < len {
            i += 1;
            result.provider = Some(args[i].clone());
        } else if arg == "--model" && i + 1 < len {
            i += 1;
            result.model = Some(args[i].clone());
        } else if arg == "--api-key" && i + 1 < len {
            i += 1;
            result.api_key = Some(args[i].clone());
        } else if arg == "--system-prompt" && i + 1 < len {
            i += 1;
            result.system_prompt = Some(args[i].clone());
        } else if arg == "--append-system-prompt" && i + 1 < len {
            i += 1;
            result.append_system_prompt = Some(args[i].clone());
        } else if arg == "--no-session" {
            result.no_session = true;
        } else if arg == "--session" && i + 1 < len {
            i += 1;
            result.session = Some(args[i].clone());
        } else if arg == "--fork" && i + 1 < len {
            i += 1;
            result.fork = Some(args[i].clone());
        } else if arg == "--session-dir" && i + 1 < len {
            i += 1;
            result.session_dir = Some(args[i].clone());
        } else if arg == "--models" && i + 1 < len {
            i += 1;
            let patterns = args[i].split(',').map(|s| s.trim().to_string()).collect();
            result.models = Some(patterns);
        } else if arg == "--no-tools" {
            result.no_tools = true;
        } else if arg == "--tools" && i + 1 < len {
            i += 1;
            let mut valid = Vec::new();
            for name in args[i].split(',').map(|s| s.trim()) {
                match name.parse::<ToolName>() {
                    Ok(t) => valid.push(t),
                    Err(_) => eprintln!(
                        "Warning: Unknown tool \"{name}\". Valid tools: {}",
                        ALL_TOOL_NAMES.join(", ")
                    ),
                }
            }
            result.tools = Some(valid);
        } else if arg == "--thinking" && i + 1 < len {
            i += 1;
            match args[i].parse::<ThinkingLevel>() {
                Ok(level) => result.thinking = Some(level),
                Err(e) => eprintln!("Warning: {e}"),
            }
        } else if arg == "--print" || arg == "-p" {
            result.print = true;
        } else if arg == "--export" && i + 1 < len {
            i += 1;
            result.export = Some(args[i].clone());
        } else if (arg == "--extension" || arg == "-e") && i + 1 < len {
            i += 1;
            result
                .extensions
                .get_or_insert_default()
                .push(args[i].clone());
        } else if arg == "--no-extensions" || arg == "-ne" {
            result.no_extensions = true;
        } else if arg == "--skill" && i + 1 < len {
            i += 1;
            result.skills.get_or_insert_default().push(args[i].clone());
        } else if arg == "--prompt-template" && i + 1 < len {
            i += 1;
            result
                .prompt_templates
                .get_or_insert_default()
                .push(args[i].clone());
        } else if arg == "--theme" && i + 1 < len {
            i += 1;
            result.themes.get_or_insert_default().push(args[i].clone());
        } else if arg == "--no-skills" || arg == "-ns" {
            result.no_skills = true;
        } else if arg == "--no-prompt-templates" || arg == "-np" {
            result.no_prompt_templates = true;
        } else if arg == "--no-themes" {
            result.no_themes = true;
        } else if arg == "--list-models" {
            // Check if next arg is a search pattern (not a flag or @file)
            if i + 1 < len && !args[i + 1].starts_with('-') && !args[i + 1].starts_with('@') {
                i += 1;
                result.list_models = Some(Some(args[i].clone()));
            } else {
                result.list_models = Some(None);
            }
        } else if arg == "--verbose" {
            result.verbose = true;
        } else if arg == "--offline" {
            result.offline = true;
        } else if let Some(stripped) = arg.strip_prefix('@') {
            result.file_args.push(stripped.to_string());
        } else if let Some(flag_name) = arg.strip_prefix("--") {
            if let Some(flags) = extension_flags
                && let Some(flag_type) = flags.get(flag_name)
            {
                match flag_type {
                    FlagType::Boolean => {
                        result
                            .unknown_flags
                            .insert(flag_name.to_string(), FlagValue::Bool(true));
                    }
                    FlagType::String if i + 1 < len => {
                        i += 1;
                        result
                            .unknown_flags
                            .insert(flag_name.to_string(), FlagValue::Str(args[i].clone()));
                    }
                    _ => {}
                }
            }
            // Unknown flags without extension_flags are silently ignored (first pass)
        } else if !arg.starts_with('-') {
            result.messages.push(arg.clone());
        }

        i += 1;
    }

    result
}

/// Print the full help message (mirrors `printHelp()` in `cli/args.ts`).
pub fn print_help(app_name: &str) {
    println!(
        r#"{app_name} - AI coding assistant with read, bash, edit, write tools

Usage:
  {app_name} [options] [@files...] [messages...]

Commands:
  {app_name} install <source> [-l]     Install extension source and add to settings
  {app_name} remove <source> [-l]      Remove extension source from settings
  {app_name} uninstall <source> [-l]   Alias for remove
  {app_name} update [source]           Update installed extensions (skips pinned sources)
  {app_name} list                      List installed extensions from settings
  {app_name} config                    Open TUI to enable/disable package resources
  {app_name} <command> --help          Show help for install/remove/uninstall/update/list

Options:
  --provider <name>              Provider name (default: google)
  --model <pattern>              Model pattern or ID (supports "provider/id" and optional ":<thinking>")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --append-system-prompt <text>  Append text or file contents to the system prompt
  --mode <mode>                  Output mode: text (default), json, or rpc
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path>               Use specific session file
  --fork <path>                  Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --models <patterns>            Comma-separated model patterns for cycling
  --no-tools                     Disable all built-in tools
  --tools <tools>                Comma-separated list of tools to enable (default: read,bash,edit,write)
                                 Available: read, bash, edit, write, grep, find, ls
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh
  --extension, -e <path>         Load an extension file (can be used multiple times)
  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)
  --skill <path>                 Load a skill file or directory (can be used multiple times)
  --no-skills, -ns               Disable skills discovery and loading
  --prompt-template <path>       Load a prompt template file or directory
  --no-prompt-templates, -np     Disable prompt template discovery and loading
  --theme <path>                 Load a theme file or directory
  --no-themes                    Disable theme discovery and loading
  --export <file>                Export session file to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --offline                      Disable startup network operations (same as PI_OFFLINE=1)
  --help, -h                     Show this help
  --version, -v                  Show version number

Examples:
  # Interactive mode
  {app_name}

  # Interactive mode with initial prompt
  {app_name} "List all .rs files in src/"

  # Non-interactive mode (process and exit)
  {app_name} -p "List all .rs files in src/"

  # Use different model
  {app_name} --provider anthropic --model claude-sonnet "Help me refactor this code"

  # Use model with provider prefix
  {app_name} --model openai/gpt-4o "Help me refactor this code"

  # Limit model cycling to specific models
  {app_name} --models claude-sonnet,claude-haiku,gpt-4o

  # Read-only mode (no file modifications possible)
  {app_name} --tools read,grep,find,ls -p "Review the code in src/"

Environment Variables:
  ANTHROPIC_API_KEY                - Anthropic Claude API key
  OPENAI_API_KEY                   - OpenAI GPT API key
  GEMINI_API_KEY                   - Google Gemini API key
  SAGE_CODING_AGENT_DIR            - Session storage directory (default: ~/.sage/coding-agent)
  PI_OFFLINE                       - Disable startup network operations when set to 1/true/yes

Available Tools (default: read, bash, edit, write):
  read   - Read file contents
  bash   - Execute bash commands
  edit   - Edit files with find/replace
  write  - Write files (creates/overwrites)
  grep   - Search file contents (read-only, off by default)
  find   - Find files by glob pattern (read-only, off by default)
  ls     - List directory contents (read-only, off by default)
"#,
        app_name = app_name
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_help_flag() {
        let args = parse_args(&sv(&["--help"]), None);
        assert!(args.help);
    }

    #[test]
    fn parse_short_help_flag() {
        let args = parse_args(&sv(&["-h"]), None);
        assert!(args.help);
    }

    #[test]
    fn parse_version_flag() {
        let args = parse_args(&sv(&["--version"]), None);
        assert!(args.version);
    }

    #[test]
    fn parse_short_version_flag() {
        let args = parse_args(&sv(&["-v"]), None);
        assert!(args.version);
    }

    #[test]
    fn parse_model_and_provider() {
        let args = parse_args(
            &sv(&["--provider", "anthropic", "--model", "claude-sonnet"]),
            None,
        );
        assert_eq!(args.provider.as_deref(), Some("anthropic"));
        assert_eq!(args.model.as_deref(), Some("claude-sonnet"));
    }

    #[test]
    fn parse_mode_text() {
        let args = parse_args(&sv(&["--mode", "text"]), None);
        assert_eq!(args.mode, Some(Mode::Text));
    }

    #[test]
    fn parse_mode_json() {
        let args = parse_args(&sv(&["--mode", "json"]), None);
        assert_eq!(args.mode, Some(Mode::Json));
    }

    #[test]
    fn parse_mode_rpc() {
        let args = parse_args(&sv(&["--mode", "rpc"]), None);
        assert_eq!(args.mode, Some(Mode::Rpc));
    }

    #[test]
    fn parse_continue_flag() {
        let args = parse_args(&sv(&["--continue"]), None);
        assert!(args.continue_session);
    }

    #[test]
    fn parse_short_continue_flag() {
        let args = parse_args(&sv(&["-c"]), None);
        assert!(args.continue_session);
    }

    #[test]
    fn parse_resume_flag() {
        let args = parse_args(&sv(&["--resume"]), None);
        assert!(args.resume);
    }

    #[test]
    fn parse_print_flag() {
        let args = parse_args(&sv(&["--print"]), None);
        assert!(args.print);
    }

    #[test]
    fn parse_short_print_flag() {
        let args = parse_args(&sv(&["-p"]), None);
        assert!(args.print);
    }

    #[test]
    fn parse_no_session() {
        let args = parse_args(&sv(&["--no-session"]), None);
        assert!(args.no_session);
    }

    #[test]
    fn parse_session_path() {
        let args = parse_args(&sv(&["--session", "abc123"]), None);
        assert_eq!(args.session.as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_fork() {
        let args = parse_args(&sv(&["--fork", "abc123"]), None);
        assert_eq!(args.fork.as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_session_dir() {
        let args = parse_args(&sv(&["--session-dir", "/tmp/sessions"]), None);
        assert_eq!(args.session_dir.as_deref(), Some("/tmp/sessions"));
    }

    #[test]
    fn parse_models_comma_separated() {
        let args = parse_args(&sv(&["--models", "claude-sonnet,gpt-4o,gemini"]), None);
        assert_eq!(
            args.models,
            Some(vec![
                "claude-sonnet".to_string(),
                "gpt-4o".to_string(),
                "gemini".to_string(),
            ])
        );
    }

    #[test]
    fn parse_tools_comma_separated() {
        let args = parse_args(&sv(&["--tools", "read,bash"]), None);
        assert_eq!(args.tools, Some(vec![ToolName::Read, ToolName::Bash]));
    }

    #[test]
    fn parse_tools_unknown_warns_and_skips() {
        // This should not panic; unknown tool is skipped
        let args = parse_args(&sv(&["--tools", "read,unknown-tool,bash"]), None);
        let tools = args.tools.unwrap();
        assert_eq!(tools, vec![ToolName::Read, ToolName::Bash]);
    }

    #[test]
    fn parse_no_tools() {
        let args = parse_args(&sv(&["--no-tools"]), None);
        assert!(args.no_tools);
    }

    #[test]
    fn parse_thinking_level_high() {
        let args = parse_args(&sv(&["--thinking", "high"]), None);
        assert_eq!(args.thinking, Some(ThinkingLevel::High));
    }

    #[test]
    fn parse_thinking_level_xhigh() {
        let args = parse_args(&sv(&["--thinking", "xhigh"]), None);
        assert_eq!(args.thinking, Some(ThinkingLevel::Xhigh));
    }

    #[test]
    fn parse_thinking_level_invalid_is_ignored() {
        let args = parse_args(&sv(&["--thinking", "supermax"]), None);
        assert!(args.thinking.is_none());
    }

    #[test]
    fn parse_file_args() {
        let args = parse_args(&sv(&["@foo.txt", "@bar.md", "hello"]), None);
        assert_eq!(args.file_args, vec!["foo.txt", "bar.md"]);
        assert_eq!(args.messages, vec!["hello"]);
    }

    #[test]
    fn parse_messages_positional() {
        let args = parse_args(&sv(&["hello", "world"]), None);
        assert_eq!(args.messages, vec!["hello", "world"]);
    }

    #[test]
    fn parse_export() {
        let args = parse_args(&sv(&["--export", "/tmp/session.jsonl"]), None);
        assert_eq!(args.export.as_deref(), Some("/tmp/session.jsonl"));
    }

    #[test]
    fn parse_extension_paths() {
        let args = parse_args(
            &sv(&["--extension", "/tmp/ext1.js", "-e", "/tmp/ext2.js"]),
            None,
        );
        assert_eq!(
            args.extensions,
            Some(vec!["/tmp/ext1.js".to_string(), "/tmp/ext2.js".to_string()])
        );
    }

    #[test]
    fn parse_no_extensions() {
        let args = parse_args(&sv(&["--no-extensions"]), None);
        assert!(args.no_extensions);
    }

    #[test]
    fn parse_skill_paths() {
        let args = parse_args(&sv(&["--skill", "/tmp/skill1"]), None);
        assert_eq!(args.skills, Some(vec!["/tmp/skill1".to_string()]));
    }

    #[test]
    fn parse_no_skills() {
        let args = parse_args(&sv(&["--no-skills"]), None);
        assert!(args.no_skills);
    }

    #[test]
    fn parse_no_skills_short() {
        let args = parse_args(&sv(&["-ns"]), None);
        assert!(args.no_skills);
    }

    #[test]
    fn parse_list_models_no_pattern() {
        let args = parse_args(&sv(&["--list-models"]), None);
        assert_eq!(args.list_models, Some(None));
    }

    #[test]
    fn parse_list_models_with_pattern() {
        let args = parse_args(&sv(&["--list-models", "sonnet"]), None);
        assert_eq!(args.list_models, Some(Some("sonnet".to_string())));
    }

    #[test]
    fn parse_verbose() {
        let args = parse_args(&sv(&["--verbose"]), None);
        assert!(args.verbose);
    }

    #[test]
    fn parse_offline() {
        let args = parse_args(&sv(&["--offline"]), None);
        assert!(args.offline);
    }

    #[test]
    fn parse_api_key() {
        let args = parse_args(&sv(&["--api-key", "sk-test"]), None);
        assert_eq!(args.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn parse_system_prompt() {
        let args = parse_args(&sv(&["--system-prompt", "You are helpful"]), None);
        assert_eq!(args.system_prompt.as_deref(), Some("You are helpful"));
    }

    #[test]
    fn parse_extension_flags_boolean() {
        let mut ext_flags = HashMap::new();
        ext_flags.insert("plan".to_string(), FlagType::Boolean);
        let args = parse_args(&sv(&["--plan"]), Some(&ext_flags));
        assert!(matches!(
            args.unknown_flags.get("plan"),
            Some(FlagValue::Bool(true))
        ));
    }

    #[test]
    fn parse_extension_flags_string() {
        let mut ext_flags = HashMap::new();
        ext_flags.insert("plan-mode".to_string(), FlagType::String);
        let args = parse_args(&sv(&["--plan-mode", "aggressive"]), Some(&ext_flags));
        assert!(matches!(
            args.unknown_flags.get("plan-mode"),
            Some(FlagValue::Str(s)) if s == "aggressive"
        ));
    }

    #[test]
    fn parse_unknown_flag_without_ext_map_silently_ignored() {
        let args = parse_args(&sv(&["--unknown-flag"]), None);
        assert!(args.unknown_flags.is_empty());
    }

    #[test]
    fn thinking_level_roundtrip_display() {
        for level in [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Xhigh,
        ] {
            let s = level.to_string();
            let parsed: ThinkingLevel = s.parse().expect("roundtrip should succeed");
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn tool_name_roundtrip() {
        for name in ALL_TOOL_NAMES {
            let t: ToolName = name.parse().expect("valid tool name");
            assert_eq!(t.to_string(), *name);
        }
    }
}
