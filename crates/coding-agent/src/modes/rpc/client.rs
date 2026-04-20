//! RPC Client for programmatic access to the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/rpc/rpc-client.ts`.
//!
//! Spawns the agent in RPC mode and provides a typed API for all operations.

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::jsonl::{read_jsonl_lines, serialize_json_line};
use super::types::{RpcResponse, RpcSessionState, RpcSlashCommand};

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct RpcClientOptions {
    /// Path to the CLI entry point. Default: `dist/cli.js`
    pub cli_path: Option<String>,
    /// Working directory for the agent.
    pub cwd: Option<String>,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Provider to use.
    pub provider: Option<String>,
    /// Model ID to use.
    pub model: Option<String>,
    /// Additional CLI arguments.
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub provider: String,
    pub id: String,
    pub context_window: u64,
    pub reasoning: bool,
}

pub type AgentEventListener = Box<dyn Fn(&Value) + Send + Sync>;

// ============================================================================
// Pending request
// ============================================================================

type PendingResolve = Box<dyn FnOnce(RpcResponse) + Send>;
type PendingReject = Box<dyn FnOnce(String) + Send>;

struct PendingRequest {
    resolve: Option<PendingResolve>,
    reject: Option<PendingReject>,
}

// ============================================================================
// RPC Client
// ============================================================================

/// A client that spawns the agent process in RPC mode and communicates via
/// JSON lines on stdin/stdout.
pub struct RpcClient {
    options: RpcClientOptions,
    process: Option<Child>,
    stdin: Option<ChildStdin>,
    event_listeners: Arc<Mutex<Vec<AgentEventListener>>>,
    pending_requests: Arc<Mutex<HashMap<String, PendingRequest>>>,
    request_id: AtomicU64,
    stderr: Arc<Mutex<String>>,
}

impl RpcClient {
    pub fn new(options: RpcClientOptions) -> Self {
        Self {
            options,
            process: None,
            stdin: None,
            event_listeners: Arc::new(Mutex::new(Vec::new())),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            request_id: AtomicU64::new(0),
            stderr: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Start the RPC agent process.
    pub fn start(&mut self) -> anyhow::Result<()> {
        if self.process.is_some() {
            anyhow::bail!("Client already started");
        }

        let cli_path = self
            .options
            .cli_path
            .clone()
            .unwrap_or_else(|| "dist/cli.js".to_string());

        let mut args = vec!["--mode".to_string(), "rpc".to_string()];
        if let Some(ref provider) = self.options.provider {
            args.push("--provider".to_string());
            args.push(provider.clone());
        }
        if let Some(ref model) = self.options.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        args.extend(self.options.args.clone());

        let mut cmd = Command::new("node");
        cmd.arg(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref cwd) = self.options.cwd {
            cmd.current_dir(cwd);
        }

        for (k, v) in &self.options.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr_handle = child.stderr.take().unwrap();

        let event_listeners = Arc::clone(&self.event_listeners);
        let pending_requests = Arc::clone(&self.pending_requests);

        // Stdout reader thread
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            read_jsonl_lines(reader, |line| {
                if let Ok(data) = serde_json::from_str::<Value>(&line) {
                    // Check if it's a response to a pending request
                    if data.get("type").and_then(|v| v.as_str()) == Some("response")
                        && let Some(id) = data.get("id").and_then(|v| v.as_str())
                    {
                        let mut reqs = pending_requests.lock().unwrap();
                        if let Some(mut pending) = reqs.remove(id) {
                            if let Ok(response) = serde_json::from_value::<RpcResponse>(data)
                                && let Some(resolve) = pending.resolve.take()
                            {
                                resolve(response);
                            }
                            return;
                        }
                    }

                    // Otherwise it's an event
                    let listeners = event_listeners.lock().unwrap();
                    for listener in listeners.iter() {
                        listener(&data);
                    }
                }
            })
            .ok();
        });

        // Stderr collector thread
        let stderr = Arc::clone(&self.stderr);
        thread::spawn(move || {
            use std::io::Read;
            let mut buf = String::new();
            let mut reader = std::io::BufReader::new(stderr_handle);
            reader.read_to_string(&mut buf).ok();
            *stderr.lock().unwrap() = buf;
        });

        // Give the process a moment to initialize (mirrors the 100ms delay in TS)
        std::thread::sleep(Duration::from_millis(100));

        // Check if process exited immediately
        if let Some(status) = child.try_wait()? {
            let err_msg = self.stderr.lock().unwrap().clone();
            anyhow::bail!(
                "Agent process exited immediately with status {:?}. Stderr: {}",
                status,
                err_msg
            );
        }

        self.stdin = Some(stdin);
        self.process = Some(child);
        Ok(())
    }

    /// Stop the RPC agent process.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(mut child) = self.process.take() {
            drop(self.stdin.take());
            child.kill().ok();
            child.wait().ok();
        }
        self.pending_requests.lock().unwrap().clear();
        Ok(())
    }

    /// Subscribe to agent events. Returns an unsubscribe closure.
    pub fn on_event<F: Fn(&Value) + Send + Sync + 'static>(&self, listener: F) -> usize {
        let mut listeners = self.event_listeners.lock().unwrap();
        let id = listeners.len();
        listeners.push(Box::new(listener));
        id
    }

    /// Get collected stderr output.
    pub fn get_stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    // =========================================================================
    // Command Methods (synchronous wrappers — use channels for blocking)
    // =========================================================================

    /// Send a prompt to the agent.
    pub fn prompt(&mut self, message: &str) -> anyhow::Result<()> {
        let cmd = serde_json::json!({
            "type": "prompt",
            "message": message,
        });
        self.send_raw(&cmd)?;
        Ok(())
    }

    /// Send a steer message.
    pub fn steer(&mut self, message: &str) -> anyhow::Result<()> {
        let cmd = serde_json::json!({"type": "steer", "message": message});
        self.send_raw(&cmd)?;
        Ok(())
    }

    /// Send a follow-up message.
    pub fn follow_up(&mut self, message: &str) -> anyhow::Result<()> {
        let cmd = serde_json::json!({"type": "follow_up", "message": message});
        self.send_raw(&cmd)?;
        Ok(())
    }

    /// Abort the current operation.
    pub fn abort(&mut self) -> anyhow::Result<()> {
        let cmd = serde_json::json!({"type": "abort"});
        self.send_raw(&cmd)?;
        Ok(())
    }

    /// Execute a bash command and return the result.
    pub fn bash(&mut self, command: &str) -> anyhow::Result<Value> {
        let response = self.send_blocking(serde_json::json!({
            "type": "bash",
            "command": command,
        }))?;
        self.get_data(response)
    }

    /// Get current session state.
    pub fn get_state(&mut self) -> anyhow::Result<RpcSessionState> {
        let response = self.send_blocking(serde_json::json!({"type": "get_state"}))?;
        let data = self.get_data(response)?;
        Ok(serde_json::from_value(data)?)
    }

    /// Get available commands.
    pub fn get_commands(&mut self) -> anyhow::Result<Vec<RpcSlashCommand>> {
        let response = self.send_blocking(serde_json::json!({"type": "get_commands"}))?;
        let data = self.get_data(response)?;
        Ok(serde_json::from_value(
            data.get("commands")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?)
    }

    // =========================================================================
    // Internal
    // =========================================================================

    fn send_raw(&mut self, cmd: &Value) -> anyhow::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Client not started"))?;
        let line = serialize_json_line(cmd)?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Send a command and block until the response arrives (via a channel).
    fn send_blocking(&mut self, mut cmd: Value) -> anyhow::Result<RpcResponse> {
        let id = format!("req_{}", self.request_id.fetch_add(1, Ordering::SeqCst));
        cmd["id"] = Value::String(id.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut reqs = self.pending_requests.lock().unwrap();
            reqs.insert(
                id,
                PendingRequest {
                    resolve: Some(Box::new(move |r| {
                        let _ = tx.send(r);
                    })),
                    reject: None,
                },
            );
        }

        self.send_raw(&cmd)?;

        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| {
            anyhow::anyhow!(
                "Timeout waiting for response to {}",
                cmd.get("type").unwrap_or(&Value::Null)
            )
        })
    }

    fn get_data(&self, response: RpcResponse) -> anyhow::Result<Value> {
        if !response.success {
            anyhow::bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string())
            );
        }
        Ok(response.data.unwrap_or(Value::Null))
    }
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.stop().ok();
    }
}
