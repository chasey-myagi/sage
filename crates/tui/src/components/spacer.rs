/// Spacer component — renders N empty lines.
use crate::tui::Component;

pub struct Spacer {
    lines: usize,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self {
            lines: lines.max(1),
        }
    }

    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines.max(1);
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Component for Spacer {
    fn render(&self, _width: u16) -> Vec<String> {
        (0..self.lines).map(|_| String::new()).collect()
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spacer_default() {
        let s = Spacer::default();
        let lines = s.render(80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "");
    }

    #[test]
    fn test_spacer_three_lines() {
        let s = Spacer::new(3);
        let lines = s.render(80);
        assert_eq!(lines.len(), 3);
    }
}
