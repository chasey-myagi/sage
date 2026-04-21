//! Hook execution engine — runs a single hook command and parses its output.
//!
//! Translated from CC `src/utils/hooks.ts` (command execution path).

use std::time::Duration;

use anyhow::{Context as _, Result};

use super::types::{
    HookBlockingError, HookCommand, HookInput, HookJsonOutput, HookOutcome, HookResult,
};

/// Default per-hook timeout for tool-related hooks (PreToolUse / PostToolUse).
const DEFAULT_TOOL_HOOK_TIMEOUT_SECS: u64 = 600; // 10 minutes

/// Executes hook commands within a session context.
pub struct HookExecutor {
    pub session_id: String,
    pub cwd: String,
    pub transcript_path: String,
}

impl HookExecutor {
    pub fn new(session_id: impl Into<String>, cwd: impl Into<String>) -> Self {
        let session_id = session_id.into();
        let cwd = cwd.into();
        let transcript_path = String::new();
        Self {
            session_id,
            cwd,
            transcript_path,
        }
    }

    /// Execute a hook command and return the result.
    pub async fn execute(&self, hook: &HookCommand, input: &HookInput) -> Result<HookResult> {
        match hook {
            HookCommand::Command {
                command,
                timeout,
                async_,
                shell,
                ..
            } => {
                let is_async = async_.unwrap_or(false);
                let shell_prog = shell.as_deref().unwrap_or("bash").to_string();
                if is_async {
                    // Async hooks run in background — fire and forget (no asyncRewake support yet).
                    let command = command.clone();
                    let cwd = self.cwd.clone();
                    let input_json = serde_json::to_string(input).unwrap_or_default();
                    let hook_event = input.hook_event_name.clone();
                    tokio::spawn(async move {
                        let _ = spawn_command(
                            &command,
                            &shell_prog,
                            &cwd,
                            &input_json,
                            &hook_event,
                            None,
                        )
                        .await;
                    });
                    return Ok(HookResult {
                        outcome: HookOutcome::Success,
                        ..Default::default()
                    });
                }
                self.execute_command_hook(command, *timeout, &shell_prog, input)
                    .await
            }
            // Prompt / Http / Agent hooks are not yet implemented.
            HookCommand::Prompt { .. } | HookCommand::Http { .. } | HookCommand::Agent { .. } => {
                Ok(HookResult {
                    outcome: HookOutcome::Success,
                    ..Default::default()
                })
            }
        }
    }

    async fn execute_command_hook(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
        shell: &str,
        input: &HookInput,
    ) -> Result<HookResult> {
        let input_json = serde_json::to_string(input).context("serialize hook input")?;
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TOOL_HOOK_TIMEOUT_SECS));

        let result = tokio::time::timeout(
            timeout,
            spawn_command(
                command,
                shell,
                &self.cwd,
                &input_json,
                &input.hook_event_name,
                None,
            ),
        )
        .await;

        match result {
            Ok(Ok((stdout, stderr, exit_code))) => {
                Ok(parse_hook_output(&stdout, &stderr, exit_code, command))
            }
            Ok(Err(e)) => Ok(HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: Some(format!("Hook execution error: {e}")),
                ..Default::default()
            }),
            Err(_elapsed) => Ok(HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: Some(format!(
                    "Hook timed out after {}s: {command}",
                    timeout.as_secs()
                )),
                ..Default::default()
            }),
        }
    }
}

/// Spawn a shell command, capture stdout/stderr, and return exit code.
async fn spawn_command(
    command: &str,
    shell: &str,
    cwd: &str,
    input_json: &str,
    hook_event: &str,
    hook_id: Option<&str>,
) -> Result<(String, String, i32)> {
    let generated_id = ulid::Ulid::new().to_string();
    let id = hook_id.unwrap_or(&generated_id);

    let output = tokio::process::Command::new(shell)
        .arg("-c")
        .arg(command)
        .env("HOOK_INPUT_JSON", input_json)
        .env("CLAUDE_HOOK_ID", id)
        .env("CLAUDE_HOOK_EVENT", hook_event)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("spawn hook shell command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);
    Ok((stdout, stderr, exit_code))
}

/// Parse hook output (stdout + exit code) into a `HookResult`.
///
/// Protocol:
/// - exit 0 → success, parse JSON if starts with `{`
/// - exit 1 → non-blocking error
/// - exit 2 → blocking error (stderr preferred for message)
/// - other  → non-blocking error
fn parse_hook_output(stdout: &str, stderr: &str, exit_code: i32, command: &str) -> HookResult {
    // exit 2: hard block
    if exit_code == 2 {
        let message = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("Hook blocked execution (exit 2): {command}")
        };
        return HookResult {
            outcome: HookOutcome::Blocking,
            stdout: Some(stdout.to_string()),
            stderr: Some(stderr.to_string()),
            blocking_error: Some(HookBlockingError {
                message,
                command: command.to_string(),
            }),
            ..Default::default()
        };
    }

    // exit 1: non-blocking error
    if exit_code == 1 {
        return HookResult {
            outcome: HookOutcome::NonBlockingError,
            stdout: Some(stdout.to_string()),
            stderr: Some(stderr.to_string()),
            ..Default::default()
        };
    }

    // Other non-zero exit codes
    if exit_code != 0 {
        return HookResult {
            outcome: HookOutcome::NonBlockingError,
            stdout: Some(stdout.to_string()),
            stderr: Some(stderr.to_string()),
            ..Default::default()
        };
    }

    // exit 0: attempt JSON parse if output looks like JSON
    let trimmed = stdout.trim();
    if trimmed.starts_with('{') {
        match serde_json::from_str::<HookJsonOutput>(trimmed) {
            Ok(json_out) => return process_json_output(json_out, stdout, stderr, command),
            Err(_) => {
                // Malformed JSON — fall through to plain text
            }
        }
    }

    // Plain text output
    HookResult {
        outcome: HookOutcome::Success,
        stdout: Some(stdout.to_string()),
        stderr: Some(stderr.to_string()),
        ..Default::default()
    }
}

/// Translate a validated `HookJsonOutput` into a `HookResult`.
fn process_json_output(
    json: HookJsonOutput,
    stdout: &str,
    stderr: &str,
    command: &str,
) -> HookResult {
    // Legacy "decision: block" field
    if json.decision.as_deref() == Some("block") {
        let message = json
            .reason
            .clone()
            .or_else(|| json.system_message.clone())
            .unwrap_or_else(|| "Blocked by hook".to_string());
        return HookResult {
            outcome: HookOutcome::Blocking,
            stdout: Some(stdout.to_string()),
            stderr: Some(stderr.to_string()),
            system_message: json.system_message,
            blocking_error: Some(HookBlockingError {
                message,
                command: command.to_string(),
            }),
            ..Default::default()
        };
    }

    let prevent_continuation = json.continue_.map(|c| !c).unwrap_or(false);

    let (additional_context, updated_input, permission_decision, permission_decision_reason) =
        extract_hook_specific(json.hook_specific_output.as_ref());

    HookResult {
        outcome: HookOutcome::Success,
        stdout: Some(stdout.to_string()),
        stderr: Some(stderr.to_string()),
        system_message: json.system_message,
        prevent_continuation,
        stop_reason: json.stop_reason,
        additional_context,
        updated_input,
        permission_decision,
        permission_decision_reason,
        ..Default::default()
    }
}

fn extract_hook_specific(
    specific: Option<&serde_json::Value>,
) -> (
    Option<String>,
    Option<serde_json::Value>,
    Option<String>,
    Option<String>,
) {
    let Some(obj) = specific else {
        return (None, None, None, None);
    };
    let additional_context = obj
        .get("additionalContext")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let updated_input = obj.get("updatedInput").cloned();
    let permission_decision = obj
        .get("permissionDecision")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let permission_decision_reason = obj
        .get("permissionDecisionReason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    (
        additional_context,
        updated_input,
        permission_decision,
        permission_decision_reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exit_2_returns_blocking() {
        let result = parse_hook_output("some output", "error msg", 2, "test_cmd");
        assert_eq!(result.outcome, HookOutcome::Blocking);
        assert!(result.blocking_error.is_some());
        let err = result.blocking_error.unwrap();
        // stderr preferred for message
        assert_eq!(err.message, "error msg");
        assert_eq!(err.command, "test_cmd");
    }

    #[test]
    fn parse_exit_2_uses_stdout_when_stderr_empty() {
        let result = parse_hook_output("blocked output", "", 2, "cmd");
        assert_eq!(result.outcome, HookOutcome::Blocking);
        assert_eq!(result.blocking_error.unwrap().message, "blocked output");
    }

    #[test]
    fn parse_exit_1_returns_non_blocking_error() {
        let result = parse_hook_output("", "oops", 1, "cmd");
        assert_eq!(result.outcome, HookOutcome::NonBlockingError);
        assert!(result.blocking_error.is_none());
    }

    #[test]
    fn parse_exit_0_plain_text_is_success() {
        let result = parse_hook_output("hello world", "", 0, "cmd");
        assert_eq!(result.outcome, HookOutcome::Success);
        assert!(result.blocking_error.is_none());
    }

    #[test]
    fn parse_exit_0_json_decision_block() {
        let stdout = r#"{"decision":"block","reason":"not allowed"}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(result.outcome, HookOutcome::Blocking);
        let err = result.blocking_error.unwrap();
        assert_eq!(err.message, "not allowed");
    }

    #[test]
    fn parse_exit_0_json_continue_false() {
        let stdout = r#"{"continue":false,"stopReason":"session finished"}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(result.outcome, HookOutcome::Success);
        assert!(result.prevent_continuation);
        assert_eq!(result.stop_reason.as_deref(), Some("session finished"));
    }

    #[test]
    fn parse_exit_0_json_additional_context() {
        let stdout = r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"file saved OK"}}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(result.outcome, HookOutcome::Success);
        assert_eq!(result.additional_context.as_deref(), Some("file saved OK"));
    }

    #[test]
    fn parse_exit_0_json_permission_decision() {
        let stdout = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"verified"}}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(result.permission_decision.as_deref(), Some("allow"));
        assert_eq!(
            result.permission_decision_reason.as_deref(),
            Some("verified")
        );
    }

    #[test]
    fn parse_exit_0_json_updated_input() {
        let stdout = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"path":"/safe/path"}}}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        let input = result.updated_input.unwrap();
        assert_eq!(input["path"], "/safe/path");
    }

    #[test]
    fn parse_exit_0_malformed_json_fallback_to_plain_text() {
        let stdout = "{not valid json}";
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(result.outcome, HookOutcome::Success);
        assert!(result.blocking_error.is_none());
    }

    #[test]
    fn parse_exit_0_json_system_message() {
        let stdout = r#"{"systemMessage":"Warning: deprecated tool"}"#;
        let result = parse_hook_output(stdout, "", 0, "cmd");
        assert_eq!(
            result.system_message.as_deref(),
            Some("Warning: deprecated tool")
        );
    }

    #[test]
    fn parse_other_nonzero_exit_code_is_non_blocking() {
        let result = parse_hook_output("", "", 127, "cmd");
        assert_eq!(result.outcome, HookOutcome::NonBlockingError);
        assert!(result.blocking_error.is_none());
    }
}
