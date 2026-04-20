/// Text component — multi-line text with word wrapping.

use crate::tui::Component;
use crate::utils::{apply_background_to_line, visible_width, wrap_text_with_ansi};

type BgFn = Box<dyn Fn(&str) -> String + Send + Sync>;

pub struct Text {
    text: String,
    padding_x: usize,
    padding_y: usize,
    custom_bg_fn: Option<BgFn>,

    // Cache
    cached_text: Option<String>,
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
}

impl Text {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            custom_bg_fn: None,
            cached_text: None,
            cached_width: None,
            cached_lines: None,
        }
    }

    pub fn with_bg<F>(mut self, bg_fn: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.custom_bg_fn = Some(Box::new(bg_fn));
        self
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cached_text = None;
        self.cached_width = None;
        self.cached_lines = None;
    }

    pub fn set_custom_bg_fn<F>(&mut self, bg_fn: Option<F>)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.custom_bg_fn = bg_fn.map(|f| Box::new(f) as BgFn);
        self.cached_text = None;
        self.cached_width = None;
        self.cached_lines = None;
    }
}

impl Component for Text {
    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;

        // Check cache
        if let (Some(ct), Some(cw), Some(cl)) =
            (&self.cached_text, &self.cached_width, &self.cached_lines)
        {
            if *ct == self.text && *cw == width {
                return cl.clone();
            }
        }

        // Don't render anything if there's no actual text
        if self.text.is_empty() || self.text.trim().is_empty() {
            return vec![];
        }

        // Replace tabs with 3 spaces
        let normalized_text = self.text.replace('\t', "   ");

        // Calculate content width (subtract left/right margins)
        let content_width = (width.saturating_sub(self.padding_x * 2)).max(1);

        // Wrap text preserving ANSI codes
        let wrapped_lines = wrap_text_with_ansi(&normalized_text, content_width);

        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let mut content_lines: Vec<String> = Vec::new();

        for line in &wrapped_lines {
            let line_with_margins = format!("{left_margin}{line}{right_margin}");
            if let Some(bg_fn) = &self.custom_bg_fn {
                content_lines.push(apply_background_to_line(&line_with_margins, width, bg_fn.as_ref()));
            } else {
                let visible_len = visible_width(&line_with_margins);
                let padding_needed = width.saturating_sub(visible_len);
                content_lines.push(format!("{line_with_margins}{}", " ".repeat(padding_needed)));
            }
        }

        // Add top/bottom padding (empty lines)
        let empty_line = " ".repeat(width);
        let mut result = Vec::new();
        for _ in 0..self.padding_y {
            let line = if let Some(bg_fn) = &self.custom_bg_fn {
                apply_background_to_line(&empty_line, width, bg_fn.as_ref())
            } else {
                empty_line.clone()
            };
            result.push(line);
        }
        result.extend(content_lines);
        for _ in 0..self.padding_y {
            let line = if let Some(bg_fn) = &self.custom_bg_fn {
                apply_background_to_line(&empty_line, width, bg_fn.as_ref())
            } else {
                empty_line.clone()
            };
            result.push(line);
        }

        if result.is_empty() {
            result.push(String::new());
        }

        result
    }

    fn invalidate(&mut self) {
        self.cached_text = None;
        self.cached_width = None;
        self.cached_lines = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_empty() {
        let t = Text::new("", 0, 0);
        let lines = t.render(80);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_text_simple() {
        let t = Text::new("hello", 0, 0);
        let lines = t.render(80);
        assert!(!lines.is_empty());
        assert!(lines[0].contains("hello"));
    }

    #[test]
    fn test_text_padding_y() {
        let t = Text::new("hello", 0, 1);
        let lines = t.render(20);
        // 1 padding above, 1 content line, 1 padding below
        assert!(lines.len() >= 3);
        assert!(lines[0].chars().all(|c| c == ' '));
    }

    #[test]
    fn test_text_wraps() {
        let long = "word ".repeat(20);
        let t = Text::new(long, 0, 0);
        let lines = t.render(20);
        assert!(lines.len() > 1);
    }
}
