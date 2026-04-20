/// TruncatedText component — single-line text truncated to fit viewport width.
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width};

pub struct TruncatedText {
    text: String,
    padding_x: usize,
    padding_y: usize,
}

impl TruncatedText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Component for TruncatedText {
    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;
        let mut result = Vec::new();

        let empty_line = " ".repeat(width);

        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }

        let available_width = (width.saturating_sub(self.padding_x * 2)).max(1);

        // Take only first line
        let single_line = self.text.split('\n').next().unwrap_or("");

        let display_text = truncate_to_width(single_line, available_width, "", false);

        let left_padding = " ".repeat(self.padding_x);
        let right_padding = " ".repeat(self.padding_x);
        let line_with_padding = format!("{left_padding}{display_text}{right_padding}");

        let line_visible_width = visible_width(&line_with_padding);
        let padding_needed = width.saturating_sub(line_visible_width);
        let final_line = format!("{line_with_padding}{}", " ".repeat(padding_needed));
        result.push(final_line);

        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }

        result
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncated_text_fits() {
        let t = TruncatedText::new("hello", 0, 0);
        let lines = t.render(20);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("hello"));
        assert_eq!(visible_width(&lines[0]), 20);
    }

    #[test]
    fn test_truncated_text_too_long() {
        let t = TruncatedText::new("hello world this is a very long string", 0, 0);
        let lines = t.render(10);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 10);
    }

    #[test]
    fn test_truncated_text_padding_y() {
        let t = TruncatedText::new("hi", 0, 1);
        let lines = t.render(10);
        assert_eq!(lines.len(), 3); // 1 above, 1 content, 1 below
    }

    // ==========================================================================
    // Tests from truncated-text.test.ts
    // ==========================================================================

    #[test]
    fn test_pads_output_lines_to_exactly_match_width() {
        // "pads output lines to exactly match width"
        let t = TruncatedText::new("Hello world", 1, 0);
        let lines = t.render(50);
        assert_eq!(
            lines.len(),
            1,
            "should have exactly one content line (no vertical padding)"
        );
        // Line should be exactly 50 visible characters
        assert_eq!(
            visible_width(&lines[0]),
            50,
            "line should be exactly 50 visible chars"
        );
    }

    #[test]
    fn test_pads_output_with_vertical_padding_lines_to_width() {
        // "pads output with vertical padding lines to width"
        let t = TruncatedText::new("Hello", 0, 2);
        let lines = t.render(40);
        // 2 padding lines + 1 content line + 2 padding lines = 5 total
        assert_eq!(lines.len(), 5, "should have 2+1+2=5 total lines");
        for line in &lines {
            assert_eq!(
                visible_width(line),
                40,
                "all lines should be exactly 40 chars"
            );
        }
    }

    #[test]
    fn test_truncates_long_text_and_pads_to_width() {
        // "truncates long text and pads to width"
        let long_text =
            "This is a very long piece of text that will definitely exceed the available width";
        let t = TruncatedText::new(long_text, 1, 0);
        let lines = t.render(30);
        assert_eq!(lines.len(), 1);
        // Should be exactly 30 characters
        assert_eq!(
            visible_width(&lines[0]),
            30,
            "should be exactly 30 visible chars"
        );
        // Text should be truncated (not contain the full string)
        assert!(
            !lines[0].contains("definitely exceed"),
            "long text should be truncated"
        );
    }

    #[test]
    fn test_handles_text_that_fits_exactly() {
        // "handles text that fits exactly"
        // With paddingX=1, available width is 30-2=28
        // "Hello world" is 11 chars, fits comfortably
        let t = TruncatedText::new("Hello world", 1, 0);
        let lines = t.render(30);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 30);
        // Should contain the text
        assert!(
            lines[0].contains("Hello world"),
            "should contain the original text"
        );
    }

    #[test]
    fn test_handles_empty_text() {
        // "handles empty text"
        let t = TruncatedText::new("", 1, 0);
        let lines = t.render(30);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 30);
    }

    #[test]
    fn test_stops_at_newline_and_shows_only_first_line() {
        // "stops at newline and only shows first line"
        let multiline = "First line\nSecond line\nThird line";
        let t = TruncatedText::new(multiline, 1, 0);
        let lines = t.render(40);
        assert_eq!(lines.len(), 1, "should have exactly 1 line");
        assert_eq!(visible_width(&lines[0]), 40);
        // Should only contain "First line"
        let stripped: String = lines[0].chars().collect();
        let trimmed = stripped.trim();
        assert!(
            trimmed.contains("First line"),
            "should contain 'First line'"
        );
        assert!(
            !trimmed.contains("Second line"),
            "should not contain 'Second line'"
        );
        assert!(
            !trimmed.contains("Third line"),
            "should not contain 'Third line'"
        );
    }

    #[test]
    fn test_truncates_first_line_even_with_newlines() {
        // "truncates first line even with newlines in text"
        let long_multi = "This is a very long first line that needs truncation\nSecond line";
        let t = TruncatedText::new(long_multi, 1, 0);
        let lines = t.render(25);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 25);
        // Should not show second line
        assert!(
            !lines[0].contains("Second line"),
            "should not contain 'Second line'"
        );
    }

    #[test]
    fn test_set_text_updates_content() {
        let mut t = TruncatedText::new("Initial", 0, 0);
        let lines1 = t.render(20);
        assert!(lines1[0].contains("Initial"));

        t.set_text("Updated");
        let lines2 = t.render(20);
        assert!(lines2[0].contains("Updated"));
        assert!(!lines2[0].contains("Initial"));
    }

    #[test]
    fn test_zero_padding() {
        // With padding_x=0 and padding_y=0, the full width is available
        let t = TruncatedText::new("Hello", 0, 0);
        let lines = t.render(10);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 10);
        assert!(lines[0].starts_with("Hello"));
    }
}
