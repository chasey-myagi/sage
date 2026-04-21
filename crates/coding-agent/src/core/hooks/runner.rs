//! Hook runner — fires hooks for specific lifecycle events.
//!
//! Implements PreToolUse / PostToolUse / Stop event triggers.
//! Translated from CC `src/utils/hooks.ts` (runHooks / runPreToolUseHooks, etc.)

use anyhow::Result;

use super::executor::HookExecutor;
use super::types::{AggregatedHookResult, HookInput, HookResult, HooksSettings};

/// Fires hooks from a loaded `HooksSettings` for lifecycle events.
pub struct HookRunner {
    executor: HookExecutor,
    hooks: HooksSettings,
}

impl HookRunner {
    pub fn new(executor: HookExecutor, hooks: HooksSettings) -> Self {
        Self { executor, hooks }
    }

    /// Returns true if any hooks are configured for the given event.
    pub fn has_hooks_for(&self, event: &str) -> bool {
        self.hooks
            .get(event)
            .map(|matchers| !matchers.is_empty())
            .unwrap_or(false)
    }

    /// Run all `PreToolUse` hooks for `tool_name` with `tool_input`.
    pub async fn run_pre_tool_use(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_use_id: &str,
    ) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: None,
            agent_id: None,
            agent_type: None,
            hook_event_name: "PreToolUse".to_string(),
            event_specific: serde_json::json!({
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_use_id": tool_use_id,
            }),
        };
        self.run_for_event("PreToolUse", Some(tool_name), &input)
            .await
    }

    /// Run all `PostToolUse` hooks after a tool executes successfully.
    pub async fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_response: &serde_json::Value,
        tool_use_id: &str,
    ) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: None,
            agent_id: None,
            agent_type: None,
            hook_event_name: "PostToolUse".to_string(),
            event_specific: serde_json::json!({
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_response": tool_response,
                "tool_use_id": tool_use_id,
            }),
        };
        self.run_for_event("PostToolUse", Some(tool_name), &input)
            .await
    }

    /// Run all `Stop` hooks when the session finishes a turn.
    ///
    /// `stop_hook_active` should be `false` on the initial call; pass `true` only
    /// when a stop hook itself triggers another stop hook (prevents infinite loops).
    pub async fn run_stop(
        &self,
        last_message: Option<&str>,
        stop_hook_active: bool,
    ) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: None,
            agent_id: None,
            agent_type: None,
            hook_event_name: "Stop".to_string(),
            event_specific: serde_json::json!({
                "stop_hook_active": stop_hook_active,
                "last_assistant_message": last_message,
            }),
        };
        self.run_for_event("Stop", None, &input).await
    }

    /// Run all `SessionStart` hooks.
    pub async fn run_session_start(&self) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: None,
            agent_id: None,
            agent_type: None,
            hook_event_name: "SessionStart".to_string(),
            event_specific: serde_json::Value::Object(Default::default()),
        };
        self.run_for_event("SessionStart", None, &input).await
    }

    /// Run all `SessionEnd` hooks (tight timeout — callers should enforce separately).
    pub async fn run_session_end(&self) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: None,
            agent_id: None,
            agent_type: None,
            hook_event_name: "SessionEnd".to_string(),
            event_specific: serde_json::Value::Object(Default::default()),
        };
        self.run_for_event("SessionEnd", None, &input).await
    }

    /// Execute all hooks configured for `event_name`, filtering by `tool_name` matcher.
    async fn run_for_event(
        &self,
        event_name: &str,
        tool_name: Option<&str>,
        input: &HookInput,
    ) -> Result<AggregatedHookResult> {
        let matchers = match self.hooks.get(event_name) {
            Some(m) if !m.is_empty() => m,
            _ => return Ok(AggregatedHookResult::default()),
        };

        let mut results: Vec<HookResult> = Vec::new();

        for matcher in matchers {
            if !matcher_matches(matcher.matcher.as_deref(), tool_name) {
                continue;
            }
            for hook_cmd in &matcher.hooks {
                if hook_cmd.if_condition().is_some() {
                    tracing::warn!("hook if-condition not evaluated in v0.2.0 — skipping hook");
                    continue;
                }
                let result = self.executor.execute(hook_cmd, input).await?;
                let is_blocking = result.outcome == super::types::HookOutcome::Blocking;
                results.push(result);
                // Stop executing further hooks if one blocked.
                if is_blocking {
                    return Ok(AggregatedHookResult::from_results(results));
                }
            }
        }

        Ok(AggregatedHookResult::from_results(results))
    }
}

/// Check whether a hook matcher pattern matches `tool_name`.
///
/// Rules (mirrors CC behaviour):
/// - `None` or `"*"` → matches everything
/// - `""` (empty) → matches everything (no matcher = global)
/// - exact name comparison (case-insensitive)
fn matcher_matches(pattern: Option<&str>, tool_name: Option<&str>) -> bool {
    match pattern {
        None | Some("") | Some("*") => true,
        Some(pat) => {
            let Some(name) = tool_name else { return false };
            // Simple case-insensitive exact match (CC uses exact tool names).
            name.eq_ignore_ascii_case(pat)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_none_matches_all() {
        assert!(matcher_matches(None, Some("Write")));
        assert!(matcher_matches(None, None));
    }

    #[test]
    fn matcher_star_matches_all() {
        assert!(matcher_matches(Some("*"), Some("Bash")));
        assert!(matcher_matches(Some("*"), None));
    }

    #[test]
    fn matcher_empty_matches_all() {
        assert!(matcher_matches(Some(""), Some("Read")));
    }

    #[test]
    fn matcher_exact_name_case_insensitive() {
        assert!(matcher_matches(Some("Write"), Some("write")));
        assert!(matcher_matches(Some("BASH"), Some("Bash")));
        assert!(!matcher_matches(Some("Write"), Some("Read")));
    }

    #[test]
    fn matcher_tool_name_required_for_specific_pattern() {
        assert!(!matcher_matches(Some("Write"), None));
    }

    #[tokio::test]
    async fn hook_with_if_condition_is_skipped() {
        use super::super::types::{HookCommand, HookMatcher};

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "exit 2".to_string(),
                    if_condition: Some("$SOME_VAR == 'value'".to_string()),
                    shell: None,
                    timeout: None,
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );
        let runner = HookRunner::new(HookExecutor::new("session-1", "/tmp"), settings);
        let result = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tool-use-1")
            .await
            .unwrap();
        // Hook with if_condition must be skipped, not executed (would have blocked with exit 2).
        assert!(!result.is_blocked());
        assert!(result.blocking_error.is_none());
    }

    #[test]
    fn has_hooks_for_returns_false_when_empty() {
        let runner = HookRunner::new(
            HookExecutor::new("session-1", "/tmp"),
            HooksSettings::default(),
        );
        assert!(!runner.has_hooks_for("PreToolUse"));
    }

    #[test]
    fn has_hooks_for_returns_true_when_configured() {
        use super::super::types::{HookCommand, HookMatcher};

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "echo test".to_string(),
                    if_condition: None,
                    shell: None,
                    timeout: None,
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );
        let runner = HookRunner::new(HookExecutor::new("session-1", "/tmp"), settings);
        assert!(runner.has_hooks_for("PreToolUse"));
        assert!(!runner.has_hooks_for("Stop"));
    }
}
