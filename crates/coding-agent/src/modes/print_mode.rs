//! Print mode (single-shot): send prompts, output result, exit.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/print-mode.ts`.
//!
//! Used for:
//! - `sage -p "prompt"` — text output
//! - `sage --mode json "prompt"` — JSON event stream

use std::io::Write;

use crate::cli::args::Mode;

// ============================================================================
// Types
// ============================================================================

/// Output of the agent session after processing a prompt (placeholder).
/// In a full implementation this would be the actual AgentSession API.
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub role: String,
    pub content: Vec<MessageContent>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
}

/// A single piece of content within an agent message.
#[derive(Debug, Clone)]
pub enum MessageContent {
    Text { text: String },
    ToolUse { id: String, name: String },
}

/// Options for print mode.
#[derive(Debug, Default)]
pub struct PrintModeOptions {
    /// Output mode: `Text` for final response only, `Json` for all events.
    pub mode: Mode,
    /// Additional prompts to send after `initial_message`.
    pub messages: Vec<String>,
    /// First message (may contain @file content).
    pub initial_message: Option<String>,
}

/// Result of running print mode.
#[derive(Debug)]
pub struct PrintModeResult {
    /// Exit code: 0 for success, 1 for error.
    pub exit_code: i32,
}

// ============================================================================
// Session trait (minimal interface for print mode)
// ============================================================================

/// Minimal interface needed by print mode to interact with an agent session.
/// This allows print mode to be tested without a real LLM connection.
#[async_trait::async_trait]
pub trait PrintModeSession: Send + Sync {
    /// Send a prompt to the agent and wait for completion.
    async fn prompt(&mut self, message: &str) -> anyhow::Result<()>;
    /// Get the last assistant message (if any).
    fn last_assistant_message(&self) -> Option<AgentMessage>;
    /// Serialize an event to JSON (for JSON mode).
    fn event_to_json(&self) -> Option<String>;
}

// ============================================================================
// run_print_mode
// ============================================================================

/// Run in print (single-shot) mode.
///
/// Sends prompts to the agent and outputs the result, mirroring
/// `runPrintMode()` in print-mode.ts.
pub async fn run_print_mode<S: PrintModeSession>(
    session: &mut S,
    options: PrintModeOptions,
) -> anyhow::Result<PrintModeResult> {
    let mut exit_code = 0i32;
    let stdout = std::io::stdout();

    // Send initial message
    if let Some(msg) = &options.initial_message
        && let Err(e) = session.prompt(msg).await
    {
        eprintln!("Error: {e}");
        exit_code = 1;
    }

    // Send remaining messages
    for msg in &options.messages {
        if let Err(e) = session.prompt(msg).await {
            eprintln!("Error: {e}");
            exit_code = 1;
        }
    }

    // Output result
    match options.mode {
        Mode::Json => {
            // In JSON mode, the caller should be subscribing to events.
            // We emit a final flush here.
            if let Some(json) = session.event_to_json() {
                let mut out = stdout.lock();
                let _ = writeln!(out, "{json}");
            }
        }
        Mode::Text | Mode::Rpc => {
            // Text mode: output final assistant response
            if let Some(msg) = session.last_assistant_message() {
                let stop_reason = msg.stop_reason.as_deref().unwrap_or("");
                if stop_reason == "error" || stop_reason == "aborted" {
                    eprintln!(
                        "{}",
                        msg.error_message
                            .as_deref()
                            .unwrap_or(&format!("Request {stop_reason}"))
                    );
                    exit_code = 1;
                } else {
                    let mut out = stdout.lock();
                    for content in &msg.content {
                        if let MessageContent::Text { text } = content {
                            let _ = writeln!(out, "{text}");
                        }
                    }
                }
            }
        }
    }

    Ok(PrintModeResult { exit_code })
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Test double for PrintModeSession
    // -------------------------------------------------------------------------

    struct MockSession {
        messages_received: Vec<String>,
        response: Option<AgentMessage>,
        should_error: bool,
    }

    impl MockSession {
        fn new_ok(text: &str) -> Self {
            MockSession {
                messages_received: Vec::new(),
                response: Some(AgentMessage {
                    role: "assistant".to_string(),
                    content: vec![MessageContent::Text {
                        text: text.to_string(),
                    }],
                    stop_reason: Some("stop".to_string()),
                    error_message: None,
                }),
                should_error: false,
            }
        }

        fn new_error() -> Self {
            MockSession {
                messages_received: Vec::new(),
                response: Some(AgentMessage {
                    role: "assistant".to_string(),
                    content: vec![],
                    stop_reason: Some("error".to_string()),
                    error_message: Some("Something went wrong".to_string()),
                }),
                should_error: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl PrintModeSession for MockSession {
        async fn prompt(&mut self, message: &str) -> anyhow::Result<()> {
            self.messages_received.push(message.to_string());
            if self.should_error {
                return Err(anyhow::anyhow!("Mock session error"));
            }
            Ok(())
        }

        fn last_assistant_message(&self) -> Option<AgentMessage> {
            self.response.clone()
        }

        fn event_to_json(&self) -> Option<String> {
            Some(r#"{"type":"done"}"#.to_string())
        }
    }

    #[tokio::test]
    async fn text_mode_sends_initial_message() {
        let mut session = MockSession::new_ok("Hello!");
        let result = run_print_mode(
            &mut session,
            PrintModeOptions {
                mode: Mode::Text,
                initial_message: Some("Hi".to_string()),
                messages: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(session.messages_received, vec!["Hi"]);
    }

    #[tokio::test]
    async fn text_mode_sends_all_messages() {
        let mut session = MockSession::new_ok("Response");
        let result = run_print_mode(
            &mut session,
            PrintModeOptions {
                mode: Mode::Text,
                initial_message: Some("first".to_string()),
                messages: vec!["second".to_string(), "third".to_string()],
            },
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(session.messages_received, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn error_stop_reason_returns_exit_code_1() {
        let mut session = MockSession::new_error();
        let result = run_print_mode(
            &mut session,
            PrintModeOptions {
                mode: Mode::Text,
                initial_message: Some("prompt".to_string()),
                messages: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn no_initial_message_no_prompt_sent() {
        let mut session = MockSession::new_ok("Response");
        run_print_mode(
            &mut session,
            PrintModeOptions {
                mode: Mode::Text,
                initial_message: None,
                messages: vec![],
            },
        )
        .await
        .unwrap();
        assert!(session.messages_received.is_empty());
    }

    #[tokio::test]
    async fn json_mode_runs_without_error() {
        let mut session = MockSession::new_ok("Response");
        let result = run_print_mode(
            &mut session,
            PrintModeOptions {
                mode: Mode::Json,
                initial_message: Some("test".to_string()),
                messages: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
    }
}
