//! Slash command definitions.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/slash-commands.ts`.

// ============================================================================
// Types
// ============================================================================

/// Source of a dynamic slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommandSource {
    Extension,
    Prompt,
    Skill,
}

impl std::fmt::Display for SlashCommandSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlashCommandSource::Extension => write!(f, "extension"),
            SlashCommandSource::Prompt => write!(f, "prompt"),
            SlashCommandSource::Skill => write!(f, "skill"),
        }
    }
}

/// Info about a dynamically-registered slash command (from extensions, prompts, or skills).
#[derive(Debug, Clone)]
pub struct SlashCommandInfo {
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    /// File path that provides this command.
    pub source_path: String,
}

/// A built-in slash command with a fixed name and description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinSlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// All built-in slash commands, mirroring `BUILTIN_SLASH_COMMANDS` in slash-commands.ts.
pub const BUILTIN_SLASH_COMMANDS: &[BuiltinSlashCommand] = &[
    BuiltinSlashCommand {
        name: "init",
        description: "Initialize a new CLAUDE.md file with codebase documentation",
    },
    BuiltinSlashCommand {
        name: "settings",
        description: "Open settings menu",
    },
    BuiltinSlashCommand {
        name: "model",
        description: "Select model (opens selector UI)",
    },
    BuiltinSlashCommand {
        name: "scoped-models",
        description: "Enable/disable models for Ctrl+P cycling",
    },
    BuiltinSlashCommand {
        name: "export",
        description: "Export session (HTML default, or specify path: .html/.jsonl)",
    },
    BuiltinSlashCommand {
        name: "import",
        description: "Import and resume a session from a JSONL file",
    },
    BuiltinSlashCommand {
        name: "share",
        description: "Share session as a secret GitHub gist",
    },
    BuiltinSlashCommand {
        name: "copy",
        description: "Copy last agent message to clipboard",
    },
    BuiltinSlashCommand {
        name: "name",
        description: "Set session display name",
    },
    BuiltinSlashCommand {
        name: "session",
        description: "Show session info and stats",
    },
    BuiltinSlashCommand {
        name: "changelog",
        description: "Show changelog entries",
    },
    BuiltinSlashCommand {
        name: "hotkeys",
        description: "Show all keyboard shortcuts",
    },
    BuiltinSlashCommand {
        name: "fork",
        description: "Create a new fork from a previous message",
    },
    BuiltinSlashCommand {
        name: "tree",
        description: "Navigate session tree (switch branches)",
    },
    BuiltinSlashCommand {
        name: "login",
        description: "Login with OAuth provider",
    },
    BuiltinSlashCommand {
        name: "logout",
        description: "Logout from OAuth provider",
    },
    BuiltinSlashCommand {
        name: "new",
        description: "Start a new session",
    },
    BuiltinSlashCommand {
        name: "compact",
        description: "Manually compact the session context",
    },
    BuiltinSlashCommand {
        name: "resume",
        description: "Resume a different session",
    },
    BuiltinSlashCommand {
        name: "reload",
        description: "Reload keybindings, extensions, skills, prompts, and themes",
    },
    BuiltinSlashCommand {
        name: "quit",
        description: "Quit sage",
    },
];

/// Find a built-in slash command by name.
pub fn find_builtin(name: &str) -> Option<&'static BuiltinSlashCommand> {
    BUILTIN_SLASH_COMMANDS.iter().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_list_non_empty() {
        assert!(!BUILTIN_SLASH_COMMANDS.is_empty());
    }

    #[test]
    fn find_builtin_quit() {
        let cmd = find_builtin("quit");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().description, "Quit sage");
    }

    #[test]
    fn find_builtin_not_found() {
        assert!(find_builtin("nonexistent").is_none());
    }

    #[test]
    fn all_builtins_have_nonempty_name_and_description() {
        for cmd in BUILTIN_SLASH_COMMANDS {
            assert!(!cmd.name.is_empty(), "Command has empty name");
            assert!(
                !cmd.description.is_empty(),
                "Command '{}' has empty description",
                cmd.name
            );
        }
    }

    #[test]
    fn slash_command_source_display() {
        assert_eq!(SlashCommandSource::Extension.to_string(), "extension");
        assert_eq!(SlashCommandSource::Prompt.to_string(), "prompt");
        assert_eq!(SlashCommandSource::Skill.to_string(), "skill");
    }

    #[test]
    fn find_builtin_settings() {
        let cmd = find_builtin("settings");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().name, "settings");
    }

    #[test]
    fn find_builtin_model() {
        assert!(find_builtin("model").is_some());
    }

    #[test]
    fn builtin_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for cmd in BUILTIN_SLASH_COMMANDS {
            assert!(
                seen.insert(cmd.name),
                "Duplicate builtin command: {}",
                cmd.name
            );
        }
    }
}
