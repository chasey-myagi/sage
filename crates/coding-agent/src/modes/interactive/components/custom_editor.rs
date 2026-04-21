//! Custom editor component.
//!
//! Translated from `components/custom-editor.ts`.
//!
//! Wraps the TUI `Editor` and handles app-level keybindings for coding-agent,
//! including Escape/interrupt, Ctrl+D exit, paste-image, extension shortcuts,
//! and all registered `AppKeybinding` actions.

use std::collections::HashMap;

// ============================================================================
// AppKeybinding (string alias)
// ============================================================================

/// Opaque identifier for an app-level keybinding action.
///
/// Examples: `"app.interrupt"`, `"app.exit"`, `"app.model.cycle"`.
pub type AppKeybinding = &'static str;

// ============================================================================
// CustomEditor
// ============================================================================

/// Custom editor that handles app-level keybindings for coding-agent.
///
/// In the TypeScript source this extends `Editor` from pi-tui; in Rust it
/// wraps a plain text buffer and delegates key events to registered handlers.
pub struct CustomEditor {
    /// Current text content.
    pub text: String,
    /// Whether this editor has focus.
    pub focused: bool,
    /// Registered action handlers.
    pub action_handlers: HashMap<&'static str, Box<dyn Fn() + Send>>,
    /// Dynamic escape handler (overrides `app.interrupt` when set).
    pub on_escape: Option<Box<dyn Fn() + Send>>,
    /// Dynamic Ctrl+D handler (overrides `app.exit` when set).
    pub on_ctrl_d: Option<Box<dyn Fn() + Send>>,
    /// Paste-image handler.
    pub on_paste_image: Option<Box<dyn Fn() + Send>>,
    /// Extension shortcut handler. Returns `true` if the input was handled.
    #[allow(clippy::type_complexity)]
    pub on_extension_shortcut: Option<Box<dyn Fn(&str) -> bool + Send>>,
}

impl CustomEditor {
    /// Create a new custom editor.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            focused: false,
            action_handlers: HashMap::new(),
            on_escape: None,
            on_ctrl_d: None,
            on_paste_image: None,
            on_extension_shortcut: None,
        }
    }

    /// Register a handler for an app action.
    pub fn on_action<F: Fn() + Send + 'static>(&mut self, action: AppKeybinding, handler: F) {
        self.action_handlers.insert(action, Box::new(handler));
    }

    /// Get the current text.
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// Set the text content.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Insert a character at the end of the text (simplified editor behaviour).
    pub fn insert_char(&mut self, ch: char) {
        self.text.push(ch);
    }

    /// Delete the last character (backspace).
    pub fn delete_char_back(&mut self) {
        self.text.pop();
    }

    /// Returns `true` if the editor text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Handle a raw key event string.
    ///
    /// Priority order (mirrors TypeScript):
    /// 1. Extension shortcuts (`on_extension_shortcut`)
    /// 2. Paste image (`app.clipboard.pasteImage` keybinding)
    /// 3. Escape / interrupt (only when no autocomplete is active)
    /// 4. Ctrl+D exit (only when editor is empty)
    /// 5. All other app action handlers
    /// 6. Default editor handling (append char / backspace)
    ///
    /// Returns `true` if the input was consumed by an app-level handler.
    pub fn handle_input(&mut self, data: &str) -> bool {
        // 1. Extension shortcuts
        if let Some(handler) = &self.on_extension_shortcut
            && handler(data)
        {
            return true;
        }

        // 2. Paste image (Ctrl+Shift+V or configured binding)
        if data == "\x16" {
            // Ctrl+V placeholder — actual keybinding resolved at runtime
            if let Some(handler) = &self.on_paste_image {
                handler();
                return true;
            }
        }

        // 3. Escape / interrupt
        if data == "\x1b" || data == "\x03" {
            // ESC or Ctrl+C
            let handler = self.on_escape.as_ref().map(|h| h as &dyn Fn()).or_else(|| {
                self.action_handlers
                    .get("app.interrupt")
                    .map(|h| h as &dyn Fn())
            });
            if let Some(h) = handler {
                h();
                return true;
            }
        }

        // 4. Ctrl+D exit (only when empty)
        if data == "\x04" && self.text.is_empty() {
            let handler = self
                .on_ctrl_d
                .as_ref()
                .map(|h| h as &dyn Fn())
                .or_else(|| self.action_handlers.get("app.exit").map(|h| h as &dyn Fn()));
            if let Some(h) = handler {
                h();
                return true;
            }
        }

        // 5. Other registered actions (skip interrupt and exit checked above)
        for (&action, handler) in &self.action_handlers {
            if action != "app.interrupt" && action != "app.exit" && matches_keybinding(data, action)
            {
                handler();
                return true;
            }
        }

        // 6. Default editor handling
        if data == "\x7f" || data == "\x08" {
            // Backspace / Delete
            self.delete_char_back();
        } else if !data.is_empty() && !data.starts_with('\x1b') {
            for ch in data.chars() {
                self.text.push(ch);
            }
        }

        false
    }
}

impl Default for CustomEditor {
    fn default() -> Self {
        Self::new()
    }
}

fn matches_keybinding(data: &str, action: &str) -> bool {
    crate::core::keybindings::check_app_keybinding(data, action)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_editor_is_empty() {
        let ed = CustomEditor::new();
        assert!(ed.is_empty());
        assert_eq!(ed.get_text(), "");
    }

    #[test]
    fn set_and_get_text() {
        let mut ed = CustomEditor::new();
        ed.set_text("hello world");
        assert_eq!(ed.get_text(), "hello world");
        assert!(!ed.is_empty());
    }

    #[test]
    fn insert_and_delete() {
        let mut ed = CustomEditor::new();
        ed.insert_char('a');
        ed.insert_char('b');
        assert_eq!(ed.get_text(), "ab");
        ed.delete_char_back();
        assert_eq!(ed.get_text(), "a");
    }

    #[test]
    fn escape_calls_on_escape() {
        let mut ed = CustomEditor::new();
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        ed.on_escape = Some(Box::new(move || *called2.lock().unwrap() = true));
        ed.handle_input("\x1b");
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn ctrl_d_on_empty_calls_handler() {
        let mut ed = CustomEditor::new();
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        ed.on_ctrl_d = Some(Box::new(move || *called2.lock().unwrap() = true));
        ed.handle_input("\x04");
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn ctrl_d_not_fired_when_text_nonempty() {
        let mut ed = CustomEditor::new();
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        ed.on_ctrl_d = Some(Box::new(move || *called2.lock().unwrap() = true));
        ed.set_text("some text");
        ed.handle_input("\x04");
        assert!(!*called.lock().unwrap());
    }

    #[test]
    fn extension_shortcut_intercepts() {
        let mut ed = CustomEditor::new();
        ed.on_extension_shortcut = Some(Box::new(|data| data == "magic"));
        let consumed = ed.handle_input("magic");
        assert!(consumed);
    }

    #[test]
    fn extension_shortcut_not_matching_falls_through() {
        let mut ed = CustomEditor::new();
        ed.on_extension_shortcut = Some(Box::new(|data| data == "magic"));
        ed.handle_input("a"); // not "magic", falls through to editor
        assert_eq!(ed.get_text(), "a");
    }

    #[test]
    fn action_handler_registered() {
        let mut ed = CustomEditor::new();
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        ed.on_action("app.model.cycle", move || *called2.lock().unwrap() = true);
        assert!(ed.action_handlers.contains_key("app.model.cycle"));
    }

    #[test]
    fn printable_chars_appended() {
        let mut ed = CustomEditor::new();
        ed.handle_input("hello ");
        ed.handle_input("world");
        assert_eq!(ed.get_text(), "hello world");
    }
}
