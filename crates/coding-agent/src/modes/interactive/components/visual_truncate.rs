//! Visual-line truncation utility.
//!
//! Translated from `components/visual-truncate.ts`.
//!
//! Accounts for line wrapping when truncating text to a maximum number of
//! visible terminal lines.

/// Result of a visual truncation operation.
pub struct VisualTruncateResult {
    /// The visual lines to display (from the end of the content).
    pub visual_lines: Vec<String>,
    /// Number of visual lines that were skipped (hidden from the top).
    pub skipped_count: usize,
}

/// Truncate text to at most `max_visual_lines` visible terminal lines,
/// taking line wrapping at `width` columns into account.
///
/// Lines are taken from the **end** of the content (tail), mirroring the TS
/// behaviour where recent output is always visible.
///
/// `padding_x` is the horizontal padding that will be applied to the text;
/// it reduces the effective content width.
pub fn truncate_to_visual_lines(
    text: &str,
    max_visual_lines: usize,
    width: u16,
    padding_x: usize,
) -> VisualTruncateResult {
    if text.is_empty() {
        return VisualTruncateResult { visual_lines: vec![], skipped_count: 0 };
    }

    let effective_width = (width as usize).saturating_sub(padding_x * 2).max(1);

    // Expand each logical line into visual lines based on visible column width.
    let mut all_visual: Vec<String> = Vec::new();
    for logical_line in text.split('\n') {
        let visual = wrap_line(logical_line, effective_width);
        if visual.is_empty() {
            all_visual.push(String::new());
        } else {
            all_visual.extend(visual);
        }
    }

    if all_visual.len() <= max_visual_lines {
        return VisualTruncateResult { visual_lines: all_visual, skipped_count: 0 };
    }

    let skipped_count = all_visual.len() - max_visual_lines;
    let visual_lines = all_visual[skipped_count..].to_vec();

    VisualTruncateResult { visual_lines, skipped_count }
}

/// Wrap a single line at `max_width` visible columns.
/// Returns one or more visual lines.
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in line.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + cw > max_width && current_width > 0 {
            result.push(current.clone());
            current.clear();
            current_width = 0;
        }
        current.push(ch);
        current_width += cw;
    }
    if !current.is_empty() || result.is_empty() {
        result.push(current);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_empty() {
        let r = truncate_to_visual_lines("", 10, 80, 0);
        assert!(r.visual_lines.is_empty());
        assert_eq!(r.skipped_count, 0);
    }

    #[test]
    fn short_text_not_truncated() {
        let r = truncate_to_visual_lines("hello\nworld", 10, 80, 0);
        assert_eq!(r.visual_lines.len(), 2);
        assert_eq!(r.skipped_count, 0);
    }

    #[test]
    fn long_text_truncated_from_start() {
        // 10 lines, max 3
        let text: String = (0..10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let r = truncate_to_visual_lines(&text, 3, 80, 0);
        assert_eq!(r.visual_lines.len(), 3);
        assert_eq!(r.skipped_count, 7);
        // Last 3 lines should be line 7, 8, 9
        assert!(r.visual_lines[0].contains("line 7"));
    }

    #[test]
    fn wrapping_creates_extra_visual_lines() {
        // A 30-char line in a 10-wide viewport = 3 visual lines
        let line = "a".repeat(30);
        let r = truncate_to_visual_lines(&line, 100, 10, 0);
        assert_eq!(r.visual_lines.len(), 3);
    }
}
