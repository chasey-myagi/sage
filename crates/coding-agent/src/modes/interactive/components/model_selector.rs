//! Model selector component.
//!
//! Translated from `components/model-selector.ts`.
//!
//! Provides a searchable model selection list above the editor.

use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// A model entry in the selector.
#[derive(Debug, Clone)]
pub struct ModelItem {
    pub id: String,
    pub provider: String,
    pub label: String,
}

/// Model selector component with fuzzy search.
pub struct ModelSelectorComponent {
    models: Vec<ModelItem>,
    filtered: Vec<usize>,
    selected_index: usize,
    search_query: String,
    current_model_id: Option<String>,
    #[allow(clippy::type_complexity)]
    on_select: Option<Box<dyn Fn(&ModelItem) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ModelSelectorComponent {
    pub fn new(models: Vec<ModelItem>, current_model_id: Option<String>) -> Self {
        let n = models.len();
        let mut comp = Self {
            models,
            filtered: (0..n).collect(),
            selected_index: 0,
            search_query: String::new(),
            current_model_id,
            on_select: None,
            on_cancel: None,
        };
        comp.refresh_filter();
        comp
    }

    pub fn set_on_select<F: Fn(&ModelItem) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    fn refresh_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = self
            .models
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if q.is_empty() {
                    return true;
                }
                m.id.to_lowercase().contains(&q)
                    || m.provider.to_lowercase().contains(&q)
                    || m.label.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        self.selected_index = 0;
    }

    /// Handle keyboard input for navigation.
    pub fn handle_key(&mut self, key: &str) -> bool {
        match key {
            "\x1b[A" | "ctrl+p" => {
                // Up arrow
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
                true
            }
            "\x1b[B" | "ctrl+n" => {
                // Down arrow
                if self.selected_index + 1 < self.filtered.len() {
                    self.selected_index += 1;
                }
                true
            }
            "\r" | "\n" => {
                // Enter — select
                if let Some(&idx) = self.filtered.get(self.selected_index)
                    && let Some(ref f) = self.on_select
                {
                    f(&self.models[idx]);
                }
                true
            }
            "\x1b" => {
                // Escape — cancel
                if let Some(ref f) = self.on_cancel {
                    f();
                }
                true
            }
            "\x7f" => {
                // Backspace
                self.search_query.pop();
                self.refresh_filter();
                true
            }
            ch if ch.len() == 1 && ch.chars().next().is_some_and(|c| !c.is_control()) => {
                self.search_query.push_str(ch);
                self.refresh_filter();
                true
            }
            _ => false,
        }
    }
}

impl Component for ModelSelectorComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();
        let _width_u = width as usize;

        // Top border
        container.add_child(Box::new(DynamicBorder::new()));

        // Search input line
        let search_line = format!(
            "{} {}",
            t.fg(ThemeColor::Dim, "Search:"),
            t.fg(ThemeColor::Accent, &self.search_query)
        );
        container.add_child(Box::new(Text::new(search_line, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

        // Model list (show up to 10 items)
        let visible_count = self.filtered.len().min(10);
        let start = if self.selected_index >= visible_count {
            self.selected_index - visible_count + 1
        } else {
            0
        };

        for (rel_idx, &abs_idx) in self.filtered[start..]
            .iter()
            .take(visible_count)
            .enumerate()
        {
            let model = &self.models[abs_idx];
            let is_selected = start + rel_idx == self.selected_index;
            let is_current = self.current_model_id.as_deref() == Some(&model.id);

            let prefix = if is_selected { "▶ " } else { "  " };
            let id_str = if is_selected {
                t.fg(ThemeColor::Accent, &model.id)
            } else {
                model.id.clone()
            };
            let provider_str = t.fg(ThemeColor::Dim, &format!(" ({})", model.provider));
            let current_mark = if is_current {
                t.fg(ThemeColor::Muted, " ✓")
            } else {
                String::new()
            };

            let line = format!("{prefix}{id_str}{provider_str}{current_mark}");
            container.add_child(Box::new(Text::new(line, 1, 0)));
        }

        if self.filtered.is_empty() {
            container.add_child(Box::new(Text::new(
                t.fg(ThemeColor::Muted, "No models found"),
                1,
                0,
            )));
        }

        container.add_child(Box::new(Spacer::new(1)));

        // Key hints
        let hints = format!(
            "{}  {}  {}",
            key_hint("↑/↓", "navigate"),
            key_hint("Enter", "select"),
            key_hint("Esc", "cancel"),
        );
        container.add_child(Box::new(Text::new(hints, 1, 0)));

        // Bottom border
        container.add_child(Box::new(DynamicBorder::new()));

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_models() -> Vec<ModelItem> {
        vec![
            ModelItem {
                id: "claude-3-5-sonnet".to_string(),
                provider: "anthropic".to_string(),
                label: "anthropic/claude-3-5-sonnet".to_string(),
            },
            ModelItem {
                id: "gpt-4o".to_string(),
                provider: "openai".to_string(),
                label: "openai/gpt-4o".to_string(),
            },
        ]
    }

    #[test]
    fn renders_model_list() {
        let comp = ModelSelectorComponent::new(sample_models(), None);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("claude-3-5-sonnet") || text.contains("gpt-4o"));
    }

    #[test]
    fn search_filters_models() {
        let mut comp = ModelSelectorComponent::new(sample_models(), None);
        comp.handle_key("g"); // type 'g' to filter
        comp.handle_key("p"); // "gp" → gpt matches
        assert_eq!(comp.filtered.len(), 1);
        assert_eq!(comp.models[comp.filtered[0]].id, "gpt-4o");
    }
}
