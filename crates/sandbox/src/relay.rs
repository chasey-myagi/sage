use agent_protocol::{GuestMessage, HostMessage};

/// Multiplexes host↔guest communication over virtio-console.
///
/// Routes responses back to the correct request by request_id.
#[allow(dead_code)]
pub struct AgentRelay {
    // TODO: virtio-console fd, pending requests map, background task
}

#[allow(dead_code)]
impl AgentRelay {
    /// Wait for the guest agent to send a Ready message.
    pub async fn wait_ready(&mut self, _timeout_secs: u64) -> anyhow::Result<()> {
        todo!("read from virtio-console until GuestMessage::Ready")
    }

    /// Send a host message and wait for the corresponding response.
    pub async fn request(&self, _msg: HostMessage) -> anyhow::Result<GuestMessage> {
        todo!("encode msg → write to console → register pending → await response")
    }
}
