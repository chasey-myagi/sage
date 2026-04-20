//! Extension selector component.
//!
//! Translated from `components/extension-selector.ts`.
//!
//! Generic list selector for string options, used by extensions to present
//! choices to the user.

// ============================================================================
// ExtensionSelectorComponent
// ============================================================================

/// Generic selector component for extensions.
///
/// Displays a list of string options with keyboard navigation (↑/↓ or j/k).
pub struct ExtensionSelectorComponent {
    /// Available options.
    pub options: Vec<String>,
    /// Currently highlighted option index.
    pub selected_index: usize,
    /// Title shown above the list.
    pub title: String,
    /// Optional auto-cancel timeout in seconds.
    pub timeout_secs: Option<u64>,
    on_select: Option<Box<dyn Fn(String) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ExtensionSelectorComponent {
    /// Create a new extension selector.
    pub fn new(
        title: impl Into<String>,
        options: Vec<String>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            options,
            selected_index: 0,
            title: title.into(),
            timeout_secs,
            on_select: None,
            on_cancel: None,
        }
    }

    pub fn set_on_select<F: Fn(String) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    /// Move selection up (wraps at top).
    pub fn select_up(&mut self) {
        if self.options.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Move selection down (clamps at bottom).
    pub fn select_down(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected_index =
            (self.selected_index + 1).min(self.options.len().saturating_sub(1));
    }

    /// Confirm the current selection.
    pub fn confirm(&self) {
        if let Some(option) = self.options.get(self.selected_index) {
            if let Some(cb) = &self.on_select {
                cb(option.clone());
            }
        }
    }

    /// Cancel without selecting.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Get the currently selected option string, if any.
    pub fn selected_option(&self) -> Option<&str> {
        self.options.get(self.selected_index).map(|s| s.as_str())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_selector(options: &[&str]) -> ExtensionSelectorComponent {
        ExtensionSelectorComponent::new(
            "Choose",
            options.iter().map(|s| s.to_string()).collect(),
            None,
        )
    }

    #[test]
    fn initial_state() {
        let sel = make_selector(&["a", "b", "c"]);
        assert_eq!(sel.selected_index, 0);
        assert_eq!(sel.selected_option(), Some("a"));
    }

    #[test]
    fn select_down() {
        let mut sel = make_selector(&["a", "b", "c"]);
        sel.select_down();
        assert_eq!(sel.selected_index, 1);
        assert_eq!(sel.selected_option(), Some("b"));
    }

    #[test]
    fn select_up_at_top_stays() {
        let mut sel = make_selector(&["a", "b"]);
        sel.select_up();
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn select_down_clamps_at_bottom() {
        let mut sel = make_selector(&["a", "b"]);
        sel.select_down();
        sel.select_down();
        sel.select_down();
        assert_eq!(sel.selected_index, 1);
    }

    #[test]
    fn confirm_calls_callback() {
        let mut sel = make_selector(&["first", "second"]);
        sel.select_down();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured2 = captured.clone();
        sel.set_on_select(move |opt| {
            *captured2.lock().unwrap() = opt;
        });
        sel.confirm();
        assert_eq!(*captured.lock().unwrap(), "second");
    }

    #[test]
    fn cancel_calls_callback() {
        let mut sel = make_selector(&["x"]);
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        sel.set_on_cancel(move || *called2.lock().unwrap() = true);
        sel.cancel();
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn empty_selector() {
        let sel = make_selector(&[]);
        assert!(sel.selected_option().is_none());
    }
}
