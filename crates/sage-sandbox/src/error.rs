use thiserror::Error;

#[derive(Error, Debug)]
pub enum SandboxError {
    #[error("failed to create VM: {0}")]
    VmCreate(String),

    #[error("guest agent not ready after {0}s")]
    AgentTimeout(u64),

    #[error("command execution failed: {0}")]
    ExecFailed(String),

    #[error("command timed out after {0}s")]
    ExecTimeout(u32),

    #[error("file operation failed: {0}")]
    FsError(String),

    #[error("wire protocol error: {0}")]
    Protocol(#[from] sage_protocol::WireError),

    #[error("sandbox already stopped")]
    AlreadyStopped,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
