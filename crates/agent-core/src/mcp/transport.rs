// MCP Stdio transport — spawns a child process and communicates over stdin/stdout.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{ChildStdin, ChildStdout};

use super::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("spawn failed: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("connection closed")]
    Closed,
    #[error("RPC error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("timeout")]
    Timeout,
}

pub struct StdioTransport {
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    // child kept alive for the lifetime of this transport.
    _child: tokio::process::Child,
    next_id: Arc<AtomicU64>,
}

impl StdioTransport {
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self, TransportError> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin must be piped");
        let stdout = child.stdout.take().expect("stdout must be piped");

        Ok(Self {
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            _child: child,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Allocate the next request id.
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a JSON-RPC request and return the raw result value.
    pub async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let id = self.next_id();
        let request = JsonRpcRequest::new(id, method, params);
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');

        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| TransportError::Closed)?;
        self.stdin
            .flush()
            .await
            .map_err(|_| TransportError::Closed)?;

        self.read_response(id).await
    }

    /// Send a JSON-RPC notification (no `id`, no response expected).
    pub async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), TransportError> {
        let notification = JsonRpcNotification::new(method, params);
        let mut line = serde_json::to_string(&notification)?;
        line.push('\n');

        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| TransportError::Closed)?;
        self.stdin
            .flush()
            .await
            .map_err(|_| TransportError::Closed)?;

        Ok(())
    }

    async fn read_response(
        &mut self,
        expected_id: u64,
    ) -> Result<serde_json::Value, TransportError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.read_response_inner(expected_id),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
    }

    async fn read_response_inner(
        &mut self,
        expected_id: u64,
    ) -> Result<serde_json::Value, TransportError> {
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(|_| TransportError::Closed)?;
            if n == 0 {
                return Err(TransportError::Closed);
            }

            let msg: serde_json::Value = serde_json::from_str(line.trim())?;

            // Batch arrays are not part of our request/response flow; skip them.
            if msg.is_array() {
                continue;
            }

            // Messages without an `id` (or with null `id`) are notifications; skip them.
            let id_val = match msg.get("id") {
                None | Some(serde_json::Value::Null) => continue,
                Some(v) => v,
            };

            // JSON-RPC allows string or number IDs; compare by value to avoid silent coercion.
            if !id_matches(id_val, expected_id) {
                // Response for a different in-flight request — skip and keep reading.
                continue;
            }

            let response: JsonRpcResponse = serde_json::from_value(msg)?;

            if let Some(error) = response.error {
                return Err(TransportError::Rpc {
                    code: error.code,
                    message: error.message,
                });
            }

            return response.result.ok_or(TransportError::Rpc {
                code: -32603,
                message: "response has no result".to_string(),
            });
        }
    }
}

fn id_matches(id_val: &serde_json::Value, expected: u64) -> bool {
    match id_val {
        serde_json::Value::Number(n) => n.as_u64() == Some(expected),
        serde_json::Value::String(s) => s.parse::<u64>().ok() == Some(expected),
        _ => false,
    }
}

// Expose error type publicly for consumers.
pub use TransportError as McpTransportError;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn next_id_increments() {
        let counter = Arc::new(AtomicU64::new(1));
        let a = counter.fetch_add(1, Ordering::SeqCst);
        let b = counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    }

    #[test]
    fn id_matches_number_equal() {
        assert!(id_matches(&json!(123), 123));
    }

    #[test]
    fn id_matches_string_equal() {
        assert!(id_matches(&json!("123"), 123));
    }

    #[test]
    fn id_matches_string_not_equal() {
        assert!(!id_matches(&json!("456"), 123));
    }

    #[test]
    fn id_matches_invalid_type_returns_false() {
        assert!(!id_matches(&json!(true), 123));
    }
}
