//! Extension input component.
//!
//! Translated from `components/extension-input.ts`.
//!
//! Simple single-line text input for extension prompts, with optional timeout.

// ============================================================================
// ExtensionInputComponent
// ============================================================================

/// Simple text input component for extensions.
pub struct ExtensionInputComponent {
    /// Current input value.
    pub value: String,
    /// Whether this component has focus.
    pub focused: bool,
    /// Title shown above the input.
    pub title: String,
    /// Optional timeout in seconds.
    pub timeout_secs: Option<u64>,
    on_submit: Option<Box<dyn Fn(String) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ExtensionInputComponent {
    /// Create a new extension input.
    ///
    /// * `title` — label displayed above the input field.
    /// * `placeholder` — placeholder hint (not used in render, kept for API parity).
    /// * `timeout_secs` — optional auto-cancel timeout in seconds.
    pub fn new(
        title: impl Into<String>,
        _placeholder: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            value: String::new(),
            focused: false,
            title: title.into(),
            timeout_secs,
            on_submit: None,
            on_cancel: None,
        }
    }

    pub fn set_on_submit<F: Fn(String) + Send + 'static>(&mut self, f: F) {
        self.on_submit = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    /// Get the current input value.
    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// Set the input value directly.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
    }

    /// Submit the current value.
    pub fn submit(&self) {
        if let Some(cb) = &self.on_submit {
            cb(self.value.clone());
        }
    }

    /// Cancel input.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let comp = ExtensionInputComponent::new("Enter value", None, None);
        assert_eq!(comp.get_value(), "");
        assert_eq!(comp.title, "Enter value");
        assert!(!comp.focused);
        assert!(comp.timeout_secs.is_none());
    }

    #[test]
    fn new_with_timeout() {
        let comp = ExtensionInputComponent::new("T", None, Some(30));
        assert_eq!(comp.timeout_secs, Some(30));
    }

    #[test]
    fn set_value() {
        let mut comp = ExtensionInputComponent::new("T", None, None);
        comp.set_value("hello");
        assert_eq!(comp.get_value(), "hello");
    }

    #[test]
    fn submit_calls_callback() {
        let mut comp = ExtensionInputComponent::new("T", None, None);
        comp.set_value("test-input");
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured2 = captured.clone();
        comp.set_on_submit(move |v| {
            *captured2.lock().unwrap() = v;
        });
        comp.submit();
        assert_eq!(*captured.lock().unwrap(), "test-input");
    }

    #[test]
    fn cancel_calls_callback() {
        let mut comp = ExtensionInputComponent::new("T", None, None);
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        comp.set_on_cancel(move || {
            *called2.lock().unwrap() = true;
        });
        comp.cancel();
        assert!(*called.lock().unwrap());
    }
}
