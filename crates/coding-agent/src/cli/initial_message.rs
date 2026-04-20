//! Initial message construction for non-interactive mode.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/initial-message.ts`.

use super::file_processor::ImageContent;

// ============================================================================
// Types
// ============================================================================

/// Result of combining all inputs into the first message.
#[derive(Debug, Default)]
pub struct InitialMessageResult {
    pub initial_message: Option<String>,
    pub initial_images: Option<Vec<ImageContent>>,
}

// ============================================================================
// Builder
// ============================================================================

/// Combine stdin content, `@file` text, and the first CLI message into a
/// single initial prompt for non-interactive mode.
///
/// Mirrors `buildInitialMessage()` from TypeScript.
pub fn build_initial_message(
    messages: &mut Vec<String>,
    file_text: Option<&str>,
    file_images: Option<Vec<ImageContent>>,
    stdin_content: Option<&str>,
) -> InitialMessageResult {
    let mut parts: Vec<String> = Vec::new();

    if let Some(stdin) = stdin_content {
        parts.push(stdin.to_string());
    }

    if let Some(ft) = file_text {
        if !ft.is_empty() {
            parts.push(ft.to_string());
        }
    }

    if !messages.is_empty() {
        parts.push(messages.remove(0));
    }

    let initial_message = if parts.is_empty() {
        None
    } else {
        Some(parts.concat())
    };

    let initial_images = file_images.filter(|imgs| !imgs.is_empty());

    InitialMessageResult {
        initial_message,
        initial_images,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_stdin_and_message() {
        let mut messages = vec!["hello".to_string()];
        let result = build_initial_message(&mut messages, None, None, Some("stdin: "));
        assert_eq!(result.initial_message.as_deref(), Some("stdin: hello"));
        assert!(messages.is_empty());
    }

    #[test]
    fn empty_when_no_inputs() {
        let mut messages = vec![];
        let result = build_initial_message(&mut messages, None, None, None);
        assert!(result.initial_message.is_none());
        assert!(result.initial_images.is_none());
    }

    #[test]
    fn remaining_messages_after_first() {
        let mut messages = vec!["first".to_string(), "second".to_string()];
        let _ = build_initial_message(&mut messages, None, None, None);
        // Only the first message is consumed
        assert_eq!(messages, vec!["second"]);
    }

    /// Translated from `initial-message.test.ts`:
    /// "merges piped stdin with the first CLI message into one prompt"
    #[test]
    fn merges_stdin_with_first_cli_message() {
        let mut messages = vec!["Summarize the text given".to_string()];
        let result = build_initial_message(&mut messages, None, None, Some("README contents\n"));
        assert_eq!(
            result.initial_message.as_deref(),
            Some("README contents\nSummarize the text given")
        );
        assert!(messages.is_empty());
    }

    /// "uses stdin as the initial prompt when no CLI message is present"
    #[test]
    fn uses_stdin_when_no_cli_message() {
        let mut messages = vec![];
        let result = build_initial_message(&mut messages, None, None, Some("README contents"));
        assert_eq!(result.initial_message.as_deref(), Some("README contents"));
        assert!(messages.is_empty());
    }

    /// "combines stdin, file text, and first CLI message in one prompt"
    #[test]
    fn combines_stdin_file_text_and_first_message() {
        let mut messages = vec!["Explain it".to_string(), "Second message".to_string()];
        let result =
            build_initial_message(&mut messages, Some("file\n"), None, Some("stdin\n"));
        assert_eq!(
            result.initial_message.as_deref(),
            Some("stdin\nfile\nExplain it")
        );
        assert_eq!(messages, vec!["Second message"]);
    }
}
