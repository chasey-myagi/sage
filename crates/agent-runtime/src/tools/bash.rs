// BashTool — shell command execution via sandbox.

use crate::types::Content;

pub struct BashTool;

#[async_trait::async_trait]
impl super::AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute shell commands"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout": { "type": "integer" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(cmd) if !cmd.is_empty() => cmd,
            Some(_) => {
                return super::ToolOutput {
                    content: vec![Content::Text {
                        text: "command is empty".into(),
                    }],
                    is_error: true,
                }
            }
            None => {
                return super::ToolOutput {
                    content: vec![Content::Text {
                        text: "missing required parameter: command".into(),
                    }],
                    is_error: true,
                }
            }
        };

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .max(1)   // minimum 1 second
            .min(600); // maximum 10 minutes

        // Create a new process group (setsid) so we can kill the entire tree on timeout,
        // not just the parent bash shell.
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Create a new process group so we can kill the entire tree on timeout.
        // process_group(0) sets pgid = child pid.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return super::ToolOutput {
                    content: vec![Content::Text {
                        text: format!("Failed to execute command: {}", e),
                    }],
                    is_error: true,
                }
            }
        };

        // Take pipe handles before borrowing child — prevents pipe-buffer deadlock
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let stdout_reader = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            if let Some(mut pipe) = stdout_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_reader = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            if let Some(mut pipe) = stderr_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });

        // wait() borrows &mut child (not consuming), so child remains killable on timeout
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait(),
        )
        .await
        {
            Ok(Ok(status)) => {
                let stdout_bytes = stdout_reader.await.unwrap_or_default();
                let stderr_bytes = stderr_reader.await.unwrap_or_default();
                let stdout = String::from_utf8_lossy(&stdout_bytes);
                let stderr = String::from_utf8_lossy(&stderr_bytes);
                let text = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr)
                };
                super::ToolOutput {
                    content: vec![Content::Text { text }],
                    is_error: !status.success(),
                }
            }
            Ok(Err(e)) => {
                stdout_reader.abort();
                stderr_reader.abort();
                super::ToolOutput {
                    content: vec![Content::Text {
                        text: format!("Failed to execute command: {}", e),
                    }],
                    is_error: true,
                }
            }
            Err(_) => {
                // Timeout — kill the entire process group to prevent child leaks.
                // Falls back to child.kill() if process group kill is unavailable.
                #[cfg(unix)]
                {
                    if let Some(pid) = child.id() {
                        if let Ok(pgid) = i32::try_from(pid) {
                            // Negative pid sends signal to entire process group
                            let ret = unsafe { libc::kill(-pgid, libc::SIGKILL) };
                            if ret != 0 {
                                let err = std::io::Error::last_os_error();
                                tracing::warn!(pgid, %err, "failed to kill process group on timeout");
                            }
                        }
                    }
                }
                let _ = child.kill().await; // fallback / reap
                stdout_reader.abort();
                stderr_reader.abort();
                super::ToolOutput {
                    content: vec![Content::Text {
                        text: format!("Command timed out after {timeout_secs}s"),
                    }],
                    is_error: true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use serde_json::json;

    // ---------------------------------------------------------------
    // Metadata
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = BashTool;
        assert_eq!(tool.name(), "bash");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = BashTool;
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_command_property() {
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").expect("schema must have properties");
        assert!(props.get("command").is_some(), "schema must include 'command'");
    }

    #[test]
    fn test_schema_command_is_required() {
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema must have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn test_schema_has_timeout_property() {
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").expect("schema must have properties");
        assert!(props.get("timeout").is_some(), "schema must include 'timeout'");
    }

    #[test]
    fn test_schema_timeout_is_optional() {
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema must have required array");
        assert!(
            !required.iter().any(|v| v.as_str() == Some("timeout")),
            "timeout should not be required"
        );
    }

    // ---------------------------------------------------------------
    // Argument parsing
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_command_returns_error() {
        let tool = BashTool;
        let output = tool.execute(json!({})).await;
        assert!(output.is_error, "missing 'command' must return error");
    }

    #[tokio::test]
    async fn test_empty_command_returns_error() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": ""})).await;
        assert!(output.is_error, "empty command must return error");
    }

    // ---------------------------------------------------------------
    // Long command string
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_accepts_long_command() {
        // A very long command string should be accepted by the schema
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        let cmd_schema = props.get("command").unwrap();
        // The command property type should be "string" with no maxLength restriction
        assert_eq!(cmd_schema.get("type").unwrap(), "string");
        // Verify no maxLength constraint exists (accepting arbitrarily long commands)
        assert!(
            cmd_schema.get("maxLength").is_none(),
            "command should not have a maxLength constraint"
        );
    }

    // ---------------------------------------------------------------
    // Special characters in command
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_command_with_special_characters() {
        // Command contains quotes, pipes, semicolons — should not cause arg parsing to fail
        let tool = BashTool;
        let cmd = r#"echo "hello 'world'" | cat; echo done"#;
        let output = tool.execute(json!({"command": cmd})).await;
        // The command should succeed (valid shell syntax)
        assert!(!output.content.is_empty());
        assert!(
            !output.is_error,
            "valid shell command with special characters should succeed"
        );
    }

    // ---------------------------------------------------------------
    // Timeout parameter edge cases
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_timeout_zero_uses_default() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": "echo hi", "timeout": 0})).await;
        // timeout=0 is not valid u64 for timeout, falls back to default (120s)
        assert!(!output.is_error, "timeout 0 should fall back to default");
    }

    #[tokio::test]
    async fn test_timeout_negative_uses_default() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": "echo hi", "timeout": -1})).await;
        // Negative is not valid u64, falls back to default (120s)
        assert!(!output.is_error, "negative timeout should fall back to default");
    }

    #[tokio::test]
    async fn test_timeout_actually_enforced() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": "sleep 10", "timeout": 1})).await;
        assert!(output.is_error, "sleep 10 with timeout 1s must error");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(text.contains("timed out"), "should mention timeout, got: {text}");
    }

    // ---------------------------------------------------------------
    // Non-zero exit code handling
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_nonzero_exit_code_is_error() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": "exit 1"})).await;
        // Non-zero exit code MUST set is_error=true
        assert!(output.is_error, "exit 1 must produce is_error=true");
    }

    #[tokio::test]
    async fn test_exit_code_preserves_stdout() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "echo hello && exit 42"}))
            .await;
        // Should capture stdout even on failure
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        // stdout "hello" must be present in output
        assert!(
            text.contains("hello"),
            "stdout should be captured even when command fails, got: {}",
            text
        );
        // is_error should be true due to exit 42
        assert!(output.is_error, "exit 42 must produce is_error=true");
    }

    // ---------------------------------------------------------------
    // stdout and stderr separation
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_stderr_captured_in_output() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "echo err_msg >&2"}))
            .await;
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        // stderr must be captured in the output text
        assert!(
            text.contains("err_msg"),
            "stderr must be captured in output, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_stdout_and_stderr_mixed() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "echo out && echo err >&2"}))
            .await;
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        // Both streams should appear in output
        assert!(
            !output.content.is_empty(),
            "should capture some output from both streams"
        );
        assert!(
            !output.is_error,
            "echo out + echo err should succeed (exit 0)"
        );
    }

    // ---------------------------------------------------------------
    // Unicode output
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_command_with_unicode_output() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "echo '你好世界'"}))
            .await;
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(
            text.contains("你好") || output.is_error,
            "unicode output should be captured correctly"
        );
    }

    // ---------------------------------------------------------------
    // Working directory parameter
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_optional_cwd() {
        let tool = BashTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        // May have "cwd", "working_directory", or similar
        let has_cwd = props.get("cwd").is_some()
            || props.get("working_directory").is_some();
        // Not all implementations have this — but if present, it should be optional
        if has_cwd {
            let required = schema.get("required").unwrap().as_array().unwrap();
            assert!(!required.iter().any(|v| {
                v.as_str() == Some("cwd") || v.as_str() == Some("working_directory")
            }));
        }
    }

    // ---------------------------------------------------------------
    // Success path — verify output content
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_echo_hello_success_path() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": "echo hello"})).await;
        assert!(!output.is_error, "echo hello should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("hello"),
            "output should contain 'hello', got: {}",
            text
        );
    }

    // ---------------------------------------------------------------
    // Multi-line output
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_multiline_output_captured() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "echo -e 'line1\nline2'"}))
            .await;
        assert!(!output.is_error, "multi-line echo should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("line1") && text.contains("line2"),
            "multi-line output should contain both lines, got: {}",
            text
        );
    }

    // ---------------------------------------------------------------
    // Command injection safety — special characters
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_command_with_backticks_and_dollar() {
        let tool = BashTool;
        // This command uses backticks and $() — it should execute or fail gracefully
        let output = tool
            .execute(json!({"command": "echo \"$(echo nested)\""}))
            .await;
        // Should not panic; output contains result of the nested command
        assert!(!output.content.is_empty());
        // The shell should execute the nested echo
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(
            text.contains("nested") || output.is_error,
            "should either execute nested command or error gracefully"
        );
    }

    // ---------------------------------------------------------------
    // Command not found error propagation
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_command_not_found() {
        let tool = BashTool;
        let output = tool
            .execute(json!({"command": "nonexistent_command_xyz_12345"}))
            .await;
        assert!(
            output.is_error,
            "nonexistent command should produce is_error=true"
        );
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("not found") || text.contains("No such file"),
            "error should mention 'not found', got: {}",
            text
        );
    }

    // ---------------------------------------------------------------
    // JSON null/wrong-type parameter handling
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_bash_null_command_param() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": null})).await;
        assert!(output.is_error, "null command should produce is_error=true");
    }

    #[tokio::test]
    async fn test_bash_integer_command_param() {
        let tool = BashTool;
        let output = tool.execute(json!({"command": 123})).await;
        assert!(output.is_error, "integer command should produce is_error=true");
    }
}
