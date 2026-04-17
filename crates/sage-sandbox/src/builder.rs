use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::SandboxError;
use crate::handle::SandboxHandle;
use crate::relay::AgentRelay;

/// Commands available inside the guest rootfs via busybox symlinks.
///
/// **Current state**: Only busybox is installed. Tools like `cargo`, `git`,
/// `python3`, `rg`, `fd` are NOT available in the default rootfs.
/// YAML configs referencing these binaries will fail at execution time.
///
/// Future work: support custom rootfs images or a tool installer mechanism
/// to provide richer development environments inside the sandbox.
pub const BUSYBOX_COMMANDS: &[&str] = &["sh", "echo", "cat", "ls", "mkdir", "rm", "cp", "mv"];

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
    /// Security configuration passed to guest via SAGE_SECURITY env var.
    security_config: Option<sage_protocol::GuestSecurityConfig>,
}

/// A volume mount mapping a host directory to a guest mount point.
///
/// Serialized as JSON and passed to the sandbox-runtime process via the
/// `SANDBOX_VOLUMES` environment variable.  The runtime adds each entry
/// as a virtiofs share; the guest agent mounts the corresponding tag.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
            guest_agent_path: PathBuf::from("target/aarch64-unknown-linux-musl/release/sage-guest"),
            krunfw_path: PathBuf::from(format!("{home}/.microsandbox/lib/libkrunfw.5.dylib")),
            runtime_binary_path: PathBuf::from("target/debug/sandbox-runtime"),
            security_config: None,
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

    pub fn security(mut self, config: sage_protocol::GuestSecurityConfig) -> Self {
        self.security_config = Some(config);
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
        // Serialize volumes as JSON for the runtime process
        let volumes_json = serde_json::to_string(&self.volumes)
            .map_err(|e| SandboxError::VmCreate(format!("serialize volumes: {e}")))?;

        // Serialize security config for the guest agent
        let security_json = self
            .security_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| SandboxError::VmCreate(format!("serialize security config: {e}")))?;

        let mut cmd = tokio::process::Command::new(&self.runtime_binary_path);
        cmd.env("SANDBOX_ROOTFS", rootfs_path.to_str().unwrap_or_default())
            .env(
                "SANDBOX_KRUNFW",
                self.krunfw_path.to_str().unwrap_or_default(),
            )
            .env("SANDBOX_VCPUS", self.cpus.to_string())
            .env("SANDBOX_MEMORY_MIB", self.memory_mib.to_string())
            .env("SANDBOX_VOLUMES", &volumes_json)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        if let Some(ref sec_json) = security_json {
            cmd.env("SAGE_SECURITY", sec_json);
        }

        let mut child = cmd
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
        let rootfs = std::env::temp_dir().join(format!("agent-sandbox-{}", unique_suffix()));

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
            for cmd in BUSYBOX_COMMANDS {
                let link = rootfs.join(format!("bin/{cmd}"));
                let _ = tokio::fs::symlink("busybox", &link).await;
            }
        } else {
            tracing::warn!("busybox not found — sandbox shell commands may not work");
        }

        // Create mount point directories for volumes (with path traversal check)
        for vol in &self.volumes {
            let mount_point = rootfs.join(vol.guest_path.trim_start_matches('/'));
            tokio::fs::create_dir_all(&mount_point).await.map_err(|e| {
                SandboxError::VmCreate(format!("mkdir volume mount point {}: {e}", vol.guest_path))
            })?;
            // Verify mount point doesn't escape rootfs (e.g. via "../")
            let canonical = tokio::fs::canonicalize(&mount_point).await.map_err(|e| {
                SandboxError::VmCreate(format!("canonicalize mount point {}: {e}", vol.guest_path))
            })?;
            let rootfs_canonical = tokio::fs::canonicalize(&rootfs)
                .await
                .map_err(|e| SandboxError::VmCreate(format!("canonicalize rootfs: {e}")))?;
            if !canonical.starts_with(&rootfs_canonical) {
                return Err(SandboxError::VmCreate(format!(
                    "volume guest_path '{}' escapes rootfs (resolved to {:?})",
                    vol.guest_path, canonical
                )));
            }
        }

        Ok(rootfs)
    }
}

/// Generate a unique suffix for temp directory names (timestamp + pid).
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    // Include a per-process atomic counter so concurrent test threads (same PID,
    // same nanosecond) always get distinct paths.
    format!("{t:x}-{pid:x}-{n:x}")
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── VolumeMount serialization ──────────────────────────────────

    #[test]
    fn volume_mount_serializes_to_json() {
        let vol = VolumeMount {
            host_path: "/host/workspace".into(),
            guest_path: "/workspace".into(),
            read_only: false,
        };
        let json = serde_json::to_string(&vol).unwrap();
        assert!(json.contains("host_path"));
        assert!(json.contains("/host/workspace"));
        assert!(json.contains("read_only"));
    }

    #[test]
    fn volume_mount_roundtrip() {
        let original = VolumeMount {
            host_path: "/data/input".into(),
            guest_path: "/mnt/input".into(),
            read_only: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: VolumeMount = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.host_path, original.host_path);
        assert_eq!(decoded.guest_path, original.guest_path);
        assert_eq!(decoded.read_only, original.read_only);
    }

    #[test]
    fn volume_mount_vec_roundtrip() {
        let volumes = vec![
            VolumeMount {
                host_path: "/host/a".into(),
                guest_path: "/a".into(),
                read_only: false,
            },
            VolumeMount {
                host_path: "/host/b".into(),
                guest_path: "/b".into(),
                read_only: true,
            },
        ];
        let json = serde_json::to_string(&volumes).unwrap();
        let decoded: Vec<VolumeMount> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].host_path, "/host/a");
        assert_eq!(decoded[1].read_only, true);
    }

    #[test]
    fn empty_volumes_serialize_to_empty_array() {
        let volumes: Vec<VolumeMount> = vec![];
        let json = serde_json::to_string(&volumes).unwrap();
        assert_eq!(json, "[]");
    }

    // ── SandboxBuilder configuration ───────────────────────────────

    #[test]
    fn builder_default_has_empty_volumes() {
        let builder = SandboxBuilder::new("test");
        assert!(builder.volumes.is_empty());
    }

    #[test]
    fn builder_mount_adds_volume() {
        let builder = SandboxBuilder::new("test").mount("/host/ws", "/workspace", false);
        assert_eq!(builder.volumes.len(), 1);
        assert_eq!(builder.volumes[0].host_path, "/host/ws");
        assert_eq!(builder.volumes[0].guest_path, "/workspace");
        assert!(!builder.volumes[0].read_only);
    }

    #[test]
    fn builder_multiple_mounts() {
        let builder = SandboxBuilder::new("test")
            .mount("/host/a", "/a", false)
            .mount("/host/b", "/b", true);
        assert_eq!(builder.volumes.len(), 2);
    }

    #[test]
    fn builder_mount_read_only() {
        let builder = SandboxBuilder::new("test").mount("/host/data", "/data", true);
        assert!(builder.volumes[0].read_only);
    }

    // ── Security config builder ─────────────────────────────────────

    #[test]
    fn builder_default_has_no_security_config() {
        let builder = SandboxBuilder::new("test");
        assert!(builder.security_config.is_none());
    }

    #[test]
    fn builder_security_sets_config() {
        let config = sage_protocol::GuestSecurityConfig {
            seccomp: true,
            landlock: false,
            max_file_size_mb: 50,
            max_open_files: 128,
            tmpfs_size_mb: 256,
            max_processes: 64,
            allowed_paths: vec!["/workspace".into()],
        };
        let builder = SandboxBuilder::new("test").security(config.clone());
        assert_eq!(builder.security_config, Some(config));
    }

    #[test]
    fn builder_security_serializes_to_json() {
        let config = sage_protocol::GuestSecurityConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let decoded: sage_protocol::GuestSecurityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, decoded);
    }

    // ── prepare_rootfs creates volume mount points ─────────────────

    #[tokio::test]
    async fn prepare_rootfs_creates_volume_mount_dirs() {
        let builder = SandboxBuilder::new("test-vol")
            .mount("/host/ws", "/workspace", false)
            .mount("/host/data", "/data/input", true);

        let rootfs = builder.prepare_rootfs().await.unwrap();

        // Verify mount point directories were created
        assert!(rootfs.join("workspace").is_dir());
        assert!(rootfs.join("data/input").is_dir());

        // Cleanup
        let _ = std::fs::remove_dir_all(&rootfs);
    }

    #[tokio::test]
    async fn prepare_rootfs_without_volumes_succeeds() {
        let builder = SandboxBuilder::new("test-novol");
        let rootfs = builder.prepare_rootfs().await.unwrap();

        assert!(rootfs.join("bin").is_dir());
        assert!(rootfs.join("proc").is_dir());
        assert!(rootfs.join("tmp").is_dir());

        let _ = std::fs::remove_dir_all(&rootfs);
    }

    #[test]
    fn test_fix_rootfs_documents_available_commands() {
        // Regression: BUSYBOX_COMMANDS must match the actual symlinks created
        // in prepare_rootfs(). If you add a command to the rootfs, add it here too.
        let expected = &["sh", "echo", "cat", "ls", "mkdir", "rm", "cp", "mv"];
        assert_eq!(BUSYBOX_COMMANDS, expected);
    }
}
