use std::sync::Arc;

use sage_protocol::{
    ExecRequest, FsEntry, FsListRequest, FsReadRequest, FsWriteRequest, GuestMessage, HostMessage,
};

use crate::error::SandboxError;
use crate::relay::AgentRelay;

/// Handle to a running sandbox VM.
///
/// Provides exec and fs operations against the guest agent via the relay.
pub struct SandboxHandle {
    name: String,
    relay: Arc<AgentRelay>,
    child: tokio::process::Child,
}

pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl SandboxHandle {
    pub(crate) fn new(name: String, relay: Arc<AgentRelay>, child: tokio::process::Child) -> Self {
        Self { name, relay, child }
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
    pub async fn stop(mut self) -> Result<(), SandboxError> {
        tracing::info!(sandbox = %self.name, "stopping sandbox");

        // Best-effort send shutdown — guest may already be gone.
        // Shutdown triggers guest process exit, so we don't wait for a response.
        let _ = self.relay.request(HostMessage::Shutdown).await;

        // Kill the runtime child process if still running
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;

        tracing::info!(sandbox = %self.name, "sandbox stopped");
        Ok(())
    }
}
