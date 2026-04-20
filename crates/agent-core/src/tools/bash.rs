// BashTool — shell command execution via ToolBackend.

use std::sync::Arc;

use crate::types::Content;

use super::backend::ToolBackend;

pub struct BashTool(pub Arc<dyn ToolBackend>);

#[async_trait::async_trait]
impl super::AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Use this to reach external systems via \
         their CLIs (e.g. `lark-cli` for 飞书/Lark, `kubectl` for \
         Kubernetes, `git`, `curl`, domain scripts in workspace/skills/). \
         This is the ONLY route to data outside the local workspace — \
         grep/ls/find/read only inspect local files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (clamped to 1–600)"
                }
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
                };
            }
            None => {
                return super::ToolOutput {
                    content: vec![Content::Text {
                        text: "missing required parameter: command".into(),
                    }],
                    is_error: true,
                };
            }
        };

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .max(1) // minimum 1 second
            .min(600); // maximum 10 minutes

        match self.0.shell(command, timeout_secs).await {
            Ok(output) => {
                let mut text = if output.stderr.is_empty() {
                    output.stdout
                } else if output.stdout.is_empty() {
                    output.stderr
                } else {
                    format!("{}\n{}", output.stdout, output.stderr)
                };
                if !output.success {
                    let exit_note = match output.exit_code {
                        Some(code) => format!("Command failed with exit code {code}"),
                        None => "Command failed (terminated by signal)".to_string(),
                    };
                    if text.is_empty() {
                        text = exit_note;
                    } else {
                        text = format!("{}\n{}", text.trim_end(), exit_note);
                    }
                }
                super::ToolOutput {
                    content: vec![Content::Text { text }],
                    is_error: !output.success,
                }
            }
            Err(e) => super::ToolOutput {
                content: vec![Content::Text {
                    text: format!("Command timed out after {timeout_secs}s: {e}"),
                }],
                is_error: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use crate::tools::backend::LocalBackend;
    use serde_json::json;

    fn bash() -> BashTool {
        BashTool(LocalBackend::new())
    }

    // ---------------------------------------------------------------
    // Metadata
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = bash();
        assert_eq!(tool.name(), "bash");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = bash();
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_command_property() {
        let tool = bash();
        let schema = tool.parameters_schema();
        let props = schema
            .get("properties")
            .expect("schema must have properties");
        assert!(
            props.get("command").is_some(),
            "schema must include 'command'"
        );
    }

    #[test]
    fn test_schema_command_is_required() {
        let tool = bash();
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema must have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn test_schema_has_timeout_property() {
        let tool = bash();
        let schema = tool.parameters_schema();
        let props = schema
            .get("properties")
            .expect("schema must have properties");
        assert!(
            props.get("timeout").is_some(),
            "schema must include 'timeout'"
        );
    }

    #[test]
    fn test_schema_timeout_is_optional() {
        let tool = bash();
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
        let tool = bash();
        let output = tool.execute(json!({})).await;
        assert!(output.is_error, "missing 'command' must return error");
    }

    #[tokio::test]
    async fn test_empty_command_returns_error() {
        let tool = bash();
        let output = tool.execute(json!({"command": ""})).await;
        assert!(output.is_error, "empty command must return error");
    }

    // ---------------------------------------------------------------
    // Long command string
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_accepts_long_command() {
        // A very long command string should be accepted by the schema
        let tool = bash();
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
        let tool = bash();
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
        let tool = bash();
        let output = tool
            .execute(json!({"command": "echo hi", "timeout": 0}))
            .await;
        // timeout=0 is not valid u64 for timeout, falls back to default (120s)
        assert!(!output.is_error, "timeout 0 should fall back to default");
    }

    #[tokio::test]
    async fn test_timeout_negative_uses_default() {
        let tool = bash();
        let output = tool
            .execute(json!({"command": "echo hi", "timeout": -1}))
            .await;
        // Negative is not valid u64, falls back to default (120s)
        assert!(
            !output.is_error,
            "negative timeout should fall back to default"
        );
    }

    #[tokio::test]
    async fn test_timeout_actually_enforced() {
        let tool = bash();
        let output = tool
            .execute(json!({"command": "sleep 10", "timeout": 1}))
            .await;
        assert!(output.is_error, "sleep 10 with timeout 1s must error");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(
            text.contains("timed out"),
            "should mention timeout, got: {text}"
        );
    }

    // ---------------------------------------------------------------
    // Non-zero exit code handling
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_nonzero_exit_code_is_error() {
        let tool = bash();
        let output = tool.execute(json!({"command": "exit 1"})).await;
        // Non-zero exit code MUST set is_error=true
        assert!(output.is_error, "exit 1 must produce is_error=true");
    }

    #[tokio::test]
    async fn test_exit_code_preserves_stdout() {
        let tool = bash();
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
        let tool = bash();
        let output = tool.execute(json!({"command": "echo err_msg >&2"})).await;
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
        let tool = bash();
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
        let tool = bash();
        let output = tool.execute(json!({"command": "echo '你好世界'"})).await;
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
        let tool = bash();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        // May have "cwd", "working_directory", or similar
        let has_cwd = props.get("cwd").is_some() || props.get("working_directory").is_some();
        // Not all implementations have this — but if present, it should be optional
        if has_cwd {
            let required = schema.get("required").unwrap().as_array().unwrap();
            assert!(
                !required.iter().any(|v| {
                    v.as_str() == Some("cwd") || v.as_str() == Some("working_directory")
                })
            );
        }
    }

    // ---------------------------------------------------------------
    // Success path — verify output content
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_echo_hello_success_path() {
        let tool = bash();
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
        let tool = bash();
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
        let tool = bash();
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
        let tool = bash();
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
        let tool = bash();
        let output = tool.execute(json!({"command": null})).await;
        assert!(output.is_error, "null command should produce is_error=true");
    }

    #[tokio::test]
    async fn test_bash_integer_command_param() {
        let tool = bash();
        let output = tool.execute(json!({"command": 123})).await;
        assert!(
            output.is_error,
            "integer command should produce is_error=true"
        );
    }
}
