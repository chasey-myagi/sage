//! Utilities for formatting keybinding hints in the UI.
//!
//! Translated from `components/keybinding-hints.ts`.

use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// Format a keybinding name with dim styling.
pub fn key_text(keybinding: &str) -> String {
    keybinding.to_string()
}

/// Format a keybinding with its description as a styled hint line.
pub fn key_hint(keybinding: &str, description: &str) -> String {
    let t = get_theme();
    format!(
        "{}{}",
        t.fg(ThemeColor::Dim, keybinding),
        t.fg(ThemeColor::Muted, &format!(" {description}"))
    )
}

/// Format a raw key string (not necessarily a keybinding name) with description.
pub fn raw_key_hint(key: &str, description: &str) -> String {
    let t = get_theme();
    format!(
        "{}{}",
        t.fg(ThemeColor::Dim, key),
        t.fg(ThemeColor::Muted, &format!(" {description}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_returns_keybinding() {
        assert_eq!(key_text("ctrl+c"), "ctrl+c");
    }

    #[test]
    fn key_hint_produces_styled_text() {
        let hint = key_hint("ctrl+c", "to quit");
        // Should contain both parts
        assert!(hint.contains("ctrl+c"));
        assert!(hint.contains("to quit"));
    }

    #[test]
    fn raw_key_hint_produces_styled_text() {
        let hint = raw_key_hint("/", "for commands");
        assert!(hint.contains("/"));
        assert!(hint.contains("for commands"));
    }
}
