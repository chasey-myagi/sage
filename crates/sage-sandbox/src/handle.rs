use std::sync::Arc;

use sage_protocol::{
    ExecRequest, FsEntry, FsListRequest, FsReadRequest, FsWriteRequest, GuestMessage, HostMessage,
};

use crate::error::SandboxError;
use crate::relay::AgentRelay;

/// Handle to a running sandbox VM.
///
/// Provides exec and fs operations against the guest agent via the relay.
/// The `child` field is wrapped in a `Mutex` so that `SandboxHandle` is
/// `Send + Sync`, allowing it to be shared across async tool executions.
pub struct SandboxHandle {
    name: String,
    relay: Arc<AgentRelay>,
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
}

pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl SandboxHandle {
    pub(crate) fn new(name: String, relay: Arc<AgentRelay>, child: tokio::process::Child) -> Self {
        Self {
            name,
            relay,
            child: tokio::sync::Mutex::new(Some(child)),
        }
    }

    /// Execute a command inside the sandbox.
    pub async fn exec(
        &self,
        command: &str,
        args: &[&str],
        timeout_secs: u32,
    ) -> Result<ExecOutput, SandboxError> {
        let request_id = self.relay.next_request_id();
        tracing::debug!(sandbox = %self.name, request_id, command, "exec");

        let msg = HostMessage::ExecRequest(ExecRequest {
            request_id,
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
            cwd: "/".to_string(),
            timeout_secs,
        });

        let response = self.relay.request(msg).await?;

        match response {
            GuestMessage::ExecExited {
                exit_code,
                stdout,
                stderr,
                ..
            } => Ok(ExecOutput {
                exit_code,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            }),
            GuestMessage::Error { message, .. } => Err(SandboxError::ExecFailed(message)),
            other => Err(SandboxError::ExecFailed(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Execute a shell command inside the sandbox.
    pub async fn shell(
        &self,
        command: &str,
        timeout_secs: u32,
    ) -> Result<ExecOutput, SandboxError> {
        self.exec("sh", &["-c", command], timeout_secs).await
    }

    /// Read a file from the sandbox filesystem.
    pub async fn fs_read(&self, path: &str) -> Result<Vec<u8>, SandboxError> {
        let request_id = self.relay.next_request_id();
        tracing::debug!(sandbox = %self.name, request_id, path, "fs_read");

        let msg = HostMessage::FsRead(FsReadRequest {
            request_id,
            path: path.to_string(),
        });

        let response = self.relay.request(msg).await?;

        match response {
            GuestMessage::FsData { data, .. } => Ok(data),
            GuestMessage::Error { message, .. } => Err(SandboxError::FsError(message)),
            other => Err(SandboxError::FsError(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Write a file to the sandbox filesystem.
    pub async fn fs_write(&self, path: &str, data: &[u8]) -> Result<(), SandboxError> {
        let request_id = self.relay.next_request_id();
        tracing::debug!(sandbox = %self.name, request_id, path, "fs_write");

        let msg = HostMessage::FsWrite(FsWriteRequest {
            request_id,
            path: path.to_string(),
            data: data.to_vec(),
        });

        let response = self.relay.request(msg).await?;

        match response {
            GuestMessage::FsResult { success, error, .. } => {
                if success {
                    Ok(())
                } else {
                    Err(SandboxError::FsError(error))
                }
            }
            GuestMessage::Error { message, .. } => Err(SandboxError::FsError(message)),
            other => Err(SandboxError::FsError(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// List directory contents in the sandbox.
    pub async fn fs_list(&self, path: &str) -> Result<Vec<FsEntry>, SandboxError> {
        let request_id = self.relay.next_request_id();
        tracing::debug!(sandbox = %self.name, request_id, path, "fs_list");

        let msg = HostMessage::FsList(FsListRequest {
            request_id,
            path: path.to_string(),
        });

        let response = self.relay.request(msg).await?;

        match response {
            GuestMessage::FsEntries { entries, .. } => Ok(entries),
            GuestMessage::Error { message, .. } => Err(SandboxError::FsError(message)),
            other => Err(SandboxError::FsError(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Stop the sandbox VM.
    ///
    /// Takes `&self` (not `self`) so the handle can be shared via `Arc`.
    /// Subsequent calls are no-ops — the child is taken on the first call.
    pub async fn stop(&self) -> Result<(), SandboxError> {
        tracing::info!(sandbox = %self.name, "stopping sandbox");

        // Best-effort send shutdown with timeout — guest may already be gone.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.relay.request(HostMessage::Shutdown),
        )
        .await;

        // Take and kill the runtime child process
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        tracing::info!(sandbox = %self.name, "sandbox stopped");
        Ok(())
    }
}

impl Drop for SandboxHandle {
    fn drop(&mut self) {
        // Best-effort kill on drop — prevents orphan VM processes when the
        // handle is dropped without an explicit stop() call (e.g. on panic).
        // get_mut() is safe in Drop because we have &mut self (no contention).
        if let Some(mut child) = self.child.get_mut().take() {
            let _ = child.start_kill();
        }
    }
}
