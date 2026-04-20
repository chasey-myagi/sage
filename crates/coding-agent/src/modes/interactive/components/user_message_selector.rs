//! User message selector component.
//!
//! Translated from `components/user-message-selector.ts`.
//!
//! Displays a list of past user messages for branch-from selection.

// ============================================================================
// Types
// ============================================================================

/// A user message entry in the selector.
#[derive(Debug, Clone)]
pub struct UserMessageItem {
    /// Entry ID in the session.
    pub id: String,
    /// The message text.
    pub text: String,
    /// Optional timestamp string.
    pub timestamp: Option<String>,
}

// ============================================================================
// UserMessageList
// ============================================================================

/// Custom user message list with selection (inner component).
///
/// Messages are stored in chronological order (oldest to newest).
/// Starts with the most recent message selected.
pub struct UserMessageList {
    pub messages: Vec<UserMessageItem>,
    pub selected_index: usize,
    pub max_visible: usize,
    pub on_select: Option<Box<dyn Fn(String) + Send>>,
    pub on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl UserMessageList {
    pub fn new(messages: Vec<UserMessageItem>) -> Self {
        let last = messages.len().saturating_sub(1);
        Self {
            messages,
            selected_index: last,
            max_visible: 10,
            on_select: None,
            on_cancel: None,
        }
    }

    /// Move selection up — wraps to bottom.
    pub fn select_up(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.messages.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Move selection down — wraps to top.
    pub fn select_down(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        if self.selected_index == self.messages.len() - 1 {
            self.selected_index = 0;
        } else {
            self.selected_index += 1;
        }
    }

    /// Confirm the current selection.
    pub fn confirm(&self) {
        if let Some(msg) = self.messages.get(self.selected_index)
            && let Some(cb) = &self.on_select
        {
            cb(msg.id.clone());
        }
    }

    /// Cancel without selecting.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Get the currently selected message, if any.
    pub fn selected_message(&self) -> Option<&UserMessageItem> {
        self.messages.get(self.selected_index)
    }

    /// Render lines for the visible window (for testing).
    pub fn render_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        if self.messages.is_empty() {
            lines.push("  No user messages found".to_string());
            return lines;
        }

        let start_index = {
            let half = self.max_visible / 2;
            let ideal = self.selected_index.saturating_sub(half);
            let max_start = self.messages.len().saturating_sub(self.max_visible);
            ideal.min(max_start)
        };
        let end_index = (start_index + self.max_visible).min(self.messages.len());

        for i in start_index..end_index {
            let msg = &self.messages[i];
            let is_selected = i == self.selected_index;

            // Normalize to single line
            let normalized: String = msg.text.replace('\n', " ").trim().to_string();
            let max_msg_width = width.saturating_sub(2);
            let truncated = if normalized.len() > max_msg_width {
                format!("{}…", &normalized[..max_msg_width.saturating_sub(1)])
            } else {
                normalized.clone()
            };

            let cursor = if is_selected { "› " } else { "  " };
            let message_line = if is_selected {
                format!("{cursor}**{truncated}**")
            } else {
                format!("{cursor}{truncated}")
            };
            lines.push(message_line);

            let position = i + 1;
            lines.push(format!("  Message {} of {}", position, self.messages.len()));
            lines.push(String::new()); // blank separator
        }

        // Scroll indicator
        if start_index > 0 || end_index < self.messages.len() {
            lines.push(format!(
                "  ({}/{})",
                self.selected_index + 1,
                self.messages.len()
            ));
        }

        lines
    }
}

// ============================================================================
// UserMessageSelectorComponent
// ============================================================================

/// Component that renders a user message selector for branching.
pub struct UserMessageSelectorComponent {
    pub message_list: UserMessageList,
}

impl UserMessageSelectorComponent {
    /// Create a new user message selector.
    ///
    /// If `messages` is empty, the `on_cancel` callback is scheduled immediately
    /// (caller should handle this).
    pub fn new(
        messages: Vec<UserMessageItem>,
        on_select: impl Fn(String) + Send + 'static,
        on_cancel: impl Fn() + Send + 'static,
    ) -> Self {
        let mut list = UserMessageList::new(messages);
        list.on_select = Some(Box::new(on_select));
        list.on_cancel = Some(Box::new(on_cancel));
        Self { message_list: list }
    }

    /// Returns `true` if there are no messages (auto-cancel condition).
    pub fn is_empty(&self) -> bool {
        self.message_list.messages.is_empty()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_messages(n: usize) -> Vec<UserMessageItem> {
        (0..n)
            .map(|i| UserMessageItem {
                id: format!("id-{i}"),
                text: format!("message {i}"),
                timestamp: None,
            })
            .collect()
    }

    #[test]
    fn starts_at_last_message() {
        let msgs = make_messages(5);
        let list = UserMessageList::new(msgs);
        assert_eq!(list.selected_index, 4);
    }

    #[test]
    fn select_up_wraps() {
        let msgs = make_messages(3);
        let mut list = UserMessageList::new(msgs);
        list.selected_index = 0;
        list.select_up();
        assert_eq!(list.selected_index, 2); // wraps to end
    }

    #[test]
    fn select_down_wraps() {
        let msgs = make_messages(3);
        let mut list = UserMessageList::new(msgs);
        list.selected_index = 2;
        list.select_down();
        assert_eq!(list.selected_index, 0); // wraps to start
    }

    #[test]
    fn confirm_calls_callback() {
        let msgs = make_messages(3);
        let mut list = UserMessageList::new(msgs);
        list.selected_index = 1;
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let cap2 = captured.clone();
        list.on_select = Some(Box::new(move |id| *cap2.lock().unwrap() = id));
        list.confirm();
        assert_eq!(*captured.lock().unwrap(), "id-1");
    }

    #[test]
    fn cancel_calls_callback() {
        let msgs = make_messages(1);
        let mut list = UserMessageList::new(msgs);
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        list.on_cancel = Some(Box::new(move || *called2.lock().unwrap() = true));
        list.cancel();
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn empty_messages() {
        let list = UserMessageList::new(vec![]);
        assert!(list.selected_message().is_none());
        let lines = list.render_lines(80);
        assert!(lines.iter().any(|l| l.contains("No user messages")));
    }

    #[test]
    fn render_lines_basic() {
        let msgs = make_messages(3);
        let mut list = UserMessageList::new(msgs);
        list.selected_index = 1;
        let lines = list.render_lines(40);
        // Should have message lines and separators
        assert!(!lines.is_empty());
    }

    #[test]
    fn is_empty_detection() {
        let _called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let comp_empty = UserMessageSelectorComponent::new(vec![], |_| {}, || {});
        assert!(comp_empty.is_empty());

        let comp_nonempty = UserMessageSelectorComponent::new(make_messages(1), |_| {}, || {});
        assert!(!comp_nonempty.is_empty());
    }
}
