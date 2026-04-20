//! Session selector component.
//!
//! Translated from `components/session-selector.ts`.
//!
//! Provides a searchable session selection list.

use std::time::{Duration, SystemTime};

use tui::tui::{Component, Container};
use tui::components::spacer::Spacer;
use tui::components::text::Text;

use crate::modes::interactive::theme::{get_theme, ThemeColor};
use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;

/// A session entry displayed in the selector.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub path: String,
    pub created_at: SystemTime,
    pub message_count: usize,
}

/// Format a session's age relative to now.
pub fn format_session_date(created_at: SystemTime) -> String {
    let now = SystemTime::now();
    let diff = now.duration_since(created_at).unwrap_or(Duration::ZERO);
    let secs = diff.as_secs();

    if secs < 60 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 4 {
        return format!("{weeks}w");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo");
    }
    let years = days / 365;
    format!("{years}y")
}

/// Shorten a path by replacing the home directory with `~`.
pub fn shorten_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        return format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

/// Session selector component.
pub struct SessionSelectorComponent {
    sessions: Vec<SessionInfo>,
    filtered: Vec<usize>,
    selected_index: usize,
    search_query: String,
    on_select: Option<Box<dyn Fn(&SessionInfo) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl SessionSelectorComponent {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        let n = sessions.len();
        let mut comp = Self {
            sessions,
            filtered: (0..n).collect(),
            selected_index: 0,
            search_query: String::new(),
            on_select: None,
            on_cancel: None,
        };
        comp.refresh_filter();
        comp
    }

    pub fn set_on_select<F: Fn(&SessionInfo) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    fn refresh_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if q.is_empty() {
                    return true;
                }
                s.id.to_lowercase().contains(&q)
                    || s.name.as_deref().unwrap_or("").to_lowercase().contains(&q)
                    || s.path.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        self.selected_index = 0;
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        match key {
            "\x1b[A" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
                true
            }
            "\x1b[B" => {
                if self.selected_index + 1 < self.filtered.len() {
                    self.selected_index += 1;
                }
                true
            }
            "\r" | "\n" => {
                if let Some(&idx) = self.filtered.get(self.selected_index) {
                    if let Some(ref f) = self.on_select {
                        f(&self.sessions[idx]);
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
            "\x7f" => {
                self.search_query.pop();
                self.refresh_filter();
                true
            }
            ch if ch.len() == 1 && ch.chars().next().map_or(false, |c| !c.is_control()) => {
                self.search_query.push_str(ch);
                self.refresh_filter();
                true
            }
            _ => false,
        }
    }
}

impl Component for SessionSelectorComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(DynamicBorder::new()));

        // Search input
        let search_line = format!(
            "{} {}",
            t.fg(ThemeColor::Dim, "Search:"),
            t.fg(ThemeColor::Accent, &self.search_query)
        );
        container.add_child(Box::new(Text::new(search_line, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

        // Session list (up to 10 visible)
        let visible_count = self.filtered.len().min(10);
        let start = if visible_count == 0 || self.selected_index < visible_count {
            0
        } else {
            self.selected_index - visible_count + 1
        };

        for (rel_idx, &abs_idx) in self.filtered[start..].iter().take(visible_count).enumerate() {
            let session = &self.sessions[abs_idx];
            let is_selected = start + rel_idx == self.selected_index;

            let prefix = if is_selected { "▶ " } else { "  " };
            let name = session.name.as_deref().unwrap_or(&session.id);
            let age = format_session_date(session.created_at);
            let path = shorten_path(&session.path);

            let name_str = if is_selected {
                t.fg(ThemeColor::Accent, name)
            } else {
                name.to_string()
            };
            let meta = t.fg(
                ThemeColor::Dim,
                &format!(" {age} • {path} • {} msgs", session.message_count),
            );

            container.add_child(Box::new(Text::new(
                format!("{prefix}{name_str}{meta}"),
                1,
                0,
            )));
        }

        if self.filtered.is_empty() {
            container.add_child(Box::new(Text::new(
                t.fg(ThemeColor::Muted, "No sessions found"),
                1,
                0,
            )));
        }

        container.add_child(Box::new(Spacer::new(1)));

        let hints = format!(
            "{}  {}  {}",
            key_hint("↑/↓", "navigate"),
            key_hint("Enter", "open"),
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
    fn format_session_date_seconds() {
        let ts = SystemTime::now() - Duration::from_secs(30);
        assert_eq!(format_session_date(ts), "now");
    }

    #[test]
    fn format_session_date_minutes() {
        let ts = SystemTime::now() - Duration::from_secs(90);
        assert_eq!(format_session_date(ts), "1m");
    }

    #[test]
    fn empty_sessions_renders() {
        let comp = SessionSelectorComponent::new(vec![]);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("No sessions"));
    }
}
