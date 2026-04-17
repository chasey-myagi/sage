// hooks/mod.rs — Sprint 4 H1 — Hook executor + bridge hook types
//
// Exit-2 protocol (matches Claude Code's hook contract):
//   exit 0         → Allow  (action proceeds normally)
//   exit 2         → Intervene { message: stderr }
//   any other code → Allow  (system error; infrastructure failures must never block)
//
// Commands are executed as: /bin/sh -c <command>
// This lets YAML specify plain script paths ("/hooks/eval.sh") or inline shell
// expressions ("exit 2"). env vars are injected into the child's environment.

use crate::config::HookConfig;
use sage_runtime::agent::{AfterToolCallHook, BeforeToolCallHook, StopAction, StopContext, StopHook};
use sage_runtime::types::{AfterToolCallContext, AfterToolCallResult, BeforeToolCallContext, BeforeToolCallResult};

/// Outcome of executing a hook command.
///
/// - `Allow` — hook exited 0, or a non-critical failure occurred (spawn error, timeout).
///   The guarded action proceeds normally.
/// - `Intervene` — hook exited 2. `message` is the trimmed stderr content, forwarded
///   as a steering message (PreToolUse) or feedback injection (Stop).
#[derive(Debug)]
pub enum HookOutcome {
    Allow,
    Intervene { message: String },
}

/// Execute a hook command string via `/bin/sh -c` with the given environment
/// variables added to the child's environment and a per-invocation timeout.
///
/// Only exit code and stderr are meaningful. stdout is captured and discarded.
///
/// # System-error behaviour
/// Spawn failures, command-not-found errors, and timeouts all return `Allow`.
/// Infrastructure failures must never silently block an agent operation.
pub async fn execute_hook(
    command: &str,
    env: &[(&str, &str)],
    timeout_secs: u32,
) -> HookOutcome {
    use std::time::Duration;
    use tokio::process::Command;
    use tokio::time::timeout;

    // timeout_secs = 0 → treat as immediate timeout (system error → Allow)
    if timeout_secs == 0 {
        return HookOutcome::Allow;
    }

    let duration = Duration::from_secs(timeout_secs as u64);

    // `kill_on_drop(true)` ensures the child is killed if the future is
    // cancelled by the timeout — no lingering background processes.
    let child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(command = %command, error = %e, "hook spawn failed — returning Allow");
            return HookOutcome::Allow;
        }
    };

    match timeout(duration, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            if out.status.code() == Some(2) {
                let message = String::from_utf8_lossy(&out.stderr).trim().to_string();
                HookOutcome::Intervene { message }
            } else {
                HookOutcome::Allow
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(command = %command, error = %e, "hook wait failed — returning Allow");
            HookOutcome::Allow
        }
        Err(_) => {
            // Timeout: the future is dropped here; kill_on_drop kills the child.
            tracing::warn!(command = %command, timeout_secs = %timeout_secs, "hook timed out — child killed, returning Allow");
            HookOutcome::Allow
        }
    }
}

// ── Bridge hook types ─────────────────────────────────────────────────────────
//
// These structs implement sage-runtime hook traits using execute_hook under the
// hood. They are the glue between HooksConfig (YAML-parsed) and the agent loop.

/// Runs a list of pre-tool-use scripts in sequence.
///
/// The first script that exits 2 blocks the tool call; its stderr is forwarded
/// as the block reason. All other outcomes (exit 0, timeouts, errors) allow.
pub struct ScriptPreToolUseHook {
    pub hooks: Vec<HookConfig>,
}

#[async_trait::async_trait]
impl BeforeToolCallHook for ScriptPreToolUseHook {
    async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
        let args_json = ctx.args.to_string();
        for hook in &self.hooks {
            let env: &[(&str, &str)] = &[
                ("SAGE_EVENT", "PreToolUse"),
                ("SAGE_TOOL_NAME", ctx.tool_name.as_str()),
                ("SAGE_TOOL_CALL_ID", ctx.tool_call_id.as_str()),
                ("SAGE_TOOL_INPUT", args_json.as_str()),
            ];
            if let HookOutcome::Intervene { message } =
                execute_hook(&hook.command, env, hook.timeout_secs.unwrap_or(30)).await
            {
                return BeforeToolCallResult {
                    block: true,
                    reason: Some(message),
                };
            }
        }
        BeforeToolCallResult { block: false, reason: None }
    }
}

/// Runs a list of post-tool-use scripts in sequence.
///
/// PostToolUse hooks are informational — they cannot alter the tool result.
/// All outcomes are discarded; the hook fires for side effects only (logging,
/// metrics, alerting).
pub struct ScriptPostToolUseHook {
    pub hooks: Vec<HookConfig>,
}

#[async_trait::async_trait]
impl AfterToolCallHook for ScriptPostToolUseHook {
    async fn after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult {
        let args_json = ctx.args.to_string();
        let is_error_str = ctx.is_error.to_string();
        for hook in &self.hooks {
            let env: &[(&str, &str)] = &[
                ("SAGE_EVENT", "PostToolUse"),
                ("SAGE_TOOL_NAME", ctx.tool_name.as_str()),
                ("SAGE_TOOL_CALL_ID", ctx.tool_call_id.as_str()),
                ("SAGE_TOOL_INPUT", args_json.as_str()),
                ("SAGE_TOOL_IS_ERROR", is_error_str.as_str()),
            ];
            // Outcome intentionally ignored — PostToolUse is observe-only.
                        execute_hook(&hook.command, env, hook.timeout_secs.unwrap_or(30)).await;
        }
        AfterToolCallResult { content: None, is_error: None }
    }
}

/// Runs a list of stop scripts in sequence.
///
/// The first script that exits 2 causes the agent loop to restart with the
/// script's stderr injected as a feedback message (the Harness mechanism).
/// Exit 0 (or any infrastructure failure) continues to the next hook; if all
/// hooks allow, the session ends normally.
pub struct ScriptStopHook {
    pub hooks: Vec<HookConfig>,
}

#[async_trait::async_trait]
impl StopHook for ScriptStopHook {
    async fn on_stop(&self, ctx: &StopContext) -> StopAction {
        let turn_count_str = ctx.turn_count.to_string();
        let stop_reason_str = format!("{:?}", ctx.stop_reason);
        for hook in &self.hooks {
            let env: &[(&str, &str)] = &[
                ("SAGE_EVENT", "Stop"),
                ("SAGE_AGENT_NAME", ctx.agent_name.as_str()),
                ("SAGE_SESSION_ID", ctx.session_id.as_str()),
                ("SAGE_TURN_COUNT", turn_count_str.as_str()),
                ("SAGE_STOP_REASON", stop_reason_str.as_str()),
                ("SAGE_MODEL", ctx.model.as_str()),
                ("SAGE_LAST_ASSISTANT_MESSAGE", ctx.last_assistant_message.as_str()),
            ];
            if let HookOutcome::Intervene { message } =
                execute_hook(&hook.command, env, hook.timeout_secs.unwrap_or(30)).await
            {
                return StopAction::Continue(message);
            }
        }
        StopAction::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Happy path ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn exit_0_returns_allow() {
        // Simple command that exits 0 — no policy decision
        let outcome = execute_hook("exit 0", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 0 must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_0_with_builtin_true() {
        // `true` is a standard POSIX utility that exits 0
        let outcome = execute_hook("true", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "`true` must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_2_with_stderr_returns_intervene() {
        // The exit-2 protocol: stderr becomes the Intervene message
        let outcome = execute_hook(
            r#"printf 'blocked by policy' >&2; exit 2"#,
            &[],
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("blocked by policy"),
                    "stderr must appear in Intervene message, got: {message:?}"
                );
            }
            other => panic!("exit 2 must be Intervene, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exit_2_empty_stderr_returns_intervene_with_empty_message() {
        // A script that exits 2 without writing to stderr is valid —
        // Intervene is triggered but the message is an empty string (not None).
        let outcome = execute_hook("exit 2", &[], 5).await;
        match outcome {
            HookOutcome::Intervene { message } => {
                // We just need the variant — message may be "" or whitespace
                let _ = message;
            }
            other => panic!("exit 2 with no stderr must still be Intervene, got {other:?}"),
        }
    }

    // ── System-error cases → Allow (never block on infrastructure failure) ────

    #[tokio::test]
    async fn exit_1_is_allow_not_intervene() {
        // exit 1 is generic failure, not the exit-2 hook protocol
        let outcome = execute_hook("exit 1", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 1 (generic failure) must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_3_is_allow_not_intervene() {
        // Only exit code 2 triggers Intervene; all other non-zero codes → Allow
        let outcome = execute_hook("exit 3", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 3 must be Allow (only exit 2 → Intervene), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_127_command_not_found_is_allow() {
        // Exit 127 is the shell's "command not found" — still a system error
        let outcome = execute_hook("exit 127", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 127 must be Allow (system error), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn nonexistent_binary_in_command_does_not_panic_or_intervene() {
        // If the script itself references a missing binary, sh exits non-zero.
        // This must never be treated as a policy Intervene.
        let outcome =
            execute_hook("nonexistent_binary_sage_sprint4_99999", &[], 5).await;
        assert!(
            !matches!(outcome, HookOutcome::Intervene { .. }),
            "spawn/exec failure must not Intervene (infrastructure error → Allow), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn timeout_is_allow_not_intervene() {
        // A script that runs longer than timeout_secs must be killed and return Allow.
        // Use 1-second timeout with a 30-second sleep.
        let outcome = execute_hook("sleep 30", &[], 1).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "timeout must be Allow (system error, never Intervene), got {outcome:?}"
        );
    }

    // ── Environment variable injection ────────────────────────────────────────

    #[tokio::test]
    async fn single_env_var_passed_to_command() {
        // If the var is NOT forwarded, the if-branch is false → exit 0 → Allow.
        // If forwarded correctly → exit 2 → Intervene.
        let outcome = execute_hook(
            r#"if [ "$SAGE_TEST_VAR" = "hello" ]; then printf "received" >&2; exit 2; fi; exit 0"#,
            &[("SAGE_TEST_VAR", "hello")],
            5,
        )
        .await;
        match &outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("received"),
                    "env var was passed; expected 'received' in stderr, got: {message:?}"
                );
            }
            HookOutcome::Allow => {
                panic!("env var was NOT passed: hook exited 0 instead of 2");
            }
        }
    }

    #[tokio::test]
    async fn multiple_env_vars_all_forwarded() {
        // All key-value pairs in the slice must reach the child process.
        let outcome = execute_hook(
            r#"if [ "$A" = "1" ] && [ "$B" = "2" ]; then exit 2; fi; exit 0"#,
            &[("A", "1"), ("B", "2")],
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "both env vars must be forwarded; expected exit 2, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn env_var_with_special_characters_forwarded() {
        // Values with spaces, colons, and slashes must survive intact.
        let outcome = execute_hook(
            r#"if [ "$SAGE_SPECIAL" = "value with spaces" ]; then exit 2; fi; exit 0"#,
            &[("SAGE_SPECIAL", "value with spaces")],
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "env var with spaces must be forwarded correctly, got {outcome:?}"
        );
    }

    // ── stdout is irrelevant ──────────────────────────────────────────────────

    #[tokio::test]
    async fn stdout_does_not_affect_outcome() {
        // Writing to stdout on exit 0 must not change the outcome
        let outcome = execute_hook(
            r#"echo "this stdout is ignored"; exit 0"#,
            &[],
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "stdout must not affect outcome, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn only_stderr_appears_in_intervene_message() {
        // When both stdout and stderr are written and hook exits 2,
        // only stderr must appear in the Intervene message.
        let outcome = execute_hook(
            r#"echo "stdout_content"; printf "stderr_content" >&2; exit 2"#,
            &[],
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("stderr_content"),
                    "stderr_content must appear in Intervene, got: {message:?}"
                );
                assert!(
                    !message.contains("stdout_content"),
                    "stdout_content must NOT appear in Intervene, got: {message:?}"
                );
            }
            other => panic!("expected Intervene, got {other:?}"),
        }
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn zero_timeout_secs_does_not_panic() {
        // timeout_secs = 0 is unusual; implementation may clamp or treat as
        // "immediate timeout" — must not panic in either case.
        let outcome = execute_hook("exit 0", &[], 0).await;
        let _ = outcome; // just assert no panic
    }

    #[tokio::test]
    async fn large_stderr_output_does_not_panic() {
        // A hook that writes many lines to stderr must not panic or OOM.
        // Implementation may truncate, but Intervene must still be returned.
        let outcome = execute_hook(
            r#"yes "long stderr line from hook" | head -1000 >&2; exit 2"#,
            &[],
            10,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "large stderr must still produce Intervene, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn empty_command_string_is_allow() {
        // An empty command is a configuration error (system error → Allow)
        let outcome = execute_hook("", &[], 5).await;
        assert!(
            !matches!(outcome, HookOutcome::Intervene { .. }),
            "empty command must be Allow (configuration error → system error), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_0_with_stderr_written_is_still_allow() {
        // Writing to stderr on exit 0 must NOT trigger Intervene.
        // Only exit 2 triggers it.
        let outcome = execute_hook(
            r#"printf "diagnostic output" >&2; exit 0"#,
            &[],
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 0 with stderr must still be Allow, got {outcome:?}"
        );
    }

    // ── Stderr content fidelity ───────────────────────────────────────────────

    #[tokio::test]
    async fn multiline_stderr_preserved_in_message() {
        // All stderr lines must appear in the Intervene message, not just the first.
        let outcome = execute_hook(
            r#"printf 'line1\nline2\nline3' >&2; exit 2"#,
            &[],
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("line1"),
                    "first line must be in message, got: {message:?}"
                );
                assert!(
                    message.contains("line3"),
                    "last line must be in message, got: {message:?}"
                );
            }
            other => panic!("expected Intervene, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exact_stderr_text_present_in_message() {
        // Verify the exact stderr text appears — catch implementations that
        // accidentally truncate or transform the content.
        let outcome = execute_hook(
            r#"printf 'exact_sentinel_abc123' >&2; exit 2"#,
            &[],
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("exact_sentinel_abc123"),
                    "exact sentinel must be in message, got: {message:?}"
                );
            }
            other => panic!("expected Intervene, got {other:?}"),
        }
    }

    // ── env var semantics ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn env_var_overrides_inherited_env() {
        // An injected env var must shadow any same-named var the parent process
        // might have in its environment.
        let outcome = execute_hook(
            // SAGE_OVERRIDE is set to "parent_value" by the parent process in practice,
            // but here we inject "injected_value" and check the hook sees the injected one.
            r#"if [ "$SAGE_OVERRIDE" = "injected_value" ]; then exit 2; fi; exit 0"#,
            &[("SAGE_OVERRIDE", "injected_value")],
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "injected env var must override any inherited value, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn hook_with_no_env_vars_still_has_access_to_path() {
        // Even with an empty env override list, the child must have access to
        // standard binaries (i.e., PATH is inherited).
        // The `sh` shell itself must be findable.
        let outcome = execute_hook("true", &[], 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "child with no extra env must still run built-in `true`, got {outcome:?}"
        );
    }

    // ── Integration: ScriptStopHook bridge (H1 → H2) ─────────────────────────
    //
    // Tests for the production ScriptStopHook struct that bridges execute_hook
    // outcomes to StopAction values. Uses HookConfig to match the production API.

    use crate::config::HookConfig;
    use sage_runtime::agent::{StopAction, StopContext};

    fn make_stop_ctx(agent_name: &str, turn_count: usize, session_id: &str) -> StopContext {
        StopContext {
            stop_reason: sage_runtime::types::StopReason::Stop,
            session_id: session_id.into(),
            task_id: session_id.into(),
            turn_count,
            agent_name: agent_name.into(),
            model: "test-model".into(),
            last_assistant_message: "Done.".into(),
        }
    }

    #[tokio::test]
    async fn script_stop_hook_exit_0_gives_pass() {
        // Harness evaluator exits 0 → agent stop is accepted → Pass
        let hook = ScriptStopHook {
            hooks: vec![HookConfig { command: "exit 0".into(), timeout_secs: Some(5) }],
        };
        let action = hook.on_stop(&make_stop_ctx("eval-agent", 3, "sess-1")).await;
        assert!(
            matches!(action, StopAction::Pass),
            "evaluator exit 0 → Pass, got {action:?}"
        );
    }

    #[tokio::test]
    async fn script_stop_hook_exit_2_gives_continue_with_feedback() {
        // Harness evaluator exits 2 with feedback → agent restarts with feedback injected
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"printf 'please verify step 3' >&2; exit 2"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("eval-agent", 1, "sess-2")).await;
        match action {
            StopAction::Continue(msg) => {
                assert!(
                    msg.contains("please verify step 3"),
                    "feedback must appear in Continue message, got: {msg:?}"
                );
            }
            other => panic!("evaluator exit 2 → Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn script_stop_hook_env_vars_forwarded() {
        // SAGE_AGENT_NAME and SAGE_TURN_COUNT must reach the evaluator script
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"if [ "$SAGE_AGENT_NAME" = "my-agent" ] && [ "$SAGE_TURN_COUNT" = "5" ]; then exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("my-agent", 5, "sess-3")).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "env vars must reach evaluator; expected Continue (exit 2 path), got {action:?}"
        );
    }
}
