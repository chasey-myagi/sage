use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::BytesMut;
use sage_protocol::{GuestMessage, HostMessage, wire};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::error::SandboxError;

/// Multiplexes host↔guest communication over piped stdio.
///
/// The relay writes HostMessages to the runtime process's stdin (which maps to
/// the guest's virtio-console), and reads GuestMessages from stdout. Responses
/// are routed to the correct caller by request_id.
pub struct AgentRelay {
    /// Writer end: host → runtime stdin → VMM → guest /dev/vport0p0
    writer: Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>,
    /// Pending requests awaiting a response, keyed by request_id.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<GuestMessage>>>>,
    /// Next request_id to assign.
    next_id: AtomicU64,
    /// Background reader task handle.
    _reader_task: JoinHandle<()>,
}

impl AgentRelay {
    /// Create a new relay from the runtime process's stdout (reader) and stdin (writer).
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<GuestMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let writer: Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>> =
            Arc::new(Mutex::new(Box::new(writer)));

        let reader_pending = pending.clone();
        let reader_task = tokio::spawn(Self::reader_loop(reader, reader_pending));

        Self {
            writer,
            pending,
            next_id: AtomicU64::new(1),
            _reader_task: reader_task,
        }
    }

    /// Wait for the guest agent to send a Ready message.
    pub async fn wait_ready(&self, timeout_secs: u64) -> Result<(), SandboxError> {
        // Register a receiver for request_id 0 (Ready uses no request_id)
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(0, tx);
        }

        let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await;

        match result {
            Ok(Ok(GuestMessage::Ready)) => {
                tracing::info!("guest agent ready");
                Ok(())
            }
            Ok(Ok(msg)) => Err(SandboxError::VmCreate(format!(
                "expected Ready, got {msg:?}"
            ))),
            Ok(Err(_)) => Err(SandboxError::VmCreate(
                "reader task dropped before Ready received".into(),
            )),
            Err(_) => Err(SandboxError::AgentTimeout(timeout_secs)),
        }
    }

    /// Send a host message and wait for the corresponding guest response.
    pub async fn request(&self, msg: HostMessage) -> Result<GuestMessage, SandboxError> {
        let request_id = self.request_id_of(&msg);

        // Register a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id, tx);
        }

        // Encode and send
        let mut frame_buf = BytesMut::new();
        wire::encode(&msg, &mut frame_buf)?;

        {
            let mut writer = self.writer.lock().await;
            AsyncWriteExt::write_all(&mut **writer, &frame_buf).await?;
            AsyncWriteExt::flush(&mut **writer).await?;
        }

        // Wait for response
        rx.await.map_err(|_| {
            SandboxError::ExecFailed("relay reader dropped before response received".into())
        })
    }

    /// Allocate the next request_id.
    pub fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Extract request_id from a HostMessage (for Shutdown, use 0).
    fn request_id_of(&self, msg: &HostMessage) -> u64 {
        match msg {
            HostMessage::ExecRequest(req) => req.request_id,
            HostMessage::FsRead(req) => req.request_id,
            HostMessage::FsWrite(req) => req.request_id,
            HostMessage::FsList(req) => req.request_id,
            HostMessage::Shutdown => 0,
        }
    }

    /// Background task: continuously read GuestMessages and route to pending callers.
    async fn reader_loop<R: AsyncRead + Unpin>(
        mut reader: R,
        pending: Arc<Mutex<HashMap<u64, oneshot::Sender<GuestMessage>>>>,
    ) {
        let mut buf = BytesMut::with_capacity(64 * 1024);

        loop {
            match AsyncReadExt::read_buf(&mut reader, &mut buf).await {
                Ok(0) => {
                    tracing::debug!("relay reader: EOF");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("relay reader error: {e}");
                    break;
                }
            }

            // Decode all complete frames
            loop {
                match wire::decode::<GuestMessage>(&mut buf) {
                    Ok(msg) => {
                        let request_id = Self::guest_msg_request_id(&msg);
                        let mut pending = pending.lock().await;
                        if let Some(tx) = pending.remove(&request_id) {
                            let _ = tx.send(msg);
                        } else {
                            tracing::warn!(request_id, "no pending receiver for guest message");
                        }
                    }
                    Err(sage_protocol::WireError::Incomplete) => break,
                    Err(e) => {
                        tracing::error!("relay decode error: {e}");
                        break;
                    }
                }
            }
        }

        // Drop all pending senders to unblock waiters
        let mut pending = pending.lock().await;
        pending.clear();
    }

    /// Extract request_id from a GuestMessage. Ready maps to 0.
    fn guest_msg_request_id(msg: &GuestMessage) -> u64 {
        match msg {
            GuestMessage::Ready => 0,
            GuestMessage::ExecStarted { request_id, .. }
            | GuestMessage::ExecStdout { request_id, .. }
            | GuestMessage::ExecStderr { request_id, .. }
            | GuestMessage::ExecExited { request_id, .. }
            | GuestMessage::FsData { request_id, .. }
            | GuestMessage::FsResult { request_id, .. }
            | GuestMessage::FsEntries { request_id, .. }
            | GuestMessage::Error { request_id, .. } => *request_id,
        }
    }
}

impl Drop for AgentRelay {
    fn drop(&mut self) {
        self._reader_task.abort();
    }
}
