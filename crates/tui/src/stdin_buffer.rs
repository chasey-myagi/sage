/// StdinBuffer buffers input and emits complete sequences.
///
/// This is necessary because stdin data events can arrive in partial chunks.
/// Without buffering, partial escape sequences can be misinterpreted.
///
/// Based on code from OpenTUI (https://github.com/anomalyco/opentui)
/// MIT License - Copyright (c) 2025 opentui

const ESC: char = '\x1b';
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";

/// Result of checking if a sequence is complete.
#[derive(Debug, PartialEq, Eq)]
enum SequenceStatus {
    Complete,
    Incomplete,
    NotEscape,
}

fn is_complete_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with(ESC) {
        return SequenceStatus::NotEscape;
    }
    if data.len() == 1 {
        return SequenceStatus::Incomplete;
    }
    let after_esc = &data[1..];

    if after_esc.starts_with('[') {
        if after_esc.starts_with("[M") {
            // Old-style mouse: ESC[M + 3 bytes = 6 total
            return if data.len() >= 6 { SequenceStatus::Complete } else { SequenceStatus::Incomplete };
        }
        return is_complete_csi_sequence(data);
    }
    if after_esc.starts_with(']') {
        return is_complete_osc_sequence(data);
    }
    if after_esc.starts_with('P') {
        return is_complete_dcs_sequence(data);
    }
    if after_esc.starts_with('_') {
        return is_complete_apc_sequence(data);
    }
    if after_esc.starts_with('O') {
        return if after_esc.len() >= 2 { SequenceStatus::Complete } else { SequenceStatus::Incomplete };
    }
    if after_esc.len() == 1 {
        return SequenceStatus::Complete;
    }
    SequenceStatus::Complete
}

fn is_complete_csi_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b[") {
        return SequenceStatus::Complete;
    }
    if data.len() < 3 {
        return SequenceStatus::Incomplete;
    }
    let payload = &data[2..];
    let last_char = match payload.chars().last() {
        Some(c) => c,
        None => return SequenceStatus::Incomplete,
    };
    let code = last_char as u32;
    if code >= 0x40 && code <= 0x7e {
        // SGR mouse sequences
        if payload.starts_with('<') {
            let mouse_re = regex::Regex::new(r"^<\d+;\d+;\d+[Mm]$").unwrap();
            if mouse_re.is_match(payload) {
                return SequenceStatus::Complete;
            }
            if last_char == 'M' || last_char == 'm' {
                let parts: Vec<&str> = payload[1..payload.len() - 1].split(';').collect();
                if parts.len() == 3 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
                    return SequenceStatus::Complete;
                }
            }
            return SequenceStatus::Incomplete;
        }
        return SequenceStatus::Complete;
    }
    SequenceStatus::Incomplete
}

fn is_complete_osc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b]") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") || data.ends_with('\x07') {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_dcs_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1bP") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_apc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b_") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn extract_complete_sequences(buffer: &str) -> (Vec<String>, String) {
    let mut sequences = Vec::new();
    let mut pos = 0;
    let bytes = buffer.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'\x1b' {
            let remaining = &buffer[pos..];
            let mut seq_end = 1;
            loop {
                if seq_end > remaining.len() {
                    return (sequences, remaining.to_string());
                }
                let candidate = &remaining[..seq_end];
                match is_complete_sequence(candidate) {
                    SequenceStatus::Complete => {
                        sequences.push(candidate.to_string());
                        pos += seq_end;
                        break;
                    }
                    SequenceStatus::Incomplete => {
                        seq_end += 1;
                    }
                    SequenceStatus::NotEscape => {
                        sequences.push(candidate.to_string());
                        pos += seq_end;
                        break;
                    }
                }
            }
        } else {
            // Single byte character (or start of multi-byte UTF-8)
            // Find the end of this character
            let ch_end = buffer[pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| pos + i)
                .unwrap_or(buffer.len());
            sequences.push(buffer[pos..ch_end].to_string());
            pos = ch_end;
        }
    }

    (sequences, String::new())
}

/// Event emitted by StdinBuffer.
#[derive(Debug)]
pub enum StdinEvent {
    /// A complete key/escape sequence
    Data(String),
    /// A bracketed paste's content
    Paste(String),
}

/// Options for StdinBuffer.
pub struct StdinBufferOptions {
    /// Maximum time to wait for sequence completion (milliseconds).
    pub timeout_ms: u64,
}

impl Default for StdinBufferOptions {
    fn default() -> Self {
        Self { timeout_ms: 10 }
    }
}

/// Buffers stdin input and emits complete sequences.
pub struct StdinBuffer {
    buffer: String,
    paste_mode: bool,
    paste_buffer: String,
    timeout_ms: u64,
}

impl StdinBuffer {
    pub fn new(options: StdinBufferOptions) -> Self {
        Self {
            buffer: String::new(),
            paste_mode: false,
            paste_buffer: String::new(),
            timeout_ms: options.timeout_ms,
        }
    }

    /// Process incoming data, returning any complete events.
    pub fn process(&mut self, data: &str) -> Vec<StdinEvent> {
        let mut events = Vec::new();

        if data.is_empty() && self.buffer.is_empty() {
            events.push(StdinEvent::Data(String::new()));
            return events;
        }

        self.buffer.push_str(data);

        if self.paste_mode {
            self.paste_buffer.push_str(&self.buffer);
            self.buffer.clear();

            if let Some(end_idx) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let content = self.paste_buffer[..end_idx].to_string();
                let remaining = self.paste_buffer[end_idx + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                events.push(StdinEvent::Paste(content));
                if !remaining.is_empty() {
                    let mut more = self.process(&remaining);
                    events.append(&mut more);
                }
            }
            return events;
        }

        if let Some(start_idx) = self.buffer.find(BRACKETED_PASTE_START) {
            if start_idx > 0 {
                let before = self.buffer[..start_idx].to_string();
                let (seqs, _) = extract_complete_sequences(&before);
                for seq in seqs {
                    events.push(StdinEvent::Data(seq));
                }
            }
            let after_start = &self.buffer[start_idx + BRACKETED_PASTE_START.len()..];
            self.paste_buffer = after_start.to_string();
            self.buffer.clear();
            self.paste_mode = true;

            if let Some(end_idx) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let content = self.paste_buffer[..end_idx].to_string();
                let remaining = self.paste_buffer[end_idx + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                events.push(StdinEvent::Paste(content));
                if !remaining.is_empty() {
                    let mut more = self.process(&remaining);
                    events.append(&mut more);
                }
            }
            return events;
        }

        let (seqs, remainder) = extract_complete_sequences(&self.buffer);
        self.buffer = remainder;
        for seq in seqs {
            events.push(StdinEvent::Data(seq));
        }

        events
    }

    /// Flush any remaining buffered data.
    pub fn flush(&mut self) -> Vec<StdinEvent> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let buf = std::mem::take(&mut self.buffer);
        vec![StdinEvent::Data(buf)]
    }

    /// Clear all buffered state.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.paste_mode = false;
        self.paste_buffer.clear();
    }

    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_buf() -> StdinBuffer {
        StdinBuffer::new(StdinBufferOptions::default())
    }

    fn collect_data(events: Vec<StdinEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|e| match e {
                StdinEvent::Data(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_plain_char() {
        let mut buf = new_buf();
        let events = buf.process("a");
        let data = collect_data(events);
        assert_eq!(data, vec!["a"]);
    }

    #[test]
    fn test_escape_complete() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[A");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b[A"]);
    }

    #[test]
    fn test_multiple_chars() {
        let mut buf = new_buf();
        let events = buf.process("abc");
        let data = collect_data(events);
        assert_eq!(data, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_bracketed_paste() {
        let mut buf = new_buf();
        let input = format!("{BRACKETED_PASTE_START}hello world{BRACKETED_PASTE_END}");
        let events = buf.process(&input);
        let pastes: Vec<String> = events
            .into_iter()
            .filter_map(|e| match e {
                StdinEvent::Paste(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(pastes, vec!["hello world"]);
    }

    #[test]
    fn test_empty_input() {
        let mut buf = new_buf();
        let events = buf.process("");
        let data = collect_data(events);
        assert_eq!(data, vec![""]);
    }

    #[test]
    fn test_flush_remainder() {
        let mut buf = new_buf();
        // Partial escape - won't complete
        buf.buffer = "\x1b[".to_string();
        let events = buf.flush();
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b["]);
    }

    #[test]
    fn test_multiple_sequences() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[A\x1b[B");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b[A", "\x1b[B"]);
    }

    #[test]
    fn test_is_complete_csi_sequence() {
        assert_eq!(is_complete_sequence("\x1b[A"), SequenceStatus::Complete);
        assert_eq!(is_complete_sequence("\x1b["), SequenceStatus::Incomplete);
        assert_eq!(is_complete_sequence("a"), SequenceStatus::NotEscape);
    }

    #[test]
    fn test_is_complete_osc_sequence() {
        assert_eq!(is_complete_sequence("\x1b]0;title\x07"), SequenceStatus::Complete);
        assert_eq!(is_complete_sequence("\x1b]0;title"), SequenceStatus::Incomplete);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Regular Characters
    // -------------------------------------------------------------------------

    #[test]
    fn test_regular_chars_passthrough() {
        let mut buf = new_buf();
        let events = buf.process("a");
        let data = collect_data(events);
        assert_eq!(data, vec!["a"]);
    }

    #[test]
    fn test_multiple_regular_chars() {
        let mut buf = new_buf();
        let events = buf.process("abc");
        let data = collect_data(events);
        assert_eq!(data, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_unicode_chars() {
        let mut buf = new_buf();
        let events = buf.process("hello 世界");
        let data = collect_data(events);
        assert_eq!(data, vec!["h", "e", "l", "l", "o", " ", "世", "界"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Complete Escape Sequences
    // -------------------------------------------------------------------------

    #[test]
    fn test_complete_mouse_sgr_sequence() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[<35;20;5m");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b[<35;20;5m"]);
    }

    #[test]
    fn test_complete_arrow_key_sequence() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[A");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b[A"]);
    }

    #[test]
    fn test_complete_function_key_sequence() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[11~");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1b[11~"]);
    }

    #[test]
    fn test_meta_key_sequence() {
        let mut buf = new_buf();
        let events = buf.process("\x1ba");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1ba"]);
    }

    #[test]
    fn test_ss3_sequence() {
        let mut buf = new_buf();
        let events = buf.process("\x1bOA");
        let data = collect_data(events);
        assert_eq!(data, vec!["\x1bOA"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Partial Escape Sequences
    // -------------------------------------------------------------------------

    #[test]
    fn test_buffer_incomplete_csi_sequence_multi_chunk() {
        let mut buf = new_buf();
        let e1 = collect_data(buf.process("\x1b["));
        assert!(e1.is_empty());

        let e2 = collect_data(buf.process("1;"));
        assert!(e2.is_empty());

        let e3 = collect_data(buf.process("5H"));
        assert_eq!(e3, vec!["\x1b[1;5H"]);
    }

    #[test]
    fn test_buffer_split_across_many_chunks() {
        let mut buf = new_buf();
        for ch in ["\x1b", "[", "<", "3", "5", ";", "2", "0", ";", "5", "m"] {
            buf.process(ch);
        }
        // After last chunk, sequence should be complete
        // Collect all events by processing the final char
        let mut buf2 = new_buf();
        let mut all_data: Vec<String> = Vec::new();
        for ch in ["\x1b", "[", "<", "3", "5", ";", "2", "0", ";", "5"] {
            all_data.extend(collect_data(buf2.process(ch)));
        }
        all_data.extend(collect_data(buf2.process("m")));
        assert_eq!(all_data, vec!["\x1b[<35;20;5m"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Mixed Content
    // -------------------------------------------------------------------------

    #[test]
    fn test_chars_followed_by_escape_sequence() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("abc\x1b[A"));
        assert_eq!(data, vec!["a", "b", "c", "\x1b[A"]);
    }

    #[test]
    fn test_escape_sequence_followed_by_chars() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[Aabc"));
        assert_eq!(data, vec!["\x1b[A", "a", "b", "c"]);
    }

    #[test]
    fn test_multiple_complete_sequences() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[A\x1b[B\x1b[C"));
        assert_eq!(data, vec!["\x1b[A", "\x1b[B", "\x1b[C"]);
    }

    #[test]
    fn test_partial_sequence_with_preceding_chars() {
        let mut buf = new_buf();
        let data1 = collect_data(buf.process("abc\x1b[<35"));
        assert_eq!(data1, vec!["a", "b", "c"]);
        assert_eq!(buf.get_buffer(), "\x1b[<35");

        let data2 = collect_data(buf.process(";20;5m"));
        assert_eq!(data2, vec!["\x1b[<35;20;5m"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Kitty Keyboard Protocol
    // -------------------------------------------------------------------------

    #[test]
    fn test_kitty_csi_u_press() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[97u"));
        assert_eq!(data, vec!["\x1b[97u"]);
    }

    #[test]
    fn test_kitty_csi_u_release() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[97;1:3u"));
        assert_eq!(data, vec!["\x1b[97;1:3u"]);
    }

    #[test]
    fn test_kitty_batched_press_and_release() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[97u\x1b[97;1:3u"));
        assert_eq!(data, vec!["\x1b[97u", "\x1b[97;1:3u"]);
    }

    #[test]
    fn test_kitty_multiple_batched_events() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[97u\x1b[97;1:3u\x1b[98u\x1b[98;1:3u"));
        assert_eq!(data, vec!["\x1b[97u", "\x1b[97;1:3u", "\x1b[98u", "\x1b[98;1:3u"]);
    }

    #[test]
    fn test_kitty_arrow_keys_with_event_type() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[1;1:1A"));
        assert_eq!(data, vec!["\x1b[1;1:1A"]);
    }

    #[test]
    fn test_kitty_functional_keys_with_event_type() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[3;1:3~"));
        assert_eq!(data, vec!["\x1b[3;1:3~"]);
    }

    #[test]
    fn test_kitty_plain_chars_mixed_with_kitty() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("a\x1b[97;1:3u"));
        assert_eq!(data, vec!["a", "\x1b[97;1:3u"]);
    }

    #[test]
    fn test_kitty_sequence_followed_by_plain_chars() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[97ua"));
        assert_eq!(data, vec!["\x1b[97u", "a"]);
    }

    #[test]
    fn test_kitty_rapid_typing_simulation() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[104u\x1b[104;1:3u\x1b[105u\x1b[105;1:3u"));
        assert_eq!(data, vec!["\x1b[104u", "\x1b[104;1:3u", "\x1b[105u", "\x1b[105;1:3u"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Mouse Events
    // -------------------------------------------------------------------------

    #[test]
    fn test_mouse_press_event() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[<0;10;5M"));
        assert_eq!(data, vec!["\x1b[<0;10;5M"]);
    }

    #[test]
    fn test_mouse_release_event() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[<0;10;5m"));
        assert_eq!(data, vec!["\x1b[<0;10;5m"]);
    }

    #[test]
    fn test_mouse_move_event() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[<35;20;5m"));
        assert_eq!(data, vec!["\x1b[<35;20;5m"]);
    }

    #[test]
    fn test_split_mouse_events() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b[<3"));
        collect_data(buf.process("5;1"));
        collect_data(buf.process("5;"));
        let data = collect_data(buf.process("10m"));
        assert_eq!(data, vec!["\x1b[<35;15;10m"]);
    }

    #[test]
    fn test_multiple_mouse_events() {
        let mut buf = new_buf();
        let data = collect_data(buf.process("\x1b[<35;1;1m\x1b[<35;2;2m\x1b[<35;3;3m"));
        assert_eq!(data, vec!["\x1b[<35;1;1m", "\x1b[<35;2;2m", "\x1b[<35;3;3m"]);
    }

    #[test]
    fn test_old_style_mouse_sequence() {
        let mut buf = new_buf();
        // ESC[M + 3 bytes = 6 total; "abc" is 3 bytes so " abc" gives ESC[M + space + a + b = 6 bytes, then c extra
        let data = collect_data(buf.process("\x1b[M abc"));
        assert_eq!(data, vec!["\x1b[M ab", "c"]);
    }

    #[test]
    fn test_buffer_incomplete_old_style_mouse() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b[M"));
        assert_eq!(buf.get_buffer(), "\x1b[M");

        collect_data(buf.process(" a"));
        assert_eq!(buf.get_buffer(), "\x1b[M a");

        let data = collect_data(buf.process("b"));
        assert_eq!(data, vec!["\x1b[M ab"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Edge Cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_empty_input_edge_case() {
        let mut buf = new_buf();
        let events = buf.process("");
        let data = collect_data(events);
        assert_eq!(data, vec![""]);
    }

    #[test]
    fn test_lone_escape_explicit_flush() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b"));
        let flushed = collect_data(buf.flush());
        assert_eq!(flushed, vec!["\x1b"]);
    }

    #[test]
    fn test_very_long_sequences() {
        let mut buf = new_buf();
        let params = "1;".repeat(50);
        let long_seq = format!("\x1b[{params}H");
        let data = collect_data(buf.process(&long_seq));
        assert_eq!(data, vec![long_seq.as_str()]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Flush
    // -------------------------------------------------------------------------

    #[test]
    fn test_flush_incomplete_sequences() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b[<35"));
        let flushed = collect_data(buf.flush());
        assert_eq!(flushed, vec!["\x1b[<35"]);
        assert_eq!(buf.get_buffer(), "");
    }

    #[test]
    fn test_flush_empty_returns_empty() {
        let mut buf = new_buf();
        let flushed = collect_data(buf.flush());
        assert!(flushed.is_empty());
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Clear
    // -------------------------------------------------------------------------

    #[test]
    fn test_clear_buffered_content() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b[<35"));
        assert_eq!(buf.get_buffer(), "\x1b[<35");

        buf.clear();
        assert_eq!(buf.get_buffer(), "");
        // No data was emitted
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Bracketed Paste
    // -------------------------------------------------------------------------

    fn collect_paste(events: Vec<StdinEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|e| match e {
                StdinEvent::Paste(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_bracketed_paste_complete() {
        let mut buf = new_buf();
        let input = format!("{BRACKETED_PASTE_START}hello world{BRACKETED_PASTE_END}");
        let events = buf.process(&input);
        let pastes = collect_paste(events.iter().filter_map(|e| match e {
            StdinEvent::Paste(s) => Some(StdinEvent::Paste(s.clone())),
            _ => None,
        }).collect());
        // Use the raw events
        let all_pastes: Vec<String> = events.into_iter().filter_map(|e| match e {
            StdinEvent::Paste(s) => Some(s),
            _ => None,
        }).collect();
        assert_eq!(all_pastes, vec!["hello world"]);
    }

    #[test]
    fn test_bracketed_paste_in_chunks() {
        let mut buf = new_buf();

        let e1: Vec<_> = buf.process("\x1b[200~").into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        assert!(e1.is_empty());

        let e2: Vec<_> = buf.process("hello ").into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        assert!(e2.is_empty());

        let e3: Vec<_> = buf.process("world\x1b[201~").into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        assert_eq!(e3, vec!["hello world"]);
    }

    #[test]
    fn test_bracketed_paste_with_text_before_and_after() {
        let mut buf = new_buf();
        buf.process("a");
        let mid_events = buf.process("\x1b[200~pasted\x1b[201~");
        let pastes: Vec<_> = mid_events.into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        let data_b = collect_data(buf.process("b"));

        assert_eq!(pastes, vec!["pasted"]);
        assert_eq!(data_b, vec!["b"]);
    }

    #[test]
    fn test_bracketed_paste_with_newlines() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[200~line1\nline2\nline3\x1b[201~");
        let pastes: Vec<_> = events.into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        assert_eq!(pastes, vec!["line1\nline2\nline3"]);
    }

    #[test]
    fn test_bracketed_paste_with_unicode() {
        let mut buf = new_buf();
        let events = buf.process("\x1b[200~Hello 世界 🎉\x1b[201~");
        let pastes: Vec<_> = events.into_iter()
            .filter_map(|e| match e { StdinEvent::Paste(s) => Some(s), _ => None }).collect();
        assert_eq!(pastes, vec!["Hello 世界 🎉"]);
    }

    // -------------------------------------------------------------------------
    // Tests from stdin-buffer.test.ts – Destroy (maps to clear + drop)
    // -------------------------------------------------------------------------

    #[test]
    fn test_destroy_clears_buffer() {
        let mut buf = new_buf();
        collect_data(buf.process("\x1b[<35"));
        assert_eq!(buf.get_buffer(), "\x1b[<35");

        buf.clear(); // destroy equivalent
        assert_eq!(buf.get_buffer(), "");
    }
}
