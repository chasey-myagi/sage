//! Shared truncation utilities for tool outputs.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/truncate.ts`.
//!
//! Truncation is based on two independent limits — whichever is hit first wins:
//! - Line limit (default: 2000 lines)
//! - Byte limit (default: 50 KB)
//!
//! Never returns partial lines (except bash tail-truncation edge case).

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024; // 50 KB
/// Maximum characters per grep match line before the line is truncated.
pub const GREP_MAX_LINE_LENGTH: usize = 500;

// ============================================================================
// TruncationResult
// ============================================================================

#[derive(Debug, Clone)]
pub struct TruncationResult {
    /// The (possibly truncated) content.
    pub content: String,
    /// Whether truncation occurred.
    pub truncated: bool,
    /// Which limit was hit, or `None` when not truncated.
    pub truncated_by: Option<TruncatedBy>,
    /// Total lines in the original content.
    pub total_lines: usize,
    /// Total bytes in the original content.
    pub total_bytes: usize,
    /// Complete lines in the truncated output.
    pub output_lines: usize,
    /// Bytes in the truncated output.
    pub output_bytes: usize,
    /// Whether the last line was partially truncated (tail-truncation edge case).
    pub last_line_partial: bool,
    /// Whether the first line alone exceeds the byte limit (head-truncation).
    pub first_line_exceeds_limit: bool,
    /// The max-lines limit that was applied.
    pub max_lines: usize,
    /// The max-bytes limit that was applied.
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

// ============================================================================
// TruncationOptions
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct TruncationOptions {
    /// Maximum number of lines (default: 2000).
    pub max_lines: Option<usize>,
    /// Maximum number of bytes (default: 50 KB).
    pub max_bytes: Option<usize>,
}

// ============================================================================
// format_size
// ============================================================================

/// Format bytes as a human-readable size string.
///
/// Mirrors `formatSize()` from `truncate.ts`.
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ============================================================================
// truncate_head
// ============================================================================

/// Truncate content from the **head** (keep the first N lines/bytes).
///
/// Suitable for file reads where you want to see the beginning.
///
/// Never returns partial lines. When the first line alone exceeds the byte
/// limit, returns empty content with `first_line_exceeds_limit = true`.
///
/// Mirrors `truncateHead()` from `truncate.ts`.
pub fn truncate_head(content: &str, options: TruncationOptions) -> TruncationResult {
    let max_lines = options.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);

    let total_bytes = content.len(); // UTF-8 bytes
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    // Fast path: no truncation needed.
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_string(),
            truncated: false,
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines,
            max_bytes,
        };
    }

    // Check if the first line alone exceeds the byte limit.
    let first_line_bytes = lines[0].len();
    if first_line_bytes > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }

    // Collect complete lines that fit within both limits.
    let mut output_lines_arr: Vec<&str> = Vec::new();
    let mut output_bytes_count: usize = 0;
    let mut truncated_by = TruncatedBy::Lines;

    for (i, &line) in lines.iter().enumerate() {
        if i >= max_lines {
            break;
        }
        // +1 for the newline separator (not charged for the very first line).
        let line_bytes = line.len() + if i > 0 { 1 } else { 0 };
        if output_bytes_count + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        output_lines_arr.push(line);
        output_bytes_count += line_bytes;
    }

    // Re-evaluate: was it a line limit or byte limit that caused truncation?
    if output_lines_arr.len() >= max_lines && output_bytes_count <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    let output_content = output_lines_arr.join("\n");
    let final_output_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: output_lines_arr.len(),
        output_bytes: final_output_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

// ============================================================================
// truncate_tail
// ============================================================================

/// Truncate content from the **tail** (keep the last N lines/bytes).
///
/// Suitable for bash output where you want to see the end (errors, results).
///
/// May return a partial first line when the last line of the original content
/// exceeds the byte limit (edge case).
///
/// Mirrors `truncateTail()` from `truncate.ts`.
pub fn truncate_tail(content: &str, options: TruncationOptions) -> TruncationResult {
    let max_lines = options.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);

    let total_bytes = content.len();
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    // Fast path: no truncation needed.
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_string(),
            truncated: false,
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines,
            max_bytes,
        };
    }

    // Work backwards from the end.
    let mut output_lines_arr: Vec<&str> = Vec::new();
    let mut output_bytes_count: usize = 0;
    let mut truncated_by = TruncatedBy::Lines;
    let mut last_line_partial = false;

    for i in (0..lines.len()).rev() {
        if output_lines_arr.len() >= max_lines {
            break;
        }
        let line = lines[i];
        // +1 for newline (not for the first element we add).
        let line_bytes = line.len() + if !output_lines_arr.is_empty() { 1 } else { 0 };

        if output_bytes_count + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            // Edge case: nothing added yet and this single line exceeds limit.
            // Take the tail of the line (partial).
            if output_lines_arr.is_empty() {
                let partial = truncate_string_to_bytes_from_end(line, max_bytes);
                output_bytes_count = partial.len();
                // We need to return an owned String, so collect into a Vec<String> is needed.
                // Defer ownership handling after the loop.
                let _ = partial; // placeholder, handled below
                last_line_partial = true;
            }
            break;
        }

        output_lines_arr.insert(0, line);
        output_bytes_count += line_bytes;
    }

    // Handle the edge-case partial last line separately.
    if last_line_partial {
        // Find the last line and take its tail.
        if let Some(&last_line) = lines.last() {
            let partial = truncate_string_to_bytes_from_end(last_line, max_bytes);
            let output_bytes = partial.len();
            return TruncationResult {
                content: partial,
                truncated: true,
                truncated_by: Some(TruncatedBy::Bytes),
                total_lines,
                total_bytes,
                output_lines: 1,
                output_bytes,
                last_line_partial: true,
                first_line_exceeds_limit: false,
                max_lines,
                max_bytes,
            };
        }
    }

    // Re-evaluate line vs byte limit.
    if output_lines_arr.len() >= max_lines && output_bytes_count <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    let output_content = output_lines_arr.join("\n");
    let final_output_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: output_lines_arr.len(),
        output_bytes: final_output_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

// ============================================================================
// truncate_string_to_bytes_from_end
// ============================================================================

/// Truncate a string to fit within `max_bytes` bytes (keeping the tail).
///
/// Handles multi-byte UTF-8 characters correctly.
fn truncate_string_to_bytes_from_end(s: &str, max_bytes: usize) -> String {
    let bytes = s.as_bytes();
    if bytes.len() <= max_bytes {
        return s.to_string();
    }
    // Start from the end, going back max_bytes.
    let mut start = bytes.len() - max_bytes;
    // Advance to a valid UTF-8 character boundary.
    while start < bytes.len() && (bytes[start] & 0xc0) == 0x80 {
        start += 1;
    }
    s[start..].to_string()
}

// ============================================================================
// truncate_line
// ============================================================================

/// Truncate a single line to `max_chars` characters, appending `"... [truncated]"`.
///
/// Used for grep match lines. Mirrors `truncateLine()` from `truncate.ts`.
pub fn truncate_line(line: &str, max_chars: Option<usize>) -> (String, bool) {
    let max = max_chars.unwrap_or(GREP_MAX_LINE_LENGTH);
    let char_count = line.chars().count();
    if char_count <= max {
        (line.to_string(), false)
    } else {
        let truncated: String = line.chars().take(max).collect();
        (format!("{truncated}... [truncated]"), true)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- format_size ----

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(500), "500B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(51200), "50.0KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0MB");
    }

    // ---- truncate_head: no truncation ----

    #[test]
    fn head_no_truncation_small_content() {
        let content = "line1\nline2\nline3";
        let result = truncate_head(content, TruncationOptions::default());
        assert!(!result.truncated);
        assert_eq!(result.content, content);
        assert_eq!(result.output_lines, 3);
        assert_eq!(result.total_lines, 3);
    }

    #[test]
    fn head_no_truncation_empty() {
        let result = truncate_head("", TruncationOptions::default());
        assert!(!result.truncated);
        assert_eq!(result.content, "");
        assert_eq!(result.total_lines, 1);
    }

    // ---- truncate_head: line limit ----

    #[test]
    fn head_truncates_by_lines() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let content = lines.join("\n");
        let result = truncate_head(
            &content,
            TruncationOptions {
                max_lines: Some(5),
                max_bytes: None,
            },
        );
        assert!(result.truncated);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(result.output_lines, 5);
        assert!(result.content.starts_with("line0"));
        assert!(!result.content.contains("line5"));
    }

    // ---- truncate_head: byte limit ----

    #[test]
    fn head_truncates_by_bytes() {
        // 5 lines of 20 chars each = 100+ bytes; limit to 50 bytes
        let content = "12345678901234567890\n12345678901234567890\n12345678901234567890\n12345678901234567890\n12345678901234567890";
        let result = truncate_head(
            content,
            TruncationOptions {
                max_lines: None,
                max_bytes: Some(50),
            },
        );
        assert!(result.truncated);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
        assert!(result.output_bytes <= 50);
    }

    // ---- truncate_head: first line exceeds limit ----

    #[test]
    fn head_first_line_exceeds_limit() {
        let content = "a".repeat(100) + "\nline2";
        let result = truncate_head(
            &content,
            TruncationOptions {
                max_lines: None,
                max_bytes: Some(50),
            },
        );
        assert!(result.truncated);
        assert!(result.first_line_exceeds_limit);
        assert_eq!(result.content, "");
        assert_eq!(result.output_lines, 0);
    }

    // ---- truncate_tail: no truncation ----

    #[test]
    fn tail_no_truncation_small_content() {
        let content = "line1\nline2\nline3";
        let result = truncate_tail(content, TruncationOptions::default());
        assert!(!result.truncated);
        assert_eq!(result.content, content);
    }

    // ---- truncate_tail: line limit ----

    #[test]
    fn tail_truncates_by_lines_keeps_end() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let content = lines.join("\n");
        let result = truncate_tail(
            &content,
            TruncationOptions {
                max_lines: Some(3),
                max_bytes: None,
            },
        );
        assert!(result.truncated);
        // Should keep the LAST 3 lines
        assert_eq!(result.output_lines, 3);
        assert!(result.content.contains("line9"));
        assert!(result.content.contains("line8"));
        assert!(result.content.contains("line7"));
        assert!(!result.content.contains("line0"));
    }

    // ---- truncate_tail: byte limit ----

    #[test]
    fn tail_truncates_by_bytes() {
        let content = "12345678901234567890\n12345678901234567890\n12345678901234567890\n12345678901234567890\n12345678901234567890";
        let result = truncate_tail(
            content,
            TruncationOptions {
                max_lines: None,
                max_bytes: Some(50),
            },
        );
        assert!(result.truncated);
        assert!(result.output_bytes <= 50);
        // Should keep the tail (last lines)
        assert!(result.content.contains("12345678901234567890"));
    }

    // ---- truncate_line ----

    #[test]
    fn line_no_truncation_short() {
        let (text, was_truncated) = truncate_line("hello world", None);
        assert_eq!(text, "hello world");
        assert!(!was_truncated);
    }

    #[test]
    fn line_truncation_long_line() {
        let long_line = "x".repeat(600);
        let (text, was_truncated) = truncate_line(&long_line, None);
        assert!(was_truncated);
        assert!(text.ends_with("... [truncated]"));
        assert!(text.len() < long_line.len() + 20); // reasonably shorter
    }

    #[test]
    fn line_truncation_custom_max() {
        let (text, was_truncated) = truncate_line("hello world", Some(5));
        assert!(was_truncated);
        assert!(text.starts_with("hello"));
    }

    // ---- constants ----

    #[test]
    fn constants_values() {
        assert_eq!(DEFAULT_MAX_LINES, 2000);
        assert_eq!(DEFAULT_MAX_BYTES, 51200);
        assert_eq!(GREP_MAX_LINE_LENGTH, 500);
    }
}
