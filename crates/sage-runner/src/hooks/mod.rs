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
use sage_runtime::agent::{
    AfterToolCallHook, BeforeToolCallHook, StopAction, StopContext, StopHook,
};
use sage_runtime::types::{
    AfterToolCallContext, AfterToolCallResult, BeforeToolCallContext, BeforeToolCallResult,
};

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
/// # stdin contract
/// When `stdin_json` is `Some(s)`, the child's stdin is opened as a pipe and
/// `s` followed by a newline is written before stdin is closed so the script
/// sees EOF. When `None`, stdin is redirected to `/dev/null` (legacy behaviour).
///
/// # System-error behaviour
/// Spawn failures, command-not-found errors, and timeouts all return `Allow`.
/// Infrastructure failures must never silently block an agent operation.
pub async fn execute_hook(
    command: &str,
    env: &[(&str, &str)],
    stdin_json: Option<&str>,
    timeout_secs: u32,
) -> HookOutcome {
    use std::time::Duration;
    use tokio::io::AsyncWriteExt as _;
    use tokio::process::Command;
    use tokio::time::timeout;

    // timeout_secs = 0 → treat as immediate timeout (system error → Allow)
    if timeout_secs == 0 {
        return HookOutcome::Allow;
    }

    let duration = Duration::from_secs(timeout_secs as u64);

    // Pipe stdin only when the caller has a payload; legacy callers (Pre/PostToolUse)
    // pass None and keep the historical null-stdin behaviour byte-for-byte.
    let stdin_cfg = if stdin_json.is_some() {
        std::process::Stdio::piped()
    } else {
        std::process::Stdio::null()
    };

    // `kill_on_drop(true)` ensures the child is killed if the future is
    // cancelled by the timeout — no lingering background processes.
    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .envs(env.iter().copied())
        .stdin(stdin_cfg)
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

    if let Some(payload) = stdin_json {
        // Drop the handle explicitly so the child sees EOF before we wait.
        // Write failures (BrokenPipe etc.) are infra-level: log and continue,
        // never surface as Intervene.
        if let Some(mut child_stdin) = child.stdin.take() {
            if let Err(e) = child_stdin.write_all(payload.as_bytes()).await {
                tracing::warn!(command = %command, error = %e, "hook stdin write failed");
            } else if let Err(e) = child_stdin.write_all(b"\n").await {
                tracing::warn!(command = %command, error = %e, "hook stdin newline write failed");
            }
            drop(child_stdin);
        }
    }

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
                execute_hook(&hook.command, env, None, hook.timeout_secs.unwrap_or(30)).await
            {
                return BeforeToolCallResult {
                    block: true,
                    reason: Some(message),
                };
            }
        }
        BeforeToolCallResult {
            block: false,
            reason: None,
        }
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
            execute_hook(&hook.command, env, None, hook.timeout_secs.unwrap_or(30)).await;
        }
        AfterToolCallResult {
            content: None,
            is_error: None,
        }
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
        // Compact single-line JSON: scripts use `read line` + grep, so any
        // pretty-printing would split fields across lines and break matches.
        let payload = serde_json::json!({
            "event": "Stop",
            "session_id": ctx.session_id,
            "agent_name": ctx.agent_name,
            "model": ctx.model,
            "turn_count": ctx.turn_count,
            "stop_reason": format!("{:?}", ctx.stop_reason),
            "last_assistant_message": ctx.last_assistant_message,
        })
        .to_string();
        let turn_count_str = ctx.turn_count.to_string();
        let stop_reason_str = format!("{:?}", ctx.stop_reason);

        for hook in &self.hooks {
            // Legacy env vars retained alongside stdin JSON: existing scripts
            // may read either, and SAGE_EVENT=Stop in particular is the
            // back-compat marker for "you're inside a Stop hook".
            let env: &[(&str, &str)] = &[
                ("SAGE_EVENT", "Stop"),
                ("SAGE_AGENT_NAME", ctx.agent_name.as_str()),
                ("SAGE_SESSION_ID", ctx.session_id.as_str()),
                ("SAGE_TURN_COUNT", turn_count_str.as_str()),
                ("SAGE_STOP_REASON", stop_reason_str.as_str()),
                ("SAGE_MODEL", ctx.model.as_str()),
                (
                    "SAGE_LAST_ASSISTANT_MESSAGE",
                    ctx.last_assistant_message.as_str(),
                ),
            ];
            if let HookOutcome::Intervene { message } = execute_hook(
                &hook.command,
                env,
                Some(&payload),
                hook.timeout_secs.unwrap_or(30),
            )
            .await
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
        let outcome = execute_hook("exit 0", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 0 must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_0_with_builtin_true() {
        // `true` is a standard POSIX utility that exits 0
        let outcome = execute_hook("true", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "`true` must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_2_with_stderr_returns_intervene() {
        // The exit-2 protocol: stderr becomes the Intervene message
        let outcome = execute_hook(r#"printf 'blocked by policy' >&2; exit 2"#, &[], None, 5).await;
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
        let outcome = execute_hook("exit 2", &[], None, 5).await;
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
        let outcome = execute_hook("exit 1", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 1 (generic failure) must be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_3_is_allow_not_intervene() {
        // Only exit code 2 triggers Intervene; all other non-zero codes → Allow
        let outcome = execute_hook("exit 3", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 3 must be Allow (only exit 2 → Intervene), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_127_command_not_found_is_allow() {
        // Exit 127 is the shell's "command not found" — still a system error
        let outcome = execute_hook("exit 127", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 127 must be Allow (system error), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn nonexistent_binary_in_command_does_not_panic_or_intervene() {
        // If the script itself references a missing binary, sh exits non-zero.
        // This must never be treated as a policy Intervene.
        let outcome = execute_hook("nonexistent_binary_sage_sprint4_99999", &[], None, 5).await;
        assert!(
            !matches!(outcome, HookOutcome::Intervene { .. }),
            "spawn/exec failure must not Intervene (infrastructure error → Allow), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn timeout_is_allow_not_intervene() {
        // A script that runs longer than timeout_secs must be killed and return Allow.
        // Use 1-second timeout with a 30-second sleep.
        let outcome = execute_hook("sleep 30", &[], None, 1).await;
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
            None,
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
            None,
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
            None,
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
        let outcome = execute_hook(r#"echo "this stdout is ignored"; exit 0"#, &[], None, 5).await;
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
            None,
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
        let outcome = execute_hook("exit 0", &[], None, 0).await;
        let _ = outcome; // just assert no panic
    }

    #[tokio::test]
    async fn large_stderr_output_does_not_panic() {
        // A hook that writes many lines to stderr must not panic or OOM.
        // Implementation may truncate, but Intervene must still be returned.
        let outcome = execute_hook(
            r#"yes "long stderr line from hook" | head -1000 >&2; exit 2"#,
            &[],
            None,
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
        let outcome = execute_hook("", &[], None, 5).await;
        assert!(
            !matches!(outcome, HookOutcome::Intervene { .. }),
            "empty command must be Allow (configuration error → system error), got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn exit_0_with_stderr_written_is_still_allow() {
        // Writing to stderr on exit 0 must NOT trigger Intervene.
        // Only exit 2 triggers it.
        let outcome = execute_hook(r#"printf "diagnostic output" >&2; exit 0"#, &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "exit 0 with stderr must still be Allow, got {outcome:?}"
        );
    }

    // ── Stderr content fidelity ───────────────────────────────────────────────

    #[tokio::test]
    async fn multiline_stderr_preserved_in_message() {
        // All stderr lines must appear in the Intervene message, not just the first.
        let outcome =
            execute_hook(r#"printf 'line1\nline2\nline3' >&2; exit 2"#, &[], None, 5).await;
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
            None,
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
            None,
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
        let outcome = execute_hook("true", &[], None, 5).await;
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
            hooks: vec![HookConfig {
                command: "exit 0".into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook
            .on_stop(&make_stop_ctx("eval-agent", 3, "sess-1"))
            .await;
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
        let action = hook
            .on_stop(&make_stop_ctx("eval-agent", 1, "sess-2"))
            .await;
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

    // =============================================================================
    // Sprint 6 S6.3 — stdin JSON contract for Stop hook
    // =============================================================================
    //
    // The Stop hook migrates from env-var-based payload to a stdin JSON contract
    // (PreToolUse / PostToolUse stay env-based for now; big refactor in S6.2).
    //
    // New execute_hook signature:
    //   execute_hook(command, env, stdin_json: Option<&str>, timeout_secs)
    //
    //   stdin_json: None    → child inherits null stdin (legacy; Pre/PostToolUse)
    //   stdin_json: Some(s) → child stdin is a pipe; s + "\n" is written then
    //                         the pipe is closed so the script sees EOF.
    //
    // These tests are intentionally RED in this TDD wave — the stub impl of
    // execute_hook ignores stdin_json. The next wave wires the pipe through.

    // ── execute_hook: stdin pipe semantics ────────────────────────────────────

    #[tokio::test]
    async fn execute_hook_with_none_stdin_matches_old_behavior() {
        // Legacy call shape: stdin_json=None → unchanged behaviour.
        let outcome = execute_hook("exit 0", &[], None, 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "stdin_json=None + exit 0 must still be Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn execute_hook_with_some_stdin_script_can_read_it() {
        // Script reads a line from stdin and only emits exit 2 (Intervene)
        // when the stdin content matches. Using exit 2 as the success signal
        // means a broken stdin pipe (null stdin) yields exit 1 → Allow,
        // which fails this assertion — exactly the red-light condition.
        let outcome = execute_hook(
            r#"read line; if [ "$line" = "hello" ]; then printf "saw:%s" "$line" >&2; exit 2; else exit 0; fi"#,
            &[],
            Some("hello"),
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("saw:hello"),
                    "script must actually read \"hello\" from stdin, got: {message:?}"
                );
            }
            other => panic!(
                "stdin_json=Some(\"hello\") must reach the script; expected Intervene, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn execute_hook_stdin_is_newline_terminated() {
        // Writer must append '\n' so `read line` terminates without EOF.
        // Non-empty line → exit 2 (Intervene). Empty/missing → exit 0 (Allow).
        let outcome = execute_hook(
            r#"IFS= read -r line; if [ -n "$line" ]; then echo "got" >&2; exit 2; else exit 0; fi"#,
            &[],
            Some("payload"),
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "stdin line must be delivered non-empty and newline-terminated; expected Intervene, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn execute_hook_stdin_with_json_roundtrip() {
        // Write a real JSON blob; script greps the event marker and only emits
        // exit 2 on a successful match (so null-stdin cannot false-positive).
        let json = r#"{"event":"Stop","session_id":"s1"}"#;
        let outcome = execute_hook(
            r#"read line; if printf '%s' "$line" | grep -q '"event":"Stop"'; then echo "match" >&2; exit 2; else exit 0; fi"#,
            &[],
            Some(json),
            5,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "JSON on stdin must roundtrip so grep matches; expected Intervene, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn execute_hook_stdin_intervene_path_still_works() {
        // Even when stdin is wired, the exit-2 protocol must work unchanged.
        let outcome = execute_hook(
            r#"read line; echo "blocked" >&2; exit 2"#,
            &[],
            Some(r#"{"event":"Stop"}"#),
            5,
        )
        .await;
        match outcome {
            HookOutcome::Intervene { message } => {
                assert!(
                    message.contains("blocked"),
                    "stderr must appear in Intervene message, got: {message:?}"
                );
            }
            other => panic!("exit 2 must be Intervene even with stdin pipe, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_hook_empty_stdin_is_valid() {
        // Some("") is a legitimate input: a lone newline is written; script
        // reads an empty line and must not panic on the host side.
        let outcome = execute_hook(r#"IFS= read -r line; exit 0"#, &[], Some(""), 5).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "empty stdin must not panic and must yield Allow, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn execute_hook_large_stdin_payload() {
        // A ~10KB payload must not deadlock or truncate. `wc -c` counts bytes
        // including the terminating newline; exit 2 only when the full payload
        // arrived, so a null-stdin stub implementation fails this assertion.
        let big = "a".repeat(10_000);
        let outcome = execute_hook(
            r#"n=$(wc -c); if [ "$n" -ge 10000 ]; then echo "big:$n" >&2; exit 2; else exit 0; fi"#,
            &[],
            Some(&big),
            10,
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Intervene { .. }),
            "~10KB stdin payload must be delivered intact; expected Intervene, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn execute_hook_stdin_write_failure_returns_allow() {
        // If the child closes its stdin before we finish writing, the host
        // write may fail. That must be treated as infra failure → Allow,
        // never a panic.
        let outcome = execute_hook(
            // Close stdin immediately, then exit 0.
            r#"exec </dev/null; exit 0"#,
            &[],
            Some(r#"{"event":"Stop"}"#),
            5,
        )
        .await;
        assert!(
            !matches!(outcome, HookOutcome::Intervene { .. }),
            "stdin write race must not surface as Intervene, got {outcome:?}"
        );
    }

    // ── ScriptStopHook: stdin JSON payload content ────────────────────────────

    // Scripts grep the stdin line for the expected field and exit 2 (→ Continue)
    // only on a successful match; any other outcome (including null-stdin /
    // grep-miss) allows and the loop ends in Pass. Asserting Continue keeps
    // these tests honest: they fail until the JSON really flows through stdin.

    #[tokio::test]
    async fn stop_hook_sends_stdin_json_with_event_field() {
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q '"event":"Stop"'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook
            .on_stop(&make_stop_ctx("agent-x", 1, "sess-json-evt"))
            .await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain \"event\":\"Stop\", expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_stdin_json_contains_session_id() {
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q '"session_id":"sess-abc"'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 1, "sess-abc")).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain session_id, expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_stdin_json_contains_agent_name_and_model() {
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q '"agent_name":"my-agent"' && printf '%s' "$line" | grep -q '"model":"test-model"'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("my-agent", 1, "sess-am")).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain agent_name AND model, expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_stdin_json_contains_turn_count() {
        // turn_count must be serialized as a JSON number, not a string.
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q '"turn_count":7'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 7, "sess-tc")).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain turn_count as number, expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_stdin_json_contains_stop_reason() {
        // stop_reason uses {:?} format → for StopReason::Stop this is "Stop".
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q '"stop_reason":"Stop"'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 1, "sess-sr")).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain stop_reason, expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_stdin_json_contains_last_assistant_message() {
        let ctx = StopContext {
            stop_reason: sage_runtime::types::StopReason::Stop,
            session_id: "sess-lam".into(),
            task_id: "sess-lam".into(),
            turn_count: 1,
            agent_name: "agent".into(),
            model: "test-model".into(),
            last_assistant_message: "hello world".into(),
        };
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; if printf '%s' "$line" | grep -q 'hello world'; then echo ok >&2; exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&ctx).await;
        assert!(
            matches!(action, StopAction::Continue(_)),
            "stdin JSON must contain last_assistant_message, expected Continue, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_sage_event_env_still_set() {
        // SAGE_EVENT=Stop is retained as a minimal backward-compat signal so
        // legacy env-reading scripts can detect they're running in a Stop hook
        // (even if they need to migrate other fields to stdin JSON).
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line >/dev/null 2>&1 || true; [ "$SAGE_EVENT" = "Stop" ] && exit 0 || exit 1"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 1, "sess-env")).await;
        assert!(
            matches!(action, StopAction::Pass),
            "SAGE_EVENT=Stop env must still be set, got {action:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_exit_2_with_stderr_returns_continue() {
        // Exit-2 protocol must continue to work under the new stdin contract.
        let hook = ScriptStopHook {
            hooks: vec![HookConfig {
                command: r#"read line; echo "bad shutdown" >&2; exit 2"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 1, "sess-exit2")).await;
        match action {
            StopAction::Continue(msg) => {
                assert!(
                    msg.contains("bad shutdown"),
                    "stderr must appear in Continue, got: {msg:?}"
                );
            }
            other => panic!("expected Continue under new stdin contract, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stop_hook_multiple_hooks_first_intervene_wins() {
        // When hook #1 intervenes (exit 2), hook #2 must NOT run.
        // Each hook touches a unique file; we verify only hook #1's file exists.
        let dir = std::env::temp_dir();
        let uniq = format!(
            "sage_stop_order_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let marker1 = dir.join(format!("{uniq}.1"));
        let marker2 = dir.join(format!("{uniq}.2"));

        // Make sure markers are clean.
        let _ = std::fs::remove_file(&marker1);
        let _ = std::fs::remove_file(&marker2);

        let hook = ScriptStopHook {
            hooks: vec![
                HookConfig {
                    command: format!(
                        r#"read line; touch {m1}; echo "stop" >&2; exit 2"#,
                        m1 = marker1.display()
                    ),
                    timeout_secs: Some(5),
                },
                HookConfig {
                    command: format!(r#"read line; touch {m2}; exit 0"#, m2 = marker2.display()),
                    timeout_secs: Some(5),
                },
            ],
        };
        let action = hook.on_stop(&make_stop_ctx("agent", 1, "sess-order")).await;

        assert!(
            matches!(action, StopAction::Continue(_)),
            "first hook exit 2 → Continue, got {action:?}"
        );
        assert!(
            marker1.exists(),
            "first hook must have run (marker1 missing at {})",
            marker1.display()
        );
        assert!(
            !marker2.exists(),
            "second hook must NOT run after first intervenes (marker2 unexpectedly at {})",
            marker2.display()
        );

        // Cleanup.
        let _ = std::fs::remove_file(&marker1);
        let _ = std::fs::remove_file(&marker2);
    }

    // ── Pre/PostToolUse: backward-compat — still env-driven, no stdin ────────

    #[tokio::test]
    async fn pre_tool_use_still_uses_env_vars_not_stdin() {
        use sage_runtime::types::BeforeToolCallContext;
        use serde_json::json;

        // PreToolUse script reads ONLY env vars — it must never depend on stdin.
        let hook = ScriptPreToolUseHook {
            hooks: vec![HookConfig {
                command: r#"if [ "$SAGE_TOOL_NAME" = "bash" ]; then exit 2; fi; exit 0"#.into(),
                timeout_secs: Some(5),
            }],
        };
        let ctx = BeforeToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({"command": "ls"}),
        };
        let result = hook.before_tool_call(&ctx).await;
        assert!(
            result.block,
            "PreToolUse env-based path must still work (SAGE_TOOL_NAME=bash → exit 2 → block)"
        );
    }

    #[tokio::test]
    async fn post_tool_use_still_uses_env_vars_not_stdin() {
        use sage_runtime::types::AfterToolCallContext;
        use serde_json::json;

        // PostToolUse is observe-only; verify the script's env-read succeeds by
        // using a side-effect marker file.
        let dir = std::env::temp_dir();
        let uniq = format!(
            "sage_post_env_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let marker = dir.join(uniq);
        let _ = std::fs::remove_file(&marker);

        let hook = ScriptPostToolUseHook {
            hooks: vec![HookConfig {
                command: format!(
                    r#"if [ "$SAGE_TOOL_NAME" = "bash" ]; then touch {m}; fi; exit 0"#,
                    m = marker.display()
                ),
                timeout_secs: Some(5),
            }],
        };
        let ctx = AfterToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
            is_error: false,
        };
        let _ = hook.after_tool_call(&ctx).await;
        assert!(
            marker.exists(),
            "PostToolUse env-based path must still work (SAGE_TOOL_NAME=bash → marker at {})",
            marker.display()
        );
        let _ = std::fs::remove_file(&marker);
    }
}
