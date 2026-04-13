use serde::{Deserialize, Serialize};

/// Host → Guest messages
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostMessage {
    ExecRequest(ExecRequest),
    FsRead(FsReadRequest),
    FsWrite(FsWriteRequest),
    FsList(FsListRequest),
    Shutdown,
}

/// Guest → Host messages
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuestMessage {
    Ready,
    ExecStarted {
        request_id: u64,
        pid: u32,
    },
    ExecStdout {
        request_id: u64,
        data: Vec<u8>,
    },
    ExecStderr {
        request_id: u64,
        data: Vec<u8>,
    },
    ExecExited {
        request_id: u64,
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    FsData {
        request_id: u64,
        data: Vec<u8>,
    },
    FsResult {
        request_id: u64,
        success: bool,
        error: String,
    },
    FsEntries {
        request_id: u64,
        entries: Vec<FsEntry>,
    },
    Error {
        request_id: u64,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub request_id: u64,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: String,
    pub timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsReadRequest {
    pub request_id: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWriteRequest {
    pub request_id: u64,
    pub path: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListRequest {
    pub request_id: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Security configuration passed from host to guest via `SAGE_SECURITY` env var.
///
/// This struct is the single source of truth for security settings shared
/// between the sandbox builder (host) and the guest agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestSecurityConfig {
    /// Enable seccomp-bpf syscall filter (Linux only).
    #[serde(default = "default_true")]
    pub seccomp: bool,
    /// Enable Landlock LSM filesystem access control (Linux only).
    #[serde(default = "default_true")]
    pub landlock: bool,
    /// Maximum file size for write operations (MiB).
    #[serde(default = "default_max_file_size_mb")]
    pub max_file_size_mb: u32,
    /// Maximum number of open file descriptors.
    #[serde(default = "default_max_open_files")]
    pub max_open_files: u32,
    /// tmpfs size limit for /tmp (MiB).
    #[serde(default = "default_tmpfs_size_mb")]
    pub tmpfs_size_mb: u32,
    /// Maximum number of processes (RLIMIT_NPROC). Prevents fork bombs.
    #[serde(default = "default_max_processes")]
    pub max_processes: u32,
    /// Paths the guest can read/write (Landlock allowlist).
    /// Default: ["/workspace", "/tmp"]
    #[serde(default = "default_allowed_paths")]
    pub allowed_paths: Vec<String>,
}

impl Default for GuestSecurityConfig {
    fn default() -> Self {
        Self {
            seccomp: default_true(),
            landlock: default_true(),
            max_file_size_mb: default_max_file_size_mb(),
            max_open_files: default_max_open_files(),
            tmpfs_size_mb: default_tmpfs_size_mb(),
            max_processes: default_max_processes(),
            allowed_paths: default_allowed_paths(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_max_file_size_mb() -> u32 {
    100
}
fn default_max_open_files() -> u32 {
    256
}
fn default_tmpfs_size_mb() -> u32 {
    512
}
fn default_max_processes() -> u32 {
    256
}
fn default_allowed_paths() -> Vec<String> {
    vec!["/workspace".into(), "/tmp".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── GuestSecurityConfig defaults ─────────────────────────────

    #[test]
    fn guest_security_config_default_values() {
        let config = GuestSecurityConfig::default();
        assert!(config.seccomp);
        assert!(config.landlock);
        assert_eq!(config.max_file_size_mb, 100);
        assert_eq!(config.max_open_files, 256);
        assert_eq!(config.tmpfs_size_mb, 512);
        assert_eq!(config.max_processes, 256);
        assert_eq!(config.allowed_paths, vec!["/workspace", "/tmp"]);
    }

    // ── JSON serialization ───────────────────────────────────────

    #[test]
    fn guest_security_config_json_roundtrip() {
        let config = GuestSecurityConfig {
            seccomp: false,
            landlock: true,
            max_file_size_mb: 200,
            max_open_files: 512,
            tmpfs_size_mb: 1024,
            max_processes: 128,
            allowed_paths: vec!["/workspace".into(), "/data".into()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: GuestSecurityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, decoded);
    }

    #[test]
    fn guest_security_config_partial_json_fills_defaults() {
        let json = r#"{"max_file_size_mb": 42}"#;
        let config: GuestSecurityConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.max_file_size_mb, 42);
        assert!(config.seccomp);
        assert!(config.landlock);
        assert_eq!(config.max_open_files, 256);
        assert_eq!(config.tmpfs_size_mb, 512);
        assert_eq!(config.allowed_paths, vec!["/workspace", "/tmp"]);
    }

    #[test]
    fn guest_security_config_empty_json_is_all_defaults() {
        let config: GuestSecurityConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, GuestSecurityConfig::default());
    }

    #[test]
    fn guest_security_config_empty_allowed_paths() {
        let json = r#"{"allowed_paths": []}"#;
        let config: GuestSecurityConfig = serde_json::from_str(json).unwrap();
        assert!(config.allowed_paths.is_empty());
    }

    #[test]
    fn guest_security_config_all_disabled() {
        let json = r#"{"seccomp": false, "landlock": false}"#;
        let config: GuestSecurityConfig = serde_json::from_str(json).unwrap();
        assert!(!config.seccomp);
        assert!(!config.landlock);
    }

    // ── CBOR roundtrip (used in wire protocol) ───────────────────

    #[test]
    fn guest_security_config_cbor_roundtrip() {
        let config = GuestSecurityConfig {
            seccomp: true,
            landlock: true,
            max_file_size_mb: 50,
            max_open_files: 128,
            tmpfs_size_mb: 256,
            max_processes: 64,
            allowed_paths: vec!["/workspace".into()],
        };
        let mut cbor_buf = Vec::new();
        ciborium::into_writer(&config, &mut cbor_buf).unwrap();
        let decoded: GuestSecurityConfig = ciborium::from_reader(&cbor_buf[..]).unwrap();
        assert_eq!(config, decoded);
    }
}
