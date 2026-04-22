//! Hook system data types.
//!
//! Translated from CC `src/schemas/hooks.ts` and `src/types/hooks.ts`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// All hook events known to the system (mirrors CC's HOOK_EVENTS).
pub const HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "SubagentStop",
    "PermissionDenied",
    "UserPromptSubmit",
    "SessionStart",
    "SessionEnd",
    "Setup",
    "SubagentStart",
    "Notification",
    "TeammateIdle",
    "TaskCreated",
    "TaskCompleted",
    "PreCompact",
    "PostCompact",
];

/// A single hook command, discriminated by `type`.
///
/// Mirrors CC's `HookCommand` union (command / prompt / http / agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HookCommand {
    Command {
        command: String,
        #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
        if_condition: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        /// Timeout in seconds (default: 600 for tool hooks, 1.5 for SessionEnd).
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(rename = "statusMessage", skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        once: Option<bool>,
        #[serde(rename = "async", skip_serializing_if = "Option::is_none")]
        async_: Option<bool>,
        #[serde(rename = "asyncRewake", skip_serializing_if = "Option::is_none")]
        async_rewake: Option<bool>,
    },
    Prompt {
        prompt: String,
        #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
        if_condition: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(rename = "statusMessage", skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        once: Option<bool>,
    },
    Http {
        url: String,
        #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
        if_condition: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
        #[serde(rename = "allowedEnvVars", skip_serializing_if = "Option::is_none")]
        allowed_env_vars: Option<Vec<String>>,
        #[serde(rename = "statusMessage", skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        once: Option<bool>,
    },
    Agent {
        prompt: String,
        #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
        if_condition: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(rename = "statusMessage", skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        once: Option<bool>,
    },
}

impl HookCommand {
    pub fn timeout_secs(&self) -> Option<u64> {
        match self {
            HookCommand::Command { timeout, .. } => *timeout,
            HookCommand::Prompt { timeout, .. } => *timeout,
            HookCommand::Http { timeout, .. } => *timeout,
            HookCommand::Agent { timeout, .. } => *timeout,
        }
    }

    pub fn if_condition(&self) -> Option<&str> {
        match self {
            HookCommand::Command { if_condition, .. } => if_condition.as_deref(),
            HookCommand::Prompt { if_condition, .. } => if_condition.as_deref(),
            HookCommand::Http { if_condition, .. } => if_condition.as_deref(),
            HookCommand::Agent { if_condition, .. } => if_condition.as_deref(),
        }
    }
}

/// Matcher configuration — wraps a list of hooks and an optional tool-name filter.
///
/// Mirrors CC's `HookMatcher`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcher {
    /// Optional tool-name pattern (e.g. `"Write"`, `"Bash"`, `"*"` for all).
    /// If absent, the matcher applies to all events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookCommand>,
}

/// Top-level hooks configuration loaded from `settings.json`.
///
/// Keys are hook event names (e.g. `"PreToolUse"`).
/// Values are arrays of matcher configurations.
pub type HooksSettings = HashMap<String, Vec<HookMatcher>>;

// ── Hook I/O ──────────────────────────────────────────────────────────────────

/// JSON sent to the hook process via `$HOOK_INPUT_JSON` environment variable.
///
/// Mirrors CC's `HookInput` + event-specific fields.
#[derive(Debug, Serialize)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub hook_event_name: String,
    /// Event-specific fields merged at top level (via `flatten`).
    #[serde(flatten)]
    pub event_specific: serde_json::Value,
}

/// Parsed JSON output from a hook process.
///
/// Mirrors CC's `SyncHookJSONOutput`.
#[derive(Debug, Deserialize)]
pub struct HookJsonOutput {
    /// `false` = stop execution after this hook.
    #[serde(rename = "continue")]
    pub continue_: Option<bool>,
    #[serde(rename = "suppressOutput")]
    pub suppress_output: Option<bool>,
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,
    /// Legacy block decision: `"approve"` | `"block"`.
    pub decision: Option<String>,
    pub reason: Option<String>,
    #[serde(rename = "systemMessage")]
    pub system_message: Option<String>,
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: Option<serde_json::Value>,
}

// ── Hook results ──────────────────────────────────────────────────────────────

/// Outcome of a single hook execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum HookOutcome {
    #[default]
    Success,
    Blocking,
    NonBlockingError,
    Cancelled,
}

/// Blocking error details returned when exit code == 2 or `decision == "block"`.
#[derive(Debug, Clone)]
pub struct HookBlockingError {
    pub message: String,
    pub command: String,
}

/// Result from executing a single hook.
#[derive(Debug, Default)]
pub struct HookResult {
    pub outcome: HookOutcome,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub system_message: Option<String>,
    pub blocking_error: Option<HookBlockingError>,
    pub updated_input: Option<serde_json::Value>,
    pub additional_context: Option<String>,
    pub stop_reason: Option<String>,
    pub permission_decision: Option<String>,
    pub permission_decision_reason: Option<String>,
    pub prevent_continuation: bool,
}


/// Aggregated result from all hooks for one event.
#[derive(Debug, Default)]
pub struct AggregatedHookResult {
    /// First blocking error encountered (if any).
    pub blocking_error: Option<HookBlockingError>,
    /// Whether the agent should stop after this event.
    pub prevent_continuation: bool,
    /// Reason to stop (from `stopReason` field or blocking error).
    pub stop_reason: Option<String>,
    /// All `additionalContext` strings from all hooks.
    pub additional_contexts: Vec<String>,
    /// Last non-None `updatedInput` (later hooks win).
    pub updated_input: Option<serde_json::Value>,
    /// Permission decision from PreToolUse hook.
    pub permission_decision: Option<String>,
    pub permission_decision_reason: Option<String>,
    /// All system messages to show the user.
    pub system_messages: Vec<String>,
}

impl AggregatedHookResult {
    /// Returns `true` if any hook blocked execution.
    pub fn is_blocked(&self) -> bool {
        self.blocking_error.is_some() || self.prevent_continuation
    }

    /// Merge multiple results into one aggregated result.
    pub fn from_results(results: Vec<HookResult>) -> Self {
        let mut agg = AggregatedHookResult::default();
        for r in results {
            if let Some(err) = r.blocking_error {
                agg.blocking_error = Some(err);
            }
            if r.prevent_continuation {
                agg.prevent_continuation = true;
            }
            if let Some(reason) = r.stop_reason {
                agg.stop_reason = Some(reason);
            }
            if let Some(ctx) = r.additional_context {
                agg.additional_contexts.push(ctx);
            }
            if let Some(input) = r.updated_input {
                agg.updated_input = Some(input);
            }
            if let Some(pd) = r.permission_decision {
                agg.permission_decision = Some(pd);
            }
            if let Some(pdr) = r.permission_decision_reason {
                agg.permission_decision_reason = Some(pdr);
            }
            if let Some(sm) = r.system_message {
                agg.system_messages.push(sm);
            }
        }
        agg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_command_command_serde_roundtrip() {
        let cmd = HookCommand::Command {
            command: "echo hello".to_string(),
            if_condition: None,
            shell: Some("bash".to_string()),
            timeout: Some(30),
            status_message: None,
            once: None,
            async_: None,
            async_rewake: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: HookCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HookCommand::Command { .. }));
    }

    #[test]
    fn hook_command_prompt_serde_roundtrip() {
        let cmd = HookCommand::Prompt {
            prompt: "Is this safe? $ARGUMENTS".to_string(),
            if_condition: None,
            timeout: None,
            model: Some("claude-haiku-4-5".to_string()),
            status_message: None,
            once: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: HookCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HookCommand::Prompt { .. }));
    }

    #[test]
    fn hook_matcher_serde_roundtrip() {
        let raw = json!({
            "matcher": "Write",
            "hooks": [
                { "type": "command", "command": "echo write hook" }
            ]
        });
        let matcher: HookMatcher = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(matcher.matcher.as_deref(), Some("Write"));
        assert_eq!(matcher.hooks.len(), 1);
    }

    #[test]
    fn hooks_settings_parses_multi_event() {
        let raw = json!({
            "PreToolUse": [
                { "matcher": "Write", "hooks": [{ "type": "command", "command": "echo pre" }] }
            ],
            "Stop": [
                { "hooks": [{ "type": "command", "command": "echo stop" }] }
            ]
        });
        let settings: HooksSettings = serde_json::from_value(raw).unwrap();
        assert!(settings.contains_key("PreToolUse"));
        assert!(settings.contains_key("Stop"));
        assert_eq!(settings["PreToolUse"].len(), 1);
        assert_eq!(settings["Stop"][0].matcher, None);
    }

    #[test]
    fn hook_command_timeout_secs() {
        let cmd = HookCommand::Command {
            command: "echo".to_string(),
            if_condition: None,
            shell: None,
            timeout: Some(42),
            status_message: None,
            once: None,
            async_: None,
            async_rewake: None,
        };
        assert_eq!(cmd.timeout_secs(), Some(42));
    }

    #[test]
    fn aggregated_result_merges_blocking_error() {
        let results = vec![
            HookResult {
                outcome: HookOutcome::Success,
                ..Default::default()
            },
            HookResult {
                outcome: HookOutcome::Blocking,
                blocking_error: Some(HookBlockingError {
                    message: "blocked".to_string(),
                    command: "test".to_string(),
                }),
                ..Default::default()
            },
        ];
        let agg = AggregatedHookResult::from_results(results);
        assert!(agg.is_blocked());
        assert!(agg.blocking_error.is_some());
    }

    #[test]
    fn aggregated_result_merges_additional_contexts() {
        let results = vec![
            HookResult {
                outcome: HookOutcome::Success,
                additional_context: Some("context A".to_string()),
                ..Default::default()
            },
            HookResult {
                outcome: HookOutcome::Success,
                additional_context: Some("context B".to_string()),
                ..Default::default()
            },
        ];
        let agg = AggregatedHookResult::from_results(results);
        assert_eq!(agg.additional_contexts.len(), 2);
        assert!(!agg.is_blocked());
    }

    #[test]
    fn aggregated_result_last_updated_input_wins() {
        let results = vec![
            HookResult {
                outcome: HookOutcome::Success,
                updated_input: Some(json!({"path": "original"})),
                ..Default::default()
            },
            HookResult {
                outcome: HookOutcome::Success,
                updated_input: Some(json!({"path": "overridden"})),
                ..Default::default()
            },
        ];
        let agg = AggregatedHookResult::from_results(results);
        let input = agg.updated_input.unwrap();
        assert_eq!(input["path"], "overridden");
    }
}
