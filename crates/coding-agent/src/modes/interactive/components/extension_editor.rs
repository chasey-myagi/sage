//! Extension editor component.
//!
//! Translated from `components/extension-editor.ts`.
//!
//! Multi-line editor for extension prompts, with optional external-editor
//! support (Ctrl+G / VISUAL / EDITOR).

// ============================================================================
// ExtensionEditorComponent
// ============================================================================

/// Multi-line editor component for extensions.
///
/// Supports Ctrl+G to open the system `$VISUAL` / `$EDITOR`.
pub struct ExtensionEditorComponent {
    /// Current text in the editor.
    pub text: String,
    /// Whether this component has focus.
    pub focused: bool,
    /// Title shown above the editor.
    pub title: String,
    on_submit: Option<Box<dyn Fn(String) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ExtensionEditorComponent {
    /// Create a new extension editor.
    ///
    /// * `title` — label displayed above the editor.
    /// * `prefill` — optional initial text.
    pub fn new(title: impl Into<String>, prefill: Option<String>) -> Self {
        Self {
            text: prefill.unwrap_or_default(),
            focused: false,
            title: title.into(),
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

    /// Open the text in an external editor (`$VISUAL` or `$EDITOR`).
    ///
    /// Writes the current text to a temp file, spawns the editor, then reads
    /// the result back. Returns `true` if the content was updated.
    pub fn open_external_editor(&mut self) -> bool {
        let editor_cmd = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .ok();

        let Some(cmd) = editor_cmd else {
            return false;
        };

        let tmp_path = std::env::temp_dir().join(format!(
            "sage-extension-editor-{}.md",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));

        if std::fs::write(&tmp_path, &self.text).is_err() {
            return false;
        }

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let Some(editor_bin) = parts.first() else {
            return false;
        };
        let editor_args = &parts[1..];

        let status = std::process::Command::new(editor_bin)
            .args(editor_args)
            .arg(&tmp_path)
            .status();

        let updated = match status {
            Ok(s) if s.success() => {
                if let Ok(content) = std::fs::read_to_string(&tmp_path) {
                    // Strip trailing newline (mirrors TypeScript `.replace(/\n$/, "")`)
                    let trimmed = content.strip_suffix('\n').unwrap_or(&content).to_string();
                    self.text = trimmed;
                    true
                } else {
                    false
                }
            }
            _ => false,
        };

        let _ = std::fs::remove_file(&tmp_path);
        updated
    }

    /// Submit the current text.
    pub fn submit(&self) {
        if let Some(cb) = &self.on_submit {
            cb(self.text.clone());
        }
    }

    /// Cancel editing.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Get the current text.
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// Set the current text.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_with_prefill() {
        let comp = ExtensionEditorComponent::new("My Title", Some("hello world".into()));
        assert_eq!(comp.get_text(), "hello world");
        assert_eq!(comp.title, "My Title");
        assert!(!comp.focused);
    }

    #[test]
    fn new_without_prefill() {
        let comp = ExtensionEditorComponent::new("Empty", None);
        assert_eq!(comp.get_text(), "");
    }

    #[test]
    fn set_text() {
        let mut comp = ExtensionEditorComponent::new("T", None);
        comp.set_text("updated");
        assert_eq!(comp.get_text(), "updated");
    }

    #[test]
    fn submit_calls_callback() {
        let mut comp = ExtensionEditorComponent::new("T", Some("value".into()));
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        comp.set_on_submit(move |_text| {
            *called2.lock().unwrap() = true;
        });
        comp.submit();
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn cancel_calls_callback() {
        let mut comp = ExtensionEditorComponent::new("T", None);
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        comp.set_on_cancel(move || {
            *called2.lock().unwrap() = true;
        });
        comp.cancel();
        assert!(*called.lock().unwrap());
    }
}
