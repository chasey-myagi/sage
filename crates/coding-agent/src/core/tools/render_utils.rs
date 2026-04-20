//! Rendering utilities shared across coding-agent tools.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/render-utils.ts`.
//!
//! These helpers produce the text strings used by each tool's `renderCall` /
//! `renderResult` functions.  They are pure functions with no side effects.

use std::env;

// ============================================================================
// shorten_path
// ============================================================================

/// Shorten an absolute path by replacing the home directory prefix with `~`.
///
/// Mirrors `shortenPath()` from `render-utils.ts`.
pub fn shorten_path(path: &str) -> String {
    if let Ok(home) = env::var("HOME")
        && path.starts_with(&home)
    {
        return format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

// ============================================================================
// str
// ============================================================================

/// Convert an `Option<&str>` or similar to `Option<String>`.
///
/// - `Some(s)` where `s` is a non-empty string  → `Some(s.to_string())`
/// - `Some(s)` where `s` is empty               → `Some(String::new())`
/// - `None`                                     → `None`
///
/// Mirrors `str()` from `render-utils.ts`, adapted for Rust's ownership model.
pub fn opt_str(value: Option<&str>) -> Option<String> {
    value.map(str::to_string)
}

// ============================================================================
// replace_tabs
// ============================================================================

/// Replace tab characters with three spaces for display purposes.
///
/// Mirrors `replaceTabs()` from `render-utils.ts`.
pub fn replace_tabs(text: &str) -> String {
    text.replace('\t', "   ")
}

// ============================================================================
// normalize_display_text
// ============================================================================

/// Normalize text for display by stripping carriage returns.
///
/// Mirrors `normalizeDisplayText()` from `render-utils.ts`.
pub fn normalize_display_text(text: &str) -> String {
    text.replace('\r', "")
}

// ============================================================================
// get_text_output
// ============================================================================

/// Extract plain text from a tool-result content array.
///
/// Concatenates all `type = "text"` blocks.  Image blocks are ignored
/// (they are handled separately in the TUI by the rendering pipeline).
///
/// Mirrors the text-extraction part of `getTextOutput()` from `render-utils.ts`.
pub fn get_text_output(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- shorten_path ----

    #[test]
    fn shorten_path_with_home() {
        let home = env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return; // skip if HOME not set
        }
        let path = format!("{home}/projects/foo.rs");
        let shortened = shorten_path(&path);
        assert!(shortened.starts_with('~'));
        assert!(shortened.ends_with("/projects/foo.rs"));
    }

    #[test]
    fn shorten_path_no_home_prefix() {
        let path = "/usr/local/bin/cargo";
        let shortened = shorten_path(path);
        assert_eq!(shortened, path);
    }

    // ---- opt_str ----

    #[test]
    fn opt_str_some_value() {
        assert_eq!(opt_str(Some("hello")), Some("hello".to_string()));
    }

    #[test]
    fn opt_str_none() {
        assert_eq!(opt_str(None), None);
    }

    #[test]
    fn opt_str_empty_string() {
        assert_eq!(opt_str(Some("")), Some(String::new()));
    }

    // ---- replace_tabs ----

    #[test]
    fn replace_tabs_single() {
        assert_eq!(replace_tabs("a\tb"), "a   b");
    }

    #[test]
    fn replace_tabs_multiple() {
        assert_eq!(replace_tabs("\t\t"), "      ");
    }

    #[test]
    fn replace_tabs_no_tabs() {
        assert_eq!(replace_tabs("hello world"), "hello world");
    }

    // ---- normalize_display_text ----

    #[test]
    fn normalize_display_text_strips_cr() {
        assert_eq!(normalize_display_text("line1\r\nline2"), "line1\nline2");
    }

    #[test]
    fn normalize_display_text_no_cr() {
        assert_eq!(normalize_display_text("clean text"), "clean text");
    }

    // ---- get_text_output ----

    #[test]
    fn get_text_output_single_text_block() {
        let content = vec![serde_json::json!({"type": "text", "text": "hello"})];
        assert_eq!(get_text_output(&content), "hello");
    }

    #[test]
    fn get_text_output_multiple_text_blocks() {
        let content = vec![
            serde_json::json!({"type": "text", "text": "line1"}),
            serde_json::json!({"type": "text", "text": "line2"}),
        ];
        assert_eq!(get_text_output(&content), "line1\nline2");
    }

    #[test]
    fn get_text_output_ignores_image_blocks() {
        let content = vec![
            serde_json::json!({"type": "text", "text": "caption"}),
            serde_json::json!({"type": "image", "data": "base64data", "mimeType": "image/png"}),
        ];
        assert_eq!(get_text_output(&content), "caption");
    }

    #[test]
    fn get_text_output_empty() {
        assert_eq!(get_text_output(&[]), "");
    }

    #[test]
    fn get_text_output_only_images() {
        let content =
            vec![serde_json::json!({"type": "image", "data": "x", "mimeType": "image/png"})];
        assert_eq!(get_text_output(&content), "");
    }
}
