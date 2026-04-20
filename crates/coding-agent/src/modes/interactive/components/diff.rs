//! Diff rendering utilities.
//!
//! Translated from `components/diff.ts`.
//!
//! Renders unified diff text with colored lines and intra-line change highlighting.

use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// Options for diff rendering (kept for API compatibility).
#[derive(Debug, Default)]
pub struct RenderDiffOptions {
    /// File path — currently unused; kept for compatibility.
    pub file_path: Option<String>,
}

/// Parse a unified diff line into (prefix, line_num, content).
fn parse_diff_line(line: &str) -> Option<(&str, &str, &str)> {
    // Match format: "+123 content", "-123 content", " 123 content"
    let prefix = match line.chars().next() {
        Some('+') => "+",
        Some('-') => "-",
        Some(' ') => " ",
        _ => return None,
    };

    let rest = &line[1..];
    // Find the space separating line number from content
    if let Some(space_pos) = rest.find(' ') {
        let line_num = &rest[..space_pos];
        let content = &rest[space_pos + 1..];
        Some((prefix, line_num, content))
    } else {
        // Line with only prefix and no space (e.g., diff header lines)
        Some((prefix, "", rest))
    }
}

/// Replace tabs with spaces for consistent rendering.
fn replace_tabs(text: &str) -> String {
    text.replace('\t', "   ")
}

/// Compute a very simple word-level diff and highlight changed words with inverse.
/// Returns (removed_line, added_line) with ANSI formatting.
fn render_intra_line_diff(old_content: &str, new_content: &str) -> (String, String) {
    let t = get_theme();

    // Simple word tokenization
    let old_words: Vec<&str> = old_content.split_whitespace().collect();
    let new_words: Vec<&str> = new_content.split_whitespace().collect();

    // LCS-based diff (very simplified)
    let mut removed_line = String::new();
    let mut added_line = String::new();

    // Build sets for quick lookup
    use std::collections::HashSet;
    let old_set: HashSet<&str> = old_words.iter().copied().collect();
    let new_set: HashSet<&str> = new_words.iter().copied().collect();

    // For removed line: highlight words not in new
    for (i, word) in old_words.iter().enumerate() {
        if i > 0 {
            removed_line.push(' ');
        }
        if !new_set.contains(word) {
            removed_line.push_str(&t.inverse(word));
        } else {
            removed_line.push_str(word);
        }
    }

    // For added line: highlight words not in old
    for (i, word) in new_words.iter().enumerate() {
        if i > 0 {
            added_line.push(' ');
        }
        if !old_set.contains(word) {
            added_line.push_str(&t.inverse(word));
        } else {
            added_line.push_str(word);
        }
    }

    (removed_line, added_line)
}

/// Render a unified diff string with colored lines and intra-line change highlighting.
///
/// - Context lines: dim/gray
/// - Removed lines (-): red
/// - Added lines (+): green
/// - Intra-line changes: inverse highlighting
pub fn render_diff(diff_text: &str, _options: RenderDiffOptions) -> String {
    let t = get_theme();
    let lines: Vec<&str> = diff_text.split('\n').collect();
    let mut result_lines: Vec<String> = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if let Some((prefix, line_num, content)) = parse_diff_line(line) {
            let content_with_tabs = replace_tabs(content);

            // Look ahead for matching +/- pair for intra-line diff
            if prefix == "-" {
                if let Some(next_line) = lines.get(i + 1)
                    && let Some(("+", next_num, next_content)) = parse_diff_line(next_line)
                {
                    let next_content_with_tabs = replace_tabs(next_content);
                    let (rm, ad) =
                        render_intra_line_diff(&content_with_tabs, &next_content_with_tabs);

                    let removed_prefix =
                        t.fg(ThemeColor::ToolDiffRemoved, &format!("-{line_num} "));
                    let added_prefix = t.fg(ThemeColor::ToolDiffAdded, &format!("+{next_num} "));

                    result_lines.push(format!(
                        "{}{}",
                        removed_prefix,
                        t.fg(ThemeColor::ToolDiffRemoved, &rm)
                    ));
                    result_lines.push(format!(
                        "{}{}",
                        added_prefix,
                        t.fg(ThemeColor::ToolDiffAdded, &ad)
                    ));
                    i += 2;
                    continue;
                }

                // No matching added line — render plain removed
                let removed_prefix = t.fg(ThemeColor::ToolDiffRemoved, &format!("-{line_num} "));
                result_lines.push(format!(
                    "{}{}",
                    removed_prefix,
                    t.fg(ThemeColor::ToolDiffRemoved, &content_with_tabs)
                ));
            } else if prefix == "+" {
                let added_prefix = t.fg(ThemeColor::ToolDiffAdded, &format!("+{line_num} "));
                result_lines.push(format!(
                    "{}{}",
                    added_prefix,
                    t.fg(ThemeColor::ToolDiffAdded, &content_with_tabs)
                ));
            } else {
                // Context line
                let ctx_prefix = t.fg(ThemeColor::ToolDiffContext, &format!(" {line_num} "));
                result_lines.push(format!(
                    "{}{}",
                    ctx_prefix,
                    t.fg(ThemeColor::ToolDiffContext, &content_with_tabs)
                ));
            }
        } else if !line.is_empty() {
            // Diff header lines (---/+++ or @@ ... @@)
            result_lines.push(t.fg(ThemeColor::Dim, line));
        }

        i += 1;
    }

    result_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_line_added() {
        let (prefix, _, content) = parse_diff_line("+1 added line").unwrap();
        assert_eq!(prefix, "+");
        assert_eq!(content, "added line");
    }

    #[test]
    fn parse_diff_line_removed() {
        let (prefix, _, content) = parse_diff_line("-5 removed line").unwrap();
        assert_eq!(prefix, "-");
        assert_eq!(content, "removed line");
    }

    #[test]
    fn render_diff_produces_output() {
        let diff = "-1 old line\n+1 new line\n 2 context";
        let out = render_diff(diff, RenderDiffOptions::default());
        assert!(!out.is_empty());
    }
}
