use serde::{Deserialize, Serialize};

/// Host → Guest messages
#[derive(Debug, Serialize, Deserialize)]
pub enum HostMessage {
    ExecRequest(ExecRequest),
    FsRead(FsReadRequest),
    FsWrite(FsWriteRequest),
    FsList(FsListRequest),
    Shutdown,
}

/// Guest → Host messages
#[derive(Debug, Serialize, Deserialize)]
pub enum GuestMessage {
    Ready,
    ExecStarted { request_id: u64, pid: u32 },
    ExecStdout { request_id: u64, data: Vec<u8> },
    ExecStderr { request_id: u64, data: Vec<u8> },
    ExecExited { request_id: u64, exit_code: i32 },
    FsData { request_id: u64, data: Vec<u8> },
    FsResult { request_id: u64, success: bool, error: String },
    FsEntries { request_id: u64, entries: Vec<FsEntry> },
    Error { request_id: u64, message: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRequest {
    pub request_id: u64,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: String,
    pub timeout_secs: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsReadRequest {
    pub request_id: u64,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsWriteRequest {
    pub request_id: u64,
    pub path: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsListRequest {
    pub request_id: u64,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}
