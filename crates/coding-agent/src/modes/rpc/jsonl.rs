//! JSONL (JSON Lines) framing utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/rpc/jsonl.ts`.
//!
//! Framing is LF-only. Records are split on `\n` only (not U+2028/U+2029).
//! This intentionally does not use `BufRead::lines()` which can normalise
//! additional Unicode line separators inside JSON string values.

use std::io::{self, BufRead, Write};

use serde::Serialize;

// ============================================================================
// Serialization
// ============================================================================

/// Serialize a value to a strict JSONL record (JSON + `\n`).
///
/// Mirrors `serializeJsonLine()` from TypeScript.
pub fn serialize_json_line<T: Serialize>(value: &T) -> anyhow::Result<String> {
    let mut s = serde_json::to_string(value)?;
    s.push('\n');
    Ok(s)
}

/// Write a serialized JSONL record to the given writer.
pub fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> anyhow::Result<()> {
    let line = serialize_json_line(value)?;
    writer.write_all(line.as_bytes())?;
    writer.flush()?;
    Ok(())
}

// ============================================================================
// LF-only JSONL reader
// ============================================================================

/// Read JSON lines from `reader`, splitting on `\n` only.
///
/// Mirrors `attachJsonlLineReader()` from TypeScript. Each non-empty line
/// is passed to `on_line`. CRLF is normalised by stripping a trailing `\r`.
///
/// Returns an error only on I/O failure; parse errors for individual lines
/// are left to the caller.
pub fn read_jsonl_lines<R: BufRead, F: FnMut(String)>(reader: R, mut on_line: F) -> io::Result<()> {
    let mut buffer = String::new();
    let mut reader = reader;

    loop {
        let n = reader.read_line(&mut buffer)?;
        if n == 0 {
            // EOF — emit any remaining content
            let line = buffer
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();
            if !line.is_empty() {
                on_line(line);
            }
            break;
        }

        // Strip LF (and optional leading CR)
        if buffer.ends_with('\n') {
            buffer.pop();
            if buffer.ends_with('\r') {
                buffer.pop();
            }
        }

        if !buffer.is_empty() {
            on_line(buffer.clone());
        }
        buffer.clear();
    }

    Ok(())
}

// ============================================================================
// Async JSONL reader (tokio)
// ============================================================================

#[cfg(feature = "async")]
pub mod async_reader {
    use tokio::io::{AsyncBufRead, AsyncBufReadExt};

    /// Async version of `read_jsonl_lines`. Reads lines from a tokio
    /// `AsyncBufRead` and calls `on_line` for each non-empty record.
    pub async fn read_jsonl_lines_async<R, F>(reader: R, mut on_line: F) -> std::io::Result<()>
    where
        R: AsyncBufRead + Unpin,
        F: FnMut(String),
    {
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await? {
            // tokio lines() already strips LF but preserves CR
            let line = line.trim_end_matches('\r').to_string();
            if !line.is_empty() {
                on_line(line);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialize_json_line_ends_with_newline() {
        let line = serialize_json_line(&json!({"hello": "world"})).unwrap();
        assert!(line.ends_with('\n'));
        assert!(!line.contains('\r'));
    }

    #[test]
    fn serialize_preserves_unicode_separators() {
        // U+2028 and U+2029 must NOT be escaped to `\n`
        let value = json!({"text": "a\u{2028}b\u{2029}c"});
        let line = serialize_json_line(&value).unwrap();
        // The string should contain the raw code points, not their escape sequences
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["text"].as_str().unwrap(), "a\u{2028}b\u{2029}c");
    }

    #[test]
    fn read_jsonl_lines_splits_on_lf_only() {
        let input = "{\"a\":1}\n{\"b\":2}\n".as_bytes();
        let mut lines = Vec::new();
        read_jsonl_lines(std::io::BufReader::new(input), |l| lines.push(l)).unwrap();
        assert_eq!(lines, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
    }

    #[test]
    fn read_jsonl_lines_handles_crlf() {
        let input = "{\"a\":1}\r\n{\"b\":2}\r\n".as_bytes();
        let mut lines = Vec::new();
        read_jsonl_lines(std::io::BufReader::new(input), |l| lines.push(l)).unwrap();
        assert_eq!(lines, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
    }

    #[test]
    fn read_jsonl_lines_no_trailing_newline() {
        let input = r#"{"a":1}"#.as_bytes();
        let mut lines = Vec::new();
        read_jsonl_lines(std::io::BufReader::new(input), |l| lines.push(l)).unwrap();
        assert_eq!(lines, vec![r#"{"a":1}"#]);
    }

    #[test]
    fn unicode_separators_survive_round_trip() {
        let value = json!({"text": "a\u{2028}b\u{2029}c"});
        let line = serialize_json_line(&value).unwrap();

        let mut lines = Vec::new();
        read_jsonl_lines(std::io::BufReader::new(line.as_bytes()), |l| lines.push(l)).unwrap();

        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["text"].as_str().unwrap(), "a\u{2028}b\u{2029}c");
    }
}
