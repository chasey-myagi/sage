/// Box component — container that applies padding and background to children.
use std::cell::RefCell;

use crate::tui::Component;
use crate::utils::{apply_background_to_line, visible_width};

type BgFn = Box<dyn Fn(&str) -> String + Send + Sync>;

struct RenderCache {
    child_lines: Vec<String>,
    width: usize,
    bg_sample: Option<String>,
    lines: Vec<String>,
}

pub struct BoxComponent {
    pub children: Vec<Box<dyn Component + Send>>,
    padding_x: usize,
    padding_y: usize,
    bg_fn: Option<BgFn>,
    cache: RefCell<Option<RenderCache>>,
}

impl BoxComponent {
    pub fn new(padding_x: usize, padding_y: usize) -> Self {
        Self {
            children: Vec::new(),
            padding_x,
            padding_y,
            bg_fn: None,
            cache: RefCell::new(None),
        }
    }

    pub fn with_bg<F>(mut self, bg_fn: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.bg_fn = Some(Box::new(bg_fn));
        self
    }

    pub fn add_child<C: Component + Send + 'static>(&mut self, child: C) {
        self.children.push(Box::new(child));
        *self.cache.borrow_mut() = None;
    }

    pub fn remove_child(&mut self, index: usize) -> Option<Box<dyn Component + Send>> {
        if index < self.children.len() {
            *self.cache.borrow_mut() = None;
            Some(self.children.remove(index))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.children.clear();
        *self.cache.borrow_mut() = None;
    }

    pub fn set_bg_fn<F>(&mut self, bg_fn: Option<F>)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.bg_fn = bg_fn.map(|f| Box::new(f) as BgFn);
    }

    fn apply_bg(&self, line: &str, width: usize) -> String {
        let vis_len = visible_width(line);
        let pad_needed = width.saturating_sub(vis_len);
        let padded = format!("{line}{}", " ".repeat(pad_needed));

        if let Some(bg_fn) = &self.bg_fn {
            apply_background_to_line(&padded, width, bg_fn.as_ref())
        } else {
            padded
        }
    }

    fn match_cache(
        &self,
        width: usize,
        child_lines: &[String],
        bg_sample: &Option<String>,
    ) -> bool {
        if let Some(cache) = self.cache.borrow().as_ref() {
            cache.width == width
                && &cache.bg_sample == bg_sample
                && cache.child_lines.len() == child_lines.len()
                && cache
                    .child_lines
                    .iter()
                    .zip(child_lines.iter())
                    .all(|(a, b)| a == b)
        } else {
            false
        }
    }
}

impl Component for BoxComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;

        if self.children.is_empty() {
            return vec![];
        }

        let content_width = (width.saturating_sub(self.padding_x * 2)).max(1);
        let left_pad = " ".repeat(self.padding_x);

        // Render all children
        let mut child_lines: Vec<String> = Vec::new();
        for child in &self.children {
            let lines = child.render(content_width as u16);
            for line in lines {
                child_lines.push(format!("{left_pad}{line}"));
            }
        }

        if child_lines.is_empty() {
            return vec![];
        }

        // Check if bgFn output changed by sampling
        let bg_sample = self.bg_fn.as_ref().map(|f| f("test"));

        // Check cache validity
        if self.match_cache(width, &child_lines, &bg_sample) {
            return self.cache.borrow().as_ref().unwrap().lines.clone();
        }

        // Apply background and padding
        let mut result: Vec<String> = Vec::new();

        // Top padding
        for _ in 0..self.padding_y {
            result.push(self.apply_bg("", width));
        }

        // Content
        for line in &child_lines {
            result.push(self.apply_bg(line, width));
        }

        // Bottom padding
        for _ in 0..self.padding_y {
            result.push(self.apply_bg("", width));
        }

        *self.cache.borrow_mut() = Some(RenderCache {
            child_lines,
            width,
            bg_sample,
            lines: result.clone(),
        });

        result
    }

    fn invalidate(&mut self) {
        *self.cache.borrow_mut() = None;
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::spacer::Spacer;

    #[test]
    fn test_box_empty() {
        let b = BoxComponent::new(1, 1);
        let lines = b.render(80);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_box_with_child() {
        let mut b = BoxComponent::new(0, 0);
        b.add_child(Spacer::new(2));
        let lines = b.render(80);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_box_padding() {
        let mut b = BoxComponent::new(1, 1);
        b.add_child(Spacer::new(1));
        let lines = b.render(10);
        // 1 top + 1 content + 1 bottom = 3
        assert_eq!(lines.len(), 3);
        // Each line should be exactly 10 chars wide
        for line in &lines {
            assert_eq!(line.len(), 10, "line: {:?}", line);
        }
    }
}
