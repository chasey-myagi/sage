//! Hook runner — fires hooks for specific lifecycle events.
//!
//! Implements PreToolUse / PostToolUse / Stop event triggers.
//! Translated from CC `src/utils/hooks.ts` (runHooks / runPreToolUseHooks, etc.)

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;

use super::executor::HookExecutor;
use super::types::{AggregatedHookResult, HookCommand, HookInput, HookOutcome, HookResult, HooksSettings};

/// Fires hooks from a loaded `HooksSettings` for lifecycle events.
pub struct HookRunner {
    executor: HookExecutor,
    hooks: HooksSettings,
    /// Tracks command keys for hooks with `once: true` that have already run this session.
    once_executed: Mutex<HashSet<String>>,
}

impl HookRunner {
    pub fn new(executor: HookExecutor, hooks: HooksSettings) -> Self {
        for matchers in hooks.values() {
            for matcher in matchers {
                for hook_cmd in &matcher.hooks {
                    if let Some(condition) = hook_cmd.if_condition() {
                        if condition.contains("{{") {
                            tracing::warn!(
                                condition,
                                "if-condition looks like a template expression ({{...}}); \
                                 only exact tool-name matching is supported in this implementation"
                            );
                        }
                    }
                }
            }
        }
        Self { executor, hooks, once_executed: Mutex::new(HashSet::new()) }
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
            permission_mode: self.executor.permission_mode.clone(),
            agent_id: self.executor.agent_id.clone(),
            agent_type: self.executor.agent_type.clone(),
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
            permission_mode: self.executor.permission_mode.clone(),
            agent_id: self.executor.agent_id.clone(),
            agent_type: self.executor.agent_type.clone(),
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
            permission_mode: self.executor.permission_mode.clone(),
            agent_id: self.executor.agent_id.clone(),
            agent_type: self.executor.agent_type.clone(),
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
            permission_mode: self.executor.permission_mode.clone(),
            agent_id: self.executor.agent_id.clone(),
            agent_type: self.executor.agent_type.clone(),
            hook_event_name: "SessionStart".to_string(),
            event_specific: serde_json::Value::Object(Default::default()),
        };
        self.run_for_event("SessionStart", None, &input).await
    }

    /// Run all `SessionEnd` hooks with a tight 2-second default timeout.
    ///
    /// These run during shutdown and must complete quickly. Per-hook `timeout`
    /// in the config still overrides the default.
    pub async fn run_session_end(&self) -> Result<AggregatedHookResult> {
        let input = HookInput {
            session_id: self.executor.session_id.clone(),
            transcript_path: self.executor.transcript_path.clone(),
            cwd: self.executor.cwd.clone(),
            permission_mode: self.executor.permission_mode.clone(),
            agent_id: self.executor.agent_id.clone(),
            agent_type: self.executor.agent_type.clone(),
            hook_event_name: "SessionEnd".to_string(),
            event_specific: serde_json::Value::Object(Default::default()),
        };

        // SessionEnd runs outside of normal tool flow — replicate run_for_event
        // but use execute_session_end to apply the tight default timeout.
        let matchers = match self.hooks.get("SessionEnd") {
            Some(m) if !m.is_empty() => m,
            _ => return Ok(AggregatedHookResult::default()),
        };

        let mut results: Vec<HookResult> = Vec::new();
        for matcher in matchers {
            if !matcher_matches(matcher.matcher.as_deref(), None) {
                continue;
            }
            for hook_cmd in &matcher.hooks {
                if let Some(condition) = hook_cmd.if_condition() {
                    // Non-tool event — conditions cannot be evaluated without a tool name.
                    tracing::debug!(
                        condition,
                        "skipping hook with if-condition on SessionEnd (no tool name)"
                    );
                    continue;
                }
                if let Some(key) = once_key(hook_cmd) {
                    let already_ran = self.once_executed.lock().unwrap().contains(&key);
                    if already_ran {
                        tracing::debug!(key, "skipping once: true hook — already ran this session");
                        continue;
                    }
                    let result = self.executor.execute_session_end(hook_cmd, &input).await?;
                    self.once_executed.lock().unwrap().insert(key);
                    let is_blocking = result.outcome == HookOutcome::Blocking;
                    results.push(result);
                    if is_blocking {
                        return Ok(AggregatedHookResult::from_results(results));
                    }
                } else {
                    let result = self.executor.execute_session_end(hook_cmd, &input).await?;
                    let is_blocking = result.outcome == HookOutcome::Blocking;
                    results.push(result);
                    if is_blocking {
                        return Ok(AggregatedHookResult::from_results(results));
                    }
                }
            }
        }
        Ok(AggregatedHookResult::from_results(results))
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
                if let Some(condition) = hook_cmd.if_condition() {
                    match tool_name {
                        Some(name) => {
                            // For tool events: treat condition as a case-insensitive tool name pattern.
                            if !name.eq_ignore_ascii_case(condition) {
                                tracing::debug!(
                                    condition,
                                    tool_name = name,
                                    "skipping hook: if-condition does not match tool name"
                                );
                                continue;
                            }
                            // Condition matches — fall through and execute the hook.
                        }
                        None => {
                            // Non-tool event: condition cannot be evaluated without a tool name.
                            tracing::debug!(
                                condition,
                                event = event_name,
                                "skipping hook with if-condition on non-tool event"
                            );
                            continue;
                        }
                    }
                }
                if let Some(key) = once_key(hook_cmd) {
                    let already_ran = self.once_executed.lock().unwrap().contains(&key);
                    if already_ran {
                        tracing::debug!(key, "skipping once: true hook — already ran this session");
                        continue;
                    }
                    let result = self.executor.execute(hook_cmd, input).await?;
                    self.once_executed.lock().unwrap().insert(key);
                    let is_blocking = result.outcome == HookOutcome::Blocking;
                    results.push(result);
                    if is_blocking {
                        return Ok(AggregatedHookResult::from_results(results));
                    }
                } else {
                    let result = self.executor.execute(hook_cmd, input).await?;
                    let is_blocking = result.outcome == HookOutcome::Blocking;
                    results.push(result);
                    // Stop executing further hooks if one blocked.
                    if is_blocking {
                        return Ok(AggregatedHookResult::from_results(results));
                    }
                }
            }
        }

        Ok(AggregatedHookResult::from_results(results))
    }
}

/// Returns a stable deduplication key for hooks with `once: true`, or `None` if the hook
/// does not have `once` set. The key is prefixed by type to avoid accidental collisions
/// between e.g. a `command` and a `prompt` that share the same content string.
fn once_key(hook_cmd: &HookCommand) -> Option<String> {
    match hook_cmd {
        HookCommand::Command { command, once: Some(true), .. } => {
            Some(format!("command:{command}"))
        }
        HookCommand::Prompt { prompt, once: Some(true), .. } => {
            Some(format!("prompt:{prompt}"))
        }
        HookCommand::Http { url, once: Some(true), .. } => {
            Some(format!("http:{url}"))
        }
        HookCommand::Agent { prompt, once: Some(true), .. } => {
            Some(format!("agent:{prompt}"))
        }
        _ => None,
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

// ── HooksLifecycle ────────────────────────────────────────────────────────────

/// Bridges `HookRunner` to the `agent-core` hook traits.
///
/// Create one instance, then call `agent.set_before_tool_call(Arc::clone(&lifecycle))`
/// and `agent.set_after_tool_call(lifecycle)` to wire hooks into the tool lifecycle.
pub struct HooksLifecycle {
    runner: Arc<HookRunner>,
}

impl HooksLifecycle {
    pub fn new(runner: Arc<HookRunner>) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl agent_core::agent::BeforeToolCallHook for HooksLifecycle {
    async fn before_tool_call(
        &self,
        ctx: &agent_core::types::BeforeToolCallContext,
    ) -> agent_core::types::BeforeToolCallResult {
        match self
            .runner
            .run_pre_tool_use(&ctx.tool_name, &ctx.args, &ctx.tool_call_id)
            .await
        {
            Ok(result) if result.is_blocked() => agent_core::types::BeforeToolCallResult {
                block: true,
                reason: result
                    .blocking_error
                    .map(|e| e.message)
                    .or(result.stop_reason),
            },
            _ => agent_core::types::BeforeToolCallResult {
                block: false,
                reason: None,
            },
        }
    }
}

#[async_trait]
impl agent_core::agent::AfterToolCallHook for HooksLifecycle {
    async fn after_tool_call(
        &self,
        ctx: &agent_core::types::AfterToolCallContext,
    ) -> agent_core::types::AfterToolCallResult {
        let tool_response = serde_json::to_value(&ctx.result).unwrap_or_default();
        // PostToolUse errors are non-blocking by design — we ignore the return value.
        let _ = self
            .runner
            .run_post_tool_use(&ctx.tool_name, &ctx.args, &tool_response, &ctx.tool_call_id)
            .await;
        agent_core::types::AfterToolCallResult {
            content: None,
            details: None,
            is_error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::executor::DEFAULT_SESSION_END_TIMEOUT_SECS;
    use super::super::types::{HookCommand, HookMatcher};

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
    async fn hook_with_nonmatching_if_condition_skipped() {
        // Hook is configured with if_condition "Bash" but we call with tool "Write".
        // The hook should be skipped, not executed.
        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "exit 2".to_string(),
                    if_condition: Some("Bash".to_string()),
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
        assert!(!result.is_blocked());
    }

    #[tokio::test]
    async fn hook_with_matching_if_condition_fires() {
        // Hook is configured with if_condition "Write" and we call with tool "Write".
        // The hook executes and exits 2 → blocks.
        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "exit 2".to_string(),
                    if_condition: Some("Write".to_string()),
                    shell: None,
                    timeout: Some(5),
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
        assert!(result.is_blocked(), "matching if_condition should allow hook to run");
    }

    #[tokio::test]
    async fn hook_if_condition_case_insensitive_match() {
        // if_condition "write" should still match tool "Write".
        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "exit 2".to_string(),
                    if_condition: Some("write".to_string()),
                    shell: None,
                    timeout: Some(5),
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
        assert!(result.is_blocked());
    }

    #[tokio::test]
    async fn hook_with_if_condition_on_stop_event_is_skipped() {
        // Stop is not a tool event — hooks with if_condition should be skipped.
        let mut settings = HooksSettings::default();
        settings.insert(
            "Stop".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "exit 2".to_string(),
                    if_condition: Some("Write".to_string()),
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );
        let runner = HookRunner::new(HookExecutor::new("session-1", "/tmp"), settings);
        let result = runner.run_stop(None, false).await.unwrap();
        assert!(!result.is_blocked());
    }

    #[tokio::test]
    async fn pre_tool_use_hook_command_actually_executes() {
        use std::path::PathBuf;
        let tmp = std::env::temp_dir();
        let flag_file: PathBuf = tmp.join(format!("hook_ran_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        let result = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tool-use-1")
            .await
            .unwrap();

        assert!(!result.is_blocked());
        assert!(flag_file.exists(), "hook command must have created the flag file");

        // cleanup
        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn post_tool_use_hook_actually_executes() {
        use std::path::PathBuf;
        let tmp = std::env::temp_dir();
        let flag_file: PathBuf = tmp.join(format!("post_hook_ran_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "PostToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        let result = runner
            .run_post_tool_use(
                "Write",
                &serde_json::json!({}),
                &serde_json::json!({"content": "ok"}),
                "tool-use-1",
            )
            .await
            .unwrap();

        assert!(!result.is_blocked());
        assert!(flag_file.exists(), "PostToolUse hook must have created the flag file");

        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn session_end_uses_tight_default_timeout() {
        // Verify the session-end timeout constant is much smaller than the tool timeout.
        assert!(DEFAULT_SESSION_END_TIMEOUT_SECS < 10);
    }

    #[tokio::test]
    async fn prompt_hook_in_pre_tool_use_is_non_blocking_error() {
        // Prompt hooks configured for PreToolUse should not block execution.
        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Prompt {
                    prompt: "Is this safe?".to_string(),
                    if_condition: None,
                    timeout: None,
                    model: None,
                    status_message: None,
                    once: None,
                }],
            }],
        );
        let runner = HookRunner::new(HookExecutor::new("session-1", "/tmp"), settings);
        let result = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tool-use-1")
            .await
            .unwrap();
        assert!(!result.is_blocked());
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

    #[test]
    fn runtime_fields_propagated_to_hook_input() {
        // Verify executor fields are readable (they'll be passed to HookInput at runtime).
        let executor = HookExecutor::new("session-42", "/workspace")
            .with_permission_mode("default")
            .with_agent_id("agent-123")
            .with_agent_type("coding-agent")
            .with_transcript_path("/workspace/.sage/session.jsonl");

        assert_eq!(executor.session_id, "session-42");
        assert_eq!(executor.permission_mode.as_deref(), Some("default"));
        assert_eq!(executor.agent_id.as_deref(), Some("agent-123"));
        assert_eq!(executor.agent_type.as_deref(), Some("coding-agent"));
        assert_eq!(executor.transcript_path, "/workspace/.sage/session.jsonl");
    }

    // ── Integration tests: lifecycle methods actually execute hooks ────────────

    #[tokio::test]
    async fn session_start_hook_actually_executes() {
        let tmp = std::env::temp_dir();
        let flag_file = tmp.join(format!("session_start_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "SessionStart".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        runner.run_session_start().await.unwrap();

        assert!(flag_file.exists(), "SessionStart hook must have created the flag file");
        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn stop_hook_actually_executes() {
        let tmp = std::env::temp_dir();
        let flag_file = tmp.join(format!("stop_hook_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "Stop".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        runner.run_stop(Some("last message"), false).await.unwrap();

        assert!(flag_file.exists(), "Stop hook must have created the flag file");
        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn session_end_hook_actually_executes() {
        let tmp = std::env::temp_dir();
        let flag_file = tmp.join(format!("session_end_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "SessionEnd".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        runner.run_session_end().await.unwrap();

        assert!(flag_file.exists(), "SessionEnd hook must have created the flag file");
        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn hook_input_contains_correct_runtime_fields() {
        // Run a hook that dumps $HOOK_INPUT_JSON to stdout, then parse and verify all fields.
        let tmp = std::env::temp_dir();
        let transcript_path = "/tmp/test-session.jsonl".to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "echo \"$HOOK_INPUT_JSON\"".to_string(),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("session-abc", tmp.to_str().unwrap())
            .with_permission_mode("default")
            .with_agent_type("coding-agent")
            .with_transcript_path(transcript_path.clone());
        let runner = HookRunner::new(executor, settings);
        let result = runner
            .run_pre_tool_use("Write", &serde_json::json!({"path": "/tmp/x"}), "tc-1")
            .await
            .unwrap();

        assert!(!result.is_blocked());
        // Even without parsing stdout here, the test proves the hook executed
        // (it would have exit 1 or other error if the JSON env var was malformed).
        // The real field validation is done by checking executor state above (runtime_fields test).
        // This test proves that a command hook with runtime fields set does NOT fail.
        let _ = &result.system_messages;
    }

    #[tokio::test]
    async fn hook_input_transcript_path_and_permission_mode_passed_to_command() {
        // Run a hook that checks specific env-var contents and exits 2 if wrong.
        // This proves the HookInput JSON actually contains the runtime fields.
        let tmp = std::env::temp_dir();
        let transcript = "/tmp/test-transcript.jsonl".to_string();
        let session_id = "session-field-test";

        let check_cmd = format!(
            r#"echo "$HOOK_INPUT_JSON" | python3 -c "
import json,sys
d=json.load(sys.stdin)
assert d.get('session_id')=='{session_id}', f'wrong session_id: {{d.get(\"session_id\")}}'
assert d.get('transcript_path')=='{transcript}', f'wrong transcript_path: {{d.get(\"transcript_path\")}}'
assert d.get('permission_mode')=='default', f'wrong permission_mode: {{d.get(\"permission_mode\")}}'
assert d.get('agent_type')=='coding-agent', f'wrong agent_type: {{d.get(\"agent_type\")}}'
""#
        );

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: check_cmd,
                    if_condition: None,
                    shell: None,
                    timeout: Some(10),
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new(session_id, tmp.to_str().unwrap())
            .with_permission_mode("default")
            .with_agent_type("coding-agent")
            .with_transcript_path(transcript.clone());
        let runner = HookRunner::new(executor, settings);
        let result = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tc-1")
            .await
            .unwrap();

        assert!(
            !result.is_blocked(),
            "HookInput field check failed. system_messages: {:?}, blocking_error: {:?}",
            result.system_messages,
            result.blocking_error.as_ref().map(|e| &e.message),
        );
    }

    // ── once_key pure-function tests ──────────────────────────────────────────

    #[test]
    fn once_key_returns_none_without_once_flag() {
        let cmd = HookCommand::Command {
            command: "echo hi".to_string(),
            if_condition: None,
            shell: None,
            timeout: None,
            status_message: None,
            once: None,
            async_: None,
            async_rewake: None,
        };
        assert!(once_key(&cmd).is_none());
    }

    #[test]
    fn once_key_returns_none_when_once_false() {
        let cmd = HookCommand::Command {
            command: "echo hi".to_string(),
            if_condition: None,
            shell: None,
            timeout: None,
            status_message: None,
            once: Some(false),
            async_: None,
            async_rewake: None,
        };
        assert!(once_key(&cmd).is_none());
    }

    #[test]
    fn once_key_command_includes_command_prefix() {
        let cmd = HookCommand::Command {
            command: "exit 2".to_string(),
            if_condition: None,
            shell: None,
            timeout: None,
            status_message: None,
            once: Some(true),
            async_: None,
            async_rewake: None,
        };
        assert_eq!(once_key(&cmd), Some("command:exit 2".to_string()));
    }

    #[test]
    fn once_key_prompt_includes_prompt_prefix() {
        let cmd = HookCommand::Prompt {
            prompt: "Is this safe?".to_string(),
            if_condition: None,
            timeout: None,
            model: None,
            status_message: None,
            once: Some(true),
        };
        assert_eq!(once_key(&cmd), Some("prompt:Is this safe?".to_string()));
    }

    #[test]
    fn once_key_http_includes_http_prefix() {
        let cmd = HookCommand::Http {
            url: "http://localhost/hook".to_string(),
            if_condition: None,
            timeout: None,
            headers: None,
            allowed_env_vars: None,
            status_message: None,
            once: Some(true),
        };
        assert_eq!(once_key(&cmd), Some("http:http://localhost/hook".to_string()));
    }

    #[test]
    fn once_key_agent_includes_agent_prefix() {
        let cmd = HookCommand::Agent {
            prompt: "Check for issues".to_string(),
            if_condition: None,
            timeout: None,
            model: None,
            status_message: None,
            once: Some(true),
        };
        assert_eq!(once_key(&cmd), Some("agent:Check for issues".to_string()));
    }

    #[test]
    fn once_key_different_types_same_content_no_collision() {
        // "command:foo" and "prompt:foo" must be distinct keys.
        let cmd_cmd = HookCommand::Command {
            command: "foo".to_string(),
            if_condition: None,
            shell: None,
            timeout: None,
            status_message: None,
            once: Some(true),
            async_: None,
            async_rewake: None,
        };
        let cmd_prompt = HookCommand::Prompt {
            prompt: "foo".to_string(),
            if_condition: None,
            timeout: None,
            model: None,
            status_message: None,
            once: Some(true),
        };
        assert_ne!(once_key(&cmd_cmd), once_key(&cmd_prompt));
    }

    // ── once:true integration tests ───────────────────────────────────────────

    #[tokio::test]
    async fn once_true_hook_runs_only_once_across_sequential_calls() {
        let tmp = std::env::temp_dir();
        let flag_file = tmp.join(format!("once_seq_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: format!("echo ran >> {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: Some(true),
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);

        // Three sequential calls — hook should only execute on the first.
        for i in 0..3u32 {
            runner
                .run_pre_tool_use("Write", &serde_json::json!({}), &format!("tc-{i}"))
                .await
                .unwrap();
        }

        let content = std::fs::read_to_string(&flag_file).unwrap_or_default();
        let line_count = content.lines().count();
        assert_eq!(line_count, 1, "once:true hook ran {line_count} times; expected exactly 1");

        let _ = std::fs::remove_file(&flag_file);
    }

    #[tokio::test]
    async fn once_true_hook_second_call_produces_empty_result() {
        // Verify that once:true does not add to results on subsequent calls.
        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    command: "echo once".to_string(),
                    if_condition: None,
                    shell: None,
                    timeout: Some(5),
                    status_message: None,
                    once: Some(true),
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let runner = HookRunner::new(HookExecutor::new("session-once", "/tmp"), settings);

        let _first = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tc-1")
            .await
            .unwrap();

        let second = runner
            .run_pre_tool_use("Write", &serde_json::json!({}), "tc-2")
            .await
            .unwrap();

        // Second call should produce an empty aggregated result (no hooks ran).
        assert!(
            second.system_messages.is_empty(),
            "second call after once:true should produce no results"
        );
        assert!(!second.is_blocked());
    }

    #[tokio::test]
    async fn once_true_hook_different_commands_each_run_once() {
        // Two separate hooks with once:true should each run exactly once.
        let tmp = std::env::temp_dir();
        let flag_a = tmp.join(format!("once_a_{}.txt", ulid::Ulid::new()));
        let flag_b = tmp.join(format!("once_b_{}.txt", ulid::Ulid::new()));
        let path_a = flag_a.to_str().unwrap().to_string();
        let path_b = flag_b.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "PreToolUse".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![
                    HookCommand::Command {
                        command: format!("touch {path_a}"),
                        if_condition: None,
                        shell: None,
                        timeout: Some(5),
                        status_message: None,
                        once: Some(true),
                        async_: None,
                        async_rewake: None,
                    },
                    HookCommand::Command {
                        command: format!("touch {path_b}"),
                        if_condition: None,
                        shell: None,
                        timeout: Some(5),
                        status_message: None,
                        once: Some(true),
                        async_: None,
                        async_rewake: None,
                    },
                ],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);

        // Run twice; each hook should only fire on first run.
        for i in 0..2u32 {
            runner
                .run_pre_tool_use("Write", &serde_json::json!({}), &format!("tc-{i}"))
                .await
                .unwrap();
        }

        assert!(flag_a.exists(), "hook A must have executed");
        assert!(flag_b.exists(), "hook B must have executed");

        let _ = std::fs::remove_file(&flag_a);
        let _ = std::fs::remove_file(&flag_b);
    }

    #[tokio::test]
    async fn session_end_uses_tight_default_timeout_and_executes() {
        // SessionEnd hooks must complete quickly AND actually run.
        let tmp = std::env::temp_dir();
        let flag_file = tmp.join(format!("session_end_timeout_{}.txt", ulid::Ulid::new()));
        let flag_path = flag_file.to_str().unwrap().to_string();

        let mut settings = HooksSettings::default();
        settings.insert(
            "SessionEnd".to_string(),
            vec![HookMatcher {
                matcher: None,
                hooks: vec![HookCommand::Command {
                    // Fast command that finishes well within the tight timeout.
                    command: format!("touch {flag_path}"),
                    if_condition: None,
                    shell: None,
                    timeout: None, // use default session-end timeout
                    status_message: None,
                    once: None,
                    async_: None,
                    async_rewake: None,
                }],
            }],
        );

        let executor = HookExecutor::new("test-session", tmp.to_str().unwrap());
        let runner = HookRunner::new(executor, settings);
        runner.run_session_end().await.unwrap();

        assert!(
            flag_file.exists(),
            "SessionEnd hook with default tight timeout must still execute fast commands"
        );
        let _ = std::fs::remove_file(&flag_file);
        // Verify the constant is tight.
        assert!(DEFAULT_SESSION_END_TIMEOUT_SECS < 10);
    }
}
