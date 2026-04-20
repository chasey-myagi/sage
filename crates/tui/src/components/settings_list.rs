/// SettingsList component — scrollable key-value settings list with optional search.

use std::collections::VecDeque;

use crate::components::input::Input;
use crate::fuzzy::fuzzy_filter;
use crate::keybindings::check_keybinding;
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, wrap_text_with_ansi};

/// Events emitted by `SettingsList` and drained via `poll_event()`.
#[derive(Debug, Clone)]
pub enum SettingsListEvent {
    /// A cycling value was changed.
    ValueChanged { id: String, value: String },
    /// An item with a submenu was activated.
    OpenSubmenu { item_id: String },
}

pub struct SettingItem {
    /// Unique identifier for this setting.
    pub id: String,
    /// Display label (left side).
    pub label: String,
    /// Optional description shown when selected.
    pub description: Option<String>,
    /// Current value to display (right side).
    pub current_value: String,
    /// If provided, Enter/Space cycles through these values.
    pub values: Option<Vec<String>>,
    /// If provided, Enter opens this submenu. Receives current value and done callback.
    pub submenu: Option<Box<dyn Fn(String, Box<dyn Fn(Option<String>) + Send>) -> Box<dyn Component + Send> + Send>>,
}

pub struct SettingsListTheme {
    pub label: Box<dyn Fn(&str, bool) -> String + Send + Sync>,
    pub value: Box<dyn Fn(&str, bool) -> String + Send + Sync>,
    pub description: Box<dyn Fn(&str) -> String + Send + Sync>,
    pub cursor: String,
    pub hint: Box<dyn Fn(&str) -> String + Send + Sync>,
}

pub struct SettingsListOptions {
    pub enable_search: bool,
}

impl Default for SettingsListOptions {
    fn default() -> Self {
        Self { enable_search: false }
    }
}

pub struct SettingsList {
    items: Vec<SettingItem>,
    filtered_items_indices: Vec<usize>,
    theme: SettingsListTheme,
    selected_index: usize,
    max_visible: usize,
    on_change: Box<dyn Fn(&str, &str) + Send>,
    on_cancel: Box<dyn Fn() + Send>,
    search_input: Option<Input>,
    search_enabled: bool,

    // Submenu state
    submenu_component: Option<Box<dyn Component + Send>>,
    submenu_item_index: Option<usize>,

    // Event queue — drained by the caller via poll_event()
    pending_events: VecDeque<SettingsListEvent>,
}

impl SettingsList {
    pub fn new(
        items: Vec<SettingItem>,
        max_visible: usize,
        theme: SettingsListTheme,
        on_change: impl Fn(&str, &str) + Send + 'static,
        on_cancel: impl Fn() + Send + 'static,
        options: SettingsListOptions,
    ) -> Self {
        let filtered_items_indices: Vec<usize> = (0..items.len()).collect();
        let search_input = if options.enable_search { Some(Input::new()) } else { None };
        Self {
            items,
            filtered_items_indices,
            theme,
            selected_index: 0,
            max_visible,
            on_change: Box::new(on_change),
            on_cancel: Box::new(on_cancel),
            search_input,
            search_enabled: options.enable_search,
            submenu_component: None,
            submenu_item_index: None,
            pending_events: VecDeque::new(),
        }
    }

    /// Drain the next pending event, if any.
    ///
    /// Call this after each `handle_input` to receive events produced by the
    /// component (e.g. `OpenSubmenu`, `ValueChanged`).
    pub fn poll_event(&mut self) -> Option<SettingsListEvent> {
        self.pending_events.pop_front()
    }

    pub fn update_value(&mut self, id: &str, new_value: &str) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.current_value = new_value.to_string();
        }
    }

    fn display_items_len(&self) -> usize {
        if self.search_enabled {
            self.filtered_items_indices.len()
        } else {
            self.items.len()
        }
    }

    fn get_display_item(&self, display_idx: usize) -> Option<&SettingItem> {
        if self.search_enabled {
            let item_idx = self.filtered_items_indices.get(display_idx)?;
            self.items.get(*item_idx)
        } else {
            self.items.get(display_idx)
        }
    }

    fn render_main_list(&self, width: u16) -> Vec<String> {
        let width = width as usize;
        let mut lines = Vec::new();

        if self.search_enabled {
            if let Some(search) = &self.search_input {
                lines.extend(search.render(width as u16));
                lines.push(String::new());
            }
        }

        if self.items.is_empty() {
            lines.push((self.theme.hint)("  No settings available"));
            if self.search_enabled {
                self.add_hint_line(&mut lines, width);
            }
            return lines;
        }

        let display_count = self.display_items_len();
        if display_count == 0 {
            lines.push(truncate_to_width(&(self.theme.hint)("  No matching settings"), width, "", false));
            self.add_hint_line(&mut lines, width);
            return lines;
        }

        // Calculate visible range
        let start_index = if display_count <= self.max_visible {
            0
        } else {
            let half = self.max_visible / 2;
            let ideal = self.selected_index.saturating_sub(half);
            ideal.min(display_count - self.max_visible)
        };
        let end_index = (start_index + self.max_visible).min(display_count);

        // Calculate max label width for alignment
        let max_label_width = self.items.iter()
            .map(|item| visible_width(&item.label))
            .max()
            .unwrap_or(0)
            .min(30);

        for i in start_index..end_index {
            let item = match self.get_display_item(i) {
                Some(it) => it,
                None => continue,
            };
            let is_selected = i == self.selected_index;
            let prefix = if is_selected { &self.theme.cursor } else { "  " };
            let prefix_width = visible_width(prefix);

            // Pad label to align values
            let label_vis = visible_width(&item.label);
            let label_padded = format!("{}{}", item.label, " ".repeat(max_label_width.saturating_sub(label_vis)));
            let label_text = (self.theme.label)(&label_padded, is_selected);

            let separator = "  ";
            let used_width = prefix_width + max_label_width + visible_width(separator);
            let value_max_width = width.saturating_sub(used_width + 2);

            let value_text = (self.theme.value)(&truncate_to_width(&item.current_value, value_max_width, "", false), is_selected);

            lines.push(truncate_to_width(&format!("{prefix}{label_text}{separator}{value_text}"), width, "", false));
        }

        // Scroll indicator
        if start_index > 0 || end_index < display_count {
            let scroll_text = format!("  ({}/{})", self.selected_index + 1, display_count);
            lines.push((self.theme.hint)(&truncate_to_width(&scroll_text, width.saturating_sub(2), "", false)));
        }

        // Description for selected item
        if let Some(item) = self.get_display_item(self.selected_index) {
            if let Some(desc) = &item.description {
                lines.push(String::new());
                let wrapped = wrap_text_with_ansi(desc, width.saturating_sub(4));
                for line in &wrapped {
                    lines.push((self.theme.description)(&format!("  {line}")));
                }
            }
        }

        self.add_hint_line(&mut lines, width);
        lines
    }

    fn add_hint_line(&self, lines: &mut Vec<String>, width: usize) {
        lines.push(String::new());
        let hint_text = if self.search_enabled {
            "  Type to search · Enter/Space to change · Esc to cancel"
        } else {
            "  Enter/Space to change · Esc to cancel"
        };
        lines.push(truncate_to_width(&(self.theme.hint)(hint_text), width, "", false));
    }

    fn activate_item(&mut self) {
        let item_index = if self.search_enabled {
            self.filtered_items_indices.get(self.selected_index).copied()
        } else {
            if self.selected_index < self.items.len() { Some(self.selected_index) } else { None }
        };

        let item_index = match item_index {
            Some(i) => i,
            None => return,
        };

        if self.items[item_index].submenu.is_some() {
            self.submenu_item_index = Some(self.selected_index);
            let item_id = self.items[item_index].id.clone();
            // Push an event instead of trying to call the closure while `self` is
            // borrowed — the caller drains this via `poll_event()` and handles
            // the submenu lifecycle externally.
            self.pending_events.push_back(SettingsListEvent::OpenSubmenu { item_id });
        } else if let Some(values) = &self.items[item_index].values {
            if !values.is_empty() {
                let current_index = values.iter().position(|v| *v == self.items[item_index].current_value).unwrap_or(0);
                let next_index = (current_index + 1) % values.len();
                let new_value = values[next_index].clone();
                let item_id = self.items[item_index].id.clone();
                self.items[item_index].current_value = new_value.clone();
                (self.on_change)(&item_id, &new_value);
                self.pending_events.push_back(SettingsListEvent::ValueChanged {
                    id: item_id,
                    value: new_value,
                });
            }
        }
    }

    fn close_submenu(&mut self) {
        self.submenu_component = None;
        if let Some(idx) = self.submenu_item_index.take() {
            self.selected_index = idx;
        }
    }

    fn apply_filter(&mut self, query: &str) {
        let labels: Vec<(usize, String)> = self.items.iter().enumerate()
            .map(|(i, item)| (i, item.label.clone()))
            .collect();

        self.filtered_items_indices = if query.is_empty() {
            (0..self.items.len()).collect()
        } else {
            fuzzy_filter(labels, query, |(_, label)| label.as_str())
                .into_iter()
                .map(|(i, _)| i)
                .collect()
        };
        self.selected_index = 0;
    }
}

impl Component for SettingsList {
    fn render(&self, width: u16) -> Vec<String> {
        if let Some(submenu) = &self.submenu_component {
            return submenu.render(width);
        }
        self.render_main_list(width)
    }

    fn handle_input(&mut self, data: &str) {
        if self.submenu_component.is_some() {
            if let Some(submenu) = &mut self.submenu_component {
                submenu.handle_input(data);
            }
            return;
        }

        let display_count = self.display_items_len();

        if check_keybinding(data, "tui.select.up") {
            if display_count == 0 { return; }
            self.selected_index = if self.selected_index == 0 {
                display_count - 1
            } else {
                self.selected_index - 1
            };
        } else if check_keybinding(data, "tui.select.down") {
            if display_count == 0 { return; }
            self.selected_index = if self.selected_index == display_count - 1 {
                0
            } else {
                self.selected_index + 1
            };
        } else if check_keybinding(data, "tui.select.confirm") || data == " " {
            self.activate_item();
        } else if check_keybinding(data, "tui.select.cancel") {
            (self.on_cancel)();
        } else if self.search_enabled {
            let sanitized = data.replace(' ', "");
            if sanitized.is_empty() { return; }
            if let Some(search) = &mut self.search_input {
                search.handle_input(&sanitized);
                let query = search.get_value().to_string();
                self.apply_filter(&query);
            }
        }
    }

    fn invalidate(&mut self) {
        if let Some(submenu) = &mut self.submenu_component {
            submenu.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_theme() -> SettingsListTheme {
        SettingsListTheme {
            label: Box::new(|s: &str, _: bool| s.to_string()),
            value: Box::new(|s: &str, _: bool| s.to_string()),
            description: Box::new(|s: &str| s.to_string()),
            cursor: "→ ".to_string(),
            hint: Box::new(|s: &str| s.to_string()),
        }
    }

    fn make_items() -> Vec<SettingItem> {
        vec![
            SettingItem {
                id: "theme".to_string(),
                label: "Theme".to_string(),
                description: None,
                current_value: "dark".to_string(),
                values: Some(vec!["dark".to_string(), "light".to_string()]),
                submenu: None,
            },
        ]
    }

    #[test]
    fn test_settings_list_renders() {
        let list = SettingsList::new(
            make_items(),
            5,
            make_theme(),
            |_id: &str, _val: &str| {},
            || {},
            Default::default(),
        );
        let lines = list.render(60);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_settings_list_cycle_values() {
        let changed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let changed_clone = changed.clone();
        let mut list = SettingsList::new(
            make_items(),
            5,
            make_theme(),
            move |_id: &str, val: &str| {
                *changed_clone.lock().unwrap() = val.to_string();
            },
            || {},
            Default::default(),
        );
        list.handle_input("\r"); // Enter
        assert_eq!(*changed.lock().unwrap(), "light");
    }
}
