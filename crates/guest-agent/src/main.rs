//! Guest Agent — runs as PID 1 inside the sandbox VM.
//!
//! Responsibilities:
//! 1. Mount essential filesystems (proc, sys, dev, tmp)
//! 2. Open virtio-console for host communication
//! 3. Signal readiness to host
//! 4. Handle exec/fs requests from host
//! 5. Reap orphan child processes (PID 1 duty)

mod exec;
mod fs;
mod init;

use agent_protocol::{GuestMessage, HostMessage, wire};
use anyhow::{Context, Result};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default console device path inside the VM.
/// msb_krun maps named console ports to /dev/vport0pN (first port = p0).
const CONSOLE_PATH: &str = "/dev/vport0p0";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("agent_guest=debug")
        .init();

    tracing::info!("guest agent starting (PID {})", std::process::id());

    // Phase 1: Mount essential filesystems
    init::mount_filesystems()?;

    // Phase 2: Start SIGCHLD reaper (PID 1 duty)
    #[cfg(target_os = "linux")]
    start_reaper();

    // Phase 3: Open console and run main loop
    run_main_loop().await
}

/// Main communication loop: open console → send Ready → handle requests.
async fn run_main_loop() -> Result<()> {
    let console = open_console().await?;
    let (reader, writer) = tokio::io::split(console);

    let writer = std::sync::Arc::new(tokio::sync::Mutex::new(writer));

    // Signal readiness to host
    send_message(&writer, &GuestMessage::Ready).await?;
    tracing::info!("guest agent ready");

    // Main loop: read HostMessage → dispatch → send GuestMessage
    let mut read_buf = BytesMut::with_capacity(64 * 1024);
    let mut reader = reader;

    loop {
        // Read data from console
        let n = reader
            .read_buf(&mut read_buf)
            .await
            .context("read console")?;

        if n == 0 {
            tracing::info!("console closed, shutting down");
            break;
        }

        // Try to decode complete frames
        loop {
            match wire::decode::<HostMessage>(&mut read_buf) {
                Ok(msg) => {
                    let writer = writer.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_message(msg, &writer).await {
                            tracing::error!("handle message error: {e}");
                        }
                    });
                }
                Err(agent_protocol::WireError::Incomplete) => break,
                Err(e) => {
                    tracing::error!("wire decode error: {e}");
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Dispatch a single host message to the appropriate handler.
async fn handle_message<W>(
    msg: HostMessage,
    writer: &std::sync::Arc<tokio::sync::Mutex<W>>,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    match msg {
        HostMessage::ExecRequest(req) => {
            let request_id = req.request_id;
            tracing::debug!(request_id, command = %req.command, "exec request");

            let response = match exec::handle_exec(&req).await {
                Ok(result) => GuestMessage::ExecExited {
                    request_id,
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                },
                Err(e) => GuestMessage::Error {
                    request_id,
                    message: format!("exec failed: {e}"),
                },
            };
            send_message(writer, &response).await?;
        }
        HostMessage::FsRead(req) => {
            let request_id = req.request_id;
            tracing::debug!(request_id, path = %req.path, "fs_read request");

            let response = match fs::handle_read(&req).await {
                Ok(data) => GuestMessage::FsData { request_id, data },
                Err(e) => GuestMessage::Error {
                    request_id,
                    message: format!("fs_read failed: {e}"),
                },
            };
            send_message(writer, &response).await?;
        }
        HostMessage::FsWrite(req) => {
            let request_id = req.request_id;
            tracing::debug!(request_id, path = %req.path, "fs_write request");

            let response = match fs::handle_write(&req).await {
                Ok(()) => GuestMessage::FsResult {
                    request_id,
                    success: true,
                    error: String::new(),
                },
                Err(e) => GuestMessage::FsResult {
                    request_id,
                    success: false,
                    error: e.to_string(),
                },
            };
            send_message(writer, &response).await?;
        }
        HostMessage::FsList(req) => {
            let request_id = req.request_id;
            tracing::debug!(request_id, path = %req.path, "fs_list request");

            let response = match fs::handle_list(&req).await {
                Ok(entries) => GuestMessage::FsEntries {
                    request_id,
                    entries,
                },
                Err(e) => GuestMessage::Error {
                    request_id,
                    message: format!("fs_list failed: {e}"),
                },
            };
            send_message(writer, &response).await?;
        }
        HostMessage::Shutdown => {
            tracing::info!("shutdown requested, exiting");
            std::process::exit(0);
        }
    }

    Ok(())
}

/// Encode and send a GuestMessage over the console.
async fn send_message<W>(
    writer: &std::sync::Arc<tokio::sync::Mutex<W>>,
    msg: &GuestMessage,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut frame_buf = BytesMut::new();
    wire::encode(msg, &mut frame_buf).context("encode message")?;

    let mut writer = writer.lock().await;
    writer
        .write_all(&frame_buf)
        .await
        .context("write to console")?;
    writer.flush().await.context("flush console")?;
    Ok(())
}

/// Open the virtio-console device for host communication.
async fn open_console() -> Result<tokio::fs::File> {
    tracing::debug!("opening console at {CONSOLE_PATH}");
    tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(CONSOLE_PATH)
        .await
        .with_context(|| format!("open {CONSOLE_PATH}"))
}

/// SIGCHLD reaper — PID 1 must reap orphan children to prevent zombies.
#[cfg(target_os = "linux")]
fn start_reaper() {
    tokio::spawn(async {
        use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
        use nix::unistd::Pid;
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigchld = match signal(SignalKind::child()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to register SIGCHLD handler: {e}");
                return;
            }
        };

        loop {
            sigchld.recv().await;
            // Reap all available children
            loop {
                match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) | Err(_) => break,
                    Ok(status) => {
                        tracing::debug!("reaped child: {status:?}");
                    }
                }
            }
        }
    });
    tracing::debug!("SIGCHLD reaper started");
}
