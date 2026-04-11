use crate::error::SandboxError;

/// Handle to a running sandbox VM.
///
/// Provides exec and fs operations against the guest agent.
#[allow(dead_code)]
pub struct SandboxHandle {
    name: String,
    // TODO: relay channel, VM process handle, etc.
}

pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[allow(dead_code)]
impl SandboxHandle {
    /// Execute a command inside the sandbox.
    pub async fn exec(
        &self,
        command: &str,
        _args: &[&str],
        _timeout_secs: u32,
    ) -> Result<ExecOutput, SandboxError> {
        tracing::debug!(sandbox = %self.name, command, "exec");
        todo!("send ExecRequest to guest agent via relay")
    }

    /// Execute a shell command inside the sandbox.
    pub async fn shell(&self, command: &str, timeout_secs: u32) -> Result<ExecOutput, SandboxError> {
        self.exec("sh", &["-c", command], timeout_secs).await
    }

    /// Read a file from the sandbox filesystem.
    pub async fn fs_read(&self, path: &str) -> Result<Vec<u8>, SandboxError> {
        tracing::debug!(sandbox = %self.name, path, "fs_read");
        todo!("send FsRead to guest agent via relay")
    }

    /// Write a file to the sandbox filesystem.
    pub async fn fs_write(&self, path: &str, _data: &[u8]) -> Result<(), SandboxError> {
        tracing::debug!(sandbox = %self.name, path, "fs_write");
        todo!("send FsWrite to guest agent via relay")
    }

    /// List directory contents in the sandbox.
    pub async fn fs_list(&self, path: &str) -> Result<Vec<agent_protocol::FsEntry>, SandboxError> {
        tracing::debug!(sandbox = %self.name, path, "fs_list");
        todo!("send FsList to guest agent via relay")
    }

    /// Stop the sandbox VM.
    pub async fn stop(self) -> Result<(), SandboxError> {
        tracing::info!(sandbox = %self.name, "stopping sandbox");
        todo!("send Shutdown to guest agent, then kill VM process")
    }
}
