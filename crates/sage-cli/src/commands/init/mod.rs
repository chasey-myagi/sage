//! `/init` slash command definition.
//!
//! Translated from pi-mono `packages/coding-agent/src/commands/init.ts`.

use std::env;

use crate::config::is_truthy_env_flag;

// ============================================================================
// Feature flag
// ============================================================================

/// Whether to use the new 8-phase init workflow.
///
/// Mirrors the duplicated feature-flag check from init.ts (lines 230-232, 246-248),
/// extracted into a single helper to avoid divergence.
pub fn should_use_new_init_workflow() -> bool {
    env::var("USER_TYPE").ok().as_deref() == Some("ant")
        || env::var("CLAUDE_CODE_NEW_INIT")
            .ok()
            .as_deref()
            .is_some_and(is_truthy_env_flag)
}

// ============================================================================
// Prompt content
// ============================================================================

static OLD_INIT_PROMPT: &str = include_str!("prompts/old_init.md");
static NEW_INIT_PROMPT: &str = include_str!("prompts/new_init.md");

pub fn get_init_prompt() -> &'static str {
    if should_use_new_init_workflow() {
        NEW_INIT_PROMPT
    } else {
        OLD_INIT_PROMPT
    }
}

// ============================================================================
// Command types
// ============================================================================

/// Source of a built-in command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    Custom,
}

// ============================================================================
// Command descriptor
// ============================================================================

pub struct InitCommand;

impl InitCommand {
    pub const NAME: &'static str = "init";

    pub fn description() -> &'static str {
        "Initialize a new CLAUDE.md file with codebase documentation"
    }

    pub fn source() -> CommandSource {
        CommandSource::Builtin
    }

    pub fn prompt() -> &'static str {
        get_init_prompt()
    }

    pub fn progress_message() -> &'static str {
        "analyzing your codebase"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn name_is_init() {
        assert_eq!(InitCommand::NAME, "init");
    }

    #[test]
    fn source_is_builtin() {
        assert_eq!(InitCommand::source(), CommandSource::Builtin);
    }

    #[test]
    fn old_prompt_nonempty() {
        assert!(!OLD_INIT_PROMPT.is_empty());
    }

    #[test]
    fn new_prompt_nonempty() {
        assert!(!NEW_INIT_PROMPT.is_empty());
    }

    #[test]
    fn prompt_nonempty() {
        assert!(!InitCommand::prompt().is_empty());
    }

    #[test]
    fn description_nonempty() {
        assert!(!InitCommand::description().is_empty());
    }

    #[test]
    fn progress_message_nonempty() {
        assert!(!InitCommand::progress_message().is_empty());
    }

    #[test]
    fn new_workflow_via_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        // CLAUDE_CODE_NEW_INIT=1 should enable new workflow
        unsafe { std::env::set_var("CLAUDE_CODE_NEW_INIT", "1") };
        let result = should_use_new_init_workflow();
        unsafe { std::env::remove_var("CLAUDE_CODE_NEW_INIT") };
        assert!(result);
    }

    #[test]
    fn new_workflow_ant_user_type() {
        let _guard = ENV_LOCK.lock().unwrap();
        // USER_TYPE=ant should enable new workflow
        unsafe { std::env::set_var("USER_TYPE", "ant") };
        let result = should_use_new_init_workflow();
        unsafe { std::env::remove_var("USER_TYPE") };
        assert!(result);
    }
}
