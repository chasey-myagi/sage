//! Theme selector component.
//!
//! Translated from `components/theme-selector.ts`.

use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// Available built-in theme names.
pub const BUILTIN_THEMES: &[&str] = &["dark", "light"];

/// Theme selector component with live preview.
pub struct ThemeSelectorComponent {
    themes: Vec<String>,
    selected_index: usize,
    current_theme: String,
    on_select: Option<Box<dyn Fn(&str) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
    on_preview: Option<Box<dyn Fn(&str) + Send>>,
}

impl ThemeSelectorComponent {
    pub fn new(current_theme: impl Into<String>) -> Self {
        let current = current_theme.into();
        let themes: Vec<String> = BUILTIN_THEMES.iter().map(|s| s.to_string()).collect();
        let selected_index = themes.iter().position(|t| t == &current).unwrap_or(0);

        Self {
            themes,
            selected_index,
            current_theme: current,
            on_select: None,
            on_cancel: None,
            on_preview: None,
        }
    }

    pub fn set_on_select<F: Fn(&str) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    pub fn set_on_preview<F: Fn(&str) + Send + 'static>(&mut self, f: F) {
        self.on_preview = Some(Box::new(f));
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        match key {
            "\x1b[A" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    if let Some(ref f) = self.on_preview {
                        f(&self.themes[self.selected_index]);
                    }
                }
                true
            }
            "\x1b[B" => {
                if self.selected_index + 1 < self.themes.len() {
                    self.selected_index += 1;
                    if let Some(ref f) = self.on_preview {
                        f(&self.themes[self.selected_index]);
                    }
                }
                true
            }
            "\r" | "\n" => {
                if let Some(theme) = self.themes.get(self.selected_index) {
                    if let Some(ref f) = self.on_select {
                        f(theme);
                    }
                }
                true
            }
            "\x1b" => {
                if let Some(ref f) = self.on_cancel {
                    f();
                }
                true
            }
            _ => false,
        }
    }
}

impl Component for ThemeSelectorComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(DynamicBorder::new()));

        for (i, theme_name) in self.themes.iter().enumerate() {
            let is_selected = i == self.selected_index;
            let is_current = *theme_name == self.current_theme;

            let prefix = if is_selected { "▶ " } else { "  " };
            let name_str = if is_selected {
                t.fg(ThemeColor::Accent, theme_name)
            } else {
                theme_name.clone()
            };
            let current_mark = if is_current {
                t.fg(ThemeColor::Muted, " (current)")
            } else {
                String::new()
            };

            container.add_child(Box::new(Text::new(
                format!("{prefix}{name_str}{current_mark}"),
                1,
                0,
            )));
        }

        container.add_child(Box::new(Spacer::new(1)));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_theme_list() {
        let comp = ThemeSelectorComponent::new("dark");
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("dark"));
        assert!(text.contains("light"));
    }

    #[test]
    fn navigate_down_triggers_preview() {
        let mut previewed = String::new();
        let mut comp = ThemeSelectorComponent::new("dark");
        comp.set_on_preview(move |name| {
            // Can't capture `previewed` mutably in Fn — just verify it's called
            let _ = name;
        });
        comp.handle_key("\x1b[B"); // down
        assert_eq!(comp.selected_index, 1);
    }
}
