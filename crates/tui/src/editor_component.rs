/// EditorComponent trait — interface for custom editor implementations.

use crate::autocomplete::AutocompleteProvider;
use crate::tui::Component;

/// Interface for custom editor components.
///
/// Allows extensions to provide their own editor implementation
/// (e.g., vim mode, emacs mode, custom keybindings) while maintaining
/// compatibility with the core application.
pub trait EditorComponent: Component {
    // =========================================================================
    // Core text access (required)
    // =========================================================================

    /// Get the current text content.
    fn get_text(&self) -> String;

    /// Set the text content.
    fn set_text(&mut self, text: &str);

    /// Handle raw terminal input (key presses, paste sequences, etc.).
    fn handle_input(&mut self, data: &str);

    // =========================================================================
    // Callbacks (required)
    // =========================================================================

    /// Set submit callback.
    fn set_on_submit(&mut self, cb: Box<dyn Fn(String) + Send + 'static>);

    /// Set on-change callback.
    fn set_on_change(&mut self, cb: Box<dyn Fn(String) + Send + 'static>);

    // =========================================================================
    // History support (optional)
    // =========================================================================

    /// Add text to history for up/down navigation.
    fn add_to_history(&mut self, _text: &str) {}

    // =========================================================================
    // Advanced text manipulation (optional)
    // =========================================================================

    /// Insert text at current cursor position.
    fn insert_text_at_cursor(&mut self, _text: &str) {}

    /// Get text with any markers expanded (e.g., paste markers).
    /// Falls back to get_text() if not implemented.
    fn get_expanded_text(&self) -> String {
        self.get_text()
    }

    // =========================================================================
    // Autocomplete support (optional)
    // =========================================================================

    /// Set the autocomplete provider.
    fn set_autocomplete_provider(&mut self, _provider: Box<dyn AutocompleteProvider>) {}

    // =========================================================================
    // Appearance (optional)
    // =========================================================================

    /// Set horizontal padding.
    fn set_padding_x(&mut self, _padding: u16) {}

    /// Set max visible items in autocomplete dropdown.
    fn set_autocomplete_max_visible(&mut self, _max_visible: usize) {}
}
