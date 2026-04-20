//! Bordered loader component — loading animation with border wrapping.
//!
//! Translated from `components/bordered-loader.ts`.

use tui::tui::{Component, Container};
use tui::components::spacer::Spacer;
use tui::components::text::Text;

use crate::modes::interactive::theme::{get_theme, ThemeColor};
use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;

// ============================================================================
// Spinner frames
// ============================================================================

const SPINNER_FRAMES: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Animated bordered loader component.
pub struct BorderedLoader {
    message: String,
    cancellable: bool,
    frame: usize,
    // Container with border + spinner text + optional cancel hint
    container: Container,
}

impl BorderedLoader {
    /// Create a loader with the given message.
    /// If `cancellable` is true, shows a cancel hint below.
    pub fn new(message: impl Into<String>, cancellable: bool) -> Self {
        let message = message.into();
        let mut comp = Self {
            message: message.clone(),
            cancellable,
            frame: 0,
            container: Container::new(),
        };
        comp.rebuild();
        comp
    }

    /// Advance the spinner to the next frame.
    pub fn tick(&mut self) {
        self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let t = get_theme();
        self.container.clear();

        let border_fn = {
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::Border, s)
        };

        self.container.add_child(Box::new(DynamicBorder::with_color(border_fn.clone())));

        let spinner = SPINNER_FRAMES[self.frame];
        let spinner_colored = t.fg(ThemeColor::Accent, spinner);
        let msg_colored = t.fg(ThemeColor::Muted, &self.message);
        let line = format!("{spinner_colored} {msg_colored}");
        self.container.add_child(Box::new(Text::new(line, 1, 0)));

        if self.cancellable {
            self.container.add_child(Box::new(Spacer::new(1)));
            let hint = key_hint("ctrl+c", "cancel");
            self.container.add_child(Box::new(Text::new(hint, 1, 0)));
        }

        self.container.add_child(Box::new(Spacer::new(1)));
        self.container.add_child(Box::new(DynamicBorder::with_color({
            let t3 = t.clone();
            move |s: &str| t3.fg(ThemeColor::Border, s)
        })));
    }
}

impl Component for BorderedLoader {
    fn render(&self, width: u16) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_message() {
        let loader = BorderedLoader::new("Loading...", false);
        let lines = loader.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Loading..."), "Expected 'Loading...' in: {text:?}");
    }

    #[test]
    fn cancellable_shows_hint() {
        let loader = BorderedLoader::new("Working", true);
        let lines = loader.render(80);
        let text = lines.join("\n");
        // Should contain cancel hint
        assert!(text.contains("cancel"), "Expected 'cancel' in: {text:?}");
    }

    #[test]
    fn tick_advances_frame() {
        let mut loader = BorderedLoader::new("Test", false);
        let frame0 = loader.frame;
        loader.tick();
        assert_ne!(loader.frame, frame0);
    }
}
