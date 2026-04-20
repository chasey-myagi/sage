//! Show-images selector component.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/components/show-images-selector.ts`.
//!
//! Presents a two-item list ("Yes" / "No") that lets the user toggle whether
//! images are shown inline in the terminal.

use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// Items in the selector.
const ITEMS: &[(&str, &str, &str)] = &[
    ("yes", "Yes", "Show images inline in terminal"),
    ("no", "No", "Show text placeholder instead"),
];

/// Component that renders a show-images selector with borders.
///
/// Mirrors `ShowImagesSelectorComponent` from TypeScript.
pub struct ShowImagesSelectorComponent {
    selected_index: usize,
    on_select: Option<Box<dyn Fn(bool) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ShowImagesSelectorComponent {
    /// Create the selector.
    ///
    /// `current_value`: the current show-images setting.
    pub fn new(current_value: bool) -> Self {
        Self {
            selected_index: if current_value { 0 } else { 1 },
            on_select: None,
            on_cancel: None,
        }
    }

    pub fn set_on_select<F: Fn(bool) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    /// Handle a keyboard event. Returns `true` if the event was consumed.
    pub fn handle_key(&mut self, key: &str) -> bool {
        match key {
            // Up arrow
            "\x1b[A" | "k" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
                true
            }
            // Down arrow
            "\x1b[B" | "j" => {
                if self.selected_index + 1 < ITEMS.len() {
                    self.selected_index += 1;
                }
                true
            }
            // Enter / Return
            "\r" | "\n" => {
                let value = ITEMS[self.selected_index].0 == "yes";
                if let Some(ref f) = self.on_select {
                    f(value);
                }
                true
            }
            // Escape
            "\x1b" => {
                if let Some(ref f) = self.on_cancel {
                    f();
                }
                true
            }
            _ => false,
        }
    }

    /// Return the currently highlighted value.
    pub fn selected_value(&self) -> bool {
        ITEMS[self.selected_index].0 == "yes"
    }
}

impl Component for ShowImagesSelectorComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(DynamicBorder::new()));

        for (i, (_value, label, description)) in ITEMS.iter().enumerate() {
            let is_selected = i == self.selected_index;

            let prefix = if is_selected { "▶ " } else { "  " };
            let label_str = if is_selected {
                t.fg(ThemeColor::Accent, label)
            } else {
                label.to_string()
            };
            let desc_str = t.fg(ThemeColor::Muted, &format!("  {description}"));

            container.add_child(Box::new(Text::new(
                format!("{prefix}{label_str}{desc_str}"),
                1,
                0,
            )));
        }

        let hints = format!(
            "{}  {}  {}",
            key_hint("↑/↓", "navigate"),
            key_hint("Enter", "select"),
            key_hint("Esc", "cancel"),
        );
        container.add_child(Box::new(Text::new(hints, 1, 0)));

        container.add_child(Box::new(DynamicBorder::new()));

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_selection_matches_current_value() {
        let comp = ShowImagesSelectorComponent::new(true);
        assert_eq!(comp.selected_index, 0); // "yes"

        let comp = ShowImagesSelectorComponent::new(false);
        assert_eq!(comp.selected_index, 1); // "no"
    }

    #[test]
    fn navigate_up_and_down() {
        let mut comp = ShowImagesSelectorComponent::new(true); // starts at 0
        comp.handle_key("\x1b[B"); // down → 1
        assert_eq!(comp.selected_index, 1);
        comp.handle_key("\x1b[A"); // up → 0
        assert_eq!(comp.selected_index, 0);
    }

    #[test]
    fn navigate_does_not_wrap() {
        let mut comp = ShowImagesSelectorComponent::new(false); // starts at 1
        comp.handle_key("\x1b[B"); // down — already at bottom
        assert_eq!(comp.selected_index, 1);

        let mut comp = ShowImagesSelectorComponent::new(true); // starts at 0
        comp.handle_key("\x1b[A"); // up — already at top
        assert_eq!(comp.selected_index, 0);
    }

    #[test]
    fn on_select_fired_with_correct_value() {
        use std::sync::{Arc, Mutex};

        let result = Arc::new(Mutex::new(None));
        let result2 = Arc::clone(&result);

        let mut comp = ShowImagesSelectorComponent::new(false); // "no" selected
        comp.set_on_select(move |v| {
            *result2.lock().unwrap() = Some(v);
        });
        comp.handle_key("\r");

        assert_eq!(*result.lock().unwrap(), Some(false));
    }

    #[test]
    fn renders_without_panic() {
        let comp = ShowImagesSelectorComponent::new(true);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Yes"));
        assert!(text.contains("No"));
    }
}
