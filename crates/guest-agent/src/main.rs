//! Guest Agent — runs as PID 1 inside the sandbox VM.
//!
//! Responsibilities:
//! 1. Mount essential filesystems (proc, sys, dev, tmp)
//! 2. Open virtio-console named port for host communication
//! 3. Signal readiness to host
//! 4. Handle exec/fs requests from host
//! 5. Reap orphan child processes (PID 1 duty)

mod exec;
mod fs;
mod init;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("agent_guest=debug")
        .init();

    tracing::info!("guest agent starting (PID {})", std::process::id());

    // Phase 1: Mount essential filesystems
    init::mount_filesystems()?;

    // Phase 2: Open virtio-console for host communication
    // TODO: discover and open /dev/vport* named port

    // Phase 3: Signal readiness
    tracing::info!("guest agent ready");
    // TODO: send GuestMessage::Ready

    // Phase 4: Main loop — handle host requests
    // TODO: read HostMessage → dispatch to exec/fs handlers → send GuestMessage

    // Phase 5: SIGCHLD reaper (background task)
    // TODO: tokio::spawn reaper that waitpid(-1, WNOHANG) on SIGCHLD

    Ok(())
}
