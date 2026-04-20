//! RPC mode: Headless operation with JSON stdin/stdout protocol.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/rpc/rpc-mode.ts`.
//!
//! Protocol:
//! - Commands: JSON objects with `type` field, optional `id` for correlation
//! - Responses: JSON objects with `type: "response"`, `command`, `success`, and optional `data`/`error`
//! - Events: session events streamed as they occur
//! - Extension UI: Extension UI requests are emitted, client responds with `extension_ui_response`

pub mod client;
pub mod jsonl;
pub mod types;

use std::io::{self, BufReader};

use serde_json::Value;

use self::jsonl::read_jsonl_lines;
use self::types::RpcResponse;

// Re-export commonly used types
pub use types::{
    QueueMode, RpcSessionState, RpcSlashCommand, SlashCommandSource, StreamingBehavior,
    ThinkingLevel,
};

// ============================================================================
// Session trait required by RPC mode
// ============================================================================

/// Session interface required by RPC mode.
///
/// The TypeScript original calls methods on `AgentSession` directly.
/// In Rust we expose a trait so the RPC runner can be tested with a mock.
pub trait RpcSession: Send {
    /// Send a prompt to the agent (fire and forget — events follow).
    fn prompt(&mut self, message: &str, source: &str) -> anyhow::Result<()>;

    /// Steer the agent mid-run.
    fn steer(&mut self, message: &str) -> anyhow::Result<()>;

    /// Queue a follow-up message.
    fn follow_up(&mut self, message: &str) -> anyhow::Result<()>;

    /// Abort the current operation.
    fn abort(&mut self) -> anyhow::Result<()>;

    /// Create a new session.
    fn new_session(&mut self, parent_session: Option<&str>) -> anyhow::Result<bool>;

    /// Get the current session state as JSON.
    fn get_state(&self) -> Value;

    /// Execute a bash command.
    fn execute_bash(&mut self, command: &str) -> anyhow::Result<Value>;

    /// Abort running bash command.
    fn abort_bash(&mut self);

    /// Get session statistics.
    fn get_session_stats(&self) -> Value;

    /// Export session to HTML.
    fn export_to_html(&mut self, output_path: Option<&str>) -> anyhow::Result<String>;

    /// Switch to a different session file.
    fn switch_session(&mut self, session_path: &str) -> anyhow::Result<bool>;

    /// Fork from a specific message.
    fn fork(&mut self, entry_id: &str) -> anyhow::Result<Value>;

    /// Get messages available for forking.
    fn get_fork_messages(&self) -> Value;

    /// Get text of last assistant message.
    fn get_last_assistant_text(&self) -> Option<String>;

    /// Set the session display name.
    fn set_session_name(&mut self, name: &str) -> anyhow::Result<()>;

    /// Get all messages.
    fn get_messages(&self) -> Value;

    /// Get available commands.
    fn get_commands(&self) -> Value;

    /// Compact session context.
    fn compact(&mut self, custom_instructions: Option<&str>) -> anyhow::Result<Value>;

    /// Set auto-compaction enabled.
    fn set_auto_compaction_enabled(&mut self, enabled: bool);

    /// Set auto-retry enabled.
    fn set_auto_retry_enabled(&mut self, enabled: bool);

    /// Abort in-progress retry.
    fn abort_retry(&mut self);

    /// Set thinking level.
    fn set_thinking_level(&mut self, level: &str);

    /// Cycle thinking level.
    fn cycle_thinking_level(&mut self) -> Option<String>;

    /// Set model by provider and ID.
    fn set_model(&mut self, provider: &str, model_id: &str) -> anyhow::Result<Value>;

    /// Cycle to next model.
    fn cycle_model(&mut self) -> anyhow::Result<Value>;

    /// Get available models.
    fn get_available_models(&self) -> Value;

    /// Set steering mode.
    fn set_steering_mode(&mut self, mode: &str);

    /// Set follow-up mode.
    fn set_follow_up_mode(&mut self, mode: &str);
}

// ============================================================================
// RPC mode runner
// ============================================================================

/// Write a JSON value as a JSONL line to stdout.
fn output_json(value: &Value) {
    if let Ok(line) = jsonl::serialize_json_line(value) {
        use std::io::Write;
        let stdout = io::stdout();
        let mut out = stdout.lock();
        out.write_all(line.as_bytes()).ok();
        out.flush().ok();
    }
}

/// Run in RPC mode.
///
/// Reads JSON-newline commands from stdin, writes JSON-newline responses/events
/// to stdout. Mirrors `runRpcMode()` in TypeScript.
pub fn run_rpc_mode<S: RpcSession>(session: &mut S) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let reader = BufReader::new(stdin.lock());

    // Collect all lines first (like the original sync approach)
    let mut lines_buf = Vec::new();
    read_jsonl_lines(reader, |line| {
        lines_buf.push(line);
    })?;

    for line in lines_buf {
        if line.trim().is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp =
                    RpcResponse::err(None, "parse", &format!("Failed to parse command: {e}"));
                output_json(&serde_json::to_value(resp).unwrap_or(Value::Null));
                continue;
            }
        };

        // Check for extension UI responses
        let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type == "extension_ui_response" {
            // Extension UI responses are handled by the extension bridge (not this loop)
            continue;
        }

        let id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let command_type = parsed
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let response = handle_command(session, &command_type, &parsed, id);
        output_json(&response);
    }

    Ok(())
}

/// Dispatch a single RPC command to the session and return the response JSON.
fn handle_command<S: RpcSession>(
    session: &mut S,
    command_type: &str,
    cmd: &Value,
    id: Option<String>,
) -> Value {
    macro_rules! ok {
        () => {
            serde_json::to_value(RpcResponse::ok(id.clone(), command_type, None)).unwrap()
        };
        ($data:expr) => {
            serde_json::to_value(RpcResponse::ok(
                id.clone(),
                command_type,
                Some(serde_json::to_value($data).unwrap_or(Value::Null)),
            ))
            .unwrap()
        };
    }

    macro_rules! err {
        ($msg:expr) => {
            serde_json::to_value(RpcResponse::err(id.clone(), command_type, $msg)).unwrap()
        };
    }

    match command_type {
        "prompt" => {
            let message = cmd.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let source = cmd.get("source").and_then(|v| v.as_str()).unwrap_or("rpc");
            match session.prompt(message, source) {
                Ok(()) => ok!(),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "steer" => {
            let message = cmd.get("message").and_then(|v| v.as_str()).unwrap_or("");
            match session.steer(message) {
                Ok(()) => ok!(),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "follow_up" => {
            let message = cmd.get("message").and_then(|v| v.as_str()).unwrap_or("");
            match session.follow_up(message) {
                Ok(()) => ok!(),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "abort" => match session.abort() {
            Ok(()) => ok!(),
            Err(e) => err!(e.to_string().as_str()),
        },

        "new_session" => {
            let parent = cmd.get("parentSession").and_then(|v| v.as_str());
            match session.new_session(parent) {
                Ok(cancelled) => ok!(serde_json::json!({ "cancelled": !cancelled })),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "get_state" => ok!(session.get_state()),

        "set_model" => {
            let provider = cmd.get("provider").and_then(|v| v.as_str()).unwrap_or("");
            let model_id = cmd.get("modelId").and_then(|v| v.as_str()).unwrap_or("");
            match session.set_model(provider, model_id) {
                Ok(data) => ok!(data),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "cycle_model" => match session.cycle_model() {
            Ok(data) => ok!(data),
            Err(e) => err!(e.to_string().as_str()),
        },

        "get_available_models" => ok!(session.get_available_models()),

        "set_thinking_level" => {
            let level = cmd
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("medium");
            session.set_thinking_level(level);
            ok!()
        }

        "cycle_thinking_level" => {
            if let Some(level) = session.cycle_thinking_level() {
                ok!(serde_json::json!({ "level": level }))
            } else {
                ok!(Value::Null)
            }
        }

        "set_steering_mode" => {
            let mode = cmd.get("mode").and_then(|v| v.as_str()).unwrap_or("all");
            session.set_steering_mode(mode);
            ok!()
        }

        "set_follow_up_mode" => {
            let mode = cmd.get("mode").and_then(|v| v.as_str()).unwrap_or("all");
            session.set_follow_up_mode(mode);
            ok!()
        }

        "compact" => {
            let instructions = cmd.get("customInstructions").and_then(|v| v.as_str());
            match session.compact(instructions) {
                Ok(data) => ok!(data),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "set_auto_compaction" => {
            let enabled = cmd
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            session.set_auto_compaction_enabled(enabled);
            ok!()
        }

        "set_auto_retry" => {
            let enabled = cmd
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            session.set_auto_retry_enabled(enabled);
            ok!()
        }

        "abort_retry" => {
            session.abort_retry();
            ok!()
        }

        "bash" => {
            let command = cmd.get("command").and_then(|v| v.as_str()).unwrap_or("");
            match session.execute_bash(command) {
                Ok(data) => ok!(data),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "abort_bash" => {
            session.abort_bash();
            ok!()
        }

        "get_session_stats" => ok!(session.get_session_stats()),

        "export_html" => {
            let output_path = cmd.get("outputPath").and_then(|v| v.as_str());
            match session.export_to_html(output_path) {
                Ok(path) => ok!(serde_json::json!({ "path": path })),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "switch_session" => {
            let session_path = cmd
                .get("sessionPath")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match session.switch_session(session_path) {
                Ok(success) => ok!(serde_json::json!({ "cancelled": !success })),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "fork" => {
            let entry_id = cmd.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
            match session.fork(entry_id) {
                Ok(data) => ok!(data),
                Err(e) => err!(e.to_string().as_str()),
            }
        }

        "get_fork_messages" => ok!(session.get_fork_messages()),

        "get_last_assistant_text" => {
            let text = session.get_last_assistant_text();
            ok!(serde_json::json!({ "text": text }))
        }

        "set_session_name" => {
            let name = cmd
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                err!("Session name cannot be empty")
            } else {
                match session.set_session_name(&name) {
                    Ok(()) => ok!(),
                    Err(e) => err!(e.to_string().as_str()),
                }
            }
        }

        "get_messages" => ok!(session.get_messages()),

        "get_commands" => ok!(session.get_commands()),

        unknown => serde_json::to_value(RpcResponse::err(
            id,
            unknown,
            &format!("Unknown command: {unknown}"),
        ))
        .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rpc_response_ok_no_data() {
        let resp = RpcResponse::ok(Some("1".to_string()), "prompt", None);
        assert_eq!(resp.id.as_deref(), Some("1"));
        assert!(resp.success);
        assert!(resp.data.is_none());
        assert!(resp.error.is_none());
    }

    #[test]
    fn rpc_response_err() {
        let resp = RpcResponse::err(None, "cancel", "Not running");
        assert!(!resp.success);
        assert_eq!(resp.error.as_deref(), Some("Not running"));
    }

    #[test]
    fn rpc_response_serialization_has_no_embedded_newline() {
        let resp = RpcResponse::ok(None, "test", None);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains('\n'));
    }
}
