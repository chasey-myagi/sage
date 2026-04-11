use crate::handle::SandboxHandle;
use crate::error::SandboxError;

/// Configuration for a sandbox VM instance.
pub struct SandboxBuilder {
    name: String,
    image: String,
    cpus: u32,
    memory_mib: u32,
    volumes: Vec<VolumeMount>,
    env: Vec<(String, String)>,
    network_allow: Vec<String>,
    idle_timeout_secs: u64,
}

pub struct VolumeMount {
    pub host_path: String,
    pub guest_path: String,
    pub read_only: bool,
}

impl SandboxBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: "alpine:latest".into(),
            cpus: 1,
            memory_mib: 256,
            volumes: Vec::new(),
            env: Vec::new(),
            network_allow: Vec::new(),
            idle_timeout_secs: 300,
        }
    }

    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    pub fn cpus(mut self, cpus: u32) -> Self {
        self.cpus = cpus;
        self
    }

    pub fn memory_mib(mut self, mib: u32) -> Self {
        self.memory_mib = mib;
        self
    }

    pub fn mount(mut self, host: impl Into<String>, guest: impl Into<String>, read_only: bool) -> Self {
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

    pub fn network_allow(mut self, domain: impl Into<String>) -> Self {
        self.network_allow.push(domain.into());
        self
    }

    pub fn idle_timeout_secs(mut self, secs: u64) -> Self {
        self.idle_timeout_secs = secs;
        self
    }

    /// Create and start the sandbox VM.
    ///
    /// This will:
    /// 1. Prepare the rootfs (OCI image or cached layer)
    /// 2. Configure the VM via msb_krun VmBuilder
    /// 3. Start the VM process
    /// 4. Wait for the guest agent to signal readiness
    pub async fn create(self) -> Result<SandboxHandle, SandboxError> {
        // TODO: Phase 1 — stub implementation
        // Phase 2 — msb_krun VmBuilder integration
        tracing::info!(name = %self.name, image = %self.image, "creating sandbox");
        todo!("msb_krun VM creation")
    }
}
