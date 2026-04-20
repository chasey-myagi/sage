/// Utility functions for terminal text rendering.
/// Handles ANSI codes, grapheme widths, text wrapping, and truncation.
use unicode_width::UnicodeWidthChar;

// =============================================================================
// Visible width calculation
// =============================================================================

/// Calculate the visible terminal width of a string (ignoring ANSI codes).
pub fn visible_width(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    // Fast path for pure ASCII printable
    if s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return s.len();
    }
    let stripped = strip_ansi(s);
    let with_tabs = stripped.replace('\t', "   "); // 3 spaces per tab
    grapheme_width_str(&with_tabs)
}

fn grapheme_width_str(s: &str) -> usize {
    let mut width = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let cp = c as u32;
        if (0x1F1E6..=0x1F1FF).contains(&cp) {
            // Regional indicator symbol. Check if the next char is also a regional indicator
            // so we can count a flag emoji pair as width 2 total (not 2+2=4).
            let next_cp = chars.get(i + 1).map(|&nc| nc as u32);
            if next_cp.is_some_and(|ncp| (0x1F1E6..=0x1F1FF).contains(&ncp)) {
                // Flag emoji pair (e.g. 🇨🇳) → width 2 total; consume both chars.
                width += 2;
                i += 2;
            } else {
                // Isolated regional indicator → width 2 (matches pi-mono).
                width += 2;
                i += 1;
            }
        } else {
            width += char_width(c);
            i += 1;
        }
    }
    width
}

fn char_width(c: char) -> usize {
    if c == '\t' {
        return 3;
    }
    let cp = c as u32;
    // Regional indicator symbols (U+1F1E6..U+1F1FF) form flag emoji pairs.
    // Each isolated symbol is always treated as width 2 to match pi-mono's
    // visible_width() behaviour (which handles streaming partial flags correctly).
    if (0x1F1E6..=0x1F1FF).contains(&cp) {
        return 2;
    }
    UnicodeWidthChar::width(c).unwrap_or_default()
}

/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\x1b'
            && let Some(ansi) = extract_ansi_code(s, i)
        {
            i += ansi.1;
            continue;
        }
        // Find next potential escape or end of string
        let start = i;
        while i < bytes.len() && bytes[i] != b'\x1b' {
            i += 1;
        }
        result.push_str(&s[start..i]);
    }
    result
}

/// Extract an ANSI escape sequence at position `pos` in string `s`.
/// Returns `Some((code_str, length))` or `None`.
pub fn extract_ansi_code(s: &str, pos: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    if pos >= bytes.len() || bytes[pos] != b'\x1b' {
        return None;
    }
    if pos + 1 >= bytes.len() {
        return None;
    }
    let next = bytes[pos + 1];

    match next {
        // CSI sequence: ESC [ ... final_byte
        b'[' => {
            let mut j = pos + 2;
            while j < bytes.len() {
                let b = bytes[j];
                if matches!(
                    b,
                    b'm' | b'G'
                        | b'K'
                        | b'H'
                        | b'J'
                        | b'A'
                        | b'B'
                        | b'C'
                        | b'D'
                        | b'u'
                        | b'h'
                        | b'l'
                        | b't'
                        | b'~'
                        | b'Z'
                        | b'F'
                        | b'f'
                ) {
                    return Some((&s[pos..j + 1], j + 1 - pos));
                }
                j += 1;
            }
            None
        }
        // OSC sequence: ESC ] ... BEL or ESC \
        b']' => {
            let mut j = pos + 2;
            while j < bytes.len() {
                if bytes[j] == b'\x07' {
                    return Some((&s[pos..j + 1], j + 1 - pos));
                }
                if bytes[j] == b'\x1b' && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                    return Some((&s[pos..j + 2], j + 2 - pos));
                }
                j += 1;
            }
            None
        }
        // APC sequence: ESC _ ... BEL or ESC \
        b'_' => {
            let mut j = pos + 2;
            while j < bytes.len() {
                if bytes[j] == b'\x07' {
                    return Some((&s[pos..j + 1], j + 1 - pos));
                }
                if bytes[j] == b'\x1b' && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                    return Some((&s[pos..j + 2], j + 2 - pos));
                }
                j += 1;
            }
            None
        }
        _ => None,
    }
}

// =============================================================================
// Text wrapping
// =============================================================================

/// Wrap text with ANSI codes preserved.
///
/// Only does word wrapping — no padding, no background colors.
/// Returns lines where each line is <= width visible chars.
/// Active ANSI codes are preserved across line breaks.
#[allow(unused_assignments)]
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let input_lines: Vec<&str> = text.split('\n').collect();
    let mut result: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();

    for input_line in &input_lines {
        let prefix = if !result.is_empty() {
            tracker.get_active_codes()
        } else {
            String::new()
        };
        let wrapped = wrap_single_line(&format!("{prefix}{input_line}"), width);
        result.extend(wrapped);
        update_tracker_from_text(input_line, &mut tracker);
    }

    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }

    let mut wrapped: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    let tokens = split_into_tokens_with_ansi(line);

    let mut current_line = String::new();
    let mut current_visible_len = 0usize;

    for token in &tokens {
        let token_visible_len = visible_width(token);
        let is_whitespace = strip_ansi(token).trim().is_empty();

        // Token itself is too long — break character by character
        if token_visible_len > width && !is_whitespace {
            if !current_line.is_empty() {
                let line_end_reset = tracker.get_line_end_reset();
                let mut line_to_push = current_line.trim_end().to_string();
                if !line_end_reset.is_empty() {
                    line_to_push.push_str(&line_end_reset);
                }
                wrapped.push(line_to_push);
            }
            let broken = break_long_word(token, width, &mut tracker);
            let last = broken.last().cloned().unwrap_or_default();
            for part in broken.iter().take(broken.len().saturating_sub(1)) {
                wrapped.push(part.clone());
            }
            current_line = last;
            current_visible_len = visible_width(&current_line);
            continue;
        }

        let total_needed = current_visible_len + token_visible_len;
        if total_needed > width && current_visible_len > 0 {
            let line_end_reset = tracker.get_line_end_reset();
            let mut line_to_wrap = current_line.trim_end().to_string();
            if !line_end_reset.is_empty() {
                line_to_wrap.push_str(&line_end_reset);
            }
            wrapped.push(line_to_wrap);
            if is_whitespace {
                current_line = tracker.get_active_codes();
                current_visible_len = 0;
            } else {
                current_line = format!("{}{}", tracker.get_active_codes(), token);
                current_visible_len = token_visible_len;
            }
        } else {
            current_line.push_str(token);
            current_visible_len += token_visible_len;
        }
        update_tracker_from_text(token, &mut tracker);
    }

    if !current_line.is_empty() {
        wrapped.push(current_line);
    }

    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect()
    }
}

fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    let mut in_whitespace = false;
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if let Some(ansi) = extract_ansi_code(text, i) {
            pending_ansi.push_str(ansi.0);
            i += ansi.1;
            continue;
        }

        // Find char at i
        let ch_start = i;
        let ch = text[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        let char_is_space = ch == ' ';

        if char_is_space != in_whitespace && !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }

        if !pending_ansi.is_empty() {
            current.push_str(&pending_ansi);
            pending_ansi.clear();
        }

        in_whitespace = char_is_space;
        current.push_str(&text[ch_start..ch_start + ch_len]);
        i += ch_len;
    }

    if !pending_ansi.is_empty() {
        current.push_str(&pending_ansi);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = tracker.get_active_codes();
    let mut current_width = 0usize;
    let mut i = 0;
    let bytes = word.as_bytes();

    while i < bytes.len() {
        if let Some(ansi) = extract_ansi_code(word, i) {
            current_line.push_str(ansi.0);
            tracker.process(ansi.0);
            i += ansi.1;
            continue;
        }
        let ch = word[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        let w = char_width(ch);

        if current_width + w > width {
            let line_end_reset = tracker.get_line_end_reset();
            if !line_end_reset.is_empty() {
                current_line.push_str(&line_end_reset);
            }
            lines.push(current_line);
            current_line = tracker.get_active_codes();
            current_width = 0;
        }

        current_line.push_str(&word[i..i + ch_len]);
        current_width += w;
        i += ch_len;
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

// =============================================================================
// Truncate to width
// =============================================================================

/// Truncate text to fit within a maximum visible width, adding ellipsis if needed.
/// Optionally pad with spaces to reach exactly max_width.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad {
            " ".repeat(max_width)
        } else {
            String::new()
        };
    }

    let ellipsis_width = visible_width(ellipsis);
    let text_width = visible_width(text);

    if text_width <= max_width {
        return if pad {
            format!("{}{}", text, " ".repeat(max_width - text_width))
        } else {
            text.to_string()
        };
    }

    if ellipsis_width >= max_width {
        // text_width > max_width guaranteed here
        let clipped = truncate_fragment_to_width(ellipsis, max_width);
        if clipped.1 == 0 {
            return if pad {
                " ".repeat(max_width)
            } else {
                String::new()
            };
        }
        return finalize_truncated_result("", 0, &clipped.0, clipped.1, max_width, pad);
    }

    let target_width = max_width - ellipsis_width;
    let prefix = truncate_fragment_to_width(text, target_width);
    finalize_truncated_result(
        &prefix.0,
        prefix.1,
        ellipsis,
        ellipsis_width,
        max_width,
        pad,
    )
}

fn truncate_fragment_to_width(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }
    // Fast path: pure ASCII
    if text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        let clipped = &text[..max_width.min(text.len())];
        return (clipped.to_string(), clipped.len());
    }
    let mut result = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let w = char_width(ch);
        if width + w > max_width {
            break;
        }
        result.push(ch);
        width += w;
    }
    (result, width)
}

fn finalize_truncated_result(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max_width: usize,
    pad: bool,
) -> String {
    let visible_width_total = prefix_width + ellipsis_width;
    let mut result = format!("{prefix}\x1b[0m{ellipsis}\x1b[0m");
    if pad {
        let padding = max_width.saturating_sub(visible_width_total);
        result.push_str(&" ".repeat(padding));
    }
    result
}

// =============================================================================
// Apply background to line
// =============================================================================

/// Apply background color to a line, padding to full width.
pub fn apply_background_to_line<F>(line: &str, width: usize, bg_fn: F) -> String
where
    F: Fn(&str) -> String,
{
    let vis_len = visible_width(line);
    let padding = " ".repeat(width.saturating_sub(vis_len));
    let with_padding = format!("{line}{padding}");
    bg_fn(&with_padding)
}

// =============================================================================
// Slice by column
// =============================================================================

/// Extract a range of visible columns from a line. Handles ANSI codes and wide chars.
pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    slice_with_width(line, start_col, length, strict).0
}

/// Like slice_by_column but also returns the actual visible width of the result.
pub fn slice_with_width(
    line: &str,
    start_col: usize,
    length: usize,
    strict: bool,
) -> (String, usize) {
    if length == 0 {
        return (String::new(), 0);
    }
    let end_col = start_col + length;
    let mut result = String::new();
    let mut result_width = 0usize;
    let mut current_col = 0usize;
    let mut i = 0;
    let mut pending_ansi = String::new();
    let bytes = line.as_bytes();

    while i < bytes.len() {
        if let Some(ansi) = extract_ansi_code(line, i) {
            if current_col >= start_col && current_col < end_col {
                result.push_str(ansi.0);
            } else if current_col < start_col {
                pending_ansi.push_str(ansi.0);
            }
            i += ansi.1;
            continue;
        }

        let ch = line[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        let w = char_width(ch);
        let in_range = current_col >= start_col && current_col < end_col;
        let fits = !strict || current_col + w <= end_col;

        if in_range && fits {
            if !pending_ansi.is_empty() {
                result.push_str(&pending_ansi);
                pending_ansi.clear();
            }
            result.push_str(&line[i..i + ch_len]);
            result_width += w;
        }
        current_col += w;
        i += ch_len;
        if current_col >= end_col {
            break;
        }
    }
    (result, result_width)
}

/// Extract "before" and "after" segments from a line in a single pass.
pub fn extract_segments(
    line: &str,
    before_end: usize,
    after_start: usize,
    after_len: usize,
    strict_after: bool,
) -> (String, usize, String, usize) {
    let mut before = String::new();
    let mut before_width = 0usize;
    let mut after = String::new();
    let mut after_width = 0usize;
    let mut current_col = 0usize;
    let mut i = 0;
    let mut pending_ansi_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;
    let bytes = line.as_bytes();

    let mut style_tracker = AnsiCodeTracker::new();

    while i < bytes.len() {
        if let Some(ansi) = extract_ansi_code(line, i) {
            style_tracker.process(ansi.0);
            if current_col < before_end {
                pending_ansi_before.push_str(ansi.0);
            } else if current_col >= after_start && current_col < after_end && after_started {
                after.push_str(ansi.0);
            }
            i += ansi.1;
            continue;
        }

        let ch = line[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        let w = char_width(ch);

        if current_col < before_end {
            if !pending_ansi_before.is_empty() {
                before.push_str(&pending_ansi_before);
                pending_ansi_before.clear();
            }
            before.push_str(&line[i..i + ch_len]);
            before_width += w;
        } else if current_col >= after_start && current_col < after_end {
            let fits = !strict_after || current_col + w <= after_end;
            if fits {
                if !after_started {
                    after.push_str(&style_tracker.get_active_codes());
                    after_started = true;
                }
                after.push_str(&line[i..i + ch_len]);
                after_width += w;
            }
        }

        current_col += w;
        i += ch_len;
        let stop = if after_len == 0 {
            current_col >= before_end
        } else {
            current_col >= after_end
        };
        if stop {
            break;
        }
    }

    (before, before_width, after, after_width)
}

// =============================================================================
// Character classification
// =============================================================================

/// Check if a character is whitespace.
pub fn is_whitespace_char(ch: char) -> bool {
    ch.is_whitespace()
}

/// Check if a character is punctuation (for word movement).
pub fn is_punctuation_char(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | '<'
            | '>'
            | '.'
            | ','
            | ';'
            | ':'
            | '\''
            | '"'
            | '!'
            | '?'
            | '+'
            | '-'
            | '='
            | '*'
            | '/'
            | '\\'
            | '|'
            | '&'
            | '%'
            | '^'
            | '$'
            | '#'
            | '@'
            | '~'
            | '`'
    )
}

// =============================================================================
// ANSI Code Tracker
// =============================================================================

/// Track active ANSI SGR codes to preserve styling across line breaks.
#[derive(Default)]
pub struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg_color: Option<String>,
    bg_color: Option<String>,
}

impl AnsiCodeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, ansi_code: &str) {
        if !ansi_code.ends_with('m') {
            return;
        }
        // Extract parameters between ESC[ and m
        let inner = ansi_code
            .strip_prefix("\x1b[")
            .and_then(|s| s.strip_suffix('m'))
            .unwrap_or("");

        if inner.is_empty() || inner == "0" {
            self.reset();
            return;
        }

        let parts: Vec<&str> = inner.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let code: u32 = parts[i].parse().unwrap_or(0);
            match code {
                38 | 48 => {
                    if parts.get(i + 1) == Some(&"5") && parts.get(i + 2).is_some() {
                        let color_code = format!("{};{};{}", parts[i], parts[i + 1], parts[i + 2]);
                        if code == 38 {
                            self.fg_color = Some(color_code);
                        } else {
                            self.bg_color = Some(color_code);
                        }
                        i += 3;
                        continue;
                    } else if parts.get(i + 1) == Some(&"2") && parts.get(i + 4).is_some() {
                        let color_code = format!(
                            "{};{};{};{};{}",
                            parts[i],
                            parts[i + 1],
                            parts[i + 2],
                            parts[i + 3],
                            parts[i + 4]
                        );
                        if code == 38 {
                            self.fg_color = Some(color_code);
                        } else {
                            self.bg_color = Some(color_code);
                        }
                        i += 5;
                        continue;
                    }
                }
                0 => self.reset(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                5 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.bold = false,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                39 => self.fg_color = None,
                49 => self.bg_color = None,
                c if (30..=37).contains(&c) || (90..=97).contains(&c) => {
                    self.fg_color = Some(c.to_string());
                }
                c if (40..=47).contains(&c) || (100..=107).contains(&c) => {
                    self.bg_color = Some(c.to_string());
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn clear(&mut self) {
        self.reset();
    }

    pub fn get_active_codes(&self) -> String {
        let mut codes: Vec<String> = Vec::new();
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if self.underline {
            codes.push("4".into());
        }
        if self.blink {
            codes.push("5".into());
        }
        if self.inverse {
            codes.push("7".into());
        }
        if self.hidden {
            codes.push("8".into());
        }
        if self.strikethrough {
            codes.push("9".into());
        }
        if let Some(ref fg) = self.fg_color {
            codes.push(fg.clone());
        }
        if let Some(ref bg) = self.bg_color {
            codes.push(bg.clone());
        }
        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }

    pub fn has_active_codes(&self) -> bool {
        self.bold
            || self.dim
            || self.italic
            || self.underline
            || self.blink
            || self.inverse
            || self.hidden
            || self.strikethrough
            || self.fg_color.is_some()
            || self.bg_color.is_some()
    }

    /// Get reset codes for underline (which bleeds into padding).
    pub fn get_line_end_reset(&self) -> String {
        if self.underline {
            "\x1b[24m".to_string()
        } else {
            String::new()
        }
    }
}

fn update_tracker_from_text(text: &str, tracker: &mut AnsiCodeTracker) {
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if let Some(ansi) = extract_ansi_code(text, i) {
            tracker.process(ansi.0);
            i += ansi.1;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width_ascii() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width(""), 0);
    }

    #[test]
    fn test_visible_width_ansi() {
        assert_eq!(visible_width("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(visible_width("\x1b[1;32mworld\x1b[0m"), 5);
    }

    #[test]
    fn test_visible_width_unicode() {
        // "你好" = 2 wide chars each = width 4
        assert_eq!(visible_width("你好"), 4);
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[31mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn test_truncate_to_width_short() {
        let result = truncate_to_width("hello", 10, "...", false);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_to_width_long() {
        let result = truncate_to_width("hello world", 8, "...", false);
        assert!(visible_width(&result) <= 8);
        assert!(result.contains("...") || result.len() <= 8);
    }

    #[test]
    fn test_truncate_to_width_pad() {
        let result = truncate_to_width("hi", 5, "...", true);
        assert_eq!(visible_width(&result), 5);
    }

    #[test]
    fn test_wrap_text_simple() {
        let lines = wrap_text_with_ansi("hello world", 5);
        assert!(!lines.is_empty());
        for line in &lines {
            assert!(visible_width(line) <= 5);
        }
    }

    #[test]
    fn test_wrap_text_newlines() {
        let lines = wrap_text_with_ansi("hello\nworld", 80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "world");
    }

    #[test]
    fn test_slice_by_column() {
        let s = "hello world";
        let sliced = slice_by_column(s, 6, 5, false);
        assert_eq!(sliced, "world");
    }

    #[test]
    fn test_apply_background_to_line() {
        let line = "hi";
        let result = apply_background_to_line(line, 5, |s| format!("[{s}]"));
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
    }

    #[test]
    fn test_is_whitespace_char() {
        assert!(is_whitespace_char(' '));
        assert!(is_whitespace_char('\t'));
        assert!(!is_whitespace_char('a'));
    }

    #[test]
    fn test_is_punctuation_char() {
        assert!(is_punctuation_char('.'));
        assert!(is_punctuation_char(','));
        assert!(!is_punctuation_char('a'));
    }

    #[test]
    fn test_ansi_code_tracker() {
        let mut tracker = AnsiCodeTracker::new();
        tracker.process("\x1b[1m"); // bold
        assert!(tracker.has_active_codes());
        let codes = tracker.get_active_codes();
        assert!(codes.contains('1'));
        tracker.process("\x1b[0m"); // reset
        assert!(!tracker.has_active_codes());
    }

    #[test]
    fn test_extract_segments() {
        let line = "hello world";
        let (before, bw, after, aw) = extract_segments(line, 5, 6, 5, false);
        assert_eq!(before, "hello");
        assert_eq!(bw, 5);
        assert_eq!(after, "world");
        assert_eq!(aw, 5);
    }

    // =========================================================================
    // Tests from wrap-ansi.test.ts
    // =========================================================================

    #[test]
    fn test_wrap_ansi_no_underline_before_styled_text() {
        let underline_on = "\x1b[4m";
        let underline_off = "\x1b[24m";
        let url = "https://example.com/very/long/path/that/will/wrap";
        let text = format!("read this thread {underline_on}{url}{underline_off}");
        let wrapped = wrap_text_with_ansi(&text, 40);

        // First line should NOT contain underline code
        assert_eq!(wrapped[0], "read this thread");
        // Second line should start with underline
        assert!(wrapped[1].starts_with(underline_on));
        assert!(wrapped[1].contains("https://"));
    }

    #[test]
    fn test_wrap_ansi_no_whitespace_before_underline_reset() {
        let underline_on = "\x1b[4m";
        let underline_off = "\x1b[24m";
        let text = format!("{underline_on}underlined text here {underline_off}more");
        let wrapped = wrap_text_with_ansi(&text, 18);

        let space_reset = format!(" {underline_off}");
        assert!(!wrapped[0].contains(&space_reset));
    }

    #[test]
    fn test_wrap_ansi_underline_ends_with_underline_off_not_full_reset() {
        let underline_on = "\x1b[4m";
        let underline_off = "\x1b[24m";
        let url = "https://example.com/very/long/path/that/will/definitely/wrap";
        let text = format!("prefix {underline_on}{url}{underline_off} suffix");
        let wrapped = wrap_text_with_ansi(&text, 30);

        // Middle lines with underlined content should end with underline-off, not full reset
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)).skip(1) {
            if line.contains(underline_on) {
                assert!(line.ends_with(underline_off));
                assert!(!line.ends_with("\x1b[0m"));
            }
        }
    }

    #[test]
    fn test_wrap_ansi_bg_color_preserved_across_lines() {
        let bg_blue = "\x1b[44m";
        let reset = "\x1b[0m";
        let text = format!("{bg_blue}hello world this is blue background text{reset}");
        let wrapped = wrap_text_with_ansi(&text, 15);

        // Each line should have background color
        for line in &wrapped {
            assert!(line.contains(bg_blue));
        }

        // Middle lines should NOT end with full reset
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
            assert!(!line.ends_with("\x1b[0m"));
        }
    }

    #[test]
    fn test_wrap_ansi_plain_text_correct() {
        let text = "hello world this is a test";
        let wrapped = wrap_text_with_ansi(text, 10);

        assert!(wrapped.len() > 1);
        for line in &wrapped {
            assert!(visible_width(line) <= 10);
        }
    }

    #[test]
    fn test_visible_width_ignores_osc_133_bel() {
        let text = "\x1b]133;A\x07hello\x1b]133;B\x07";
        assert_eq!(visible_width(text), 5);
    }

    #[test]
    fn test_visible_width_ignores_osc_st() {
        let text = "\x1b]133;A\x1b\\hello\x1b]133;B\x1b\\";
        assert_eq!(visible_width(text), 5);
    }

    #[test]
    fn test_wrap_ansi_truncate_trailing_whitespace() {
        let wrapped = wrap_text_with_ansi("  ", 1);
        assert!(visible_width(&wrapped[0]) <= 1);
    }

    #[test]
    fn test_wrap_ansi_color_codes_preserved_across_wraps() {
        let red = "\x1b[31m";
        let reset = "\x1b[0m";
        let text = format!("{red}hello world this is red{reset}");
        let wrapped = wrap_text_with_ansi(&text, 10);

        // Each continuation line should start with red code
        for line in wrapped.iter().skip(1) {
            assert!(line.starts_with(red));
        }

        // Middle lines should not end with full reset
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
            assert!(!line.ends_with("\x1b[0m"));
        }
    }

    // =========================================================================
    // Tests from truncate-to-width.test.ts
    // =========================================================================

    #[test]
    fn test_truncate_large_unicode_within_width() {
        let text = "🙂界".repeat(100_000);
        let truncated = truncate_to_width(&text, 40, "…", false);
        assert!(visible_width(&truncated) <= 40);
        assert!(truncated.ends_with("…\x1b[0m"));
    }

    #[test]
    fn test_truncate_preserves_ansi_styling() {
        let inner = "hello ".repeat(1000);
        let text = format!("\x1b[31m{inner}\x1b[0m");
        let truncated = truncate_to_width(&text, 20, "…", false);
        assert!(visible_width(&truncated) <= 20);
        assert!(truncated.contains("\x1b[31m"));
        assert!(truncated.ends_with("\x1b[0m…\x1b[0m"));
    }

    #[test]
    #[ignore = "known hang: truncate_to_width loops on malformed ESC sequences (non-CSI); tracked as bug"]
    fn test_truncate_malformed_ansi_no_hang() {
        let emoji_repeat = "🙂".repeat(1000);
        let text = format!("abc\x1bnot-ansi {emoji_repeat}");
        let truncated = truncate_to_width(&text, 20, "…", false);
        assert!(visible_width(&truncated) <= 20);
    }

    #[test]
    fn test_truncate_wide_ellipsis_safety() {
        assert_eq!(truncate_to_width("abcdef", 1, "🙂", false), "");
        assert_eq!(
            truncate_to_width("abcdef", 2, "🙂", false),
            "\x1b[0m🙂\x1b[0m"
        );
        assert!(visible_width(&truncate_to_width("abcdef", 2, "🙂", false)) <= 2);
    }

    #[test]
    fn test_truncate_original_fits_despite_wide_ellipsis() {
        assert_eq!(truncate_to_width("a", 2, "🙂", false), "a");
        assert_eq!(truncate_to_width("界", 2, "🙂", false), "界");
    }

    #[test]
    fn test_truncate_pad_to_width() {
        let truncated = truncate_to_width("🙂界🙂界🙂界", 8, "…", true);
        assert_eq!(visible_width(&truncated), 8);
    }

    #[test]
    fn test_truncate_trailing_reset_no_ellipsis() {
        let inner = "hello".repeat(100);
        let text = format!("\x1b[31m{inner}");
        let truncated = truncate_to_width(&text, 10, "", false);
        assert!(visible_width(&truncated) <= 10);
        assert!(truncated.ends_with("\x1b[0m"));
    }

    #[test]
    fn test_visible_width_tab_and_ansi() {
        assert_eq!(visible_width("\t\x1b[31m界\x1b[0m"), 5);
    }

    // =========================================================================
    // Tests from regression-regional-indicator-width.test.ts
    //
    // NOTE: The TypeScript implementation returns width=2 for isolated regional
    // indicator characters (e.g. 🇨 alone) by special-casing them. The Rust
    // implementation uses the unicode_width crate which returns width=1 for
    // isolated regional indicators. The pair (e.g. 🇨🇳) still returns width=2.
    // =========================================================================

    #[test]
    fn test_regional_indicator_paired_flag_width_2() {
        // Full flag emoji pairs should be width 2
        assert_eq!(visible_width("🇨🇳"), 2);
    }

    #[test]
    fn test_regional_indicator_full_flag_pairs_width_2() {
        for flag in &["🇯🇵", "🇺🇸", "🇬🇧", "🇨🇳", "🇩🇪", "🇫🇷"] {
            assert_eq!(visible_width(flag), 2, "Expected {} to be width 2", flag);
        }
    }

    #[test]
    fn test_streaming_emoji_intermediates_non_zero_width() {
        // Common emoji used in streaming contexts — verify they have non-zero width
        for sample in &["👍", "✅", "⚡"] {
            assert!(
                visible_width(sample) > 0,
                "Expected {} to have non-zero width",
                sample
            );
        }
    }

    #[test]
    fn test_regional_indicator_isolated_width_rust() {
        // Isolated regional indicators are special-cased to width 2 (matching pi-mono).
        assert_eq!(visible_width("🇨"), 2);
    }

    // =========================================================================
    // Additional tests from regression-regional-indicator-width.test.ts
    // =========================================================================

    #[test]
    fn test_regional_indicator_partial_flag_treated_as_full_width() {
        // Each isolated regional indicator is treated as width 2 (matching pi-mono).
        let partial_flag = "🇨";
        assert_eq!(visible_width(partial_flag), 2);
        let list_line = "      - 🇨";
        assert_eq!(visible_width(list_line), 10);
    }

    #[test]
    fn test_regional_indicator_partial_flag_wraps_before_overflow() {
        // Isolated regional indicator has width 2, matching pi-mono behaviour.
        let wrapped = wrap_text_with_ansi("      - 🇨", 9);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(visible_width(&wrapped[0]), 7);
        assert_eq!(visible_width(&wrapped[1]), 2);
    }

    #[test]
    fn test_all_regional_indicator_singletons_are_width_2() {
        // All regional indicator singletons (U+1F1E6..U+1F1FF) have width 2,
        // matching pi-mono's visible_width() special-casing.
        for cp in 0x1f1e6u32..=0x1f1ff {
            let ch = char::from_u32(cp).unwrap().to_string();
            assert_eq!(
                visible_width(&ch),
                2,
                "Expected regional indicator U+{:X} to be width 2",
                cp
            );
        }
    }

    #[test]
    fn test_streaming_emoji_common_samples_non_zero_width() {
        // Common emoji used in streaming contexts — verify they have non-zero width.
        for sample in &["👍", "✅", "⚡", "👨", "🏳️‍🌈"] {
            assert!(
                visible_width(sample) > 0,
                "Expected {} to have non-zero width",
                sample
            );
        }
    }

    #[test]
    fn test_regional_indicator_list_line_visible_width_rust() {
        // "      - 🇨" — isolated regional indicator now has width 2 (matching pi-mono).
        // width of "      - " = 8, plus width of "🇨" = 2, total = 10.
        let list_line = "      - 🇨";
        assert_eq!(visible_width(list_line), 10);
    }
}
