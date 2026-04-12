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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExecRequest, FsEntry, FsListRequest, FsReadRequest, FsWriteRequest, GuestMessage,
        HostMessage,
    };

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(msg: &T) {
        let mut buf = BytesMut::new();
        encode(msg, &mut buf).unwrap();
        let decoded: T = decode(&mut buf).unwrap();
        assert_eq!(&decoded, msg);
        assert!(buf.is_empty(), "buffer should be fully consumed");
    }

    // --- HostMessage roundtrip tests ---

    #[test]
    fn host_exec_request_roundtrip() {
        roundtrip(&HostMessage::ExecRequest(ExecRequest {
            request_id: 42,
            command: "echo".into(),
            args: vec!["hello".into(), "world".into()],
            env: vec![("PATH".into(), "/usr/bin".into())],
            cwd: "/tmp".into(),
            timeout_secs: 30,
        }));
    }

    #[test]
    fn host_fs_read_roundtrip() {
        roundtrip(&HostMessage::FsRead(FsReadRequest {
            request_id: 1,
            path: "/etc/hostname".into(),
        }));
    }

    #[test]
    fn host_fs_write_roundtrip() {
        roundtrip(&HostMessage::FsWrite(FsWriteRequest {
            request_id: 2,
            path: "/tmp/test.txt".into(),
            data: b"hello world".to_vec(),
        }));
    }

    #[test]
    fn host_fs_list_roundtrip() {
        roundtrip(&HostMessage::FsList(FsListRequest {
            request_id: 3,
            path: "/tmp".into(),
        }));
    }

    #[test]
    fn host_shutdown_roundtrip() {
        roundtrip(&HostMessage::Shutdown);
    }

    // --- GuestMessage roundtrip tests ---

    #[test]
    fn guest_ready_roundtrip() {
        roundtrip(&GuestMessage::Ready);
    }

    #[test]
    fn guest_exec_started_roundtrip() {
        roundtrip(&GuestMessage::ExecStarted {
            request_id: 1,
            pid: 1234,
        });
    }

    #[test]
    fn guest_exec_stdout_roundtrip() {
        roundtrip(&GuestMessage::ExecStdout {
            request_id: 1,
            data: b"output line\n".to_vec(),
        });
    }

    #[test]
    fn guest_exec_stderr_roundtrip() {
        roundtrip(&GuestMessage::ExecStderr {
            request_id: 1,
            data: b"error: not found\n".to_vec(),
        });
    }

    #[test]
    fn guest_exec_exited_roundtrip() {
        roundtrip(&GuestMessage::ExecExited {
            request_id: 1,
            exit_code: 0,
            stdout: b"hello\n".to_vec(),
            stderr: Vec::new(),
        });
    }

    #[test]
    fn guest_exec_exited_with_error_roundtrip() {
        roundtrip(&GuestMessage::ExecExited {
            request_id: 99,
            exit_code: 127,
            stdout: Vec::new(),
            stderr: b"command not found\n".to_vec(),
        });
    }

    #[test]
    fn guest_fs_data_roundtrip() {
        roundtrip(&GuestMessage::FsData {
            request_id: 2,
            data: b"file contents here".to_vec(),
        });
    }

    #[test]
    fn guest_fs_result_roundtrip() {
        roundtrip(&GuestMessage::FsResult {
            request_id: 3,
            success: true,
            error: String::new(),
        });
    }

    #[test]
    fn guest_fs_entries_roundtrip() {
        roundtrip(&GuestMessage::FsEntries {
            request_id: 4,
            entries: vec![
                FsEntry {
                    name: "file.txt".into(),
                    is_dir: false,
                    size: 1024,
                },
                FsEntry {
                    name: "subdir".into(),
                    is_dir: true,
                    size: 0,
                },
            ],
        });
    }

    #[test]
    fn guest_error_roundtrip() {
        roundtrip(&GuestMessage::Error {
            request_id: 5,
            message: "permission denied".into(),
        });
    }

    // --- Wire layer edge cases ---

    #[test]
    fn decode_incomplete_length() {
        let mut buf = BytesMut::from(&[0u8, 0, 0][..]);
        let result = decode::<GuestMessage>(&mut buf);
        assert!(matches!(result, Err(WireError::Incomplete)));
    }

    #[test]
    fn decode_incomplete_payload() {
        let mut buf = BytesMut::new();
        encode(&GuestMessage::Ready, &mut buf).unwrap();
        // Truncate the payload
        buf.truncate(buf.len() - 1);
        let result = decode::<GuestMessage>(&mut buf);
        assert!(matches!(result, Err(WireError::Incomplete)));
    }

    #[test]
    fn decode_too_large_frame() {
        let mut buf = BytesMut::new();
        // Write a length header that exceeds MAX_FRAME_SIZE
        buf.put_u32(MAX_FRAME_SIZE + 1);
        let result = decode::<GuestMessage>(&mut buf);
        assert!(matches!(result, Err(WireError::TooLarge(_))));
    }

    #[test]
    fn multiple_frames_sequential_decode() {
        let mut buf = BytesMut::new();
        let msg1 = GuestMessage::Ready;
        let msg2 = GuestMessage::ExecStarted {
            request_id: 1,
            pid: 42,
        };
        let msg3 = GuestMessage::ExecExited {
            request_id: 1,
            exit_code: 0,
            stdout: b"ok\n".to_vec(),
            stderr: Vec::new(),
        };

        encode(&msg1, &mut buf).unwrap();
        encode(&msg2, &mut buf).unwrap();
        encode(&msg3, &mut buf).unwrap();

        assert_eq!(decode::<GuestMessage>(&mut buf).unwrap(), msg1);
        assert_eq!(decode::<GuestMessage>(&mut buf).unwrap(), msg2);
        assert_eq!(decode::<GuestMessage>(&mut buf).unwrap(), msg3);
        assert!(buf.is_empty());
    }

    #[test]
    fn empty_data_roundtrip() {
        roundtrip(&GuestMessage::ExecStdout {
            request_id: 0,
            data: Vec::new(),
        });
        roundtrip(&HostMessage::FsWrite(FsWriteRequest {
            request_id: 0,
            path: String::new(),
            data: Vec::new(),
        }));
    }

    #[test]
    fn large_data_roundtrip() {
        let large_data = vec![0xABu8; 1024 * 1024]; // 1 MiB
        roundtrip(&GuestMessage::FsData {
            request_id: 1,
            data: large_data,
        });
    }

    #[test]
    fn empty_buffer_returns_incomplete() {
        let mut buf = BytesMut::new();
        let result = decode::<HostMessage>(&mut buf);
        assert!(matches!(result, Err(WireError::Incomplete)));
    }
}
