// Truncation engine — shared by all tools that handle large text output.

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;
pub const GREP_MAX_LINE_LENGTH: usize = 500;

/// Result of a truncation operation.
pub struct TruncationResult {
    pub content: String,
    pub truncated: bool,
    pub original_lines: usize,
    pub original_bytes: usize,
    pub kept_lines: usize,
}

/// Split input into line segments, each including its trailing `\n` if present.
fn line_segments(input: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (i, byte) in input.bytes().enumerate() {
        if byte == b'\n' {
            segments.push(&input[start..=i]);
            start = i + 1;
        }
    }
    if start < input.len() {
        segments.push(&input[start..]);
    }
    segments
}

/// Keep the first `max_lines` lines within `max_bytes`.
pub fn truncate_head(input: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    if input.is_empty() {
        return TruncationResult {
            content: String::new(),
            truncated: false,
            original_lines: 0,
            original_bytes: 0,
            kept_lines: 0,
        };
    }

    let original_bytes = input.len();
    let original_lines = input.lines().count();
    let segments = line_segments(input);

    let mut content = String::new();
    let mut kept_lines = 0;

    for segment in &segments {
        if kept_lines >= max_lines {
            break;
        }
        if content.len() + segment.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(content.len());
            if remaining > 0 {
                let safe_end = (0..=remaining.min(segment.len()))
                    .rev()
                    .find(|&i| segment.is_char_boundary(i))
                    .unwrap_or(0);
                if safe_end > 0 {
                    content.push_str(&segment[..safe_end]);
                    kept_lines += 1;
                }
            }
            break;
        }
        content.push_str(segment);
        kept_lines += 1;
    }

    let truncated = kept_lines < original_lines || content.len() < original_bytes;

    TruncationResult {
        content,
        truncated,
        original_lines,
        original_bytes,
        kept_lines,
    }
}

/// Keep the last `max_lines` lines within `max_bytes`.
pub fn truncate_tail(input: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    if input.is_empty() {
        return TruncationResult {
            content: String::new(),
            truncated: false,
            original_lines: 0,
            original_bytes: 0,
            kept_lines: 0,
        };
    }

    let original_bytes = input.len();
    let original_lines = input.lines().count();
    let segments = line_segments(input);

    let mut content = String::new();
    let mut kept_lines = 0;

    for segment in segments.iter().rev() {
        if kept_lines >= max_lines {
            break;
        }
        if content.len() + segment.len() > max_bytes {
            break;
        }
        content.insert_str(0, segment);
        kept_lines += 1;
    }

    let truncated = kept_lines < original_lines || content.len() < original_bytes;

    TruncationResult {
        content,
        truncated,
        original_lines,
        original_bytes,
        kept_lines,
    }
}

/// Truncate a single line to `max_chars` characters, appending "..." if truncated.
pub fn truncate_line(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let truncated: String = input.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    #[test]
    fn test_default_max_lines_is_2000() {
        assert_eq!(DEFAULT_MAX_LINES, 2000);
    }

    #[test]
    fn test_default_max_bytes_is_50kb() {
        assert_eq!(DEFAULT_MAX_BYTES, 50 * 1024);
    }

    #[test]
    fn test_grep_max_line_length_is_500() {
        assert_eq!(GREP_MAX_LINE_LENGTH, 500);
    }

    // ---------------------------------------------------------------
    // truncate_head — keeps the first N lines / bytes
    // ---------------------------------------------------------------

    #[test]
    fn test_head_empty_string() {
        let r = truncate_head("", 100, 1024);
        assert_eq!(r.content, "");
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 0);
        assert_eq!(r.original_bytes, 0);
        assert_eq!(r.kept_lines, 0);
    }

    #[test]
    fn test_head_single_line_no_truncation() {
        let r = truncate_head("hello world", 10, 1024);
        assert_eq!(r.content, "hello world");
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 1);
        assert_eq!(r.kept_lines, 1);
    }

    #[test]
    fn test_head_no_truncation_needed() {
        let input = "line1\nline2\nline3\n";
        let r = truncate_head(input, 100, 10_000);
        assert_eq!(r.content, input);
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 3);
        assert_eq!(r.kept_lines, 3);
        assert_eq!(r.original_bytes, input.len());
    }

    #[test]
    fn test_head_truncate_by_lines() {
        let input = "a\nb\nc\nd\ne\n";
        let r = truncate_head(input, 3, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 3);
        assert_eq!(r.original_lines, 5);
        // Content should contain first 3 lines
        assert!(r.content.starts_with("a\nb\nc"));
    }

    #[test]
    fn test_head_truncate_by_bytes() {
        let input = "aaaa\nbbbb\ncccc\ndddd\n";
        // 5 bytes per line ("xxxx\n"), total 20 bytes. Limit to 10 bytes.
        let r = truncate_head(input, 1000, 10);
        assert!(r.truncated);
        assert_eq!(r.original_bytes, 20);
        assert!(r.content.len() <= 10);
    }

    #[test]
    fn test_head_lines_limit_triggers_before_bytes() {
        // 3 short lines, limit = 2 lines but generous byte budget
        let input = "a\nb\nc\n";
        let r = truncate_head(input, 2, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
    }

    #[test]
    fn test_head_bytes_limit_triggers_before_lines() {
        // 3 lines, generous line budget but tight byte budget
        let input = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n";
        let r = truncate_head(input, 1000, 15);
        assert!(r.truncated);
        assert!(r.content.len() <= 15);
    }

    #[test]
    fn test_head_exact_line_boundary() {
        let input = "a\nb\nc\n";
        let r = truncate_head(input, 3, 10_000);
        assert!(!r.truncated);
        assert_eq!(r.kept_lines, 3);
    }

    #[test]
    fn test_head_exact_byte_boundary() {
        let input = "abc"; // 3 bytes
        let r = truncate_head(input, 1000, 3);
        assert!(!r.truncated);
        assert_eq!(r.content, "abc");
    }

    #[test]
    fn test_head_unicode_multibyte_does_not_split_char() {
        // "你好\n世界\n" — each CJK char is 3 bytes
        let input = "你好\n世界\n";
        let byte_limit = 7; // just enough for "你好\n" (6+1=7)
        let r = truncate_head(input, 1000, byte_limit);
        // Must not split a multi-byte character
        assert!(r.content.is_char_boundary(r.content.len()));
        // The content should be valid UTF-8 (Rust String guarantees this)
        assert!(r.truncated || r.content == input);
    }

    #[test]
    fn test_head_trailing_newline_counted_as_line() {
        // "a\nb\n" has 2 lines (trailing newline), not 3
        let input = "a\nb\n";
        let r = truncate_head(input, 100, 10_000);
        assert_eq!(r.original_lines, 2);
    }

    #[test]
    fn test_head_no_trailing_newline() {
        let input = "a\nb\nc";
        let r = truncate_head(input, 100, 10_000);
        assert_eq!(r.original_lines, 3);
        assert_eq!(r.kept_lines, 3);
    }

    #[test]
    fn test_head_one_line_limit() {
        let input = "first\nsecond\nthird\n";
        let r = truncate_head(input, 1, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 1);
        assert!(r.content.starts_with("first"));
    }

    #[test]
    fn test_head_preserves_original_bytes() {
        let input = "hello\nworld\n";
        let r = truncate_head(input, 1, 10_000);
        assert_eq!(r.original_bytes, input.len());
    }

    // ---------------------------------------------------------------
    // truncate_tail — keeps the last N lines / bytes
    // ---------------------------------------------------------------

    #[test]
    fn test_tail_empty_string() {
        let r = truncate_tail("", 100, 1024);
        assert_eq!(r.content, "");
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 0);
        assert_eq!(r.original_bytes, 0);
        assert_eq!(r.kept_lines, 0);
    }

    #[test]
    fn test_tail_single_line_no_truncation() {
        let r = truncate_tail("hello", 10, 1024);
        assert_eq!(r.content, "hello");
        assert!(!r.truncated);
    }

    #[test]
    fn test_tail_no_truncation_needed() {
        let input = "a\nb\nc\n";
        let r = truncate_tail(input, 100, 10_000);
        assert_eq!(r.content, input);
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 3);
        assert_eq!(r.kept_lines, 3);
    }

    #[test]
    fn test_tail_truncate_by_lines() {
        let input = "a\nb\nc\nd\ne\n";
        let r = truncate_tail(input, 2, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
        assert_eq!(r.original_lines, 5);
        // Content should contain the last 2 lines
        assert!(r.content.contains("d"));
        assert!(r.content.contains("e"));
    }

    #[test]
    fn test_tail_truncate_by_bytes() {
        let input = "aaaa\nbbbb\ncccc\ndddd\n";
        let r = truncate_tail(input, 1000, 10);
        assert!(r.truncated);
        assert!(r.content.len() <= 10);
        // Should contain tail content
        assert!(r.content.contains("dddd") || r.content.contains("cccc"));
    }

    #[test]
    fn test_tail_lines_limit_triggers_before_bytes() {
        let input = "a\nb\nc\nd\n";
        let r = truncate_tail(input, 2, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
        assert!(r.content.contains("c"));
        assert!(r.content.contains("d"));
    }

    #[test]
    fn test_tail_bytes_limit_triggers_before_lines() {
        let input = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n";
        let r = truncate_tail(input, 1000, 15);
        assert!(r.truncated);
        assert!(r.content.len() <= 15);
    }

    #[test]
    fn test_tail_exact_line_boundary() {
        let input = "a\nb\nc\n";
        let r = truncate_tail(input, 3, 10_000);
        assert!(!r.truncated);
        assert_eq!(r.kept_lines, 3);
    }

    #[test]
    fn test_tail_unicode_multibyte_does_not_split_char() {
        let input = "你好\n世界\n再见\n";
        let byte_limit = 10;
        let r = truncate_tail(input, 1000, byte_limit);
        // Must produce valid UTF-8 — Rust enforces this by String type
        assert!(r.content.is_char_boundary(r.content.len()));
    }

    #[test]
    fn test_tail_one_line_limit() {
        let input = "first\nsecond\nthird\n";
        let r = truncate_tail(input, 1, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 1);
        assert!(r.content.contains("third"));
    }

    #[test]
    fn test_tail_no_trailing_newline() {
        let input = "a\nb\nc";
        let r = truncate_tail(input, 2, 10_000);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
        assert!(r.content.contains("b"));
        assert!(r.content.contains("c"));
    }

    #[test]
    fn test_tail_preserves_original_lines() {
        let input = "line1\nline2\nline3\nline4\nline5\n";
        let r = truncate_tail(input, 2, 10_000);
        assert_eq!(r.original_lines, 5);
    }

    // ---------------------------------------------------------------
    // truncate_line — single-line truncation with "..."
    // ---------------------------------------------------------------

    #[test]
    fn test_line_empty_string() {
        let result = truncate_line("", 100);
        assert_eq!(result, "");
    }

    #[test]
    fn test_line_short_no_truncation() {
        let result = truncate_line("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_line_exact_boundary_no_truncation() {
        let result = truncate_line("12345", 5);
        assert_eq!(result, "12345");
    }

    #[test]
    fn test_line_over_limit_truncated_with_ellipsis() {
        let result = truncate_line("123456", 5);
        assert!(result.len() <= 8); // 5 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_line_very_long_string() {
        let long = "x".repeat(1000);
        let result = truncate_line(&long, 100);
        assert!(result.ends_with("..."));
        // Total char count: max_chars + "..."
        assert!(result.chars().count() <= 103);
    }

    #[test]
    fn test_line_unicode_truncation() {
        // CJK characters: each is 1 char, but 3 bytes
        let input = "你好世界再见你好世界再见";
        let result = truncate_line(input, 4);
        assert!(result.ends_with("..."));
        // Should keep 4 chars + "..."
        let without_dots: &str = result.trim_end_matches("...");
        assert!(without_dots.chars().count() <= 4);
    }

    #[test]
    fn test_line_max_chars_zero() {
        // Edge case: max_chars = 0 should produce either "..." (truncated) or ""
        let result = truncate_line("hello", 0);
        // "hello" has 5 chars which exceeds max_chars=0, so it must be truncated
        assert!(
            result == "..." || result.is_empty(),
            "max_chars=0 should produce '...' or empty, got: {:?}",
            result
        );
    }

    #[test]
    fn test_line_one_char_limit() {
        let result = truncate_line("abcdef", 1);
        assert!(result.ends_with("..."));
    }

    // ---------------------------------------------------------------
    // TruncationResult field validation
    // ---------------------------------------------------------------

    #[test]
    fn test_result_fields_no_truncation() {
        let input = "hello\nworld\n";
        let r = truncate_head(input, 100, 10_000);
        assert_eq!(r.content, input);
        assert!(!r.truncated);
        assert_eq!(r.original_lines, 2);
        assert_eq!(r.original_bytes, input.len());
        assert_eq!(r.kept_lines, 2);
    }

    #[test]
    fn test_result_fields_with_truncation() {
        let input = "a\nb\nc\nd\ne\n";
        let r = truncate_head(input, 2, 10_000);
        assert!(r.truncated);
        assert_eq!(r.original_lines, 5);
        assert_eq!(r.original_bytes, input.len());
        assert_eq!(r.kept_lines, 2);
        assert!(r.content.len() < input.len());
    }

    #[test]
    fn test_result_kept_lines_matches_actual_content() {
        let input = "a\nb\nc\nd\ne\nf\ng\n";
        let r = truncate_head(input, 3, 10_000);
        let actual_lines = r.content.lines().count();
        assert_eq!(r.kept_lines, actual_lines);
    }

    #[test]
    fn test_tail_result_kept_lines_matches_actual_content() {
        let input = "a\nb\nc\nd\ne\nf\ng\n";
        let r = truncate_tail(input, 3, 10_000);
        let actual_lines = r.content.lines().count();
        assert_eq!(r.kept_lines, actual_lines);
    }

    // ---------------------------------------------------------------
    // truncate_head — pure newline input
    // ---------------------------------------------------------------

    #[test]
    fn test_head_pure_newlines() {
        let r = truncate_head("\n\n\n", 2, 1024);
        // "\n\n\n" has 3 line segments: "\n", "\n", "\n"
        // .lines() counts 3 empty lines
        assert!(r.truncated, "limiting to 2 lines from 3 should truncate");
        assert_eq!(r.kept_lines, 2, "should keep exactly 2 lines");
    }

    // ---------------------------------------------------------------
    // truncate_head — max_lines=0
    // ---------------------------------------------------------------

    #[test]
    fn test_head_max_lines_zero() {
        let r = truncate_head("hello", 0, 1024);
        // 0 lines means nothing should be kept
        assert_eq!(r.kept_lines, 0, "max_lines=0 should keep 0 lines");
        assert!(r.content.is_empty(), "content should be empty with max_lines=0");
        assert!(r.truncated, "should be marked as truncated");
    }

    // ---------------------------------------------------------------
    // truncate_head — max_bytes=0
    // ---------------------------------------------------------------

    #[test]
    fn test_head_max_bytes_zero() {
        let r = truncate_head("hello", 100, 0);
        // 0 bytes means nothing can fit
        assert!(r.content.is_empty(), "content should be empty with max_bytes=0");
        assert!(r.truncated, "should be marked as truncated");
        assert_eq!(r.kept_lines, 0, "should keep 0 lines with max_bytes=0");
    }

    // ---------------------------------------------------------------
    // truncate_head — line + byte limits triggered simultaneously
    // ---------------------------------------------------------------

    #[test]
    fn test_head_both_line_and_byte_limits_hit() {
        // "aa\nbb\ncc\ndd\n" — each line is 3 bytes ("xx\n")
        // max_lines=2, max_bytes=6 (exactly the first 2 lines: "aa\nbb\n")
        let input = "aa\nbb\ncc\ndd\n";
        let r = truncate_head(input, 2, 6);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
        assert_eq!(r.original_lines, 4);
        // Content should be exactly the first 2 lines
        assert!(r.content.starts_with("aa\nbb"));
        assert!(r.content.len() <= 6);
    }

    // ---------------------------------------------------------------
    // truncate_tail — line + byte limits triggered simultaneously
    // ---------------------------------------------------------------

    #[test]
    fn test_tail_both_line_and_byte_limits_hit() {
        // "aa\nbb\ncc\ndd\n" — each line is 3 bytes
        // max_lines=2, max_bytes=6 (exactly the last 2 lines: "cc\ndd\n")
        let input = "aa\nbb\ncc\ndd\n";
        let r = truncate_tail(input, 2, 6);
        assert!(r.truncated);
        assert_eq!(r.kept_lines, 2);
        assert_eq!(r.original_lines, 4);
        // Content should contain the last 2 lines
        assert!(r.content.contains("cc"));
        assert!(r.content.contains("dd"));
        assert!(r.content.len() <= 6);
    }

    // ---------------------------------------------------------------
    // truncate_tail — max_bytes=0
    // ---------------------------------------------------------------

    #[test]
    fn test_tail_max_bytes_zero() {
        let result = truncate_tail("hello\nworld\n", 100, 0);
        // 0 bytes allowed = everything truncated
        assert!(
            result.truncated,
            "max_bytes=0 should mark as truncated"
        );
        assert!(
            result.content.is_empty(),
            "max_bytes=0 should produce empty content, got: {:?}",
            result.content
        );
        assert_eq!(
            result.kept_lines, 0,
            "max_bytes=0 should keep 0 lines"
        );
    }

    // ---------------------------------------------------------------
    // Truncation chaining: head then tail
    // ---------------------------------------------------------------

    #[test]
    fn test_truncation_chaining_head_then_tail() {
        let input = "line1\nline2\nline3\nline4\nline5\n";
        let head_result = truncate_head(input, 4, 1024);
        // head keeps first 4 lines: line1, line2, line3, line4
        assert_eq!(head_result.kept_lines, 4);

        let tail_result = truncate_tail(&head_result.content, 2, 1024);
        // tail keeps last 2 of those 4: line3, line4
        assert_eq!(tail_result.kept_lines, 2);
        assert!(
            tail_result.content.contains("line3"),
            "chained tail should contain line3, got: {:?}",
            tail_result.content
        );
        assert!(
            tail_result.content.contains("line4"),
            "chained tail should contain line4, got: {:?}",
            tail_result.content
        );
        assert!(
            !tail_result.content.contains("line1"),
            "chained tail should NOT contain line1"
        );
        assert!(
            !tail_result.content.contains("line2"),
            "chained tail should NOT contain line2"
        );
    }
}
