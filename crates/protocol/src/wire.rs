use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WireError {
    #[error("incomplete frame, need more data")]
    Incomplete,
    #[error("frame too large: {0} bytes")]
    TooLarge(u32),
    #[error("cbor encode error: {0}")]
    Encode(String),
    #[error("cbor decode error: {0}")]
    Decode(String),
}

const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024; // 16 MiB

/// Encode a message into a length-prefixed CBOR frame.
///
/// Frame format: [u32 BE length][CBOR payload]
pub fn encode<T: Serialize>(msg: &T, buf: &mut BytesMut) -> Result<(), WireError> {
    let mut payload = Vec::new();
    ciborium::into_writer(msg, &mut payload).map_err(|e| WireError::Encode(e.to_string()))?;

    let len = payload.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(WireError::TooLarge(len));
    }

    buf.put_u32(len);
    buf.put_slice(&payload);
    Ok(())
}

/// Try to decode a length-prefixed CBOR frame from the buffer.
///
/// Returns `Err(WireError::Incomplete)` if not enough data yet.
pub fn decode<T: for<'de> Deserialize<'de>>(buf: &mut BytesMut) -> Result<T, WireError> {
    if buf.len() < 4 {
        return Err(WireError::Incomplete);
    }

    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if len > MAX_FRAME_SIZE {
        return Err(WireError::TooLarge(len));
    }

    let total = 4 + len as usize;
    if buf.len() < total {
        return Err(WireError::Incomplete);
    }

    buf.advance(4);
    let payload = buf.split_to(len as usize);
    ciborium::from_reader(payload.as_ref()).map_err(|e| WireError::Decode(e.to_string()))
}
