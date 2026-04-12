use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::SandboxError;
use crate::handle::SandboxHandle;
use crate::relay::AgentRelay;

/// Configuration for a sandbox VM instance.
pub struct SandboxBuilder {
    name: String,
    cpus: u32,
    memory_mib: u32,
    volumes: Vec<VolumeMount>,
    env: Vec<(String, String)>,
    idle_timeout_secs: u64,
    /// Path to the guest-agent binary (cross-compiled Linux aarch64 musl).
    guest_agent_path: PathBuf,
    /// Path to libkrunfw library.
    krunfw_path: PathBuf,
    /// Path to the sandbox-runtime binary.
    runtime_binary_path: PathBuf,
}

pub struct VolumeMount {
    pub host_path: String,
    pub guest_path: String,
    pub read_only: bool,
}

impl SandboxBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();

        Self {
            name: name.into(),
            cpus: 1,
            memory_mib: 256,
            volumes: Vec::new(),
            env: Vec::new(),
            idle_timeout_secs: 300,
            guest_agent_path: PathBuf::from("target/aarch64-unknown-linux-musl/release/agent-guest"),
            krunfw_path: PathBuf::from(format!("{home}/.microsandbox/lib/libkrunfw.5.dylib")),
            runtime_binary_path: PathBuf::from("target/debug/sandbox-runtime"),
        }
    }

    pub fn cpus(mut self, cpus: u32) -> Self {
        self.cpus = cpus;
        self
    }

    pub fn memory_mib(mut self, mib: u32) -> Self {
        self.memory_mib = mib;
        self
    }

    pub fn mount(
        mut self,
        host: impl Into<String>,
        guest: impl Into<String>,
        read_only: bool,
    ) -> Self {
        self.volumes.push(VolumeMount {
            host_path: host.into(),
            guest_path: guest.into(),
            read_only,
        });
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn idle_timeout_secs(mut self, secs: u64) -> Self {
        self.idle_timeout_secs = secs;
        self
    }

    pub fn guest_agent_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.guest_agent_path = path.into();
        self
    }

    pub fn krunfw_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.krunfw_path = path.into();
        self
    }

    pub fn runtime_binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.runtime_binary_path = path.into();
        self
    }

    /// Create and start the sandbox VM.
    ///
    /// 1. Prepare a minimal rootfs directory
    /// 2. Spawn the sandbox-runtime child process
    /// 3. Wait for the guest agent to signal readiness
    /// 4. Return a SandboxHandle for exec/fs operations
    pub async fn create(self) -> Result<SandboxHandle, SandboxError> {
        tracing::info!(name = %self.name, cpus = self.cpus, memory_mib = self.memory_mib, "creating sandbox");

        // 1. Prepare rootfs
        let rootfs_path = self.prepare_rootfs().await?;
        tracing::debug!(rootfs = %rootfs_path.display(), "rootfs prepared");

        // 2. Spawn runtime process
        //    stdin/stdout are piped — the runtime maps them to virtio-console
        //    The guest agent reads/writes /dev/vport0p0 which connects to these pipes
        let mut child = tokio::process::Command::new(&self.runtime_binary_path)
            .env("SANDBOX_ROOTFS", rootfs_path.to_str().unwrap_or_default())
            .env("SANDBOX_KRUNFW", self.krunfw_path.to_str().unwrap_or_default())
            .env("SANDBOX_VCPUS", self.cpus.to_string())
            .env("SANDBOX_MEMORY_MIB", self.memory_mib.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| SandboxError::VmCreate(format!("spawn runtime: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SandboxError::VmCreate("failed to capture child stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SandboxError::VmCreate("failed to capture child stdout".into()))?;

        tracing::debug!("runtime process spawned, creating relay");

        // 3. Create relay (stdout = reader, stdin = writer)
        let relay = Arc::new(AgentRelay::new(stdout, stdin));

        // 4. Wait for guest agent Ready
        relay.wait_ready(30).await?;

        Ok(SandboxHandle::new(self.name, relay, child))
    }

    /// Prepare a minimal rootfs directory for the VM.
    ///
    /// Creates: /init (guest-agent), /bin/busybox + symlinks, /proc, /sys, /dev, /tmp
    async fn prepare_rootfs(&self) -> Result<PathBuf, SandboxError> {
        let rootfs = std::env::temp_dir().join(format!("agent-sandbox-{}", uuid_v4()));

        // Create directory structure
        for dir in &["bin", "proc", "sys", "dev", "tmp"] {
            tokio::fs::create_dir_all(rootfs.join(dir))
                .await
                .map_err(|e| SandboxError::VmCreate(format!("mkdir {dir}: {e}")))?;
        }

        // Copy guest-agent as /init
        if self.guest_agent_path.exists() {
            tokio::fs::copy(&self.guest_agent_path, rootfs.join("init"))
                .await
                .map_err(|e| {
                    SandboxError::VmCreate(format!(
                        "copy guest-agent to /init: {e} (from {:?})",
                        self.guest_agent_path
                    ))
                })?;
            // Make executable
            set_executable(&rootfs.join("init")).await?;
        } else {
            tracing::warn!(
                path = %self.guest_agent_path.display(),
                "guest-agent binary not found — rootfs will be incomplete"
            );
        }

        // Copy busybox if available
        let busybox_path = find_busybox();
        if let Some(bb) = busybox_path {
            let bb_dest = rootfs.join("bin/busybox");
            tokio::fs::copy(&bb, &bb_dest)
                .await
                .map_err(|e| SandboxError::VmCreate(format!("copy busybox: {e}")))?;
            set_executable(&bb_dest).await?;

            // Create symlinks for common commands
            for cmd in &["sh", "echo", "cat", "ls", "mkdir", "rm", "cp", "mv"] {
                let link = rootfs.join(format!("bin/{cmd}"));
                let _ = tokio::fs::symlink("busybox", &link).await;
            }
        } else {
            tracing::warn!("busybox not found — sandbox shell commands may not work");
        }

        Ok(rootfs)
    }
}

/// Generate a simple UUID v4 for temp directory names.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{t:x}-{pid:x}")
}

/// Set a file as executable (chmod +x).
async fn set_executable(path: &Path) -> Result<(), SandboxError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| SandboxError::VmCreate(format!("stat {:?}: {e}", path)))?;
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        tokio::fs::set_permissions(path, perms)
            .await
            .map_err(|e| SandboxError::VmCreate(format!("chmod {:?}: {e}", path)))?;
    }
    Ok(())
}

/// Try to find a static busybox binary.
fn find_busybox() -> Option<PathBuf> {
    let candidates = [
        // Project-local
        "vendor/busybox",
        // System
        "/usr/bin/busybox",
        "/usr/local/bin/busybox",
    ];
    for p in candidates {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    None
}
