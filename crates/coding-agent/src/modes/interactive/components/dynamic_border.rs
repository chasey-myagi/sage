//! Dynamic border component — full-width horizontal rule.
//!
//! Translated from `components/dynamic-border.ts`.

use tui::tui::Component;

use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// A horizontal line that fills the viewport width, colored with the theme's border color.
pub struct DynamicBorder {
    color_fn: Box<dyn Fn(&str) -> String + Send + Sync>,
}

impl Default for DynamicBorder {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicBorder {
    /// Create with the default theme border color.
    pub fn new() -> Self {
        Self {
            color_fn: Box::new(|s| {
                let t = get_theme();
                t.fg(ThemeColor::Border, s)
            }),
        }
    }

    /// Create with a custom color function.
    pub fn with_color<F>(color_fn: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            color_fn: Box::new(color_fn),
        }
    }
}

impl Component for DynamicBorder {
    fn render(&self, width: u16) -> Vec<String> {
        let n = (width as usize).max(1);
        let line = "─".repeat(n);
        vec![(self.color_fn)(&line)]
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_correct_width() {
        let b = DynamicBorder::with_color(|s| s.to_string());
        let lines = b.render(10);
        assert_eq!(lines.len(), 1);
        // The line should be 10 em-dash chars (each is 3 UTF-8 bytes, 1 column wide)
        let col_count = unicode_width::UnicodeWidthStr::width(lines[0].as_str());
        assert_eq!(col_count, 10);
    }

    #[test]
    fn renders_minimum_width() {
        let b = DynamicBorder::with_color(|s| s.to_string());
        let lines = b.render(0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].is_empty()); // at least 1 char
    }
}
